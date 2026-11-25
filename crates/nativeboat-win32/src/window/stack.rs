//! Painted content above native child HWNDs, in compiled display-list order.

use super::*;
#[allow(unused_imports)]
use crate::prelude::*;
use nativeboat_ir::NativeChild;

fn parent(data: &WindowData, child: NativeChild) -> Option<u64> {
    match child {
        NativeChild::Surface(0) => None,
        NativeChild::Surface(id) => Some(data.surfaces.get(&id)?.parent),
        NativeChild::Control(id) => Some(data.controls.get(&id)?.surface),
    }
}

fn handle(data: &WindowData, child: NativeChild) -> Option<HWND> {
    match child {
        NativeChild::Surface(id) => Some(data.surfaces.get(&id)?.hwnd),
        NativeChild::Control(id) => Some(data.controls.get(&id)?.hwnd),
    }
}

fn scope(item: &DisplayItem) -> bool {
    matches!(
        item,
        DisplayItem::PushClip { .. }
            | DisplayItem::PushClipPath { .. }
            | DisplayItem::PopClip
            | DisplayItem::PushTransform(_)
            | DisplayItem::PopTransform
            | DisplayItem::PushOpacity(_)
            | DisplayItem::PopOpacity
            | DisplayItem::PushFilter(_)
            | DisplayItem::PopFilter
            | DisplayItem::PushMask(_)
            | DisplayItem::PopMask
    )
}

fn following(items: &[DisplayItem], child: NativeChild) -> Option<usize> {
    let index = items
        .iter()
        .position(|item| *item == DisplayItem::Child(child))?
        + 1;
    items[index..]
        .iter()
        .any(|item| !scope(item) && !matches!(item, DisplayItem::Child(_)))
        .then_some(index)
}

pub(crate) fn exists(window: WindowId, mut child: NativeChild) -> bool {
    STATE.with_borrow(|state| {
        let Some(data) = state.windows.get(&window) else {
            return false;
        };
        while let Some(id) = parent(data, child) {
            let Some(surface) = data.surfaces.get(&id) else {
                return false;
            };
            if following(&surface.items, child).is_some() {
                return true;
            }
            child = NativeChild::Surface(id);
        }
        false
    })
}

