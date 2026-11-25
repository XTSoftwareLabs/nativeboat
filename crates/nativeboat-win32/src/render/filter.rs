//! CSS filter functions on premultiplied pixels, following the Filter
//! Effects specification: color functions work on unpremultiplied sRGB
//! values, and blurs use the three-box approximation of a Gaussian that SVG
//! prescribes for larger deviations.

#[allow(unused_imports)]
use crate::prelude::*;
use nativeboat_css::types::Filter;

/// Premultiplied RGBA pixels with channels between 0 and 1.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Image {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<[f32; 4]>,
}

fn channel(pixel: u32, shift: u32) -> i32 {
    (pixel >> shift & 255) as i32
}

impl Image {
    /// Recovers a transparent layer from the same content painted over
    /// black and over white 32-bit RGB pixels: what lets the white show
    /// through is the uncovered part.
    pub fn isolate(black: &[u32], white: &[u32], width: usize, height: usize) -> Self {
        let pixels = black
            .iter()
            .zip(white)
            .map(|(&black, &white)| {
                let coverage = [16, 8, 0]
                    .map(|shift| 255 - (channel(white, shift) - channel(black, shift)))
                    .into_iter()
                    .max()
                    .unwrap_or(0)
                    .min(255);
                let alpha = coverage as f32 / 255.0;
                let color = |shift| (channel(black, shift) as f32 / 255.0).min(alpha);
                [color(16), color(8), color(0), alpha]
            })
            .collect();
        Image {
            width,
            height,
            pixels,
        }
    }

    /// Opaque pixels from 32-bit RGB values; their fourth byte is ignored.
    pub fn opaque(pixels: &[u32], width: usize, height: usize) -> Self {
        let pixels = pixels
            .iter()
            .map(|&pixel| {
                let color = |shift| channel(pixel, shift) as f32 / 255.0;
                [color(16), color(8), color(0), 1.0]
            })
            .collect();
        Image {
            width,
            height,
            pixels,
        }
    }

    /// Premultiplied `0xAARRGGBB` values, as GDI+ PARGB bitmaps hold them.
    pub fn pargb(&self) -> Vec<u32> {
        self.pixels
            .iter()
            .map(|pixel| {
                let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
                let alpha = byte(pixel[3]);
                let color = |value: f32| byte(value).min(alpha);
                alpha << 24 | color(pixel[0]) << 16 | color(pixel[1]) << 8 | color(pixel[2])
            })
            .collect()
    }

    /// The image grown by a border that mirrors the pixels at its edges.
    pub fn mirrored(&self, x: usize, y: usize) -> Image {
        let (width, height) = (self.width + 2 * x, self.height + 2 * y);
        let mut pixels = Vec::with_capacity(width * height);
        for row in 0..height {
            let source = mirror(row as isize - y as isize, self.height) * self.width;
            for column in 0..width {
                let column = mirror(column as isize - x as isize, self.width);
                pixels.push(self.pixels[source + column]);
            }
        }
        Image {
            width,
            height,
            pixels,
        }
    }

    /// A rectangle of the image.
    pub fn crop(&self, x: usize, y: usize, width: usize, height: usize) -> Image {
        let pixels = (y..y + height)
            .flat_map(|row| &self.pixels[row * self.width + x..row * self.width + x + width])
            .copied()
            .collect();
        Image {
            width,
            height,
            pixels,
        }
    }
}

/// An index reflected into `0..length`, repeating the edge pixels.
fn mirror(index: isize, length: usize) -> usize {
    let length = length as isize;
    let folded = index.rem_euclid(2 * length);
    (if folded < length {
        folded
    } else {
        2 * length - 1 - folded
    }) as usize
}

/// How much a linear map scales areas, as a factor for lengths.
fn length_scale(matrix: [f32; 4]) -> f32 {
    (matrix[0] * matrix[3] - matrix[1] * matrix[2]).abs().sqrt()
}

/// A CSS offset in whole device pixels.
fn offset(matrix: [f32; 4], x: f32, y: f32) -> (isize, isize) {
    let [a, b, c, d] = matrix;
    (
        (a * x + c * y).round() as isize,
        (b * x + d * y).round() as isize,
    )
}

/// How far a blur with a deviation spreads, in pixels.
fn extent(sigma: f32) -> usize {
    (3.0 * sigma).ceil().max(0.0) as usize
}

