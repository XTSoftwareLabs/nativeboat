//! Classic scripts: `<script>` elements without `type="module"`. Pages run
//! them in order with their top-level `var`, `let`, `const`, function and
//! class declarations shared through the global object, so one script sees
//! what another declared. Compiled as modules, each script's top level would
//! be its own; [`globalize`] rewrites a script so its top-level bindings
//! live on `globalThis` instead. Inline scripts become virtual modules next
//! to their page.

use oxc::ast_visit::Visit;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

thread_local! {
    /// The text of inline scripts, by their virtual paths.
    static INLINE: RefCell<HashMap<PathBuf, String>> = RefCell::default();
    /// The entry paths of classic scripts.
    static CLASSIC: RefCell<HashSet<PathBuf>> = RefCell::default();
    /// The files package and worker scripts run, by their virtual paths.
    static FILES: RefCell<HashMap<PathBuf, PathBuf>> = RefCell::default();
}

fn key(path: &Path) -> PathBuf {
    nativeboat_bundler::resolve::realpath(path)
}

/// The virtual path of a page's inline script and records its text.
pub(crate) fn inline(page: &Path, index: usize, text: &str) -> PathBuf {
    let stem = page
        .file_stem()
        .map_or_else(|| "page".into(), |stem| stem.to_string_lossy());
    let path = page
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!(".{stem}.inline-script-{index}.js"));
    INLINE.with_borrow_mut(|inline| inline.insert(key(&path), text.to_owned()));
    path
}

/// The event handler attributes of a page's HTML, as `onclick="..."`, and
/// of the markup its scripts build (see [`script_handlers`]), compiled into
/// a module that runs before the page's scripts: it gives each element of
/// the page with such an attribute its handler, and keeps the handlers by
/// their text for `setAttribute` and parsed markup to find. Handlers that
/// do not parse are left out, as browsers ignore them. `None` when there
/// are none.
pub(crate) fn handlers(page: &Path, html: &str, scripts: &[PathBuf]) -> Option<PathBuf> {
    use oxc::allocator::Allocator;
    use oxc::parser::Parser;
    use oxc::span::SourceType;

    let document = nativeboat_html::parse_document(html);
    let mut attributes: Vec<(String, String)> = document
        .elements()
        .iter()
        .flat_map(|element| {
            element
                .attributes
                .iter()
                .map(|attribute| (attribute.name.clone(), attribute.value.clone()))
        })
        .collect();
    attributes.extend(script_handlers(scripts));
    let mut seen = HashSet::new();
    let mut entries = String::new();
    for (name, value) in &attributes {
        if !name.starts_with("on") || name.len() < 3 || !seen.insert((name, value)) {
            continue;
        }
        let function = format!("function (event) {{\n{value}\n}}");
        let allocator = Allocator::default();
        let check = format!("({function});");
        if Parser::new(&allocator, &check, SourceType::cjs())
            .parse()
            .diagnostics
            .has_errors()
        {
            continue;
        }
        let key = serde_json::to_string(&format!("{name}\u{0}{value}")).ok()?;
        entries.push_str(&format!("  [{key}, {function}],\n"));
    }
    if entries.is_empty() {
        return None;
    }
    let text = format!(
        "(() => {{\nconst handlers = new Map([\n{entries}]);\nglobalThis.__nativeboatHandlers = handlers;\nfor (const element of document.getElementsByTagName('*')) {{\n  for (const {{ name, value }} of [...element.attributes]) {{\n    const handler = name.startsWith('on') && handlers.get(name + '\\u0000' + value);\n    if (handler && name in element) element[name] = handler;\n  }}\n}}\n}})();\n"
    );
    let stem = page
        .file_stem()
        .map_or_else(|| "page".into(), |stem| stem.to_string_lossy());
    let path = page
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!(".{stem}.event-handlers.js"));
    INLINE.with_borrow_mut(|inline| inline.insert(key(&path), text));
    Some(path)
}

