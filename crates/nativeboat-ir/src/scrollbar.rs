//! Scrollbar geometry and paint: where a scroll container's buttons,
//! tracks, thumbs and corner sit, for the scrollbars native surfaces paint
//! over their content and those of scroll containers painted in place.

use crate::native::{Corner, DisplayItem, PathSegment, Rect, ScrollbarPaint, Scrollbars};
#[allow(unused_imports)]
use crate::prelude::*;
use nativeboat_css::Color;
use nativeboat_css::types::{FillRule, LengthPercentageAuto, Size};
use serde::{Deserialize, Serialize};

/// The thickness of classic scrollbars, and the length of their buttons.
pub const THICKNESS: f32 = 17.0;

/// The least length of the thumbs of classic and author-styled scrollbars.
pub const MINIMUM_THUMB: f32 = 20.0;

/// How far a press of an arrow button scrolls, as in Chromium
/// (`cc::kPixelsPerLineStep`).
pub const STEP: f32 = 40.0;

/// How long a pressed arrow button or track waits before it scrolls again,
/// and then between scrolls, in milliseconds, as in Chromium
/// (`kInitialAutoscrollTimerDelay` and `1 / kAutoscrollMultiplier`).
pub const REPEAT_DELAYS: [u32; 2] = [250, 50];

/// How far a press of the track scrolls a container whose visible length on
/// the axis is `length`: Chromium's page step on Windows, seven eighths of
/// it in whole pixels (`cc::ScrollUtils::CalculatePageStep`).
pub fn page_step(length: f32) -> f32 {
    // Truncation floors the positive lengths, without `std`.
    ((length * 0.875) as i32 as f32).max(1.0)
}

/// Chromium's Fluent scrollbar metrics in device-independent pixels: the
/// track's thickness, the thumb's, the thumb's least length, the buttons'
/// length, and the side of the square their arrows fit in, normally and
/// pressed, out of a button 18 long.
const FLUENT_THICKNESS: f32 = 15.0;
const FLUENT_THUMB: f32 = 9.0;
const FLUENT_MINIMUM_THUMB: f32 = 17.0;
const FLUENT_BUTTON: f32 = 18.0;
const FLUENT_ARROW: [i32; 2] = [9, 7];

/// How the browser draws scrollbars that no author CSS styles: as Windows'
/// classic ones, or as the Fluent ones Chromium draws on Windows since its
/// release 135.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Look {
    #[default]
    Classic,
    Fluent,
}

/// Rounds a positive length to the nearest whole number, halves up.
fn round(length: f32) -> f32 {
    (length + 0.5) as i32 as f32
}

/// A length in CSS pixels rounded to whole device pixels of size `pixel`,
/// as Chromium sizes scrollbar parts; `0` leaves it as it is.
fn device(length: f32, pixel: f32) -> f32 {
    if pixel > 0.0 {
        round(length / pixel) * pixel
    } else {
        length
    }
}

impl Look {
    /// The look of a Chromium major release on Windows.
    pub fn of_chromium(major: u32) -> Self {
        if major >= 135 {
            Look::Fluent
        } else {
            Look::Classic
        }
    }

    /// The scrollbars' thickness in CSS pixels, in whole device pixels of
    /// size `pixel` for Fluent ones.
    pub fn thickness(self, pixel: f32) -> f32 {
        match self {
            Look::Classic => THICKNESS,
            Look::Fluent => device(FLUENT_THICKNESS, pixel),
        }
    }

    /// The length of their arrow buttons along them.
    pub fn button_length(self, pixel: f32) -> f32 {
        match self {
            Look::Classic => THICKNESS,
            Look::Fluent => device(FLUENT_BUTTON, pixel),
        }
    }

    /// The least length of their thumbs.
    pub fn minimum_thumb(self, pixel: f32) -> f32 {
        match self {
            Look::Classic => MINIMUM_THUMB,
            Look::Fluent => device(FLUENT_MINIMUM_THUMB, pixel),
        }
    }
}