/// How far the functions spread content horizontally and vertically, in
/// device pixels. `matrix` maps CSS pixels to device pixels.
pub(super) fn reach(filters: &[Filter], matrix: [f32; 4]) -> (usize, usize) {
    let scale = length_scale(matrix);
    filters.iter().fold((0, 0), |(x, y), filter| match *filter {
        Filter::Blur(radius) => {
            let spread = extent(radius * scale);
            (x + spread, y + spread)
        }
        Filter::DropShadow {
            x: dx, y: dy, blur, ..
        } => {
            let (dx, dy) = offset(matrix, dx, dy);
            let spread = extent(blur / 2.0 * scale);
            (
                x + dx.unsigned_abs() + spread,
                y + dy.unsigned_abs() + spread,
            )
        }
        _ => (x, y),
    })
}

/// Applies the functions in order. `matrix` maps CSS pixels to device
/// pixels, scaling blurs and moving shadows.
pub(super) fn apply(image: &mut Image, filters: &[Filter], matrix: [f32; 4]) {
    let scale = length_scale(matrix);
    for filter in filters {
        match *filter {
            Filter::Blur(radius) => blur(image, radius * scale),
            Filter::Brightness(amount) => recolor(image, |c| c.map(|value| value * amount)),
            Filter::Contrast(amount) => recolor(image, |c| {
                c.map(|value| amount * value + 0.5 - 0.5 * amount)
            }),
            Filter::Grayscale(amount) => {
                let a = 1.0 - amount;
                recolor(
                    image,
                    transform([
                        [
                            0.2126 + 0.7874 * a,
                            0.7152 - 0.7152 * a,
                            0.0722 - 0.0722 * a,
                        ],
                        [
                            0.2126 - 0.2126 * a,
                            0.7152 + 0.2848 * a,
                            0.0722 - 0.0722 * a,
                        ],
                        [
                            0.2126 - 0.2126 * a,
                            0.7152 - 0.7152 * a,
                            0.0722 + 0.9278 * a,
                        ],
                    ]),
                )
            }
            Filter::Sepia(amount) => {
                let a = 1.0 - amount;
                recolor(
                    image,
                    transform([
                        [0.393 + 0.607 * a, 0.769 - 0.769 * a, 0.189 - 0.189 * a],
                        [0.349 - 0.349 * a, 0.686 + 0.314 * a, 0.168 - 0.168 * a],
                        [0.272 - 0.272 * a, 0.534 - 0.534 * a, 0.131 + 0.869 * a],
                    ]),
                )
            }
            Filter::Saturate(s) => recolor(
                image,
                transform([
                    [0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s],
                    [0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s],
                    [0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s],
                ]),
            ),
            Filter::HueRotate(degrees) => {
                let (sin, cos) = degrees.to_radians().sin_cos();
                recolor(
                    image,
                    transform([
                        [
                            0.213 + cos * 0.787 - sin * 0.213,
                            0.715 - cos * 0.715 - sin * 0.715,
                            0.072 - cos * 0.072 + sin * 0.928,
                        ],
                        [
                            0.213 - cos * 0.213 + sin * 0.143,
                            0.715 + cos * 0.285 + sin * 0.140,
                            0.072 - cos * 0.072 - sin * 0.283,
                        ],
                        [
                            0.213 - cos * 0.213 - sin * 0.787,
                            0.715 - cos * 0.715 + sin * 0.715,
                            0.072 + cos * 0.928 + sin * 0.072,
                        ],
                    ]),
                )
            }
            Filter::Invert(amount) => recolor(image, |c| {
                c.map(|value| amount + value * (1.0 - 2.0 * amount))
            }),
            Filter::Opacity(amount) => {
                for pixel in &mut image.pixels {
                    *pixel = pixel.map(|value| value * amount);
                }
            }
            Filter::DropShadow { x, y, blur, color } => {
                let (dx, dy) = offset(matrix, x, y);
                shadow(image, (dx, dy), blur / 2.0 * scale, color)
            }
        }
    }
}

/// A color matrix as a function of unpremultiplied RGB.
fn transform(matrix: [[f32; 3]; 3]) -> impl Fn([f32; 3]) -> [f32; 3] {
    move |c| matrix.map(|row| row[0] * c[0] + row[1] * c[1] + row[2] * c[2])
}

/// Changes the unpremultiplied colors of visible pixels; alpha stays.
fn recolor(image: &mut Image, change: impl Fn([f32; 3]) -> [f32; 3]) {
    for pixel in &mut image.pixels {
        let alpha = pixel[3];
        if alpha <= 0.0 {
            continue;
        }
        let color = change([pixel[0] / alpha, pixel[1] / alpha, pixel[2] / alpha]);
        for (value, changed) in pixel.iter_mut().zip(color) {
            *value = changed.clamp(0.0, 1.0) * alpha;
        }
    }
}

