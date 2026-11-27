//! Script templates: scripts a page builds as text while it runs, from a
//! template literal, and runs by inserting a `<script>` with that text, as
//! VS Code's webview host defines `acquireVsCodeApi` in the documents it
//! writes. No engine runs script text, so each template literal whose text
//! is a script compiles ahead of time to a function of its substitutions:
//! a substitution inside a string literal of the script is text of that
//! string, and one where the script has an expression is the text of an
//! expression, which the page evaluates as it runs the script (the Web API
//! prelude's `__nbTemplateHole`).
//!
//! Templates are found in the sources the renderer graphs compile, and
//! every renderer graph compiles all of them, as one document may build a
//! script that another runs. A page runs the template whose fixed text an
//! inserted script's text matches, with the text between as substitutions.

use nativeboat_compiler::{ModuleKind, ModuleSource};
use oxc::allocator::Allocator;
use oxc::ast::ast::{
    IdentifierReference, StringLiteral, TaggedTemplateExpression, TemplateLiteral,
};
use oxc::ast_visit::{Visit, walk};
use oxc::parser::Parser;
use oxc::span::SourceType;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

/// A template whose text is a script.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Template {
    /// The fixed text around its substitutions.
    quasis: Vec<String>,
    /// The script, with each substitution read from `__nbHoles`.
    body: String,
}

thread_local! {
    static TEMPLATES: RefCell<Vec<Template>> = const { RefCell::new(Vec::new()) };
    static SCANNING: Cell<bool> = const { Cell::new(false) };
}

/// The marker standing for substitution `index` while a template is parsed.
fn marker(index: usize) -> String {
    format!("__nbHole{index}__")
}

/// Starts or stops finding templates in the sources compiled meanwhile.
pub(crate) fn scanning(on: bool) {
    SCANNING.with(|scanning| scanning.set(on));
}

/// How many templates were found.
pub(crate) fn count() -> usize {
    TEMPLATES.with_borrow(Vec::len)
}

/// Finds the script templates of a source the build compiles, while
/// [`scanning`].
pub(crate) fn scan(path: &Path, text: &str) {
    if !SCANNING.with(Cell::get) || !text.contains("${") {
        return;
    }
    let Ok(source_type) = SourceType::from_path(path) else {
        return;
    };
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, text, source_type).parse();
    if parsed.diagnostics.has_errors() {
        return;
    }
    let mut finder = Finder::default();
    finder.visit_program(&parsed.program);
    TEMPLATES.with_borrow_mut(|known| {
        for template in finder.found {
            if !known.iter().any(|other| other.quasis == template.quasis) {
                known.push(template);
            }
        }
    });
}

#[derive(Default)]
struct Finder {
    found: Vec<Template>,
}

impl<'a> Visit<'a> for Finder {
    fn visit_tagged_template_expression(&mut self, expression: &TaggedTemplateExpression<'a>) {
        // A tag decides what its template means.
        self.visit_expression(&expression.tag);
        for substitution in &expression.quasi.expressions {
            self.visit_expression(substitution);
        }
    }

    fn visit_template_literal(&mut self, literal: &TemplateLiteral<'a>) {
        if !literal.expressions.is_empty()
            && let Some(quasis) = literal
                .quasis
                .iter()
                .map(|quasi| quasi.value.cooked.as_ref().map(|text| text.to_string()))
                .collect::<Option<Vec<_>>>()
            && let Some(body) = script_body(&quasis)
        {
            self.found.push(Template { quasis, body });
        }
        walk::walk_template_literal(self, literal);
    }
}

/// The script of a template's fixed text, with its substitutions read from
/// `__nbHoles`, if the text is a script: it parses, does something (calls,
/// assigns or declares), and every substitution is in a string literal or
/// stands where an expression does.
fn script_body(quasis: &[String]) -> Option<String> {
    let mut joined = String::new();
    for (index, quasi) in quasis.iter().enumerate() {
        if index > 0 {
            joined.push_str(&marker(index - 1));
        }
        joined.push_str(quasi);
    }
    let fixed: usize = quasis.iter().map(String::len).sum();
    if fixed < 40 || !joined.contains('(') || !joined.contains([';', '\n']) {
        return None;
    }
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, &joined, SourceType::cjs()).parse();
    if parsed.diagnostics.has_errors() || parsed.program.body.is_empty() {
        return None;
    }
    let mut holes = Holes {
        markers: (0..quasis.len() - 1).map(marker).collect(),
        found: vec![0; quasis.len() - 1],
        edits: Vec::new(),
        active: false,
    };
    holes.visit_program(&parsed.program);
    if !holes.active || holes.found.iter().any(|found| *found != 1) {
        return None;
    }
    // Every marker must be one the walk placed; others are in comments,
    // names or property keys.
    let total = holes
        .markers
        .iter()
        .map(|marker| joined.matches(marker.as_str()).count())
        .sum::<usize>();
    if total != holes.markers.len() {
        return None;
    }
    let mut edits = holes.edits;
    edits.sort_by_key(|(start, ..)| *start);
    let mut body = String::with_capacity(joined.len());
    let mut at = 0;
    for (start, end, text) in edits {
        body.push_str(&joined[at..start as usize]);
        body.push_str(&text);
        at = end as usize;
    }
    body.push_str(&joined[at..]);
    Some(body)
}

