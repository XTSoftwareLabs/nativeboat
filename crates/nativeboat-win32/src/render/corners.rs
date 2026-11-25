//! Rounded corners as Chromium's GPU rasterization antialiases them. A box
//! with rounded corners is painted with hard, whole-pixel edges, then the
//! pixels of its corner rectangles are blended with what was beneath them by
//! how much of each pixel the shape covers. A circular corner covers
//! `r - d + 0.5`, clamped, for a pixel centre `d` from its corner's centre.
//! An elliptical one covers `0.5 - f / w`, clamped, for the ellipse's
//! implicit function `f = (x/rx)² + (y/ry)² - 1` at the pixel centre and `w`
//! the sum of its differences across and down the 2×2 quad of pixels the GPU
//! shades together. A ring, as a rounded border, covers what its outer edge
//! covers and its inner edge does not. Fills, borders and rounded clips share
//! the coverage, as they do in Chromium.

#[allow(unused_imports)]
use crate::prelude::*;
use windows_sys::Win32::Graphics::Gdi::BITMAP;

/// Corner radii, horizontal and vertical, top left first, clockwise.
pub(super) type Corners = [(f32, f32); 4];

/// A rounded rectangle in device pixels, or a ring between two.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Shape {
    /// Whole-pixel left, top, right and bottom edges.
    pub rect: [i32; 4],
    /// Corner radii, horizontal and vertical, top left first, clockwise.
    pub radii: Corners,
    /// The inner edge of a ring: its left, top, right and bottom edges and
    /// corner radii.
    pub inner: Option<([f32; 4], Corners)>,
}

impl Shape {
    /// How much of the pixel at `x`, `y` the shape covers, from 0 to 1.
    pub fn coverage(&self, x: i32, y: i32) -> f32 {
        let edges = self.rect.map(|edge| edge as f32);
        let outer = rounded(edges, self.radii, x, y);
        match self.inner {
            Some((edges, radii)) => outer * (1.0 - rounded(edges, radii, x, y)),
            None => outer,
        }
    }

    /// The device pixels whose coverage may be partial: the squares of the
    /// corners, or the whole box when they meet.
    pub fn tiles(&self) -> Vec<[i32; 4]> {
        let [left, top, right, bottom] = self.rect;
        let size = |(x, y): (f32, f32)| (x.ceil() as i32, y.ceil() as i32);
        let [top_left, top_right, bottom_right, bottom_left] = self.radii.map(size);
        let squares = [
            [left, top, left + top_left.0, top + top_left.1],
            [right - top_right.0, top, right, top + top_right.1],
            [
                right - bottom_right.0,
                bottom - bottom_right.1,
                right,
                bottom,
            ],
            [left, bottom - bottom_left.1, left + bottom_left.0, bottom],
        ];
        let squares: Vec<[i32; 4]> = squares
            .into_iter()
            .filter(|square| square[2] > square[0] && square[3] > square[1])
            .collect();
        let meet =
            |a: &[i32; 4], b: &[i32; 4]| a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];
        let overlapping = squares.iter().enumerate().any(|(index, a)| {
            squares[index + 1..].iter().any(|b| meet(a, b))
                || a[0] < left
                || a[1] < top
                || a[2] > right
                || a[3] > bottom
        });
        if overlapping {
            alloc::vec![self.rect]
        } else {
            squares
        }
    }
}

/// The coverage of a rounded rectangle with possibly fractional edges.
fn rounded([left, top, right, bottom]: [f32; 4], radii: Corners, x: i32, y: i32) -> f32 {
    let (centre_x, centre_y) = (x as f32 + 0.5, y as f32 + 0.5);
    let mut coverage = (centre_x - left + 0.5)
        .min(right - centre_x + 0.5)
        .min(centre_y - top + 0.5)
        .min(bottom - centre_y + 0.5);
    let [top_left, top_right, bottom_right, bottom_left] = radii;
    // Each corner's centre and radii, and whether the pixel centre lies in
    // its rectangle.
    let corners = [
        (
            (left + top_left.0, top + top_left.1),
            top_left,
            centre_x < left + top_left.0 && centre_y < top + top_left.1,
        ),
        (
            (right - top_right.0, top + top_right.1),
            top_right,
            centre_x > right - top_right.0 && centre_y < top + top_right.1,
        ),
        (
            (right - bottom_right.0, bottom - bottom_right.1),
            bottom_right,
            centre_x > right - bottom_right.0 && centre_y > bottom - bottom_right.1,
        ),
        (
            (left + bottom_left.0, bottom - bottom_left.1),
            bottom_left,
            centre_x < left + bottom_left.0 && centre_y > bottom - bottom_left.1,
        ),
    ];
    for (centre, (radius_x, radius_y), within) in corners {
        if radius_x <= 0.0 || radius_y <= 0.0 || !within {
            continue;
        }
        let corner = if radius_x == radius_y {
            let (dx, dy) = (centre_x - centre.0, centre_y - centre.1);
            radius_x - (dx * dx + dy * dy).sqrt() + 0.5
        } else {
            let implicit = |x: i32, y: i32| {
                let dx = (x as f32 + 0.5 - centre.0) / radius_x;
                let dy = (y as f32 + 0.5 - centre.1) / radius_y;
                dx * dx + dy * dy - 1.0
            };
            // The GPU differences each 2×2 quad's pixels: an even column
            // with the odd one after it, an even row with the next.
            let (quad_x, quad_y) = (x & !1, y & !1);
            let width = (implicit(quad_x + 1, y) - implicit(quad_x, y)).abs()
                + (implicit(x, quad_y + 1) - implicit(x, quad_y)).abs();
            0.5 - implicit(x, y) / width.max(f32::MIN_POSITIVE)
        };
        coverage = coverage.min(corner);
    }
    coverage.clamp(0.0, 1.0)
}