/// Draws a moved, blurred and colored copy of the alpha behind the image.
fn shadow(image: &mut Image, (dx, dy): (isize, isize), sigma: f32, color: nativeboat_css::Color) {
    let (width, height) = (image.width as isize, image.height as isize);
    let paint = [
        f32::from(color.r) / 255.0 * color.a,
        f32::from(color.g) / 255.0 * color.a,
        f32::from(color.b) / 255.0 * color.a,
        color.a,
    ];
    let mut copy = Image {
        width: image.width,
        height: image.height,
        pixels: vec![[0.0; 4]; image.pixels.len()],
    };
    for y in 0.max(dy)..height.min(height + dy) {
        for x in 0.max(dx)..width.min(width + dx) {
            let alpha = image.pixels[((y - dy) * width + x - dx) as usize][3];
            copy.pixels[(y * width + x) as usize] = paint.map(|value| value * alpha);
        }
    }
    blur(&mut copy, sigma);
    for (pixel, under) in image.pixels.iter_mut().zip(&copy.pixels) {
        let rest = 1.0 - pixel[3];
        for (value, under) in pixel.iter_mut().zip(under) {
            *value += under * rest;
        }
    }
}

/// One pass of a separable blur along a line.
#[derive(Debug, Clone, PartialEq)]
enum Pass {
    /// The average over `left` pixels before, the pixel and `right` after.
    Box { left: usize, right: usize },
    /// Weights centered on the pixel.
    Kernel(Vec<f32>),
}

/// A Gaussian blur as passes: an exact kernel for small deviations, and
/// otherwise three box blurs whose sizes SVG's feGaussianBlur defines.
fn passes(sigma: f32) -> Vec<Pass> {
    if sigma < 2.0 {
        let radius = extent(sigma) as isize;
        let weights: Vec<f32> = (-radius..=radius)
            .map(|k| (-((k * k) as f32) / (2.0 * sigma * sigma)).exp())
            .collect();
        let total: f32 = weights.iter().sum();
        return vec![Pass::Kernel(
            weights.iter().map(|weight| weight / total).collect(),
        )];
    }
    let size = (sigma * 3.0 * (2.0 * core::f32::consts::PI).sqrt() / 4.0 + 0.5).floor() as usize;
    let half = size / 2;
    if size % 2 == 1 {
        vec![
            Pass::Box {
                left: half,
                right: half
            };
            3
        ]
    } else {
        vec![
            Pass::Box {
                left: half,
                right: half - 1,
            },
            Pass::Box {
                left: half - 1,
                right: half,
            },
            Pass::Box {
                left: half,
                right: half,
            },
        ]
    }
}

/// Blurs with a standard deviation in pixels; pixels beyond the image are
/// transparent.
fn blur(image: &mut Image, sigma: f32) {
    if sigma <= 0.0 {
        return;
    }
    let passes = passes(sigma);
    let (width, height) = (image.width, image.height);
    let (mut line, mut scratch) = (Vec::new(), Vec::new());
    for row in image.pixels.chunks_mut(width.max(1)) {
        line.clear();
        line.extend_from_slice(row);
        run(&passes, &mut line, &mut scratch);
        row.copy_from_slice(&line);
    }
    for column in 0..width {
        line.clear();
        line.extend((0..height).map(|row| image.pixels[row * width + column]));
        run(&passes, &mut line, &mut scratch);
        for (row, pixel) in line.iter().enumerate() {
            image.pixels[row * width + column] = *pixel;
        }
    }
}

fn run(passes: &[Pass], line: &mut Vec<[f32; 4]>, scratch: &mut Vec<[f32; 4]>) {
    let length = line.len();
    for pass in passes {
        scratch.clear();
        match pass {
            Pass::Box { left, right } => {
                let mut sums = Vec::with_capacity(length + 1);
                let mut sum = [0.0_f64; 4];
                sums.push(sum);
                for pixel in line.iter() {
                    for (total, value) in sum.iter_mut().zip(pixel) {
                        *total += f64::from(*value);
                    }
                    sums.push(sum);
                }
                let size = (left + right + 1) as f64;
                scratch.extend((0..length).map(|index| {
                    let (low, high) = (
                        sums[index.saturating_sub(*left)],
                        sums[(index + right + 1).min(length)],
                    );
                    [0, 1, 2, 3].map(|channel| ((high[channel] - low[channel]) / size) as f32)
                }));
            }
            Pass::Kernel(weights) => {
                let radius = weights.len() / 2;
                scratch.extend((0..length).map(|index| {
                    let mut value = [0.0; 4];
                    for (offset, weight) in weights.iter().enumerate() {
                        let Some(source) = (index + offset)
                            .checked_sub(radius)
                            .and_then(|source| line.get(source))
                        else {
                            continue;
                        };
                        for (total, channel) in value.iter_mut().zip(source) {
                            *total += weight * channel;
                        }
                    }
                    value
                }));
            }
        }
        core::mem::swap(line, scratch);
    }
}

#[cfg(test)]
mod tests;
