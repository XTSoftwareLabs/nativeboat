//! Decodes WebP images as Chromium does, with libwebp's algorithms, so they
//! show on every system, whether or not Windows has a codec for them, and
//! with Chromium's pixels.

#[allow(unused_imports)]
use crate::prelude::*;

mod lossless;
mod lossy;
mod tables;

/// The largest image decoded, in bytes of pixels.
const MAX_PIXELS_BYTES: u64 = 256 * 1024 * 1024;

/// Whether an image this size is small enough to decode.
fn fits(width: u32, height: u32) -> bool {
    u64::from(width) * u64::from(height) * 4 <= MAX_PIXELS_BYTES
}

/// Whether `bytes` are a WebP file.
pub(crate) fn is_webp(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP"
}

/// The chunks of a RIFF body: their tags and payloads. A truncated last
/// chunk keeps what there is of it.
fn chunks(mut body: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> + '_ {
    core::iter::from_fn(move || {
        let header = body.get(..8)?;
        let tag = [header[0], header[1], header[2], header[3]];
        let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        let payload = &body[8..];
        let payload = &payload[..size.min(payload.len())];
        // Chunks are padded to an even size.
        body = body.get(8 + size + (size & 1)..).unwrap_or(&[]);
        Some((tag, payload))
    })
}

fn u24(bytes: &[u8]) -> u32 {
    u32::from(bytes[0]) | u32::from(bytes[1]) << 8 | u32::from(bytes[2]) << 16
}

/// The size and straight-alpha RGBA pixels of a WebP file, of its first
/// frame when animated; `None` when it is not one or is broken.
pub(crate) fn decode(bytes: &[u8]) -> Option<((u32, u32), Vec<u8>)> {
    if !is_webp(bytes) {
        return None;
    }
    let size = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let body = &bytes[12..bytes.len().min(8 + size).max(12)];
    let mut chunks = chunks(body);
    let (tag, payload) = chunks.next()?;
    if &tag != b"VP8X" {
        return image(core::iter::once((tag, payload)));
    }
    let header = payload.get(..10)?;
    let animated = header[0] & 0x02 != 0;
    let canvas = (u24(&header[4..]) + 1, u24(&header[7..]) + 1);
    if !animated {
        // A still image fills its canvas exactly.
        return image(chunks).filter(|(size, _)| *size == canvas);
    }
    // The first frame, on a transparent canvas, as Chromium shows it first
    // and ignores the background color.
    let (_, frame) = chunks.find(|(tag, _)| tag == b"ANMF")?;
    let header = frame.get(..16)?;
    let (x, y) = (u24(header) as usize * 2, u24(&header[3..]) as usize * 2);
    let ((width, height), pixels) = image(self::chunks(&frame[16..]))?;
    if !fits(canvas.0, canvas.1) {
        return None;
    }
    let (canvas_width, canvas_height) = (canvas.0 as usize, canvas.1 as usize);
    let mut rgba = vec![0_u8; canvas_width * canvas_height * 4];
    let (width, height) = (width as usize, height as usize);
    for row in 0..height.min(canvas_height.saturating_sub(y)) {
        let columns = width.min(canvas_width.saturating_sub(x));
        let from = row * width * 4;
        let to = ((y + row) * canvas_width + x) * 4;
        rgba[to..to + columns * 4].copy_from_slice(&pixels[from..from + columns * 4]);
    }
    Some((canvas, rgba))
}

/// Decodes the bitstream among an image's chunks, with its alpha.
fn image<'a>(chunks: impl Iterator<Item = ([u8; 4], &'a [u8])>) -> Option<((u32, u32), Vec<u8>)> {
    let mut alpha = None;
    for (tag, payload) in chunks {
        match &tag {
            b"ALPH" if alpha.is_none() => alpha = Some(payload),
            b"VP8 " => return lossy::decode(payload, alpha),
            b"VP8L" => return lossless::decode(payload),
            _ => {}
        }
    }
    None
}

/// The alpha plane an ALPH chunk holds, as libwebp decodes it: raw or
/// losslessly compressed, then unfiltered row by row.
fn alpha_plane(data: &[u8], (width, height): (usize, usize)) -> Option<Vec<u8>> {
    let (&header, data) = data.split_first()?;
    let method = header & 0x03;
    let filter = (header >> 2) & 0x03;
    let preprocessing = (header >> 4) & 0x03;
    if method > 1 || preprocessing > 1 || header >> 6 != 0 || data.is_empty() {
        return None;
    }
    let mut plane = if method == 0 {
        data.get(..width * height)?.to_vec()
    } else {
        lossless::decode_alpha(data, (width, height))?
    };
    if filter != 0 {
        for row in 0..height {
            let (done, rest) = plane.split_at_mut(row * width);
            let previous = (row > 0).then(|| &done[done.len() - width..]);
            unfilter(filter, previous, &mut rest[..width]);
        }
    }
    Some(plane)
}

/// Undoes an alpha row's horizontal (1), vertical (2) or gradient (3)
/// prediction; the first row is predicted horizontally whatever the filter.
fn unfilter(filter: u8, previous: Option<&[u8]>, row: &mut [u8]) {
    match (filter, previous) {
        (2, Some(previous)) => {
            for (value, above) in row.iter_mut().zip(previous) {
                *value = value.wrapping_add(*above);
            }
        }
        (3, Some(previous)) => {
            let mut left = previous[0];
            let mut top_left = previous[0];
            for (value, &top) in row.iter_mut().zip(previous) {
                let prediction =
                    (i32::from(left) + i32::from(top) - i32::from(top_left)).clamp(0, 255);
                left = value.wrapping_add(prediction as u8);
                top_left = top;
                *value = left;
            }
        }
        _ => {
            let mut prediction = previous.map_or(0, |previous| previous[0]);
            for value in row {
                *value = value.wrapping_add(prediction);
                prediction = *value;
            }
        }
    }
}

#[cfg(test)]
mod tests;
