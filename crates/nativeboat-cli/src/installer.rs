//! The Windows installer of a native build and its update metadata, in
//! place of electron-builder's NSIS installer and `latest.yml`: the setup
//! program the runtime libraries include (`setup/setup.rs`), linked with the
//! application's resources, with the output folder's files compressed after
//! it. electron-updater downloads the installer the metadata names and runs
//! it with NSIS's options.

use nativeboat_project::Project;
use std::ffi::OsString;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"NBSETUP\0";
const END_MAGIC: &[u8; 8] = b"NBSETUP\x01";

/// What the installer step needs from the build.
pub(crate) struct Inputs<'a> {
    pub rustc: &'a crate::native::Toolchain,
    pub runtime: &'a Path,
    pub work: &'a Path,
    pub resources: &'a Path,
    pub project: &'a Project,
    /// The built application folder.
    pub application: &'a Path,
    /// The executable's file name.
    pub executable: &'a str,
    pub destination: &'a Path,
    pub sign: Option<&'a str>,
}

/// Where a build writes its installer: the folder `--installer` names, else
/// `<output>-setup` when the project has an update feed, which updates
/// install from.
pub(crate) fn directory(
    project: &Project,
    output: &Path,
    option: Option<&Path>,
) -> Option<PathBuf> {
    match option {
        Some(directory) => Some(project.root.join(directory)),
        None => project.updater_config.as_ref().map(|_| {
            let mut name = output.file_name().unwrap_or_default().to_os_string();
            name.push("-setup");
            output.with_file_name(name)
        }),
    }
}

/// The files of a folder, relative to it with `\` separators, sorted.
fn files(root: &Path, directory: &Path, found: &mut Vec<String>) -> Result<(), String> {
    let entries = std::fs::read_dir(directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(root, &path, found)?;
        } else if let Ok(relative) = path.strip_prefix(root) {
            found.push(relative.to_string_lossy().replace('/', "\\"));
        }
    }
    Ok(())
}

/// The settings the setup program reads, as `key=value` lines.
fn settings(project: &Project, executable: &str) -> String {
    let installer = &project.installer;
    let metadata = &project.metadata;
    let folder: String = metadata
        .name
        .chars()
        .map(|ch| {
            if "<>:\"/\\|?*".contains(ch) || ch.is_control() {
                '-'
            } else {
                ch
            }
        })
        .collect();
    let flag = |value: bool| if value { "1" } else { "0" };
    [
        ("mode", "install"),
        ("product", &metadata.product_name),
        ("shortcut", &installer.shortcut_name),
        ("exe", executable),
        ("version", &metadata.version),
        ("appid", &installer.app_id),
        ("publisher", &installer.publisher),
        ("folder", &folder),
        ("data", &metadata.app_name),
        ("desktop", flag(installer.desktop_shortcut)),
        ("startmenu", flag(installer.start_menu_shortcut)),
        ("run", flag(installer.run_after_finish)),
        ("appdata", flag(installer.delete_app_data)),
    ]
    .iter()
    .map(|(key, value)| format!("{key}={}\n", value.replace(['\r', '\n'], " ")))
    .collect()
}

/// Appends the payload to the setup program at `path`: the settings, then
/// each file raw-deflated, then where the payload starts.
fn append_payload(
    path: &Path,
    settings: &str,
    root: &Path,
    names: &[String],
) -> Result<(), String> {
    let fail = |error: std::io::Error| format!("cannot write {}: {error}", path.display());
    let mut output = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(fail)?;
    let start = output.seek(SeekFrom::End(0)).map_err(fail)?;
    output.write_all(MAGIC).map_err(fail)?;
    output.write_all(&1u32.to_le_bytes()).map_err(fail)?;
    output
        .write_all(&(settings.len() as u32).to_le_bytes())
        .map_err(fail)?;
    output.write_all(settings.as_bytes()).map_err(fail)?;
    output
        .write_all(&(names.len() as u32).to_le_bytes())
        .map_err(fail)?;
    for name in names {
        let source = root.join(name);
        let bytes = std::fs::read(&source)
            .map_err(|error| format!("cannot read {}: {error}", source.display()))?;
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&bytes).map_err(fail)?;
        let compressed = encoder.finish().map_err(fail)?;
        output
            .write_all(&(name.len() as u32).to_le_bytes())
            .map_err(fail)?;
        output.write_all(name.as_bytes()).map_err(fail)?;
        output
            .write_all(&(bytes.len() as u64).to_le_bytes())
            .map_err(fail)?;
        output
            .write_all(&(compressed.len() as u64).to_le_bytes())
            .map_err(fail)?;
        output.write_all(&compressed).map_err(fail)?;
    }
    output.write_all(&start.to_le_bytes()).map_err(fail)?;
    output.write_all(END_MAGIC).map_err(fail)
}

