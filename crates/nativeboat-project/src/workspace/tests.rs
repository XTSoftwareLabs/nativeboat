use super::*;
use crate::tests::fixture::Fixture;

#[test]
fn pnpm_tooling_root_selects_the_desktop_sources_and_application_metadata() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"tooling","version":"9.0.0","devDependencies":{"electron":"42","typescript":"6"},"dependencies":{"react":"18"}}"#,
        ),
        (
            "pnpm-workspace.yaml",
            "packages:\n  - 'packages/*'\n  - 'packages/library/examples'\n# unrelated settings\nallowBuilds:\n  electron: true\n",
        ),
        (
            "packages/desktop/package.json",
            r#"{"name":"desktop","version":"1.2.3","main":"out/main/index.js","devDependencies":{"electron-vite":"5"},"dependencies":{"react":"19"}}"#,
        ),
        (
            "packages/desktop/src/main/index.ts",
            "import {app} from 'electron';",
        ),
        ("packages/desktop/src/preload/index.ts", ""),
        (
            "packages/desktop/src/renderer/index.html",
            "<h1>Desktop</h1>",
        ),
        (
            "packages/library/package.json",
            r#"{"name":"library","main":"index.js"}"#,
        ),
        ("packages/library/index.js", "exports.value=1"),
        (
            "packages/library/examples/package.json",
            r#"{"name":"example"}"#,
        ),
    ]);
    let project = load(fixture.root()).unwrap();
    assert_eq!(project.root, fixture.path("packages/desktop"));
    assert_eq!(
        project.main.source,
        fixture.path("packages/desktop/src/main/index.ts")
    );
    assert_eq!(project.main.runtime_paths, vec!["out/main/index.js"]);
    assert_eq!(project.metadata.name, "desktop");
    assert_eq!(project.metadata.version, "1.2.3");
    assert_eq!(project.dependencies["react"], "19");
    assert_eq!(project.dependencies["electron"], "42");
    assert_eq!(project.layout, Layout::ElectronVite);
    assert_eq!(
        project.pages[0].source,
        fixture.path("packages/desktop/src/renderer/index.html")
    );
}

#[test]
fn workspace_extglobs_select_declared_applications_and_exclude_alternatives() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"workspaces":["apps/@(desktop|legacy)","!apps/+(legacy)"]}"#,
        ),
        (
            "apps/desktop/package.json",
            r#"{"name":"app","main":"main.js","devDependencies":{"electron":"42"}}"#,
        ),
        ("apps/desktop/main.js", "require('electron')"),
        (
            "apps/legacy/package.json",
            r#"{"devDependencies":{"electron":"42"}}"#,
        ),
        (
            "apps/unlisted/package.json",
            r#"{"devDependencies":{"electron":"42"}}"#,
        ),
    ]);
    assert_eq!(
        load(fixture.root()).unwrap().root,
        fixture.path("apps/desktop")
    );
}

#[test]
fn npm_and_yarn_workspace_forms_respect_recursive_patterns_and_exclusions() {
    for declaration in [
        r#"["apps/**","!apps/**/legacy"]"#,
        r#"{"packages":["./apps/**","!apps/**/legacy"]}"#,
    ] {
        let manifest = format!(r#"{{"workspaces":{declaration}}}"#);
        let fixture = Fixture::new(&[
            ("package.json", &manifest),
            (
                "apps/team/desktop/package.json",
                r#"{"name":"app","main":"main.js","devDependencies":{"electron-nightly":"42"}}"#,
            ),
            ("apps/team/desktop/main.js", "require('electron')"),
            (
                "apps/library/package.json",
                r#"{"name":"library","main":"index.js"}"#,
            ),
            ("apps/library/index.js", "exports.value=1"),
            (
                "apps/team/legacy/package.json",
                r#"{"dependencies":{"electron":"42"}}"#,
            ),
            (
                "apps/team/desktop/node_modules/other/package.json",
                r#"{"dependencies":{"electron":"42"}}"#,
            ),
            (
                "apps/team/.cache/package.json",
                r#"{"dependencies":{"electron":"42"}}"#,
            ),
            (
                "outside/package.json",
                r#"{"dependencies":{"electron":"42"}}"#,
            ),
        ]);
        assert_eq!(
            load(fixture.root()).unwrap().root,
            fixture.path("apps/team/desktop")
        );
    }
}

#[test]
fn shared_electron_is_used_for_a_plain_app_with_an_electron_start_command() {
    for (runtime, command) in [
        ("electron", "electron"),
        ("electron-nightly", "electron.cmd"),
    ] {
        let manifest = format!(
            r#"{{"workspaces":["apps/*"],"devDependencies":{{"{runtime}":"42","electron-vite":"5"}}}}"#
        );
        let application = format!(r#"{{"main":"main.js","scripts":{{"start":"{command} ."}}}}"#);
        let fixture = Fixture::new(&[
            ("package.json", &manifest),
            ("apps/plain/package.json", &application),
            ("apps/plain/main.js", "require('electron')"),
            ("apps/library/package.json", r#"{"main":"index.js"}"#),
            ("apps/library/index.js", "exports.value=1"),
        ]);
        let project = load(fixture.root()).unwrap();
        assert_eq!(
            project.layout,
            Layout::Plain,
            "shared tooling must not change an app's layout"
        );
        assert_eq!(project.main.source, fixture.path("apps/plain/main.js"));
    }
}

#[test]
fn root_apps_and_explicit_root_entry_errors_take_precedence_over_workspace_members() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"main":"main.js","devDependencies":{"electron":"42"},"workspaces":["apps/*"]}"#,
        ),
        ("main.js", "require('electron')"),
        (
            "apps/child/package.json",
            r#"{"devDependencies":{"electron":"42"}}"#,
        ),
        ("apps/child/index.js", "require('electron')"),
    ]);
    assert_eq!(load(fixture.root()).unwrap().root, fixture.root);
    std::fs::remove_file(fixture.path("main.js")).unwrap();
    assert_eq!(
        load(fixture.root()).unwrap_err(),
        "cannot find the main process entry \"main.js\""
    );
    fixture.write(
        "package.json",
        br#"{"workspaces":["apps/*"],"devDependencies":{"electron":"42"}}"#,
    );
    fixture.write("index.js", b"require('electron')");
    assert_eq!(load(fixture.root()).unwrap().root, fixture.root);
    std::fs::remove_file(fixture.path("index.js")).unwrap();
    fixture.write(
        "electron.vite.config.ts",
        b"export default {main:{build:{rollupOptions:{input:chooseMain()}}}}",
    );
    assert!(
        load(fixture.root())
            .unwrap_err()
            .contains("cannot statically resolve electron-vite build input")
    );
}

