//! Font files as pages load them: TrueType and OpenType fonts, and WOFF
//! files, whose zlib-compressed tables become the font they wrap.

use alloc::vec::Vec;

/// The TrueType or OpenType font in a font file: the file itself, or the
/// font a WOFF file wraps. `None` for other files, including WOFF 2.
pub fn font_data(file: &[u8]) -> Option<Vec<u8>> {
    match file.get(..4)? {
        b"wOFF" => unwrap_woff(file),
        [0, 1, 0, 0] | b"OTTO" | b"true" => Some(file.to_vec()),
        _ => None,
    }
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

/// Rebuilds the font a WOFF file compresses: its table directory, then each
/// table, inflated when stored compressed, padded to four bytes.
fn unwrap_woff(file: &[u8]) -> Option<Vec<u8>> {
    const HEADER: usize = 44;
    const ENTRY: usize = 20;
    let flavor = u32_at(file, 4)?;
    let count = usize::from(u16_at(file, 12)?);
    let mut tables = Vec::with_capacity(count);
    for index in 0..count {
        let entry = HEADER + ENTRY * index;
        let tag = file.get(entry..entry + 4)?;
        let offset = u32_at(file, entry + 4)? as usize;
        let compressed = u32_at(file, entry + 8)? as usize;
        let length = u32_at(file, entry + 12)? as usize;
        let checksum = u32_at(file, entry + 16)?;
        let stored = file.get(offset..offset.checked_add(compressed)?)?;
        let data = if compressed < length {
            let data =
                miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(stored, length).ok()?;
            (data.len() == length).then_some(data)?
        } else {
            stored.to_vec()
        };
        tables.push((tag, checksum, data));
    }
    let selector = if count == 0 { 0 } else { count.ilog2() as u16 };
    let search = (1u16 << selector) * 16;
    let mut font = Vec::new();
    font.extend_from_slice(&flavor.to_be_bytes());
    font.extend_from_slice(&(count as u16).to_be_bytes());
    font.extend_from_slice(&search.to_be_bytes());
    font.extend_from_slice(&selector.to_be_bytes());
    font.extend_from_slice(&((count as u16) * 16 - search).to_be_bytes());
    let mut offset = 12 + 16 * count;
    for (tag, checksum, data) in &tables {
        font.extend_from_slice(tag);
        font.extend_from_slice(&checksum.to_be_bytes());
        font.extend_from_slice(&(offset as u32).to_be_bytes());
        font.extend_from_slice(&(data.len() as u32).to_be_bytes());
        offset += data.len().next_multiple_of(4);
    }
    for (_, _, data) in &tables {
        font.extend_from_slice(data);
        font.resize(font.len().next_multiple_of(4), 0);
    }
    Some(font)
}

#[cfg(test)]
mod tests;
