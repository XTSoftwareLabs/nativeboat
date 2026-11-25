use super::*;

/// A project folder with files, removed when dropped.
struct Project(PathBuf);

impl Project {
    fn new(files: &[(&str, &str)]) -> Project {
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("css-imports-{}-{id}", std::process::id()));
        for (name, text) in files {
            let path = root.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        Project(root)
    }

    fn inline(&self, name: &str) -> (String, Vec<String>) {
        let path = self.0.join(name);
        let text = std::fs::read_to_string(&path).unwrap();
        inline(&path, &text, &self.0, &[])
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The text without blank lines and indentation, for comparisons.
fn compact(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn imports_inline_with_their_layers_conditions_and_media() {
    let project = Project::new(&[
        (
            "src/main.css",
            "@charset \"utf-8\";\n/* theme */\n@layer theme, base;\n@import \"tw\";\n@import './local.css' layer(base) supports(display: grid) screen;\n@import url(\"plain\") layer;\n.after { color: red; }\n@import 'ignored.css';\n",
        ),
        (
            "node_modules/tw/package.json",
            r#"{"name":"tw","exports":{".":{"style":"./index.css","import":"./lib.mjs"}}}"#,
        ),
        (
            "node_modules/tw/index.css",
            "@import './theme.css' layer(theme);\n@tailwind utilities;\n",
        ),
        ("node_modules/tw/theme.css", ":root { --tw: 1; }"),
        ("src/local.css", ".local { color: blue; }"),
        (
            "src/plain.css",
            "@import './main.css';\n.plain { margin: 0; }",
        ),
        ("src/ignored.css", ".ignored {}"),
    ]);
    let (text, errors) = project.inline("src/main.css");
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(
        compact(&text),
        compact(
            "/* theme */\n@layer theme, base;\n\
             @layer theme {\n:root { --tw: 1; }\n}\n@tailwind utilities;\n\
             @layer base {\n@supports (display: grid) {\n@media screen {\n.local { color: blue; }\n}\n}\n}\n\
             @layer {\n.plain { margin: 0; }\n}\n\
             .after { color: red; }\n@import 'ignored.css';\n"
        )
    );
}

#[test]
fn package_style_fields_assets_and_missing_sheets() {
    let project = Project::new(&[
        (
            "main.css",
            "@import '@scope/font';\n@import './missing.css';\n@import 'https://example.com/x.css';\nbody {}",
        ),
        (
            "node_modules/@scope/font/package.json",
            r#"{"name":"@scope/font","style":"font.css","main":"index.js"}"#,
        ),
        (
            "node_modules/@scope/font/font.css",
            "@font-face { src: url(./files/a.woff2); }",
        ),
        ("node_modules/@scope/font/files/a.woff2", "font"),
    ]);
    let (text, errors) = project.inline("main.css");
    assert!(
        text.contains("url(\"data:font/woff2;base64,Zm9udA==\")"),
        "{text}"
    );
    assert!(
        text.contains("@import 'https://example.com/x.css';"),
        "{text}"
    );
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].contains("cannot resolve stylesheet \"./missing.css\""),
        "{errors:?}"
    );
}
