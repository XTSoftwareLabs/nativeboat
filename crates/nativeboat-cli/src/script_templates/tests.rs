use super::*;

fn quasis(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

/// A template that builds a script, as VS Code's webview host builds the one
/// that defines `acquireVsCodeApi`: substitutions in strings become string
/// parts, and those standing for expressions are evaluated.
#[test]
fn scripts_compile_with_their_substitutions() {
    let body = script_body(&quasis(&[
        "\n globalThis.acquireApi = (function() {\n  const post = window.parent['",
        "'].bind(window.parent);\n  let state = ",
        ";\n  return () => ({ post, multiple: !",
        ", state });\n })();\n",
    ]))
    .unwrap();
    assert!(
        body.contains("window.parent[(\"\" + String(__nbHoles[0]) + \"\")]"),
        "{body}"
    );
    assert!(
        body.contains("let state = __nbTemplateHole(__nbHoles[1]);"),
        "{body}"
    );
    assert!(
        body.contains("multiple: !__nbTemplateHole(__nbHoles[2])"),
        "{body}"
    );
}

/// Templates of other text, short ones, and scripts whose substitutions are
/// names or keys are not script templates.
#[test]
fn other_templates_are_not_scripts() {
    for parts in [
        &[
            "<div class=\"",
            "\">hello world, how are things going today</div>",
        ][..],
        &["", "px"],
        &["const ", " = call(1);\nconsole.log('a value long enough');"],
        &[
            "const value = object.",
            ";\nconsole.log('a value long enough to count');",
        ],
        &[
            "// a comment about ",
            "\nconsole.log('a value long enough to count');",
        ],
    ] {
        assert_eq!(script_body(&quasis(parts)), None, "{parts:?}");
    }
}

/// Sources are scanned while the renderer graphs compile, and each
/// template becomes a module that every graph can load by its key.
#[test]
fn scanned_templates_become_modules() {
    let root = Path::new("C:/app");
    scanning(true);
    let before = count();
    // Only sources that make script elements are scanned.
    scan(
        Path::new("C:/app/style.js"),
        "export const make = (name) => `\\n window[\"${name}\"] = function () { return 0; };\\n console.log('styled');\\n`;",
    );
    assert_eq!(count(), before);
    scan(
        Path::new("C:/app/page.js"),
        "export const make = (name) => `\\n window[\"${name}\"] = function () { return 1; };\\n console.log('defined');\\n`;\nexport const element = () => document.createElement('script');",
    );
    scanning(false);
    assert_eq!(count(), before + 1);
    let (key, path) = named_scripts(root).pop().unwrap();
    assert!(key.starts_with("nativeboat-script-template-"));
    let module = source(&path).unwrap();
    assert!(
        module
            .text
            .starts_with("export default function (__nbHoles) {")
    );
    assert!(registration().contains("register_script_templates"));
    // Without scanning, sources are not read for templates.
    scan(
        Path::new("C:/app/other.js"),
        "export const other = (n) => `\\n window[\"${n}\"] = function () { return 2; };\\n console.log('other');\\n`;",
    );
    assert_eq!(count(), before + 1);
}
