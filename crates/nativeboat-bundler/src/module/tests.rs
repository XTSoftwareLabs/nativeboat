use super::*;

fn esm(code: &str) -> RewrittenModule {
    let module = rewrite(code).unwrap();
    assert!(module.esm, "{}", module.code);
    module
}

fn specifiers(module: &RewrittenModule) -> Vec<(&str, DependencyKind)> {
    module
        .dependencies
        .iter()
        .map(|dependency| (dependency.specifier.as_str(), dependency.kind))
        .collect()
}

#[test]
fn require_resolve_collects_only_unshadowed_literal_dependencies() {
    let module = rewrite("require.resolve('tool/bin/tool.exe'); require.resolve('tool/bin/tool.exe'); require.resolve(name); other.resolve('ignored'); function local(require) { require.resolve('shadowed'); }").unwrap();
    assert_eq!(
        specifiers(&module),
        vec![("tool/bin/tool.exe", DependencyKind::Resolve)]
    );
    assert_eq!(
        module.warnings,
        vec!["dynamic require.resolve: require.resolve(name)"]
    );
    assert!(module.code.contains("require.resolve('tool/bin/tool.exe')"));
}

#[test]
fn create_require_bindings_are_recognized_by_their_module_and_filename() {
    for source in [
        "import {createRequire as make} from 'node:module'; const load=make(import.meta.url); load.resolve(name); load('known');",
        "import * as modules from 'module'; const load=modules.createRequire(import.meta.filename); load.resolve(name); load('known');",
        "import modules from 'node:module'; const load=modules.createRequire(__filename); load.resolve(name); load('known');",
        "const modules=require('module'); const {createRequire:make,other}=modules; const load=make(__filename); load.resolve(name); load('known');",
        "const {createRequire:make}=require('node:module'); const load=make(__filename); load.resolve(name); load('known');",
        "const load=require('module').createRequire(__filename); load.resolve(name); load('known');",
    ] {
        let module = rewrite(source).unwrap();
        assert!(module.dynamic_resolutions, "{source}");
        assert!(
            module
                .dependencies
                .iter()
                .any(|dependency| dependency.specifier == "known"
                    && dependency.kind == DependencyKind::Require)
        );
        assert!(
            module
                .warnings
                .iter()
                .any(|warning| warning.contains("dynamic require.resolve"))
        );
    }
    let literal = esm(
        "import {createRequire} from 'module'; const load=createRequire(import.meta.url); load.resolve('resource.exe');",
    );
    assert!(
        literal
            .dependencies
            .iter()
            .any(|dependency| dependency.specifier == "resource.exe"
                && dependency.kind == DependencyKind::Resolve)
    );
    assert!(!literal.dynamic_resolutions);
}

#[test]
fn unrelated_mutable_and_shadowed_loaders_do_not_expand_dependencies() {
    for source in [
        "import {createRequire} from 'unrelated'; const load=createRequire(import.meta.url); load.resolve(name);",
        "import {createRequire as make} from 'module'; function local(make) { const load=make(import.meta.url); load.resolve(name); }",
        "import {createRequire} from 'module'; let load=createRequire(import.meta.url); load.resolve(name);",
        "import {createRequire} from 'module'; const __filename='elsewhere'; const load=createRequire(__filename); load.resolve(name);",
        "import {createRequire} from 'module'; const load=createRequire('C:/elsewhere.cjs'); load.resolve(name);",
        "import {createRequire} from 'module'; const load=createRequire(other.url); load.resolve(name);",
        "import {createRequire} from 'module'; const load=createRequire(import.meta.other); load.resolve(name);",
        "import {createRequire} from 'module'; const load=createRequire(); load.resolve(name);",
        "const unrelated=require('other'); const load=unrelated.createRequire(__filename); load.resolve(name);",
        "function local(require) {const {createRequire}=require('module'); const load=createRequire(__filename); load.resolve(name);}",
        "const [load]=other; load.resolve(name); const {createRequire:{nested}}=require('module');",
        "const alias=plain; const load=alias(__filename); load.resolve(name); let uninitialized; require(name);",
    ] {
        assert!(!rewrite(source).unwrap().dynamic_resolutions, "{source}");
    }
}

