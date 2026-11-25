use super::*;
use nativeboat_html::parse_document;

fn page(paths: &[&str]) -> Page {
    Page {
        runtime_paths: paths.iter().map(|path| (*path).to_owned()).collect(),
        document: parse_document("<p>x</p>"),
        styles: vec![Style {
            path: "src/a.css".to_owned(),
            sheet: nativeboat_css::parse_stylesheet("p { color: red }").0,
        }],
        scripts: Script::default(),
        module_entries: Vec::new(),
    }
}

fn sample() -> AppPackage {
    let mut package = AppPackage::new(Metadata {
        name: "demo".to_owned(),
        product_name: "Demo".to_owned(),
        version: "1.0.0".to_owned(),
        description: String::new(),
        app_id: Some("com.demo".to_owned()),
    });
    package.main.runtime_paths = vec!["main.js".to_owned()];
    package
        .main
        .env
        .insert("PROD".to_owned(), serde_json::Value::Bool(true));
    package.preloads.push(Script {
        runtime_paths: vec!["dist-electron/preload.js".to_owned()],
        ..Script::default()
    });
    package.pages.push(page(&["dist/index.html"]));
    package.pages.push(page(&["dist/settings.html"]));
    package.pages.push(page(&["a/dup.html"]));
    package.pages.push(page(&["b/dup.html"]));
    package.assets.push(Asset {
        url: "/src/logo.png".to_owned(),
        data: vec![0, 1, 2, 255],
    });
    package
}

#[test]
fn packages_round_trip() {
    let package = sample();
    let bytes = package.to_bytes();
    assert_eq!(AppPackage::from_bytes(&bytes).unwrap(), package);
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("\"data\":\"AAEC/w==\""));
    assert!(
        AppPackage::from_bytes(b"{")
            .unwrap_err()
            .starts_with("corrupt application package")
    );
    let mut old = package.clone();
    old.format = 0;
    assert_eq!(
        AppPackage::from_bytes(&old.to_bytes()).unwrap_err(),
        "application package format 0 is not supported (expected 1)"
    );
    let invalid = text.replace("AAEC/w==", "!!");
    assert!(AppPackage::from_bytes(invalid.as_bytes()).is_err());
}

#[test]
fn lookups() {
    let package = sample();
    let first = |path: &str| package.page(path).map(|page| page.runtime_paths[0].clone());
    assert_eq!(first("dist/index.html").as_deref(), Some("dist/index.html"));
    assert_eq!(
        first("dist\\..\\dist\\.\\settings.html").as_deref(),
        Some("dist/settings.html")
    );
    assert_eq!(
        first("/C:/app/elsewhere/index.html").as_deref(),
        Some("dist/index.html")
    );
    assert_eq!(first("dup.html"), None);
    assert_eq!(first("missing.html"), None);
    assert!(package.preload("dist-electron/preload.js").is_some());
    assert!(package.preload("preload.js").is_none());
    assert_eq!(
        package.asset("/src/logo.png?v=1").unwrap().data,
        vec![0, 1, 2, 255]
    );
    assert!(package.asset("src/logo.png#x").is_some());
    assert!(package.asset("/nope.png").is_none());
    assert_eq!(normalize_runtime_path("./a//b/../c"), "a/c");
    assert_eq!(normalize_runtime_path("../../a"), "a");
}

#[test]
fn payloads_embed_and_extract() {
    let package = sample();
    let executable = embed(b"MZstub", &package);
    assert!(executable.starts_with(b"MZstub"));
    assert!(executable.ends_with(MAGIC));
    assert_eq!(extract(&executable).unwrap(), Some(package.clone()));
    assert_eq!(extract(b"MZ").unwrap(), None);
    assert_eq!(extract(b"0123456789abcdefgh").unwrap(), None);
    let mut broken = Vec::new();
    broken.extend_from_slice(&u64::MAX.to_le_bytes());
    broken.extend_from_slice(MAGIC);
    assert_eq!(
        extract(&broken).unwrap_err(),
        "corrupt application package: invalid length"
    );
}

#[test]
fn executables_are_written_and_read() {
    let directory = std::env::temp_dir().join(format!("nativeboat-ir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let stub = directory.join("stub.exe");
    std::fs::write(&stub, b"MZruntime").unwrap();
    let output = directory.join("dist/nested/App.exe");
    let package = sample();
    write_executable(&stub, &output, &package).unwrap();
    assert_eq!(read_executable(&output).unwrap(), Some(package.clone()));
    assert_eq!(read_executable(&stub).unwrap(), None);
    let error = write_executable(&output, &directory.join("again.exe"), &package).unwrap_err();
    assert!(error.ends_with("already contains an application"));
    let error = write_executable(&directory.join("missing.exe"), &output, &package).unwrap_err();
    assert!(error.starts_with("cannot read runtime "));
    let error = read_executable(&directory.join("missing.exe")).unwrap_err();
    assert!(error.starts_with("cannot read "));
    std::fs::write(directory.join("file"), b"").unwrap();
    let error = write_executable(&stub, &directory.join("file/App.exe"), &package).unwrap_err();
    assert!(error.starts_with("cannot create "));
    let error = write_executable(&stub, &directory.join("dist"), &package).unwrap_err();
    assert!(error.starts_with("cannot write "));
    let error = write_executable(&stub, Path::new(""), &package).unwrap_err();
    assert!(error.starts_with("cannot write "));
    std::fs::remove_dir_all(&directory).unwrap();
}
