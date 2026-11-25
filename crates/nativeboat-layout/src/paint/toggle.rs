//! Checkboxes and radio buttons as Chromium's native theme draws them: a
//! 13-pixel box, or the largest square in the control's box, with a light
//! border that fills with the accent color when checked.

use crate::prelude::*;
use nativeboat_css::Color;
use nativeboat_css::types::{BorderStyle, FillRule, LineCap, LineJoin};
use nativeboat_ir::{Corner, DisplayItem, PathSegment, Rect, Stroke};

/// What decides a toggle's look.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Look {
    /// A radio button rather than a checkbox.
    pub radio: bool,
    pub checked: bool,
    pub indeterminate: bool,
    pub disabled: bool,
    pub hovered: bool,
    pub pressed: bool,
    /// Whether the control has focus the page shows, as from the keyboard.
    pub focus_visible: bool,
    /// `accent-color`, when it names one.
    pub accent: Option<Color>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Scheme {
    Light,
    Dark,
}

/// Chromium's `kMinimumVisibleContrastRatio`.
const MINIMUM_VISIBLE_CONTRAST: f32 = 3.0;

const fn gray(level: u8, alpha: f32) -> Color {
    Color::rgba(level, level, level, alpha)
}

fn background(scheme: Scheme, disabled: bool) -> Color {
    match (scheme, disabled) {
        (Scheme::Light, false) => gray(0xFF, 1.0),
        (Scheme::Light, true) => gray(0xFF, 0x99 as f32 / 255.0),
        (Scheme::Dark, _) => gray(0x3B, 1.0),
    }
}

fn border(scheme: Scheme, look: &Look) -> Color {
    match scheme {
        Scheme::Light if look.disabled => gray(0x76, 0x4D as f32 / 255.0),
        Scheme::Light if look.pressed => gray(0x8D, 1.0),
        Scheme::Light if look.hovered => gray(0x4F, 1.0),
        Scheme::Light => gray(0x76, 1.0),
        Scheme::Dark if look.disabled => gray(0x62, 1.0),
        Scheme::Dark if look.pressed => gray(0x6E, 1.0),
        Scheme::Dark if look.hovered => gray(0xAC, 1.0),
        Scheme::Dark => gray(0x85, 1.0),
    }
}

fn accent(scheme: Scheme, look: &Look) -> Color {
    if look.disabled {
        return match scheme {
            Scheme::Light => gray(0x76, 0x4D as f32 / 255.0),
            Scheme::Dark => gray(0x75, 1.0),
        };
    }
    if let Some(color) = look.accent {
        // Chromium shades an author's accent by a fixed step of HSL
        // lightness: darker when hovered and lighter when pressed in light
        // pages, the other way round in dark ones.
        let step = match (scheme, look.pressed, look.hovered) {
            (_, false, false) => 0.0,
            (Scheme::Light, true, _) | (Scheme::Dark, false, true) => 0.11,
            (Scheme::Light, false, true) | (Scheme::Dark, true, _) => -0.11,
        };
        return lighten(color, step);
    }
    let [r, g, b] = match (scheme, look.pressed, look.hovered) {
        (Scheme::Light, true, _) => [0x37, 0x93, 0xFF],
        (Scheme::Light, false, true) => [0x00, 0x5C, 0xC8],
        (Scheme::Light, false, false) => [0x00, 0x75, 0xFF],
        (Scheme::Dark, true, _) => [0x61, 0xA9, 0xFF],
        (Scheme::Dark, false, true) => [0xD1, 0xE6, 0xFF],
        (Scheme::Dark, false, false) => [0x99, 0xC8, 0xFF],
    };
    Color::rgb(r, g, b)
}

/// The color with its HSL lightness moved by `step`, kept within 0 and 1.
fn lighten(color: Color, step: f32) -> Color {
    if step == 0.0 {
        return color;
    }
    let [r, g, b] = [color.r, color.g, color.b].map(|channel| f32::from(channel) / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let lightness = (max + min) / 2.0;
    let delta = max - min;
    let (hue, saturation) = if delta == 0.0 {
        (0.0, 0.0)
    } else {
        let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs());
        let hue = if max == r {
            let hue = (g - b) / delta;
            if hue < 0.0 { hue + 6.0 } else { hue }
        } else if max == g {
            (b - r) / delta + 2.0
        } else {
            (r - g) / delta + 4.0
        };
        (hue / 6.0, saturation)
    };
    let lightness = (lightness + step).clamp(0.0, 1.0);
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let sector = hue * 6.0;
    let x = chroma * (1.0 - (sector - 2.0 * crate::math::floor(sector / 2.0) - 1.0).abs());
    let (r, g, b) = match sector as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let base = lightness - chroma / 2.0;
    let channel = |value: f32| crate::math::round((value + base) * 255.0).clamp(0.0, 255.0) as u8;
    Color::rgba(channel(r), channel(g), channel(b), color.a)
}

