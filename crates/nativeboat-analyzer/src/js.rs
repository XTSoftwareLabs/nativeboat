//! Feature detection in JavaScript and TypeScript sources.

use crate::catalog::{
    Feature, canvas_2d, electron, electron_member, element, exec_command, framework,
    is_partial_electron, is_partial_module, language, node, node_export, web, web_contents_member,
};
use oxc::allocator::Allocator;
use oxc::ast::ast::*;
use oxc::ast_visit::{Visit, walk};
use oxc::parser::Parser;
use oxc::semantic::{Reference, Scoping, SemanticBuilder, SymbolId};
use oxc::span::SourceType;
use std::collections::{HashMap, HashSet};

/// A feature found at a byte offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The feature.
    pub feature: Feature,
    /// Byte offset in the source.
    pub offset: u32,
}

fn is_electron(specifier: &str) -> bool {
    matches!(
        specifier,
        "electron" | "electron/main" | "electron/renderer" | "electron/common"
    )
}

fn builtin_name(specifier: &str) -> Option<&str> {
    // Electron's `original-fs` is `fs` without ASAR support.
    if specifier == "original-fs" {
        return Some("fs");
    }
    let name = specifier.strip_prefix("node:").unwrap_or(specifier);
    let base = name.split('/').next().unwrap_or(name);
    const NODE: &[&str] = &[
        "assert",
        "async_hooks",
        "buffer",
        "child_process",
        "cluster",
        "console",
        "constants",
        "crypto",
        "dgram",
        "diagnostics_channel",
        "dns",
        "domain",
        "events",
        "fs",
        "http",
        "http2",
        "https",
        "inspector",
        "module",
        "net",
        "os",
        "path",
        "perf_hooks",
        "process",
        "punycode",
        "querystring",
        "readline",
        "repl",
        "stream",
        "string_decoder",
        "sys",
        "timers",
        "tls",
        "trace_events",
        "tty",
        "url",
        "util",
        "v8",
        "vm",
        "wasi",
        "worker_threads",
        "zlib",
    ];
    NODE.contains(&base).then_some(name)
}

fn require_specifier<'a>(expression: &'a Expression<'_>) -> Option<&'a str> {
    let Expression::CallExpression(call) = expression else {
        return None;
    };
    let Expression::Identifier(callee) = &call.callee else {
        return None;
    };
    match (callee.name.as_str(), call.arguments.first()) {
        ("require", Some(Argument::StringLiteral(literal))) => Some(literal.value.as_str()),
        _ => None,
    }
}

fn string_argument(call: &CallExpression<'_>) -> Option<String> {
    match call.arguments.first() {
        Some(Argument::StringLiteral(literal)) => Some(literal.value.to_string()),
        _ => None,
    }
}

struct Detector<'s> {
    scoping: &'s Scoping,
    typescript: bool,
    findings: Vec<Finding>,
    electron_namespaces: HashSet<SymbolId>,
    /// Local names of Node.js modules the runtime supports in part, whose
    /// members are judged one by one.
    node_namespaces: HashMap<SymbolId, String>,
    /// Local names of Electron modules the runtime supports in part, such
    /// as `webFrame`, whose members are judged one by one.
    electron_objects: HashMap<SymbolId, String>,
    /// Local names bound to a canvas's 2D context.
    canvas_contexts: HashSet<SymbolId>,
}

/// Whether an expression is the call `<canvas>.getContext('2d')`.
fn is_canvas_context(expression: &Expression<'_>) -> bool {
    let Expression::CallExpression(call) = expression.get_inner_expression() else {
        return false;
    };
    matches!(&call.callee, Expression::StaticMemberExpression(member) if member.property.name == "getContext")
        && string_argument(call).is_some_and(|argument| argument == "2d")
}

/// Whether an expression is a document: `document`, `window.document` or a
/// node's `ownerDocument`.
fn is_document(expression: &Expression<'_>) -> bool {
    match expression.get_inner_expression() {
        Expression::Identifier(identifier) => identifier.name == "document",
        Expression::StaticMemberExpression(member) => {
            matches!(member.property.name.as_str(), "document" | "ownerDocument")
        }
        _ => false,
    }
}

