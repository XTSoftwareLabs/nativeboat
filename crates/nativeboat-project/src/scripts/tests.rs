use super::*;
use crate::tests::fixture::Fixture;
use serde_json::json;

fn steps(package: Value) -> Vec<(String, String)> {
    build_steps(Path::new("."), &package, &[])
        .into_iter()
        .map(|step| (step.script, step.command))
        .collect()
}

fn owned(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(script, command)| ((*script).to_owned(), (*command).to_owned()))
        .collect()
}

#[test]
fn the_packaging_script_runs_its_steps_but_compilers() {
    // The layout of an application with a native sidecar.
    let package = json!({"scripts": {
        "build": "tsc --project tsconfig.json && node scripts/copy-assets.mjs",
        "build:sidecar": "node scripts/build-sidecar.mjs --profile release",
        "build:sidecar:dev": "node scripts/build-sidecar.mjs --profile debug",
        "package:win": "npm run build:sidecar && npm run build && electron-builder --win portable --x64",
        "start": "npm run build:sidecar:dev && npm run build && electron ."
    }});
    assert_eq!(
        steps(package),
        owned(&[
            (
                "build:sidecar",
                "node scripts/build-sidecar.mjs --profile release"
            ),
            ("build", "node scripts/copy-assets.mjs"),
        ])
    );
}

#[test]
fn electron_abi_rebuilds_are_excluded_while_resources_and_native_sidecars_run() {
    let package = json!({"scripts":{
        "prepackage":"electron-builder install-app-deps && node prepare-resources.mjs",
        "rebuild":"electron-rebuild -f",
        "package":"npm run rebuild && npx --yes @electron/rebuild@4.0.0 && pnpm exec electron-builder node-gyp-rebuild && node-gyp rebuild && cargo build --release && tsx minify.ts && electron-vite build && electron-builder --win"
    }});
    assert_eq!(
        steps(package),
        owned(&[
            ("prepackage", "node prepare-resources.mjs"),
            ("package", "node-gyp rebuild"),
            ("package", "cargo build --release"),
            ("package", "tsx minify.ts"),
        ])
    );
    for command in [
        "electron-rebuild",
        "./node_modules/.bin/electron-rebuild.cmd -f",
        "npx @electron/rebuild",
        "pnpm dlx @electron/rebuild",
        "yarn exec electron-rebuild",
        "cross-env MODE=build electron-rebuild",
        "electron-builder install-app-deps",
        "npm exec electron-builder node-gyp-rebuild",
    ] {
        assert!(is_electron_rebuild(command), "{command}");
    }
    for command in [
        "",
        "npx --yes",
        "node-gyp rebuild",
        "cargo build",
        "rebuild",
        "electron-builder --win",
        "electron-builder create-self-signed-cert",
        "node scripts/rebuild.mjs",
        "node scripts/electron-rebuild.js",
    ] {
        assert!(!is_electron_rebuild(command), "{command}");
    }
}

#[test]
fn package_managers_hooks_and_arguments() {
    let package = json!({"scripts": {
        "prebuild": "node gen.js",
        "build": "cross-env NODE_ENV=production webpack --mode production; npx --yes tsc -p .",
        "postbuild": "NODE_ENV=x vite build && node post.js",
        "icons": "node icons.js",
        "native": "cargo build",
        "dist": "yarn build && pnpm run icons && bun run native -- --release && npm run -s missing && yarn && npm exec electron-vite build && electron-builder",
        "after": "echo never"
    }});
    assert_eq!(
        steps(package),
        owned(&[
            ("prebuild", "node gen.js"),
            ("postbuild", "node post.js"),
            ("icons", "node icons.js"),
            ("native", "cargo build --release"),
            ("dist", "npm run -s missing"),
            ("dist", "yarn"),
        ])
    );
}

#[test]
fn windows_packaging_is_preferred() {
    let mac = ("dist:mac", "node mac.js && electron-builder --mac");
    let plain = ("dist", "node plain.js && electron-builder");
    let windows = (
        "make",
        "node win.js && electron-forge make --platform=win32",
    );
    let scripts = |entries: &[(&str, &str)]| {
        let map: Map<String, Value> = entries
            .iter()
            .map(|(name, command)| ((*name).to_owned(), Value::from(*command)))
            .collect();
        json!({ "scripts": map })
    };
    let first = |entries: &[(&str, &str)]| steps(scripts(entries))[0].1.clone();
    assert_eq!(first(&[mac, plain, windows]), "node win.js");
    assert_eq!(first(&[mac, plain]), "node plain.js");
    assert_eq!(first(&[mac]), "node mac.js");
    assert_eq!(
        first(&[("all", "node w.js && electron-builder -w"), plain]),
        "node w.js"
    );
    assert_eq!(
        first(&[
            ("pack", "node p.js && electron-packager . --platform linux"),
            plain
        ]),
        "node plain.js"
    );
    assert_eq!(
        first(&[("darwin", "node d.js && electron-packager ."), plain]),
        "node plain.js"
    );
}

#[test]
fn only_packaging_commands_count() {
    let package = json!({"scripts": {
        "postinstall": "electron-builder install-app-deps",
        "forge": "electron-forge start",
        "quoted": "echo \"a && b\" && echo 'c; d'",
        "loop": "npm run loop",
        "twice": "npm run once && npm run once && electron-forge package",
        "once": "node once.js",
        "number": 3
    }});
    assert_eq!(steps(package), owned(&[("once", "node once.js")]));
    let quoted = json!({"scripts": {"dist": "echo \"a && b\" && echo 'c; d' && electron-builder"}});
    assert_eq!(
        steps(quoted),
        owned(&[("dist", "echo \"a && b\""), ("dist", "echo 'c; d'")])
    );
    // Scripts that run themselves stop.
    let looping = json!({"scripts": {"dist": "npm run dist && electron-builder"}});
    assert!(steps(looping).is_empty());
    // A `pre` script can package as well.
    let hooked =
        json!({"scripts": {"prepack": "node a.js && electron-builder", "pack": "node b.js"}});
    assert_eq!(steps(hooked), owned(&[("prepack", "node a.js")]));
    assert!(steps(json!({})).is_empty());
    assert!(steps(json!({"scripts": {"build": "node x.js"}})).is_empty());
}

