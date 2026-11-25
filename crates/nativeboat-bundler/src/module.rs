//! Converts ES modules into the runtime's CommonJS-style module format.
//!
//! Imports become `const __nui_iN = __nui_interop(__nui_require("..."))` and every
//! reference to an imported binding is rewritten to a property access, so
//! bindings stay live like in ES modules. Exports are installed up front as
//! getters with `__nui_export`, which keeps circular imports working.
//!
//! The runtime provides these globals:
//!
//! * `__nui_interop(value)`: returns `value` when it is an ES module namespace
//!   (`__esModule`), otherwise `{ default: value, ...value }`.
//! * `__nui_export(exports, getters)`: marks `exports` as an ES module and
//!   defines an enumerable getter per entry.
//! * `__nui_export_star(exports, namespace)`: re-exports every name except
//!   `default` that is not already exported.
//!
//! Module functions also receive `__nui_import_meta` for `import.meta` and
//! `__nui_require`, the module's `require` under a name that the module's
//! own declarations, such as `const require = createRequire(...)`, cannot
//! shadow.

use oxc::allocator::Allocator;
use oxc::ast::ast::*;
use oxc::ast_visit::{Visit, walk};
use oxc::parser::Parser;
use oxc::semantic::{Scoping, SemanticBuilder, SymbolId};
use oxc::span::{GetSpan, SourceType, Span};
use oxc::syntax::scope::ScopeFlags;
use oxc_ecmascript::BoundNames;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// How a module refers to another one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DependencyKind {
    /// `import ... from "x"`
    Import,
    /// `export ... from "x"`
    ExportFrom,
    /// `require("x")`
    Require,
    /// `require.resolve("x")`, which must not evaluate a resource.
    Resolve,
    /// `import("x")`
    DynamicImport,
}

/// A module specifier found in the source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    /// The specifier as written.
    pub specifier: String,
    /// How it is referenced.
    pub kind: DependencyKind,
}

/// The rewritten module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewrittenModule {
    /// CommonJS-style code to wrap in a module function.
    pub code: String,
    /// Statically known dependencies, without duplicates, in source order.
    pub dependencies: Vec<Dependency>,
    /// Whether the source was an ES module.
    pub esm: bool,
    /// Constructs the runtime may not handle, such as `require(variable)`.
    pub warnings: Vec<String>,
    /// A computed resolve call using this file's module context.
    pub dynamic_resolutions: bool,
}

/// Rewrites JavaScript produced by [`crate::transpile::transpile`].
pub fn rewrite(code: &str) -> Result<RewrittenModule, Vec<String>> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, code, SourceType::unambiguous()).parse();
    if parsed.diagnostics.has_errors() {
        return Err(parsed
            .diagnostics
            .into_vec()
            .into_iter()
            .map(|diagnostic| diagnostic.message.to_string())
            .collect());
    }
    let program = parsed.program;
    let scoping = SemanticBuilder::new()
        .build(&program)
        .semantic
        .into_scoping();
    let mut rewriter = Rewriter {
        code,
        scoping: &scoping,
        edits: Vec::new(),
        imports: Vec::new(),
        bindings: HashMap::new(),
        exports: Vec::new(),
        stars: Vec::new(),
        dependencies: Vec::new(),
        warnings: Vec::new(),
        errors: Vec::new(),
        function_depth: 0,
        esm: false,
        requires: HashSet::new(),
        module_objects: HashSet::new(),
        create_helpers: HashSet::new(),
        const_declaration: false,
        dynamic_resolutions: false,
    };
    rewriter.collect_module_syntax(&program);
    rewriter.visit_program(&program);
    if !rewriter.errors.is_empty() {
        return Err(rewriter.errors);
    }
    Ok(rewriter.finish())
}

struct Edit {
    start: u32,
    end: u32,
    text: String,
}

struct Rewriter<'s> {
    code: &'s str,
    scoping: &'s Scoping,
    edits: Vec<Edit>,
    /// Module variables with their specifiers, in import order.
    imports: Vec<(String, String)>,
    /// Imported symbols and the expression replacing them.
    bindings: HashMap<SymbolId, String>,
    /// Exported names and getter expressions.
    exports: Vec<(String, String)>,
    /// Module variables re-exported with `export *`.
    stars: Vec<String>,
    dependencies: Vec<Dependency>,
    warnings: Vec<String>,
    errors: Vec<String>,
    function_depth: usize,
    esm: bool,
    requires: HashSet<SymbolId>,
    module_objects: HashSet<SymbolId>,
    create_helpers: HashSet<SymbolId>,
    const_declaration: bool,
    dynamic_resolutions: bool,
}

