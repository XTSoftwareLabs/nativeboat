//! CSS animations: `animation-*` properties run the named `@keyframes`
//! over time, for elements and pseudo-elements.

use super::values::{PROPERTIES, Value};
use super::{Target, TransitionEvent};
use crate::prelude::*;
use crate::styles::Styles;
use alloc::collections::BTreeMap;
use nativeboat_css::cascade::keyframe_style;
use nativeboat_css::computed::Specified;
use nativeboat_css::parser::{Keyframe, Rule, Stylesheet};
use nativeboat_css::types::{AnimationDirection, Display, Easing, FillMode, Iterations, PlayState};
use nativeboat_css::{ComputedStyle, MediaEnvironment, PropertyId};

/// The `@keyframes` rules of a document by name; the last rule of a name
/// wins.
pub type Keyframes = BTreeMap<String, Vec<Keyframe>>;

/// The `@keyframes` rules of style sheets, including those in `@media`
/// rules that match `media`, `@supports` and `@layer` rules.
pub fn keyframes<'a>(
    sheets: impl IntoIterator<Item = &'a Stylesheet>,
    media: &MediaEnvironment,
) -> Keyframes {
    fn collect(rules: &[Rule], media: &MediaEnvironment, found: &mut Keyframes) {
        for rule in rules {
            match rule {
                Rule::Keyframes { name, keyframes } => {
                    found.insert(name.clone(), keyframes.clone());
                }
                Rule::Media { query, rules } if query.matches(media) => {
                    collect(rules, media, found);
                }
                Rule::Supports { condition, rules }
                    if nativeboat_css::cascade::supports(condition) =>
                {
                    collect(rules, media, found);
                }
                Rule::Layer { rules, .. } | Rule::Container { rules, .. } => {
                    collect(rules, media, found);
                }
                _ => {}
            }
        }
    }
    let mut found = Keyframes::new();
    for sheet in sheets {
        collect(&sheet.rules, media, &mut found);
    }
    found
}

/// One keyframe's value of a property.
#[derive(Debug, Clone)]
struct Stop {
    offset: f32,
    value: Value,
    /// The timing function to the next stop.
    easing: Easing,
}

/// The timing of an animation, from its element's `animation-*` lists.
#[derive(Debug, Clone, PartialEq)]
struct Timing {
    duration: f64,
    delay: f64,
    iterations: f64,
    direction: AnimationDirection,
    fill: FillMode,
    easing: Easing,
    paused: bool,
}