#[test]
fn imports_become_live_member_accesses() {
    let module = esm(
        "import React, { useState as useS, \"a-b\" as ab } from 'react';\n\
         import * as path from 'path';\n\
         import 'side-effect';\n\
         import again from 'react';\n\
         const x = useS(1) + ab + path.sep + React.version + again;\n\
         const o = { useS, other: 1, x };\n\
         useS`tpl`;\n\
         x.f(); plain`t`;\n",
    );
    assert!(
        module
            .code
            .starts_with("\"use strict\";\n__nui_export(exports, {  });\n")
    );
    assert!(
        module
            .code
            .contains("const __nui_i0 = __nui_interop(__nui_require(\"react\"));")
    );
    assert!(
        module
            .code
            .contains("const __nui_i1 = __nui_interop(__nui_require(\"path\"));")
    );
    assert!(
        module
            .code
            .contains("const __nui_i2 = __nui_interop(__nui_require(\"side-effect\"));")
    );
    assert!(!module.code.contains("__nui_i3"));
    assert!(module.code.contains(
        "const x = (0, __nui_i0.useState)(1) + __nui_i0[\"a-b\"] + __nui_i1.sep + __nui_i0.default.version + __nui_i0.default;"
    ));
    assert!(
        module
            .code
            .contains("const o = { useS: __nui_i0.useState, other: 1, x };")
    );
    assert!(module.code.contains("(0, __nui_i0.useState)`tpl`;"));
    assert!(module.code.contains("x.f(); plain`t`;"));
    assert_eq!(
        specifiers(&module),
        vec![
            ("react", DependencyKind::Import),
            ("path", DependencyKind::Import),
            ("side-effect", DependencyKind::Import),
        ]
    );
}

#[test]
fn exports_become_getters() {
    let module = esm("import { a as imported } from './a';\n\
         export const one = 1, { two, nested: [three] } = { two: 2, nested: [3] };\n\
         export let four;\n\
         export function five() {}\n\
         export class Six {}\n\
         export { one as uno, imported, imported as again };\n\
         export { b as bee, default as dflt, \"str\" as \"s-name\" } from './b';\n\
         export * from './c';\n\
         export * as nsd from './d';\n\
         export default class Seven {}\n");
    let header = module.code.lines().nth(1).unwrap();
    assert_eq!(
        header,
        "__nui_export(exports, { \"bee\": () => __nui_i1.b, \"dflt\": () => __nui_i1.default, \
         \"s-name\": () => __nui_i1.str, \"nsd\": () => __nui_i3, \"one\": () => one, \"two\": () => two, \
         \"three\": () => three, \"four\": () => four, \"five\": () => five, \"Six\": () => Six, \
         \"uno\": () => one, \"imported\": () => __nui_i0.a, \"again\": () => __nui_i0.a, \
         \"default\": () => Seven });"
    );
    assert!(
        module
            .code
            .contains("__nui_export_star(exports, __nui_i2);")
    );
    assert!(
        module
            .code
            .contains("\nconst one = 1, { two, nested: [three] }")
    );
    assert!(module.code.contains("\nfunction five() {}"));
    assert!(module.code.contains("\nclass Seven {}"));
    assert!(!module.code.contains("export "));
    assert_eq!(
        specifiers(&module),
        vec![
            ("./a", DependencyKind::Import),
            ("./b", DependencyKind::ExportFrom),
            ("./c", DependencyKind::ExportFrom),
            ("./d", DependencyKind::ExportFrom),
        ]
    );
}

#[test]
fn default_exports() {
    let function = esm("export default function named() { return 1 }");
    assert!(function.code.contains("\"default\": () => named"));
    assert!(function.code.ends_with("function named() { return 1 }"));
    let anonymous = esm("export default function () { return 1 }");
    assert!(anonymous.code.contains("\"default\": () => __nui_default"));
    assert!(
        anonymous
            .code
            .ends_with("const __nui_default = function () { return 1 };")
    );
    let class = esm("export default class { }");
    assert!(class.code.ends_with("const __nui_default = class { };"));
    let expression = esm("import v from 'v';\nexport default v + 1;");
    assert!(
        expression
            .code
            .ends_with("const __nui_default = __nui_i0.default + 1;")
    );
}