fn quote(text: &str) -> String {
    let mut quoted = String::from("\"");
    for character in text.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

fn is_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_alphabetic() || first == '_' || first == '$')
        && characters
            .all(|character| character.is_alphanumeric() || character == '_' || character == '$')
}

fn member(object: &str, name: &str) -> String {
    if is_identifier(name) {
        format!("{object}.{name}")
    } else {
        format!("{object}[{}]", quote(name))
    }
}

fn export_name(name: &ModuleExportName<'_>) -> String {
    match name {
        ModuleExportName::IdentifierName(name) => name.name.to_string(),
        ModuleExportName::IdentifierReference(name) => name.name.to_string(),
        ModuleExportName::StringLiteral(literal) => literal.value.to_string(),
    }
}

/// Evaluates conditions that are constant after defines are applied.
fn static_truth(expression: &Expression<'_>) -> Option<bool> {
    match expression {
        Expression::BooleanLiteral(literal) => Some(literal.value),
        Expression::ParenthesizedExpression(inner) => static_truth(&inner.expression),
        Expression::UnaryExpression(unary) if unary.operator == UnaryOperator::LogicalNot => {
            static_truth(&unary.argument).map(|value| !value)
        }
        Expression::LogicalExpression(logical) => {
            let left = static_truth(&logical.left)?;
            match logical.operator {
                LogicalOperator::And if !left => Some(false),
                LogicalOperator::Or if left => Some(true),
                LogicalOperator::Coalesce => None,
                _ => static_truth(&logical.right),
            }
        }
        Expression::BinaryExpression(binary) => {
            let literal = |expression: &Expression<'_>| match expression {
                Expression::StringLiteral(literal) => Some(literal.value.to_string()),
                _ => None,
            };
            let left = literal(&binary.left)?;
            let right = literal(&binary.right)?;
            match binary.operator {
                BinaryOperator::StrictEquality | BinaryOperator::Equality => Some(left == right),
                BinaryOperator::StrictInequality | BinaryOperator::Inequality => {
                    Some(left != right)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

impl<'s> Rewriter<'s> {
    fn text(&self, span: Span) -> &'s str {
        &self.code[span.start as usize..span.end as usize]
    }

    fn replace(&mut self, start: u32, end: u32, text: impl Into<String>) {
        self.edits.push(Edit {
            start,
            end,
            text: text.into(),
        });
    }

    fn depend(&mut self, specifier: &str, kind: DependencyKind) {
        let dependency = Dependency {
            specifier: specifier.to_owned(),
            kind,
        };
        if !self.dependencies.contains(&dependency) {
            self.dependencies.push(dependency);
        }
    }

    fn module_variable(&mut self, specifier: &str, kind: DependencyKind) -> String {
        self.depend(specifier, kind);
        if let Some((variable, _)) = self
            .imports
            .iter()
            .find(|(_, existing)| existing == specifier)
        {
            return variable.clone();
        }
        let variable = format!("__nui_i{}", self.imports.len());
        self.imports.push((variable.clone(), specifier.to_owned()));
        variable
    }

    fn bind(&mut self, local: &BindingIdentifier<'_>, expression: String) {
        if let Some(symbol) = local.symbol_id.get() {
            self.bindings.insert(symbol, expression);
        }
    }

    fn binding_of(&self, reference: &IdentifierReference<'_>) -> Option<&String> {
        let reference = reference.reference_id.get()?;
        let symbol = self.scoping.get_reference(reference).symbol_id()?;
        self.bindings.get(&symbol)
    }

    fn reference_symbol(&self, reference: &IdentifierReference<'_>) -> Option<SymbolId> {
        reference
            .reference_id
            .get()
            .and_then(|id| self.scoping.get_reference(id).symbol_id())
    }

    fn is_require(&self, expression: &Expression<'_>) -> bool {
        let Expression::Identifier(reference) = expression else {
            return false;
        };
        match self.reference_symbol(reference) {
            Some(symbol) => self.requires.contains(&symbol),
            None => reference.name == "require",
        }
    }

    fn module_reference(&self, expression: &Expression<'_>) -> bool {
        match expression {
            Expression::Identifier(reference) => {
                if self
                    .reference_symbol(reference)
                    .is_some_and(|id| self.module_objects.contains(&id))
                {
                    return true;
                }
                self.binding_of(reference).is_some_and(|binding| {
                    self.imports.iter().any(|(variable, name)| {
                        matches!(name.as_str(), "module" | "node:module")
                            && (binding == variable || *binding == format!("{variable}.default"))
                    })
                })
            }
            Expression::CallExpression(call) if self.is_require(&call.callee) => {
                matches!(call.arguments.as_slice(), [Argument::StringLiteral(name)] if matches!(name.value.as_str(),"module"|"node:module"))
            }
            _ => false,
        }
    }

    fn create_require(&self, expression: &Expression<'_>) -> bool {
        match expression {
            Expression::Identifier(reference) => {
                if self
                    .reference_symbol(reference)
                    .is_some_and(|id| self.create_helpers.contains(&id))
                {
                    return true;
                }
                self.binding_of(reference).is_some_and(|binding| {
                    self.imports.iter().any(|(variable, name)| {
                        matches!(name.as_str(), "module" | "node:module")
                            && *binding == format!("{variable}.createRequire")
                    })
                })
            }
            Expression::StaticMemberExpression(member)
                if member.property.name == "createRequire" =>
            {
                self.module_reference(&member.object)
            }
            _ => false,
        }
    }

    fn own_filename(&self, argument: &Argument<'_>) -> bool {
        match argument {
            Argument::StaticMemberExpression(member) => {
                matches!(member.property.name.as_str(), "url" | "filename")
                    && matches!(member.object, Expression::ImportMeta(_))
            }
            Argument::Identifier(reference) => {
                reference.name == "__filename" && self.reference_symbol(reference).is_none()
            }
            _ => false,
        }
    }

    fn collect_module_syntax(&mut self, program: &Program<'_>) {
        for statement in &program.body {
            match statement {
                Statement::ImportDeclaration(import) => {
                    self.esm = true;
                    let variable =
                        self.module_variable(&import.source.value, DependencyKind::Import);
                    for specifier in import.specifiers.iter().flatten() {
                        match specifier {
                            ImportDeclarationSpecifier::ImportSpecifier(named) => {
                                let expression = member(&variable, &export_name(&named.imported));
                                self.bind(&named.local, expression);
                            }
                            ImportDeclarationSpecifier::ImportDefaultSpecifier(default) => {
                                self.bind(&default.local, format!("{variable}.default"));
                            }
                            ImportDeclarationSpecifier::ImportNamespaceSpecifier(namespace) => {
                                self.bind(&namespace.local, variable.clone());
                            }
                        }
                    }
                    self.replace(import.span.start, import.span.end, "");
                }
                Statement::ExportFromDeclaration(export) => {
                    self.esm = true;
                    let variable =
                        self.module_variable(&export.source.value, DependencyKind::ExportFrom);
                    for specifier in &export.specifiers {
                        let expression = member(&variable, &export_name(&specifier.local));
                        self.exports
                            .push((export_name(&specifier.exported), expression));
                    }
                    self.replace(export.span.start, export.span.end, "");
                }
                Statement::ExportAllDeclaration(export) => {
                    self.esm = true;
                    let variable =
                        self.module_variable(&export.source.value, DependencyKind::ExportFrom);
                    match &export.exported {
                        Some(name) => self.exports.push((export_name(name), variable)),
                        None => self.stars.push(variable),
                    }
                    self.replace(export.span.start, export.span.end, "");
                }
                _ => {}
            }
        }
        for statement in &program.body {
            match statement {
                Statement::ExportNamedDeclaration(export) => {
                    self.esm = true;
                    for specifier in &export.specifiers {
                        let local = export_name(&specifier.local);
                        let expression = self
                            .scoping
                            .get_root_binding(local.as_str().into())
                            .and_then(|symbol| self.bindings.get(&symbol))
                            .cloned()
                            .unwrap_or(local);
                        self.exports
                            .push((export_name(&specifier.exported), expression));
                    }
                    self.replace(export.span.start, export.span.end, "");
                }
                Statement::ExportDeclaration(export) => {
                    self.esm = true;
                    let mut names = Vec::new();
                    export
                        .declaration
                        .bound_names(&mut |identifier| names.push(identifier.name.to_string()));
                    for name in names {
                        self.exports.push((name.clone(), name));
                    }
                    self.replace(export.span.start, export.declaration.span().start, "");
                }
                Statement::ExportDefaultDeclaration(export) => {
                    self.esm = true;
                    let declaration = export.declaration.span();
                    let named = match &export.declaration {
                        ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                            function.id.as_ref().map(|id| id.name.to_string())
                        }
                        ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                            class.id.as_ref().map(|id| id.name.to_string())
                        }
                        _ => None,
                    };
                    match named {
                        Some(name) => {
                            self.exports.push(("default".to_owned(), name));
                            self.replace(export.span.start, declaration.start, "");
                        }
                        None => {
                            self.exports
                                .push(("default".to_owned(), "__nui_default".to_owned()));
                            self.replace(
                                export.span.start,
                                declaration.start,
                                "const __nui_default = ",
                            );
                            if declaration.end == export.span.end {
                                self.replace(export.span.end, export.span.end, ";");
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn finish(mut self) -> RewrittenModule {
        self.edits.sort_by_key(|edit| (edit.start, edit.end));
        let mut body = String::with_capacity(self.code.len());
        let mut position = 0;
        for edit in &self.edits {
            body.push_str(&self.code[position..edit.start as usize]);
            body.push_str(&edit.text);
            position = edit.end as usize;
        }
        body.push_str(&self.code[position..]);
        if !self.esm {
            return RewrittenModule {
                code: body,
                dependencies: self.dependencies,
                esm: false,
                warnings: self.warnings,
                dynamic_resolutions: self.dynamic_resolutions,
            };
        }
        let mut code = String::from("\"use strict\";\n");
        let getters: Vec<String> = self
            .exports
            .iter()
            .map(|(name, expression)| format!("{}: () => {expression}", quote(name)))
            .collect();
        code.push_str(&format!(
            "__nui_export(exports, {{ {} }});\n",
            getters.join(", ")
        ));
        for (variable, specifier) in &self.imports {
            code.push_str(&format!(
                "const {variable} = __nui_interop(__nui_require({}));\n",
                quote(specifier)
            ));
        }
        for variable in &self.stars {
            code.push_str(&format!("__nui_export_star(exports, {variable});\n"));
        }
        code.push_str(&body);
        RewrittenModule {
            code,
            dependencies: self.dependencies,
            esm: true,
            warnings: self.warnings,
            dynamic_resolutions: self.dynamic_resolutions,
        }
    }

    /// Rewrites a callee or tag; returns false when it is not an import.
    fn rewrite_callee(&mut self, callee: &Expression<'_>) -> bool {
        let Expression::Identifier(reference) = callee else {
            return false;
        };
        let Some(expression) = self.binding_of(reference).cloned() else {
            return false;
        };
        self.replace(
            reference.span.start,
            reference.span.end,
            format!("(0, {expression})"),
        );
        true
    }
}

impl<'a> Visit<'a> for Rewriter<'_> {
    fn visit_variable_declaration(&mut self, declaration: &VariableDeclaration<'a>) {
        let previous = self.const_declaration;
        self.const_declaration = declaration.kind == VariableDeclarationKind::Const;
        walk::walk_variable_declaration(self, declaration);
        self.const_declaration = previous;
    }

    fn visit_variable_declarator(&mut self, declaration: &VariableDeclarator<'a>) {
        if self.const_declaration
            && let Some(init) = &declaration.init
        {
            match &declaration.id {
                BindingPattern::BindingIdentifier(binding) => {
                    if self.module_reference(init)
                        && let Some(symbol) = binding.symbol_id.get()
                    {
                        self.module_objects.insert(symbol);
                    }
                    if let Expression::CallExpression(call) = init
                        && self.create_require(&call.callee)
                        && matches!(call.arguments.as_slice(), [argument] if self.own_filename(argument))
                        && let Some(symbol) = binding.symbol_id.get()
                    {
                        self.requires.insert(symbol);
                    }
                }
                BindingPattern::ObjectPattern(pattern) if self.module_reference(init) => {
                    for property in &pattern.properties {
                        if property
                            .key
                            .static_name()
                            .is_some_and(|name| name == "createRequire")
                            && let BindingPattern::BindingIdentifier(binding) = &property.value
                            && let Some(symbol) = binding.symbol_id.get()
                        {
                            self.create_helpers.insert(symbol);
                        }
                    }
                }
                _ => {}
            }
        }
        walk::walk_variable_declarator(self, declaration);
    }
    fn visit_statement(&mut self, statement: &Statement<'a>) {
        match statement {
            Statement::ImportDeclaration(_)
            | Statement::ExportFromDeclaration(_)
            | Statement::ExportAllDeclaration(_)
            | Statement::ExportNamedDeclaration(_) => {}
            other => walk::walk_statement(self, other),
        }
    }

    fn visit_identifier_reference(&mut self, reference: &IdentifierReference<'a>) {
        if let Some(expression) = self.binding_of(reference).cloned() {
            self.replace(reference.span.start, reference.span.end, expression);
        }
    }

    fn visit_object_property(&mut self, property: &ObjectProperty<'a>) {
        if property.shorthand
            && let Expression::Identifier(reference) = &property.value
            && let Some(expression) = self.binding_of(reference).cloned()
        {
            self.replace(
                property.span.start,
                property.span.end,
                format!("{}: {expression}", reference.name),
            );
            return;
        }
        walk::walk_object_property(self, property);
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        let (callee, resolve) = match &call.callee {
            Expression::StaticMemberExpression(member) if member.property.name == "resolve" => {
                (&member.object, true)
            }
            other => (other, false),
        };
        if self.is_require(callee) {
            match call.arguments.first() {
                Some(Argument::StringLiteral(literal)) if call.arguments.len() == 1 => {
                    self.depend(
                        &literal.value,
                        if resolve {
                            DependencyKind::Resolve
                        } else {
                            DependencyKind::Require
                        },
                    );
                }
                _ => {
                    self.dynamic_resolutions |= resolve;
                    let operation = if resolve {
                        "require.resolve"
                    } else {
                        "require"
                    };
                    self.warnings
                        .push(format!("dynamic {operation}: {}", self.text(call.span)));
                }
            }
        }
        if self.rewrite_callee(&call.callee) {
            for argument in &call.arguments {
                self.visit_argument(argument);
            }
            return;
        }
        walk::walk_call_expression(self, call);
    }

    fn visit_tagged_template_expression(&mut self, tagged: &TaggedTemplateExpression<'a>) {
        if self.rewrite_callee(&tagged.tag) {
            self.visit_template_literal(&tagged.quasi);
            return;
        }
        walk::walk_tagged_template_expression(self, tagged);
    }

    fn visit_import_meta(&mut self, meta: &ImportMeta) {
        self.esm = true;
        self.replace(meta.span.start, meta.span.end, "__nui_import_meta");
    }

    fn visit_import_expression(&mut self, import: &ImportExpression<'a>) {
        match &import.source {
            Expression::StringLiteral(literal) => {
                self.depend(&literal.value, DependencyKind::DynamicImport);
            }
            _ => self
                .warnings
                .push(format!("dynamic import: {}", self.text(import.span))),
        }
        let source = import.source.span();
        self.replace(import.span.start, source.start, "Promise.resolve(");
        self.replace(
            source.end,
            import.span.end,
            ").then((specifier) => __nui_interop(__nui_require(specifier)))",
        );
        self.visit_expression(&import.source);
    }

    fn visit_if_statement(&mut self, statement: &IfStatement<'a>) {
        match static_truth(&statement.test) {
            Some(true) => self.visit_statement(&statement.consequent),
            Some(false) => {
                if let Some(alternate) = &statement.alternate {
                    self.visit_statement(alternate);
                }
            }
            None => walk::walk_if_statement(self, statement),
        }
    }

    fn visit_conditional_expression(&mut self, conditional: &ConditionalExpression<'a>) {
        match static_truth(&conditional.test) {
            Some(true) => self.visit_expression(&conditional.consequent),
            Some(false) => self.visit_expression(&conditional.alternate),
            None => walk::walk_conditional_expression(self, conditional),
        }
    }

    fn visit_logical_expression(&mut self, logical: &LogicalExpression<'a>) {
        let skip_right = matches!(
            (static_truth(&logical.left), logical.operator),
            (Some(false), LogicalOperator::And) | (Some(true), LogicalOperator::Or)
        );
        if skip_right {
            self.visit_expression(&logical.left);
            return;
        }
        walk::walk_logical_expression(self, logical);
    }

    fn visit_function(&mut self, function: &Function<'a>, flags: ScopeFlags) {
        self.function_depth += 1;
        walk::walk_function(self, function, flags);
        self.function_depth -= 1;
    }

    fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        self.function_depth += 1;
        walk::walk_arrow_function_expression(self, arrow);
        self.function_depth -= 1;
    }

    fn visit_await_expression(&mut self, expression: &AwaitExpression<'a>) {
        if self.function_depth == 0 {
            self.errors
                .push("top-level await is not supported".to_owned());
        }
        walk::walk_await_expression(self, expression);
    }
}

#[cfg(test)]
mod tests;
