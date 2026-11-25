use super::*;
use boa_engine::job::TimeoutJob;
use nativeboat_bundler::{Bundle, Module, rewrite};
use shimforge::{Session, mock};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

mod stack;

type Output = Rc<RefCell<Vec<(Level, String)>>>;

fn host(node: bool) -> (Host, Output) {
    let output: Output = Rc::default();
    let sink = output.clone();
    let host = Host::new(
        node,
        Rc::new(move |level, text: &str| sink.borrow_mut().push((level, text.to_owned()))),
    );
    (host, output)
}

fn lines(output: &Output) -> Vec<String> {
    output
        .borrow_mut()
        .drain(..)
        .map(|(_, text)| text)
        .collect()
}

#[test]
fn module_compilation_preserves_host_optimizer_options_on_success_and_parse_failure() {
    let (mut host, _) = host(false);
    let options = boa_engine::optimizer::OptimizerOptions::CONSTANT_FOLDING;
    host.context().set_optimizer_options(options);
    let args = |code: &str| {
        [
            JsString::from(code).into(),
            JsString::from("module-optimizer.js").into(),
            JsString::from("").into(),
        ]
    };
    let value = compile(
        &JsValue::undefined(),
        &args("exports.value=1+2"),
        host.context(),
    )
    .unwrap();
    assert!(value.as_object().unwrap().is_callable());
    assert_eq!(host.context().optimizer_options(), options);
    assert!(compile(&JsValue::undefined(), &args("const ="), host.context()).is_err());
    assert_eq!(host.context().optimizer_options(), options);
    assert_eq!(string(&mut host, "1+2"), "3");
}

fn run(host: &mut Host, code: &str) -> JsValue {
    host.eval(code, "test.js").unwrap()
}

fn string(host: &mut Host, code: &str) -> String {
    let value = run(host, code);
    value
        .to_string(host.context())
        .unwrap()
        .to_std_string_escaped()
}

/// A module in the runtime format, from ES module or CommonJS source.
fn module(path: &str, source: &str, dependencies: &[(&str, Resolution)]) -> Module {
    Module {
        path: path.to_owned(),
        runtime_path: None,
        code: rewrite(source).unwrap().code,
        dependencies: dependencies
            .iter()
            .map(|(specifier, resolution)| ((*specifier).to_owned(), resolution.clone()))
            .collect(),
    }
}

fn script(modules: Vec<Module>, entries: Vec<usize>) -> Script {
    Script {
        runtime_paths: Vec::new(),
        bundle: Bundle {
            entries,
            modules,
            ..Bundle::default()
        },
        env: BTreeMap::from([("MODE".to_owned(), json!("production"))]),
    }
}

#[test]
fn cached_literals_keep_native_identity_state_prototypes_and_module_scope() {
    let (mut host, _) = host(false);
    let code = r#"
        'use strict';
        const original = RegExp;
        const proto = RegExp.prototype;
        globalThis.RegExp = function () { throw Error('overridden constructor'); };
        function literal() { return /(?<letter>a)/gy; }
        const first = literal();
        first.lastIndex = 1;
        const second = literal();
        const descriptor = Object.getOwnPropertyDescriptor(second, 'lastIndex');
        globalThis.regexResult = [first !== second, first.lastIndex === 1,
            second.lastIndex === 0, second.exec('a').groups.letter === 'a',
            first.exec('ba').index === 1, Object.getPrototypeOf(second) === proto,
            descriptor.writable, !descriptor.enumerable, !descriptor.configurable,
            second.source === '(?<letter>a)', second.flags === 'gy',
            /😀/u.test('😀'), /\uD800/.test('\uD800'),
            arguments.length === 0, this === exports,
            typeof __nui_regex_0 === 'undefined'];
        globalThis.RegExp = original;
    "#;
    host.load(
        &script(vec![module("regex.js", code, &[])], vec![0]),
        &[],
        "",
    )
    .unwrap();
    assert_eq!(string(&mut host, "regexResult.every(Boolean)"), "true");
}