#[test]
fn quoting_and_member_names() {
    assert_eq!(quote("a\"b\\c\nd\re"), "\"a\\\"b\\\\c\\nd\\re\"");
    assert_eq!(member("m", "plain_$1"), "m.plain_$1");
    assert_eq!(member("m", "$"), "m.$");
    assert_eq!(member("m", "1a"), "m[\"1a\"]");
    assert_eq!(member("m", ""), "m[\"\"]");
    let module = esm("import x from 'we\"ird';\nx;");
    assert!(module.code.contains("require(\"we\\\"ird\")"));
}

#[test]
fn commonjs_modules_are_left_alone() {
    let module = rewrite(
        "const a = require('a');\n\
         const b = require(`b`);\n\
         const c = require('c', 'extra');\n\
         function f(require) { return require('shadowed'); }\n\
         module.exports = { a, f };\n",
    )
    .unwrap();
    assert!(!module.esm);
    assert!(module.code.starts_with("const a = require('a');"));
    assert_eq!(specifiers(&module), vec![("a", DependencyKind::Require)]);
    assert_eq!(
        module.warnings,
        vec![
            "dynamic require: require(`b`)".to_owned(),
            "dynamic require: require('c', 'extra')".to_owned()
        ]
    );
}

#[test]
fn dynamic_imports_and_import_meta() {
    let module =
        rewrite("const a = import('./a');\nconst b = import(name);\nconsole.log(import.meta.url);")
            .unwrap();
    assert!(module.esm);
    assert!(module.code.contains(
        "const a = Promise.resolve('./a').then((specifier) => __nui_interop(__nui_require(specifier)));"
    ));
    assert!(
        module
            .code
            .contains("const b = Promise.resolve(name).then(")
    );
    assert!(module.code.contains("console.log(__nui_import_meta.url);"));
    assert_eq!(
        specifiers(&module),
        vec![("./a", DependencyKind::DynamicImport)]
    );
    assert_eq!(
        module.warnings,
        vec!["dynamic import: import(name)".to_owned()]
    );
    let commonjs = rewrite("module.exports = () => import('./x');").unwrap();
    assert!(!commonjs.esm);
    assert!(commonjs.code.contains("Promise.resolve('./x')"));
}

#[test]
fn dead_branches_are_skipped() {
    let module = rewrite(
        "if (\"production\" === \"production\") { require('live1'); } else { require('dead1'); }\n\
         if (\"a\" !== \"a\") require('dead2'); else require('live2');\n\
         if (\"a\" != \"a\") require('dead3');\n\
         if ((\"a\" == \"b\")) require('dead4');\n\
         if (!true) require('dead5');\n\
         if (false && x) require('dead6');\n\
         if (true || x) require('live3');\n\
         if (true && \"a\" === \"a\") require('live4');\n\
         if (false || false) require('dead7');\n\
         if (true ?? x) require('maybe1');\n\
         if (\"a\" < \"b\") require('maybe2');\n\
         if (\"a\" === b) require('maybe3');\n\
         if (b) require('maybe4');\n\
         const c = \"a\" === \"a\" ? require('live5') : require('dead8');\n\
         const d = \"a\" === \"b\" ? require('dead9') : require('live6');\n\
         const e = b ? require('maybe5') : require('maybe6');\n\
         const f = false && require('dead10');\n\
         const g = true || require('dead11');\n\
         const h = b && require('maybe7');\n",
    )
    .unwrap();
    let names: Vec<&str> = specifiers(&module)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(
        names,
        vec![
            "live1", "live2", "live3", "live4", "maybe1", "maybe2", "maybe3", "maybe4", "live5",
            "live6", "maybe5", "maybe6", "maybe7"
        ]
    );
}

#[test]
fn errors() {
    assert!(rewrite("let let = ;").is_err());
    let error = rewrite("export const x = await load();").unwrap_err();
    assert_eq!(error, vec!["top-level await is not supported".to_owned()]);
    assert!(
        rewrite("export async function f() { await g(); }\nconst h = async () => await g();")
            .is_ok()
    );
}
