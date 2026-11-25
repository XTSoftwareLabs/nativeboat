use super::*;

fn project(feed: Option<serde_json::Value>) -> (crate::tests::fixture::Fixture, Project) {
    let fixture = crate::tests::fixture::Fixture::new(&[
        (
            "package.json",
            r#"{"name":"my-app","productName":"My App","version":"1.2.0","author":"Ada <ada@example.com>","main":"main.js","devDependencies":{"electron":"44"},"build":{"appId":"com.example.my-app","nsis":{"deleteAppDataOnUninstall":true,"createDesktopShortcut":false}}}"#,
        ),
        ("main.js", ""),
    ]);
    let mut project = nativeboat_project::load(fixture.root()).unwrap();
    project.updater_config = feed;
    (fixture, project)
}

#[test]
fn installers_go_where_asked_or_beside_outputs_that_update() {
    let (_fixture, plain) = project(None);
    let output = plain.root.join("dist");
    assert_eq!(directory(&plain, &output, None), None);
    assert_eq!(
        directory(&plain, &output, Some(Path::new("setup"))),
        Some(plain.root.join("setup"))
    );
    let (_fixture, updating) = project(Some(serde_json::json!({"provider": "generic"})));
    assert_eq!(
        directory(&updating, &updating.root.join("dist"), None),
        Some(updating.root.join("dist-setup"))
    );
}

#[test]
fn settings_describe_the_install() {
    let (_fixture, mut project) = project(None);
    project.metadata.name = "my:app\n".into();
    assert_eq!(
        settings(&project, "My App.exe"),
        "mode=install\nproduct=My App\nshortcut=My App\nexe=My App.exe\nversion=1.2.0\n\
         appid=com.example.my-app\npublisher=Ada\nfolder=my-app-\ndata=My App\ndesktop=0\n\
         startmenu=1\nrun=1\nappdata=1\n"
    );
}

#[test]
fn payloads_follow_the_setup_program() {
    let (fixture, _) = project(None);
    fixture.write("app/a.txt", b"hello hello hello");
    fixture.write("app/sub/b.bin", b"");
    fixture.write("setup.exe", b"MZ-image");
    let mut names = Vec::new();
    files(&fixture.path("app"), &fixture.path("app"), &mut names).unwrap();
    names.sort();
    assert_eq!(names, ["a.txt", "sub\\b.bin"]);
    append_payload(
        &fixture.path("setup.exe"),
        "k=v\n",
        &fixture.path("app"),
        &names,
    )
    .unwrap();
    let bytes = std::fs::read(fixture.path("setup.exe")).unwrap();
    let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
    let u64_at = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize;
    assert_eq!(&bytes[bytes.len() - 8..], END_MAGIC);
    let start = u64_at(bytes.len() - 16);
    assert_eq!(start, 8);
    assert_eq!(&bytes[start..start + 8], MAGIC);
    assert_eq!(u32_at(start + 8), 1);
    assert_eq!(&bytes[start + 16..start + 20], b"k=v\n");
    assert_eq!(u32_at(start + 20), 2);
    let entry = start + 24;
    assert_eq!(&bytes[entry + 4..entry + 9], b"a.txt");
    assert_eq!(u64_at(entry + 9), 17);
    let compressed = u64_at(entry + 17);
    let data = &bytes[entry + 25..entry + 25 + compressed];
    let mut inflated = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::DeflateDecoder::new(data), &mut inflated)
        .unwrap();
    assert_eq!(inflated, b"hello hello hello");
    let missing = append_payload(
        &fixture.path("setup.exe"),
        "",
        &fixture.path("app"),
        &["gone".into()],
    );
    assert!(missing.unwrap_err().contains("cannot read"));
    assert!(
        append_payload(
            &fixture.path("absent/setup.exe"),
            "",
            &fixture.path("app"),
            &[]
        )
        .is_err()
    );
    assert!(files(&fixture.path("absent"), &fixture.path("absent"), &mut names).is_err());
}

#[test]
fn update_metadata_names_the_installer_and_its_digest() {
    let digest = sha512(b"abc").unwrap();
    assert_eq!(
        digest[..8],
        [0xdd, 0xaf, 0x35, 0xa1, 0x93, 0x61, 0x7a, 0xba]
    );
    assert_eq!(
        release_date(std::time::UNIX_EPOCH + std::time::Duration::from_millis(951_782_400_123)),
        "2000-02-29T00:00:00.123Z"
    );
    assert_eq!(
        release_date(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_793_446_245)),
        "2026-10-31T11:30:45.000Z"
    );
    let info = update_info(
        "1.2.0",
        "My-App-Setup-1.2.0.exe",
        &[0, 1, 2],
        42,
        "2026-10-31T00:00:00.000Z",
    );
    assert_eq!(
        info,
        "version: 1.2.0\nfiles:\n- url: My-App-Setup-1.2.0.exe\n  sha512: AAEC\n  size: 42\n\
         path: My-App-Setup-1.2.0.exe\nsha512: AAEC\nreleaseDate: 2026-10-31T00:00:00.000Z\n"
    );
}
