//! Image clipboard interoperability. DIB bytes are bounded by the allocation,
//! and native bitmap pixels use premultiplied BGRA rather than canvas RGBA.

use super::{Open, read_open, registered_format, text_entries, write};
#[allow(unused_imports)]
use crate::prelude::*;
use crate::{Error, add_image, image_encode, image_from_pixels, image_pixels};
use core::ptr;
use windows_sys::Win32::System::Ole::{CF_DIB, CF_DIBV5};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
const MAX_PIXELS_BYTES: usize = 256 * 1024 * 1024;

/// Copies the clipboard image into the owned image cache under `url`.
/// PNG takes precedence; a malformed nonempty PNG produces an empty image.
pub fn read_clipboard_image(url: &str) -> Option<(f32, f32)> {
    let _open = Open::new(ptr::null_mut()).ok()?;
    if let Ok(format) = registered_format("PNG")
        && let Some(data) = read_open(format)
        && !data.is_empty()
    {
        return data
            .starts_with(PNG)
            .then(|| add_image(url, &data))
            .flatten();
    }
    for format in [CF_DIB, CF_DIBV5] {
        if let Some(data) = read_open(u32::from(format))
            && let Some((size, pixels)) = decode_dib(&data)
        {
            return image_from_pixels(url, size, &pixels);
        }
    }
    None
}

/// Replaces the clipboard with an image and optional text/HTML in one write.
/// An empty NativeImage clears the clipboard, preserving explicit text/HTML.
pub fn write_clipboard_image(
    image: Option<&str>,
    text: Option<&str>,
    html: Option<&str>,
) -> Result<(), Error> {
    let mut entries = Vec::new();
    if let Some(image) = image {
        let bitmap = image_pixels(image).and_then(|(w, h, data)| encode_dib((w, h), &data));
        let png = image_encode(image, false);
        let (Some(bitmap), Some(png)) = (bitmap, png) else {
            return Err(Error {
                message: "Could not encode clipboard image".into(),
            });
        };
        // PNG is advertised first so receivers retain its exact transparency.
        entries.push((registered_format("PNG")?, png));
        entries.push((u32::from(CF_DIBV5), bitmap));
    }
    entries.extend(text_entries(text, html));
    write(&entries)
}

fn field(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        data.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn component(value: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 255;
    }
    let shift = mask.trailing_zeros();
    let maximum = u64::from(mask >> shift);
    ((u64::from((value & mask) >> shift) * 255 + maximum / 2) / maximum) as u8
}

