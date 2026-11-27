//! Command line front end of the nativeboat compiler.
//!
//! `nativeboat build` compiles the project's modules into a native executable
//! without a JavaScript engine; `analyze` inventories compatibility,
//! separately from native compilation completeness.

mod classic_script;
mod css_imports;
mod installer;
mod native;
mod node_lib;
mod packages;
mod prepare;
mod resources;
mod script_templates;
pub mod steps;
mod tailwind_tool;
mod vite_glob;
mod vue_sfc;
mod vue_tool;
mod web_prelude;
mod worker_scripts;

use clap::{Args, Parser, Subcommand};
use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;

/// An embedded text file, unscrambled on first use; see `nativeboat_conceal`.
pub(crate) type Text = fn() -> &'static str;

/// Parsed command line.
#[derive(Debug, Parser)]
#[command(
    name = "nativeboat",
    version,
    about = "Compile unchanged Electron applications into native Windows applications"
)]
pub struct Cli {
    /// The command to run.
    #[command(subcommand)]
    pub command: Command,
}

/// A `nativeboat` command.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Compile the Electron project into a native executable without a JavaScript engine.
    Build(native::Args),
    /// Report how much of the Electron project the native target supports.
    Analyze(AnalyzeArgs),
    /// Precompile the runtime libraries into a toolchain, for packaging.
    #[cfg(feature = "embedded-runtime")]
    #[command(hide = true)]
    ExportRuntime(native::ExportArgs),
}

/// Options of `nativeboat analyze`.
#[derive(Debug, Args)]
pub struct AnalyzeArgs {
    /// Electron project directory.
    #[arg(default_value = ".")]
    pub project: PathBuf,
    /// Print the report as JSON.
    #[arg(long)]
    pub json: bool,
}

/// Runs the command line and returns the process exit code.
pub fn run<'a, I, T>(args: I, stdout: &'a mut dyn Write, stderr: &'a mut dyn Write) -> u8
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    match Cli::try_parse_from(args) {
        Ok(cli) => execute(cli, stdout, stderr),
        Err(error) => {
            let text = error.render().to_string();
            let sink = if error.use_stderr() { stderr } else { stdout };
            let _ = sink.write_all(text.as_bytes());
            u8::try_from(error.exit_code()).unwrap_or(1)
        }
    }
}

fn execute(cli: Cli, stdout: &mut dyn Write, stderr: &mut dyn Write) -> u8 {
    match cli.command {
        Command::Analyze(args) => analyze(&args, stdout, stderr),
        Command::Build(args) => native::run(&args, stdout, stderr),
        #[cfg(feature = "embedded-runtime")]
        Command::ExportRuntime(args) => native::export_runtime(&args, stdout, stderr),
    }
}

fn load(path: &std::path::Path, stderr: &mut dyn Write) -> Option<nativeboat_project::Project> {
    nativeboat_project::load(path)
        .map_err(|error| {
            let _ = writeln!(stderr, "error: {error}");
        })
        .ok()
}

fn analyze(args: &AnalyzeArgs, stdout: &mut dyn Write, stderr: &mut dyn Write) -> u8 {
    let Some(project) = load(&args.project, stderr) else {
        return 1;
    };
    let report = nativeboat_analyzer::analyze(&project);
    // Native modules are recognized by their installed packages.
    if let Some(install) = nativeboat_project::install::plan(&project.root, false) {
        let _ = writeln!(
            stderr,
            "note: {} dependencies are not installed (such as {}), so native modules among them are not reported; install them first",
            install.missing.len(),
            install.missing[0]
        );
    }
    let text = if args.json {
        report.to_json()
            + "
"
    } else {
        report.to_text()
    };
    let _ = stdout.write_all(text.as_bytes());
    0
}

#[cfg(test)]
mod tests;