/// A part of a scrollbar: by axis, 0 horizontal and 1 vertical, and for
/// buttons whether it is the one at the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Track(usize),
    Thumb(usize),
    Button(usize, bool),
    Corner,
}

impl Hit {
    /// The axis of the scrollbar the part belongs to.
    pub fn axis(self) -> Option<usize> {
        match self {
            Hit::Track(axis) | Hit::Thumb(axis) | Hit::Button(axis, _) => Some(axis),
            Hit::Corner => None,
        }
    }
}

/// Whether a scrollbar shows on an axis, and how thick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Axis {
    /// Whether overflow allows a scrollbar on the axis.
    pub enabled: bool,
    /// Whether it shows when the content fits (`overflow: scroll`).
    pub force: bool,
    /// Its thickness, before it is limited to the box.
    pub thickness: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    /// The whole bar, its buttons included.
    pub whole: Rect,
    /// The buttons at its start and end, when it has them.
    pub buttons: Option<[Rect; 2]>,
    pub track: Rect,
    /// The thumb, empty when there is nothing to scroll.
    pub thumb: Rect,
    /// The largest scroll offset on the axis.
    pub maximum: f32,
    /// How far the thumb moves along the track.
    pub travel: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    pub bars: [Option<Bar>; 2],
    pub corner: Option<Rect>,
}

/// Whether content overflows an extent on one axis.
pub fn overflows(content: f32, extent: f32) -> bool {
    content > extent + 0.5
}

impl Bar {
    /// Whether the bar has a thumb: it has something to scroll.
    pub fn has_thumb(&self) -> bool {
        self.thumb.width > 0.0 && self.thumb.height > 0.0
    }
}

impl Geometry {
    /// How far a press of `part` scrolls, for the pointer at `pointer` along
    /// the part's axis, in a container whose visible length on that axis is
    /// `length`: a step for an arrow button, and a page toward the pointer
    /// for the track. `None` for the other parts, and for the track once
    /// the thumb has reached the pointer, as Chromium stops paging there.
    pub fn press_amount(&self, part: Hit, pointer: f32, length: f32) -> Option<f32> {
        match part {
            Hit::Button(_, end) => Some(if end { STEP } else { -STEP }),
            Hit::Track(axis) => {
                let bar = self.bars[axis]?;
                let (start, size) = if axis == 0 {
                    (bar.thumb.x, bar.thumb.width)
                } else {
                    (bar.thumb.y, bar.thumb.height)
                };
                if bar.has_thumb() && pointer >= start && pointer < start + size {
                    None
                } else if pointer < start {
                    Some(-page_step(length))
                } else {
                    Some(page_step(length))
                }
            }
            Hit::Thumb(_) | Hit::Corner => None,
        }
    }
}

fn contains(rect: Rect, x: f32, y: f32) -> bool {
    x >= rect.x && y >= rect.y && x < rect.x + rect.width && y < rect.y + rect.height
}

impl Geometry {
    /// The geometry of author-styled scrollbars, which have no buttons;
    /// `default` is the thickness of those whose width is automatic.
    pub fn new(
        style: &Scrollbars,
        view: (f32, f32),
        content: (f32, f32),
        scroll: (f32, f32),
        default: f32,
    ) -> Self {
        let axes = core::array::from_fn(|axis| {
            let bar = &style.axes[axis];
            let cross = if axis == 0 { view.1 } else { view.0 };
            Axis {
                enabled: bar.enabled,
                force: bar.force,
                thickness: match &bar.thickness {
                    Size::Length(value) => value.resolve(cross),
                    _ => default,
                },
            }
        });
        Self::with(axes, (view, view), content, scroll, (0.0, MINIMUM_THUMB))
    }

