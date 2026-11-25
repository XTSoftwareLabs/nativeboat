//! Release builds of nativeboat start on its own process startup code,
//! `nativeboat-std-start`, instead of the Visual C++ runtime, so the
//! executable imports operating-system DLLs only.
//!
//! They also link against import libraries made here rather than the
//! Windows SDK's: the export names of the system DLLs that Rust `std`
//! links are read from the DLLs themselves, and the toolchain's `rust-lld`
//! writes the import libraries. Building nativeboat needs neither the
//! Windows SDK nor the Visual C++ build tools.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The DLLs whose import libraries Rust `std` names.
const SYSTEM_LIBRARIES: [&str; 5] = ["kernel32", "ntdll", "userenv", "ws2_32", "dbghelp"];

fn main() {
    println!("cargo::rustc-check-cfg=cfg(nativeboat_own_runtime)");
    println!("cargo::rerun-if-changed=build.rs");
    let release = std::env::var("PROFILE").is_ok_and(|profile| profile == "release");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc");
    if !(release && msvc) {
        return;
    }
    println!("cargo::rustc-cfg=nativeboat_own_runtime");
    println!("cargo::rustc-link-arg-bins=/NODEFAULTLIB");
    // The Visual C++ linker warns that the initializer sections `std`
    // registers in have no C runtime to run them; the startup code does.
    println!("cargo::rustc-link-arg-bins=/IGNORE:4210");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let system =
        PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
            .join("System32");
    let linker = rust_lld();
    for library in SYSTEM_LIBRARIES {
        import_library(
            &linker,
            &system.join(format!("{library}.dll")),
            library,
            &out,
        );
    }
    // Searched before the libraries any installed SDK provides.
    println!("cargo::rustc-link-search=native={}", out.display());
}

/// The toolchain's own linker.
fn rust_lld() -> PathBuf {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let output = Command::new(rustc)
        .args(["--print", "sysroot"])
        .output()
        .expect("rustc prints its sysroot");
    let sysroot = String::from_utf8(output.stdout).expect("the sysroot is UTF-8");
    let host = std::env::var("HOST").expect("cargo sets HOST");
    Path::new(sysroot.trim())
        .join("lib/rustlib")
        .join(host)
        .join("bin/rust-lld.exe")
}

/// Writes `<library>.lib`, importing every function the DLL exports.
fn import_library(linker: &Path, dll: &Path, library: &str, out: &Path) {
    let image =
        std::fs::read(dll).unwrap_or_else(|error| panic!("cannot read {}: {error}", dll.display()));
    let mut definition = format!("LIBRARY {library}.dll\nEXPORTS\n");
    for name in export_names(&image) {
        definition.push_str("  ");
        definition.push_str(&name);
        definition.push('\n');
    }
    let def = out.join(format!("{library}.def"));
    std::fs::write(&def, definition).expect("the module definition is written");
    let status = Command::new(linker)
        .args(["-flavor", "link", "-lib", "-machine:x64"])
        .arg(format!("-def:{}", def.display()))
        .arg(format!(
            "-out:{}",
            out.join(format!("{library}.lib")).display()
        ))
        .status()
        .expect("rust-lld runs");
    assert!(status.success(), "rust-lld could not write {library}.lib");
}

/// The names in a PE image's export directory.
fn export_names(image: &[u8]) -> Vec<String> {
    let u16_at = |offset: usize| u16::from_le_bytes([image[offset], image[offset + 1]]) as usize;
    let u32_at = |offset: usize| {
        u32::from_le_bytes(image[offset..offset + 4].try_into().expect("four bytes")) as usize
    };
    let header = u32_at(0x3C);
    let sections = u16_at(header + 6);
    let optional = header + 24;
    // The export directory is the first data directory of a PE32+ image.
    let directories = optional + 112;
    let section_table = optional + u16_at(header + 20);
    let offset = |rva: usize| {
        (0..sections)
            .map(|index| section_table + 40 * index)
            .find_map(|section| {
                let size = u32_at(section + 8).max(u32_at(section + 16));
                let start = u32_at(section + 12);
                (start..start + size)
                    .contains(&rva)
                    .then(|| rva - start + u32_at(section + 20))
            })
            .expect("the address lies in a section")
    };
    let exports = offset(u32_at(directories));
    let count = u32_at(exports + 24);
    let names = offset(u32_at(exports + 32));
    (0..count)
        .map(|index| {
            let start = offset(u32_at(names + 4 * index));
            let end = start
                + image[start..]
                    .iter()
                    .position(|byte| *byte == 0)
                    .expect("names end");
            String::from_utf8_lossy(&image[start..end]).into_owned()
        })
        .collect()
}