fn luminance(color: Color) -> f32 {
    let linear = |channel: u8| {
        let value = f32::from(channel) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            crate::math::pow((value + 0.055) / 1.055, 2.4)
        }
    };
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}

fn contrast(one: Color, other: Color) -> f32 {
    let (one, other) = (luminance(one), luminance(other));
    (one.max(other) + 0.05) / (one.min(other) + 0.05)
}

/// The scheme a checked control paints in: a light page's control whose
/// accent barely shows on white paints as in a dark page, and the other way
/// round, so that its mark stands out as Chromium keeps it.
fn scheme(look: &Look) -> Scheme {
    let Some(accent) = look.accent.filter(|_| look.checked && !look.disabled) else {
        return Scheme::Light;
    };
    let light = contrast(accent, background(Scheme::Light, false));
    let dark = contrast(accent, background(Scheme::Dark, false));
    if light < MINIMUM_VISIBLE_CONTRAST && light < dark {
        Scheme::Dark
    } else {
        Scheme::Light
    }
}

fn inset(rect: Rect, x: f32, y: f32) -> Rect {
    Rect::new(
        rect.x + x,
        rect.y + y,
        (rect.width - 2.0 * x).max(0.0),
        (rect.height - 2.0 * y).max(0.0),
    )
}

fn ring(rect: Rect, width: f32, color: Color, radius: f32) -> DisplayItem {
    DisplayItem::Border {
        rect,
        widths: [width; 4],
        colors: [color; 4],
        styles: [BorderStyle::Solid; 4],
        radii: [Corner::round(radius); 4],
    }
}

/// Paints a toggle into `rect`, the control's box.
pub(super) fn paint(items: &mut Vec<DisplayItem>, rect: Rect, look: &Look) {
    let size = rect.width.min(rect.height);
    if size <= 0.0 {
        return;
    }
    let rect = Rect::new(
        rect.x + (rect.width - size) / 2.0,
        rect.y + (rect.height - size) / 2.0,
        size,
        size,
    );
    let scheme = scheme(look);
    let radius = if look.radio { size / 2.0 } else { 2.0 };
    if size <= 2.0 {
        items.push(DisplayItem::Fill {
            rect,
            color: border(scheme, look),
            radii: [Corner::SQUARE; 4],
        });
        return;
    }
    let filled = !look.radio && look.checked && !look.indeterminate;
    items.push(DisplayItem::Fill {
        rect: inset(rect, 0.2, 0.2),
        color: background(scheme, look.disabled),
        radii: [Corner::round(radius); 4],
    });
    if !filled {
        // A checked radio button's ring takes the accent color.
        let color = if look.radio && look.checked {
            accent(scheme, look)
        } else {
            border(scheme, look)
        };
        items.push(ring(rect, 1.0, color, radius));
    }
    if look.radio {
        if look.checked {
            items.push(DisplayItem::Fill {
                rect: inset(rect, size * 0.2, size * 0.2),
                color: accent(scheme, look),
                radii: [Corner::round(size * 0.3); 4],
            });
        }
    } else if look.indeterminate {
        items.push(DisplayItem::Fill {
            rect: inset(rect, size * 2.5 / 13.0, size * 5.5 / 13.0),
            color: border(scheme, look),
            radii: [Corner::round(radius); 4],
        });
    } else if look.checked {
        items.push(DisplayItem::Fill {
            rect,
            color: accent(scheme, look),
            radii: [Corner::round(radius); 4],
        });
        let (x, y) = (rect.x, rect.y);
        items.push(DisplayItem::Path {
            segments: vec![
                PathSegment::Move(x + size * 0.2, y + size / 2.0),
                PathSegment::Line(x + size * 0.4, y + size * 0.7),
                PathSegment::Line(x + size * 0.8, y + size * 0.2),
            ],
            transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            fill: None,
            fill_rule: FillRule::NonZero,
            stroke: Some(Stroke {
                color: background(scheme, false),
                width: size * 0.16,
                cap: LineCap::Butt,
                join: LineJoin::Miter,
                dashes: Vec::new(),
                dash_offset: 0.0,
            }),
        });
    }
    if look.focus_visible {
        // Chromium's focus ring: dark around the box, with a light line
        // between them.
        let outer = inset(rect, -2.0, -2.0);
        items.push(ring(outer, 2.0, gray(0x10, 1.0), radius + 2.0));
        items.push(ring(
            inset(rect, -0.5, -0.5),
            0.5,
            gray(0xFF, 1.0),
            radius + 0.5,
        ));
    }
}

#[cfg(test)]
mod tests;