#[test]
fn cached_literals_preserve_compact_keyword_boundaries_and_new_targets() {
    let (mut host, _) = host(false);
    host.load(
        &script(vec![module("compact-regex.js", r#"
          function returned(){return/x/;}
          function* yielded(){yield/y/;}
          let thrown;try{throw/z/;}catch(value){thrown=value.source;}
          const constructed=new /x/.constructor('y','g');
          let nonconstructor=false;try{new /x/;}catch(error){nonconstructor=error instanceof TypeError;}
          globalThis.compactRegex=[returned().source,yielded().next().value.source,
            typeof/x/,void/x/,thrown,constructed.source,constructed.flags,nonconstructor].join('|');
        "#, &[])], vec![0]),
        &[], "",
    ).unwrap();
    assert_eq!(string(&mut host, "compactRegex"), "x|y|object||z|y|g|true");
}

#[test]
fn cached_literals_allow_indirect_eval_without_exposing_lexical_factory() {
    let (mut host, _) = host(false);
    host.load(
        &script(
            vec![module(
                "indirect-eval.js",
                r#"
          const local='module';
          const intrinsics={eval};
          function literal(){return /x/g;}
          globalThis.indirectRegex=[typeof eval, intrinsics.eval('typeof local'),
            (0,eval)('typeof __nui_regex_0'), eval?.('typeof __nui_regex_0'),
            literal() !== literal(), literal().lastIndex === 0, literal().source].join('|');
        "#,
                &[],
            )],
            vec![0],
        ),
        &[],
        "",
    )
    .unwrap();
    assert_eq!(
        string(&mut host, "indirectRegex"),
        "function|undefined|undefined|undefined|true|true|x"
    );
}

#[test]
fn matcher_caching_preserves_function_source_and_dynamic_construction() {
    let (mut host, _) = host(false);
    host.load(
        &script(
            vec![module(
                "regex-reflection.js",
                r#"
        function literal(){return /(?<word>a+)/gy;}
        const copied=Function('return ('+literal.toString()+')')();
        const first=copied(), second=copied();
        first.lastIndex=1;
        let reads=0;
        const source={toString(){reads++;return 'a+';}};
        const dynamicOne=new RegExp(source,'g'), dynamicTwo=new RegExp(source,'g');
        globalThis.regexReflection=[first!==second,second.lastIndex===0,
          first.exec('baa').groups.word==='aa',second.exec('aa').index===0,
          !literal.toString().includes('__nui_regex'),reads===2,
          dynamicOne!==dynamicTwo,dynamicOne.source===dynamicTwo.source,
          dynamicOne.exec('a').index===0,dynamicTwo.lastIndex===0].every(Boolean);
    "#,
                &[],
            )],
            vec![0],
        ),
        &[],
        "",
    )
    .unwrap();
    assert_eq!(string(&mut host, "regexReflection"), "true");
}

#[test]
fn cached_matchers_preserve_dynamic_eval_and_extra_parameter_scopes() {
    let (mut host, _) = host(false);
    let code = "const name='local'; globalThis.evalRegex=eval('name') + /x/.source;";
    host.load(
        &script(vec![module("eval.js", code, &[])], vec![0]),
        &[],
        "",
    )
    .unwrap();
    assert_eq!(string(&mut host, "evalRegex"), "localx");
    let factory = compile(
        &JsValue::undefined(),
        &[
            JsString::from("globalThis.extraRegex=[process,/x/.source,arguments.length,this];")
                .into(),
            JsString::from("extra-regex.js").into(),
            JsString::from("process").into(),
        ],
        host.context(),
    )
    .unwrap();
    let receiver = JsObject::with_null_proto();
    let mut args = vec![JsValue::undefined(); 7];
    args.push(42.into());
    factory
        .as_callable()
        .unwrap()
        .call(&receiver.clone().into(), &args, host.context())
        .unwrap();
    assert_eq!(
        string(&mut host, "extraRegex.slice(0,3).join(',')"),
        "42,x,0"
    );
    let actual = run(&mut host, "extraRegex[3]");
    assert!(JsObject::equals(&actual.as_object().unwrap(), &receiver));
}

#[test]
fn literal_preparation_preserves_global_scripts_and_early_syntax_errors() {
    let (mut host, _) = host(false);
    let error = host
        .load(
            &script(
                vec![module("invalid-pattern.js", "if(false) /(/;", &[])],
                vec![0],
            ),
            &[],
            "",
        )
        .unwrap_err();
    assert!(error.contains("SyntaxError"), "{error}");
    host.eval(
        "const keptGlobalBinding=19; globalThis.keptGlobalRegex=/x/g;",
        "global.js",
    )
    .unwrap();
    assert_eq!(
        string(
            &mut host,
            "[keptGlobalBinding, keptGlobalRegex.source, typeof __nui_regex_0].join(',')"
        ),
        "19,x,undefined"
    );
}

#[test]
fn native_sources_survive_the_input_bundle_and_only_compile_requested_modules() {
    let (mut host, _) = host(false);
    let modules = vec![
        module(
            "entry.js",
            "globalThis.later=()=>require('./later');globalThis.invalidLater=()=>require('./invalid');",
            &[
                ("./later", Resolution::Module(1)),
                ("./invalid", Resolution::Module(2)),
            ],
        ),
        module(
            "later.js",
            "globalThis.lazyRuns=(globalThis.lazyRuns||0)+1;module.exports='船🚢';",
            &[],
        ),
        Module {
            path: "invalid.js".into(),
            runtime_path: None,
            code: "const =".into(),
            dependencies: Vec::new(),
        },
    ];
    host.load(&script(modules, vec![0]), &[], "/native/")
        .unwrap();
    assert_eq!(string(&mut host, "typeof lazyRuns"), "undefined");
    assert_eq!(
        string(&mut host, "[later(),later(),lazyRuns].join(',')"),
        "船🚢,船🚢,1"
    );
    assert_eq!(
        string(
            &mut host,
            "try { invalidLater(); } catch(error) { error.name; }"
        ),
        "SyntaxError"
    );
}

#[test]
fn lazy_requires_keep_unloaded_contexts_identity_and_graph_replacement() {
    let (mut host, _) = host(false);
    let object = host.host_object();
    let create_require = object
        .get(js_string!("createRequire"), host.context())
        .unwrap();
    host.define_builtin("test-create-require", create_require);
    let mut unloaded = module(
        "lazy.js",
        "globalThis.lazyRequireRuns=(globalThis.lazyRequireRuns||0)+1;module.exports=require;",
        &[("answer", Resolution::Module(2))],
    );
    unloaded.runtime_path = Some("lazy/index.js".into());
    let mut duplicate = module("duplicate.js", "module.exports='unused';", &[]);
    duplicate.runtime_path = unloaded.runtime_path.clone();
    host.load(
        &script(
            vec![
                module(
                    "entry.js",
                    "globalThis.createRequire=require('test-create-require');globalThis.rootRequire=require;",
                    &[],
                ),
                unloaded,
                module("answer.js", "module.exports=42;", &[]),
                duplicate,
            ],
            vec![0],
        ),
        &[],
        "C:/native/",
    )
    .unwrap();
    assert_eq!(
        string(
            &mut host,
            r#"
            globalThis.lazyRequire=createRequire('C:/native/lazy/index.js');
            [typeof lazyRequireRuns,
             lazyRequire===createRequire('C:\\NATIVE\\lazy\\index.js'),
             lazyRequire===createRequire('C:/native/lazy/synthetic.cjs'),
             lazyRequire.cache===rootRequire.cache,
             lazyRequire.resolve('answer'), typeof lazyRequireRuns].join('|')
            "#
        ),
        "undefined|true|true|true|C:/native/answer.js|undefined"
    );
    assert_eq!(
        string(
            &mut host,
            "[lazyRequire('answer'),rootRequire('./lazy/index.js')===lazyRequire,lazyRequireRuns].join('|')"
        ),
        "42|true|1"
    );
    let mut replacement = module("new.js", "module.exports='new';", &[]);
    replacement.runtime_path = Some("lazy/index.js".into());
    host.load(&script(vec![replacement], vec![]), &[], "C:/native/")
        .unwrap();
    assert_eq!(
        string(
            &mut host,
            "createRequire('C:/native/lazy/index.js')!==lazyRequire && lazyRequire('answer')===42"
        ),
        "true"
    );
}

#[test]
fn console_formats_values() {
    let (mut host, output) = host(false);
    run(
        &mut host,
        r#"
        class Point { constructor() { this.x = 1; } }
        const circle = { name: 'loop' }; circle.self = circle;
        console.log('text', 1, -0, 2n, Symbol('s'), true, undefined, null);
        console.log({ a: [1, 'two', { deep: { deeper: { deepest: 1 } } }], 'b-c': "it's" });
        console.log(new Map([[1, 'one']]), new Set([2]), new Uint8Array([3, 4]), [], {});
        console.log(function named() {}, () => {}, new Date(0), new Date(NaN), /x/g);
        console.log(new Point(), Object.create(null), circle, Promise.resolve(1));
        console.log(new Array(102).fill(0).length, new Array(102).fill(0));
        console.log('%s|%d|%i|%f|%j|%o|%O|%c|%%|%s', 'a', '4.5', 4.5, '1.5', { j: 1 }, [1], { o: 1 }, 'color: red', { s: 1 });
        console.log('%s %s', 'only');
        console.info('info'); console.debug('debug'); console.warn('warn'); console.error('error');
        console.trace('here');
        console.dir({ dir: 1 }); console.table([1]);
        console.assert(true, 'fine'); console.assert(false, 'broken');
        console.group('group'); console.log('inner\nlines'); console.groupCollapsed(); console.groupEnd(); console.groupEnd();
        console.count(); console.count(); console.count('x'); console.countReset();
        console.time(); console.timeLog('missing'); console.timeEnd();
        console.log(new Error('boom'));
        "#,
    );
    let printed = output.borrow().clone();
    let texts: Vec<&str> = printed.iter().map(|(_, text)| text.as_str()).collect();
    assert_eq!(texts[0], "text 1 -0 2n Symbol(s) true undefined null");
    assert_eq!(
        texts[1],
        "{ a: [ 1, 'two', { deep: [Object] } ], 'b-c': 'it\\'s' }"
    );
    assert_eq!(
        texts[2],
        "Map(1) { 1 => 'one' } Set(1) { 2 } Uint8Array(2) [ 3, 4 ] [] {}"
    );
    assert_eq!(
        texts[3],
        "[Function: named] [Function (anonymous)] 1970-01-01T00:00:00.000Z Invalid Date /x/g"
    );
    assert_eq!(
        texts[4],
        "Point { x: 1 } [Object: null prototype] {} { name: 'loop', self: [Circular] } Promise { <pending> }"
    );
    assert!(texts[5].starts_with("102 [ 0, 0,"));
    assert!(texts[5].ends_with("0, ... 2 more items ]"));
    assert_eq!(texts[6], "a|4.5|4|1.5|{\"j\":1}|[ 1 ]|{ o: 1 }||%|{ s: 1 }");
    assert_eq!(texts[7], "only %s");
    let levels: Vec<Level> = printed[8..12].iter().map(|(level, _)| *level).collect();
    assert_eq!(
        levels,
        [Level::Info, Level::Debug, Level::Warn, Level::Error]
    );
    assert_eq!(texts[12], "Trace: here");
    assert_eq!(texts[13..15], ["{ dir: 1 }", "[ 1 ]"]);
    assert_eq!(texts[15], "Assertion failed broken");
    assert_eq!(texts[16..18], ["group", "  inner\n  lines"]);
    assert_eq!(texts[18..21], ["default: 1", "default: 2", "x: 1"]);
    assert!(texts[21].starts_with("default: ") && texts[21].ends_with("ms"));
    assert!(texts[22].starts_with("Error: boom"), "{}", texts[22]);
    assert_eq!(texts.len(), 23);
}

#[test]
fn bundles_load_with_interop() {
    let (mut host, output) = host(false);
    let fs = run(&mut host, "({ readFileSync: () => 'data' })");
    host.define_builtin("fs", fs);
    let modules = vec![
        module(
            "src/main.js",
            "import value, { named } from './lib.js';\n\
             import * as all from './lib.js';\n\
             import cjs, { helper } from './cjs.js';\n\
             import { readFileSync } from 'fs';\n\
             export * from './lib.js';\n\
             globalThis.result = [value, named, all.named, cjs.helper(), helper(), readFileSync(),\n\
                 import.meta.env.MODE, import.meta.url, __filename, __dirname,\n\
                 require('node:fs') === require('fs'), typeof require.resolve('./lib.js'), typeof require.cache].join(',');\n\
             export const later = 1;",
            &[
                ("./lib.js", Resolution::Module(1)),
                ("./cjs.js", Resolution::Module(2)),
                ("fs", Resolution::Builtin("fs".to_owned())),
            ],
        ),
        module(
            "src/lib.js",
            "export default 'default'; export const named = 'named'; export const later = 2;",
            &[],
        ),
        module(
            "src/cjs.js",
            "module.exports = { helper: () => 'helped' }; module.exports.default = 'ignored';",
            &[],
        ),
        Module {
            path: "classic.js".to_owned(),
            runtime_path: None,
            code: "var classicGlobal = typeof module;".to_owned(),
            dependencies: Vec::new(),
        },
        module("src/plain.js", "module.exports = 7;", &[]),
        module(
            "src/uses-plain.js",
            "import seven from './plain.js'; import * as ns from './plain.js'; globalThis.plain = [seven, ns.default, require('./plain.js')].join();",
            &[("./plain.js", Resolution::Module(4))],
        ),
    ];
    host.load(&script(modules, vec![0, 3, 5]), &[true, false], "/app/")
        .unwrap();
    assert!(lines(&output).is_empty());
    assert_eq!(
        string(&mut host, "result"),
        "default,named,named,helped,helped,data,production,file:///app/src/main.js,/app/src/main.js,/app/src,true,string,object"
    );
    assert_eq!(string(&mut host, "classicGlobal"), "undefined");
    assert_eq!(string(&mut host, "plain"), "7,7,7");
    let scope = run(&mut host, "({ injected: 'scoped', other: 2 })");
    let scoped = vec![module(
        "scoped.js",
        "globalThis.seen = injected + other + typeof injected;",
        &[],
    )];
    host.load_scoped(
        &script(scoped, vec![0]),
        &[],
        "",
        scope.as_object().as_ref(),
    )
    .unwrap();
    assert_eq!(
        string(&mut host, "seen + typeof injected"),
        "scoped2stringundefined"
    );
    let empty = run(&mut host, "({})");
    let unscoped = vec![module(
        "empty.js",
        "globalThis.empty = typeof exports;",
        &[],
    )];
    host.load_scoped(
        &script(unscoped, vec![0]),
        &[],
        "",
        empty.as_object().as_ref(),
    )
    .unwrap();
    assert_eq!(string(&mut host, "empty"), "object");
    let windows = vec![module(
        "main.js",
        "globalThis.windowsDirname = __dirname;",
        &[],
    )];
    host.load(&script(windows, vec![0]), &[], "C:\\app\\")
        .unwrap();
    assert_eq!(string(&mut host, "windowsDirname"), "C:\\app");
    let mut built = script(
        vec![module(
            "electron/main.ts",
            "globalThis.built = __filename + '|' + import.meta.url;",
            &[],
        )],
        vec![0],
    );
    built.runtime_paths = vec!["dist-electron/main.js".to_owned()];
    host.load(&built, &[], "/app/").unwrap();
    assert_eq!(
        string(&mut host, "built"),
        "/app/dist-electron/main.js|file:///app/dist-electron/main.js"
    );
    // Sources compiled file by file run from their own output files.
    let mut compiled = script(
        vec![module(
            "src/main/main.ts",
            "globalThis.compiled = __dirname + '|' + import.meta.url;",
            &[],
        )],
        vec![0],
    );
    compiled.bundle.modules[0].runtime_path = Some("dist/src/main/main.js".to_owned());
    compiled.runtime_paths = vec!["dist/src/main/other.js".to_owned()];
    host.load(&compiled, &[], "/app/").unwrap();
    assert_eq!(
        string(&mut host, "compiled"),
        "/app/dist/src/main|file:///app/dist/src/main/main.js"
    );
    // ES modules may declare the names CommonJS modules receive.
    let shadowing = script(
        vec![
            module(
                "esm.mjs",
                "import { x } from './dep.js'; const __dirname = 'mine'; const require = () => 'own'; const module = 7;\
                 globalThis.shadowed = [x, __dirname, require(), module, typeof exports].join();",
                &[("./dep.js", Resolution::Module(1))],
            ),
            module("dep.js", "export const x = 'dep';", &[]),
        ],
        vec![0],
    );
    host.load(&shadowing, &[], "/app/").unwrap();
    assert_eq!(string(&mut host, "shadowed"), "dep,mine,own,7,object");
}

#[test]
fn circular_and_failing_modules() {
    let (mut host, _) = host(true);
    let modules = vec![
        module(
            "a.js",
            "import { b } from './b.js'; export const a = 'a'; globalThis.circle = b();",
            &[("./b.js", Resolution::Module(1))],
        ),
        module(
            "b.js",
            "import { a } from './a.js'; export const b = () => a;",
            &[("./a.js", Resolution::Module(0))],
        ),
    ];
    host.load(&script(modules, vec![0]), &[], "").unwrap();
    assert_eq!(string(&mut host, "circle"), "a");
    let failing = vec![
        module(
            "main.js",
            "let first; try { require('./bad.js'); } catch (error) { first = error.message; }\n\
             let missing; try { require('optional'); } catch (error) { missing = error.code + ' ' + error.message; }\n\
             let unknown; try { require('electron'); } catch (error) { unknown = error.message; }\n\
             globalThis.failures = [first, missing, unknown];\n\
             require('./bad.js');",
            &[
                ("./bad.js", Resolution::Module(1)),
                ("optional", Resolution::Missing("optional".to_owned())),
            ],
        ),
        module("bad.js", "throw new TypeError('bad module');", &[]),
    ];
    let error = host.load(&script(failing, vec![0]), &[], "").unwrap_err();
    assert!(error.starts_with("TypeError: bad module"), "{error}");
    assert_eq!(
        string(&mut host, "failures.join('|')"),
        "bad module|MODULE_NOT_FOUND Cannot find module 'optional'|Cannot find module 'electron': it is not provided by nativeboat"
    );
}

#[test]
fn resolve_only_files_have_paths_without_becoming_executable_modules() {
    let (mut host, _) = host(true);
    let modules = vec![module(
        "main.js",
        r#"
      globalThis.tool = require.resolve('tool/bin/tool.exe');
      globalThis.computedTool = require.resolve(tool);
      try { require('tool/bin/tool.exe'); } catch(error) { globalThis.rawFailure=error.code+' '+error.message; }
    "#,
        &[(
            "tool/bin/tool.exe",
            Resolution::File("node_modules/tool/bin/tool.exe".into()),
        )],
    )];
    host.load(&script(modules, vec![0]), &[], "C:/installed/app/")
        .unwrap();
    assert_eq!(
        string(&mut host, "tool"),
        "C:/installed/app/node_modules/tool/bin/tool.exe"
    );
    assert_eq!(
        string(&mut host, "computedTool"),
        "C:/installed/app/node_modules/tool/bin/tool.exe"
    );
    assert_eq!(
        string(&mut host, "rawFailure"),
        "MODULE_NOT_FOUND Cannot find module 'tool/bin/tool.exe': this file has no compiled module"
    );
}

#[test]
fn empty_browser_replacements_require_as_empty_modules() {
    let (mut host, _) = host(false);
    let entry = module(
        "main.js",
        "const empty = require('./optional'); empty.answer = 42; globalThis.emptyAnswer = empty.answer; globalThis.emptyPath = require.resolve('./optional');",
        &[("./optional", Resolution::Empty)],
    );
    host.load(&script(vec![entry], vec![0]), &[], "C:/app/")
        .unwrap();

    assert_eq!(
        string(&mut host, "emptyAnswer + '|' + emptyPath"),
        "42|./optional"
    );
}

#[test]
fn timers_run_in_order() {
    let (mut host, output) = host(false);
    assert_eq!(host.next_timer(), None);
    run(
        &mut host,
        r#"
        globalThis.log = [];
        setTimeout((tag) => log.push(tag), 0, 'timeout');
        const cancelled = setTimeout(() => log.push('cancelled'), 0);
        clearTimeout(cancelled);
        setImmediate(() => { log.push('immediate'); Promise.resolve().then(() => log.push('microtask')); });
        const interval = setInterval(() => { log.push('interval'); if (log.filter((x) => x === 'interval').length === 2) clearInterval(interval); }, 0);
        setTimeout(() => { throw new Error('timer failed'); }, 0);
        queueMicrotask(() => log.push('queued'));
        setTimeout(() => log.push('late'), 60000);
        typeof cancelled;
        "#,
    );
    host.run_microtasks();
    assert_eq!(string(&mut host, "log.join()"), "queued");
    let wait = host.next_timer().unwrap();
    assert!(wait <= Duration::from_millis(1));
    std::thread::sleep(Duration::from_millis(5));
    host.run_timers();
    host.run_timers();
    std::thread::sleep(Duration::from_millis(5));
    host.run_timers();
    assert_eq!(
        string(&mut host, "log.join()"),
        "queued,timeout,immediate,microtask,interval,interval"
    );
    let errors = lines(&output);
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0].starts_with("Uncaught Error: timer failed"),
        "{errors:?}"
    );
    let wait = host.next_timer().unwrap();
    assert!(wait > Duration::from_secs(50));
    for code in ["setTimeout('code')", "queueMicrotask(1)"] {
        assert!(
            host.eval(code, "bad.js")
                .unwrap_err()
                .starts_with("TypeError")
        );
    }
}

#[test]
fn node_timers_return_handles() {
    let (mut host, _) = host(true);
    run(
        &mut host,
        r#"
        globalThis.fired = [];
        const handle = setTimeout(() => fired.push('first'), 0);
        globalThis.checks = [handle.ref() === handle, handle.unref() === handle, handle.hasRef(), typeof +handle, global === globalThis];
        handle.refresh();
        const other = setTimeout(() => fired.push('other'), 0);
        clearTimeout(other);
        "#,
    );
    let gone = run(
        &mut host,
        "const gone = setTimeout(() => {}, 0); clearTimeout(gone); gone",
    );
    let refresh = gone
        .as_object()
        .unwrap()
        .get(js_string!("refresh"), host.context())
        .unwrap();
    host.call(&refresh, &gone, &[]).unwrap();
    std::thread::sleep(Duration::from_millis(5));
    host.run_timers();
    assert_eq!(string(&mut host, "fired.join()"), "first");
    assert_eq!(
        string(&mut host, "checks.join()"),
        "true,true,true,number,true"
    );
}

#[test]
fn native_timers_snapshot_due_work_and_allow_callback_and_microtask_cancellation() {
    let (mut host, _) = host(false);
    run(
        &mut host,
        r#"
      globalThis.order=[];
      let second, third;
      setTimeout(function(a,b) {
        order.push([a,b,this===globalThis].join(':'));
        clearTimeout(second);
        Promise.resolve().then(()=>{order.push('microtask');clearTimeout(third)});
        setTimeout(()=>order.push('new'),0);
      },0,'first',7);
      second=setTimeout(()=>order.push('cancelled'),0);
      third=setTimeout(()=>order.push('also cancelled'),0);
      for (const value of [null,undefined,-1,NaN,Infinity,{},'1',1.5]) clearTimeout(value);
      setTimeout(()=>order.push('never'),Infinity);
    "#,
    );
    boa_gc::force_collect();
    host.run_timers();
    assert_eq!(string(&mut host, "order.join()"), "first:7:true,microtask");
    host.run_timers();
    assert_eq!(
        string(&mut host, "order.join()"),
        "first:7:true,microtask,new"
    );
    assert_eq!(host.next_timer(), None);
}

#[test]
fn matcher_caching_preserves_strict_directives_and_original_arguments() {
    let (mut host, _) = host(false);
    // Load a dependent entry so the assertion observes its actual exports.
    host.load(&script(vec![
        module("entry.js", "globalThis.strictResult=require('./strict.js');", &[("./strict.js",Resolution::Module(1))]),
        module("strict.js", "'use strict'; module.exports=[arguments.length,/x/g.test('x'),(function(){return this})()===undefined];", &[])
    ],vec![0]), &[], "/").unwrap();
    assert_eq!(string(&mut host, "strictResult.join()"), "0,true,true");
}

#[test]
fn errors_and_rejections_are_reported() {
    let (mut host, output) = host(false);
    run(
        &mut host,
        r#"
        Promise.reject(new Error('unhandled'));
        const later = Promise.reject('handled later');
        later.catch(() => {});
        Promise.reject(42);
        "#,
    );
    host.run_microtasks();
    let reported = lines(&output);
    assert_eq!(reported.len(), 2, "{reported:?}");
    assert!(reported[0].starts_with("Uncaught (in promise) Error: unhandled"));
    assert_eq!(reported[1], "Uncaught (in promise) 42");
    let thrower = run(
        &mut host,
        "(function thrower() { throw new RangeError('range'); })",
    );
    assert!(host.call(&thrower, &JsValue::undefined(), &[]).is_none());
    assert!(
        host.call(&JsValue::from(1), &JsValue::undefined(), &[])
            .is_none()
    );
    let echo = run(&mut host, "((value) => value * 2)");
    assert_eq!(
        host.call(&echo, &JsValue::undefined(), &[JsValue::from(21)]),
        Some(JsValue::from(42))
    );
    let reported = lines(&output);
    assert!(reported[0].starts_with("Uncaught RangeError: range"));
    assert_eq!(
        reported[1],
        "Uncaught TypeError: the callback is not a function"
    );
    run(&mut host, "globalThis.seen = [];");
    let handler = run(
        &mut host,
        "((error, promise) => { seen.push(String(error) + promise); if (error === 'throw') throw new Error('handler failed'); return error === 'handled'; })",
    );
    host.host_object()
        .set(js_string!("onerror"), handler, false, host.context())
        .unwrap();
    for value in ["handled", "unhandled", "throw"] {
        host.report(JsError::from_opaque(JsValue::from(js_string!(value))));
    }
    assert_eq!(
        string(&mut host, "seen.join()"),
        "handledfalse,unhandledfalse,throwfalse"
    );
    let reported = lines(&output);
    assert_eq!(reported.len(), 3);
    assert_eq!(reported[0], "Uncaught 'unhandled'");
    assert!(reported[1].starts_with("Uncaught Error: handler failed\n    at "));
    assert_eq!(reported[2], "Uncaught 'throw'");
    let message = host
        .eval("throw { custom: true }", "custom.js")
        .unwrap_err();
    assert_eq!(message, "{ custom: true }");
    let message = host.eval("syntax error here", "syntax.js").unwrap_err();
    assert!(message.starts_with("SyntaxError"), "{message}");
    let error = host
        .eval(
            "const e = new Error('no stack'); e.stack = undefined; throw e",
            "stack.js",
        )
        .unwrap_err();
    assert_eq!(error, "Error: no stack");
    let error = host
        .eval(
            "const f = new Error('custom stack'); f.stack = 'at somewhere'; throw f",
            "stack.js",
        )
        .unwrap_err();
    assert_eq!(error, "Error: custom stack\nat somewhere");
}

#[test]
fn queue_runs_generic_jobs_and_reports_failures() {
    let (mut host, output) = host(false);
    let realm = host.context().realm().clone();
    host.context().enqueue_job(
        GenericJob::new(
            |_| Err(JsNativeError::error().with_message("job failed").into()),
            realm.clone(),
        )
        .into(),
    );
    host.context()
        .enqueue_job(GenericJob::new(|_| Ok(JsValue::undefined()), realm).into());
    host.context()
        .enqueue_job(TimeoutJob::new(NativeJobStub::job(), 10).into());
    host.run_microtasks();
    let reported = lines(&output);
    assert_eq!(reported.len(), 1);
    assert!(
        reported[0].starts_with("Uncaught Error: job failed"),
        "{reported:?}"
    );
}

struct NativeJobStub;

#[test]
fn weak_references_release_their_targets_after_the_checkpoint() {
    let (mut host, _) = host(false);
    run(&mut host, "globalThis.held = new WeakRef({});");
    boa_gc::force_collect();
    assert_eq!(string(&mut host, "typeof held.deref()"), "object");
    host.run_microtasks();
    boa_gc::force_collect();
    assert_eq!(string(&mut host, "typeof held.deref()"), "undefined");
}

#[test]
fn engine_failures_preserve_diagnostics_and_bypass_javascript_error_handlers() {
    use boa_engine::error::RuntimeLimitError;
    let (mut host, output) = host(false);
    run(&mut host, "globalThis.handled=0;");
    let natives = host.host_object();
    let handler = run(&mut host, "(()=>{handled++; return true;})");
    natives
        .set(js_string!("onerror"), handler, false, host.context())
        .unwrap();
    for failure in [
        RuntimeLimitError::LoopIteration,
        RuntimeLimitError::Recursion,
        RuntimeLimitError::StackSize,
    ] {
        let error: JsError = failure.into();
        let diagnostic = error.to_string();
        assert_eq!(host.describe(error.clone()), diagnostic);
        host.report(error);
        assert_eq!(lines(&output), vec![format!("Uncaught {diagnostic}")]);
    }
    let realm = host.context().realm().clone();
    host.context()
        .enqueue_job(GenericJob::new(|_| Err(RuntimeLimitError::Recursion.into()), realm).into());
    host.run_microtasks();
    assert!(
        lines(&output)[0]
            .contains("RuntimeLimitError: reached the maximum number of recursive calls")
    );
    assert_eq!(string(&mut host, "handled"), "0");
    host.context().runtime_limits_mut().set_recursion_limit(10);
    let error = host
        .eval(
            "function recurse(){return recurse();} recurse();",
            "deep-application.js",
        )
        .unwrap_err();
    assert!(error.starts_with("RuntimeLimitError:"), "{error}");
    assert!(error.contains("deep-application.js"), "{error}");
    host.context().runtime_limits_mut().set_recursion_limit(512);
    run(&mut host, "setTimeout(()=>{while(true){}},0);");
    host.context()
        .runtime_limits_mut()
        .set_loop_iteration_limit(5);
    host.run_timers();
    let printed = lines(&output);
    assert!(
        printed.iter().any(|line| line
            .contains("RuntimeLimitError: reached the maximum number of iteration loops")),
        "{printed:?}"
    );
    host.context()
        .runtime_limits_mut()
        .disable_loop_iteration_limit();
    assert_eq!(string(&mut host, "handled"), "0");
}

impl NativeJobStub {
    fn job() -> boa_engine::job::NativeJob {
        boa_engine::job::NativeJob::new(|_| Ok(JsValue::undefined()))
    }
}

#[test]
fn host_plumbing_failures() {
    let (mut host, output) = host(false);
    let thrower = run(&mut host, "(() => { throw new Error('call failed'); })");
    assert!(host.call(&thrower, &JsValue::undefined(), &[]).is_none());
    assert!(lines(&output)[0].contains("call failed"));
    let natives = host.host_object();
    let timers = natives.get(js_string!("timers"), host.context()).unwrap();
    timers
        .as_object()
        .unwrap()
        .set(js_string!("next"), JsValue::from(5), false, host.context())
        .unwrap();
    assert_eq!(host.next_timer(), None);
    natives
        .set(
            js_string!("describe"),
            JsValue::undefined(),
            false,
            host.context(),
        )
        .unwrap();
    assert_eq!(host.eval("throw 1", "x.js").unwrap_err(), "unknown error");
    let print = natives.get(js_string!("print"), host.context()).unwrap();
    host.call(
        &print,
        &JsValue::undefined(),
        &[
            JsValue::from(js_string!("other")),
            JsValue::from(js_string!("text")),
        ],
    );
    assert_eq!(
        output.borrow().last().unwrap(),
        &(Level::Log, "text".to_owned())
    );
    assert!(format!("{host:?}").starts_with("Host"));
}

#[test]
fn json_round_trips() {
    let (mut host, _) = host(false);
    let value = host.from_json(&json!({ "a": [1, "two", null, true] }));
    assert_eq!(host.to_json(&value), json!({ "a": [1, "two", null, true] }));
    let function = run(&mut host, "(() => 1)");
    assert_eq!(host.to_json(&function), Value::Null);
    assert_eq!(host.to_json(&JsValue::undefined()), Value::Null);
    let cyclic = run(&mut host, "const c = {}; c.c = c; c");
    assert_eq!(host.to_json(&cyclic), Value::Null);
}

#[test]
fn background_work_reaches_task_handlers() {
    let (mut host, output) = host(true);
    let natives = host.host_object();
    host.context()
        .global_object()
        .set(js_string!("nui"), natives, false, host.context())
        .unwrap();
    let id = run(
        &mut host,
        "globalThis.events = []; nui.task((event, value) => { events.push([event, value instanceof Uint8Array ? [...value] : value]); \
         if (event === 'boom') throw new Error('handler failed'); Promise.resolve().then(() => events.push('microtask')); })",
    )
    .to_u32(host.context())
    .unwrap();
    let background = Background::of(host.context());
    assert!(format!("{background:?}").starts_with("Background"));
    let sender = background.clone();
    std::thread::spawn(move || sender.send(id, "text", Payload::Text("hi".to_owned())))
        .join()
        .unwrap();
    let woken = Arc::new(AtomicUsize::new(0));
    let counter = woken.clone();
    host.set_waker(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    background.send(id, "number", Payload::Number(2.5));
    background.send(id, "bytes", Payload::Bytes(vec![1, 2]));
    background.send(id, "none", Payload::None);
    background.send(id, "boom", Payload::None);
    background.send(999, "unknown", Payload::None);
    assert_eq!(woken.load(Ordering::SeqCst), 5);
    assert!(host.run_background());
    assert!(!host.run_background());
    assert_eq!(
        string(&mut host, "JSON.stringify(events)"),
        r#"[["text","hi"],"microtask",["number",2.5],"microtask",["bytes",[1,2]],"microtask",["none",null],"microtask",["boom",null]]"#
    );
    assert!(lines(&output)[0].contains("handler failed"));
    let sender = background.clone();
    let late = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        sender.send(id, "late", Payload::None);
        sender.send(id, "later", Payload::None);
    });
    assert!(host.wait_background(Duration::from_secs(10)));
    late.join().unwrap();
    host.run_background();
    assert_eq!(
        string(
            &mut host,
            "events.filter((e) => e[0] && e[0].startsWith('lat')).length"
        ),
        "2"
    );
    assert!(!host.wait_background(Duration::from_millis(1)));
    run(&mut host, &format!("nui.endTask({id})"));
    background.send(id, "ended", Payload::None);
    host.run_background();
    assert_eq!(
        string(&mut host, "String(events.some((e) => e[0] === 'ended'))"),
        "false"
    );
    drop(host);
    background.send(id, "gone", Payload::None);
}

