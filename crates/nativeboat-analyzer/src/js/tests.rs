use super::*;
use crate::catalog::Support;
use std::path::Path;

#[test]
fn drag_and_drop_is_detected_in_dynamic_markup_and_event_handlers() {
    for source in [
        "row.draggable=true",
        "row.addEventListener('dragstart',()=>{})",
        "row.addEventListener('drop',()=>{})",
        "row.ondragover=e=>e.preventDefault()",
        "event.dataTransfer.files",
        "new DataTransfer()",
        "new DragEvent('drop')",
    ] {
        assert!(
            ids(source, SourceType::mjs()).contains(&"web:drag-drop".to_owned()),
            "{source}"
        );
    }
    assert!(
        !ids("row.addEventListener('click',()=>{})", SourceType::mjs())
            .contains(&"web:drag-drop".to_owned())
    );
}

fn ids(source: &str, source_type: SourceType) -> Vec<String> {
    detect(source, source_type)
        .into_iter()
        .map(|finding| finding.feature.id)
        .collect()
}

#[test]
fn module_namespaces_follow_symbols_through_shadowed_scopes() {
    for source in [
        "import cp from 'node:child_process'; function shadow(cp) { cp.fork('fake'); const {fork}=cp; return cp; } cp.spawn('real');",
        "const cp=require('child_process'); { const cp={fork(){}}; cp.fork(); const {fork}=cp; } cp.spawn('real');",
        "import * as electron from 'electron'; function shadow(electron) { electron.MissingApi(); const {MissingApi}=electron; } electron.app.quit();",
        "const electron=require('electron'); { let electron={MissingApi(){}}; electron.MissingApi(); const {MissingApi}=electron; } electron.app.quit();",
        "import cp = require('child_process'); function shadow(cp:any) { cp.fork(); } cp.spawn('real');",
    ] {
        let found = detect(source, SourceType::ts());
        assert!(
            !found
                .iter()
                .any(|finding| finding.feature.support == Support::Unsupported),
            "{source}: {found:?}"
        );
        assert!(
            found.iter().any(|finding| matches!(
                finding.feature.id.as_str(),
                "node:child_process" | "electron:app"
            )),
            "{source}: {found:?}"
        );
    }
    let found = ids(
        "import cp from 'child_process'; function nested(){ const cp=require('electron'); cp.app.quit(); } cp.spawn('real');",
        SourceType::mjs(),
    );
    assert_eq!(found, ["electron:app", "node:child_process"]);
}

#[test]
fn later_module_declarations_are_known_inside_earlier_functions() {
    for source in [
        "function launch(){ return cp.fork('worker.js'); } import cp from 'child_process';",
        "function launch(){ return cp.fork('worker.js'); } const cp=require('child_process');",
        "function launch(){ const {fork}=cp; return fork('worker.js'); } import * as cp from 'child_process';",
    ] {
        let found = detect(source, SourceType::mjs());
        assert!(
            found
                .iter()
                .any(|finding| finding.feature.id == "node:child_process.fork"
                    && finding.feature.support == Support::Partial),
            "{source}: {found:?}"
        );
    }
    assert_eq!(
        ids(
            "function work(){ return electron.clipboard.readText(); } import electron from 'electron';",
            SourceType::mjs()
        ),
        ["electron:clipboard"]
    );
    assert_eq!(
        ids(
            "function shadow(cp){ return cp.fork(); } const cp=require('child_process');",
            SourceType::mjs()
        ),
        Vec::<String>::new()
    );
}

#[test]
fn variables_assigned_a_module_judge_its_members() {
    // undici: `let crypto; ... crypto = require('node:crypto')`.
    let found = ids(
        "let crypto; let electron;\n\
         try { crypto = require('node:crypto'); electron = require('electron'); } catch {}\n\
         crypto.getHashes(); crypto.createSign('sha256'); electron.app.quit();",
        SourceType::cjs(),
    );
    assert!(found.contains(&"node:crypto".to_owned()), "{found:?}");
    assert!(
        found.contains(&"node:crypto.createSign".to_owned()),
        "{found:?}"
    );
    assert!(found.contains(&"electron:app".to_owned()), "{found:?}");
}

