use super::*;

#[test]
fn prepared_paths_preserve_relative_names_and_directory_boundaries() {
    let mapping = PathMapping {
        source_root: PathBuf::from("prepared/ui"),
        runtime_root: "out/renderer".into(),
    };
    assert_eq!(
        mapping.map(Path::new("prepared/ui/assets/icon.svg")),
        Some("out/renderer/assets/icon.svg".into())
    );
    assert_eq!(
        mapping.map(Path::new("prepared/ui")),
        Some("out/renderer/".into())
    );
    assert_eq!(mapping.map(Path::new("prepared/ui-other/icon.svg")), None);
    assert_eq!(mapping.map(Path::new("app/main.js")), None);
}

fn emit(root_dir: &str, out_dir: &str) -> Emit {
    Emit {
        root_dir: root_dir.to_owned(),
        out_dir: out_dir.to_owned(),
    }
}

#[test]
fn sources_map_to_emitted_files() {
    let layout = emit("", "dist");
    assert_eq!(
        layout.output("src/main/main.ts").as_deref(),
        Some("dist/src/main/main.js")
    );
    assert_eq!(
        layout.output("src/preload/preload.cts").as_deref(),
        Some("dist/src/preload/preload.cjs")
    );
    assert_eq!(
        layout.output("src/worker.mts").as_deref(),
        Some("dist/src/worker.mjs")
    );
    assert_eq!(
        layout.output("src/view.tsx").as_deref(),
        Some("dist/src/view.js")
    );
    assert_eq!(layout.output("src/types.d.ts"), None);
    assert_eq!(layout.output("src/index.html"), None);
    assert_eq!(layout.output("README"), None);
    let nested = emit("src", "build");
    assert_eq!(nested.output("src/a/b.js").as_deref(), Some("build/a/b.js"));
    assert_eq!(nested.output("other/b.ts"), None);
    assert_eq!(nested.output("srcx/b.ts"), None);
    // Without an output directory, files are emitted beside their sources.
    let beside = emit("src", "src");
    assert_eq!(beside.output("src/main.ts").as_deref(), Some("src/main.js"));
}

#[test]
fn emitted_files_map_to_sources() {
    let layout = emit("", "dist");
    assert_eq!(
        layout.sources("dist/src/main/main.js"),
        [
            "src/main/main.ts",
            "src/main/main.tsx",
            "src/main/main.js",
            "src/main/main.jsx"
        ]
    );
    assert_eq!(
        layout.sources("dist/src/preload/preload.cjs"),
        ["src/preload/preload.cts", "src/preload/preload.cjs"]
    );
    assert_eq!(layout.sources("dist/a.mjs"), ["a.mts", "a.mjs"]);
    assert_eq!(layout.sources("dist/a.jsx"), ["a.tsx", "a.jsx"]);
    // Copied files and extensionless paths keep their place.
    assert_eq!(
        layout.sources("dist/src/renderer/index.html"),
        ["src/renderer/index.html"]
    );
    assert_eq!(layout.sources("dist/v1.2/main"), ["v1.2/main"]);
    assert!(layout.sources("out/main.js").is_empty());
    let nested = emit("src", "build");
    assert_eq!(nested.sources("build"), ["src"]);
    assert_eq!(nested.sources("build/x.css"), ["src/x.css"]);
}