/// Uncompressed Windows INFO/V2/V3/V4/V5 DIBs: indexed 1/4/8-bit, RGB555,
/// RGB/BGR24 and 16/32-bit bitfields. RLE and legacy CORE headers are rejected.
fn decode_dib(data: &[u8]) -> Option<((u32, u32), Vec<u8>)> {
    let header = field(data, 0)? as usize;
    if !matches!(header, 40 | 52 | 56 | 108 | 124) || data.len() < header {
        return None;
    }
    let width = field(data, 4)? as i32;
    let signed_height = field(data, 8)? as i32;
    let height = signed_height.checked_abs()?;
    let planes = u16::from_le_bytes(data[12..14].try_into().ok()?);
    let depth = u16::from_le_bytes(data[14..16].try_into().ok()?);
    let compression = field(data, 16)?;
    if width <= 0
        || height == 0
        || planes != 1
        || !matches!(depth, 1 | 4 | 8 | 16 | 24 | 32)
        || !matches!(compression, 0 | 3 | 6)
        || (compression != 0 && !matches!(depth, 16 | 32))
        || (compression == 6 && header == 52)
    {
        return None;
    }
    let (width, height) = (width as usize, height as usize);
    let stride = width.checked_mul(usize::from(depth))?.checked_add(31)? / 32 * 4;
    let length = width.checked_mul(height)?.checked_mul(4)?;
    if length > MAX_PIXELS_BYTES {
        return None;
    }
    let mut offset = header;
    let mut masks = if depth == 16 {
        [0x7c00, 0x03e0, 0x001f, 0]
    } else {
        [0xff0000, 0xff00, 0xff, 0xff000000]
    };
    if compression != 0 {
        let count = if compression == 6 || header >= 56 {
            4
        } else {
            3
        };
        for (index, mask) in masks.iter_mut().enumerate() {
            *mask = if index < count {
                field(data, 40 + index * 4)?
            } else {
                0
            };
        }
        if header == 40 {
            offset += count * 4;
        }
        let mut used = 0u32;
        for (index, mask) in masks.iter().copied().enumerate() {
            if mask == 0 {
                if index == 3 {
                    continue;
                }
                return None;
            }
            let bits = u64::from(mask >> mask.trailing_zeros());
            if bits & (bits + 1) != 0 || u64::from(mask) >= (1u64 << depth) || used & mask != 0 {
                return None;
            }
            used |= mask;
        }
    }
    let declared_colors = field(data, 32)? as usize;
    let colors = if depth <= 8 && declared_colors == 0 {
        1usize << depth
    } else {
        declared_colors
    };
    if depth <= 8 && colors > 1usize << depth {
        return None;
    }
    let palette_end = offset.checked_add(colors.checked_mul(4)?)?;
    let palette = data.get(offset..palette_end)?;
    offset = palette_end;
    let pixels = data.get(offset..offset.checked_add(stride.checked_mul(height)?)?)?;
    if header == 124 {
        let profile = field(data, 112)? as usize;
        let size = field(data, 116)? as usize;
        if size != 0
            && (profile < offset + pixels.len()
                || data.get(profile..profile.checked_add(size)?).is_none())
        {
            return None;
        }
    }
    let mut rgba = Vec::with_capacity(length);
    let mut bad_alpha = false;
    for y in 0..height {
        let row = if signed_height < 0 { y } else { height - 1 - y };
        let row = &pixels[row * stride..(row + 1) * stride];
        for x in 0..width {
            let pixel = match depth {
                1 | 4 | 8 => {
                    let bit = x * usize::from(depth);
                    let index = usize::from(
                        (row[bit / 8] >> (8 - usize::from(depth) - bit % 8))
                            & ((1u16 << depth) - 1) as u8,
                    );
                    let color = palette.get(index * 4..index * 4 + 4)?;
                    [color[2], color[1], color[0], 255]
                }
                24 => [row[x * 3 + 2], row[x * 3 + 1], row[x * 3], 255],
                _ => {
                    let value = if depth == 16 {
                        u32::from(u16::from_le_bytes(row[x * 2..x * 2 + 2].try_into().ok()?))
                    } else {
                        field(row, x * 4)?
                    };
                    let mut pixel = masks.map(|mask| component(value, mask));
                    if depth == 16 {
                        pixel[3] = 255;
                    }
                    bad_alpha |= pixel[..3].iter().any(|color| *color > pixel[3]);
                    pixel
                }
            };
            rgba.extend_from_slice(&pixel);
        }
    }
    for pixel in rgba.chunks_exact_mut(4) {
        if bad_alpha {
            pixel[3] = 255;
        }
        let alpha = u32::from(pixel[3]);
        for color in &mut pixel[..3] {
            *color = (u32::from(*color) * 255 + alpha / 2)
                .checked_div(alpha)
                .unwrap_or_default()
                .min(255) as u8;
        }
    }
    Some(((width as u32, height as u32), rgba))
}

fn encode_dib((width, height): (u32, u32), rgba: &[u8]) -> Option<Vec<u8>> {
    let width = i32::try_from(width).ok()?;
    let height = i32::try_from(height).ok()?;
    let length = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    if width == 0 || height == 0 || length != rgba.len() || length > MAX_PIXELS_BYTES {
        return None;
    }
    let mut dib = vec![0; 124 + length];
    for (offset, value) in [
        (0, 124),
        (4, width as u32),
        (8, height as u32),
        (12, 1 | (32 << 16)),
        (20, length as u32),
        (52, 0xff000000),
        (56, 0x57696e20),
        (108, 4),
    ] {
        dib[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    let stride = width as usize * 4;
    for y in 0..height as usize {
        let source = &rgba[y * stride..(y + 1) * stride];
        let target = &mut dib
            [124 + (height as usize - 1 - y) * stride..124 + (height as usize - y) * stride];
        for (from, to) in source.chunks_exact(4).zip(target.chunks_exact_mut(4)) {
            let alpha = u16::from(from[3]);
            to.copy_from_slice(&[
                ((u16::from(from[2]) * alpha + 127) / 255) as u8,
                ((u16::from(from[1]) * alpha + 127) / 255) as u8,
                ((u16::from(from[0]) * alpha + 127) / 255) as u8,
                from[3],
            ]);
        }
    }
    Some(dib)
}

#[cfg(test)]
mod tests;