#[test]
fn electron_and_node_usage() {
    let found = ids(
        "import { app, BrowserWindow as Window, 'dialog' as d } from 'electron';\n\
         import * as all from 'electron/main';\n\
         import electronDefault from 'electron/renderer';\n\
         import fs from 'node:fs';\nimport React from 'react';\n\
         export { join } from 'path';\nexport * from './local';\n\
         const { ipcMain, shell: openShell, ...rest } = require('electron');\n\
         const electron = require('electron');\n\
         const [first] = require('electron');\n\
         const os = require('os');\nconst lodash = require('lodash');\nconst nothing = require('electron/common');\n\
         require(dynamicName);\nconst later = import('child_process');\nconst local = import(computed);\n\
         electron.clipboard.writeText('x'); all.screen; electronDefault.webFrame; other.app;\n\
         const plain = notRequire('x');\nconst noInit;\n",
        SourceType::mjs(),
    );
    assert_eq!(
        found,
        vec![
            "electron:app",
            "electron:BrowserWindow",
            "electron:dialog",
            "node:fs",
            "framework:react",
            "node:path",
            "electron:ipcMain",
            "electron:shell",
            "node:os",
            "language:dynamic-require",
            "node:child_process",
            "electron:clipboard",
            "electron:screen",
            "electron:webFrame",
        ]
    );
}

#[test]
fn types_namespaces_and_exports_of_partial_modules() {
    let found = ids(
        "import type { BrowserWindowConstructorOptions } from 'electron';\n\
         import { type Readable, type Writable } from 'node:stream';\n\
         import { createInterface, type Interface } from 'node:readline';\n\
         import { spawn, fork } from 'node:child_process';\n\
         import electron from 'electron';\n\
         import electronMain = require('electron/main');\n\
         import type types = require('electron');\n\
         import fs = require('fs');\n\
         import zlib from 'zlib';\n\
         import * as net from 'net';\n\
         import 'stream';\n\
         import {} from 'dns';\n\
         import Alias = Some.Namespace;\n\
         export type { Socket } from 'net';\n\
         export { type Server } from 'net';\n\
         export { request } from 'https';\n\
         export * from 'http';\n\
         export type * from 'http2';\n\
         const { app, BrowserWindow } = electron;\n\
         const { shell } = electronMain;\n\
         const { gzipSync, brotliCompressSync } = zlib;\n\
         const { lookup } = require('dns');\n\
         const cp = require('child_process');\n\
         const { join } = require('path');\n\
         const { pick } = require('lodash');\n\
         const lodash = require('lodash');\n\
         const plain = { notElectron: 1 };\n\
         const { x } = plain;\n\
         cp.execFile('a'); net.connect(1); passAround(net);\n\
         require('http').createServer(); require('module').register(); require('fs').readFileSync('x');\n\
         let typed: cp.ChildProcess;\nfunction f(options: Electron.BrowserWindowConstructorOptions) {}\n",
        SourceType::ts(),
    );
    assert_eq!(
        found,
        vec![
            "node:readline",
            "node:child_process",
            "node:child_process.fork",
            "node:fs",
            "node:stream",
            "node:dns",
            "node:https",
            "node:http",
            "electron:app",
            "electron:BrowserWindow",
            "electron:shell",
            "node:zlib",
            "node:zlib.brotliCompressSync",
            "node:dns",
            "node:path",
            "node:child_process",
            "node:net",
            "node:net",
            "node:http",
            "node:module.register",
            "node:fs",
        ]
    );
    let supports: Vec<Support> = detect(
        "import { spawn } from 'child_process';\nimport * as net from 'net';\nnet.isIP(1); use(net);\n",
        SourceType::ts(),
    )
    .into_iter()
    .map(|finding| finding.feature.support)
    .collect();
    assert_eq!(
        supports,
        vec![Support::Supported, Support::Supported, Support::Partial]
    );
}

#[test]
fn canvas_contexts_and_editing_commands_are_judged_by_their_use() {
    let judged = |source: &str| {
        detect(source, SourceType::mjs())
            .into_iter()
            .map(|finding| (finding.feature.id, finding.feature.support))
            .collect::<Vec<_>>()
    };
    use crate::catalog::Support::{Partial, Supported};
    // Drawing that the native canvas supports, and resets to defaults.
    assert_eq!(
        judged(
            "const ctx = canvas.getContext('2d'); ctx.fillStyle = 'red'; ctx.drawImage(img, 0, 0);\n\
             ctx.shadowBlur = 0; ctx.shadowColor = 'transparent'; ctx.globalCompositeOperation = 'copy';\n\
             ctx.filter = 'none'; list.filter = 'all'; editor.execCommand('bold');"
        ),
        vec![("web:canvas-2d".to_owned(), Supported)]
    );
    for source in [
        "ctx.shadowBlur = 4;",
        "ctx.shadowColor = color;",
        "ctx.globalCompositeOperation = 'multiply';",
        "const ctx = c.getContext('2d'); ctx.filter = 'blur(2px)';",
        "ctx.createPattern(img, 'repeat');",
        "ctx.isPointInPath(path, 1, 2);",
        "ctx.createConicGradient(0, 1, 2);",
    ] {
        assert!(
            judged(source).contains(&("web:canvas-2d".to_owned(), Partial)),
            "{source}"
        );
    }
    assert_eq!(
        judged(
            "document.execCommand('insertText', false, 'x'); el.ownerDocument.execCommand('undo');"
        ),
        vec![
            ("web:exec-command".to_owned(), Supported),
            ("web:exec-command".to_owned(), Supported)
        ]
    );
    assert_eq!(
        judged("document.execCommand('bold'); document.execCommand(name);"),
        vec![
            ("web:exec-command".to_owned(), Partial),
            ("web:exec-command".to_owned(), Partial)
        ]
    );
}