    /// The geometry of scrollbars over a box of size `view` whose content
    /// scrolls in an area of size `visible`, with buttons `buttons` long at
    /// each end of their tracks and thumbs `minimum` long at least.
    /// Positions are relative to the box.
    pub fn with(
        axes: [Axis; 2],
        (view, visible): ((f32, f32), (f32, f32)),
        content: (f32, f32),
        scroll: (f32, f32),
        (buttons, minimum): (f32, f32),
    ) -> Self {
        let thickness: [f32; 2] = core::array::from_fn(|axis| {
            let bar = &axes[axis];
            let (extent, content) = if axis == 0 {
                (view.0, content.0)
            } else {
                (view.1, content.1)
            };
            if !bar.enabled || (!bar.force && !overflows(content, extent)) {
                return 0.0;
            }
            let cross = if axis == 0 { view.1 } else { view.0 };
            bar.thickness.clamp(0.0, cross)
        });
        let bars = core::array::from_fn(|axis| {
            let cross = thickness[axis];
            if cross <= 0.0 {
                return None;
            }
            let (extent, content, offset, length) = if axis == 0 {
                (
                    visible.0,
                    content.0,
                    scroll.0,
                    (view.0 - thickness[1]).max(0.0),
                )
            } else {
                (
                    visible.1,
                    content.1,
                    scroll.1,
                    (view.1 - thickness[0]).max(0.0),
                )
            };
            // Buttons share a bar too short for both.
            let button = buttons.min(length / 2.0);
            let room = (length - 2.0 * button).max(0.0);
            // Content past the extent by less than half a pixel scrolls no
            // further, as Chromium snaps both to whole pixels first.
            let maximum = if overflows(content, extent) {
                content - extent
            } else {
                0.0
            };
            // A bar with nothing to scroll has no thumb, as Chromium's
            // ScrollbarTheme::ThumbLength gives disabled scrollbars none.
            let thumb_length = if maximum > 0.0 {
                (room * extent / content.max(extent).max(1.0))
                    .max(minimum)
                    .min(room)
            } else {
                0.0
            };
            let travel = if maximum > 0.0 {
                (room - thumb_length).max(0.0)
            } else {
                0.0
            };
            let position = if maximum > 0.0 {
                offset.clamp(0.0, maximum) / maximum * travel
            } else {
                0.0
            };
            let along = |start: f32, size: f32| {
                if axis == 0 {
                    Rect::new(start, view.1 - cross, size, cross)
                } else {
                    Rect::new(view.0 - cross, start, cross, size)
                }
            };
            Some(Bar {
                whole: along(0.0, length),
                buttons: (button > 0.0)
                    .then(|| [along(0.0, button), along(length - button, button)]),
                track: along(button, room),
                thumb: along(button + position, thumb_length),
                maximum,
                travel,
            })
        });
        let corner = (thickness[0] > 0.0 && thickness[1] > 0.0).then_some(Rect::new(
            view.0 - thickness[1],
            view.1 - thickness[0],
            thickness[1],
            thickness[0],
        ));
        Self { bars, corner }
    }

    /// The part at a point relative to the box.
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        if self.corner.is_some_and(|rect| contains(rect, x, y)) {
            return Some(Hit::Corner);
        }
        for (axis, bar) in self.bars.iter().enumerate() {
            let Some(bar) = bar else {
                continue;
            };
            if let Some([start, end]) = bar.buttons {
                if contains(start, x, y) {
                    return Some(Hit::Button(axis, false));
                }
                if contains(end, x, y) {
                    return Some(Hit::Button(axis, true));
                }
            }
            if contains(bar.track, x, y) {
                return Some(if contains(bar.thumb, x, y) {
                    Hit::Thumb(axis)
                } else {
                    Hit::Track(axis)
                });
            }
        }
        None
    }
}

/// Paints an author-styled part's box.
pub fn box_items(paint: &ScrollbarPaint, rect: Rect, items: &mut Vec<DisplayItem>) {
    if !paint.visible {
        return;
    }
    let margins = paint.margins.each_ref().map(|margin| match margin {
        LengthPercentageAuto::Length(length) => length.resolve(rect.width),
        LengthPercentageAuto::Auto => 0.0,
    });
    let rect = Rect::new(
        rect.x + margins[3],
        rect.y + margins[0],
        (rect.width - margins[1] - margins[3]).max(0.0),
        (rect.height - margins[0] - margins[2]).max(0.0),
    );
    let radii = paint.radii.each_ref().map(|radius| Corner {
        x: radius.horizontal.resolve(rect.width).max(0.0),
        y: radius.vertical.resolve(rect.height).max(0.0),
    });
    items.push(DisplayItem::Fill {
        rect,
        color: paint.background,
        radii,
    });
    if paint.widths.iter().any(|width| *width > 0.0) {
        items.push(DisplayItem::Border {
            rect,
            widths: paint.widths,
            colors: paint.colors,
            styles: paint.borders,
            radii,
        });
    }
}

