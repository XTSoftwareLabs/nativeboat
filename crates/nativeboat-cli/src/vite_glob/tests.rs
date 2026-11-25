use super::*;
use std::path::PathBuf;

/// A temporary project with files, removed when dropped.
struct Project(PathBuf);

impl Project {
    fn new(files: &[&str]) -> Project {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "nativeboat-glob-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        for file in files {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "export default 1;").unwrap();
        }
        Project(root)
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn lazy_globs_become_loaders_keyed_by_relative_path() {
    let project = Project::new(&[
        "src/modes/a.js",
        "src/modes/nested/b.js",
        "src/modes/c.ts",
        "src/modes/node_modules/skip.js",
        "node_modules/lib/mode/x/x.js",
        "node_modules/lib/mode/x/test.js",
    ]);
    let module = project.0.join("src/main.ts");
    let source = "const modes = import.meta.glob('./modes/**/*.js');\nconst libs = import.meta.glob(\n  ['../node_modules/lib/mode/**/*.js', '!**/test.js'],\n  { import: 'default' }\n) as Record<string, unknown>;\nconst other = import.meta.glob(someVariable);\n";
    let expanded = expand(&module, &project.0, source);
    assert_eq!(
        expanded.lines().next().unwrap(),
        "const modes = Object.assign(Object.create(null), {\"./modes/a.js\": () => import(\"./modes/a.js\"), \"./modes/nested/b.js\": () => import(\"./modes/nested/b.js\")});"
    );
    assert!(expanded.contains(
        "{\"../node_modules/lib/mode/x/x.js\": () => import(\"../node_modules/lib/mode/x/x.js\").then((module) => module[\"default\"])\n\n\n}) as Record<string, unknown>;"
    ));
    // Lines are kept, and calls without literal patterns stay.
    assert_eq!(expanded.lines().count(), source.lines().count());
    assert!(expanded.contains("import.meta.glob(someVariable)"));
}

#[test]
fn eager_and_absolute_globs_import_statically() {
    let project = Project::new(&[
        "src/locales/en.json",
        "src/locales/zh-CN.json",
        "src/locales/readme.md",
    ]);
    let module = project.0.join("src/i18n/index.ts");
    let source = "const messages = import.meta.glob('/src/locales/*.{json,yaml}', { eager: true, query: '?raw', import: 'default' });\nexport const all = import.meta.glob(`/src/locales/[a-z][a-z].json`, { eager: true });\n";
    let expanded = expand(&module, &project.0, source);
    assert!(expanded.starts_with(
        "import { default as __nativeboat_glob_0_0 } from \"../locales/en.json?raw\";\n"
    ));
    assert!(expanded.contains("import * as __nativeboat_glob_1_0 from \"../locales/en.json\";\n"));
    assert!(expanded.contains("\"/src/locales/zh-CN.json\": __nativeboat_glob_0_1"));
    assert!(expanded.contains("{\"/src/locales/en.json\": __nativeboat_glob_1_0}"));
    assert!(!expanded.contains("readme"));
}

#[test]
fn sources_without_literal_globs_are_unchanged() {
    let path = Path::new("C:/app/src/main.js");
    assert_eq!(
        expand(path, Path::new("C:/app"), "const a = 1;"),
        "const a = 1;"
    );
    let dynamic = "const files = import.meta.glob(pattern);";
    assert_eq!(expand(path, Path::new("C:/app"), dynamic), dynamic);
    let broken = "import.meta.glob('./*.js' +";
    assert_eq!(expand(path, Path::new("C:/app"), broken), broken);
    let unknown = Path::new("C:/app/src/main.unknown");
    assert_eq!(
        expand(unknown, Path::new("C:/app"), "import.meta.glob('./*.js')"),
        "import.meta.glob('./*.js')"
    );
    let spread = "import.meta.glob('./*.js', { ...options });";
    assert_eq!(expand(path, Path::new("C:/app"), spread), spread);
}

#[test]
fn patterns_match_segments_braces_and_classes() {
    assert_eq!(braces("a{b,c{d,e}}f"), ["abf", "acdf", "acef"]);
    assert_eq!(braces("a{b"), ["a{b"]);
    let parts = |pattern: &str| segments(pattern);
    assert!(matches_segments(&parts("**/*.js"), &parts("a/b/c.js")));
    assert!(matches_segments(&parts("**/*.js"), &parts("c.js")));
    assert!(!matches_segments(&parts("a/*.js"), &parts("a/b/c.js")));
    assert!(wildcard("[a-c]?.[!t]s", "bx.js"));
    assert!(!wildcard("[^a-c]?.js", "bx.js"));
    assert!(wildcard("[ab", "[ab"));
    assert!(!wildcard("[ab]", ""));
    assert!(!wildcard("?", ""));
    assert_eq!(
        lexical_relative(
            Path::new("C:/app/src/i18n"),
            Path::new("C:/app/src/locales/en.json")
        ),
        "../locales/en.json"
    );
    assert_eq!(
        lexical_relative(
            Path::new("C:/app/src"),
            Path::new("C:/app/src/./locales/en.json")
        ),
        "./locales/en.json"
    );
}