#[test]
fn web_platform_usage() {
    let found = ids(
        "const gl = canvas.getContext('webgl2');\nconst c2d = canvas.getContext('2d');\nconst bitmap = canvas.getContext('bitmaprenderer');\n\
         el.attachShadow({ mode: 'open' });\ndocument.execCommand('copy');\n\
         document.createElement('iframe');\ndocument.createElement('div');\ndocument.createElement(tag);\n\
         window.open('https://x');\nother.open('x');\ncustomElements.define('x-y', X);\nregistry.define('a');\n\
         fetch('/api');\nnew Worker('w.js');\nnew SharedWorker('w.js');\nnew WebSocket('ws://x');\n\
         new XMLHttpRequest();\nnew AudioContext();\nnew webkitAudioContext();\nnew OfflineAudioContext();\n\
         new ResizeObserver(cb);\nnew MutationObserver(cb);\nnew IntersectionObserver(cb);\nnew Map();\nnew x.Y();\n\
         navigator.serviceWorker.register('sw.js');\nnavigator.mediaDevices.getUserMedia();\n\
         navigator.clipboard.writeText('x');\nnavigator.clipboard.write([]);\nnavigator.language;\nwindow.indexedDB.open('db');\n\
         indexedDB.open('db');\nlocalStorage.setItem('a', 1);\nsessionStorage.clear();\nWebAssembly.instantiate(bytes);\ncrypto.subtle.digest('SHA-256', d);\nwindow.crypto.subtle;\nlib.subtle;\nfoo();\n(() => 1)();\n",
        SourceType::cjs(),
    );
    assert_eq!(
        found,
        vec![
            "web:webgl",
            "web:canvas-2d",
            "web:shadow-dom",
            "web:exec-command",
            "html:iframe",
            "web:window-open",
            "web:custom-elements",
            "web:fetch",
            "web:worker",
            "web:worker",
            "web:websocket",
            "web:xhr",
            "web:web-audio",
            "web:web-audio",
            "web:web-audio",
            "web:resize-observer",
            "web:mutation-observer",
            "web:intersection-observer",
            "web:service-worker",
            "web:media-devices",
            "web:clipboard-text",
            "web:clipboard-api",
            "web:indexeddb",
            "web:indexeddb",
            "web:local-storage",
            "web:local-storage",
            "web:webassembly",
            "web:web-crypto",
            "web:web-crypto",
        ]
    );
}

#[test]
fn jsx_and_decorators() {
    let found = ids(
        "@Component class A { @Input() value; }\n\
         const view = () => <div><iframe src=\"x\" /><Custom /><ns:tag /><webview /><button>x</button></div>;\n",
        SourceType::tsx(),
    );
    assert_eq!(
        found,
        vec![
            "language:decorators",
            "language:decorators",
            "framework:react",
            "html:basics",
            "html:iframe",
            "html:webview",
            "html:controls",
        ]
    );
}

#[test]
fn implicit_type_imports_resolve_symbols_instead_of_matching_names() {
    let source = "import {app, BrowserWindow, MenuItemConstructorOptions as Options, IpcRendererEvent} from 'electron';\n\
                  import DefaultType from 'node:stream';\n\
                  import * as Types from 'node:net';\n\
                  import fs = require('fs');\n\
                  let options: Options; let event: IpcRendererEvent; let win: BrowserWindow;\n\
                  type Default = DefaultType; type Namespace = Types.Socket; type Files = typeof fs;\n\
                  function shadow(Options: number) { return Options; }\n\
                  app.whenReady(); new BrowserWindow(options);";
    assert_eq!(
        ids(source, SourceType::ts()),
        vec!["electron:app", "electron:BrowserWindow"]
    );
    let emitted = nativeboat_bundler::transpile::transpile(
        source,
        Path::new("main.ts"),
        &nativeboat_bundler::TranspileOptions::default(),
    )
    .unwrap();
    assert_eq!(
        ids(&emitted, SourceType::mjs()),
        ids(source, SourceType::ts())
    );
    assert_eq!(
        ids(
            "import {Missing} from 'electron'; let value: Missing; console.log(Missing);",
            SourceType::ts()
        ),
        vec!["electron:Missing"]
    );
    assert_eq!(
        ids(
            "import {Missing} from 'electron'; function f(Missing: number){return Missing;} type X=Missing;",
            SourceType::ts()
        ),
        Vec::<String>::new()
    );
}