#[test]
fn randomness_comes_from_the_system() {
    let (mut host, _) = host(false);
    assert_eq!(
        string(
            &mut host,
            "(() => { const values = crypto.getRandomValues(new Uint32Array(64)); const uuid = crypto.randomUUID(); const errors = [];\
             for (const bad of [[], new DataView(new ArrayBuffer(1)), new Uint8Array(65537)]) { try { crypto.getRandomValues(bad); } catch (error) { errors.push(error.name); } }\
             return [new Set(values).size > 60, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(uuid), crypto.subtle, errors].join(); })()"
        ),
        "true,true,,TypeError,TypeError,QuotaExceededError"
    );
    assert_eq!(random_bytes(3).unwrap().len(), 3);
    assert!(random_bytes(1 << 33).unwrap_err().contains("too many"));
    let mut session = Session::new();
    let generate = mock!(
        session,
        BCryptGenRandom,
        unsafe extern "system" fn(*mut core::ffi::c_void, *mut u8, u32, u32) -> i32
    );
    generate.expect().returning(|_, _, _, _| -1);
    assert!(random_bytes(1).unwrap_err().contains("0xFFFFFFFF"));
    assert_eq!(
        string(
            &mut host,
            "(() => { try { crypto.randomUUID(); } catch (error) { return error.message; } })()"
        ),
        "the system random generator failed (0xFFFFFFFF)"
    );
}