/// What assigning `value` to a 2D context property relies on that is not
/// drawn, if anything. `filter` is judged only on known contexts.
fn canvas_assignment(
    property: &str,
    value: &Expression<'_>,
    context: bool,
) -> Option<&'static str> {
    let text = match value.get_inner_expression() {
        Expression::StringLiteral(literal) => {
            Some(literal.value.as_str().trim().to_ascii_lowercase())
        }
        _ => None,
    };
    let zero = matches!(value.get_inner_expression(), Expression::NumericLiteral(number) if number.value == 0.0);
    match property {
        "shadowBlur" | "shadowOffsetX" | "shadowOffsetY" if !zero => Some("shadows are not drawn"),
        "shadowColor"
            if !text.as_deref().is_some_and(|color| {
                matches!(color, "transparent" | "rgba(0, 0, 0, 0)" | "rgba(0,0,0,0)")
            }) =>
        {
            Some("shadows are not drawn")
        }
        "globalCompositeOperation"
            if !text
                .as_deref()
                .is_some_and(|mode| matches!(mode, "source-over" | "copy")) =>
        {
            Some("compositing modes other than source-over and copy draw as source-over")
        }
        "filter" if context && text.as_deref() != Some("none") => Some("filters are not applied"),
        _ => None,
    }
}

/// The names an object pattern takes, with their offsets.
fn pattern_names(pattern: &ObjectPattern<'_>) -> Vec<(String, u32)> {
    pattern
        .properties
        .iter()
        .filter_map(|property| {
            property
                .key
                .static_name()
                .map(|name| (name.to_string(), property.span.start))
        })
        .collect()
}

impl Detector<'_> {
    fn add(&mut self, feature: Feature, offset: u32) {
        self.findings.push(Finding { feature, offset });
    }

    /// Records exports of a module used by name.
    fn names(&mut self, specifier: &str, names: Vec<(String, u32)>, offset: u32) {
        self.framework(specifier, offset);
        if is_electron(specifier) {
            for (name, at) in names {
                self.add(electron(&name), at);
            }
            return;
        }
        let Some(module) = builtin_name(specifier) else {
            return;
        };
        if is_partial_module(module) && !names.is_empty() {
            for (name, at) in names {
                self.add(node_export(module, &name), at);
            }
        } else {
            self.add(node(module), offset);
        }
    }

    /// Binds a local name to a whole module.
    fn namespace(&mut self, specifier: &str, local: &BindingIdentifier<'_>, offset: u32) {
        self.framework(specifier, offset);
        if is_electron(specifier) {
            self.electron_namespaces.insert(local.symbol_id());
        } else if let Some(module) = builtin_name(specifier) {
            if is_partial_module(module) {
                self.node_namespaces
                    .insert(local.symbol_id(), module.to_owned());
            } else {
                self.add(node(module), offset);
            }
        }
    }

    fn add_web(&mut self, api: &str, offset: u32) {
        if let Some(feature) = web(api) {
            self.add(feature, offset);
        }
    }

    fn module(&mut self, specifier: &str, offset: u32) {
        self.framework(specifier, offset);
        if let Some(name) = builtin_name(specifier) {
            self.add(node(name), offset);
        }
    }

    fn framework(&mut self, specifier: &str, offset: u32) {
        let name = match specifier.rsplit('.').next() {
            Some("scss" | "sass") => "sass",
            Some("less") => "less",
            Some("styl" | "stylus") => "stylus",
            _ => {
                let end = if specifier.starts_with('@') {
                    specifier.match_indices('/').nth(1)
                } else {
                    specifier.match_indices('/').next()
                };
                end.map_or(specifier, |(index, _)| &specifier[..index])
            }
        };
        if let Some(feature) = framework(name)
            && !self
                .findings
                .iter()
                .any(|finding| finding.feature.id == feature.id)
        {
            self.add(feature, offset);
        }
    }

    /// Bindings used exclusively in type positions are erased by the
    /// TypeScript transformer. Resolve references by symbol, so a shadowed
    /// value with the same name cannot turn a type import into a runtime API.
    fn type_only(&self, binding: &BindingIdentifier<'_>) -> bool {
        let mut references = self.scoping.get_resolved_references(binding.symbol_id());
        self.typescript
            && references.next().is_some_and(Reference::is_type)
            && references.all(Reference::is_type)
    }

    fn symbol(&self, reference: &IdentifierReference<'_>) -> Option<SymbolId> {
        self.scoping
            .get_reference(reference.reference_id())
            .symbol_id()
    }

    fn electron_namespace(&self, reference: &IdentifierReference<'_>) -> bool {
        self.symbol(reference)
            .is_some_and(|symbol| self.electron_namespaces.contains(&symbol))
    }

    fn node_namespace(&self, reference: &IdentifierReference<'_>) -> Option<&str> {
        self.node_namespaces
            .get(&self.symbol(reference)?)
            .map(String::as_str)
    }

    fn electron_object(&self, reference: &IdentifierReference<'_>) -> Option<&str> {
        self.electron_objects
            .get(&self.symbol(reference)?)
            .map(String::as_str)
    }

    /// Binds a local name to an Electron module judged member by member.
    fn electron_binding(&mut self, api: &str, local: &BindingIdentifier<'_>) -> bool {
        if is_partial_electron(api) {
            self.electron_objects
                .insert(local.symbol_id(), api.to_owned());
            return true;
        }
        false
    }

    /// The Electron modules an object pattern binds, judged member by member;
    /// returns the other names.
    fn electron_pattern(&mut self, pattern: &ObjectPattern<'_>) -> Vec<(String, u32)> {
        let mut names = Vec::new();
        for property in &pattern.properties {
            let Some(name) = property.key.static_name() else {
                continue;
            };
            let bound = match &property.value {
                BindingPattern::BindingIdentifier(local) => self.electron_binding(&name, local),
                _ => false,
            };
            if !bound {
                names.push((name.to_string(), property.span.start));
            }
        }
        names
    }
}