/// Where a template's substitution markers stand in its parsed script.
struct Holes {
    markers: Vec<String>,
    /// How many times each marker was placed.
    found: Vec<usize>,
    edits: Vec<(u32, u32, String)>,
    /// Whether the script calls, assigns or declares anything.
    active: bool,
}

impl<'a> Visit<'a> for Holes {
    fn visit_identifier_reference(&mut self, identifier: &IdentifierReference<'a>) {
        if let Some(index) = self
            .markers
            .iter()
            .position(|marker| identifier.name == marker.as_str())
        {
            self.found[index] += 1;
            self.edits.push((
                identifier.span.start,
                identifier.span.end,
                format!("__nbTemplateHole(__nbHoles[{index}])"),
            ));
        }
    }

    fn visit_string_literal(&mut self, literal: &StringLiteral<'a>) {
        let value = literal.value.as_str();
        if !self
            .markers
            .iter()
            .any(|marker| value.contains(marker.as_str()))
        {
            return;
        }
        // The literal becomes its text joined with the substitutions.
        let mut parts = Vec::new();
        let mut rest = value;
        loop {
            let next = self
                .markers
                .iter()
                .enumerate()
                .filter_map(|(index, marker)| rest.find(marker.as_str()).map(|at| (at, index)))
                .min();
            let Some((at, index)) = next else {
                break;
            };
            parts.push(serde_json::to_string(&rest[..at]).unwrap_or_default());
            parts.push(format!("String(__nbHoles[{index}])"));
            self.found[index] += 1;
            rest = &rest[at + self.markers[index].len()..];
        }
        parts.push(serde_json::to_string(rest).unwrap_or_default());
        self.edits.push((
            literal.span.start,
            literal.span.end,
            format!("({})", parts.join(" + ")),
        ));
    }

    fn visit_call_expression(&mut self, call: &oxc::ast::ast::CallExpression<'a>) {
        self.active = true;
        walk::walk_call_expression(self, call);
    }

    fn visit_assignment_expression(
        &mut self,
        assignment: &oxc::ast::ast::AssignmentExpression<'a>,
    ) {
        self.active = true;
        walk::walk_assignment_expression(self, assignment);
    }
}

/// The module a template compiles to: a function of its substitutions that
/// runs its script, with the script's top level made global.
fn module_text(template: &Template) -> String {
    format!(
        "export default function (__nbHoles) {{\n{}\n}}\n",
        crate::classic_script::globalize(&template.body)
    )
}

/// The virtual path of template `index` in a project.
fn path(root: &Path, index: usize) -> PathBuf {
    root.join(format!(".nativeboat/generated/script-template-{index}.mjs"))
}

/// The key a page finds template `index` by.
fn key(index: usize) -> String {
    format!("nativeboat-script-template-{index}.mjs")
}

/// The templates as scripts every renderer graph compiles: their keys and
/// virtual paths.
pub(crate) fn named_scripts(root: &Path) -> Vec<(String, PathBuf)> {
    (0..count())
        .map(|index| (key(index), path(root, index)))
        .collect()
}

/// The module source of a template's virtual path.
pub(crate) fn source(path: &Path) -> Option<ModuleSource> {
    let name = path.file_name()?.to_str()?;
    let index: usize = name
        .strip_prefix("script-template-")?
        .strip_suffix(".mjs")?
        .parse()
        .ok()?;
    if !path.parent()?.ends_with(".nativeboat/generated") {
        return None;
    }
    let text = TEMPLATES.with_borrow(|templates| templates.get(index).map(module_text))?;
    Some(ModuleSource {
        text,
        kind: ModuleKind::EsModule,
    })
}

/// The Rust that registers the templates with the runtime: each one's fixed
/// text and key.
pub(crate) fn registration() -> String {
    TEMPLATES.with_borrow(|templates| {
        if templates.is_empty() {
            return String::new();
        }
        let items: Vec<String> = templates
            .iter()
            .enumerate()
            .map(|(index, template)| format!("(&{:?}, {:?})", template.quasis, key(index)))
            .collect();
        format!(
            "{{ static TEMPLATES: ::nativeboat_native::web::ScriptTemplates = &[{}]; ::nativeboat_native::web::register_script_templates(TEMPLATES); }} ",
            items.join(", ")
        )
    })
}

#[cfg(test)]
mod tests;