#[test]
fn decorator_helpers_follow_typescript() {
    let (mut host, _) = host(false);
    let text = string(
        &mut host,
        r#"(() => {
        const log = [];
        const { decorate, decorateParam, decorateMetadata } = babelHelpers;
        const sealed = (target) => { log.push('class ' + target.name); };
        const replace = (target) => class extends target { extra() { return 'extra'; } };
        const loud = (target, key, descriptor) => ({ ...descriptor, value() { return 'LOUD'; } });
        const field = (target, key) => { log.push('field ' + key); };
        const param = (target, key, index) => { log.push(`param ${key} ${index}`); };
        let Service = class Service { run() { return 'quiet'; } };
        decorate([field], Service.prototype, 'name', undefined);
        decorate([loud, decorateParam(0, param), undefined], Service.prototype, 'run', null);
        Service = decorate([replace, sealed], Service);
        log.push(new Service().run(), new Service().extra(), String(decorateMetadata('design:type', String)));
        globalThis.Reflect.metadata = (key, value) => `${key}=${value.name}`;
        globalThis.Reflect.decorate = () => 'reflected';
        log.push(decorateMetadata('design:type', Number), decorate([], Service));
        delete Reflect.metadata; delete Reflect.decorate;
        return JSON.stringify(log);
        })()"#,
    );
    assert_eq!(
        text,
        r#"["field name","param run 0","class Service","LOUD","extra","undefined","design:type=Number","reflected"]"#
    );
}

