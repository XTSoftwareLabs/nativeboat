use super::*;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug)]
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "nativeboat-regexp-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
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
        assert_eq!(args[0], "equivalent-1.0.2/src/lib.rs");
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
    assert_eq!(calls, 7);
    assert_eq!(library, scratch.0.join("libnativeboat_regexp.rlib"));
    for (name, source) in sources::SOURCES {
        assert_eq!(
            std::fs::read(scratch.0.join(name)).unwrap(),
            source.as_bytes()
        );
    }
    for name in [
        "regress-0.12.0",
        "memchr-2.8.3",
        "foldhash-0.2.0",
        "hashbrown-0.17.1",
        "allocator-api2-0.2.21",
        "equivalent-1.0.2",
    ] {
        assert!(NOTICES.contains(name));
    }
    assert!(NOTICES.contains("Orson Peters"));
    assert!(NOTICES.contains("Unicode Character Database 16.0.0"));
    // The fingerprint is stable and covers sources and build arguments.
    assert_eq!(fingerprint(), fingerprint());
    assert_ne!(fingerprint(), 0xcbf2_9ce4_8422_2325);
}

#[cfg(windows)]
#[test]
fn packaged_matcher_compiles_and_runs_without_cargo_std_or_crt() {
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
use alloc::vec::Vec;
use nativeboat_regexp::{Flags, Pattern};
#[unsafe(no_mangle)]
pub fn nativeboat_main() -> i32 {
    let text = "word word".encode_utf16().collect::<Vec<_>>();
    let source = "(?<word>[a-z]+)\\s+\\k<word>".encode_utf16().collect::<Vec<_>>();
    let pattern = Pattern::compile(&source, Flags::default()).unwrap();
    let found = pattern.search(&text, 0).unwrap();
    assert_eq!(found.range, 0..9);
    assert_eq!(found.captures[0], Some(0..4));
    assert_eq!(found.groups[0].0, "word");
    let unicode = Pattern::compile(&[0x2E], Flags { unicode: true, ..Flags::default() }).unwrap();
    assert_eq!(unicode.search(&[0xD83D, 0xDE00, 0xD800], 1).unwrap().range, 0..2);
    assert_eq!(unicode.search(&[0xD83D, 0xDE00, 0xD800], 2).unwrap().range, 2..3);
    let units = Pattern::compile(&[0x2E], Flags::default()).unwrap();
    assert_eq!(units.search(&[0xD83D, 0xDE00], 1).unwrap().range, 1..2);
    0
}
"#,
    )
    .unwrap();
    rustc(
        &[
            "app.rs".into(),
            "--edition=2024".into(),
            "--extern=nativeboat_regexp=libnativeboat_regexp.rlib".into(),
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
