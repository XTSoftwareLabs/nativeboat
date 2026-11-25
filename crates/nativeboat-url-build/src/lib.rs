//! Compile the packaged URL parser sources with a caller-selected Rust toolchain.
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
        name: "unicode_joining_type",
        source: "unicode-joining-type-1.0.0",
        edition: "2018",
        features: &[],
        dependencies: &[],
        warnings: "--cap-lints=allow",
    },
    Library {
        name: "idna_mapping",
        source: "idna_mapping-1.1.0",
        edition: "2021",
        features: &[],
        dependencies: &["unicode_joining_type"],
        warnings: "--cap-lints=allow",
    },
    Library {
        name: "unicode_bidi",
        source: "unicode-bidi-0.3.18",
        edition: "2018",
        features: &["hardcoded-data"],
        dependencies: &[],
        warnings: "--cap-lints=allow",
    },
    Library {
        name: "tinyvec",
        source: "tinyvec-1.13.3",
        edition: "2018",
        features: &["alloc", "default"],
        dependencies: &[],
        warnings: "--cap-lints=allow",
    },
    Library {
        name: "unicode_normalization",
        source: "unicode-normalization-0.1.24",
        edition: "2018",
        features: &[],
        dependencies: &["tinyvec"],
        warnings: "--cap-lints=allow",
    },
    Library {
        name: "idna_adapter",
        source: "idna_adapter-1.1.0",
        edition: "2021",
        features: &["compiled_data"],
        dependencies: &["idna_mapping", "unicode_bidi", "unicode_normalization"],
        warnings: "--cap-lints=allow",
    },
    Library {
        name: "percent_encoding",
        source: "percent-encoding-2.3.2",
        edition: "2018",
        features: &["alloc"],
        dependencies: &[],
        warnings: "--cap-lints=allow",
    },
    Library {
        name: "form_urlencoded",
        source: "form_urlencoded-1.2.2",
        edition: "2018",
        features: &["alloc"],
        dependencies: &["percent_encoding"],
        warnings: "--cap-lints=allow",
    },
    Library {
        name: "smallvec",
        source: "smallvec-1.16.3",
        edition: "2018",
        features: &["const_generics"],
        dependencies: &[],
        warnings: "--cap-lints=allow",
    },
    Library {
        name: "utf8_iter",
        source: "utf8_iter-1.0.4",
        edition: "2021",
        features: &[],
        dependencies: &[],
        warnings: "--cap-lints=allow",
    },
    Library {
        name: "idna",
        source: "idna-1.1.0",
        edition: "2018",
        features: &["alloc", "compiled_data"],
        dependencies: &["idna_adapter", "smallvec", "utf8_iter"],
        warnings: "--cap-lints=allow",
    },
    Library {
        name: "url",
        source: "url-2.5.8",
        edition: "2018",
        features: &[],
        dependencies: &["form_urlencoded", "idna", "percent_encoding"],
        warnings: "--cap-lints=allow",
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

/// Writes pinned sources into `directory` and builds twelve no_std libraries in
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
    Ok(directory.join("liburl.rlib"))
}

#[cfg(test)]
mod tests;
