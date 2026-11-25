use super::*;
use crate::tests::fixture::Fixture;

/// A stand-in compiler: `@tailwind utilities` becomes a rule for each
/// candidate starting with `tw-`, colored by the `@plugin` module.
const STUB: &str = r#"export async function compile(css, options) {
  const plugin = /@plugin "([^"]+)";/.exec(css);
  const loaded = plugin ? await options.loadModule(plugin[1], options.base, 'plugin') : null;
  const color = loaded ? loaded.module.color : 'red';
  return {
    build(candidates) {
      const rules = candidates.filter((name) => name.startsWith('tw-')).map((name) => '.' + name + '{color:' + color + '}');
      return css.replace(/@plugin "[^"]+";/, '').replace('@tailwind utilities;', rules.join(''));
    },
  };
}
"#;

#[test]
fn candidates_keep_arbitrary_values_and_variants_whole() {
    let mut found = BTreeSet::new();
    extract(
        r#"<div className={cn("px-2 [&>*]:p-1", ok && 'w-[calc(100%-2rem)]')} class="data-[state=open]:bg-red-500 content-['a_b'] hover:underline.">"#,
        &mut found,
    );
    for candidate in [
        "px-2",
        "[&>*]:p-1",
        "w-[calc(100%-2rem)]",
        "data-[state=open]:bg-red-500",
        "content-['a_b']",
        "hover:underline",
    ] {
        assert!(found.contains(candidate), "{candidate} in {found:?}");
    }
}

#[test]
fn stylesheets_name_their_modules_and_need_the_compiler_for_directives() {
    assert!(needed("@import 'x';\n@theme { --a: 1 }"));
    assert!(!needed("@layer base { a {} }"));
    assert_eq!(
        modules(
            "@plugin \"@tailwindcss/typography\";\n@config './tailwind.config.js';\n@plugin \"@tailwindcss/typography\";"
        ),
        ["./tailwind.config.js", "@tailwindcss/typography"]
    );
}

#[test]
fn stylesheets_with_directives_build_with_the_project_compiler_and_its_plugins() {
    let fixture = Fixture::new(&[
        (
            "package.json",
            r#"{"name":"native-tailwind","main":"main.cjs","devDependencies":{"electron":"44"}}"#,
        ),
        ("main.cjs", "require('./src/style.css');"),
        (
            "src/style.css",
            "@import \"tailwindcss\";\n@plugin \"tint\";\nbody { margin: 0 }",
        ),
        (
            "src/App.tsx",
            "export const App = () => <p className=\"tw-a tw-b plain\" />;",
        ),
        ("src/plain.css", "a { color: red }"),
        (
            "node_modules/tailwindcss/package.json",
            r#"{"name":"tailwindcss","exports":{".":{"style":"./index.css","import":"./lib.mjs"}}}"#,
        ),
        ("node_modules/tailwindcss/index.css", "@tailwind utilities;"),
        ("node_modules/tailwindcss/lib.mjs", STUB),
        (
            "node_modules/tint/package.json",
            r#"{"name":"tint","main":"index.cjs"}"#,
        ),
        (
            "node_modules/tint/index.cjs",
            "module.exports = { color: 'blue' };",
        ),
    ]);
    let args = Args {
        project: fixture.root().to_path_buf(),
        out: Some("native-output".into()),
        rustc: Some("rustc".into()),
        work_dir: None,
        sign: None,
        installer: None,
        config: None,
    };
    let cache = fixture.path("cache");
    let project = nativeboat_project::load(fixture.root()).unwrap();
    let sheets = prepare_in(&project, &args, &cache).unwrap();
    assert_eq!(sheets.len(), 1, "{sheets:?}");
    let built = &sheets[&fixture.path("src/style.css")];
    assert!(
        built.contains(".tw-a{color:blue}.tw-b{color:blue}"),
        "{built}"
    );
    assert!(
        built.contains("body { margin: 0 }") && !built.contains("@plugin"),
        "{built}"
    );
    // The second build reuses the tool and the built sheet.
    assert_eq!(&prepare_in(&project, &args, &cache).unwrap(), &sheets);
}
