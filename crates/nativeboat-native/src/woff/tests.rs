use super::*;
use std::io::Write;

fn woff(tables: &[(&[u8; 4], &[u8], bool)]) -> Vec<u8> {
    let mut entries = Vec::new();
    let mut data = Vec::new();
    let start = 44 + 20 * tables.len();
    for (tag, bytes, compress) in tables {
        let stored = if *compress {
            let mut encoder =
                flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(bytes).unwrap();
            encoder.finish().unwrap()
        } else {
            bytes.to_vec()
        };
        entries.extend_from_slice(*tag);
        entries.extend_from_slice(&((start + data.len()) as u32).to_be_bytes());
        entries.extend_from_slice(&(stored.len() as u32).to_be_bytes());
        entries.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        entries.extend_from_slice(&7u32.to_be_bytes());
        data.extend_from_slice(&stored);
        data.resize(data.len().next_multiple_of(4), 0);
    }
    let mut file = b"wOFF".to_vec();
    file.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    file.extend_from_slice(&((start + data.len()) as u32).to_be_bytes());
    file.extend_from_slice(&(tables.len() as u16).to_be_bytes());
    file.resize(44, 0);
    file.extend_from_slice(&entries);
    file.extend_from_slice(&data);
    file
}

#[test]
fn woff_files_unwrap_into_the_fonts_they_compress() {
    let name = [0x41u8; 30];
    let file = woff(&[
        (b"head", &[1, 2, 3, 4, 5, 6], false),
        (b"name", &name, true),
    ]);
    let font = font_data(&file).unwrap();
    // The table directory: version, two tables, search fields.
    assert_eq!(font[..12], [0, 1, 0, 0, 0, 2, 0, 32, 0, 1, 0, 0]);
    assert_eq!(&font[12..16], b"head");
    assert_eq!(&font[16..20], &7u32.to_be_bytes());
    assert_eq!(&font[20..24], &44u32.to_be_bytes());
    assert_eq!(&font[24..28], &6u32.to_be_bytes());
    assert_eq!(&font[28..32], b"name");
    assert_eq!(&font[36..40], &52u32.to_be_bytes());
    // Tables start on four-byte boundaries.
    assert_eq!(font[44..52], [1, 2, 3, 4, 5, 6, 0, 0]);
    assert_eq!(font[52..82], name);
    assert_eq!(font.len(), 84);
    // Plain fonts are used as they are; other files are not fonts.
    assert_eq!(font_data(&font).unwrap(), font);
    assert_eq!(font_data(b"OTTO-font").unwrap(), b"OTTO-font");
    assert!(font_data(b"wOF2-font").is_none());
    assert!(font_data(b"<svg").is_none());
    // Damaged files are not fonts.
    let mut damaged = file.clone();
    damaged.truncate(60);
    assert!(font_data(&damaged).is_none());
}
