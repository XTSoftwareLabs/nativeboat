pub(crate) mod fixture;

use super::*;
use fixture::Fixture;

fn run_args(args: &[&str]) -> (u8, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = run(args.iter().copied(), &mut out, &mut err);
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

#[test]
fn version_goes_to_stdout_with_success() {
    let (code, out, err) = run_args(&["nativeboat", "--version"]);
    assert_eq!(code, 0);
    assert!(out.contains(env!("CARGO_PKG_VERSION")));
    assert!(err.is_empty());
}

#[test]
fn unknown_arguments_fail_on_stderr() {
    let (code, out, err) = run_args(&["nativeboat", "--bogus"]);
    assert_eq!(code, 2);
    assert!(out.is_empty());
    assert!(err.contains("--bogus"));
}

#[test]
fn build_parses_project_and_output() {
    let cli = Cli::try_parse_from(["nativeboat", "build", "app", "--out", "o"]).unwrap();
    match cli.command {
        Command::Build(args) => {
            assert_eq!(args.project, PathBuf::from("app"));
            assert_eq!(args.out, Some(PathBuf::from("o")));
        }
        _ => panic!("expected build"),
    }
}

#[test]
fn analyze_defaults_to_the_current_directory() {
    let cli = Cli::try_parse_from(["nativeboat", "analyze", "--json"]).unwrap();
    match cli.command {
        Command::Analyze(args) => {
            assert_eq!(args.project, PathBuf::from("."));
            assert!(args.json);
        }
        _ => panic!("expected analyze"),
    }
}

#[test]
fn analyze_prints_text_and_json_reports() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"main": "main.js", "devDependencies": {"electron": "30"}}"#,
        ),
        ("main.js", "const { app } = require('electron');"),
    ]);
    let root = fixture.root().to_string_lossy().into_owned();
    let (code, out, err) = run_args(&["nativeboat", "analyze", &root]);
    assert_eq!(code, 0, "{err}");
    assert!(out.starts_with("Project compatibility: 100%"));
    assert!(out.contains("Electron app"));
    let (code, out, _) = run_args(&["nativeboat", "analyze", &root, "--json"]);
    assert_eq!(code, 0);
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json["score"], 100);
    assert!(err.is_empty(), "{err}");
}

/// Native modules are found among installed packages, so a report on a
/// project whose dependencies are missing says that it cannot list them.
#[test]
fn analyze_notes_dependencies_that_are_not_installed() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"main": "main.js", "dependencies": {"left-pad": "1", "sqlite": "5"}, "devDependencies": {"electron": "30"}}"#,
        ),
        ("main.js", "const { app } = require('electron');"),
    ]);
    let root = fixture.root().to_string_lossy().into_owned();
    let (code, out, err) = run_args(&["nativeboat", "analyze", &root]);
    assert_eq!(code, 0, "{err}");
    assert!(out.starts_with("Project compatibility: 100%"));
    assert_eq!(
        err,
        "note: 2 dependencies are not installed (such as left-pad), so native modules among them are not reported; install them first\n"
    );
}

#[test]
fn analyze_reports_invalid_projects() {
    let fixture = Fixture::new(&[]);
    let root = fixture.root().to_string_lossy().into_owned();
    let (code, out, err) = run_args(&["nativeboat", "analyze", &root]);
    assert_eq!(code, 1);
    assert!(out.is_empty());
    assert!(err.starts_with("error: cannot read "));
}
