use super::*;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "nativeboat-encoding-{}-{}",
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
                .starts_with("nativeboat-encoding-")
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
fn detector_build_packages_sources_and_compiles_all_libraries_without_std() {
    let scratch = Scratch::new();
    let library = build(&scratch.0, &mut rustc).unwrap();
    assert_eq!(library, scratch.0.join("libchardetng.rlib"));
    assert!(library.is_file());
    for name in ["cfg_if", "encoding_rs", "memchr"] {
        assert!(scratch.0.join(format!("lib{name}.rlib")).is_file());
    }
    assert!(NOTICES.contains("Mozilla Foundation"));
    assert!(NOTICES.contains("Copyright © WHATWG"));

    let consumer = scratch.0.join("consumer.rs");
    std::fs::write(
        &consumer,
        r#"#![no_std]
pub fn detect(input: &[u8]) -> &'static str {
    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Allow);
    detector.feed(input, true);
    detector.guess(None, chardetng::Utf8Detection::Allow).name()
}
"#,
    )
    .unwrap();
    let output = Command::new("rustc")
        .arg(&consumer)
        .args([
            "--crate-name=consumer",
            "--crate-type=rlib",
            "--edition=2024",
            "-Cpanic=abort",
            "-Ldependency=.",
        ])
        .arg("--extern")
        .arg(format!("chardetng={}", library.display()))
        .arg("-o")
        .arg(scratch.0.join("libconsumer.rlib"))
        .current_dir(&scratch.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn build_propagates_the_selected_compiler_error() {
    let scratch = Scratch::new();
    let error = build(&scratch.0, &mut |args, _| {
        assert_eq!(args[0], "cfg-if-1.0.5/src/lib.rs");
        Err("selected Rust compiler failed".into())
    })
    .unwrap_err();
    assert_eq!(error, "selected Rust compiler failed");
}

#[test]
fn shared_memchr_is_reused_without_building_a_second_copy() {
    let scratch = Scratch::new();
    let shared = scratch.0.join("matcher/libmemchr.rlib");
    let mut calls = Vec::new();
    build_with_shared_memchr(
        &scratch.0.join("encoding"),
        Some(&shared),
        &mut |args, _| {
            calls.push(args.to_vec());
            Ok(())
        },
    )
    .unwrap();

    assert_eq!(calls.len(), 3, "memchr is supplied by the matcher");
    let chardetng = calls.last().unwrap();
    assert!(chardetng.contains(&format!("--extern=memchr={}", shared.display()).into()));
    assert!(!scratch.0.join("encoding/libmemchr.rlib").exists());
}

#[test]
fn fingerprint_is_repeatable_for_the_packaged_build_inputs() {
    assert_ne!(fingerprint(), 0);
    assert_eq!(fingerprint(), fingerprint());
}

#[test]
fn source_directory_failures_name_the_packaged_path() {
    let scratch = Scratch::new();
    let blocked = scratch.0.join("blocked");
    std::fs::write(&blocked, "not a directory").unwrap();
    let error = build(&blocked, &mut |_, _| panic!("compiler must not run")).unwrap_err();
    assert!(error.contains("cannot create source directory"), "{error}");
    assert!(error.contains("cfg-if-1.0.5"), "{error}");
}
