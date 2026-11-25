//! Chromium's `skia::ImageOperations::Resize`, which Electron's
//! `NativeImage.resize` uses: a separable convolution with a Lanczos3 or
//! Hamming1 filter whose weights are 14-bit fixed point, horizontally then
//! vertically, over premultiplied BGRA pixels.

use alloc::vec::Vec;

/// The bits of a filter weight's fraction.
const SHIFT: u32 = 14;

/// The filters Electron's `quality` option chooses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Method {
    /// `good` and `better`.
    Hamming,
    /// `best`, the default.
    Lanczos,
}

impl Method {
    /// The source pixels the filter reaches on each side, in destination
    /// pixels.
    fn support(self) -> f32 {
        match self {
            Method::Hamming => 1.0,
            Method::Lanczos => 3.0,
        }
    }

    fn evaluate(self, x: f32) -> f32 {
        let size = self.support();
        if x <= -size || x >= size {
            return 0.0;
        }
        if x > -f32::EPSILON && x < f32::EPSILON {
            return 1.0;
        }
        let xpi = x * core::f32::consts::PI;
        let sinc = sin(xpi) / xpi;
        match self {
            Method::Hamming => sinc * (0.54 + 0.46 * cos(xpi / size)),
            Method::Lanczos => sinc * sin(xpi / size) / (xpi / size),
        }
    }
}

fn sin(x: f32) -> f32 {
    crate::libm::sin(f64::from(x)) as f32
}

fn cos(x: f32) -> f32 {
    crate::libm::cos(f64::from(x)) as f32
}

/// One destination pixel's filter: the first source pixel it reads and the
/// fixed-point weight of each.
struct Filter {
    start: usize,
    weights: Vec<i32>,
}

/// The filters of each destination pixel along one axis.
fn filters(method: Method, source: u32, destination: u32) -> Vec<Filter> {
    let scale = destination as f32 / source as f32;
    // Magnifying, the filter keeps its width in source pixels.
    let clamped = scale.min(1.0);
    let support = method.support() / clamped;
    let inverse = 1.0 / scale;
    (0..destination)
        .map(|pixel| {
            // Pixel centres: a destination pixel covers the source around
            // its centre's place, not its corner's.
            let centre = (pixel as f32 + 0.5) * inverse;
            let begin = crate::libm::floor(f64::from(centre - support)).max(0.0) as f32;
            let end =
                (crate::libm::ceil(f64::from(centre + support)) as f32).min(source as f32 - 1.0);
            let mut values = Vec::new();
            let mut sum = 0.0_f32;
            let mut at = begin as i32;
            while at as f32 <= end {
                let value = method.evaluate(((at as f32 + 0.5) - centre) * clamped);
                values.push(value);
                sum += value;
                at += 1;
            }
            // Normalized, then truncated to fixed point; what truncation
            // loses goes to the middle weight, so brightness is kept.
            let mut weights: Vec<i32> = values
                .iter()
                .map(|value| i32::from((value / sum * (1 << SHIFT) as f32) as i16))
                .collect();
            let leftover = (1 << SHIFT) - weights.iter().sum::<i32>();
            let middle = weights.len() / 2;
            if let Some(weight) = weights.get_mut(middle) {
                *weight += leftover;
            }
            Filter {
                start: begin as usize,
                weights,
            }
        })
        .collect()
}

fn clamp(value: i32) -> u8 {
    (value >> SHIFT).clamp(0, 255) as u8
}

/// Premultiplied BGRA pixels resized to `width` by `height`. Opaque
/// pictures stay opaque; others keep each colour within its alpha, as
/// premultiplied pixels must.
pub(crate) fn resize(
    pixels: &[u8],
    (source_width, source_height): (u32, u32),
    (width, height): (u32, u32),
    method: Method,
    opaque: bool,
) -> Vec<u8> {
    let columns = filters(method, source_width, width);
    let rows = filters(method, source_height, height);
    let channels = if opaque { 3 } else { 4 };
    let source_width = source_width as usize;
    // Each source row, filtered across.
    let across: Vec<Vec<u8>> = (0..source_height as usize)
        .map(|row| {
            let line = &pixels[row * source_width * 4..(row + 1) * source_width * 4];
            let mut out = Vec::with_capacity(width as usize * 4);
            for filter in &columns {
                let mut sums = [0_i32; 4];
                for (offset, weight) in filter.weights.iter().enumerate() {
                    let pixel = &line[(filter.start + offset) * 4..];
                    for channel in 0..channels {
                        sums[channel] += weight * i32::from(pixel[channel]);
                    }
                }
                let alpha = if opaque { 255 } else { clamp(sums[3]) };
                out.extend([clamp(sums[0]), clamp(sums[1]), clamp(sums[2]), alpha]);
            }
            out
        })
        .collect();
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for filter in &rows {
        for column in 0..width as usize {
            let mut sums = [0_i32; 4];
            for (offset, weight) in filter.weights.iter().enumerate() {
                let pixel = &across[filter.start + offset][column * 4..];
                for channel in 0..channels {
                    sums[channel] += weight * i32::from(pixel[channel]);
                }
            }
            let [blue, green, red] = [clamp(sums[0]), clamp(sums[1]), clamp(sums[2])];
            let alpha = if opaque {
                255
            } else {
                // Rounding can leave a colour above its alpha.
                clamp(sums[3]).max(blue).max(green).max(red)
            };
            out.extend([blue, green, red, alpha]);
        }
    }
    out
}