/// The state of a part: 0 normal, 1 hovered, 2 pressed.
fn state(part: Hit, hover: Option<Hit>, pressed: Option<Hit>) -> usize {
    if pressed == Some(part) {
        2
    } else if hover == Some(part) {
        1
    } else {
        0
    }
}

/// Paints author-styled scrollbars: each bar, its track and thumb, and the
/// corner, as the pointer hovers or presses them.
pub fn styled_items(
    style: &Scrollbars,
    geometry: &Geometry,
    hover: Option<Hit>,
    pressed: Option<Hit>,
    items: &mut Vec<DisplayItem>,
) {
    for (axis, bar) in geometry.bars.iter().enumerate() {
        let Some(bar) = bar else {
            continue;
        };
        let on = |hit: Option<Hit>| hit.is_some_and(|hit| hit.axis() == Some(axis));
        let whole = if on(pressed) {
            2
        } else if on(hover) {
            1
        } else {
            0
        };
        let style = &style.axes[axis];
        box_items(&style.bar.0[whole], bar.whole, items);
        let track = state(Hit::Track(axis), hover, pressed);
        box_items(&style.track.0[track], bar.track, items);
        if bar.has_thumb() {
            let thumb = state(Hit::Thumb(axis), hover, pressed);
            box_items(&style.thumb.0[thumb], bar.thumb, items);
        }
    }
    if let Some(corner) = geometry.corner {
        let index = state(Hit::Corner, hover, pressed);
        box_items(&style.corner.0[index], corner, items);
    }
}

/// Paints scrollbars that no author CSS styles as the browser draws them
/// in a look; `pixel` is the size of a device pixel, whose grid Fluent
/// thumbs and arrows fit (`0` when it is not known: CSS pixels).
pub fn platform_items(
    look: Look,
    geometry: &Geometry,
    scroll: (f32, f32),
    (hover, pressed): (Option<Hit>, Option<Hit>),
    pixel: f32,
    items: &mut Vec<DisplayItem>,
) {
    match look {
        Look::Classic => default_items(geometry, scroll, hover, pressed, items),
        Look::Fluent => fluent_items(geometry, hover, pressed, pixel, items),
    }
}

