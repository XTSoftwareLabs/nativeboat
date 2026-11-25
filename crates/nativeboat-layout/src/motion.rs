//! CSS transitions: when a document's computed styles change, the styles it
//! presents move from the old values to the new ones over time. Elements
//! and their `::before`, `::after` and `::placeholder` pseudo-elements
//! transition alike.

mod animations;
mod values;

pub use animations::{Keyframes, keyframes};

use crate::prelude::*;
use crate::styles::Styles;
use alloc::collections::BTreeMap;
use nativeboat_css::shorthand::longhands_of;
use nativeboat_css::types::{Display, Easing};
use nativeboat_css::{ComputedStyle, PropertyId};
use nativeboat_dom::{Dom, NodeId};
use values::{PROPERTIES, Value};

/// What a transition animates: an element (slot 0) or one of its
/// pseudo-elements, as [`Styles::target`] numbers them.
pub type Target = (NodeId, u8);

/// A transition or animation event for the document's scripts.
#[derive(Debug, Clone, PartialEq)]
pub struct TransitionEvent {
    /// The element the event is fired at.
    pub node: NodeId,
    /// The pseudo-element that transitioned, such as `::before`, or empty.
    pub pseudo: &'static str,
    /// `transitionrun`, `transitionstart`, `transitionend`,
    /// `transitioncancel`, `animationstart`, `animationiteration`,
    /// `animationend` or `animationcancel`.
    pub kind: &'static str,
    /// The transitioning longhand; empty for animations.
    pub property: &'static str,
    /// The animation's name; empty for transitions.
    pub name: String,
    /// Seconds of the transition or animation that had run.
    pub elapsed: f64,
}

#[derive(Debug)]
struct Transition {
    from: Value,
    to: Value,
    reverse: Value,
    factor: f32,
    start: f64,
    duration: f64,
    initial_elapsed: f64,
    easing: Easing,
    started: bool,
}

impl Transition {
    fn elapsed(&self, now: f64) -> f64 {
        (now - self.start).clamp(0.0, self.duration)
    }

    fn progress(&self, now: f64) -> f32 {
        if now < self.start {
            return 0.0;
        }
        self.easing
            .apply((self.elapsed(now) / self.duration.max(f64::EPSILON)) as f32)
    }

    fn value(&self, now: f64) -> Value {
        self.from
            .interpolate(&self.to, self.progress(now))
            .expect("transition endpoints are interpolatable")
    }
}

fn timing(style: &ComputedStyle, id: PropertyId) -> Option<(f64, f64, Easing)> {
    let index = style.transition_property.0.iter().rposition(|name| {
        let name = name.0.to_ascii_lowercase();
        name == "all"
            || name == id.name()
            || longhands_of(&name).is_some_and(|ids| ids.contains(&id))
    })?;
    Some((
        f64::from(style.transition_duration.at(index)?.0),
        f64::from(style.transition_delay.at(index)?.0),
        *style.transition_timing_function.at(index)?,
    ))
}

/// The transitions and animations of a document. Times are in seconds.
#[derive(Debug, Default)]
pub struct Motion {
    targets: Option<Rc<Styles>>,
    running: BTreeMap<(Target, usize), Transition>,
    animations: animations::Animations,
    keyframes: Rc<Keyframes>,
    /// Events since they were last taken, in order.
    pub events: Vec<TransitionEvent>,
}

impl Motion {
    /// The computed styles transitions run toward.
    pub fn targets(&self) -> Option<&Rc<Styles>> {
        self.targets.as_ref()
    }

    /// Uses the `@keyframes` rules of the document's style sheets from
    /// the next style change on.
    pub fn set_keyframes(&mut self, keyframes: Rc<Keyframes>) {
        self.keyframes = keyframes;
    }

    /// Whether a transition or animation is running or waiting for its
    /// delay, so the document needs frames.
    pub fn active(&self) -> bool {
        !self.running.is_empty() || self.animations.active()
    }

    /// Whether the presented styles differ from the computed ones: also
    /// animations that are paused or hold their end values.
    pub fn affects(&self) -> bool {
        !self.running.is_empty() || self.animations.any()
    }

    /// Whether transitions run or events wait to be taken.
    pub fn has_work(&self) -> bool {
        self.active() || !self.events.is_empty()
    }

    fn event(&mut self, key: (Target, usize), kind: &'static str, elapsed: f64) {
        self.events.push(TransitionEvent {
            node: key.0.0,
            pseudo: Styles::target_pseudo(key.0.1),
            property: PROPERTIES[key.1].id.name(),
            name: String::new(),
            kind,
            elapsed,
        });
    }

    fn advance(&mut self, now: f64) {
        let mut events = Vec::new();
        self.running.retain(|key, transition| {
            if !transition.started && now >= transition.start {
                transition.started = true;
                events.push((*key, "transitionstart", transition.initial_elapsed));
            }
            if now >= transition.start + transition.duration {
                events.push((*key, "transitionend", transition.duration));
                false
            } else {
                true
            }
        });
        for (key, kind, elapsed) in events {
            self.event(key, kind, elapsed);
        }
    }

