//! Scrollbars painted over a surface's content: author-styled ones, and
//! the browser's own where no author CSS styles them. Positions are in
//! client CSS pixels.

use super::*;
#[allow(unused_imports)]
use crate::prelude::*;
use nativeboat_css::types::Size;
use nativeboat_ir::Scrollbars;
use nativeboat_ir::scrollbar::{Axis, Geometry, Hit, MINIMUM_THUMB, REPEAT_DELAYS, overflows};

/// The timer that repeats a held arrow button's or track's scrolling.
pub(super) const REPEAT_TIMER: usize = 0x5C80;

#[derive(Debug, Clone, Copy)]
struct Drag {
    axis: usize,
    pointer: f32,
    scroll: f32,
    travel: f32,
    maximum: f32,
}

#[derive(Default)]
pub(super) struct State {
    pub style: Option<Box<Scrollbars>>,
    hover: Option<Hit>,
    pressed: Option<Hit>,
    /// Where along its axis the pointer is while it holds an arrow button
    /// or the track down.
    pressed_at: f32,
    drag: Option<Drag>,
    /// For the window's root surface, the room its scrollbars take from
    /// the viewport: the horizontal bar's height and the vertical bar's
    /// width.
    pub room: Option<(f32, f32)>,
    /// Whether the page is laying out again for room its bars just took
    /// or gave back.
    settling: bool,
}

/// The thickness of a surface's scrollbar on an axis, 0 across and 1 down.
fn thickness(style: &Scrollbars, axis: usize, view: (f32, f32), scale: f32) -> f32 {
    if let Some(look) = style.platform {
        return look.thickness(1.0 / scale);
    }
    let cross = if axis == 0 { view.1 } else { view.0 };
    match &style.axes[axis].thickness {
        Size::Length(value) => value.resolve(cross),
        _ => {
            // SAFETY: reads a system metric without changing window or user
            // state.
            let metric = unsafe { GetSystemMetricsForDpi(SM_CXVSCROLL, px(96.0, scale) as u32) };
            metric as f32 / scale
        }
    }
}

/// Gives the window's root scrollbars room, as Chromium's viewport gives
/// its own: a bar where the content laid out overflows what the bars
/// already there leave, so a bar the other one makes needed comes with the
/// layout that bar's room causes. Returns whether the room changed.
pub(super) fn reserve(surface: &mut SurfaceData, scale: f32, zoom: f32) -> bool {
    let mut room = surface.scrollbar.style.as_ref().map(|style| {
        let view = view(surface.hwnd, scale, zoom);
        let (across, down) = surface.scrollbar.room.unwrap_or_default();
        let visible = (view.0 - down, view.1 - across);
        let content = surface.content;
        let shows = |axis: usize| {
            let bar = &style.axes[axis];
            let (content, extent) = if axis == 0 {
                (content.0, visible.0)
            } else {
                (content.1, visible.1)
            };
            if bar.enabled && (bar.force || overflows(content, extent)) {
                thickness(style, axis, view, scale)
            } else {
                0.0
            }
        };
        (shows(0), shows(1))
    });
    // While the page lays out again for its bars' room, bars may come but
    // not go, as in Chromium's relayout for scrollbars: content that fits
    // only without a bar would otherwise toggle it forever.
    if surface.scrollbar.settling
        && let (Some(new), Some(old)) = (&mut room, surface.scrollbar.room)
    {
        *new = (new.0.max(old.0), new.1.max(old.1));
    }
    let changed = room != surface.scrollbar.room;
    surface.scrollbar.room = room;
    surface.scrollbar.settling = changed;
    changed
}

/// A surface's size, and what its window root scrollbars leave of it.
fn visible(surface: &SurfaceData, scale: f32, zoom: f32) -> ((f32, f32), (f32, f32)) {
    let view = view(surface.hwnd, scale, zoom);
    let (across, down) = surface.scrollbar.room.unwrap_or_default();
    (view, (view.0 - down, view.1 - across))
}

fn geometry(surface: &SurfaceData, scale: f32, zoom: f32) -> Geometry {
    let (view, visible) = visible(surface, scale, zoom);
    let style = surface.scrollbar.style.as_ref().unwrap();
    let mut axes: [Axis; 2] = core::array::from_fn(|axis| Axis {
        enabled: style.axes[axis].enabled,
        force: style.axes[axis].force,
        thickness: thickness(style, axis, view, scale),
    });
    // The window's root bars show where they were given room, and the
    // content scrolls in what they leave.
    if let Some((across, down)) = surface.scrollbar.room {
        for (axis, room) in [across, down].into_iter().enumerate() {
            axes[axis].enabled = room > 0.0;
            axes[axis].force = room > 0.0;
        }
    }
    // The browser's own scrollbars have arrow buttons.
    let pixel = 1.0 / scale;
    let parts = style.platform.map_or((0.0, MINIMUM_THUMB), |look| {
        (look.button_length(pixel), look.minimum_thumb(pixel))
    });
    Geometry::with(
        axes,
        (view, visible),
        surface.content,
        surface.scroll,
        parts,
    )
}

