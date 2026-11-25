//! Compile the packaged no_std encoding detector with a caller-selected toolchain.
//! This build-time library uses std; none of it is linked into applications.
use std::ffi::OsString;
use std::path::{Path, PathBuf};

mod sources;
pub use sources::NOTICES;

/// Invocation of the selected Rust compiler and sysroot.
pub type Compiler<'a> = dyn FnMut(&[OsString], &Path) -> Result<(), String> + 'a;

#[derive(Debug)]
struct Library {
    name: &'static str,
    source: &'static str,
    edition: &'static str,
    crate_type: &'static str,
    artifact: &'static str,
    features: &'static [&'static str],
    dependencies: &'static [&'static str],
}

const LIBRARIES: &[Library] = &[
    Library {
        name: "cfg_if",
        source: "cfg-if-1.0.5",
        edition: "2018",
        crate_type: "rlib",
        artifact: "libcfg_if.rlib",
        features: &[],
        dependencies: &[],
    },
    Library {
        name: "encoding_rs",
        source: "encoding_rs-0.8.35",
        edition: "2018",
        crate_type: "rlib",
        artifact: "libencoding_rs.rlib",
        features: &[],
        dependencies: &["cfg_if"],
    },
    Library {
        name: "memchr",
        source: "memchr-2.8.3",
        edition: "2021",
        crate_type: "rlib",
        artifact: "libmemchr.rlib",
        features: &["alloc"],
        dependencies: &[],
    },
    Library {
        name: "chardetng",
        source: "chardetng-1.0.0",
        edition: "2018",
        crate_type: "rlib",
        artifact: "libchardetng.rlib",
        features: &[],
        dependencies: &["cfg_if", "encoding_rs", "memchr"],
    },
];

/// Fingerprint the source and compiler flags used for the cached rlibs.
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

/// Build the detector and its no_std dependencies in dependency order.
/// The returned rlib requires `-L dependency=directory` when linked.
pub fn build(directory: &Path, rustc: &mut Compiler<'_>) -> Result<PathBuf, String> {
    build_with_shared_memchr(directory, None, rustc)
}

/// Build the detector using a previously compiled `memchr` artifact. The
/// native compiler shares this dependency with the regexp matcher so rustc
/// sees one crate hash for both consumers.
pub fn build_with_shared_memchr(
    directory: &Path,
    shared_memchr: Option<&Path>,
    rustc: &mut Compiler<'_>,
) -> Result<PathBuf, String> {
    let directory = std::path::absolute(directory).map_err(|error| error.to_string())?;
    for (name, source) in sources::SOURCES {
        let path = directory.join(name);
        std::fs::create_dir_all(path.parent().expect("source path has a parent")).map_err(
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
        if library.name == "memchr" && shared_memchr.is_some() {
            continue;
        }
        let mut args: Vec<OsString> = vec![
            format!("{}/src/lib.rs", library.source).into(),
            format!("--edition={}", library.edition).into(),
            format!("--crate-type={}", library.crate_type).into(),
            format!("--crate-name={}", library.name).into(),
            "-Cpanic=abort".into(),
            "-Copt-level=2".into(),
            "--cap-lints=allow".into(),
            "-Ldependency=.".into(),
            "-o".into(),
            library.artifact.into(),
        ];
        for dependency in library.dependencies {
            let shared = (*dependency == "memchr").then_some(shared_memchr).flatten();
            let artifact = shared.map_or_else(
                || {
                    let artifact = LIBRARIES
                        .iter()
                        .find(|candidate| candidate.name == *dependency)
                        .expect("all packaged library dependencies are listed")
                        .artifact;
                    PathBuf::from(artifact)
                },
                Path::to_path_buf,
            );
            let artifact = artifact.as_os_str();
            let mut argument = OsString::from(format!("--extern={dependency}="));
            argument.push(artifact);
            args.push(argument);
        }
        for feature in library.features {
            args.extend(["--cfg".into(), format!("feature=\"{feature}\"").into()]);
        }
        rustc(&args, &directory)?;
    }
    Ok(directory.join("libchardetng.rlib"))
}

#[cfg(test)]
mod tests;