/// A tile of a shape's corners and the pixels that were beneath it.
type Saved = ([i32; 4], Vec<u32>);

/// Open coverage groups over a 32-bit top-down DIB: each keeps the pixels
/// beneath its corner squares until its content is painted.
pub(super) struct Coverage {
    bits: *mut u32,
    width: i32,
    height: i32,
    stride: usize,
    open: Vec<(Shape, Vec<Saved>)>,
}

impl Coverage {
    /// The caller keeps this top-down 32-bit DIB alive until the groups are
    /// dropped and flushes GDI/GDI+ before pushing or popping.
    pub unsafe fn new(bitmap: BITMAP) -> Self {
        let usable = bitmap.bmBitsPixel == 32
            && !bitmap.bmBits.is_null()
            && bitmap.bmWidthBytes > 0
            && bitmap.bmWidthBytes % 4 == 0
            && bitmap.bmHeight > 0;
        Self {
            bits: bitmap.bmBits.cast(),
            width: if usable { bitmap.bmWidth } else { 0 },
            height: if usable { bitmap.bmHeight } else { 0 },
            stride: bitmap.bmWidthBytes.max(0) as usize / 4,
            open: Vec::new(),
        }
    }

    /// The bitmap's pixels.
    fn pixels(&mut self) -> &mut [u32] {
        if self.width == 0 {
            return &mut [];
        }
        // SAFETY: `new`'s caller keeps the bitmap live and flushes drawing
        // first; no other view of it survives this borrow.
        unsafe { core::slice::from_raw_parts_mut(self.bits, self.stride * self.height as usize) }
    }

    /// Starts painting `shape`: keeps the pixels beneath its corners.
    pub fn push(&mut self, shape: Shape) {
        let (width, height) = (self.width, self.height);
        let stride = self.stride;
        let tiles: Vec<[i32; 4]> = shape
            .tiles()
            .into_iter()
            .map(|[left, top, right, bottom]| {
                [
                    left.clamp(0, width),
                    top.clamp(0, height),
                    right.clamp(0, width),
                    bottom.clamp(0, height),
                ]
            })
            .filter(|tile| tile[2] > tile[0] && tile[3] > tile[1])
            .collect();
        let pixels = self.pixels();
        let saved = tiles
            .into_iter()
            .map(|tile| {
                let [left, top, right, bottom] = tile;
                let mut beneath = Vec::new();
                for row in top..bottom {
                    let start = row as usize * stride;
                    beneath
                        .extend_from_slice(&pixels[start + left as usize..start + right as usize]);
                }
                (tile, beneath)
            })
            .collect();
        self.open.push((shape, saved));
    }

    /// Ends the last shape: its corner pixels become what was beneath them
    /// mixed with what was painted, by the shape's coverage.
    pub fn pop(&mut self) {
        let Some((shape, saved)) = self.open.pop() else {
            return;
        };
        let stride = self.stride;
        let pixels = self.pixels();
        for ([left, top, right, bottom], beneath) in saved {
            let width = (right - left) as usize;
            for (row, line) in (top..bottom).zip(beneath.chunks_exact(width)) {
                let start = row as usize * stride + left as usize;
                for ((x, pixel), before) in
                    (left..).zip(&mut pixels[start..start + width]).zip(line)
                {
                    *pixel = mix(*pixel, *before, shape.coverage(x, row));
                }
            }
        }
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }
}

/// `painted` over `beneath` by `coverage`, channel by channel; the unused
/// fourth byte is the painted one's.
fn mix(painted: u32, beneath: u32, coverage: f32) -> u32 {
    if coverage >= 1.0 || painted == beneath {
        return painted;
    }
    let mut mixed = painted & 0xff00_0000;
    for shift in [0, 8, 16] {
        let (over, under) = (
            (painted >> shift & 255) as f32,
            (beneath >> shift & 255) as f32,
        );
        let channel = under + (over - under) * coverage;
        mixed |= ((channel + 0.5) as u32).min(255) << shift;
    }
    mixed
}

#[cfg(test)]
mod tests;