/// Checks expressions against what Chromium's ICU produces.
fn expect_all(host: &mut Host, cases: &[(&str, &str)]) {
    let mut failures = Vec::new();
    for (code, expected) in cases {
        let actual = match host.eval(code, "intl.js") {
            Ok(value) => value
                .to_string(host.context())
                .unwrap()
                .to_std_string_escaped(),
            Err(error) => format!("error: {}", error.lines().next().unwrap_or_default()),
        };
        if actual != *expected {
            failures.push(format!(
                "{code}\n  expected {expected:?}\n  got      {actual:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn intl_formats_dates_like_chromium() {
    let (mut host, _) = host(false);
    run(
        &mut host,
        "globalThis.may = new Date(2026, 4, 31, 13, 5, 9, 45); globalThis.midnight = new Date(2026, 4, 31);",
    );
    expect_all(
        &mut host,
        &[
            ("midnight.toLocaleDateString('en-US')", "5/31/2026"),
            (
                "midnight.toLocaleDateString('en-US', { day: 'numeric', month: 'short', year: 'numeric' })",
                "May 31, 2026",
            ),
            (
                "midnight.toLocaleDateString('en-US', { weekday: 'long', year: 'numeric', month: 'long', day: 'numeric' })",
                "Sunday, May 31, 2026",
            ),
            (
                "midnight.toLocaleDateString('en-US', { month: '2-digit', day: '2-digit', year: '2-digit' })",
                "05/31/26",
            ),
            (
                "midnight.toLocaleDateString('en-US', { weekday: 'short', month: 'numeric', day: 'numeric' })",
                "Sun, 5/31",
            ),
            (
                "midnight.toLocaleDateString('en-US', { month: 'long' })",
                "May",
            ),
            (
                "midnight.toLocaleDateString('en-US', { weekday: 'narrow' })",
                "S",
            ),
            (
                "midnight.toLocaleDateString('en-US', { month: 'narrow', day: 'numeric' })",
                "M 31",
            ),
            (
                "midnight.toLocaleDateString('en-US', { year: 'numeric', month: 'short' })",
                "May 2026",
            ),
            (
                "midnight.toLocaleDateString('en-US', { year: 'numeric', era: 'short' })",
                "2026 AD",
            ),
            (
                "midnight.toLocaleDateString('en-US', { day: 'numeric' })",
                "31",
            ),
            (
                "may.toLocaleString('en-US')",
                "5/31/2026, 1:05:09\u{202f}PM",
            ),
            ("may.toLocaleTimeString('en-US')", "1:05:09\u{202f}PM"),
            (
                "may.toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit' })",
                "01:05\u{202f}PM",
            ),
            (
                "may.toLocaleTimeString('en-US', { hour12: false })",
                "13:05:09",
            ),
            (
                "may.toLocaleTimeString('en-US', { hourCycle: 'h11', hour: 'numeric' })",
                "1\u{202f}PM",
            ),
            (
                "midnight.toLocaleTimeString('en-US', { hourCycle: 'h24', hour: 'numeric', minute: 'numeric' })",
                "24:00",
            ),
            (
                "may.toLocaleTimeString('en-US', { minute: '2-digit', second: '2-digit', fractionalSecondDigits: 2 })",
                "05:09.04",
            ),
            (
                "may.toLocaleTimeString('en-US', { minute: 'numeric' })",
                "5",
            ),
            (
                "may.toLocaleTimeString('en-US', { second: 'numeric' })",
                "9",
            ),
            (
                "may.toLocaleTimeString('en-US-u-hc-h23', { hour: 'numeric', minute: 'numeric' })",
                "13:05",
            ),
            (
                "new Intl.DateTimeFormat('en-US', { dateStyle: 'full', timeStyle: 'short' }).format(may)",
                "Sunday, May 31, 2026 at 1:05\u{202f}PM",
            ),
            (
                "new Intl.DateTimeFormat('en-US', { dateStyle: 'medium', timeStyle: 'medium' }).format(may)",
                "May 31, 2026, 1:05:09\u{202f}PM",
            ),
            (
                "new Intl.DateTimeFormat('en-US', { dateStyle: 'short' }).format(may)",
                "5/31/26",
            ),
            (
                "new Intl.DateTimeFormat('en-US', { timeStyle: 'short', hour12: false }).format(may)",
                "13:05",
            ),
            (
                "new Intl.DateTimeFormat('en-US', { timeZone: 'UTC', timeStyle: 'full' }).format(Date.UTC(2026, 0, 1, 15))",
                "3:00:00\u{202f}PM Coordinated Universal Time",
            ),
            (
                "new Intl.DateTimeFormat('en-US', { timeZone: 'UTC', timeStyle: 'long' }).format(Date.UTC(2026, 0, 1, 15))",
                "3:00:00\u{202f}PM UTC",
            ),
            (
                "new Intl.DateTimeFormat('en-US', { timeZone: 'America/Los_Angeles', hour: 'numeric', timeZoneName: 'short' }).format(Date.UTC(2026, 6, 1, 15))",
                "8\u{202f}AM PDT",
            ),
            (
                "new Intl.DateTimeFormat('en-US', { timeZone: 'America/Los_Angeles', hour: 'numeric', timeZoneName: 'long' }).format(Date.UTC(2026, 0, 1, 15))",
                "7\u{202f}AM Pacific Standard Time",
            ),
            (
                "new Intl.DateTimeFormat('en-US', { timeZone: 'America/New_York', hour: 'numeric', timeZoneName: 'shortGeneric' }).format(Date.UTC(2026, 0, 1, 15))",
                "10\u{202f}AM ET",
            ),
            (
                "new Intl.DateTimeFormat('en-US', { timeZone: 'America/New_York', hour: 'numeric', timeZoneName: 'longGeneric' }).format(Date.UTC(2026, 0, 1, 15))",
                "10\u{202f}AM Eastern Time",
            ),
            (
                "new Intl.DateTimeFormat('en-US', { timeZone: 'asia/kolkata', hour: 'numeric', minute: '2-digit', timeZoneName: 'short' }).format(Date.UTC(2026, 0, 1, 0))",
                "5:30\u{202f}AM GMT+5:30",
            ),
            (
                "new Intl.DateTimeFormat('en-US', { timeZone: 'Europe/London', hour: 'numeric', timeZoneName: 'longOffset' }).format(Date.UTC(2026, 0, 1, 0))",
                "12\u{202f}AM GMT",
            ),
            (
                "new Intl.DateTimeFormat('en-US', { timeZone: 'Europe/London', hour: 'numeric', timeZoneName: 'shortOffset' }).format(Date.UTC(2026, 6, 1, 0))",
                "1\u{202f}AM GMT+1",
            ),
            (
                "new Intl.DateTimeFormat('en-US', { timeZone: '+05:45', hour: 'numeric', timeZoneName: 'longOffset' }).format(Date.UTC(2026, 0, 1, 0))",
                "5\u{202f}AM GMT+05:45",
            ),
            (
                "new Intl.DateTimeFormat('en-US', { timeZone: 'Etc/GMT+5', hour: 'numeric' }).format(Date.UTC(2026, 0, 1, 12))",
                "7\u{202f}AM",
            ),
            (
                "new Intl.DateTimeFormat('en-US', { timeZone: 'Mars/Base' })",
                "error: RangeError: Invalid time zone specified: Mars/Base",
            ),
            (
                "new Intl.DateTimeFormat('en-US', { dateStyle: 'full', hour: 'numeric' })",
                "error: TypeError: Can't set option hour when dateStyle is used",
            ),
            (
                "midnight.toLocaleDateString('en-US', { timeStyle: 'short' })",
                "error: TypeError: Invalid option : timeStyle",
            ),
            (
                "midnight.toLocaleTimeString('en-US', { dateStyle: 'short' })",
                "error: TypeError: Invalid option : dateStyle",
            ),
            ("new Date(NaN).toLocaleString()", "Invalid Date"),
            (
                "new Intl.DateTimeFormat('en-US').format(NaN)",
                "error: RangeError: Invalid time value",
            ),
            (
                "new Intl.DateTimeFormat('en-US', { month: 'bogus' })",
                "error: RangeError: Value bogus out of range for Intl options property month",
            ),
            (
                "JSON.stringify(new Intl.DateTimeFormat('en-US', { hour: 'numeric', minute: 'numeric', timeZone: 'UTC' }).resolvedOptions())",
                "{\"locale\":\"en-US\",\"calendar\":\"gregory\",\"numberingSystem\":\"latn\",\"timeZone\":\"UTC\",\"hourCycle\":\"h12\",\"hour12\":true,\"hour\":\"numeric\",\"minute\":\"2-digit\"}",
            ),
            (
                "JSON.stringify(new Intl.DateTimeFormat('en-US', { dateStyle: 'short', timeStyle: 'short', hour12: false, timeZone: 'UTC' }).resolvedOptions())",
                "{\"locale\":\"en-US\",\"calendar\":\"gregory\",\"numberingSystem\":\"latn\",\"timeZone\":\"UTC\",\"hourCycle\":\"h23\",\"hour12\":false,\"dateStyle\":\"short\",\"timeStyle\":\"short\"}",
            ),
            (
                "JSON.stringify(new Intl.DateTimeFormat('en-US', { hour: 'numeric', hour12: false, timeZone: 'UTC' }).resolvedOptions().hour)",
                "\"2-digit\"",
            ),
            (
                "JSON.stringify(new Intl.DateTimeFormat('en-US', { hour: 'numeric' }).formatToParts(may))",
                "[{\"type\":\"hour\",\"value\":\"1\"},{\"type\":\"literal\",\"value\":\"\u{202f}\"},{\"type\":\"dayPeriod\",\"value\":\"PM\"}]",
            ),
            (
                "JSON.stringify(new Intl.DateTimeFormat('en-US', { year: 'numeric', era: 'long' }).formatToParts(new Date(-62167219200000)))",
                "[{\"type\":\"year\",\"value\":\"1\"},{\"type\":\"literal\",\"value\":\" \"},{\"type\":\"era\",\"value\":\"Before Christ\"}]",
            ),
            (
                "new Intl.DateTimeFormat('en-US', { era: 'narrow', year: '2-digit' }).format(may)",
                "26 A",
            ),
            (
                "new Intl.DateTimeFormat('en-US').formatRange(midnight, new Date(2026, 5, 1))",
                "5/31/2026 – 6/1/2026",
            ),
            (
                "new Intl.DateTimeFormat('en-US').formatRange(midnight, may)",
                "5/31/2026",
            ),
            (
                "new Intl.DateTimeFormat('en-US').formatRangeToParts(midnight, may).length",
                "5",
            ),
            (
                "new Intl.DateTimeFormat('en-US').formatRangeToParts(midnight, new Date(2026, 5, 1)).length",
                "11",
            ),
            ("Intl.DateTimeFormat('en-US').format(midnight)", "5/31/2026"),
            (
                "Intl.DateTimeFormat.supportedLocalesOf(['en-US', 'de-DE']).join()",
                "en-US,de-DE",
            ),
            // Other locales take their patterns and names from Windows.
            ("midnight.toLocaleDateString('de-DE')", "31.5.2026"),
            (
                "midnight.toLocaleDateString('de-DE', { dateStyle: 'long' })",
                "31. Mai 2026",
            ),
            ("may.toLocaleString('de-DE')", "31.5.2026, 13:05:09"),
            (
                "midnight.toLocaleDateString('fr-FR', { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' })",
                "dimanche 31 mai 2026",
            ),
            (
                "midnight.toLocaleDateString('fr-FR', { month: 'short', day: 'numeric' })",
                "31 mai",
            ),
            (
                "midnight.toLocaleDateString('ru-RU', { day: 'numeric', month: 'long', year: 'numeric' })",
                "31 мая 2026 г.",
            ),
            ("midnight.toLocaleDateString('ja-JP')", "2026/5/31"),
            (
                "midnight.toLocaleDateString('zh-CN', { year: 'numeric', month: 'long', day: 'numeric' })",
                "2026年5月31日",
            ),
            (
                "may.toLocaleTimeString('en-GB', { hour: 'numeric', minute: '2-digit' })",
                "13:05",
            ),
            (
                "midnight.toLocaleDateString('en-GB', { month: '2-digit', year: 'numeric' })",
                "05/2026",
            ),
            (
                "new Date(Date.UTC(2026, 4, 31, 11, 5, 9, 45)).toLocaleTimeString('de-DE', { timeZone: 'Etc/GMT-2', hour: 'numeric', minute: 'numeric', second: 'numeric', fractionalSecondDigits: 1, timeZoneName: 'short' })",
                "13:05:09,0 GMT+2",
            ),
            (
                "new Intl.DateTimeFormat('de-DE', { dateStyle: 'short', timeStyle: 'full', timeZone: 'Etc/GMT-2' }).format(Date.UTC(2026, 4, 31, 11, 5, 9))",
                "31.05.26, 13:05:09 GMT+02:00",
            ),
            (
                "new Intl.DateTimeFormat('xx').resolvedOptions().locale === new Intl.DateTimeFormat().resolvedOptions().locale",
                "true",
            ),
        ],
    );
}

#[test]
fn intl_formats_numbers_like_chromium() {
    let (mut host, _) = host(false);
    expect_all(
        &mut host,
        &[
            ("(1234567.891).toLocaleString('en-US')", "1,234,567.891"),
            (
                "(1000).toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })",
                "1,000.00",
            ),
            ("(-0).toLocaleString('en-US')", "-0"),
            (
                "(1e21).toLocaleString('en-US')",
                "1,000,000,000,000,000,000,000",
            ),
            (
                "(1.5e-7).toLocaleString('en-US', { maximumSignificantDigits: 2 })",
                "0.00000015",
            ),
            (
                "(0).toLocaleString('en-US', { minimumSignificantDigits: 3 })",
                "0.00",
            ),
            (
                "(12).toLocaleString('en-US', { minimumSignificantDigits: 3 })",
                "12.0",
            ),
            (
                "(9.995).toLocaleString('en-US', { maximumFractionDigits: 2 })",
                "10",
            ),
            (
                "(1.005).toLocaleString('en-US', { maximumFractionDigits: 2 })",
                "1.01",
            ),
            (
                "(5).toLocaleString('en-US', { minimumIntegerDigits: 3 })",
                "005",
            ),
            (
                "(1234).toLocaleString('en-US', { useGrouping: false })",
                "1234",
            ),
            (
                "(1234).toLocaleString('en-US', { useGrouping: 'min2' })",
                "1234",
            ),
            (
                "(12345).toLocaleString('en-US', { useGrouping: 'min2' })",
                "12,345",
            ),
            (
                "(1234.5).toLocaleString('en-US', { maximumFractionDigits: 0 })",
                "1,235",
            ),
            (
                "NaN.toLocaleString('en-US') + ' ' + (-Infinity).toLocaleString('en-US')",
                "NaN -∞",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD' }).format(-1234.5)",
                "-$1,234.50",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'currency', currency: 'usd', currencySign: 'accounting' }).format(-1234.5)",
                "($1,234.50)",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', currencySign: 'accounting' }).format(5)",
                "$5.00",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'currency', currency: 'EUR', currencyDisplay: 'code' }).format(12)",
                "EUR\u{a0}12.00",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'currency', currency: 'CAD', currencyDisplay: 'narrowSymbol' }).format(1)",
                "$1.00",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'currency', currency: 'CAD' }).format(1)",
                "CA$1.00",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', currencyDisplay: 'name' }).format(1)",
                "1.00 US dollars",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'currency', currency: 'EUR', currencyDisplay: 'name', maximumFractionDigits: 0 }).format(1)",
                "1 euro",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'currency', currency: 'XYZ' }).format(1)",
                "XYZ\u{a0}1.00",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'currency', currency: 'XYZ', currencyDisplay: 'name' }).format(2)",
                "2.00 XYZ",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'currency', currency: 'JPY' }).format(1234.5)",
                "¥1,235",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'currency', currency: 'KWD' }).format(1)",
                "KWD\u{a0}1.000",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', notation: 'compact' }).format(1234567)",
                "$1.2M",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'percent' }).format(0.256)",
                "26%",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'percent', minimumFractionDigits: 1 }).format(-0.256)",
                "-25.6%",
            ),
            (
                "new Intl.NumberFormat('en-US', { notation: 'compact' }).format(1234)",
                "1.2K",
            ),
            (
                "new Intl.NumberFormat('en-US', { notation: 'compact' }).format(999999)",
                "1M",
            ),
            (
                "new Intl.NumberFormat('en-US', { notation: 'compact' }).format(123456)",
                "123K",
            ),
            (
                "new Intl.NumberFormat('en-US', { notation: 'compact' }).format(12.34)",
                "12",
            ),
            (
                "new Intl.NumberFormat('en-US', { notation: 'compact' }).format(0)",
                "0",
            ),
            (
                "new Intl.NumberFormat('en-US', { notation: 'compact' }).format(1.5e15)",
                "1500T",
            ),
            (
                "new Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 2 }).format(1234)",
                "1.23K",
            ),
            (
                "new Intl.NumberFormat('en-US', { notation: 'compact', compactDisplay: 'long' }).format(12345)",
                "12 thousand",
            ),
            (
                "new Intl.NumberFormat('en-US', { notation: 'scientific' }).format(123456)",
                "1.235E5",
            ),
            (
                "new Intl.NumberFormat('en-US', { notation: 'scientific' }).format(0.00012)",
                "1.2E-4",
            ),
            (
                "new Intl.NumberFormat('en-US', { notation: 'scientific' }).format(9.9999)",
                "1E1",
            ),
            (
                "new Intl.NumberFormat('en-US', { notation: 'engineering' }).format(123456)",
                "123.456E3",
            ),
            (
                "new Intl.NumberFormat('en-US', { notation: 'engineering' }).format(999.9999)",
                "1E3",
            ),
            (
                "new Intl.NumberFormat('en-US', { maximumSignificantDigits: 3 }).format(123456)",
                "123,000",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'unit', unit: 'kilobyte' }).format(1.5)",
                "1.5 kB",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'unit', unit: 'day', unitDisplay: 'long' }).format(1)",
                "1 day",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'unit', unit: 'percent' }).format(5)",
                "5%",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'unit', unit: 'celsius', unitDisplay: 'narrow' }).format(20)",
                "20°C",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'unit', unit: 'kilometer-per-hour', unitDisplay: 'long' }).format(50)",
                "50 kilometers per hour",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'unit', unit: 'kilometer-per-hour' }).format(50)",
                "50 km/h",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'unit', unit: 'meter-per-second' }).format(3)",
                "3 m/sec",
            ),
            (
                "new Intl.NumberFormat('en-US', { signDisplay: 'always' }).format(5)",
                "+5",
            ),
            (
                "new Intl.NumberFormat('en-US', { signDisplay: 'exceptZero' }).format(0) + new Intl.NumberFormat('en-US', { signDisplay: 'exceptZero' }).format(-1)",
                "0-1",
            ),
            (
                "new Intl.NumberFormat('en-US', { signDisplay: 'never' }).format(-1)",
                "1",
            ),
            (
                "new Intl.NumberFormat('en-US', { signDisplay: 'negative' }).format(-0)",
                "0",
            ),
            (
                "new Intl.NumberFormat('en-US', { signDisplay: 'always' }).format(NaN)",
                "+NaN",
            ),
            (
                "(12345678901234567890n).toLocaleString('en-US')",
                "12,345,678,901,234,567,890",
            ),
            (
                "new Intl.NumberFormat('en-US').format('12345678901234567890')",
                "12,345,678,901,234,567,890",
            ),
            ("new Intl.NumberFormat('en-US').format('1.5')", "1.5"),
            (
                "JSON.stringify(new Intl.NumberFormat('en-US').formatToParts(-1234.5))",
                "[{\"type\":\"minusSign\",\"value\":\"-\"},{\"type\":\"integer\",\"value\":\"1\"},{\"type\":\"group\",\"value\":\",\"},{\"type\":\"integer\",\"value\":\"234\"},{\"type\":\"decimal\",\"value\":\".\"},{\"type\":\"fraction\",\"value\":\"5\"}]",
            ),
            (
                "new Intl.NumberFormat('en-US').formatRange(1, 5) + ' ' + new Intl.NumberFormat('en-US').formatRange(3, 3)",
                "1–5 ~3",
            ),
            (
                "new Intl.NumberFormat('en-US').formatRange(1)",
                "error: TypeError: start or end is undefined",
            ),
            (
                "JSON.stringify(new Intl.NumberFormat('en-US', { style: 'currency', currency: 'EUR' }).resolvedOptions())",
                "{\"locale\":\"en-US\",\"numberingSystem\":\"latn\",\"style\":\"currency\",\"currency\":\"EUR\",\"currencyDisplay\":\"symbol\",\"currencySign\":\"standard\",\"minimumIntegerDigits\":1,\"minimumFractionDigits\":2,\"maximumFractionDigits\":2,\"useGrouping\":\"auto\",\"notation\":\"standard\",\"signDisplay\":\"auto\",\"roundingIncrement\":1,\"roundingMode\":\"halfExpand\",\"roundingPriority\":\"auto\",\"trailingZeroDisplay\":\"auto\"}",
            ),
            (
                "JSON.stringify(new Intl.NumberFormat('en-US', { style: 'unit', unit: 'byte', notation: 'compact', maximumSignificantDigits: 2 }).resolvedOptions())",
                "{\"locale\":\"en-US\",\"numberingSystem\":\"latn\",\"style\":\"unit\",\"unit\":\"byte\",\"unitDisplay\":\"short\",\"minimumIntegerDigits\":1,\"minimumSignificantDigits\":1,\"maximumSignificantDigits\":2,\"useGrouping\":\"min2\",\"notation\":\"compact\",\"compactDisplay\":\"short\",\"signDisplay\":\"auto\",\"roundingIncrement\":1,\"roundingMode\":\"halfExpand\",\"roundingPriority\":\"auto\",\"trailingZeroDisplay\":\"auto\"}",
            ),
            (
                "new Intl.NumberFormat('en-US', { useGrouping: true }).resolvedOptions().useGrouping + ' ' + new Intl.NumberFormat('en-US', { useGrouping: 'always' }).resolvedOptions().useGrouping",
                "always always",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'currency' })",
                "error: TypeError: Currency code is required with currency style.",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'currency', currency: 'US' })",
                "error: RangeError: Invalid currency code : US",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'unit' })",
                "error: TypeError: Unit is required with unit style.",
            ),
            (
                "new Intl.NumberFormat('en-US', { style: 'unit', unit: 'furlong' })",
                "error: RangeError: Invalid unit argument for Intl.NumberFormat() 'furlong'",
            ),
            (
                "new Intl.NumberFormat('en-US', { maximumFractionDigits: 101 })",
                "error: RangeError: maximumFractionDigits value is out of range.",
            ),
            (
                "new Intl.NumberFormat('en-US', { minimumFractionDigits: 3, maximumFractionDigits: 1 })",
                "error: RangeError: maximumFractionDigits value is out of range.",
            ),
            (
                "new Intl.NumberFormat('en-US', null)",
                "error: TypeError: Cannot convert undefined or null to object",
            ),
            (
                "new Intl.NumberFormat('en-US', { useGrouping: 'sometimes' })",
                "error: RangeError: Value sometimes out of range for Intl options property useGrouping",
            ),
            (
                "new Intl.NumberFormat('en-US', { minimumFractionDigits: 1 }).format(1)",
                "1.0",
            ),
            (
                "new Intl.NumberFormat('en-US', { maximumFractionDigits: 1, style: 'currency', currency: 'USD' }).format(1.25)",
                "$1.3",
            ),
            (
                "new Intl.NumberFormat('en-US', { useGrouping: '' }).format(12345)",
                "12345",
            ),
            // Other locales take their separators and patterns from Windows.
            (
                "new Intl.NumberFormat('de-DE', { style: 'currency', currency: 'EUR' }).format(1234.5)",
                "1.234,50\u{a0}€",
            ),
            (
                "new Intl.NumberFormat('de-DE', { style: 'currency', currency: 'USD' }).format(1)",
                "1,00\u{a0}$",
            ),
            (
                "new Intl.NumberFormat('fr-FR').format(1234567.89)",
                "1\u{202f}234\u{202f}567,89",
            ),
            (
                "new Intl.NumberFormat('de-DE', { style: 'percent' }).format(0.5)",
                "50\u{a0}%",
            ),
            (
                "new Intl.NumberFormat('en-IN').format(1234567.89)",
                "12,34,567.89",
            ),
            (
                "new Intl.NumberFormat('ja-JP', { style: 'currency', currency: 'JPY' }).format(1234)",
                "￥1,234",
            ),
        ],
    );
}