/// The event handler attributes in the markup that scripts build: each
/// `on<name>="..."` (or with single quotes) in a string or a template
/// literal's text, whose handler is a whole text of the literal, read from
/// the scripts and the modules they import by relative paths.
pub(crate) fn script_handlers(scripts: &[PathBuf]) -> Vec<(String, String)> {
    use oxc::allocator::Allocator;
    use oxc::ast::ast::{
        ExportAllDeclaration, ExportFromDeclaration, ImportDeclaration, StringLiteral,
        TemplateLiteral,
    };
    use oxc::parser::Parser;
    use oxc::span::SourceType;

    #[derive(Default)]
    struct Texts {
        texts: Vec<String>,
        imports: Vec<String>,
    }
    impl<'a> Visit<'a> for Texts {
        fn visit_string_literal(&mut self, literal: &StringLiteral<'a>) {
            if literal.value.to_ascii_lowercase().contains("on") {
                self.texts.push(literal.value.to_string());
            }
        }
        fn visit_template_literal(&mut self, template: &TemplateLiteral<'a>) {
            for quasi in &template.quasis {
                let text = quasi.value.cooked.as_ref().unwrap_or(&quasi.value.raw);
                if text.to_ascii_lowercase().contains("on") {
                    self.texts.push(text.to_string());
                }
            }
            oxc::ast_visit::walk::walk_template_literal(self, template);
        }
        fn visit_import_declaration(&mut self, import: &ImportDeclaration<'a>) {
            self.imports.push(import.source.value.to_string());
        }
        fn visit_export_from_declaration(&mut self, export: &ExportFromDeclaration<'a>) {
            self.imports.push(export.source.value.to_string());
        }
        fn visit_export_all_declaration(&mut self, export: &ExportAllDeclaration<'a>) {
            self.imports.push(export.source.value.to_string());
        }
    }

    const MAX_MODULES: usize = 2000;
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    let mut pending: Vec<PathBuf> = scripts.to_vec();
    while let Some(path) = pending.pop() {
        if seen.len() >= MAX_MODULES || !seen.insert(key(&path)) {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        let kind = SourceType::from_path(&path).unwrap_or_else(|_| SourceType::mjs());
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, &source, kind).parse();
        let mut texts = Texts::default();
        texts.visit_program(&parsed.program);
        for text in &texts.texts {
            found.extend(markup_handlers(text));
        }
        let directory = path.parent().unwrap_or(Path::new("."));
        for specifier in texts.imports {
            if !(specifier.starts_with("./") || specifier.starts_with("../")) {
                continue;
            }
            let target = directory.join(&specifier);
            let candidates = [
                target.clone(),
                target.with_extension("js"),
                target.with_extension("mjs"),
                target.join("index.js"),
            ];
            if let Some(module) = candidates.into_iter().find(|candidate| candidate.is_file()) {
                pending.push(module);
            }
        }
    }
    found
}

/// The `on<name>="..."` attributes in a text of markup, with their values'
/// character references decoded.
fn markup_handlers(text: &str) -> Vec<(String, String)> {
    // Attribute names are not case-sensitive; ASCII lowercase keeps offsets.
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut found = Vec::new();
    let mut index = 0;
    while let Some(offset) = lower[index..].find("on") {
        let start = index + offset;
        index = start + 2;
        // An attribute name starts after whitespace.
        if start == 0 || !bytes[start - 1].is_ascii_whitespace() {
            continue;
        }
        let name_end = start
            + 2
            + bytes[start + 2..]
                .iter()
                .take_while(|byte| byte.is_ascii_alphabetic())
                .count();
        if name_end == start + 2 {
            continue;
        }
        let mut cursor = name_end;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'=') {
            continue;
        }
        cursor += 1;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        let Some(&quote @ (b'"' | b'\'')) = bytes.get(cursor) else {
            continue;
        };
        let Some(length) = text[cursor + 1..].find(quote as char) else {
            continue;
        };
        let value = &text[cursor + 1..cursor + 1 + length];
        let decoded = value
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&apos;", "'")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&");
        found.push((lower[start..name_end].to_owned(), decoded));
        index = cursor + 1 + length;
    }
    found
}

/// Notes that an entry is a classic script.
pub(crate) fn mark_classic(path: &Path) {
    CLASSIC.with_borrow_mut(|classic| classic.insert(key(path)));
}

/// The virtual path of a file a page or worker runs as a classic script.
/// Modules may import the same file, and they must get it as written rather
/// than with its top level made global, so the script is a module of its own.
pub(crate) fn script(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .map_or_else(|| "script".into(), |stem| stem.to_string_lossy());
    let extension = path
        .extension()
        .map_or_else(|| "js".into(), |extension| extension.to_string_lossy());
    let script = key(&path.with_file_name(format!(".{stem}.classic.{extension}")));
    FILES.with_borrow_mut(|files| files.insert(script.clone(), path.to_owned()));
    mark_classic(&script);
    script
}

/// The source of a module the compiler asks for, when it is an inline or a
/// classic script: the script with its top level made global.
pub(crate) fn source(path: &Path) -> Option<String> {
    let classic = CLASSIC.with_borrow(|classic| classic.contains(path));
    let file = FILES.with_borrow(|files| files.get(path).cloned());
    let text = match INLINE.with_borrow(|inline| inline.get(path).cloned()) {
        Some(text) => text,
        None if classic => std::fs::read_to_string(file.as_deref().unwrap_or(path)).ok()?,
        None => return None,
    };
    Some(if classic { globalize(&text) } else { text })
}

