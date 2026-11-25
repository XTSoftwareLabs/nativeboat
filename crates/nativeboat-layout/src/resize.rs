//! Resizing boxes by their grip: the user drags the corner of a box whose
//! `resize` allows it, and the box takes the size as its inline style, as
//! browsers resize it.

use crate::paint::Scene;
use crate::prelude::*;
use crate::styles::Styles;
use crate::tree::LayoutResult;
use nativeboat_css::types::{BoxSizing, Cursor, Size};
use nativeboat_dom::{Dom, NodeId};

/// The smallest size a drag gives a box, as in Chromium.
const MINIMUM: f32 = 15.0;

/// The box whose resize grip is at a point in window coordinates: the
/// grip shows above the box's content, so the topmost element at the point
/// is the box or inside it.
pub fn resizer_at(scene: &Scene, dom: &Dom, x: f32, y: f32) -> Option<NodeId> {
    let node = scene.resizer_at(x, y)?;
    let target = scene.hit_test(x, y)?;
    dom.contains(node, target).then_some(node)
}

/// The cursor over a box's resize grip.
pub fn cursor(styles: &Styles, node: NodeId) -> Cursor {
    match styles.get(node).map(|style| style.resize.axes()) {
        Some((true, false)) => Cursor::EwResize,
        Some((false, true)) => Cursor::NsResize,
        _ => Cursor::NwseResize,
    }
}

/// A drag of a resize grip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resizing {
    node: NodeId,
    /// Where the pointer was from the box's bottom-right corner when the
    /// drag began.
    grab: (f32, f32),
}

impl Resizing {
    /// Starts a drag where the pointer is pressed on a resize grip.
    pub fn start(scene: &Scene, dom: &Dom, x: f32, y: f32) -> Option<Resizing> {
        let node = resizer_at(scene, dom, x, y)?;
        let rect = scene.client_rect(node)?;
        Some(Resizing {
            node,
            grab: (x - (rect.x + rect.width), y - (rect.y + rect.height)),
        })
    }

    /// The box being resized.
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// Sizes the box so that its corner keeps its distance from the
    /// pointer: the inline style takes the width and the height its
    /// `resize` lets change, in whole pixels of its `box-sizing`, no smaller
    /// than its minimum size or 15 pixels. Form controls keep their margins
    /// as inline style too, as Chromium makes them explicit.
    pub fn drag(
        &self,
        scene: &Scene,
        styles: &Styles,
        layout: &LayoutResult,
        dom: &mut Dom,
        x: f32,
        y: f32,
    ) {
        let node = self.node;
        let (Some(rect), Some(style), Some(geometry)) = (
            scene.client_rect(node),
            styles.get(node),
            layout.boxes.get(&node),
        ) else {
            return;
        };
        let basis = dom
            .parent_element(node)
            .and_then(|parent| layout.boxes.get(&parent))
            .map_or((0.0, 0.0), |parent| {
                let content = parent.content_box();
                (content.width, content.height)
            });
        let minimum = |size: &Size, basis: f32| match size {
            Size::Length(length) => length.resolve(basis).max(MINIMUM),
            _ => MINIMUM,
        };
        let size = (geometry.rect.width, geometry.rect.height);
        let wanted = (
            (x - self.grab.0 - rect.x).max(minimum(&style.min_width, basis.0)),
            (y - self.grab.1 - rect.y).max(minimum(&style.min_height, basis.1)),
        );
        let content = style.box_sizing == BoxSizing::ContentBox;
        let edges = |start: usize, end: usize| {
            if content {
                geometry.border[start]
                    + geometry.border[end]
                    + geometry.padding[start]
                    + geometry.padding[end]
            } else {
                0.0
            }
        };
        let control = matches!(
            dom.local_name(node),
            Some("textarea" | "input" | "select" | "button")
        );
        let (horizontal, vertical) = style.resize.axes();
        let mut changes: Vec<(&str, f32)> = Vec::new();
        if horizontal && libm::roundf(wanted.0) != libm::roundf(size.0) {
            if control {
                changes.push(("margin-left", geometry.margin[3]));
                changes.push(("margin-right", geometry.margin[1]));
            }
            changes.push(("width", libm::roundf(wanted.0 - edges(1, 3))));
        }
        if vertical && libm::roundf(wanted.1) != libm::roundf(size.1) {
            if control {
                changes.push(("margin-top", geometry.margin[0]));
                changes.push(("margin-bottom", geometry.margin[2]));
            }
            changes.push(("height", libm::roundf(wanted.1 - edges(0, 2))));
        }
        if changes.is_empty() {
            return;
        }
        dom.update_inline_style(node, &mut |inline| {
            for (name, value) in &changes {
                inline.set(name, &format!("{value}px"), false);
            }
        });
    }
}

#[cfg(test)]
mod tests;
