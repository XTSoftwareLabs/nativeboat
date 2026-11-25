use super::*;
use crate::tests::fixture::Fixture;
use serde_json::json;

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn installed_and_provided_packages_are_not_needed() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"dependencies":{"left-pad":"1","ced":"2","keytar":"7","fs":"0","electron-nightly":"1","opt":"1"},
                "optionalDependencies":{"opt":"1","other":"1"},
                "devDependencies":{"electron":"30","tool":"1"}}"#,
        ),
        ("package-lock.json", "{}"),
        ("node_modules/left-pad/package.json", "{}"),
        // A directory without a manifest is not an installed package.
        ("node_modules/tool/index.js", ""),
    ]);
    assert_eq!(plan(fixture.root(), false), None);
    assert_eq!(
        plan(fixture.root(), true),
        Some(Install {
            root: fixture.root().to_path_buf(),
            manager: Manager::Npm,
            version: None,
            missing: names(&["tool"]),
        })
    );
    // Without a manifest there is nothing to install.
    assert_eq!(plan(&fixture.path("node_modules"), true), None);
}

#[test]
fn workspace_members_install_from_the_workspace() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"packageManager":"pnpm@10.33.4+sha512.abc","devDependencies":{"tsx":"4"}}"#,
        ),
        ("pnpm-workspace.yaml", "packages: ['packages/*']"),
        ("pnpm-lock.yaml", ""),
        (
            "packages/app/package.json",
            r#"{"dependencies":{"vue":"3"},"devDependencies":{"electron-vite":"5"}}"#,
        ),
        // Hoisted into the workspace.
        ("node_modules/vue/package.json", "{}"),
    ]);
    let app = fixture.path("packages/app");
    assert_eq!(plan(&app, false), None);
    assert_eq!(
        plan(&app, true),
        Some(Install {
            root: fixture.root().to_path_buf(),
            manager: Manager::Pnpm,
            version: Some("10.33.4".to_owned()),
            missing: names(&["electron-vite", "tsx"]),
        })
    );
    // A member with its own lockfile installs by itself.
    fixture.write("packages/app/package-lock.json", b"{}");
    let install = plan(&app, true).unwrap();
    assert_eq!(install.root, app);
    assert_eq!(install.manager, Manager::Npm);
    assert_eq!(install.missing, names(&["electron-vite"]));
}

#[test]
fn install_roots_stay_inside_the_repository() {
    let fixture = Fixture::new(&[
        ("package-lock.json", "{}"),
        ("repo/.git/HEAD", ""),
        (
            "repo/package.json",
            r#"{"workspaces":{"packages":["app"]}}"#,
        ),
        ("repo/app/package.json", r#"{"dependencies":{"a":"1"}}"#),
        ("other/.git", ""),
        ("other/app/package.json", r#"{"dependencies":{"a":"1"}}"#),
        ("yarn/.git", ""),
        ("yarn/pnpm-workspace.yaml", ""),
        ("yarn/nested/package.json", r#"{"workspaces":["*"]}"#),
        (
            "yarn/nested/app/package.json",
            r#"{"dependencies":{"a":"1"}}"#,
        ),
        ("stray/app/package.json", r#"{"dependencies":{"a":"1"}}"#),
    ]);
    let install = plan(&fixture.path("repo/app"), false).unwrap();
    assert_eq!(install.root, fixture.path("repo"));
    assert_eq!(install.manager, Manager::Npm);
    let install = plan(&fixture.path("other/app"), false).unwrap();
    assert_eq!(install.root, fixture.path("other/app"));
    // The nearest workspace declaration wins.
    let install = plan(&fixture.path("yarn/nested/app"), false).unwrap();
    assert_eq!(install.root, fixture.path("yarn/nested"));
    assert_eq!(install.manager, Manager::Npm);
    // A lockfile above an application that is no workspace member is not its own.
    let install = plan(&fixture.path("stray/app"), false).unwrap();
    assert_eq!(install.root, fixture.path("stray/app"));
}

#[test]
fn package_manager_fields_and_lockfiles_select_the_manager() {
    let fixture = Fixture::new(&[("berry/.yarnrc.yml", ""), ("plain/x", "")]);
    let berry = fixture.path("berry");
    let plain = fixture.path("plain");
    let pinned = |root: &Path, field: &str| manager(root, &json!({ "packageManager": field }));
    assert_eq!(
        pinned(&plain, "yarn@1.22.19"),
        (Manager::Yarn, Some("1.22.19".to_owned()))
    );
    assert_eq!(pinned(&plain, "yarn@4.1.0").0, Manager::YarnBerry);
    assert_eq!(pinned(&berry, "yarn@next").0, Manager::YarnBerry);
    assert_eq!(pinned(&plain, "bun@1.1.0").0, Manager::Bun);
    assert_eq!(pinned(&plain, "npm@10.0.0").0, Manager::Npm);
    assert_eq!(pinned(&plain, "pnpm@9.0.0").0, Manager::Pnpm);
    // Unknown managers fall back to the lockfiles.
    assert_eq!(pinned(&plain, "deno@2"), (Manager::Npm, None));
    assert_eq!(pinned(&plain, "none"), (Manager::Npm, None));
    for (lockfile, expected) in [
        ("yarn.lock", Manager::Yarn),
        ("bun.lockb", Manager::Bun),
        ("bun.lock", Manager::Bun),
        ("npm-shrinkwrap.json", Manager::Npm),
        ("pnpm-workspace.yaml", Manager::Pnpm),
    ] {
        let directory = lockfile.replace('.', "-");
        fixture.write(&format!("{directory}/{lockfile}"), b"");
        assert_eq!(
            manager(&fixture.path(&directory), &Value::Null),
            (expected, None),
            "{lockfile}"
        );
    }
    fixture.write("berry/yarn.lock", b"");
    assert_eq!(manager(&berry, &Value::Null), (Manager::YarnBerry, None));
}

#[test]
fn managers_name_their_programs_and_packages() {
    let all = [
        Manager::Npm,
        Manager::Pnpm,
        Manager::Yarn,
        Manager::YarnBerry,
        Manager::Bun,
    ];
    assert_eq!(
        all.map(Manager::program),
        ["npm", "pnpm", "yarn", "yarn", "bun"]
    );
    assert_eq!(
        all.map(Manager::package),
        ["npm", "pnpm", "yarn", "@yarnpkg/cli-dist", "bun"]
    );
}