#[test]
fn intl_plurals_lists_relative_times_and_text() {
    let (mut host, _) = host(false);
    expect_all(
        &mut host,
        &[
            (
                "[0, 1, 2, 1.5].map((n) => new Intl.PluralRules('en-US').select(n)).join()",
                "other,one,other,other",
            ),
            (
                "new Intl.PluralRules('en-US', { minimumFractionDigits: 1 }).select(1)",
                "other",
            ),
            (
                "[1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 1.5].map((n) => new Intl.PluralRules('en-US', { type: 'ordinal' }).select(n)).join()",
                "one,two,few,other,other,other,other,one,two,few,other",
            ),
            (
                "new Intl.PluralRules('de', { type: 'ordinal' }).select(1)",
                "other",
            ),
            (
                "[0, 1, 2, 1.5].map((n) => new Intl.PluralRules('fr').select(n)).join()",
                "one,one,other,one",
            ),
            (
                "[1, 2, 5, 21, 1.5].map((n) => new Intl.PluralRules('ru').select(n)).join()",
                "one,few,many,one,other",
            ),
            (
                "[1, 2, 5, 22, 1.5].map((n) => new Intl.PluralRules('pl').select(n)).join()",
                "one,few,many,few,other",
            ),
            (
                "[1, 3, 5, 1.5].map((n) => new Intl.PluralRules('cs').select(n)).join()",
                "one,few,other,many",
            ),
            (
                "[0, 1, 2, 3, 11, 100].map((n) => new Intl.PluralRules('ar').select(n)).join()",
                "zero,one,two,few,many,other",
            ),
            (
                "[1, 2, 3].map((n) => new Intl.PluralRules('he').select(n)).join()",
                "one,two,other",
            ),
            (
                "[1, 2, 11, 1.5].map((n) => new Intl.PluralRules('lt').select(n)).join()",
                "one,few,other,many",
            ),
            (
                "[0, 1, 2].map((n) => new Intl.PluralRules('pt').select(n)).join() + ' ' + new Intl.PluralRules('pt-PT').select(0)",
                "one,one,other other",
            ),
            (
                "[1, 2].map((n) => new Intl.PluralRules('es').select(n)).join() + ' ' + new Intl.PluralRules('ja').select(1)",
                "one,other other",
            ),
            (
                "new Intl.PluralRules('en').select(Infinity) + new Intl.PluralRules('en').selectRange(1, 2)",
                "otherother",
            ),
            (
                "new Intl.PluralRules('en-US').resolvedOptions().pluralCategories.join()",
                "one,other",
            ),
            (
                "new Intl.PluralRules('en-US', { type: 'ordinal' }).resolvedOptions().pluralCategories.join()",
                "few,one,two,other",
            ),
            (
                "new Intl.PluralRules('ru').resolvedOptions().pluralCategories.join() + ' ' + new Intl.PluralRules('de', { type: 'ordinal' }).resolvedOptions().pluralCategories.join()",
                "one,few,many,other other",
            ),
            (
                "new Intl.PluralRules('en', { maximumSignificantDigits: 2 }).resolvedOptions().maximumSignificantDigits",
                "2",
            ),
            (
                "new Intl.RelativeTimeFormat('en', { numeric: 'auto' }).format(-1, 'day')",
                "yesterday",
            ),
            (
                "new Intl.RelativeTimeFormat('en', { numeric: 'auto' }).format(0, 'second')",
                "now",
            ),
            (
                "new Intl.RelativeTimeFormat('en', { numeric: 'auto', style: 'short' }).format(1, 'year')",
                "next yr.",
            ),
            (
                "new Intl.RelativeTimeFormat('en', { numeric: 'auto' }).format(-2, 'day')",
                "2 days ago",
            ),
            (
                "new Intl.RelativeTimeFormat('en', { numeric: 'auto' }).format(1, 'hour')",
                "in 1 hour",
            ),
            (
                "new Intl.RelativeTimeFormat('en').format(3, 'hours')",
                "in 3 hours",
            ),
            (
                "new Intl.RelativeTimeFormat('en').format(-0, 'day')",
                "0 days ago",
            ),
            (
                "new Intl.RelativeTimeFormat('en').format(1234, 'second')",
                "in 1,234 seconds",
            ),
            (
                "new Intl.RelativeTimeFormat('en', { style: 'short' }).format(-2, 'month')",
                "2 mo. ago",
            ),
            (
                "new Intl.RelativeTimeFormat('en', { style: 'narrow' }).format(3, 'hour') + ' ' + new Intl.RelativeTimeFormat('en', { style: 'narrow' }).format(-1, 'day')",
                "in 3h 1 day ago",
            ),
            (
                "new Intl.RelativeTimeFormat('en').format(1, 'fortnight')",
                "error: RangeError: Invalid unit argument for format() 'fortnight'",
            ),
            (
                "new Intl.RelativeTimeFormat('en').format(NaN, 'day')",
                "error: RangeError: Invalid time value",
            ),
            (
                "JSON.stringify(new Intl.RelativeTimeFormat('en').formatToParts(10, 'day'))",
                "[{\"type\":\"literal\",\"value\":\"in \"},{\"type\":\"integer\",\"value\":\"10\",\"unit\":\"day\"},{\"type\":\"literal\",\"value\":\" days\"}]",
            ),
            (
                "JSON.stringify(new Intl.RelativeTimeFormat('en').resolvedOptions())",
                "{\"locale\":\"en\",\"style\":\"long\",\"numeric\":\"always\",\"numberingSystem\":\"latn\"}",
            ),
            (
                "new Intl.ListFormat('en').format(['a', 'b', 'c'])",
                "a, b, and c",
            ),
            ("new Intl.ListFormat('en').format(['a', 'b'])", "a and b"),
            (
                "new Intl.ListFormat('en', { style: 'short' }).format(['a', 'b', 'c'])",
                "a, b, & c",
            ),
            (
                "new Intl.ListFormat('en', { style: 'narrow' }).format(['a', 'b', 'c'])",
                "a, b, c",
            ),
            (
                "new Intl.ListFormat('en', { type: 'disjunction' }).format(['a', 'b', 'c'])",
                "a, b, or c",
            ),
            (
                "new Intl.ListFormat('en', { type: 'unit', style: 'narrow' }).format(['5 lb', '12 oz'])",
                "5 lb 12 oz",
            ),
            (
                "new Intl.ListFormat('en', { type: 'unit' }).format(['5 lb', '12 oz'])",
                "5 lb, 12 oz",
            ),
            (
                "new Intl.ListFormat('en').format([]) + new Intl.ListFormat('en').format(['x'])",
                "x",
            ),
            (
                "new Intl.ListFormat('en').format([1])",
                "error: TypeError: Iterable yielded 1 which is not a string",
            ),
            (
                "JSON.stringify(new Intl.ListFormat('en').resolvedOptions())",
                "{\"locale\":\"en\",\"type\":\"conjunction\",\"style\":\"long\"}",
            ),
            (
                "['b', 'a', 'B', 'A', 'résumé', 'resume', 'Résumé', 'file10', 'file2', '10', '9', ' x', '-x'].sort(new Intl.Collator('en').compare).join('|')",
                " x|-x|10|9|a|A|b|B|file10|file2|resume|résumé|Résumé",
            ),
            (
                "['file10', 'file2', 'file02'].sort(new Intl.Collator('en', { numeric: true }).compare).join()",
                "file2,file02,file10",
            ),
            (
                "['file10', 'file2'].sort(new Intl.Collator('en-u-kn').compare).join()",
                "file2,file10",
            ),
            (
                "['a', 'A'].sort(new Intl.Collator('en', { caseFirst: 'upper' }).compare).join()",
                "A,a",
            ),
            (
                "[new Intl.Collator('en', { sensitivity: 'base' }).compare('a', 'á'), new Intl.Collator('en', { sensitivity: 'accent' }).compare('a', 'á'), new Intl.Collator('en', { sensitivity: 'case' }).compare('a', 'á'), new Intl.Collator('en', { sensitivity: 'case' }).compare('a', 'A')].join()",
                "0,-1,0,-1",
            ),
            (
                "new Intl.Collator('en', { ignorePunctuation: true }).compare('a-b', 'ab')",
                "0",
            ),
            ("new Intl.Collator('en').compare('ab', 'abc')", "-1"),
            (
                "new Intl.Collator('en').compare('\\u0301a', 'a') !== 0",
                "true",
            ),
            (
                "JSON.stringify(Intl.Collator('en').resolvedOptions())",
                "{\"locale\":\"en\",\"usage\":\"sort\",\"sensitivity\":\"variant\",\"ignorePunctuation\":false,\"collation\":\"default\",\"numeric\":false,\"caseFirst\":\"false\"}",
            ),
            (
                "'a'.localeCompare('B') + ',' + 'a'.localeCompare('A', undefined, { sensitivity: 'base' }) + ',' + 'b'.localeCompare('a', 'en')",
                "-1,0,1",
            ),
            (
                "String.prototype.localeCompare.call(null, 'a')",
                "error: TypeError: String.prototype.localeCompare called on null or undefined",
            ),
            (
                "'istanbul'.toLocaleUpperCase('tr-TR') + ' ' + 'ISTANBUL'.toLocaleLowerCase('tr') + ' ' + 'i'.toLocaleUpperCase('en') + 'I'.toLocaleLowerCase()",
                "İSTANBUL ıstanbul Ii",
            ),
            ("'x'.toLocaleUpperCase('!!')", "X"),
            (
                "[...new Intl.Segmenter('en', { granularity: 'word' }).segment('Hello, world!')].filter((s) => s.isWordLike).map((s) => s.segment).join('|')",
                "Hello|world",
            ),
            (
                "[...new Intl.Segmenter('en', { granularity: 'word' }).segment('Hi there')].map((s) => s.index + ':' + s.segment).join('|')",
                "0:Hi|2: |3:there",
            ),
            (
                "[...new Intl.Segmenter().segment('e\\u0301👍🏽🇺🇸👨‍👩‍👧\\r\\n')].map((s) => s.segment.length).join()",
                "2,4,4,8,2",
            ),
            (
                "[...new Intl.Segmenter('en', { granularity: 'sentence' }).segment('One. Two? Three')].map((s) => s.segment).join('|')",
                "One. |Two? |Three",
            ),
            (
                "new Intl.Segmenter().segment('abc').containing(1).segment + new Intl.Segmenter().segment('abc').containing(9)",
                "bundefined",
            ),
            (
                "new Intl.Segmenter().segment('abc').containing().index",
                "0",
            ),
            (
                "JSON.stringify(new Intl.Segmenter('en', { granularity: 'word' }).resolvedOptions())",
                "{\"locale\":\"en\",\"granularity\":\"word\"}",
            ),
            (
                "new Intl.Locale('en-us').maximize().toString()",
                "en-Latn-US",
            ),
            ("new Intl.Locale('xx').maximize().toString()", "xx"),
            (
                "new Intl.Locale('zh-Hant-TW-u-ca-gregory-hc-h23-kn').toString()",
                "zh-Hant-TW-u-ca-gregory-hc-h23-kn",
            ),
            (
                "const l = new Intl.Locale('en-Latn-US-u-co-phonebk-kf-upper-nu-latn', { region: 'gb', hourCycle: 'h12', numeric: true }); [l.language, l.script, l.region, l.baseName, l.calendar, l.collation, l.hourCycle, l.caseFirst, l.numeric, l.numberingSystem, l.minimize().toString()].join()",
                "en,Latn,GB,en-Latn-GB,,phonebk,h12,upper,true,latn,en-u-co-phonebk-hc-h12-kf-upper-kn-nu-latn",
            ),
            (
                "new Intl.Locale(new Intl.Locale('fr-CA'), { language: 'en', script: 'Latn' }).baseName",
                "en-Latn-CA",
            ),
            (
                "new Intl.Locale()",
                "error: TypeError: First argument to Intl.Locale constructor can't be empty or missing",
            ),
            ("new Intl.Locale('en', { numeric: false }).numeric", "false"),
            (
                "Intl.getCanonicalLocales(['EN-us', 'zh-hant-tw', 'en_us', 'de-ch-1996']).join()",
                "en-US,zh-Hant-TW,de-CH-1996",
            ),
            (
                "Intl.getCanonicalLocales('EN').join() + Intl.getCanonicalLocales().length",
                "en0",
            ),
            (
                "Intl.getCanonicalLocales('not a tag')",
                "error: RangeError: Incorrect locale information provided",
            ),
            (
                "Intl.getCanonicalLocales([5])",
                "error: TypeError: Language ID should be string or object.",
            ),
            (
                "new Intl.DisplayNames(['en'], { type: 'region' }).of('us')",
                "United States",
            ),
            (
                "new Intl.DisplayNames(['en'], { type: 'language' }).of('fr-CA')",
                "French",
            ),
            (
                "new Intl.DisplayNames(['en'], { type: 'language' }).of('en-US')",
                "American English",
            ),
            (
                "new Intl.DisplayNames(['en'], { type: 'currency' }).of('eur')",
                "Euro",
            ),
            (
                "new Intl.DisplayNames(['en'], { type: 'region' }).of('ZZ') + ' ' + new Intl.DisplayNames(['en'], { type: 'script', fallback: 'none' }).of('Latn')",
                "ZZ undefined",
            ),
            (
                "new Intl.DisplayNames(['en'], { type: 'calendar' }).of('gregory')",
                "gregory",
            ),
            (
                "new Intl.DisplayNames(['en'], {})",
                "error: TypeError: Required option 'type' is missing",
            ),
            (
                "JSON.stringify(new Intl.DisplayNames(['en'], { type: 'region' }).resolvedOptions())",
                "{\"locale\":\"en\",\"style\":\"long\",\"type\":\"region\",\"fallback\":\"code\"}",
            ),
            (
                "['calendar', 'collation', 'numberingSystem'].map((key) => Intl.supportedValuesOf(key).join()).join(' ')",
                "gregory default latn",
            ),
            (
                "Intl.supportedValuesOf('currency').includes('USD') && Intl.supportedValuesOf('unit').includes('kilobyte') && Intl.supportedValuesOf('timeZone').includes('America/New_York') && !Intl.supportedValuesOf('timeZone').includes('UTC')",
                "true",
            ),
            (
                "Intl.supportedValuesOf('colors')",
                "error: RangeError: Invalid key : colors",
            ),
            (
                "Intl.NumberFormat.supportedLocalesOf('en-US').join() + Intl.PluralRules.supportedLocalesOf(['xx', 'de']).join()",
                "en-USde",
            ),
            (
                "Object.prototype.toString.call(Intl) + Object.prototype.toString.call(new Intl.Collator())",
                "[object Intl][object Intl.Collator]",
            ),
            (
                "[1234.5, new Date(Date.UTC(2026, 0, 1)), null].toLocaleString('en-US', { timeZone: 'UTC' })",
                "1,234.5,1/1/2026, 12:00:00\u{202f}AM,",
            ),
            ("(1234n).toLocaleString('de-DE')", "1.234"),
        ],
    );
}

