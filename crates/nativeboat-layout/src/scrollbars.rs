//! The scrollbars of scroll containers painted in place, under transforms
//! that scale or rotate them: the pointer presses, drags and hovers them,
//! and the wheel scrolls the containers, as with native scroll surfaces.

use crate::paint::{InPlace, Scene};
use nativeboat_dom::{Dom, NodeId};
use nativeboat_ir::scrollbar::Hit as Part;

/// The code a scrollbar part is kept by in the document.
pub fn code(part: Part) -> u8 {
    match part {
        Part::Track(axis) => axis as u8,
        Part::Thumb(axis) => 2 + axis as u8,
        Part::Button(axis, end) => 4 + 2 * axis as u8 + u8::from(end),
        Part::Corner => 8,
    }
}

/// The part a code stands for.
pub fn part(code: u8) -> Option<Part> {
    Some(match code {
        0 | 1 => Part::Track(usize::from(code)),
        2 | 3 => Part::Thumb(usize::from(code - 2)),
        4..=7 => Part::Button(usize::from((code - 4) / 2), code % 2 == 1),
        8 => Part::Corner,
        _ => return None,
    })
}

/// The scroll container painted in place whose scrollbar is at a point in
/// window coordinates, and the part there: the scrollbar shows above the
/// container's content, so the topmost element at the point is the
/// container or inside it.
pub fn scrollbar_at(scene: &Scene, dom: &Dom, x: f32, y: f32) -> Option<(InPlace, Part)> {
    let (scroller, part) = scene.scrollbar_at(x, y)?;
    let target = scene.hit_test(x, y)?;
    dom.contains(scroller.node, target)
        .then_some((*scroller, part))
}

/// Scrolls a container painted in place by an amount, within its range;
/// returns whether it moved.
fn scroll_by(dom: &mut Dom, scroller: &InPlace, (dx, dy): (f32, f32)) -> bool {
    let (left, top) = dom.scroll(scroller.node);
    let to = (
        (left + dx).clamp(0.0, scroller.maximum.0),
        (top + dy).clamp(0.0, scroller.maximum.1),
    );
    if to == (left, top) {
        return false;
    }
    dom.set_scroll(scroller.node, to.0, to.1);
    true
}

/// A drag of a scrollbar's thumb.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dragging {
    node: NodeId,
    axis: usize,
    /// Where along the axis the pointer pressed the thumb.
    pointer: f32,
    /// The scroll offset then.
    scroll: f32,
}

impl Dragging {
    /// The scroll container whose thumb is dragged.
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// Scrolls the container so that the thumb follows the pointer.
    pub fn drag(&self, scene: &Scene, dom: &mut Dom, x: f32, y: f32) {
        let Some(scroller) = scene.in_place(self.node) else {
            return;
        };
        let (Some(bar), Some((x, y))) = (scroller.bars.bars[self.axis], scroller.local(x, y))
        else {
            return;
        };
        let (point, maximum) = if self.axis == 0 {
            (x, scroller.maximum.0)
        } else {
            (y, scroller.maximum.1)
        };
        let target = if bar.travel > 0.0 {
            self.scroll + (point - self.pointer) / bar.travel * maximum
        } else {
            self.scroll
        };
        let (left, top) = dom.scroll(self.node);
        let delta = if self.axis == 0 {
            (target - left, 0.0)
        } else {
            (0.0, target - top)
        };
        scroll_by(dom, scroller, delta);
    }
}

/// Scrolls a container painted in place as a press of its scrollbar's
/// `part` at a point in its own coordinates does: an arrow button a step,
/// the track a page toward the point until the thumb reaches it. Returns
/// whether the part is one that scrolls so.
fn press_part(dom: &mut Dom, scroller: &InPlace, part: Part, (x, y): (f32, f32)) -> bool {
    let (Part::Button(axis, _) | Part::Track(axis)) = part else {
        return false;
    };
    let (point, length) = if axis == 0 {
        (x, scroller.view.0)
    } else {
        (y, scroller.view.1)
    };
    if let Some(amount) = scroller.bars.press_amount(part, point, length) {
        let delta = if axis == 0 {
            (amount, 0.0)
        } else {
            (0.0, amount)
        };
        scroll_by(dom, scroller, delta);
    }
    true
}

/// Presses the scrollbar part at a point: an arrow button scrolls a step,
/// the track a page toward the point and the thumb starts a drag. `None`
/// when no scrollbar is there.
pub fn press(scene: &Scene, dom: &mut Dom, x: f32, y: f32) -> Option<Option<Dragging>> {
    let (scroller, part) = scrollbar_at(scene, dom, x, y)?;
    let (x, y) = scroller.local(x, y)?;
    let node = scroller.node;
    let pressed = Some((node, code(part)));
    dom.set_scrollbar_parts((pressed, pressed));
    if let Part::Thumb(axis) = part {
        let (left, top) = dom.scroll(node);
        let (pointer, scroll) = if axis == 0 { (x, left) } else { (y, top) };
        return Some(Some(Dragging {
            node,
            axis,
            pointer,
            scroll,
        }));
    }
    press_part(dom, &scroller, part, (x, y));
    Some(None)
}

/// Scrolls again for the arrow button or track held down, while the pointer,
/// at a point in window coordinates, stays over it, as Chromium's
/// scrollbars repeat their presses. Returns whether one is held.
pub fn repeat(scene: &Scene, dom: &mut Dom, x: f32, y: f32) -> bool {
    let (over, pressed) = dom.scrollbar_parts();
    let Some((node, code)) = pressed else {
        return false;
    };
    let (Some(part), Some(scroller)) = (part(code), scene.in_place(node)) else {
        return false;
    };
    let scroller = *scroller;
    if over != pressed {
        return !matches!(part, Part::Thumb(_) | Part::Corner);
    }
    match scroller.local(x, y) {
        Some(point) => press_part(dom, &scroller, part, point),
        None => false,
    }
}

/// Shows which scrollbar part the pointer is over; returns whether it is
/// over one.
pub fn hover(scene: &Scene, dom: &mut Dom, x: f32, y: f32) -> bool {
    let over = scrollbar_at(scene, dom, x, y).map(|(scroller, part)| (scroller.node, code(part)));
    let (_, pressed) = dom.scrollbar_parts();
    dom.set_scrollbar_parts((over, pressed));
    over.is_some()
}

/// Ends a press of a scrollbar part.
pub fn release(dom: &mut Dom) {
    let (over, _) = dom.scrollbar_parts();
    dom.set_scrollbar_parts((over, None));
}

/// Scrolls the innermost scroll container painted in place around an
/// element that can scroll the way the wheel turned, by the turn or a page
/// at most. Returns `false` when a native scroll surface comes first,
/// which scrolls itself, or none can scroll.
pub fn wheel(scene: &Scene, dom: &mut Dom, target: NodeId, (dx, dy): (f32, f32)) -> bool {
    let mut node = Some(target);
    while let Some(current) = node {
        if scene.has_surface(current) {
            return false;
        }
        if let Some(scroller) = scene.in_place(current) {
            let (width, height) = scroller.view;
            let delta = (dx.clamp(-width, width), dy.clamp(-height, height));
            if scroll_by(dom, scroller, delta) {
                return true;
            }
        }
        node = dom.parent_element(current);
    }
    false
}

#[cfg(test)]
mod tests;
