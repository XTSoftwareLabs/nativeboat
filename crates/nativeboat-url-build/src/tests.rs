use super::*;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug)]
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "nativeboat-url-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let resolved = self.0.canonicalize().unwrap();
        let parent = std::env::temp_dir().canonicalize().unwrap();
        assert_eq!(resolved.parent(), Some(parent.as_path()));
        assert!(
            resolved
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("nativeboat-url-")
        );
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn rustc(args: &[OsString], directory: &Path) -> Result<(), String> {
    let output = Command::new("rustc")
        .args(args)
        .current_dir(directory)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

#[test]
fn packaged_build_reports_filesystem_and_compiler_errors() {
    let error = build(Path::new(""), &mut |_, _| panic!("compiler must not run")).unwrap_err();
    assert!(error.contains("empty path"));
    let scratch = Scratch::new();
    let file = scratch.0.join("file");
    std::fs::write(&file, "occupied").unwrap();
    let error = build(&file, &mut |_, _| panic!("compiler must not run")).unwrap_err();
    assert!(error.contains("cannot create source directory"));
    let directory = scratch.0.join("write-failure");
    std::fs::create_dir_all(directory.join(sources::SOURCES[0].0)).unwrap();
    let error = build(&directory, &mut |_, _| panic!("compiler must not run")).unwrap_err();
    assert!(error.contains("cannot write packaged source"));
    let directory = scratch.0.join("compiler-failure");
    let error = build(&directory, &mut |args, working| {
        assert_eq!(working, directory);
        assert_eq!(args[0], "unicode-joining-type-1.0.0/src/lib.rs");
        Err("selected compiler failed".into())
    })
    .unwrap_err();
    assert_eq!(error, "selected compiler failed");
}

#[test]
fn packaged_source_and_arguments_are_complete_and_no_std() {
    let scratch = Scratch::new();
    let mut calls = 0;
    let library = build(&scratch.0, &mut |args, directory| {
        let specification = &LIBRARIES[calls];
        assert!(directory.join(&args[0]).is_file());
        assert!(args.iter().any(|arg| arg == "-Cpanic=abort"));
        assert!(!args.iter().any(|arg| arg == "feature=\"std\""));
        for dependency in specification.dependencies {
            assert!(args.contains(&format!("--extern={dependency}=lib{dependency}.rlib").into()));
        }
        calls += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(calls, 12);
    assert_eq!(library, scratch.0.join("liburl.rlib"));
    let final_sources: std::collections::BTreeMap<_, _> =
        sources::SOURCES.iter().copied().collect();
    for (name, source) in final_sources {
        assert_eq!(
            std::fs::read(scratch.0.join(name)).unwrap(),
            source.as_bytes()
        );
    }
    let joining = std::fs::read_to_string(
        scratch
            .0
            .join("unicode-joining-type-1.0.0/src/joining_type.rs"),
    )
    .unwrap();
    assert!(joining.contains("include!(\"generated/joining_type.rs\")"));
    assert!(!joining.contains("env!(\"OUT_DIR\")"));
    for name in [
        "url-2.5.8",
        "idna-1.1.0",
        "idna_mapping-1.1.0",
        "unicode-joining-type-1.0.0",
        "unicode-normalization-0.1.24",
        "unicode-bidi-0.3.18",
    ] {
        assert!(NOTICES.contains(name), "{name}");
    }
    assert!(NOTICES.contains("UNICODE LICENSE V3"));
    // The fingerprint is stable and covers sources and build arguments.
    assert_eq!(fingerprint(), fingerprint());
    assert_ne!(fingerprint(), 0xcbf2_9ce4_8422_2325);
}

#[cfg(windows)]
#[test]
fn packaged_url_compiles_and_runs_without_cargo_std_or_crt() {
    let scratch = Scratch::new();
    let library = build(&scratch.0, &mut rustc).unwrap();
    assert!(library.is_file());
    std::fs::write(
        scratch.0.join("start.rs"),
        include_str!("../../nativeboat-start/src/lib.rs"),
    )
    .unwrap();
    std::fs::write(
        scratch.0.join("NOTICES.txt"),
        format!(
            "{}\n{NOTICES}",
            include_str!("../../nativeboat-start/src/NOTICES.txt")
        ),
    )
    .unwrap();
    rustc(
        &[
            "start.rs".into(),
            "--crate-name=nativeboat_start".into(),
            "--crate-type=rlib".into(),
            "--edition=2024".into(),
            "-Cpanic=abort".into(),
            "-Copt-level=2".into(),
            "-Dwarnings".into(),
            "-o".into(),
            "libnativeboat_start.rlib".into(),
        ],
        &scratch.0,
    )
    .unwrap();
    std::fs::write(
        scratch.0.join("app.rs"),
        r#"
#![no_std]
#![no_main]
extern crate alloc;
extern crate nativeboat_start;
use url::Url;
#[unsafe(no_mangle)]
pub fn nativeboat_main() -> i32 {
    let unicode = Url::parse("https://user:pass@Exämple.com:443/a/../b?x=1#f").unwrap();
    assert_eq!(unicode.as_str(), "https://user:pass@xn--exmple-cua.com/b?x=1#f");
    assert_eq!(unicode.join("../c").unwrap().as_str(), "https://user:pass@xn--exmple-cua.com/c");
    assert_eq!(Url::parse("http://0x7f000001").unwrap().as_str(), "http://127.0.0.1/");
    assert_eq!(Url::parse("http://[2001:0db8:0:0:0:0:0:1]:80/a").unwrap().as_str(), "http://[2001:db8::1]/a");
    assert!(Url::parse("http://[bad]").is_err());
    assert_eq!(idna::domain_to_ascii("faß.de").unwrap(), "xn--fa-hia.de");
    assert_eq!(idna::domain_to_unicode("xn--fa-hia.de").0, "faß.de");
    0
}
"#,
    )
    .unwrap();
    rustc(
        &[
            "app.rs".into(),
            "--edition=2024".into(),
            "--extern=url=liburl.rlib".into(),
            "--extern=idna=libidna.rlib".into(),
            "--extern=nativeboat_start=libnativeboat_start.rlib".into(),
            "-Ldependency=.".into(),
            "-Cpanic=abort".into(),
            "-Copt-level=2".into(),
            "-Dwarnings".into(),
            "-Clinker=rust-lld".into(),
            "-Clinker-flavor=lld-link".into(),
            "-Clink-arg=/NODEFAULTLIB".into(),
            "-Clink-arg=/ENTRY:nativeboat_start".into(),
            "-Clink-arg=/SUBSYSTEM:CONSOLE".into(),
            "-o".into(),
            "app.exe".into(),
        ],
        &scratch.0,
    )
    .unwrap();
    let output = Command::new(scratch.0.join("app.exe")).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = std::fs::read(scratch.0.join("app.exe")).unwrap();
    assert_eq!(imports(&bytes), ["kernel32.dll"]);
    // Notices survive optimization and linking, including the matcher authors.
    assert!(
        bytes
            .windows(NOTICES.len())
            .any(|part| part == NOTICES.as_bytes())
    );
}

#[cfg(windows)]
fn imports(bytes: &[u8]) -> Vec<String> {
    let u16_at =
        |offset| u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap()) as usize;
    let u32_at =
        |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
    let pe = u32_at(0x3C);
    assert_eq!(&bytes[pe..pe + 4], b"PE\0\0");
    let sections = pe + 24 + u16_at(pe + 20);
    let map = |rva| {
        for section in (0..u16_at(pe + 6)).map(|index| sections + index * 40) {
            let base = u32_at(section + 12);
            if rva >= base && rva - base < u32_at(section + 16) {
                return rva - base + u32_at(section + 20);
            }
        }
        panic!("unmapped PE RVA {rva}")
    };
    assert_eq!(u16_at(pe + 24), 0x20B);
    let mut descriptor = map(u32_at(pe + 24 + 120));
    let mut names = Vec::new();
    while u32_at(descriptor + 12) != 0 {
        let start = map(u32_at(descriptor + 12));
        let length = bytes[start..].iter().position(|byte| *byte == 0).unwrap();
        names.push(
            std::str::from_utf8(&bytes[start..start + length])
                .unwrap()
                .to_ascii_lowercase(),
        );
        descriptor += 20;
    }
    names
}