/// Rewrites a classic script so that its top-level declarations and the
/// references to them, and its top-level `this`, use `globalThis`. A script
/// that does not parse is left for the compiler to report.
pub(crate) fn globalize(source: &str) -> String {
    use oxc::allocator::Allocator;
    use oxc::parser::Parser;
    use oxc::semantic::SemanticBuilder;
    use oxc::span::SourceType;

    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::cjs()).parse();
    if parsed.diagnostics.has_errors() {
        return source.to_owned();
    }
    let semantic = SemanticBuilder::new().build(&parsed.program);
    if semantic.diagnostics.has_errors() {
        return source.to_owned();
    }
    let scoping = semantic.semantic.into_scoping();
    let globals: HashSet<_> = scoping.iter_bindings_in(scoping.root_scope_id()).collect();
    let mut rewrite = Rewrite {
        scoping: &scoping,
        globals: &globals,
        edits: Vec::new(),
        functions: 0,
        heads: 0,
        program_start: parsed
            .program
            .directives
            .last()
            .map_or(0, |directive| directive.span.end),
    };
    rewrite.visit_program(&parsed.program);
    apply(source, rewrite.edits)
}

/// Replaces `[start, end)` with the text; insertions have `start == end`
/// and go before replacements at the same place.
type Edit = (u32, u32, String);

fn apply(source: &str, mut edits: Vec<Edit>) -> String {
    edits.sort_by_key(|(start, end, _)| (*start, *end != *start));
    let mut output = String::with_capacity(source.len() + edits.len() * 16);
    let mut at = 0;
    for (start, end, text) in edits {
        let (start, end) = (start as usize, end as usize);
        if start < at {
            continue;
        }
        output.push_str(&source[at..start]);
        output.push_str(&text);
        at = end;
    }
    output.push_str(&source[at..]);
    output
}

struct Rewrite<'s> {
    scoping: &'s oxc::semantic::Scoping,
    globals: &'s HashSet<oxc::semantic::SymbolId>,
    edits: Vec<Edit>,
    /// How many non-arrow functions and classes enclose the walk, which
    /// give `this` another meaning.
    functions: u32,
    /// How deep the walk is in `for` heads.
    heads: u32,
    /// Where hoisted global assignments go, after any directives.
    program_start: u32,
}

impl Rewrite<'_> {
    fn global_binding(&self, identifier: &oxc::ast::ast::BindingIdentifier) -> bool {
        identifier
            .symbol_id
            .get()
            .is_some_and(|symbol| self.globals.contains(&symbol))
    }

    /// Whether a reference reads the global object: a top-level binding, or
    /// CommonJS's `module` and `exports`, which a page's classic scripts
    /// only have when the page defines them. Universal module bundles then
    /// take their browser branch and set their global, as in a browser.
    fn global_reference(&self, identifier: &oxc::ast::ast::IdentifierReference) -> bool {
        identifier.reference_id.get().is_some_and(|reference| {
            match self.scoping.get_reference(reference).symbol_id() {
                Some(symbol) => self.globals.contains(&symbol),
                None => matches!(identifier.name.as_str(), "module" | "exports"),
            }
        })
    }

    fn replace(&mut self, span: oxc::span::Span, name: &str) {
        self.edits
            .push((span.start, span.end, format!("globalThis.{name}")));
    }

    fn insert(&mut self, at: u32, text: String) {
        self.edits.push((at, at, text));
    }

    /// Rewrites the bindings of a declaration pattern into assignment
    /// targets on `globalThis`.
    fn pattern(&mut self, pattern: &oxc::ast::ast::BindingPattern) {
        use oxc::ast::ast::BindingPattern;
        match pattern {
            BindingPattern::BindingIdentifier(identifier) => {
                self.replace(identifier.span, &identifier.name);
            }
            BindingPattern::ObjectPattern(object) => {
                for property in &object.properties {
                    if property.shorthand {
                        let name = match &property.value {
                            BindingPattern::BindingIdentifier(identifier) => {
                                Some(identifier.name.to_string())
                            }
                            BindingPattern::AssignmentPattern(assignment) => {
                                match &assignment.left {
                                    BindingPattern::BindingIdentifier(identifier) => {
                                        Some(identifier.name.to_string())
                                    }
                                    _ => None,
                                }
                            }
                            _ => None,
                        };
                        if let Some(name) = name {
                            self.insert(property.span.start, format!("{name}: "));
                        }
                    }
                    self.pattern(&property.value);
                }
                if let Some(rest) = &object.rest {
                    self.pattern(&rest.argument);
                }
            }
            BindingPattern::ArrayPattern(array) => {
                for element in array.elements.iter().flatten() {
                    self.pattern(element);
                }
                if let Some(rest) = &array.rest {
                    self.pattern(&rest.argument);
                }
            }
            BindingPattern::AssignmentPattern(assignment) => {
                self.pattern(&assignment.left);
                self.visit_expression(&assignment.right);
            }
        }
    }
}

