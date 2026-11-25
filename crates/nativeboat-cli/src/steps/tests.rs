use super::*;
use crate::tests::fixture::Fixture;

fn step(script: &str, command: &str) -> Step {
    Step {
        script: script.to_owned(),
        command: command.to_owned(),
        optional: false,
    }
}

fn run_steps(root: &Path, steps: &[Step], shell: &Path) -> (Result<(), String>, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let result = run(root, steps, shell, &mut out, &mut err);
    (
        result,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

#[test]
fn steps_run_in_the_project_as_npm_runs_them() {
    let fixture = Fixture::new(&[("node_modules/.bin/greet.cmd", "@echo hello from bin")]);
    let (result, out, err) = run_steps(
        fixture.root(),
        &[
            step("first", "greet && echo oops 1>&2"),
            step("second", "echo %npm_lifecycle_event%> event.txt"),
        ],
        &shell(),
    );
    assert_eq!(result, Ok(()));
    assert!(out.starts_with("> first: greet && echo oops 1>&2"), "{out}");
    assert!(out.contains("hello from bin"));
    assert!(out.contains("> second: "));
    assert!(err.contains("oops"));
    let event = std::fs::read_to_string(fixture.path("event.txt")).unwrap();
    assert_eq!(event.trim(), "second");
}

#[test]
fn failing_steps_stop_the_build() {
    let fixture = Fixture::new(&[]);
    let (result, out, _) = run_steps(
        fixture.root(),
        &[step("bad", "exit 3"), step("never", "echo never")],
        &shell(),
    );
    assert_eq!(
        result,
        Err("the bad script failed with exit code 3: exit 3".to_owned())
    );
    assert!(!out.contains("never"));
    let (result, _, _) = run_steps(
        fixture.root(),
        &[step("any", "echo x")],
        Path::new("Z:\\missing\\cmd.exe"),
    );
    assert!(
        result
            .unwrap_err()
            .starts_with("cannot run Z:\\missing\\cmd.exe: ")
    );
}

#[test]
fn steps_take_additional_environment_and_unterminated_output() {
    let fixture = Fixture::new(&[]);
    let mut out = Vec::new();
    let mut err = Vec::new();
    let result = run_with(
        fixture.root(),
        &[
            step("env", "echo %EXTRA%"),
            step("partial", "<nul set /p =no newline& exit 0"),
        ],
        &shell(),
        &[("EXTRA", "extra value")],
        &mut |_, _, _| false,
        &mut out,
        &mut err,
    );
    assert_eq!(result, Ok(()));
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("extra value\r\n"), "{out}");
    assert!(out.ends_with("no newline"), "{out}");
}

#[test]
fn shell_messages_reach_the_output_as_utf8() {
    let fixture = Fixture::new(&[]);
    let mut out = Vec::new();
    let mut err = Vec::new();
    let result = run_with(
        fixture.root(),
        &[step("missing", "nativeboat-no-such-tool")],
        &shell(),
        &[],
        &mut |_, _, _| false,
        &mut out,
        &mut err,
    );
    assert!(result.is_err());
    let err = String::from_utf8(err).unwrap();
    assert!(err.contains("'nativeboat-no-such-tool'"), "{err}");
}

#[test]
fn output_in_the_console_code_page_is_converted() {
    assert_eq!(decode("caf\u{e9}".as_bytes()), "caf\u{e9}".as_bytes());
    let mut session = shimforge::Session::new();
    let page = shimforge::mock!(
        session,
        GetConsoleOutputCP,
        unsafe extern "system" fn() -> u32
    );
    page.expect().times(1).returning(|| 1252);
    assert_eq!(decode(b"caf\xe9\r\n"), "caf\u{e9}\r\n".as_bytes());
    // Without a console the OEM code page applies.
    page.expect().times(1).returning(|| 0);
    assert!(String::from_utf8(decode(b"caf\xe9")).is_ok());
    // Unknown code pages keep what is valid.
    page.expect().times(1).returning(|| 12345);
    assert_eq!(decode(b"caf\xe9"), "caf\u{fffd}".as_bytes());
}

/// A failed step runs again once `recover` fixes what it lacked, a failed
/// optional step is reported while the others still run, and a required
/// step that stays failed stops the run with the end of its error output
/// given to `recover`.
#[test]
fn failed_steps_run_again_when_recovered_and_optional_ones_are_reported() {
    let fixture = Fixture::new(&[]);
    let steps = [
        Step {
            script: "needs".to_owned(),
            command: "if not exist ready.txt (echo missing ready 1>&2 & exit /b 3)".to_owned(),
            optional: false,
        },
        Step {
            script: "guess".to_owned(),
            command: "exit /b 5".to_owned(),
            optional: true,
        },
        step("after", "echo after> after.txt"),
    ];
    let mut seen = Vec::new();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let result = run_with(
        fixture.root(),
        &steps,
        &shell(),
        &[],
        &mut |errors, _, _| {
            seen.push(errors.to_owned());
            if seen.len() == 1 {
                std::fs::write(fixture.path("ready.txt"), "").unwrap();
            }
            seen.len() == 1
        },
        &mut out,
        &mut err,
    );
    assert_eq!(result, Ok(()));
    assert_eq!(seen.len(), 2);
    assert!(seen[0].contains("missing ready"), "{seen:?}");
    let err = String::from_utf8_lossy(&err);
    assert!(
        err.contains(
            "warning: the guess script failed with exit code 5: exit /b 5; the build goes on without what it makes"
        ),
        "{err}"
    );
    assert!(fixture.path("after.txt").is_file());
    let mut kept = String::new();
    let result = run_with(
        fixture.root(),
        &[step(
            "fails",
            "(for /l %i in (1,1,3000) do @echo %i padding padding padding padding padding 1>&2) & exit /b 2",
        )],
        &shell(),
        &[],
        &mut |errors, _, _| {
            kept = errors.to_owned();
            false
        },
        &mut Vec::new(),
        &mut Vec::new(),
    );
    assert_eq!(
        result,
        Err("the fails script failed with exit code 2: (for /l %i in (1,1,3000) do @echo %i padding padding padding padding padding 1>&2) & exit /b 2".to_owned())
    );
    assert!(kept.len() <= 2 * KEPT_ERRORS, "{}", kept.len());
    assert!(
        kept.trim_end()
            .ends_with("3000 padding padding padding padding padding")
    );
    assert!(!kept.contains("\n1 padding"));
}

/// The packages Node could not load and the files that imported them, for
/// ES modules and CommonJS; relative, absolute and built-in specifiers and
/// errors without an importer name nothing.
#[test]
fn missing_packages_are_read_from_node_errors() {
    let errors = "Error [ERR_MODULE_NOT_FOUND]: Cannot find package 'gulp-merge-json' imported from C:\\repo\\build\\lib\\facade.ts\n\
        Error: Cannot find module '@scope/tool/lib/x'\n\
        Require stack:\n\
        - C:\\repo\\tools\\a.js\n\
        - C:\\repo\\b.js\n\
        Error: Cannot find module './local' imported from C:\\repo\\c.mjs\n\
        Error: Cannot find module 'node:missing'\n\
        Require stack:\n\
        - C:\\repo\\d.js\n\
        Error: Cannot find module 'lonely'\n\
        Cannot find package 'unterminated\n\
        Cannot find package '@scope' imported from C:\\repo\\e.mjs\n";
    assert_eq!(
        missing_imports(errors),
        vec![
            (
                "gulp-merge-json".to_owned(),
                PathBuf::from("C:\\repo\\build\\lib\\facade.ts")
            ),
            (
                "@scope/tool".to_owned(),
                PathBuf::from("C:\\repo\\tools\\a.js")
            ),
        ]
    );
    assert_eq!(package_name(""), None);
    assert_eq!(package_name("/absolute"), None);
    assert_eq!(package_name("\\absolute"), None);
    assert_eq!(package_name("C:\\x"), None);
    assert_eq!(package_name("@scope/"), None);
    assert_eq!(package_name("plain/sub/path"), Some("plain".to_owned()));
}