impl<'a> Visit<'a> for Detector<'_> {
    // Type-only imports, exports and annotations disappear when TypeScript
    // is compiled.
    fn visit_import_declaration(&mut self, declaration: &ImportDeclaration<'a>) {
        if declaration.import_kind.is_type() {
            return;
        }
        let offset = declaration.span.start;
        let source = declaration.source.value.as_str();
        let mut named = Vec::new();
        let mut values = 0;
        for specifier in declaration.specifiers.iter().flatten() {
            let local = match specifier {
                ImportDeclarationSpecifier::ImportSpecifier(specifier) => &specifier.local,
                ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier) => &specifier.local,
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(specifier) => &specifier.local,
            };
            if self.type_only(local) {
                continue;
            }
            match specifier {
                ImportDeclarationSpecifier::ImportSpecifier(specifier)
                    if specifier.import_kind.is_type() => {}
                ImportDeclarationSpecifier::ImportSpecifier(specifier) => {
                    let name = specifier.imported.name();
                    if !(is_electron(source) && self.electron_binding(&name, &specifier.local)) {
                        named.push((name.to_string(), offset));
                    }
                }
                ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier) => {
                    values += 1;
                    self.namespace(source, &specifier.local, offset);
                }
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(specifier) => {
                    values += 1;
                    self.namespace(source, &specifier.local, offset);
                }
            }
        }
        let side_effect = declaration
            .specifiers
            .as_ref()
            .is_none_or(|specifiers| specifiers.is_empty());
        if !named.is_empty() || (side_effect && values == 0) {
            self.names(source, named, offset);
        }
    }

    fn visit_export_from_declaration(&mut self, declaration: &ExportFromDeclaration<'a>) {
        if declaration.export_kind.is_type() {
            return;
        }
        let offset = declaration.span.start;
        let names: Vec<(String, u32)> = declaration
            .specifiers
            .iter()
            .filter(|specifier| !specifier.export_kind.is_type())
            .map(|specifier| (specifier.local.name().to_string(), offset))
            .collect();
        if !names.is_empty() || declaration.specifiers.is_empty() {
            self.names(declaration.source.value.as_str(), names, offset);
        }
    }

    fn visit_export_all_declaration(&mut self, declaration: &ExportAllDeclaration<'a>) {
        if !declaration.export_kind.is_type() {
            self.names(
                declaration.source.value.as_str(),
                Vec::new(),
                declaration.span.start,
            );
        }
    }

    fn visit_ts_import_equals_declaration(&mut self, declaration: &TSImportEqualsDeclaration<'a>) {
        if let TSModuleReference::ExternalModuleReference(reference) = &declaration.module_reference
            && !declaration.import_kind.is_type()
            && !self.type_only(&declaration.id)
        {
            self.namespace(
                &reference.expression.value,
                &declaration.id,
                declaration.span.start,
            );
        }
    }

    fn visit_ts_type(&mut self, _: &TSType<'a>) {}

    fn visit_assignment_expression(&mut self, assignment: &AssignmentExpression<'a>) {
        // `crypto = require('node:crypto')`: a variable declared earlier
        // holds a whole module, as one initialized with it does.
        if let AssignmentTarget::AssignmentTargetIdentifier(target) = &assignment.left
            && let Some(specifier) = require_specifier(&assignment.right)
            && let Some(symbol) = self.symbol(target)
        {
            if is_electron(specifier) {
                self.electron_namespaces.insert(symbol);
            } else if let Some(module) = builtin_name(specifier).filter(|m| is_partial_module(m)) {
                self.node_namespaces.insert(symbol, module.to_owned());
            }
        }
        if let AssignmentTarget::StaticMemberExpression(member) = &assignment.left {
            let context = matches!(&member.object, Expression::Identifier(object)
                if self.symbol(object).is_some_and(|symbol| self.canvas_contexts.contains(&symbol)));
            if let Some(gap) = canvas_assignment(&member.property.name, &assignment.right, context)
            {
                self.add(canvas_2d(Some(gap)), assignment.span.start);
            }
        }
        walk::walk_assignment_expression(self, assignment);
    }

    fn visit_variable_declarator(&mut self, declarator: &VariableDeclarator<'a>) {
        let offset = declarator.span.start;
        let init = declarator.init.as_ref();
        if let (BindingPattern::BindingIdentifier(identifier), Some(init)) = (&declarator.id, init)
            && is_canvas_context(init)
        {
            self.canvas_contexts.insert(identifier.symbol_id());
        }
        let handled = match (&declarator.id, init) {
            // `const { app } = require('electron')`, `const cp = require('child_process')`
            (BindingPattern::ObjectPattern(pattern), Some(init))
                if require_specifier(init).is_some() =>
            {
                let specifier = require_specifier(init).unwrap_or_default();
                let names = if is_electron(specifier) {
                    self.electron_pattern(pattern)
                } else {
                    pattern_names(pattern)
                };
                self.names(specifier, names, offset);
                builtin_name(specifier).is_some() || is_electron(specifier)
            }
            (BindingPattern::BindingIdentifier(identifier), Some(init))
                if require_specifier(init).is_some() =>
            {
                let specifier = require_specifier(init).unwrap_or_default();
                self.namespace(specifier, identifier, offset);
                builtin_name(specifier).is_some() || is_electron(specifier)
            }
            // `const { app } = electron`, `const { spawn } = cp`
            (BindingPattern::ObjectPattern(pattern), Some(Expression::Identifier(object))) => {
                if self.electron_namespace(object) {
                    for (name, at) in self.electron_pattern(pattern) {
                        self.add(electron(&name), at);
                    }
                    true
                } else if let Some(module) = self.node_namespace(object).map(str::to_owned) {
                    for (name, at) in pattern_names(pattern) {
                        self.add(node_export(&module, &name), at);
                    }
                    true
                } else {
                    false
                }
            }
            _ => false,
        };
        if handled {
            self.visit_binding_pattern(&declarator.id);
        } else {
            walk::walk_variable_declarator(self, declarator);
        }
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        let offset = call.span.start;
        match &call.callee {
            Expression::Identifier(callee) if callee.name == "require" => {
                match string_argument(call) {
                    Some(specifier) if !is_electron(&specifier) => self.module(&specifier, offset),
                    Some(_) => {}
                    None => {
                        if let Some(feature) = language("dynamic-require") {
                            self.add(feature, offset);
                        }
                    }
                }
            }
            Expression::Identifier(callee) if callee.name == "fetch" => {
                self.add_web("fetch", offset)
            }
            Expression::StaticMemberExpression(member) => {
                let argument = string_argument(call)
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                match member.property.name.as_str() {
                    "addEventListener"
                        if matches!(
                            argument.as_str(),
                            "dragstart"
                                | "drag"
                                | "dragenter"
                                | "dragleave"
                                | "dragover"
                                | "drop"
                                | "dragend"
                        ) =>
                    {
                        self.add_web("drag-drop", offset)
                    }
                    "getContext" if argument.contains("webgl") => self.add_web("webgl", offset),
                    "getContext" if argument == "2d" => self.add(canvas_2d(None), offset),
                    "createPattern" => self.add(canvas_2d(Some("patterns are not drawn")), offset),
                    "isPointInPath" | "isPointInStroke" => {
                        self.add(canvas_2d(Some("hit testing reports no hit")), offset)
                    }
                    "createConicGradient" => self.add(
                        canvas_2d(Some("conic gradients are drawn as linear gradients")),
                        offset,
                    ),
                    "attachShadow" => self.add_web("shadow-dom", offset),
                    // Editors such as CodeMirror have their own execCommand.
                    "execCommand" if is_document(&member.object) => {
                        let command = string_argument(call);
                        self.add(exec_command(command.as_deref()), offset);
                    }
                    "createElement" => {
                        if let Some(feature) = element(&argument) {
                            self.add(feature, offset);
                        }
                    }
                    "open" if matches!(&member.object, Expression::Identifier(object) if object.name == "window") =>
                    {
                        self.add_web("window-open", offset);
                    }
                    "define" if matches!(&member.object, Expression::Identifier(object) if object.name == "customElements") =>
                    {
                        self.add_web("custom-elements", offset);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        walk::walk_call_expression(self, call);
    }

    fn visit_import_expression(&mut self, expression: &ImportExpression<'a>) {
        if let Expression::StringLiteral(literal) = &expression.source {
            self.module(&literal.value, expression.span.start);
        }
        walk::walk_import_expression(self, expression);
    }

    fn visit_new_expression(&mut self, expression: &NewExpression<'a>) {
        if let Expression::Identifier(callee) = &expression.callee {
            let api = match callee.name.as_str() {
                "Worker" | "SharedWorker" => Some("worker"),
                "WebSocket" => Some("websocket"),
                "XMLHttpRequest" => Some("xhr"),
                "AudioContext" | "webkitAudioContext" | "OfflineAudioContext" => Some("web-audio"),
                "ResizeObserver" => Some("resize-observer"),
                "MutationObserver" => Some("mutation-observer"),
                "IntersectionObserver" => Some("intersection-observer"),
                "DataTransfer" | "DragEvent" => Some("drag-drop"),
                _ => None,
            };
            if let Some(api) = api {
                self.add_web(api, expression.span.start);
            }
        }
        walk::walk_new_expression(self, expression);
    }

    fn visit_static_member_expression(&mut self, member: &StaticMemberExpression<'a>) {
        let offset = member.span.start;
        let property = member.property.name.as_str();
        if matches!(
            property,
            "draggable"
                | "dataTransfer"
                | "ondragstart"
                | "ondrag"
                | "ondragenter"
                | "ondragleave"
                | "ondragover"
                | "ondrop"
                | "ondragend"
        ) {
            self.add_web("drag-drop", offset);
        }
        if property == "setDragImage" {
            self.add_web("drag-image", offset);
        }
        if let Expression::Identifier(object) = &member.object
            && object.name == "WebAssembly"
        {
            self.add_web("webassembly", offset);
        }
        // `crypto.subtle`, `window.crypto.subtle`, `globalThis.crypto.subtle`
        if property == "subtle"
            && match &member.object {
                Expression::Identifier(object) => object.name == "crypto",
                Expression::StaticMemberExpression(inner) => inner.property.name == "crypto",
                _ => false,
            }
        {
            self.add_web("web-crypto", offset);
        }
        // `cp.spawn(...)`, `require('child_process').spawn(...)`
        let module = match &member.object {
            Expression::Identifier(object) => self.node_namespace(object).map(str::to_owned),
            object => require_specifier(object)
                .and_then(builtin_name)
                .filter(|module| is_partial_module(module))
                .map(str::to_owned),
        };
        if let Some(module) = module {
            self.add(node_export(&module, property), offset);
            return;
        }
        // `session.defaultSession.resolveProxy(...)`: the default session's
        // members.
        if let Expression::StaticMemberExpression(inner) = &member.object
            && inner.property.name == "defaultSession"
            && let Expression::Identifier(object) = &inner.object
            && self.electron_object(object) == Some("session")
        {
            self.add(
                electron_member("session", &format!("defaultSession.{property}")),
                offset,
            );
            return;
        }
        // `webFrame.setZoomFactor(...)`, `electron.webFrame.setZoomFactor(...)`
        let api = match &member.object {
            Expression::Identifier(object) => self.electron_object(object).map(str::to_owned),
            Expression::StaticMemberExpression(inner) => match &inner.object {
                Expression::Identifier(object)
                    if self.electron_namespace(object)
                        && is_partial_electron(&inner.property.name) =>
                {
                    Some(inner.property.name.to_string())
                }
                _ => None,
            },
            _ => None,
        };
        if let Some(api) = api {
            self.add(electron_member(&api, property), offset);
            return;
        }
        // `navigator.clipboard.writeText(...)`: text works as in Chromium;
        // other members are judged as the API.
        if let Expression::StaticMemberExpression(inner) = &member.object
            && inner.property.name == "clipboard"
            && matches!(&inner.object, Expression::Identifier(object) if object.name == "navigator")
        {
            let api = match property {
                "readText" | "writeText" => "clipboard-text",
                _ => "clipboard-api",
            };
            self.add_web(api, offset);
            return;
        }
        // `win.webContents.capturePage()`: a window's contents, judged member
        // by member, unlike the `webContents` module.
        if let Expression::StaticMemberExpression(inner) = &member.object
            && inner.property.name == "webContents"
            && !matches!(&inner.object, Expression::Identifier(object) if self.electron_namespace(object))
        {
            self.add(web_contents_member(property), offset);
        }
        if let Expression::Identifier(object) = &member.object {
            if self.electron_namespace(object) {
                self.add(electron(property), offset);
            }
            match (object.name.as_str(), property) {
                ("navigator", "serviceWorker") => self.add_web("service-worker", offset),
                ("navigator", "mediaDevices") => self.add_web("media-devices", offset),
                ("navigator", "clipboard") => self.add_web("clipboard-api", offset),
                (_, "indexedDB") => self.add_web("indexeddb", offset),
                _ => {}
            }
        }
        walk::walk_static_member_expression(self, member);
    }

    fn visit_identifier_reference(&mut self, identifier: &IdentifierReference<'a>) {
        // A module namespace used other than by its members.
        if let Some(module) = self.node_namespace(identifier) {
            let feature = node(module);
            self.add(feature, identifier.span.start);
            return;
        }
        if let Some(api) = self.electron_object(identifier) {
            let feature = electron(api);
            self.add(feature, identifier.span.start);
            return;
        }
        let api = match identifier.name.as_str() {
            "indexedDB" => "indexeddb",
            "localStorage" | "sessionStorage" => "local-storage",
            _ => return,
        };
        self.add_web(api, identifier.span.start);
    }

    fn visit_jsx_opening_element(&mut self, opening: &JSXOpeningElement<'a>) {
        self.framework("react", opening.span.start);
        if let JSXElementName::Identifier(name) = &opening.name {
            match element(name.name.as_str()) {
                Some(feature) => self.add(feature, opening.span.start),
                None => self.add(crate::catalog::html_basics(), opening.span.start),
            }
        }
        walk::walk_jsx_opening_element(self, opening);
    }

    fn visit_jsx_opening_fragment(&mut self, opening: &JSXOpeningFragment) {
        self.framework("react", opening.span.start);
    }

    fn visit_decorator(&mut self, decorator: &Decorator<'a>) {
        if let Some(feature) = language("decorators") {
            self.add(feature, decorator.span.start);
        }
        walk::walk_decorator(self, decorator);
    }
}

/// Detects features in one source file. Unparseable files report nothing.
pub fn detect(source: &str, source_type: SourceType) -> Vec<Finding> {
    detect_with_metadata(source, source_type, false)
}

/// Decorator metadata can turn annotation references into runtime values.
/// Retain ordinary imports conservatively when that emit option is enabled.
pub(crate) fn detect_with_metadata(
    source: &str,
    source_type: SourceType,
    decorator_metadata: bool,
) -> Vec<Finding> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    let scoping = SemanticBuilder::new()
        .build(&parsed.program)
        .semantic
        .into_scoping();
    let mut detector = Detector {
        scoping: &scoping,
        typescript: source_type.is_typescript() && !decorator_metadata,
        findings: Vec::new(),
        electron_namespaces: HashSet::new(),
        node_namespaces: HashMap::new(),
        electron_objects: HashMap::new(),
        canvas_contexts: HashSet::new(),
    };
    detector.visit_program(&parsed.program);
    // Seed module bindings before reporting references. A function may refer to
    // an import or require binding whose declaration appears later in the file.
    detector.findings.clear();
    detector.visit_program(&parsed.program);
    detector.findings
}

#[cfg(test)]
mod tests;
