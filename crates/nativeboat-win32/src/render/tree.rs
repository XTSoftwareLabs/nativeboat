//! Painting a whole native tree into one window, for hosts that draw nested
//! scroll surfaces themselves instead of as child windows.

use super::{Layer, meet, paint_area};
#[allow(unused_imports)]
use crate::prelude::*;
use nativeboat_css::Color;
use nativeboat_ir::{NativeTree, Surface};
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{BitBlt, HDC, SRCCOPY};

/// Paints `tree` over `background` into `dc`, whose drawable area is
/// `size` device pixels, at `scale` device pixels per CSS pixel. The tree
/// is painted off screen first, so the window never shows it half drawn.
///
/// # Safety
/// `dc` must be a live device context, such as one from `BeginPaint`.
pub unsafe fn paint_tree(
    dc: HDC,
    tree: &NativeTree,
    scale: f32,
    size: (i32, i32),
    background: Color,
) {
    let (width, height) = size;
    if width <= 0 || height <= 0 {
        return;
    }
    let fill =
        (u32::from(background.r) << 16) | (u32::from(background.g) << 8) | u32::from(background.b);
    let Some(layer) = Layer::new(width, height, fill) else {
        return;
    };
    let area = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    surface(layer.dc, &tree.root, scale, (0.0, 0.0), area);
    // SAFETY: the caller supplies a live DC; the layer has the copied size.
    unsafe { BitBlt(dc, 0, 0, width, height, layer.dc, 0, 0, SRCCOPY) };
}

/// Paints a surface whose top-left corner is at `origin` in window
/// coordinates, clipped to `clip` in device pixels, then its children.
pub(crate) fn surface(dc: HDC, surface: &Surface, scale: f32, origin: (f32, f32), clip: RECT) {
    // Content at (x, y) shows at origin + (x, y) - scroll.
    let scroll = (surface.scroll_x - origin.0, surface.scroll_y - origin.1);
    paint_area(dc, &surface.items, scale, scroll, Some(clip), false);
    for child in &surface.children {
        let corner = (child.rect.x - scroll.0, child.rect.y - scroll.1);
        let bounds = RECT {
            left: (corner.0 * scale).floor() as i32,
            top: (corner.1 * scale).floor() as i32,
            right: ((corner.0 + child.rect.width) * scale).ceil() as i32,
            bottom: ((corner.1 + child.rect.height) * scale).ceil() as i32,
        };
        if let Some(inner) = meet(clip, bounds) {
            self::surface(dc, child, scale, corner, inner);
        }
    }
}

#[cfg(test)]
mod tests;