    /// Starts, retargets and cancels transitions for new computed styles,
    /// from the values presented at `now`.
    pub fn update(&mut self, targets: impl Into<Rc<Styles>>, now: f64) {
        let targets = targets.into();
        self.advance(now);
        let previous = self.targets.take();
        if previous
            .as_ref()
            .is_some_and(|previous| Rc::ptr_eq(previous, &targets))
        {
            self.targets = previous;
            return;
        }
        self.animations
            .update(&targets, &self.keyframes, now, &mut self.events);
        let Some(previous) = previous else {
            self.targets = Some(targets);
            return;
        };
        let removed: Vec<_> = self
            .running
            .keys()
            .copied()
            .filter(|(target, _)| {
                targets
                    .target(*target)
                    .is_none_or(|style| style.display == Display::None)
            })
            .collect();
        for key in removed {
            let old = self
                .running
                .remove(&key)
                .expect("the transition was running");
            self.event(key, "transitioncancel", old.elapsed(now));
        }
        for (target, after) in targets.targets() {
            // Most elements have no transitions; avoid comparing their values.
            let enabled = after.transition_duration.0.iter().any(|time| time.0 > 0.0)
                || after.transition_delay.0.iter().any(|time| time.0 > 0.0);
            if !enabled && !self.running.keys().any(|(running, _)| *running == target) {
                continue;
            }
            let Some(before) = previous
                .target(target)
                .filter(|style| style.display != Display::None && after.display != Display::None)
            else {
                continue;
            };
            for (index, property) in PROPERTIES.iter().enumerate() {
                let key = (target, index);
                let end = (property.read)(after);
                let timing = timing(after, property.id);
                if self
                    .running
                    .get(&key)
                    .is_some_and(|old| old.to == end && timing.is_some())
                {
                    continue;
                }
                let old = self.running.remove(&key);
                let from = old
                    .as_ref()
                    .map_or_else(|| (property.read)(before), |old| old.value(now));
                if let Some(old) = &old {
                    self.event(key, "transitioncancel", old.elapsed(now));
                }
                let Some((mut duration, mut delay, easing)) = timing else {
                    continue;
                };
                if from == end || from.interpolate(&end, 0.5).is_none() || duration + delay <= 0.0 {
                    continue;
                }
                let (reverse, factor) = match old {
                    Some(old) if old.reverse == end => {
                        let factor = (old.progress(now) * old.factor + 1.0 - old.factor)
                            .abs()
                            .clamp(0.0, 1.0);
                        duration *= f64::from(factor);
                        if delay < 0.0 {
                            delay *= f64::from(factor);
                        }
                        (old.to, factor)
                    }
                    _ => (from.clone(), 1.0),
                };
                let initial_elapsed = (-delay).clamp(0.0, duration);
                self.event(key, "transitionrun", initial_elapsed);
                self.running.insert(
                    key,
                    Transition {
                        from,
                        to: end,
                        reverse,
                        factor,
                        start: now + delay,
                        duration,
                        initial_elapsed,
                        easing,
                        started: false,
                    },
                );
            }
        }
        self.targets = Some(targets);
        self.advance(now);
    }

    /// The styles presented at `now`: the targets with the running
    /// transitions' current values.
    pub fn sample(&mut self, now: f64) -> Styles {
        self.advance(now);
        self.animations.advance(now, &mut self.events);
        let mut styles = self.targets.as_deref().cloned().unwrap_or_default();
        // Transitions override animations, as in the cascade.
        self.animations.apply(&mut styles, now);
        for ((target, index), transition) in &self.running {
            if let Some(style) = styles.target_mut(*target) {
                (PROPERTIES[*index].write)(style, transition.value(now));
            }
        }
        styles
    }

    /// The styles presented at `now`, as [`Motion::sample`] gives them,
    /// where the descendants and pseudo-elements of an element that inherit
    /// a transitioning value present it too, as browsers inherit animated
    /// values.
    pub fn sample_inherited(&mut self, now: f64, dom: &Dom) -> Styles {
        let mut styles = self.sample(now);
        for (((node, slot), index), transition) in &self.running {
            if *slot != 0 || !PROPERTIES[*index].id.inherited() {
                continue;
            }
            let value = transition.value(now);
            inherit(
                &mut styles,
                dom,
                &self.running,
                (*node, *index),
                &transition.to,
                &value,
            );
        }
        styles
    }
}

/// Presents `value`, the value of an element's transitioning inherited
/// property, on its pseudo-elements and the descendants that inherit it:
/// those whose own value is the transition's `end`, and that run no
/// transition of the property themselves.
fn inherit(
    styles: &mut Styles,
    dom: &Dom,
    running: &BTreeMap<(Target, usize), Transition>,
    (node, index): (NodeId, usize),
    end: &Value,
    value: &Value,
) {
    let property = &PROPERTIES[index];
    let inherits = |styles: &Styles, target: Target| {
        !running.contains_key(&(target, index))
            && styles
                .target(target)
                .is_some_and(|style| (property.read)(style) == *end)
    };
    let mut pending = vec![node];
    while let Some(element) = pending.pop() {
        for slot in 1..=3 {
            if inherits(styles, (element, slot))
                && let Some(style) = styles.target_mut((element, slot))
            {
                (property.write)(style, value.clone());
            }
        }
        for child in dom.flat_child_elements(element) {
            if inherits(styles, (child, 0))
                && let Some(style) = styles.target_mut((child, 0))
            {
                (property.write)(style, value.clone());
                pending.push(child);
            }
        }
    }
}

#[cfg(test)]
mod tests;