/// The SHA-512 digest of `data`, from Windows' CNG.
fn sha512(data: &[u8]) -> Result<[u8; 64], String> {
    use windows_sys::Win32::Security::Cryptography::{BCRYPT_SHA512_ALG_HANDLE, BCryptHash};
    let mut digest = [0u8; 64];
    // SAFETY: the pseudo-handle needs no opening; both buffers are live for
    // the lengths passed.
    let status = unsafe {
        BCryptHash(
            BCRYPT_SHA512_ALG_HANDLE,
            core::ptr::null(),
            0,
            data.as_ptr(),
            data.len() as u32,
            digest.as_mut_ptr(),
            digest.len() as u32,
        )
    };
    if status == 0 {
        Ok(digest)
    } else {
        Err(format!("cannot hash the installer (status {status:#x})"))
    }
}

/// An ISO 8601 UTC time with milliseconds, as electron-builder dates releases.
fn release_date(time: std::time::SystemTime) -> String {
    let since = time
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = since.as_secs();
    let days = (seconds / 86_400) as i64;
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    let rest = seconds % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60,
        since.subsec_millis()
    )
}

/// The update metadata electron-updater reads: `<channel>.yml`.
fn update_info(version: &str, name: &str, digest: &[u8], size: u64, date: &str) -> String {
    use base64::Engine as _;
    let sha512 = base64::engine::general_purpose::STANDARD.encode(digest);
    let info = serde_json::json!({
        "version": version,
        "files": [{"url": name, "sha512": sha512, "size": size}],
        "path": name,
        "sha512": sha512,
        "releaseDate": date,
    });
    nativeboat_project::builder::app_update_yml(&info)
}

/// Writes the installer and its update metadata; returns the installer.
pub(crate) fn write(inputs: &Inputs) -> Result<PathBuf, String> {
    let installer = &inputs.project.installer;
    let mut names = Vec::new();
    files(inputs.application, inputs.application, &mut names)?;
    names.retain(|name| !name.starts_with(".nativeboat-build-"));
    names.sort();
    std::fs::write(
        inputs.work.join("setup.rs"),
        "#![no_std]\n#![no_main]\nextern crate nativeboat_setup;\n",
    )
    .map_err(|error| format!("cannot write the setup program: {error}"))?;
    let mut setup = OsString::from("--extern=nativeboat_setup=");
    setup.push(inputs.runtime.join("libnativeboat_setup.rlib"));
    let mut dependencies = OsString::from("-Ldependency=");
    dependencies.push(inputs.runtime);
    let mut resources = OsString::from("-Clink-arg=");
    resources.push(inputs.resources);
    crate::native::toolchain(
        inputs.rustc,
        &[
            "setup.rs".into(),
            "--edition=2024".into(),
            "--crate-name=nativeboat_installer".into(),
            setup,
            dependencies,
            "-Cpanic=abort".into(),
            "-Copt-level=s".into(),
            "-Dwarnings".into(),
            "-Clinker=rust-lld".into(),
            "-Clinker-flavor=lld-link".into(),
            "-Clink-arg=/NODEFAULTLIB".into(),
            "-Clink-arg=/ENTRY:nativeboat_start".into(),
            "-Clink-arg=/SUBSYSTEM:WINDOWS".into(),
            resources,
            "-o".into(),
            "setup.exe".into(),
        ],
        inputs.work,
    )?;
    let staged = inputs.work.join("setup.exe");
    append_payload(
        &staged,
        &settings(inputs.project, inputs.executable),
        inputs.application,
        &names,
    )?;
    if let Some(command) = inputs.sign {
        crate::native::sign(command, &staged, &inputs.project.root)?;
    }
    std::fs::create_dir_all(inputs.destination)
        .map_err(|error| format!("cannot create {}: {error}", inputs.destination.display()))?;
    let path = inputs.destination.join(&installer.artifact_name);
    std::fs::copy(&staged, &path)
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    let bytes =
        std::fs::read(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let info = update_info(
        &inputs.project.metadata.version,
        &installer.artifact_name,
        &sha512(&bytes)?,
        bytes.len() as u64,
        &release_date(std::time::SystemTime::now()),
    );
    let metadata = inputs
        .destination
        .join(format!("{}.yml", installer.channel));
    std::fs::write(&metadata, info)
        .map_err(|error| format!("cannot write {}: {error}", metadata.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests;