pub(super) fn set(window: WindowId, id: u64, style: Option<Box<Scrollbars>>) {
    cancel(window, id, true);
    let handle = STATE.with_borrow_mut(|state| {
        let surface = state.windows.get_mut(&window)?.surfaces.get_mut(&id)?;
        surface.scrollbar.style = style;
        Some(surface.hwnd)
    });
    if let Some(hwnd) = handle {
        refresh(window, id, hwnd)
    }
}

pub(super) fn cancel(window: WindowId, id: u64, leave: bool) {
    let handle = STATE.with_borrow_mut(|state| {
        let surface = state.windows.get_mut(&window)?.surfaces.get_mut(&id)?;
        let bars = &mut surface.scrollbar;
        let captured = bars.pressed.take().is_some();
        let changed = captured | (leave && bars.hover.take().is_some());
        bars.drag = None;
        changed.then_some((surface.hwnd, captured))
    });
    if let Some((hwnd, captured)) = handle {
        // SAFETY: the handle is live; release only a capture this scrollbar owned.
        unsafe {
            if captured {
                KillTimer(hwnd, REPEAT_TIMER);
            }
            if captured && GetCapture() == hwnd {
                ReleaseCapture();
            }
            InvalidateRect(hwnd, null(), 0);
        }
    }
}

pub(super) fn leave(window: WindowId, id: u64) {
    let handle = STATE.with_borrow_mut(|state| {
        let surface = state.windows.get_mut(&window)?.surfaces.get_mut(&id)?;
        surface.scrollbar.hover.take().map(|_| surface.hwnd)
    });
    if let Some(hwnd) = handle {
        // SAFETY: invalidates the live surface; a captured drag keeps going.
        unsafe {
            InvalidateRect(hwnd, null(), 0);
        }
    }
}