#[test]
fn ambiguous_workspaces_and_broken_application_sources_have_specific_diagnostics() {
    let fixture = Fixture::new(&[
        ("package.json", r#"{"workspaces":["apps/*"]}"#),
        (
            "apps/b/package.json",
            r#"{"dependencies":{"electron":"42"}}"#,
        ),
        (
            "apps/a/package.json",
            r#"{"dependencies":{"electron":"42"}}"#,
        ),
    ]);
    assert!(
        load(fixture.root())
            .unwrap_err()
            .contains("multiple Electron applications in workspace: apps/a, apps/b")
    );
    fixture.write("package.json", br#"{"workspaces":["apps/a"]}"#);
    assert_eq!(
        load(fixture.root()).unwrap_err(),
        "cannot find the main process entry \"index.js\""
    );
    fixture.write("apps/a/package.json", b"{");
    assert!(load(fixture.root()).unwrap_err().contains("invalid "));
}

#[test]
fn workspace_configuration_failures_are_reported_and_pnpm_takes_precedence() {
    let fixture = Fixture::new(&[("package.json", r#"{"workspaces":["apps/*"]}"#)]);
    for yaml in ["packages: [", "packages: no", "packages: [42]"] {
        fixture.write("pnpm-workspace.yaml", yaml.as_bytes());
        assert!(load(fixture.root()).is_err());
    }
    fixture.write("pnpm-workspace.yaml", b"allowBuilds: {electron: true}");
    assert!(
        patterns(
            fixture.root(),
            &serde_json::json!({"workspaces":["apps/*"]})
        )
        .unwrap()
        .is_empty()
    );
    std::fs::remove_file(fixture.path("pnpm-workspace.yaml")).unwrap();
    std::fs::create_dir(fixture.path("pnpm-workspace.yaml")).unwrap();
    assert!(load(fixture.root()).unwrap_err().contains("cannot read "));
    std::fs::remove_dir(fixture.path("pnpm-workspace.yaml")).unwrap();
    for workspaces in [serde_json::json!(42), serde_json::json!([42])] {
        assert!(
            patterns(
                fixture.root(),
                &serde_json::json!({"workspaces":workspaces})
            )
            .is_err()
        );
    }
    assert!(
        patterns(fixture.root(), &serde_json::json!({"workspaces":{}}))
            .unwrap()
            .is_empty()
    );
    assert!(!descend(&[], &segments("anywhere")));
    assert!(!descend(&segments("apps/*"), &segments("other")));
}

#[test]
fn yaml_workspace_merge_keys_select_the_declared_application() {
    let fixture = Fixture::new(&[
        ("package.json", "{}"),
        (
            "pnpm-workspace.yaml",
            "defaults: &defaults\n  packages: ['apps/*']\n<<: *defaults\n",
        ),
        (
            "apps/desktop/package.json",
            r#"{"dependencies":{"electron":"42"}}"#,
        ),
        ("apps/desktop/index.js", "require('electron')"),
    ]);
    assert_eq!(
        load(fixture.root()).unwrap().root,
        fixture.path("apps/desktop")
    );
    for yaml in ["<<: 42", "? [complex, key]\n: value"] {
        fixture.write("pnpm-workspace.yaml", yaml.as_bytes());
        assert!(load(fixture.root()).unwrap_err().contains("invalid "));
    }
}
