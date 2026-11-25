extern crate std;

use super::*;
use std::format;

#[test]
fn data_urls_decode_base64_and_percent_encoded_data() {
    let page = "file:///C:/app/index.html";
    assert_eq!(
        fetch(page, "data:font/woff;base64,AQID").unwrap(),
        [1, 2, 3]
    );
    assert_eq!(fetch(page, "DATA:text/plain;BASE64,aGk=\n").unwrap(), b"hi");
    assert_eq!(fetch(page, "data:,a%20b%2").unwrap(), b"a b%2");
    assert!(fetch(page, "data:;base64,a*b").is_none());
    assert!(fetch(page, "data:no-comma").is_none());
}

#[test]
fn file_urls_and_relative_urls_name_windows_paths() {
    let page = "file:///C:/app/out/renderer/index.html?x=1#top";
    assert_eq!(
        file_path(page, "file:///C:/doc/a%20b.png").unwrap(),
        r"C:\doc\a b.png"
    );
    assert_eq!(
        file_path(page, "file://server/share/x.png").unwrap(),
        r"\\server\share\x.png"
    );
    assert_eq!(
        file_path(page, r"D:\pictures\cat.png").unwrap(),
        r"D:\pictures\cat.png"
    );
    assert_eq!(
        file_path(page, "C:/pictures/cat.png").unwrap(),
        r"C:\pictures\cat.png"
    );
    assert_eq!(
        file_path(page, "fonts/x.woff?v=2").unwrap(),
        r"C:\app\out\renderer\fonts\x.woff"
    );
    assert_eq!(
        file_path(page, "../../logo.png").unwrap(),
        r"C:\app\logo.png"
    );
    assert_eq!(
        file_path(page, "./a/./b/../c.png").unwrap(),
        r"C:\app\out\renderer\a\c.png"
    );
    assert!(file_path(page, "https://example.com/x.png").is_none());
    assert!(file_path(page, "").is_none());
    // A page that is not a file has no base.
    assert!(file_path("https://example.com/", "x.png").is_none());
}

#[test]
fn files_are_read_from_disk() {
    let directory = std::env::temp_dir().join(format!("nativeboat-assets-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let file = directory.join("asset.bin");
    std::fs::write(&file, [9u8, 8, 7]).unwrap();
    let page = format!("file:///{}", directory.join("index.html").display()).replace('\\', "/");
    assert_eq!(fetch(&page, "asset.bin").unwrap(), [9, 8, 7]);
    assert!(fetch(&page, "missing.bin").is_none());
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn font_face_rules_give_their_family_weight_style_and_sources() {
    let rule = |text: &str| nativeboat_css::parser::parse_declaration_list(text);
    let face = font_face(&rule(
        "font-family: 'Open Sans'; font-weight: 300; font-style: italic; \
         src: local('Open Sans Light'), url(\"data:font/woff;base64,AA==\") format('woff'), url(fonts/x.ttf)",
    ))
    .unwrap();
    assert_eq!(
        (face.family.as_str(), face.weights, face.italic),
        ("Open Sans", (300, 300), true)
    );
    assert_eq!(face.urls, ["data:font/woff;base64,AA==", "fonts/x.ttf"]);
    assert!(face.ranges.is_empty());
    let face = font_face(&rule("font-family: X; font-weight: bold; src: url(a)")).unwrap();
    assert_eq!((face.weights, face.italic), ((700, 700), false));
    // A variable font serves a range of weights.
    let face = font_face(&rule("font-family: X; font-weight: 200 800; src: url(a)")).unwrap();
    assert_eq!(face.weights, (200, 800));
    assert!(font_face(&rule("src: url(a)")).is_none());
}

#[test]
fn font_face_unicode_ranges_take_every_form() {
    let rule = |text: &str| nativeboat_css::parser::parse_declaration_list(text);
    let face = font_face(&rule(
        "font-family: Inter; src: url(a); \
         unicode-range: U+0000-00FF, U+0131, u+4??, U+2000-206F, U+FFFF-0, nope, U+110000",
    ))
    .unwrap();
    assert_eq!(
        face.ranges,
        [(0, 0xFF), (0x131, 0x131), (0x400, 0x4FF), (0x2000, 0x206F)]
    );
}

/// URLs of other schemes resolve against their base as hierarchical URLs
/// do, as a `vscode-file:` page's style sheets and images resolve.
#[test]
fn relative_urls_join_absolute_urls() {
    let page = "vscode-file://vscode-app/c:/app/out/vs/code/workbench/workbench.html?x=1#top";
    let join = |url| join_url(page, url);
    assert_eq!(
        join("../../media/a.css").as_deref(),
        Some("vscode-file://vscode-app/c:/app/out/vs/media/a.css")
    );
    assert_eq!(
        join("./b.png?v=2").as_deref(),
        Some("vscode-file://vscode-app/c:/app/out/vs/code/workbench/b.png?v=2")
    );
    assert_eq!(
        join("/c:/other/c.ttf").as_deref(),
        Some("vscode-file://vscode-app/c:/other/c.ttf")
    );
    assert_eq!(
        join("../../../../../../../up.css").as_deref(),
        Some("vscode-file://vscode-app/up.css")
    );
    assert_eq!(
        join("dir/").as_deref(),
        Some("vscode-file://vscode-app/c:/app/out/vs/code/workbench/dir/")
    );
    assert_eq!(
        join_url("app:folder/page.html", "x.css").as_deref(),
        Some("app:/folder/x.css")
    );
    assert_eq!(
        join_url("app://host", "x.css").as_deref(),
        Some("app://host/x.css")
    );
    // Drives and paths are not URLs to join against.
    assert_eq!(join_url(r"C:\app\index.html", "x.css"), None);
    // Windows paths, as rebased style sheet URLs, are files, not URLs.
    assert!(windows_path("C:/app/img/dot.png"));
    assert!(windows_path(r"C:\app\img\dot.png"));
    assert!(windows_path(r"\\server\share\a.png"));
    assert!(!windows_path("img/dot.png"));
    assert!(!windows_path("app://./img/dot.png"));
    assert!(!windows_path("c:"));
    assert_eq!(served_url(page, "C:/app/img/dot.png"), None);
    // Without a served scheme, nothing is served.
    assert_eq!(served_url(page, "a.css"), None);
    assert_eq!(served_url(page, "vscode-file://vscode-app/c:/a.css"), None);
    assert_eq!(locate(page, "a.css"), None);
}
