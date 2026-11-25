//! Runs a project's build steps the way npm runs scripts: in the Windows
//! shell, from the project directory, with the `node_modules/.bin`
//! directories of the project and its parents on the search path.

use nativeboat_project::scripts::Step;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Sender, channel};
use windows_sys::Win32::Globalization::{CP_OEMCP, MultiByteToWideChar};
use windows_sys::Win32::System::Console::GetConsoleOutputCP;

/// The shell npm runs scripts in.
pub fn shell() -> PathBuf {
    PathBuf::from(std::env::var_os("ComSpec").unwrap_or(OsString::from("cmd.exe")))
}

/// The search path of scripts: the `node_modules/.bin` directories of the
/// project and its parents, then the inherited path.
pub(crate) fn search_path(root: &Path) -> OsString {
    let mut directories: Vec<PathBuf> = root
        .ancestors()
        .map(|directory| directory.join("node_modules").join(".bin"))
        .collect();
    if let Some(path) = std::env::var_os("PATH") {
        directories.extend(std::env::split_paths(&path));
    }
    std::env::join_paths(directories).unwrap_or_default()
}

/// Converts a line a child process wrote to UTF-8. Node tools write UTF-8,
/// but the Windows shell and other console programs write in the console's
/// code page.
fn decode(line: &[u8]) -> Vec<u8> {
    if std::str::from_utf8(line).is_ok() {
        return line.to_vec();
    }
    // SAFETY: takes no arguments.
    let page = match unsafe { GetConsoleOutputCP() } {
        0 => CP_OEMCP,
        page => page,
    };
    let length = i32::try_from(line.len()).unwrap_or(i32::MAX);
    // SAFETY: `line` holds `length` readable bytes; a null buffer of size 0
    // asks for the length of the conversion.
    let size =
        unsafe { MultiByteToWideChar(page, 0, line.as_ptr(), length, std::ptr::null_mut(), 0) };
    if size <= 0 {
        return String::from_utf8_lossy(line).into_owned().into_bytes();
    }
    let mut wide = vec![0u16; size.unsigned_abs() as usize];
    // SAFETY: `wide` holds `size` writable characters.
    unsafe { MultiByteToWideChar(page, 0, line.as_ptr(), length, wide.as_mut_ptr(), size) };
    String::from_utf16_lossy(&wide).into_bytes()
}

/// Passes what a child process writes on line by line, converted to UTF-8
/// and tagged with whether it is error output.
fn forward(mut reader: Box<dyn Read + Send>, error: bool, sender: Sender<(bool, Vec<u8>)>) {
    std::thread::spawn(move || {
        let mut buffer = [0; 4096];
        let mut line = Vec::new();
        while let Ok(count @ 1..) = reader.read(&mut buffer) {
            for &byte in &buffer[..count] {
                line.push(byte);
                if byte == b'\n' || byte == b'\r' {
                    let _ = sender.send((error, decode(&std::mem::take(&mut line))));
                }
            }
        }
        if !line.is_empty() {
            let _ = sender.send((error, decode(&line)));
        }
    });
}

/// Runs the steps in order, passing their output on, and stops at the
/// first that fails.
pub fn run(
    root: &Path,
    steps: &[Step],
    shell: &Path,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), String> {
    run_with(
        root,
        steps,
        shell,
        &[],
        &mut |_, _, _| false,
        stdout,
        stderr,
    )
}

/// What a failed step wrote to its error output, given a chance to fix
/// what it lacked: it returns whether the step should run again.
pub(crate) type Recover<'a> = dyn FnMut(&str, &mut dyn Write, &mut dyn Write) -> bool + 'a;

/// How much of a step's error output is kept for [`Recover`].
const KEPT_ERRORS: usize = 64 * 1024;

