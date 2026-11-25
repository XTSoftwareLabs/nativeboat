//! Scrollbar parts use their host's variables and their own dynamic state.

use nativeboat_css::selector::state;
use nativeboat_css::types::{Display, Overflow, ScrollbarWidth, Size, Visibility};
use nativeboat_css::{Color, ComputedStyle, PseudoElement, RuleMap, StyleEnvironment};
use nativeboat_dom::ElementRef;
use nativeboat_ir::{ScrollbarAxis, ScrollbarPaint, ScrollbarPart, Scrollbars};

fn paint(style: &ComputedStyle) -> ScrollbarPaint {
    let alpha = |mut color: Color| {
        color.a *= style.opacity;
        color
    };
    ScrollbarPaint {
        visible: style.display != Display::None && style.visibility == Visibility::Visible,
        background: alpha(style.background_color),
        widths: [
            style.border_width.top.0,
            style.border_width.right.0,
            style.border_width.bottom.0,
            style.border_width.left.0,
        ],
        colors: [
            alpha(style.border_color.top),
            alpha(style.border_color.right),
            alpha(style.border_color.bottom),
            alpha(style.border_color.left),
        ],
        borders: [
            style.border_style.top,
            style.border_style.right,
            style.border_style.bottom,
            style.border_style.left,
        ],
        radii: [
            style.border_radius.top_left.clone(),
            style.border_radius.top_right.clone(),
            style.border_radius.bottom_right.clone(),
            style.border_radius.bottom_left.clone(),
        ],
        margins: [
            style.margin.top.clone(),
            style.margin.right.clone(),
            style.margin.bottom.clone(),
            style.margin.left.clone(),
        ],
    }
}

/// Only called for a viewport or a box whose overflow can make a surface.
pub(super) fn compute(
    element: &ElementRef<'_>,
    rules: &RuleMap<'_>,
    host: &ComputedStyle,
    environment: &StyleEnvironment,
    viewport: bool,
) -> Option<Scrollbars> {
    let standard = host.scrollbar_width != ScrollbarWidth::Auto || host.scrollbar_color.0.is_some();
    let defaults = ComputedStyle::default();
    let mut matched = standard;
    let part = |pseudo: PseudoElement, axis: u32, fallback: Color, matched: &mut bool| {
        let base = if standard {
            None
        } else {
            rules.compute_pseudo_style_state(element, &pseudo, axis, host, environment)
        };
        *matched |= base.is_some();
        let normal = base.clone().unwrap_or_else(|| ComputedStyle {
            background_color: fallback,
            ..defaults.clone()
        });
        let states = [0, state::HOVER, state::HOVER | state::ACTIVE].map(|flags| {
            let computed = if standard {
                None
            } else {
                rules.compute_pseudo_style_state(element, &pseudo, axis | flags, host, environment)
            };
            *matched |= computed.is_some();
            paint(computed.as_ref().unwrap_or(&normal))
        });
        (ScrollbarPart(states), base)
    };
    let (thumb, track) = host.scrollbar_color.0.unwrap_or((
        Color {
            a: 0.45,
            ..environment.system.canvas_text
        },
        environment.system.canvas,
    ));
    let axes = core::array::from_fn(|index| {
        let axis = if index == 0 {
            state::HORIZONTAL
        } else {
            state::VERTICAL
        };
        let overflow = if index == 0 {
            host.overflow_x
        } else {
            host.overflow_y
        };
        let (bar, base) = part(PseudoElement::Scrollbar, axis, track, &mut matched);
        let thickness = match host.scrollbar_width {
            ScrollbarWidth::None => Size::Length(nativeboat_css::values::LengthPercentage::Px(0.0)),
            ScrollbarWidth::Thin => Size::Length(nativeboat_css::values::LengthPercentage::Px(8.0)),
            ScrollbarWidth::Auto => base.as_ref().map_or(Size::Auto, |style| {
                if style.display == Display::None {
                    Size::Length(nativeboat_css::values::LengthPercentage::Px(0.0))
                } else if index == 0 {
                    style.height.clone()
                } else {
                    style.width.clone()
                }
            }),
        };
        ScrollbarAxis {
            enabled: bar.0[0].visible
                && (matches!(overflow, Overflow::Auto | Overflow::Scroll)
                    || (viewport && overflow == Overflow::Visible)),
            force: overflow == Overflow::Scroll,
            thickness,
            bar,
            track: part(
                PseudoElement::ScrollbarTrack,
                axis,
                Color::TRANSPARENT,
                &mut matched,
            )
            .0,
            thumb: part(PseudoElement::ScrollbarThumb, axis, thumb, &mut matched).0,
        }
    });
    let corner = part(PseudoElement::ScrollbarCorner, 0, track, &mut matched).0;
    matched.then_some(Scrollbars {
        axes,
        corner,
        platform: None,
    })
}