#[test]
fn frameworks_are_found_in_runtime_modules_and_stylesheet_imports() {
    assert_eq!(
        ids(
            "import {x} from '@angular/core/testing'; import '@angular/core';\n\
             import '@unrecognized/package'; import React from 'react';\n\
             export {x} from 'vue/runtime'; const jq=require('jquery');\n\
             import('svelte'); require('./theme.scss'); require('./theme.less');\n\
             import './theme.styl'; import 'sass'; const fragment = <></>;",
            SourceType::tsx()
        ),
        vec![
            "framework:@angular/core",
            "framework:react",
            "framework:vue",
            "framework:jquery",
            "framework:svelte",
            "framework:sass",
            "framework:less",
            "framework:stylus"
        ]
    );
    assert!(
        ids(
            "import Angular from '@angular/core'; type T=Angular;",
            SourceType::ts()
        )
        .is_empty()
    );
}

#[test]
fn unparseable_sources_report_what_was_parsed() {
    assert!(ids("const = ;", SourceType::mjs()).is_empty());
    assert_eq!(builtin_name("node:fs/promises"), Some("fs/promises"));
    assert_eq!(builtin_name("original-fs"), Some("fs"));
    assert_eq!(builtin_name("left-pad"), None);
    assert!(require_specifier(&dummy_expression()).is_none());
}

fn dummy_expression() -> Expression<'static> {
    let allocator: &'static Allocator = Box::leak(Box::new(Allocator::default()));
    Parser::new(allocator, "1", SourceType::mjs())
        .parse_expression()
        .unwrap()
}

#[test]
fn members_of_partial_electron_modules_are_judged_one_by_one() {
    let found = |source: &str| -> Vec<(String, Support)> {
        detect(source, SourceType::ts())
            .into_iter()
            .map(|finding| (finding.feature.id, finding.feature.support))
            .collect()
    };
    assert_eq!(
        found(
            "import { webFrame, net as network, app } from 'electron';\n\
             import * as electron from 'electron';\n\
             const { crashReporter } = require('electron');\n\
             webFrame.setZoomFactor(1.5);\nnetwork.fetch(url);\ncrashReporter.start({});\n\
             electron.webFrame.setZoomLevel(1);\nwebFrame.getWordSuggestions('x');\n"
        ),
        vec![
            ("electron:app".to_owned(), Support::Supported),
            ("electron:webFrame".to_owned(), Support::Supported),
            ("electron:net".to_owned(), Support::Supported),
            ("electron:crashReporter".to_owned(), Support::Supported),
            ("electron:webFrame".to_owned(), Support::Supported),
            (
                "electron:webFrame.getWordSuggestions".to_owned(),
                Support::Unsupported
            ),
        ]
    );
    // Passed on whole, a module may be used in any way.
    assert_eq!(
        found("import { webFrame } from 'electron';\nexpose({ webFrame });\n"),
        vec![("electron:webFrame".to_owned(), Support::Partial)]
    );
    // The default session is judged by its members where they are named.
    assert_eq!(
        found(
            "const { session } = require('electron');\n\
             session.defaultSession.resolveProxy(url);\nsession.fromPartition('p');\n\
             session.defaultSession.clearCache();\nsession.defaultSession.loadExtension(path);\n\
             const ses = session.defaultSession;\n"
        ),
        vec![
            ("electron:session".to_owned(), Support::Supported),
            ("electron:session".to_owned(), Support::Supported),
            (
                "electron:session.defaultSession.clearCache".to_owned(),
                Support::Partial
            ),
            (
                "electron:session.defaultSession.loadExtension".to_owned(),
                Support::Unsupported
            ),
            ("electron:session".to_owned(), Support::Partial),
        ]
    );
}

#[test]
fn members_of_window_contents_are_judged_one_by_one() {
    let found = |source: &str| -> Vec<(String, Support)> {
        detect(source, SourceType::mjs())
            .into_iter()
            .map(|finding| (finding.feature.id, finding.feature.support))
            .collect()
    };
    assert_eq!(
        found(
            "win.webContents.send('ready');\n\
             this.window?.webContents.executeJavaScript('1');\n\
             win.webContents.capturePage().then(save);\n"
        ),
        vec![
            ("electron:BrowserWindow".to_owned(), Support::Supported),
            (
                "electron:webContents.executeJavaScript".to_owned(),
                Support::Partial
            ),
            (
                "electron:webContents.capturePage".to_owned(),
                Support::Unsupported
            ),
        ]
    );
    // The `webContents` module is judged by its own members.
    assert_eq!(
        found(
            "import * as electron from 'electron';\n\
             electron.webContents.getAllWebContents();\n\
             electron.webContents.fromFrame(frame);\n"
        ),
        vec![
            ("electron:webContents".to_owned(), Support::Supported),
            (
                "electron:webContents.fromFrame".to_owned(),
                Support::Unsupported
            ),
        ]
    );
}