/// Paints scrollbars as Chromium draws them on Windows since release 135:
/// a light track, between buttons of its color, with a rounded gray thumb
/// narrower than it in its middle, and triangular arrows a step off their
/// buttons' centers, away from the thumb. The thumb and an arrow darken as
/// the pointer hovers or presses them, and a pressed arrow shrinks; arrows
/// do not dim where the content cannot scroll further. Their sizes and
/// places are whole device pixels, as Chromium's are.
fn fluent_items(
    geometry: &Geometry,
    hover: Option<Hit>,
    pressed: Option<Hit>,
    pixel: f32,
    items: &mut Vec<DisplayItem>,
) {
    let pixel = if pixel > 0.0 { pixel } else { 1.0 };
    let track = Color::rgb(0xFC, 0xFC, 0xFC);
    let shade = |part: Hit| {
        if state(part, hover, pressed) == 0 {
            Color::rgb(0x8B, 0x8B, 0x8B)
        } else {
            Color::rgb(0x63, 0x63, 0x63)
        }
    };
    let fill = |rect: Rect, color: Color, radius: f32, items: &mut Vec<DisplayItem>| {
        items.push(DisplayItem::Fill {
            rect,
            color,
            radii: [Corner::round(radius); 4],
        })
    };
    let devices = |length: f32| round(length / pixel) as i32;
    for (axis, bar) in geometry.bars.iter().enumerate() {
        let Some(bar) = bar else {
            continue;
        };
        fill(bar.whole, track, 0.0, items);
        // The thumb is as much narrower than the track on each side.
        let cross = if axis == 0 {
            bar.whole.height
        } else {
            bar.whole.width
        };
        let track_pixels = devices(cross);
        let thumb_pixels = devices(FLUENT_THUMB).min(track_pixels);
        let thumb_pixels = thumb_pixels - (track_pixels - thumb_pixels) % 2;
        let thickness = thumb_pixels as f32 * pixel;
        let inset = (cross - thickness) / 2.0;
        let rect = bar.thumb;
        let rect = if axis == 0 {
            Rect::new(rect.x, rect.y + inset, rect.width, thickness)
        } else {
            Rect::new(rect.x + inset, rect.y, thickness, rect.height)
        };
        if rect.width > 0.0 && rect.height > 0.0 {
            fill(rect, shade(Hit::Thumb(axis)), thickness / 2.0, items);
        }
        for (end, button) in bar.buttons.into_iter().flatten().enumerate() {
            let end = end == 1;
            let part = Hit::Button(axis, end);
            if let Some(segments) =
                fluent_arrow(button, axis, end, state(part, hover, pressed) == 2, pixel)
            {
                items.push(DisplayItem::Path {
                    segments,
                    transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                    fill: Some(shade(part)),
                    fill_rule: FillRule::NonZero,
                    stroke: None,
                });
            }
        }
    }
    if let Some(corner) = geometry.corner {
        fill(corner, track, 0.0, items);
    }
}

/// The triangle a Fluent arrow button shows, pointing along its axis toward
/// its end, as Chromium draws it without Windows 11's icon font: in a
/// square of whole device pixels in proportion to the button's length, at
/// its center, a pixel per 18 away from the thumb.
fn fluent_arrow(
    button: Rect,
    axis: usize,
    end: bool,
    pressed: bool,
    pixel: f32,
) -> Option<Vec<PathSegment>> {
    let width = round(button.width / pixel) as i32;
    let height = round(button.height / pixel) as i32;
    let (short, long) = (width.min(height), width.max(height));
    if short <= 0 {
        return None;
    }
    let arrow = FLUENT_ARROW[usize::from(pressed)];
    // The side rounds up, then to the button's parity to stay centered.
    let scaled = long as f32 * arrow as f32 / FLUENT_BUTTON;
    let mut side = scaled as i32;
    if (side as f32) < scaled {
        side += 1;
    }
    side += (short - side) % 2;
    let (box_width, box_height) = (side.min(width), side.min(height));
    let mut x = (width - box_width).div_euclid(2) as f32;
    let mut y = (height - box_height).div_euclid(2) as f32;
    let offset = round(long as f32 / FLUENT_BUTTON);
    let away = if end { offset } else { -offset };
    if axis == 0 {
        x += away;
    } else {
        y += away;
    }
    let (w, h) = (box_width as f32, box_height as f32);
    let (center_x, center_y) = (x + w / 2.0, y + h / 2.0);
    // Chromium's path: a base the box's side long, a tip half the side and
    // a pixel away, moved to sit in the box, then flipped about its center
    // to point backward.
    let points: [(f32, f32); 3] = if axis == 1 {
        let altitude = box_height / 2 + 1;
        let shift = (1 - altitude / 2) as f32;
        let points = [
            (x, y + h + shift),
            (x + w, y + h + shift),
            (x + w / 2.0, y + h - altitude as f32 + shift),
        ];
        if end {
            points.map(|(px, py)| (px, 2.0 * center_y - py))
        } else {
            points
        }
    } else {
        let altitude = box_width / 2 + 1;
        let shift = (altitude / 2) as f32;
        let points = [
            (x + shift, y),
            (x + shift, y + h),
            (x + altitude as f32 + shift, y + h - h / 2.0),
        ];
        if end {
            points
        } else {
            points.map(|(px, py)| (2.0 * center_x - px, py))
        }
    };
    let at = |(px, py): (f32, f32)| (button.x + px * pixel, button.y + py * pixel);
    let [first, second, third] = points.map(at);
    Some(vec![
        PathSegment::Move(first.0, first.1),
        PathSegment::Line(second.0, second.1),
        PathSegment::Line(third.0, third.1),
        PathSegment::Close,
    ])
}