#[test]
fn without_packaging_scripts_producers_of_missing_resources_run() {
    let fixture = Fixture::new(&[
        ("scripts/make.mjs", "await build('core_server.exe');"),
        ("scripts/other.mjs", "console.log('x');"),
    ]);
    let package = json!({"scripts": {
        "build:core": "node scripts/make.mjs --release",
        "build:core:dev": "node scripts/make.mjs --debug",
        "sidecars": "echo sidecars",
        "other": "node scripts/other.mjs",
        "number": 1
    }});
    let found = build_steps(
        fixture.root(),
        &package,
        &[
            "sidecars".to_owned(),
            "core_server.exe".to_owned(),
            "x".to_owned(),
        ],
    );
    let commands: Vec<&str> = found.iter().map(|step| step.command.as_str()).collect();
    assert_eq!(
        commands,
        vec!["node scripts/make.mjs --release", "echo sidecars"]
    );
    assert!(build_steps(fixture.root(), &package, &[]).is_empty());
}

/// Without a packaging script, a task a build script runs through a task
/// runner that is named after a file a stylesheet names but that does not
/// exist runs alone, as VS Code's `copy-codicons` copies its icon font.
/// Other scripts that mention such files, as for test stylesheets, are not
/// run for them.
#[test]
fn task_runner_tasks_named_after_missing_stylesheet_assets_run_alone() {
    let fixture = Fixture::new(&[
        (
            "src/icons/icons.css",
            "/* url(old.ttf) */ @font-face { src: url(\"./codicon.ttf?v=1\") format('truetype'); }\n\
             .a { background: url(data:image/png;base64,AA), url('#x'), url(/root.png), url(present.png) } /* open",
        ),
        ("src/icons/present.png", ""),
        (
            "test/fixture.css",
            "a { background: url(blossoms.jpg) } b { background: url(open",
        ),
        (
            "node_modules/pkg/style.css",
            "a { background: url(gone.png) }",
        ),
        (".hidden/style.css", "a { background: url(hidden.png) }"),
        ("build/fast.mjs", "run(['copy-codicons', 'blossoms'])"),
    ]);
    assert_eq!(
        missing_stylesheet_assets(fixture.root()),
        vec!["test/blossoms.jpg", "src/icons/codicon.ttf"]
    );
    let package = json!({"scripts": {
        "gulp": "node ./node_modules/gulp/bin/gulp.js",
        "build-assets": "npm run gulp copy-codicons compile-extensions --series",
        "copy-again": "gulp copy-codicons other",
        "watch-assets": "npm run gulp watch-codicons",
        "fast": "node build/fast.mjs",
        "compile": "npm run gulp compile"
    }});
    let found = build_steps(fixture.root(), &package, &[]);
    let commands: Vec<&str> = found.iter().map(|step| step.command.as_str()).collect();
    assert_eq!(
        commands,
        vec![
            "node ./node_modules/gulp/bin/gulp.js copy-codicons --series",
            "gulp copy-codicons"
        ]
    );
    assert_eq!(found[0].script, "gulp");
    fixture.write("src/icons/codicon.ttf", b"");
    fixture.write("test/blossoms.jpg", b"");
    assert!(build_steps(fixture.root(), &package, &[]).is_empty());
}

#[test]
fn program_names() {
    assert_eq!(name("./node_modules/.bin/tsc.cmd"), "tsc");
    assert_eq!(name("Vite.EXE"), "vite.exe");
    assert_eq!(name("webpack.js"), "webpack");
    assert_eq!(name("v1.2"), "v1.2");
    assert_eq!(name(""), "");
    assert!(launched(&[]).is_empty());
    assert!(!is_packager(&[]));
    assert!(invoked(&[], &Map::new()).is_none());
}

#[test]
fn configured_plugin_compilers_keep_the_projects_resource_sequence() {
    let package = serde_json::json!({"scripts": {
        "build:win": "node generate.js && electron-rebuild && electron-vite build && tsc && electron-builder --win"
    }});
    let found = build_steps_with_compiler(Path::new("."), &package, &[], Some("electron-vite"));
    assert_eq!(
        found
            .iter()
            .map(|step| step.command.as_str())
            .collect::<Vec<_>>(),
        ["node generate.js", "electron-vite build"]
    );
    assert!(uses_tool("npx electron-vite build", "electron-vite"));
    assert!(uses_tool(
        "./node_modules/.bin/electron-vite.cmd build",
        "electron-vite"
    ));
    assert!(!uses_tool("", "electron-vite"));
    assert!(!uses_tool("node generate.js", "electron-vite"));
    let package =
        serde_json::json!({"scripts":{"build":"node generate.js && electron-vite build"}});
    assert_eq!(
        build_steps_with_compiler(Path::new("."), &package, &[], Some("electron-vite")).len(),
        2
    );
    for package in [
        serde_json::json!({}),
        serde_json::json!({"scripts":{}}),
        serde_json::json!({"scripts":{"build":"node generate.js"}}),
    ] {
        let found = build_steps_with_compiler(Path::new("."), &package, &[], Some("electron-vite"));
        assert_eq!(found.last().unwrap().command, "electron-vite build");
    }
}
