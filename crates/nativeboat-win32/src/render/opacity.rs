//! Group opacity over a copied RGB backdrop. For ordinary source-over
//! drawing, mixing the completed group with its saved backdrop once is
//! equivalent to compositing an isolated transparent layer.
#[allow(unused_imports)]
use crate::prelude::*;
use windows_sys::Win32::{Foundation::RECT, Graphics::Gdi::BITMAP};

const BUDGET: usize = 32 * 1024 * 1024;

struct Group {
    alpha: u16,
    saved: Option<Vec<u32>>,
    region: RECT,
}

pub(super) struct Groups {
    bits: *mut u32,
    length: usize,
    stride: usize,
    open: Vec<Group>,
    pool: Vec<Vec<u32>>,
    allocated: usize,
    budget: usize,
}

#[cfg(test)]
mod tests;

impl Groups {
    /// The caller keeps this top-down 32-bit DIB alive until the groups are
    /// dropped and flushes GDI/GDI+ before reading or changing its pixels.
    pub unsafe fn new(bitmap: BITMAP) -> Self {
        let length = if bitmap.bmBitsPixel == 32
            && !bitmap.bmBits.is_null()
            && bitmap.bmWidthBytes > 0
            && bitmap.bmWidthBytes % 4 == 0
            && bitmap.bmHeight > 0
        {
            (bitmap.bmWidthBytes as usize)
                .checked_mul(bitmap.bmHeight as usize)
                .unwrap_or(0)
        } else {
            0
        };
        Self {
            bits: bitmap.bmBits.cast(),
            length,
            stride: bitmap.bmWidthBytes.max(0) as usize / 4,
            open: Vec::new(),
            pool: Vec::new(),
            allocated: 0,
            budget: BUDGET,
        }
    }

    /// Whether the group can paint without an opacity multiplier. If its
    /// backdrop does not fit the budget, the caller uses per-item alpha.
    pub fn push(&mut self, opacity: f32) -> bool {
        self.push_in(opacity, None)
    }

    /// Saves only the device pixels the group's content and clip can change.
    /// `region` must conservatively include antialiasing and filter overflow.
    pub fn push_in(&mut self, opacity: f32, region: Option<RECT>) -> bool {
        let full = RECT {
            left: 0,
            top: 0,
            right: self.stride as i32,
            bottom: (self.length / 4).checked_div(self.stride).unwrap_or(0) as i32,
        };
        let region = region.map_or(full, |region| RECT {
            left: region.left.clamp(0, full.right),
            top: region.top.clamp(0, full.bottom),
            right: region.right.clamp(0, full.right),
            bottom: region.bottom.clamp(0, full.bottom),
        });
        let width = (region.right - region.left).max(0) as usize;
        let height = (region.bottom - region.top).max(0) as usize;
        let length = width * height;
        let saved = if opacity > 0.0 && opacity < 1.0 && length > 0 {
            let reusable = self.pool.iter().position(|buffer| buffer.len() == length);
            let buffer = reusable
                .map(|index| self.pool.swap_remove(index))
                .or_else(|| {
                    // Keep the budget shared by open and reusable buffers. Only
                    // completed groups can be evicted; nested groups stay live.
                    while length * 4 > self.budget.saturating_sub(self.allocated) {
                        let buffer = self.pool.pop()?;
                        self.allocated -= buffer.len() * 4;
                    }
                    self.allocated += length * 4;
                    Some(vec![0; length])
                });
            buffer.map(|mut buffer| {
                // SAFETY: `new`'s caller guarantees the bitmap's lifetime and
                // flushes drawing before this copy; no borrowed view survives.
                let pixels = unsafe { core::slice::from_raw_parts(self.bits, self.length / 4) };
                if width == self.stride {
                    // Full-width rows are contiguous. Keep the single-copy
                    // path for full backdrops and horizontal bands.
                    let start = region.top as usize * self.stride;
                    buffer.copy_from_slice(&pixels[start..start + length]);
                } else {
                    for (row, saved) in buffer.chunks_exact_mut(width).enumerate() {
                        let start =
                            (region.top as usize + row) * self.stride + region.left as usize;
                        saved.copy_from_slice(&pixels[start..start + width]);
                    }
                }
                buffer
            })
        } else {
            None
        };
        let isolated = saved.is_some();
        self.open.push(Group {
            alpha: (opacity * 255.0).round() as u16,
            saved,
            region,
        });
        isolated
    }

    pub fn pop(&mut self) {
        if let Some(Group {
            alpha,
            saved: Some(buffer),
            region,
        }) = self.open.pop()
        {
            // SAFETY: the caller flushes drawing, and the bitmap stays live.
            // RGB32's fourth byte is unused and is deliberately preserved.
            let pixels = unsafe { core::slice::from_raw_parts_mut(self.bits, self.length / 4) };
            let width = (region.right - region.left) as usize;
            if width == self.stride {
                let start = region.top as usize * self.stride;
                blend(&mut pixels[start..start + buffer.len()], &buffer, alpha);
            } else {
                for (row, saved) in buffer.chunks_exact(width).enumerate() {
                    let start = (region.top as usize + row) * self.stride + region.left as usize;
                    blend(&mut pixels[start..start + width], saved, alpha);
                }
            }
            self.pool.push(buffer);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }
}

fn blend(pixels: &mut [u32], saved: &[u32], alpha: u16) {
    for (pixel, before) in pixels.iter_mut().zip(saved) {
        // Unchanged backdrop pixels already have the correct value.
        if (*pixel ^ before) & 0x00ff_ffff == 0 {
            continue;
        }
        let mut mixed = *pixel & 0xff00_0000;
        for shift in [0, 8, 16] {
            let channel = (((*pixel >> shift & 255) * u32::from(alpha)
                + (before >> shift & 255) * u32::from(255 - alpha)
                + 127)
                / 255)
                << shift;
            mixed |= channel;
        }
        *pixel = mixed;
    }
}