/// Paints scrollbars as Chromium draws them on Windows before release 135,
/// when no author CSS styles them: a light track between two arrow buttons, a gray thumb inset from
/// its sides, which darkens as the pointer hovers or presses it, and a
/// corner between two bars. Arrows dim where the content cannot scroll
/// further.
fn default_items(
    geometry: &Geometry,
    scroll: (f32, f32),
    hover: Option<Hit>,
    pressed: Option<Hit>,
    items: &mut Vec<DisplayItem>,
) {
    let fill = |rect: Rect, color: Color, items: &mut Vec<DisplayItem>| {
        items.push(DisplayItem::Fill {
            rect,
            color,
            radii: [Corner::SQUARE; 4],
        })
    };
    for (axis, bar) in geometry.bars.iter().enumerate() {
        let Some(bar) = bar else {
            continue;
        };
        fill(bar.whole, Color::rgb(241, 241, 241), items);
        let offset = if axis == 0 { scroll.0 } else { scroll.1 };
        for (end, button) in bar.buttons.into_iter().flatten().enumerate() {
            let end = end == 1;
            let part = Hit::Button(axis, end);
            let enabled = if end {
                offset < bar.maximum - 0.5
            } else {
                offset > 0.5
            };
            let (background, arrow) = match (enabled, state(part, hover, pressed)) {
                (false, _) => (None, Color::rgb(163, 163, 163)),
                (true, 2) => (Some(Color::rgb(120, 120, 120)), Color::WHITE),
                (true, 1) => (Some(Color::rgb(210, 210, 210)), Color::rgb(80, 80, 80)),
                (true, _) => (None, Color::rgb(80, 80, 80)),
            };
            if let Some(background) = background {
                fill(button, background, items);
            }
            // A triangle pointing along the axis, centered in the button.
            let size = button.width.min(button.height);
            // Whole pixels keep the arrow's edges sharp.
            let half = ((size * 0.2) as i32) as f32;
            let depth = half;
            let (cx, cy) = (
                button.x + button.width / 2.0,
                button.y + button.height / 2.0,
            );
            let sign = if end { 1.0 } else { -1.0 };
            let (tip, base) = (sign * depth / 2.0, -sign * depth / 2.0);
            let points = if axis == 0 {
                [
                    (cx + tip, cy),
                    (cx + base, cy - half),
                    (cx + base, cy + half),
                ]
            } else {
                [
                    (cx, cy + tip),
                    (cx - half, cy + base),
                    (cx + half, cy + base),
                ]
            };
            items.push(DisplayItem::Path {
                segments: vec![
                    PathSegment::Move(points[0].0, points[0].1),
                    PathSegment::Line(points[1].0, points[1].1),
                    PathSegment::Line(points[2].0, points[2].1),
                    PathSegment::Close,
                ],
                transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                fill: Some(arrow),
                fill_rule: FillRule::NonZero,
                stroke: None,
            });
        }
        if !bar.has_thumb() {
            continue;
        }
        let thumb = match state(Hit::Thumb(axis), hover, pressed) {
            2 => Color::rgb(120, 120, 120),
            1 => Color::rgb(168, 168, 168),
            _ => Color::rgb(193, 193, 193),
        };
        let inset = 2.0f32.min(bar.thumb.width.min(bar.thumb.height) / 4.0);
        let rect = bar.thumb;
        let rect = if axis == 0 {
            Rect::new(
                rect.x,
                rect.y + inset,
                rect.width,
                rect.height - 2.0 * inset,
            )
        } else {
            Rect::new(
                rect.x + inset,
                rect.y,
                rect.width - 2.0 * inset,
                rect.height,
            )
        };
        fill(rect, thumb, items);
    }
    if let Some(corner) = geometry.corner {
        fill(corner, Color::rgb(220, 220, 220), items);
    }
}

#[cfg(test)]
mod tests;
