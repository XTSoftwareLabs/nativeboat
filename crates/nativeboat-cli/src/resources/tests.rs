use super::*;
use crate::tests::fixture::Fixture;
use std::path::PathBuf;

/// The resources of a `.res` file: type, name and data.
fn entries(res: &[u8]) -> Vec<(u16, u16, Vec<u8>)> {
    let mut found = Vec::new();
    let mut at = 0;
    while at < res.len() {
        let size = u32_at(res, at).unwrap() as usize;
        let header = u32_at(res, at + 4).unwrap() as usize;
        assert_eq!(header, 32);
        assert_eq!(u16_at(res, at + 8), Some(0xffff));
        let kind = u16_at(res, at + 10).unwrap();
        let name = u16_at(res, at + 14).unwrap();
        found.push((kind, name, res[at + header..at + header + size].to_vec()));
        at = (at + header + size).next_multiple_of(4);
    }
    found
}

fn contains_text(data: &[u8], text: &str) -> bool {
    let needle: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
    data.windows(needle.len()).any(|window| window == needle)
}

fn metadata(icon: Option<PathBuf>, description: &str) -> Metadata {
    Metadata {
        name: "notes".into(),
        product_name: "Notes".into(),
        app_name: "notes".into(),
        version: "1.2.3-beta.4".into(),
        description: description.into(),
        app_id: None,
        icon,
        locales: Vec::new(),
    }
}

fn ico(images: &[(u8, &[u8])]) -> Vec<u8> {
    let mut data = Vec::new();
    for value in [0u16, 1, images.len() as u16] {
        data.extend_from_slice(&value.to_le_bytes());
    }
    let mut offset = 6 + images.len() * 16;
    for (side, image) in images {
        data.extend_from_slice(&[*side, *side, 0, 0]);
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&32u16.to_le_bytes());
        data.extend_from_slice(&(image.len() as u32).to_le_bytes());
        data.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += image.len();
    }
    for (_, image) in images {
        data.extend_from_slice(image);
    }
    data
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut data = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    data.extend_from_slice(&width.to_be_bytes());
    data.extend_from_slice(&height.to_be_bytes());
    data.extend_from_slice(b"rest of the image");
    data
}

#[test]
fn versions_keep_up_to_four_leading_numbers() {
    assert_eq!(version_numbers("1.2.3"), [1, 2, 3, 0]);
    assert_eq!(version_numbers("10.0.0-beta.4"), [10, 0, 0, 0]);
    assert_eq!(version_numbers("1.2.3-4+build.5"), [1, 2, 3, 0]);
    assert_eq!(version_numbers("1.2.3.4.5"), [1, 2, 3, 4]);
    assert_eq!(version_numbers("70000.x.2rc"), [65535, 0, 2, 0]);
    assert_eq!(version_numbers(""), [0, 0, 0, 0]);
}

#[test]
fn executables_get_a_manifest_and_version_information() {
    let res = resources(&metadata(None, "")).unwrap();
    let found = entries(&res);
    // The leading empty entry marks a 32-bit resource file.
    assert_eq!((found[0].0, found[0].1, found[0].2.len()), (0, 0, 0));
    assert_eq!(found.len(), 3);
    let manifest = String::from_utf8(found[1].2.clone()).unwrap();
    assert_eq!((found[1].0, found[1].1), (RT_MANIFEST, 1));
    for setting in [
        "Microsoft.Windows.Common-Controls",
        "PerMonitorV2",
        "longPathAware",
        "UTF-8",
        "asInvoker",
    ] {
        assert!(manifest.contains(setting), "{setting}");
    }
    let (kind, name, version) = &found[2];
    assert_eq!((*kind, *name), (RT_VERSION, 1));
    assert_eq!(usize::from(u16_at(version, 0).unwrap()), version.len());
    assert!(contains_text(version, "VS_VERSION_INFO"));
    let fixed = version
        .windows(4)
        .position(|w| w == 0xfeef_04bdu32.to_le_bytes())
        .unwrap();
    assert_eq!(u32_at(version, fixed + 8), Some((1 << 16) | 2));
    assert_eq!(u32_at(version, fixed + 12), Some(3 << 16));
    for text in [
        "1.2.3-beta.4",
        "Notes.exe",
        "ProductName",
        "040904b0",
        "Translation",
    ] {
        assert!(contains_text(version, text), "{text}");
    }
    // Without a description the product name describes the file.
    assert!(contains_text(version, "FileDescription"));
    let described = resources(&metadata(None, "Take notes")).unwrap();
    assert!(contains_text(&entries(&described)[2].2, "Take notes"));
}

#[test]
fn icons_come_from_ico_files_or_png_images() {
    let fixture = Fixture::new(&[]);
    fixture.write("app.ico", &ico(&[(16, b"small"), (0, b"large image")]));
    let res = resources(&metadata(Some(fixture.path("app.ico")), "")).unwrap();
    let found = entries(&res);
    assert_eq!(found.len(), 6);
    assert_eq!(
        (found[3].0, found[3].1, found[3].2.as_slice()),
        (RT_ICON, 1, &b"small"[..])
    );
    assert_eq!(
        (found[4].0, found[4].1, found[4].2.as_slice()),
        (RT_ICON, 2, &b"large image"[..])
    );
    let (kind, name, group) = &found[5];
    assert_eq!((*kind, *name), (RT_GROUP_ICON, 1));
    assert_eq!(u16_at(group, 4), Some(2));
    assert_eq!(&group[6..8], &[16, 16]);
    assert_eq!(u32_at(group, 6 + 8), Some(5));
    assert_eq!(u16_at(group, 6 + 12), Some(1));
    assert_eq!(&group[20..22], &[0, 0]);
    assert_eq!(u16_at(group, 20 + 12), Some(2));

    for (side, stored) in [(256, 0u8), (48, 48)] {
        fixture.write("app.png", &png(side, side));
        let res = resources(&metadata(Some(fixture.path("app.png")), "")).unwrap();
        let group = &entries(&res)[4].2;
        assert_eq!(&group[6..8], &[stored, stored]);
        assert_eq!(u16_at(group, 6 + 6), Some(32));
    }
}

#[test]
fn unusable_icons_are_reported() {
    let fixture = Fixture::new(&[]);
    let mut truncated = ico(&[(16, b"small")]);
    truncated.truncate(truncated.len() - 2);
    for (name, data) in [
        ("large.png", png(512, 512)),
        ("empty.png", png(0, 16)),
        ("truncated.ico", truncated),
        ("none.ico", ico(&[])),
        ("cursor.ico", vec![0, 0, 2, 0, 1, 0]),
        ("text.ico", b"not an icon".to_vec()),
        ("short.png", b"\x89PNG\r\n\x1a\n".to_vec()),
    ] {
        fixture.write(name, &data);
        let error = resources(&metadata(Some(fixture.path(name)), "")).unwrap_err();
        assert!(
            error.contains("the icon must be an .ico file"),
            "{name}: {error}"
        );
    }
    let missing = fixture.path("missing.ico");
    let error = resources(&metadata(Some(missing), "")).unwrap_err();
    assert!(error.starts_with("cannot read the icon"), "{error}");
}
