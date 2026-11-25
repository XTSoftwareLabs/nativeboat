use super::*;

#[test]
#[ignore = "requires an unchanged MarkText checkout; set NATIVEBOAT_MARKTEXT_PROJECT"]
fn unchanged_marktext_workspace_discovers_packaging_and_sources() {
    let root = PathBuf::from(std::env::var_os("NATIVEBOAT_MARKTEXT_PROJECT").unwrap());
    let desktop = root.join("packages/desktop");
    let project = load(&root).unwrap();
    let direct = load(&desktop).unwrap();
    assert_eq!(project.root, desktop);
    assert_eq!(project.metadata, direct.metadata);
    assert_eq!(project.main, direct.main);
    assert_eq!(project.preloads, direct.preloads);
    assert_eq!(project.pages, direct.pages);
    assert_eq!(project.extra_files, direct.extra_files);
    assert_eq!(project.metadata.name, "marktext");
    assert_eq!(project.metadata.product_name, "MarkText");
    assert_eq!(project.metadata.app_name, "marktext");
    assert_eq!(
        project.metadata.app_id.as_deref(),
        Some("com.github.marktext.marktext")
    );
    assert_eq!(project.metadata.icon, Some(desktop.join("static/icon.ico")));
    assert_eq!(project.output_directory, "../../dist");
    assert_eq!(project.defines, direct.defines);
    assert_eq!(project.renderer_defines, direct.renderer_defines);
    assert_eq!(project.aliases, direct.aliases);
    assert_eq!(project.preload_aliases, direct.preload_aliases);
    assert_eq!(project.renderer_aliases, direct.renderer_aliases);
    for aliases in [
        &project.aliases,
        &project.preload_aliases,
        &project.renderer_aliases,
    ] {
        for (pattern, path) in [
            ("@", "src/renderer/src"),
            ("common", "src/common"),
            ("@shared", "src/shared"),
        ] {
            let alias = aliases
                .iter()
                .find(|alias| alias.pattern == pattern)
                .unwrap();
            assert_eq!(Path::new(&alias.target), desktop.join(path));
            assert_eq!(alias.kind, nativeboat_bundler::AliasKind::Path);
        }
    }
    assert!(
        !project
            .aliases
            .iter()
            .chain(&project.preload_aliases)
            .any(|alias| alias.pattern == "path")
    );
    let path = project
        .renderer_aliases
        .iter()
        .find(|alias| alias.pattern == "path")
        .unwrap();
    assert_eq!(path.target, "pathe");
    assert_eq!(path.kind, nativeboat_bundler::AliasKind::Specifier);
    assert!(
        project
            .defines
            .contains(&("MARKTEXT_VERSION".to_owned(), "\"0.20.0\"".to_owned(),))
    );
    assert!(project.defines.contains(&(
        "MARKTEXT_VERSION_STRING".to_owned(),
        "\"v0.20.0\"".to_owned(),
    )));
    assert_eq!(
        project.renderer_defines,
        [("global".to_owned(), "globalThis".to_owned())]
    );
    assert!(project.preload_defines.is_empty());
    assert_eq!(project.build_steps, direct.build_steps);
    assert_eq!(
        project.build_steps,
        [
            scripts::Step {
                script: "build:win".to_owned(),
                command: "tsx ../../scripts/minify-locales.ts".to_owned(),
                optional: false
            },
            scripts::Step {
                script: "build:win".to_owned(),
                command: "electron-vite build".to_owned(),
                optional: false
            }
        ]
    );
    assert_eq!(project.main.source, desktop.join("src/main/index.ts"));
    assert_eq!(project.main.runtime_paths, ["out/main/index.js"]);
    assert_eq!(
        project.preloads[0].source,
        desktop.join("src/preload/index.ts")
    );
    assert_eq!(
        project.pages[0].source,
        desktop.join("src/renderer/index.html")
    );
    for (source, destination) in [
        ("build/icons/icon.ico", "resources/icons/icon.ico"),
        ("build/icons/md.ico", "resources/icons/md.ico"),
        ("static/icon.ico", "resources/static/icon.ico"),
        ("static/preference.json", "resources/static/preference.json"),
    ] {
        assert!(project.extra_files.contains(&builder::ExtraFile {
            source: desktop.join(source),
            destination: destination.to_owned(),
        }));
    }
    assert!(project.extra_files.iter().all(|file| {
        !file.destination.starts_with("resources/static/locales/")
            || file.destination.ends_with(".min.json")
    }));
    println!(
        "MarkText: workspace entry, metadata, icon, output directory and {} resource files discovered without project scripts",
        project.extra_files.len()
    );
}