impl<'a> oxc::ast_visit::Visit<'a> for Rewrite<'_> {
    fn visit_identifier_reference(&mut self, identifier: &oxc::ast::ast::IdentifierReference<'a>) {
        if self.global_reference(identifier) {
            self.replace(identifier.span, &identifier.name);
        }
    }

    fn visit_object_property(&mut self, property: &oxc::ast::ast::ObjectProperty<'a>) {
        if property.shorthand
            && let oxc::ast::ast::Expression::Identifier(identifier) = &property.value
            && self.global_reference(identifier)
        {
            self.insert(property.span.start, format!("{}: ", identifier.name));
        }
        oxc::ast_visit::walk::walk_object_property(self, property);
    }

    fn visit_assignment_target_property_identifier(
        &mut self,
        property: &oxc::ast::ast::AssignmentTargetPropertyIdentifier<'a>,
    ) {
        if self.global_reference(&property.binding) {
            self.insert(property.span.start, format!("{}: ", property.binding.name));
        }
        oxc::ast_visit::walk::walk_assignment_target_property_identifier(self, property);
    }

    fn visit_this_expression(&mut self, this: &oxc::ast::ast::ThisExpression) {
        if self.functions == 0 {
            self.edits
                .push((this.span.start, this.span.end, "globalThis".into()));
        }
    }

    fn visit_function(
        &mut self,
        function: &oxc::ast::ast::Function<'a>,
        flags: oxc::semantic::ScopeFlags,
    ) {
        if function.is_declaration()
            && let Some(id) = &function.id
            && self.global_binding(id)
        {
            // Function declarations are hoisted: they are global before
            // any of the script runs.
            let at = self.program_start;
            self.insert(at, format!("\nglobalThis.{0} = {0};\n", id.name));
        }
        self.functions += 1;
        oxc::ast_visit::walk::walk_function(self, function, flags);
        self.functions -= 1;
    }

    fn visit_class(&mut self, class: &oxc::ast::ast::Class<'a>) {
        if class.is_declaration()
            && let Some(id) = &class.id
            && self.global_binding(id)
        {
            self.insert(class.span.end, format!("\nglobalThis.{0} = {0};", id.name));
        }
        self.functions += 1;
        oxc::ast_visit::walk::walk_class(self, class);
        self.functions -= 1;
    }

    fn visit_for_in_statement(&mut self, statement: &oxc::ast::ast::ForInStatement<'a>) {
        self.heads += 1;
        self.visit_for_statement_left(&statement.left);
        self.heads -= 1;
        self.visit_expression(&statement.right);
        self.visit_statement(&statement.body);
    }

    fn visit_for_of_statement(&mut self, statement: &oxc::ast::ast::ForOfStatement<'a>) {
        self.heads += 1;
        self.visit_for_statement_left(&statement.left);
        self.heads -= 1;
        self.visit_expression(&statement.right);
        self.visit_statement(&statement.body);
    }

    fn visit_variable_declaration(&mut self, declaration: &oxc::ast::ast::VariableDeclaration<'a>) {
        let global = !declaration.declarations.is_empty()
            && declaration.declarations.iter().all(|declarator| {
                declarator
                    .id
                    .get_binding_identifiers()
                    .iter()
                    .all(|identifier| self.global_binding(identifier))
            });
        if !global {
            oxc::ast_visit::walk::walk_variable_declaration(self, declaration);
            return;
        }
        let first = &declaration.declarations[0];
        let last = &declaration.declarations[declaration.declarations.len() - 1];
        let in_head = self.heads > 0;
        // `var a = 1, b;` becomes `(globalThis.a = 1, globalThis.b);`, the
        // parentheses keeping object patterns from reading as blocks; a
        // `for (var x of ...)` head takes the target alone.
        self.edits.push((
            declaration.span.start,
            first.span.start,
            if in_head { String::new() } else { "(".into() },
        ));
        if !in_head {
            self.insert(last.span.end, ")".into());
        }
        for declarator in &declaration.declarations {
            self.pattern(&declarator.id);
            if let Some(init) = &declarator.init {
                self.visit_expression(init);
            }
        }
    }
}

#[cfg(test)]
mod tests;