/// Native children share the same CSS paint order, including a single
/// control above a scroller. Reordering keeps bounds and focus unchanged.
pub(super) fn order(window: WindowId, surface: u64, children: &[NativeChild]) {
    let handles: Vec<_> = STATE.with_borrow(|state| {
        let Some(data) = state.windows.get(&window) else {
            return Vec::new();
        };
        children
            .iter()
            .filter(|child| parent(data, **child) == Some(surface))
            .filter_map(|child| handle(data, *child))
            .collect()
    });
    for hwnd in handles {
        // SAFETY: handles are owned by this window on this thread.
        unsafe {
            SetWindowPos(
                hwnd,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
}

/// How far backdrop filters in the ancestor display items painted above a
/// child read beyond an area of it, in device pixels: they filter the
/// child's pixels, so its partial repaints paint that far too.
pub(crate) fn backdrop_reach(data: &WindowData, mut child: NativeChild) -> i32 {
    let mut reach = 0;
    while let Some((id, surface)) =
        parent(data, child).and_then(|id| Some((id, data.surfaces.get(&id)?)))
    {
        if let Some(index) = following(&surface.items, child) {
            let above = surface.items[..index]
                .iter()
                .filter(|item| scope(item))
                .chain(&surface.items[index..]);
            reach = reach.max(render::backdrop_reach(above, data.scale));
        }
        child = NativeChild::Surface(id);
    }
    reach
}

/// Paints subsequent ancestor display items into an owned 32-bit DIB.
/// Actual HWND origins account for DPI rounding. Replayed scope commands
/// preserve ancestor transforms and clipping without repainting backgrounds
/// that precede the child.
pub(crate) fn paint(window: WindowId, child: NativeChild, target: HWND, dc: HDC, area: RECT) {
    paint_at(window, child, target, dc, area, POINT::default());
}

/// A child's frame, when page content paints above it: its window size
/// and the offset and size of its client area in it.
struct Frame {
    size: (i32, i32),
    inset: POINT,
    client: RECT,
}

fn frame(window: WindowId, child: NativeChild, hwnd: HWND) -> Option<Frame> {
    if !exists(window, child) {
        return None;
    }
    let (mut bounds, mut client) = (RECT::default(), RECT::default());
    let mut corner = POINT::default();
    // SAFETY: reads the geometry of a live child of this window.
    unsafe {
        GetWindowRect(hwnd, &mut bounds);
        GetClientRect(hwnd, &mut client);
        ClientToScreen(hwnd, &mut corner);
    }
    let size = (bounds.right - bounds.left, bounds.bottom - bounds.top);
    let inset = POINT {
        x: corner.x - bounds.left,
        y: corner.y - bounds.top,
    };
    let framed = inset.x != 0 || inset.y != 0 || client.right != size.0 || client.bottom != size.1;
    (size.0 > 0 && size.1 > 0 && framed).then_some(Frame {
        size,
        inset,
        client,
    })
}

/// Renders a child's whole window into a DC: its native frame, its client
/// as `client` paints it at the offset it is given, and the ancestor
/// display items above them.
fn render_frame(
    window: WindowId,
    child: NativeChild,
    hwnd: HWND,
    dc: HDC,
    frame: &Frame,
    client: impl FnOnce(HDC, POINT),
) {
    // SAFETY: the child prints into the caller's live DC.
    unsafe {
        SendMessageW(hwnd, WM_PRINT, dc as usize, PRF_NONCLIENT as isize);
    }
    client(dc, frame.inset);
    // SAFETY: completes GDI drawing before the display items read it.
    unsafe { GdiFlush() };
    let whole = RECT {
        left: 0,
        top: 0,
        right: frame.size.0,
        bottom: frame.size.1,
    };
    let offset = POINT {
        x: -frame.inset.x,
        y: -frame.inset.y,
    };
    paint_at(window, child, hwnd, dc, whole, offset);
}

/// Paints a child's frame and native scroll bars, its non-client area, with
/// the ancestor display items above it, as its client area paints, so that
/// backdrop filters above it filter them too. `client` paints the client
/// area into the frame's DIB at the offset it is given, for filters to
/// read. Returns `false`, leaving the frame to the system, when nothing
/// paints above the child or it has no frame.
pub(crate) fn paint_frame(
    window: WindowId,
    child: NativeChild,
    hwnd: HWND,
    client: impl FnOnce(HDC, POINT),
) -> bool {
    let Some(frame) = frame(window, child, hwnd) else {
        return false;
    };
    let (width, height) = frame.size;
    // SAFETY: the child is live; every temporary GDI object is released
    // here, and the window DC is released after the copy.
    unsafe {
        let target = GetWindowDC(hwnd);
        if target.is_null() {
            return false;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        let memory = CreateCompatibleDC(target);
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(memory, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if bitmap.is_null() {
            DeleteDC(memory);
            ReleaseDC(hwnd, target);
            return false;
        }
        let previous = SelectObject(memory, bitmap);
        PatBlt(memory, 0, 0, width, height, WHITENESS);
        render_frame(window, child, hwnd, memory, &frame, client);
        let inset = frame.inset;
        ExcludeClipRect(
            target,
            inset.x,
            inset.y,
            inset.x + frame.client.right,
            inset.y + frame.client.bottom,
        );
        BitBlt(target, 0, 0, width, height, memory, 0, 0, SRCCOPY);
        SelectObject(memory, previous);
        DeleteObject(bitmap);
        DeleteDC(memory);
        ReleaseDC(hwnd, target);
    }
    true
}

/// Paints the ancestor display items above a child into a device context
/// whose origin lies at `offset` from the child's client origin.
fn paint_at(
    window: WindowId,
    mut child: NativeChild,
    target: HWND,
    dc: HDC,
    area: RECT,
    offset: POINT,
) {
    STATE.with_borrow(|state| {
        let Some(data) = state.windows.get(&window) else {
            return;
        };
        while let Some(id) = parent(data, child) {
            let Some(surface) = data.surfaces.get(&id) else {
                return;
            };
            if let Some(index) = following(&surface.items, child) {
                let mut items: Vec<_> = surface.items[..index]
                    .iter()
                    .filter(|item| scope(item))
                    .cloned()
                    .collect();
                items.extend_from_slice(&surface.items[index..]);
                let mut origin = POINT::default();
                let mut bounds = RECT::default();
                // SAFETY: the target and ancestor belong to this window.
                unsafe {
                    MapWindowPoints(target, surface.hwnd, &mut origin, 1);
                    GetClientRect(surface.hwnd, &mut bounds);
                }
                origin.x += offset.x;
                origin.y += offset.y;
                let clip = RECT {
                    left: area.left.max(-origin.x),
                    top: area.top.max(-origin.y),
                    right: area.right.min(bounds.right - origin.x),
                    bottom: area.bottom.min(bounds.bottom - origin.y),
                };
                if clip.left < clip.right && clip.top < clip.bottom {
                    render::paint_area(
                        dc,
                        &items,
                        data.scale,
                        (
                            surface.scroll.0 + origin.x as f32 / data.scale,
                            surface.scroll.1 + origin.y as f32 / data.scale,
                        ),
                        Some(clip),
                        false,
                    );
                }
            }
            child = NativeChild::Surface(id);
        }
    });
}

/// Repaints an area of a surface, or all of it, with the parts of its
/// descendant HWNDs under it, their frames included: a parent paint change
/// can alter pixels above a descendant even when its own display list did
/// not change.
pub(super) fn invalidate(hwnd: HWND, area: Option<&RECT>) {
    // SAFETY: invalidates only this owned surface and its native children.
    unsafe {
        RedrawWindow(
            hwnd,
            area.map_or(null(), |area| area),
            null_mut(),
            RDW_INVALIDATE | RDW_ALLCHILDREN,
        );
        // The surface's own frame, a window's caption for the root, stays.
        let mut screen = match area {
            Some(area) => *area,
            None => client(hwnd),
        };
        MapWindowPoints(hwnd, null_mut(), &mut screen as *mut RECT as *mut POINT, 2);
        EnumChildWindows(hwnd, Some(frame_under), &screen as *const RECT as LPARAM);
    }
}

/// Repaints the frame of a descendant with one, where it meets a screen
/// area.
unsafe extern "system" fn frame_under(child: HWND, area: LPARAM) -> windows_sys::core::BOOL {
    // SAFETY: `area` points at the caller's live rectangle for the
    // enumeration; the child belongs to the enumerated window.
    unsafe {
        let area = *(area as *const RECT);
        let (mut bounds, mut inner) = (RECT::default(), RECT::default());
        GetWindowRect(child, &mut bounds);
        GetClientRect(child, &mut inner);
        MapWindowPoints(child, null_mut(), &mut inner as *mut RECT as *mut POINT, 2);
        let framed = (inner.left, inner.top, inner.right, inner.bottom)
            != (bounds.left, bounds.top, bounds.right, bounds.bottom);
        let meets = bounds.left < area.right
            && area.left < bounds.right
            && bounds.top < area.bottom
            && area.top < bounds.bottom;
        if framed && meets {
            let mut part = area;
            MapWindowPoints(null_mut(), child, &mut part as *mut RECT as *mut POINT, 2);
            RedrawWindow(
                child,
                &part,
                null_mut(),
                RDW_INVALIDATE | RDW_FRAME | RDW_NOCHILDREN,
            );
        }
    }
    1
}

#[cfg(test)]
mod tests;
