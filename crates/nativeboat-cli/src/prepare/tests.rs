use super::*;
use crate::tests::fixture::Fixture;

/// A package manager that installs `left-pad` into the current directory
/// and records its arguments and environment next to itself, or fails while
/// a `fail` file sits next to it.
const FAKE_MANAGER: &str = "@echo off\r
echo %*^|%CI%^|%NODE_ENV%>>\"%~dp0calls.txt\"\r
if exist \"%~dp0fail\" exit /b 7\r
if not exist node_modules\\left-pad mkdir node_modules\\left-pad\r
>node_modules\\left-pad\\package.json echo {\"name\":\"left-pad\",\"main\":\"index.js\"}\r
>node_modules\\left-pad\\index.js echo 'use strict';module.exports=function(text){return text};\r
";

const PAD_APP: &str = r#"{"name":"pad-app","main":"main.js","packageManager":"pnpm@9.0.0","dependencies":{"left-pad":"1"},"devDependencies":{"electron":"30"}}"#;

const MAIN: &str = "'use strict'; module.exports = require('left-pad')('padded');";

fn build(fixture: &Fixture, project: &str, extra: &[&str]) -> (u8, String, String) {
    let project = fixture.path(project).to_string_lossy().into_owned();
    let work = fixture.path("work").to_string_lossy().into_owned();
    let mut args = vec!["nativeboat", "build", &project, "--work-dir", &work];
    args.extend_from_slice(extra);
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = crate::run(args, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

fn calls(fixture: &Fixture) -> Vec<String> {
    std::fs::read_to_string(fixture.path("work/node_modules/.bin/calls.txt"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// The only build copy in the fixture's work directory.
fn mirror(fixture: &Fixture) -> PathBuf {
    let mirrors: Vec<PathBuf> = std::fs::read_dir(fixture.path("work"))
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && !path.ends_with("node_modules"))
        .collect();
    assert_eq!(mirrors.len(), 1, "{mirrors:?}");
    mirrors[0].clone()
}

/// Runs a built executable, which must exit successfully.
fn runs(executable: &Path) {
    let status = Command::new(executable).status().unwrap();
    assert!(status.success(), "{status:?}");
}

#[test]
fn missing_dependencies_install_into_a_copy_that_later_builds_reuse() {
    let package = PAD_APP.to_owned();
    let fixture = Fixture::new(&[
        ("app/package.json", &package),
        ("app/pnpm-lock.yaml", "lockfileVersion: '9.0'"),
        ("app/main.js", MAIN),
        ("work/node_modules/.bin/pnpm.cmd", FAKE_MANAGER),
    ]);
    let (code, out, err) = build(&fixture, "app", &[]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        out.starts_with("Installing dependencies (1 missing, such as left-pad) to build from "),
        "{out}"
    );
    assert!(out.contains("pnpm.cmd\" install --ignore-scripts --no-frozen-lockfile"));
    assert_eq!(
        calls(&fixture),
        ["install --ignore-scripts --no-frozen-lockfile|true|development"]
    );
    // The executable reaches the project's output; the project's folder is
    // otherwise unchanged.
    runs(&fixture.path("app/dist/pad-app.exe"));
    assert!(!fixture.path("app/node_modules").exists());
    let mirror = mirror(&fixture);
    assert!(
        mirror
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("app-")
    );
    assert!(mirror.join("node_modules/left-pad/index.js").is_file());

    // Unchanged manifests reuse the installation; files the project no
    // longer has leave the copy, and its output never enters it.
    std::fs::write(mirror.join("stale.txt"), "stale").unwrap();
    std::fs::create_dir_all(mirror.join("old/deeper")).unwrap();
    std::fs::write(mirror.join("old/deeper/x.txt"), "x").unwrap();
    std::fs::write(mirror.join("main.js"), "changed in the copy").unwrap();
    let (code, out, err) = build(&fixture, "app", &[]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        out.starts_with(&format!("Building from {}", mirror.display())),
        "{out}"
    );
    assert_eq!(calls(&fixture).len(), 1);
    assert!(!mirror.join("stale.txt").exists());
    assert!(!mirror.join("old").exists());
    assert!(!mirror.join("dist").exists());
    assert_eq!(
        std::fs::read_to_string(mirror.join("main.js")).unwrap(),
        MAIN
    );
    assert!(mirror.join("node_modules/left-pad/index.js").is_file());

    // Changed manifests install again.
    fixture.write(
        "app/package.json",
        package.replace("pad-app", "pad-app2").as_bytes(),
    );
    let (code, out, err) = build(&fixture, "app", &[]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.starts_with("Installing dependencies"), "{out}");
    assert_eq!(calls(&fixture).len(), 2);
    assert!(fixture.path("app/dist/pad-app2.exe").is_file());
}

#[test]
fn failed_preparations_stop_the_build() {
    let fixture = Fixture::new(&[
        ("app/package.json", PAD_APP),
        ("app/pnpm-lock.yaml", ""),
        ("app/main.js", MAIN),
        ("work/node_modules/.bin/pnpm.cmd", FAKE_MANAGER),
        ("work/node_modules/.bin/fail", ""),
    ]);
    let (code, _, err) = build(&fixture, "app", &[]);
    assert_eq!(code, 1);
    assert!(
        err.contains("error: the nativeboat:install script failed with exit code 7"),
        "{err}"
    );
    let mirror = mirror(&fixture);
    let stamp = mirror.with_extension("installed");
    assert!(!stamp.exists());
    assert!(!fixture.path("app/dist").exists());

    // The stamp cannot be written.
    std::fs::remove_file(fixture.path("work/node_modules/.bin/fail")).unwrap();
    std::fs::create_dir(&stamp).unwrap();
    let (code, _, err) = build(&fixture, "app", &[]);
    assert_eq!(code, 1);
    assert!(
        err.contains(&format!("error: {}: ", stamp.display())),
        "{err}"
    );
    std::fs::remove_dir(&stamp).unwrap();

    // A file cannot be copied.
    fixture.write("app/extra.txt", b"extra");
    std::fs::create_dir_all(mirror.join("extra.txt/node_modules")).unwrap();
    let (code, _, err) = build(&fixture, "app", &[]);
    assert_eq!(code, 1);
    let blocked = mirror.join("extra.txt");
    assert!(
        err.contains(&format!("error: {}: ", blocked.display())),
        "{err}"
    );
    std::fs::remove_dir_all(&blocked).unwrap();
    let (code, _, err) = build(&fixture, "app", &[]);
    assert_eq!(code, 0, "{err}");

    // The work directory cannot be created.
    let blocker = fixture.path("blocker");
    std::fs::write(&blocker, "").unwrap();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = crate::run(
        [
            "nativeboat",
            "build",
            fixture.path("app").to_str().unwrap(),
            "--work-dir",
            blocker.to_str().unwrap(),
        ],
        &mut out,
        &mut err,
    );
    assert_eq!(code, 1);
    let err = String::from_utf8(err).unwrap();
    assert!(
        err.starts_with(&format!("error: {}: ", blocker.display())),
        "{err}"
    );
}

fn git(directory: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

#[test]
fn repositories_are_mirrored_as_checked_out() {
    let fixture = Fixture::new(&[
        (
            "repo/.gitignore",
            "ignored/\n.env.local\n.env/\ndist/\nnode_modules/\n",
        ),
        ("repo/app/package.json", PAD_APP),
        ("repo/app/pnpm-lock.yaml", ""),
        ("repo/app/main.js", MAIN),
        ("repo/ignored/big.bin", "big"),
        ("repo/.env.local", "VITE_LOCAL=1"),
        ("repo/.env/lib.py", ""),
        ("repo/deleted.txt", ""),
        ("repo/nested/file.txt", "nested"),
        ("repo/vendor/node_modules/x/package.json", "{}"),
        ("work/node_modules/.bin/pnpm.cmd", FAKE_MANAGER),
    ]);
    let repo = fixture.path("repo");
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "deleted.txt"]);
    std::fs::remove_file(repo.join("deleted.txt")).unwrap();
    git(&repo.join("nested"), &["init", "-q"]);
    // Packages a repository tracks are not mirrored either.
    git(&repo, &["add", "-f", "vendor"]);
    let (code, out, err) = build(&fixture, "repo/app", &[]);
    assert_eq!(code, 0, "{out}{err}");
    let mirror = mirror(&fixture);
    assert!(
        mirror
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("repo-")
    );
    assert!(mirror.join("app/main.js").is_file());
    assert!(mirror.join("app/node_modules/left-pad/index.js").is_file());
    assert!(mirror.join(".gitignore").is_file());
    assert!(mirror.join(".env.local").is_file());
    assert!(mirror.join("nested/file.txt").is_file());
    for absent in [
        "ignored",
        ".env",
        "deleted.txt",
        "nested/.git",
        ".git",
        "vendor",
    ] {
        assert!(!mirror.join(absent).exists(), "{absent}");
    }
    assert!(!repo.join("app/node_modules").exists());
    runs(&repo.join("app/dist/pad-app.exe"));
}

#[test]
fn folders_git_cannot_list_are_mirrored_whole() {
    let fixture = Fixture::new(&[
        ("broken/.git", "gitdir: nowhere"),
        ("broken/package.json", PAD_APP),
        ("broken/pnpm-lock.yaml", ""),
        ("broken/main.js", MAIN),
        ("broken/node_modules/other/package.json", "{}"),
        ("broken/generated/out.txt", "kept"),
        ("work/node_modules/.bin/pnpm.cmd", FAKE_MANAGER),
    ]);
    let (code, out, err) = build(&fixture, "broken", &[]);
    assert_eq!(code, 0, "{out}{err}");
    let mirror = mirror(&fixture);
    assert!(mirror.join("generated/out.txt").is_file());
    assert!(!mirror.join(".git").exists());
    assert!(!mirror.join("node_modules/other").exists());
}

#[test]
fn workspace_members_are_built_from_a_copy_of_the_workspace() {
    let member = PAD_APP.replace(r#""packageManager":"pnpm@9.0.0","#, "");
    let fixture = Fixture::new(&[
        ("ws/package.json", r#"{"name":"ws","workspaces":["app"]}"#),
        ("ws/app/package.json", &member),
        ("ws/app/main.js", MAIN),
        ("work/node_modules/.bin/npm.cmd", FAKE_MANAGER),
    ]);
    let (code, out, err) = build(&fixture, "ws/app", &[]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        out.contains("npm.cmd\" install --ignore-scripts --include=dev"),
        "{out}"
    );
    let mirror = mirror(&fixture);
    assert!(
        mirror
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("ws-")
    );
    assert!(mirror.join("node_modules/left-pad/index.js").is_file());
    assert!(fixture.path("ws/app/dist/pad-app.exe").is_file());
}

fn found(names: &'static [&'static str]) -> impl Fn(&str) -> Option<PathBuf> {
    move |program| {
        names
            .contains(&program)
            .then(|| PathBuf::from(format!("C:\\bin\\{program}.cmd")))
    }
}

fn install(manager: Manager, version: Option<&str>) -> Install {
    Install {
        root: PathBuf::from("C:\\app"),
        manager,
        version: version.map(str::to_owned),
        missing: vec!["a".to_owned()],
    }
}

fn installer(
    manager: Manager,
    version: Option<&str>,
    names: &'static [&'static str],
) -> Result<String, String> {
    command(&install(manager, version), &found(names)).map(|(command, _)| command)
}

#[test]
fn installed_package_managers_run_before_corepack_and_npm() {
    const ALL: &[&str] = &["npm", "pnpm", "yarn", "bun", "corepack"];
    assert_eq!(
        installer(Manager::Npm, None, ALL).unwrap(),
        "\"C:\\bin\\npm.cmd\" install --ignore-scripts --include=dev --no-audit --no-fund"
    );
    assert_eq!(
        installer(Manager::Pnpm, Some("9.0.0"), ALL).unwrap(),
        "\"C:\\bin\\pnpm.cmd\" install --ignore-scripts --no-frozen-lockfile"
    );
    assert_eq!(
        installer(Manager::Yarn, None, ALL).unwrap(),
        "\"C:\\bin\\yarn.cmd\" install --ignore-scripts --non-interactive"
    );
    assert_eq!(
        installer(Manager::Bun, None, ALL).unwrap(),
        "\"C:\\bin\\bun.cmd\" install --ignore-scripts"
    );
    assert_eq!(
        installer(Manager::YarnBerry, None, ALL).unwrap(),
        "\"C:\\bin\\yarn.cmd\" install --mode=skip-build --no-immutable"
    );
    // Corepack runs pinned versions, also of Yarn when Yarn 1 is installed.
    assert_eq!(
        installer(Manager::YarnBerry, Some("4.1.0"), ALL).unwrap(),
        "\"C:\\bin\\corepack.cmd\" yarn install --mode=skip-build --no-immutable"
    );
    assert_eq!(
        installer(Manager::Pnpm, Some("9.0.0"), &["npm", "corepack"]).unwrap(),
        "\"C:\\bin\\corepack.cmd\" pnpm install --ignore-scripts --no-frozen-lockfile"
    );
    // npm fetches what Corepack does not run.
    assert_eq!(
        installer(Manager::Pnpm, None, &["npm", "corepack"]).unwrap(),
        "\"C:\\bin\\npm.cmd\" exec --yes --package=pnpm@latest -- pnpm install --ignore-scripts --no-frozen-lockfile"
    );
    assert_eq!(
        installer(Manager::Bun, Some("1.1.0"), &["npm", "corepack"]).unwrap(),
        "\"C:\\bin\\npm.cmd\" exec --yes --package=bun@1.1.0 -- bun install --ignore-scripts"
    );
    assert_eq!(
        installer(Manager::YarnBerry, Some("4.1.0"), &["npm", "yarn"]).unwrap(),
        "\"C:\\bin\\npm.cmd\" exec --yes --package=@yarnpkg/cli-dist@4.1.0 -- yarn install --mode=skip-build --no-immutable"
    );
    for manager in [Manager::Npm, Manager::Bun] {
        let error = installer(manager, Some("1.0.0"), &["corepack"]).unwrap_err();
        assert!(
            error.starts_with(&format!(
                "installing the project's dependencies needs {}, which was not found",
                manager.program()
            )),
            "{error}"
        );
    }
}

#[test]
fn installations_use_development_settings_and_node_modules() {
    let (_, environment) = command(&install(Manager::Npm, None), &found(&["npm"])).unwrap();
    assert_eq!(
        environment,
        [
            ("CI", "true"),
            ("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0"),
            ("NODE_ENV", "development")
        ]
    );
    let (_, environment) = command(&install(Manager::YarnBerry, None), &found(&["yarn"])).unwrap();
    assert_eq!(
        environment[3..],
        [
            ("YARN_NODE_LINKER", "node-modules"),
            ("YARN_ENABLE_SCRIPTS", "0")
        ]
    );
}

#[test]
fn copies_map_paths_below_their_source() {
    let copy = Copy {
        source: PathBuf::from("C:\\project"),
        target: PathBuf::from("D:\\work\\project-1"),
    };
    assert_eq!(copy.of(Path::new("C:\\project")), copy.target);
    assert_eq!(
        copy.of(Path::new("C:\\project\\packages\\app")),
        Path::new("D:\\work\\project-1\\packages\\app")
    );
    assert_eq!(copy.of(Path::new("C:\\other")), Path::new("C:\\other"));
    assert!(default_work_dir().ends_with("nativeboat\\projects"));
    assert_eq!(
        mirror_name(Path::new("C:\\Some Project (1)")),
        mirror_name(Path::new("c:\\some project (1)")).replace("some_project", "Some_Project")
    );
    assert!(mirror_name(Path::new("C:\\Some Project (1)")).starts_with("Some_Project__1_-"));
    assert_ne!(
        mirror_name(Path::new("C:\\a\\app")),
        mirror_name(Path::new("C:\\b\\app"))
    );
}

/// A build step that fails for a package a folder of the project declares,
/// with a lockfile of its own, gets that folder's dependencies installed,
/// once, as VS Code's `build` tools need theirs. Importers outside the
/// project, undeclared packages and folders with nothing missing install
/// nothing.
#[test]
fn packages_build_steps_miss_are_installed_in_the_folders_declaring_them() {
    let fixture = Fixture::new(&[
        ("app/package.json", r#"{"name":"app"}"#),
        ("app/node_modules/.bin/npm.cmd", FAKE_MANAGER),
        (
            "app/tools/package.json",
            r#"{"name":"tools","devDependencies":{"left-pad":"1"}}"#,
        ),
        ("app/tools/package-lock.json", "{}"),
        (
            "app/other/package.json",
            r#"{"name":"other","devDependencies":{"left-pad":"1"}}"#,
        ),
        ("app/other/package-lock.json", "{}"),
    ]);
    let root = fixture.path("app");
    let shell = crate::steps::shell();
    let errors = format!(
        "Error [ERR_MODULE_NOT_FOUND]: Cannot find package 'left-pad' imported from {}\n",
        root.join("tools/lib/facade.mjs").display()
    );
    let mut tried = BTreeSet::new();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert!(install_missing(
        &root, &errors, &shell, &mut tried, &mut out, &mut err
    ));
    let out = String::from_utf8_lossy(&out);
    assert!(
        out.contains("Installing the dependencies of") && out.contains("for left-pad"),
        "{out}"
    );
    assert!(
        out.contains("npm.cmd\" install --ignore-scripts --include=dev"),
        "{out}"
    );
    assert!(
        root.join("tools/node_modules/left-pad/package.json")
            .is_file()
    );
    let undeclared = format!(
        "Error: Cannot find module 'right-pad'\nRequire stack:\n- {}\n",
        root.join("other/x.js").display()
    );
    let cases = [
        errors,
        format!(
            "Cannot find package 'left-pad' imported from {}\n",
            fixture.path("elsewhere/x.mjs").display()
        ),
        undeclared,
        format!(
            "Cannot find package 'left-pad' imported from {}\n",
            root.join("lib/x.mjs").display()
        ),
    ];
    for errors in cases {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert!(
            !install_missing(&root, &errors, &shell, &mut tried, &mut out, &mut err),
            "{errors}"
        );
    }
    assert!(!root.join("other/node_modules").exists());
}

/// Plugin packages with a manifest and lockfile of their own get their
/// runtime dependencies installed once; installed and dependency-free
/// plugins install nothing.
#[test]
fn plugin_packages_get_their_runtime_dependencies_installed() {
    let fixture = Fixture::new(&[
        (
            "app/package.json",
            r#"{"name":"app","main":"main.js","nativeboat":{"plugins":["plugins/*"]},"devDependencies":{"electron":"30"}}"#,
        ),
        ("app/main.js", "console.log('main');"),
        ("app/node_modules/.bin/npm.cmd", FAKE_MANAGER),
        (
            "app/plugins/p/package.json",
            r#"{"name":"p","main":"index.js","dependencies":{"left-pad":"1"}}"#,
        ),
        ("app/plugins/p/package-lock.json", "{}"),
        (
            "app/plugins/p/index.js",
            "module.exports = require('left-pad');",
        ),
        (
            "app/plugins/q/package.json",
            r#"{"name":"q","main":"index.js"}"#,
        ),
        ("app/plugins/q/index.js", "module.exports = 1;"),
    ]);
    let root = fixture.path("app");
    let project = nativeboat_project::load(&root).unwrap();
    assert_eq!(project.plugins.len(), 2);
    let shell = crate::steps::shell();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert!(install_plugin_dependencies(
        &project, &shell, &mut out, &mut err
    ));
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("Installing the dependencies of the plugin package")
            && (text.contains(r"plugins\p") || text.contains("plugins/p")),
        "{text}"
    );
    assert!(
        root.join("plugins/p/node_modules/left-pad/package.json")
            .is_file()
    );
    assert!(!root.join("plugins/q/node_modules").exists());
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert!(!install_plugin_dependencies(
        &project, &shell, &mut out, &mut err
    ));
}

/// The package around plugin packages, which keeps what they share, gets
/// its dependencies installed too.
#[test]
fn packages_around_plugins_get_their_dependencies_installed() {
    let fixture = Fixture::new(&[
        (
            "app/package.json",
            r#"{"name":"app","main":"main.js","nativeboat":{"plugins":["plugins/*"]},"devDependencies":{"electron":"30"}}"#,
        ),
        ("app/main.js", "console.log('main');"),
        ("app/node_modules/.bin/npm.cmd", FAKE_MANAGER),
        (
            "app/plugins/package.json",
            r#"{"name":"plugins","private":true,"dependencies":{"left-pad":"1"}}"#,
        ),
        ("app/plugins/package-lock.json", "{}"),
        (
            "app/plugins/p/package.json",
            r#"{"name":"p","main":"index.js"}"#,
        ),
        ("app/plugins/p/index.js", "module.exports = 1;"),
    ]);
    let root = fixture.path("app");
    let project = nativeboat_project::load(&root).unwrap();
    let shell = crate::steps::shell();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert!(install_plugin_dependencies(
        &project, &shell, &mut out, &mut err
    ));
    assert!(
        root.join("plugins/node_modules/left-pad/package.json")
            .is_file()
    );
    assert!(!root.join("plugins/p/node_modules").exists());
}
