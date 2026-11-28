use super::*;

#[test]
fn top_level_bindings_live_on_the_global_object() {
    let script = globalize("var a = 1, b; function f() { return a; } a++; let o = { a };");
    assert_eq!(
        script,
        "\nglobalThis.f = f;\n(globalThis.a = 1, globalThis.b); function f() { return globalThis.a; } globalThis.a++; (globalThis.o = { a: globalThis.a });"
    );
}

#[test]
fn patterns_loops_classes_and_this() {
    assert_eq!(
        globalize("let {x, y: [z = 2]} = o;"),
        "({x: globalThis.x, y: [globalThis.z = 2]} = o);"
    );
    assert_eq!(
        globalize("for (var i = 0; i < 3; i++) {} for (var k in o) {} for (const v of l) use(v);"),
        "for ((globalThis.i = 0); globalThis.i < 3; globalThis.i++) {} for (globalThis.k in o) {} for (const v of l) use(v);"
    );
    assert_eq!(
        globalize(
            "class C { m() { return this; } } new C(); (function (root) { root.x = 1; })(this);"
        ),
        "class C { m() { return this; } }\nglobalThis.C = C; new globalThis.C(); (function (root) { root.x = 1; })(globalThis);"
    );
    assert_eq!(
        globalize("var a = 1; function g(a) { const b = a; return b; } ({ a } = o);"),
        "\nglobalThis.g = g;\n(globalThis.a = 1); function g(a) { const b = a; return b; } ({ a: globalThis.a } = o);"
    );
    // Directives stay first.
    assert_eq!(
        globalize("'use strict';\nfunction f() {}"),
        "'use strict';\nglobalThis.f = f;\n\nfunction f() {}"
    );
    assert_eq!(globalize("var = ;"), "var = ;");
}

#[test]
fn inline_and_classic_sources() {
    let page = std::env::temp_dir()
        .join("nativeboat-classic-test")
        .join("index.html");
    let inline = inline(&page, 0, "var shared = 1;");
    assert!(inline.ends_with(".index.inline-script-0.js"));
    assert_eq!(source(&key(&inline)), Some("var shared = 1;".to_owned()));
    mark_classic(&inline);
    assert_eq!(
        source(&key(&inline)),
        Some("(globalThis.shared = 1);".to_owned())
    );
    assert_eq!(source(&page.with_file_name("missing.js")), None);
}

#[test]
fn files_run_as_scripts_are_modules_of_their_own() {
    let folder = std::env::temp_dir().join("nativeboat-classic-script-test");
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("lib.min.js");
    std::fs::write(&file, "var count = 1;").unwrap();
    let script = script(&file);
    assert!(script.ends_with(".lib.min.classic.js"));
    assert_eq!(source(&script), Some("(globalThis.count = 1);".to_owned()));
    // Modules importing the file get it as written.
    assert_eq!(source(&key(&file)), None);
}

#[test]
fn handler_attributes_compile_into_a_module() {
    let page = std::env::temp_dir()
        .join("nativeboat-handlers-test")
        .join("index.html");
    assert_eq!(handlers(&page, "<p>no handlers</p>", &[]), None);
    let path = handlers(
        &page,
        "<button onclick=\"go(1); return false\">a</button><b onclick=\"go(1); return false\" onmouseover=\"((\">b</b><i title=x>c</i>",
        &[],
    )
    .unwrap();
    assert!(path.ends_with(".index.event-handlers.js"));
    let text = source(&key(&path)).unwrap();
    // Each text once; one that does not parse is left out.
    assert_eq!(text.matches("function (event)").count(), 1);
    assert!(text.contains(
        "[\"onclick\\u0000go(1); return false\", function (event) {\ngo(1); return false\n}]"
    ));
    assert!(!text.contains("onmouseover"));
    assert!(text.contains("if (handler && name in element) element[name] = handler;"));
}

#[test]
fn commonjs_names_are_globals_in_classic_scripts() {
    // A universal module bundle sees no `module` or `exports` and sets its
    // global, as in a browser.
    assert_eq!(
        globalize(
            "typeof exports == 'object' && typeof module < 'u' ? module.exports = f() : g();"
        ),
        "typeof globalThis.exports == 'object' && typeof globalThis.module < 'u' ? globalThis.module.exports = f() : g();"
    );
    assert_eq!(
        globalize("x = { exports };"),
        "x = { exports: globalThis.exports };"
    );
    assert_eq!(
        globalize("(function (module) { return module; })();"),
        "(function (module) { return module; })();"
    );
}

#[test]
fn handler_attributes_of_markup_scripts_build_compile_too() {
    let directory = std::env::temp_dir().join("nativeboat-script-handlers-test");
    std::fs::create_dir_all(directory.join("lib")).unwrap();
    let entry = directory.join("renderer.js");
    std::fs::write(
        &entry,
        "import { card } from './lib/card.js';\nimport 'electron';\nconst image = (src) => `<img src=\"${src}\" alt=\"x\" onload=\"this.parentElement.classList.add('loaded')\" onerror=\"this.onerror=null;this.src='../thumb.png';\">`;\nconst dynamic = (id) => `<b onclick=\"go(${id})\">`;\n",
    )
    .unwrap();
    std::fs::write(
        directory.join("lib").join("card.js"),
        "export const card = '<div ONCLICK = \"open(&quot;card&quot;)\" title=\"onclick\">' + \"<p>con=\\\"x\\\"</p>\";\n",
    )
    .unwrap();
    let mut found = script_handlers(std::slice::from_ref(&entry));
    found.sort();
    assert_eq!(
        found,
        [
            ("onclick".to_owned(), "open(\"card\")".to_owned()),
            (
                "onerror".to_owned(),
                "this.onerror=null;this.src='../thumb.png';".to_owned()
            ),
            (
                "onload".to_owned(),
                "this.parentElement.classList.add('loaded')".to_owned()
            ),
        ]
    );
    // The page's module binds them, and keeps them for parsed markup.
    let page = directory.join("index.html");
    let path = handlers(&page, "<p>no handlers</p>", &[entry]).unwrap();
    let text = source(&key(&path)).unwrap();
    assert_eq!(text.matches("function (event)").count(), 3);
    assert!(text.contains("this.onerror=null;this.src='../thumb.png';"));
    std::fs::remove_dir_all(&directory).unwrap();
}