impl Timing {
    fn active_duration(&self) -> f64 {
        if self.duration <= 0.0 || self.iterations <= 0.0 {
            0.0
        } else {
            self.duration * self.iterations
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    Before,
    Active,
    After,
}

#[derive(Debug)]
struct Animation {
    name: String,
    timing: Timing,
    /// The stops of each animated property, by index into `PROPERTIES`.
    properties: Vec<(usize, Vec<Stop>)>,
    /// When the animation began, in seconds.
    start: f64,
    /// Its local time while paused.
    held: Option<f64>,
    phase: Option<Phase>,
    iteration: u64,
}

impl Animation {
    fn local(&self, now: f64) -> f64 {
        self.held.unwrap_or(now - self.start) - self.timing.delay
    }

    fn phase(&self, now: f64) -> Phase {
        let local = self.local(now);
        if local < 0.0 {
            Phase::Before
        } else if local >= self.timing.active_duration() {
            Phase::After
        } else {
            Phase::Active
        }
    }

    /// The iteration and the progress through it at `now`, if the
    /// animation has an effect then.
    fn progress(&self, now: f64) -> Option<(u64, f32)> {
        let timing = &self.timing;
        let local = self.local(now);
        let (iteration, fraction) = match self.phase(now) {
            Phase::Before => {
                if !matches!(timing.fill, FillMode::Backwards | FillMode::Both) {
                    return None;
                }
                (0, 0.0)
            }
            Phase::After => {
                if !matches!(timing.fill, FillMode::Forwards | FillMode::Both) {
                    return None;
                }
                let end = timing.iterations.max(0.0);
                if timing.duration <= 0.0 || end <= 0.0 {
                    (0, if end > 0.0 { 1.0 } else { 0.0 })
                } else if fract(end) == 0.0 {
                    ((end - 1.0) as u64, 1.0)
                } else {
                    (libm::floor(end) as u64, fract(end))
                }
            }
            Phase::Active => {
                let position = local / timing.duration;
                (libm::floor(position) as u64, fract(position))
            }
        };
        let reversed = match timing.direction {
            AnimationDirection::Normal => false,
            AnimationDirection::Reverse => true,
            AnimationDirection::Alternate => iteration % 2 == 1,
            AnimationDirection::AlternateReverse => iteration % 2 == 0,
        };
        let fraction = fraction as f32;
        Some((iteration, if reversed { 1.0 - fraction } else { fraction }))
    }
}

/// The fractional part of a non-negative number.
fn fract(value: f64) -> f64 {
    value - libm::floor(value)
}

/// A property's value at a progress through its stops.
fn value_at(stops: &[Stop], progress: f32) -> Value {
    let next = stops
        .iter()
        .position(|stop| stop.offset > progress)
        .unwrap_or(stops.len() - 1)
        .max(1);
    let (from, to) = (&stops[next - 1], &stops[next]);
    let span = to.offset - from.offset;
    let local = if span <= 0.0 {
        1.0
    } else {
        ((progress - from.offset) / span).clamp(0.0, 1.0)
    };
    let eased = from.easing.apply(local);
    from.value.interpolate(&to.value, eased).unwrap_or_else(|| {
        if eased < 0.5 {
            from.value.clone()
        } else {
            to.value.clone()
        }
    })
}

/// An element's timing for the animation at `index` of its name list.
fn timing(style: &ComputedStyle, index: usize) -> Timing {
    let at = |list_len: usize| index % list_len.max(1);
    let duration = &style.animation_duration.0;
    let delay = &style.animation_delay.0;
    let count = &style.animation_iteration_count.0;
    let direction = &style.animation_direction.0;
    let fill = &style.animation_fill_mode.0;
    let easing = &style.animation_timing_function.0;
    let state = &style.animation_play_state.0;
    Timing {
        duration: duration
            .get(at(duration.len()))
            .map_or(0.0, |time| f64::from(time.0)),
        delay: delay
            .get(at(delay.len()))
            .map_or(0.0, |time| f64::from(time.0)),
        iterations: match count.get(at(count.len())) {
            Some(Iterations::Count(count)) => f64::from(*count),
            Some(Iterations::Infinite) => f64::INFINITY,
            None => 1.0,
        },
        direction: direction
            .get(at(direction.len()))
            .copied()
            .unwrap_or_default(),
        fill: fill.get(at(fill.len())).copied().unwrap_or_default(),
        easing: easing
            .get(at(easing.len()))
            .copied()
            .unwrap_or(Easing::EASE),
        paused: state.get(at(state.len())) == Some(&PlayState::Paused),
    }
}

/// The stops of each property the keyframes animate for an element whose
/// style is `style`; offsets without a keyframe take the element's value.
fn resolve(
    keyframes: &[Keyframe],
    style: &ComputedStyle,
    environment: &nativeboat_css::StyleEnvironment,
    easing: Easing,
) -> Vec<(usize, Vec<Stop>)> {
    let mut stops: BTreeMap<usize, Vec<Stop>> = BTreeMap::new();
    let mut ordered: Vec<(f32, &Keyframe)> = keyframes
        .iter()
        .flat_map(|keyframe| {
            keyframe
                .offsets
                .iter()
                .map(move |offset| (*offset as f32, keyframe))
        })
        .filter(|(offset, _)| (0.0..=1.0).contains(offset))
        .collect();
    ordered.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (offset, keyframe) in ordered {
        let ids: Vec<PropertyId> = keyframe
            .declarations
            .iter()
            .filter(|declaration| !declaration.important)
            .flat_map(|declaration| {
                let name = nativeboat_css::shorthand::unprefixed(&declaration.name);
                PropertyId::from_name(name)
                    .map(|id| vec![id])
                    .or_else(|| nativeboat_css::shorthand::longhands_of(name))
                    .unwrap_or_default()
            })
            .collect();
        if ids.is_empty() {
            continue;
        }
        let frame = keyframe_style(style, &keyframe.declarations, environment);
        let frame_easing = keyframe
            .declarations
            .iter()
            .rev()
            .find(|declaration| {
                nativeboat_css::shorthand::unprefixed(&declaration.name)
                    == "animation-timing-function"
            })
            .and_then(|declaration| {
                let values = nativeboat_css::values::parse_component_values(&declaration.value);
                ComputedStyle::is_valid(PropertyId::AnimationTimingFunction, &values).then(
                    || {
                        let specified = BTreeMap::from([(
                            PropertyId::AnimationTimingFunction,
                            Specified::Value(values),
                        )]);
                        style
                            .with_specified(&specified, environment)
                            .animation_timing_function
                            .0
                            .first()
                            .copied()
                    },
                )?
            })
            .unwrap_or(easing);
        for (index, property) in PROPERTIES.iter().enumerate() {
            if !ids.contains(&property.id) {
                continue;
            }
            let list = stops.entry(index).or_default();
            // A later keyframe at the same offset replaces an earlier one.
            list.retain(|stop| stop.offset != offset);
            list.push(Stop {
                offset,
                value: (property.read)(&frame),
                easing: frame_easing,
            });
        }
    }
    stops
        .into_iter()
        .map(|(index, mut list)| {
            let underlying = (PROPERTIES[index].read)(style);
            if list.first().is_none_or(|stop| stop.offset > 0.0) {
                list.insert(
                    0,
                    Stop {
                        offset: 0.0,
                        value: underlying.clone(),
                        easing,
                    },
                );
            }
            if list.last().is_none_or(|stop| stop.offset < 1.0) {
                list.push(Stop {
                    offset: 1.0,
                    value: underlying,
                    easing,
                });
            }
            (index, list)
        })
        .collect()
}

/// The running CSS animations of a document. Times are in seconds.
#[derive(Debug, Default)]
pub(super) struct Animations {
    running: BTreeMap<(Target, usize), Animation>,
}

impl Animations {
    /// Whether an animation waits for its delay or runs: frames are needed.
    pub fn active(&self) -> bool {
        self.running
            .values()
            .any(|animation| animation.held.is_none() && animation.phase != Some(Phase::After))
    }

    /// Whether an animation changes what the document presents.
    pub fn any(&self) -> bool {
        !self.running.is_empty()
    }

    fn event(
        events: &mut Vec<TransitionEvent>,
        target: Target,
        name: &str,
        kind: &'static str,
        elapsed: f64,
    ) {
        events.push(TransitionEvent {
            node: target.0,
            pseudo: Styles::target_pseudo(target.1),
            kind,
            property: "",
            name: name.into(),
            elapsed,
        });
    }

    /// Starts, updates and cancels animations for new computed styles.
    pub fn update(
        &mut self,
        targets: &Styles,
        keyframes: &Keyframes,
        now: f64,
        events: &mut Vec<TransitionEvent>,
    ) {
        let mut seen = Vec::new();
        for (target, style) in targets.targets() {
            if style.display == Display::None {
                continue;
            }
            for (index, name) in style.animation_name.0.iter().enumerate() {
                if name.0 == "none" {
                    continue;
                }
                let Some(frames) = keyframes.get(&name.0) else {
                    continue;
                };
                let key = (target, index);
                let timing = timing(style, index);
                let properties = resolve(frames, style, targets.environment(), timing.easing);
                seen.push(key);
                match self.running.get_mut(&key) {
                    Some(animation) if animation.name == name.0 => {
                        // Pausing holds the local time; resuming continues
                        // from it.
                        match (animation.held, timing.paused) {
                            (None, true) => animation.held = Some(now - animation.start),
                            (Some(held), false) => {
                                animation.start = now - held;
                                animation.held = None;
                            }
                            _ => {}
                        }
                        animation.timing = timing;
                        animation.properties = properties;
                    }
                    _ => {
                        if let Some(old) = self.running.remove(&key)
                            && old.phase == Some(Phase::Active)
                        {
                            Self::event(
                                events,
                                target,
                                &old.name,
                                "animationcancel",
                                old.local(now).max(0.0),
                            );
                        }
                        let held = timing.paused.then_some(0.0);
                        self.running.insert(
                            key,
                            Animation {
                                name: name.0.clone(),
                                timing,
                                properties,
                                start: now,
                                held,
                                phase: None,
                                iteration: 0,
                            },
                        );
                    }
                }
            }
        }
        let gone: Vec<_> = self
            .running
            .keys()
            .copied()
            .filter(|key| !seen.contains(key))
            .collect();
        for key in gone {
            let old = self.running.remove(&key).expect("the animation ran");
            if old.phase == Some(Phase::Active) {
                Self::event(
                    events,
                    key.0,
                    &old.name,
                    "animationcancel",
                    old.local(now).max(0.0),
                );
            }
        }
        self.advance(now, events);
    }

    /// Moves animations to their phase at `now`, with their events.
    pub fn advance(&mut self, now: f64, events: &mut Vec<TransitionEvent>) {
        for ((target, _), animation) in &mut self.running {
            let phase = animation.phase(now);
            let duration = animation.timing.duration;
            let active = animation.timing.active_duration();
            let previous = animation.phase;
            if previous == Some(phase) {
                if phase == Phase::Active && duration > 0.0 {
                    let iteration = libm::floor(animation.local(now) / duration) as u64;
                    if iteration > animation.iteration {
                        animation.iteration = iteration;
                        Self::event(
                            events,
                            *target,
                            &animation.name,
                            "animationiteration",
                            iteration as f64 * duration,
                        );
                    }
                }
                continue;
            }
            animation.phase = Some(phase);
            match (previous, phase) {
                (_, Phase::Before) => {}
                (_, Phase::Active) => {
                    animation.iteration = libm::floor(animation.local(now) / duration) as u64;
                    Self::event(
                        events,
                        *target,
                        &animation.name,
                        "animationstart",
                        (-animation.timing.delay).max(0.0),
                    );
                }
                (Some(Phase::Active), Phase::After) => {
                    Self::event(events, *target, &animation.name, "animationend", active)
                }
                (_, Phase::After) => {
                    // Ended before a frame saw it run.
                    Self::event(
                        events,
                        *target,
                        &animation.name,
                        "animationstart",
                        (-animation.timing.delay).max(0.0),
                    );
                    Self::event(events, *target, &animation.name, "animationend", active);
                }
            }
        }
    }

    /// Writes the animated values at `now` into presented styles.
    pub fn apply(&self, styles: &mut Styles, now: f64) {
        for ((target, _), animation) in &self.running {
            let Some((_, progress)) = animation.progress(now) else {
                continue;
            };
            let Some(style) = styles.target_mut(*target) else {
                continue;
            };
            for (index, stops) in &animation.properties {
                (PROPERTIES[*index].write)(style, value_at(stops, progress));
            }
        }
    }
}

#[cfg(test)]
mod tests;