#[test]
fn intl_uses_the_default_locale() {
    let (mut host, _) = host(false);
    crate::intl::set_default_locale(Some("de-DE".to_owned()));
    let german = string(
        &mut host,
        "new Intl.NumberFormat().resolvedOptions().locale + ' ' + (1234.5).toLocaleString() + ' ' + new Date(2026, 4, 31).toLocaleDateString() + ' ' + ['b', 'a'].sort((x, y) => x.localeCompare(y)).join()",
    );
    crate::intl::set_default_locale(Some("not a locale".to_owned()));
    let fallback = string(
        &mut host,
        "new Intl.NumberFormat().resolvedOptions().locale",
    );
    crate::intl::set_default_locale(None);
    // The host functions the runtime calls.
    let object = host.host_object();
    let mut call = |name: &str, args: &[JsValue]| {
        let function = object.get(JsString::from(name), host.context()).unwrap();
        let value = function
            .as_callable()
            .unwrap()
            .call(&JsValue::undefined(), args, host.context())
            .unwrap();
        value
            .to_string(host.context())
            .unwrap()
            .to_std_string_escaped()
    };
    call("setLocale", &[JsValue::from(JsString::from("ja-JP"))]);
    let japanese = call("locale", &[]);
    call("setLocale", &[JsValue::null()]);
    assert_eq!(japanese, "ja-JP");
    assert_eq!(call("locale", &[]), call("systemLocale", &[]));
    assert_eq!(german, "de-DE 1.234,5 31.5.2026 a,b");
    assert_eq!(fallback, "en-US");
    assert_eq!(crate::intl::default_locale(), {
        let mut buffer = [0u16; 85];
        // SAFETY: the buffer holds `buffer.len()` characters.
        let length = unsafe {
            windows_sys::Win32::Globalization::GetUserDefaultLocaleName(
                buffer.as_mut_ptr(),
                buffer.len() as i32,
            )
        };
        String::from_utf16_lossy(&buffer[..length as usize - 1])
    });
}