/// Runs the steps like [`run`] with additional environment variables. A
/// failed step runs again when `recover` fixed what it lacked; a failed
/// optional step is reported and the others still run.
pub(crate) fn run_with(
    root: &Path,
    steps: &[Step],
    shell: &Path,
    environment: &[(&str, &str)],
    recover: &mut Recover<'_>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), String> {
    for step in steps {
        loop {
            let (code, errors) = run_one(root, step, shell, environment, stdout, stderr)?;
            if code == Some(0) {
                break;
            }
            if recover(&errors, stdout, stderr) {
                continue;
            }
            let code = code.map_or(String::from("none"), |code| code.to_string());
            let message = format!(
                "the {} script failed with exit code {code}: {}",
                step.script, step.command
            );
            if !step.optional {
                return Err(message);
            }
            let _ = writeln!(
                stderr,
                "warning: {message}; the build goes on without what it makes"
            );
            break;
        }
    }
    Ok(())
}

/// Runs one step, passing its output on: its exit code and the end of its
/// error output.
fn run_one(
    root: &Path,
    step: &Step,
    shell: &Path,
    environment: &[(&str, &str)],
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(Option<i32>, String), String> {
    let path = search_path(root);
    {
        let _ = writeln!(stdout, "> {}: {}", step.script, step.command);
        let mut child = Command::new(shell)
            .raw_arg(format!("/d /s /c \"{}\"", step.command))
            .current_dir(root)
            .env("PATH", &path)
            .env("npm_lifecycle_event", &step.script)
            .env("INIT_CWD", root)
            .envs(environment.iter().copied())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("cannot run {}: {error}", shell.display()))?;
        let (sender, receiver) = channel();
        if let Some(output) = child.stdout.take() {
            forward(Box::new(output), false, sender.clone());
        }
        if let Some(output) = child.stderr.take() {
            forward(Box::new(output), true, sender);
        }
        let mut errors = Vec::new();
        for (error, chunk) in receiver {
            let sink: &mut dyn Write = if error { &mut *stderr } else { &mut *stdout };
            let _ = sink.write_all(&chunk);
            if error {
                errors.extend_from_slice(&chunk);
                if errors.len() > 2 * KEPT_ERRORS {
                    errors.drain(..errors.len() - KEPT_ERRORS);
                }
            }
        }
        let code = child.wait().ok().and_then(|status| status.code());
        Ok((code, String::from_utf8_lossy(&errors).into_owned()))
    }
}

/// The packages a failed Node program could not load, with the files that
/// imported them, from Node's errors: `Cannot find package 'name' imported
/// from file` for ES modules and `Cannot find module 'name'` with its
/// require stack for CommonJS. Relative and absolute specifiers and
/// built-in modules name no package.
pub(crate) fn missing_imports(errors: &str) -> Vec<(String, PathBuf)> {
    let lines: Vec<&str> = errors.lines().collect();
    let mut found = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        for marker in ["Cannot find package '", "Cannot find module '"] {
            let Some(start) = line.find(marker) else {
                continue;
            };
            let rest = &line[start + marker.len()..];
            let Some(end) = rest.find('\'') else {
                continue;
            };
            let importer = rest[end + 1..]
                .strip_prefix(" imported from ")
                .or_else(|| {
                    lines[index + 1..]
                        .iter()
                        .take(3)
                        .skip_while(|line| line.trim() != "Require stack:")
                        .nth(1)
                        .and_then(|line| line.trim().strip_prefix("- "))
                })
                .map(|file| PathBuf::from(file.trim()));
            if let (Some(package), Some(importer)) = (package_name(&rest[..end]), importer) {
                found.push((package, importer));
            }
        }
    }
    found
}

/// The package a bare specifier names: its first path segment, or two for
/// a scoped package.
fn package_name(specifier: &str) -> Option<String> {
    if specifier.is_empty() || specifier.starts_with(['.', '/', '\\']) || specifier.contains(':') {
        return None;
    }
    let mut segments = specifier.split('/');
    let first = segments.next()?;
    if first.starts_with('@') {
        Some(format!(
            "{first}/{}",
            segments.next().filter(|name| !name.is_empty())?
        ))
    } else {
        Some(first.to_owned())
    }
}

#[cfg(test)]
mod tests;