/// Consumes scrollbar presses and drags before they reach the page's editor.
pub(super) fn pointer(
    window: WindowId,
    id: u64,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> bool {
    let action = STATE.with_borrow_mut(|state| {
        let data = state.windows.get_mut(&window)?;
        let surface = data.surfaces.get_mut(&id)?;
        surface.scrollbar.style.as_ref()?;
        let geometry = geometry(surface, data.scale, data.zoom);
        let (_, visible) = visible(surface, data.scale, data.zoom);
        let (x, y) = (
            low(lparam) as f32 / data.scale,
            high(lparam) as f32 / data.scale,
        );
        let style = surface.scrollbar.style.as_ref().unwrap();
        let hit = geometry.hit(x, y).map(|hit| match hit {
            Hit::Thumb(axis)
                if style.platform.is_none() && !style.axes[axis].thumb.0[0].visible =>
            {
                Hit::Track(axis)
            }
            other => other,
        });
        let mut delta = (0.0, 0.0);
        let mut capture = 0;
        let mut consumed = false;
        let mut repeats = false;
        let bars = &mut surface.scrollbar;
        let previous = (bars.hover, bars.pressed);
        bars.hover = hit;
        match message {
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK if hit.is_some() => {
                bars.pressed = hit;
                consumed = true;
                capture = 1;
                match hit {
                    // An arrow button scrolls a step and the track a page
                    // toward the pointer, again and again while held.
                    Some(part @ (Hit::Button(axis, _) | Hit::Track(axis))) => {
                        let (point, length) = if axis == 0 {
                            (x, visible.0)
                        } else {
                            (y, visible.1)
                        };
                        bars.pressed_at = point;
                        repeats = true;
                        if let Some(amount) = geometry.press_amount(part, point, length) {
                            if axis == 0 {
                                delta.0 = amount
                            } else {
                                delta.1 = amount
                            }
                        }
                    }
                    Some(Hit::Thumb(axis)) => {
                        let bar = geometry.bars[axis].unwrap();
                        let (pointer, scroll) = if axis == 0 {
                            (x, surface.scroll.0)
                        } else {
                            (y, surface.scroll.1)
                        };
                        bars.drag = Some(Drag {
                            axis,
                            pointer,
                            scroll,
                            travel: bar.travel,
                            maximum: bar.maximum,
                        });
                    }
                    _ => {}
                }
            }
            WM_MOUSEMOVE => {
                consumed =
                    bars.pressed.is_some() || (hit.is_some() && wparam as u32 & MK_LBUTTON == 0);
                if let Some(axis) = bars.pressed.and_then(Hit::axis) {
                    bars.pressed_at = if axis == 0 { x } else { y };
                }
                if let Some(drag) = bars.drag {
                    let point = if drag.axis == 0 { x } else { y };
                    let target = if drag.travel > 0.0 {
                        drag.scroll + (point - drag.pointer) / drag.travel * drag.maximum
                    } else {
                        drag.scroll
                    };
                    if drag.axis == 0 {
                        delta.0 = target - surface.scroll.0
                    } else {
                        delta.1 = target - surface.scroll.1
                    }
                }
            }
            WM_LBUTTONUP if bars.pressed.is_some() => {
                bars.pressed = None;
                bars.drag = None;
                capture = -1;
                consumed = true;
            }
            _ => {}
        }
        Some((
            surface.hwnd,
            consumed,
            capture,
            delta,
            previous != (bars.hover, bars.pressed),
            repeats,
        ))
    });
    let Some((hwnd, consumed, capture, delta, changed, repeats)) = action else {
        return false;
    };
    // SAFETY: mouse capture, timers and leave tracking belong to this live
    // surface.
    unsafe {
        if capture > 0 {
            SetCapture(hwnd);
        } else if capture < 0 {
            KillTimer(hwnd, REPEAT_TIMER);
            ReleaseCapture();
        }
        if repeats {
            SetTimer(hwnd, REPEAT_TIMER, REPEAT_DELAYS[0], None);
        }
        if message == WM_MOUSEMOVE {
            let mut track = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            TrackMouseEvent(&mut track);
        }
        if changed {
            InvalidateRect(hwnd, null(), 0);
        }
    }
    if delta != (0.0, 0.0) {
        scroll_by(window, id, delta.0, delta.1);
    }
    consumed
}

/// Scrolls again for an arrow button or the track held down, while the
/// pointer stays over it, as Chromium's scrollbars repeat their presses:
/// the track only until the thumb reaches the pointer.
pub(super) fn repeat(window: WindowId, id: u64) {
    let action = STATE.with_borrow(|state| {
        let data = state.windows.get(&window)?;
        let surface = data.surfaces.get(&id)?;
        let bars = &surface.scrollbar;
        let held = bars.pressed.and_then(|pressed| {
            let axis = pressed.axis()?;
            if bars.hover != Some(pressed) || surface.scrollbar.style.is_none() {
                return Some((axis, None));
            }
            let (_, visible) = visible(surface, data.scale, data.zoom);
            let length = if axis == 0 { visible.0 } else { visible.1 };
            let geometry = geometry(surface, data.scale, data.zoom);
            Some((
                axis,
                geometry.press_amount(pressed, bars.pressed_at, length),
            ))
        });
        Some((surface.hwnd, held))
    });
    let Some((hwnd, held)) = action else {
        return;
    };
    // SAFETY: the timer belongs to this live surface; it stops once nothing
    // is held.
    unsafe {
        if held.is_some() {
            SetTimer(hwnd, REPEAT_TIMER, REPEAT_DELAYS[1], None);
        } else {
            KillTimer(hwnd, REPEAT_TIMER);
        }
    }
    if let Some((axis, Some(amount))) = held {
        if axis == 0 {
            scroll_by(window, id, amount, 0.0);
        } else {
            scroll_by(window, id, 0.0, amount);
        }
    }
}

pub(super) fn items(surface: &SurfaceData, scale: f32, zoom: f32) -> Vec<DisplayItem> {
    let mut items = Vec::new();
    let Some(style) = &surface.scrollbar.style else {
        return items;
    };
    let geometry = geometry(surface, scale, zoom);
    let bars = &surface.scrollbar;
    match style.platform {
        Some(look) => nativeboat_ir::scrollbar::platform_items(
            look,
            &geometry,
            surface.scroll,
            (bars.hover, bars.pressed),
            1.0 / scale,
            &mut items,
        ),
        None => nativeboat_ir::scrollbar::styled_items(
            style,
            &geometry,
            bars.hover,
            bars.pressed,
            &mut items,
        ),
    }
    items
}

pub(super) fn cursor(window: WindowId, id: u64) -> bool {
    STATE.with_borrow(|state| {
        state
            .windows
            .get(&window)
            .and_then(|data| data.surfaces.get(&id))
            .is_some_and(|surface| {
                surface.scrollbar.hover.is_some() || surface.scrollbar.pressed.is_some()
            })
    })
}

#[cfg(test)]
mod tests;
