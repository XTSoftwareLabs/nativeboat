//! Compile the packaged matcher sources with a caller-selected Rust toolchain.
//! This build-time library uses std; none of it is linked into applications.
use std::ffi::OsString;
use std::path::{Path, PathBuf};

mod sources;
pub use sources::NOTICES;

/// Invocation of a caller-selected Rust compiler and sysroot.
pub type Compiler<'a> = dyn FnMut(&[OsString], &Path) -> Result<(), String> + 'a;

#[derive(Debug)]
struct Library {
    name: &'static str,
    source: &'static str,
    edition: &'static str,
    features: &'static [&'static str],
    dependencies: &'static [&'static str],
    warnings: &'static str,
}

const LIBRARIES: &[Library] = &[
    Library {
        name: "equivalent",
        source: "equivalent-1.0.2",
        edition: "2015",
        features: &[],
        dependencies: &[],
        warnings: "--cap-lints=allow",
    },
    Library {
        name: "allocator_api2",
        source: "allocator-api2-0.2.21",
        edition: "2018",
        features: &["alloc"],
        dependencies: &[],
        warnings: "--cap-lints=allow",
    },
    Library {
        name: "foldhash",
        source: "foldhash-0.2.0",
        edition: "2021",
        features: &[],
        dependencies: &[],
        warnings: "--cap-lints=allow",
    },
    Library {
        name: "hashbrown",
        source: "hashbrown-0.17.1",
        edition: "2024",
        features: &[
            "default-hasher",
            "inline-more",
            "allocator-api2",
            "equivalent",
            "raw-entry",
        ],
        dependencies: &["equivalent", "allocator_api2", "foldhash"],
        warnings: "--cap-lints=allow",
    },
    Library {
        name: "memchr",
        source: "memchr-2.8.3",
        edition: "2021",
        features: &["alloc"],
        dependencies: &[],
        warnings: "--cap-lints=allow",
    },
    Library {
        name: "regress",
        source: "regress-0.12.0",
        edition: "2024",
        features: &["alloc", "utf16"],
        dependencies: &["hashbrown", "memchr"],
        warnings: "--cap-lints=allow",
    },
    Library {
        name: "nativeboat_regexp",
        source: "nativeboat-regexp",
        edition: "2024",
        features: &[],
        dependencies: &["regress"],
        warnings: "-Dwarnings",
    },
];

/// A digest of the packaged sources and their build arguments. Callers that
/// cache built libraries combine it with the toolchain's identity.
pub fn fingerprint() -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for (name, source) in sources::SOURCES {
        feed(name.as_bytes());
        feed(&[0]);
        feed(source.as_bytes());
        feed(&[0]);
    }
    feed(format!("{LIBRARIES:?}").as_bytes());
    hash
}

/// Writes pinned sources into `directory` and builds seven no_std libraries in
/// dependency order. The callback invokes the selected rustc/sysroot with the
/// supplied arguments and working directory. It must return compiler errors.
/// The returned rlib requires `-L dependency=directory` when linking callers.
/// This intentionally has no Cargo, registry, build script or SDK dependency.
pub fn build(directory: &Path, rustc: &mut Compiler<'_>) -> Result<PathBuf, String> {
    let directory = std::path::absolute(directory).map_err(|error| error.to_string())?;
    for (name, source) in sources::SOURCES {
        let path = directory.join(name);
        std::fs::create_dir_all(path.parent().expect("packaged source has a parent")).map_err(
            |error| {
                format!(
                    "cannot create source directory for {}: {error}",
                    path.display()
                )
            },
        )?;
        std::fs::write(&path, source)
            .map_err(|error| format!("cannot write packaged source {}: {error}", path.display()))?;
    }
    for library in LIBRARIES {
        let mut args: Vec<OsString> = vec![
            format!("{}/src/lib.rs", library.source).into(),
            format!("--edition={}", library.edition).into(),
            "--crate-type=rlib".into(),
            format!("--crate-name={}", library.name).into(),
            "-Cpanic=abort".into(),
            "-Copt-level=2".into(),
            library.warnings.into(),
            "-Ldependency=.".into(),
            "-o".into(),
            format!("lib{}.rlib", library.name).into(),
        ];
        for feature in library.features {
            args.extend(["--cfg".into(), format!("feature=\"{feature}\"").into()]);
        }
        for dependency in library.dependencies {
            args.push(format!("--extern={dependency}=lib{dependency}.rlib").into());
        }
        rustc(&args, &directory)?;
    }
    Ok(directory.join("libnativeboat_regexp.rlib"))
}

#[cfg(test)]
mod tests;
