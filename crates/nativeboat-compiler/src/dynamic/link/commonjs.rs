//! Compiled CommonJS factories. No source, path search or parser runs in an app.
use super::*;
use oxc::ast_visit::{Visit, walk};

const PREFIX: &str =
    "export default function (exports, require, module, __filename, __dirname) {\n";

/// A CommonJS module's unchanged source, with the text the compiler inserted
/// into its factory: by original position, the inserted length, ascending.
pub(super) struct Original {
    pub(super) text: String,
    edits: Vec<(u32, u32)>,
}

impl Original {
    /// The original position of a position in the factory's text. Inserted
    /// text maps to where it was inserted.
    fn position(&self, wrapped: u32) -> u32 {
        let position = wrapped.saturating_sub(PREFIX.len() as u32);
        let mut shift = 0;
        for &(at, length) in &self.edits {
            let start = at + shift;
            if position < start {
                break;
            }
            if position < start + length {
                return at;
            }
            shift += length;
        }
        (position - shift).min(self.text.len() as u32)
    }
}

/// Translate wrapper byte positions back into the unchanged original source.
pub(super) fn remap(error: &mut Diagnostic, original: &Original) {
    let span = Span::new(
        original.position(error.span.start),
        original.position(error.span.end),
    );
    *error = Source {
        path: &error.path,
        text: &original.text,
    }
    .error(span, &error.message);
}

/// Annex B.3.3: in sloppy code, a function declared in a block is also a
/// `var` of the enclosing function, given the function's value when its
/// declaration is evaluated, unless a lexical declaration of the name would
/// make that `var` an early error. Factories are compiled as strict code,
/// so this inserts that `var` at the start of the function's body, with a
/// setter, and a call of the setter after the declaration. Positions in the
/// source are unchanged; the inserted text is listed by position.
struct AnnexB {
    functions: Vec<AnnexScope>,
    /// Whether each enclosing block is a function body, with the names it
    /// declares lexically.
    blocks: Vec<(bool, Vec<String>)>,
    strict: Vec<bool>,
    edits: Vec<(u32, String)>,
}

struct AnnexScope {
    insert_at: u32,
    params: Vec<String>,
    /// The index in `blocks` of the function's body.
    body: usize,
    declared: Vec<String>,
}

fn annex_b_setter(name: &str) -> String {
    format!("__nativeboat_annex_b_{name}")
}

/// The names statements declare lexically: `let`, `const` and classes, and
/// in blocks also functions.
fn lexical_names(statements: &[Statement<'_>], block: bool) -> Vec<String> {
    let mut names = Vec::new();
    for statement in statements {
        match statement {
            Statement::VariableDeclaration(declaration)
                if declaration.kind != VariableDeclarationKind::Var =>
            {
                for declarator in &declaration.declarations {
                    names.extend(
                        declarator
                            .id
                            .get_binding_identifiers()
                            .iter()
                            .map(|identifier| identifier.name.to_string()),
                    );
                }
            }
            Statement::ClassDeclaration(class) => {
                names.extend(class.id.iter().map(|id| id.name.to_string()));
            }
            Statement::FunctionDeclaration(function) if block => {
                names.extend(function.id.iter().map(|id| id.name.to_string()));
            }
            _ => {}
        }
    }
    names
}

fn has_use_strict(body: &FunctionBody<'_>) -> bool {
    body.directives
        .iter()
        .any(|directive| directive.directive == "use strict")
}

impl AnnexB {
    fn enter_function(
        &mut self,
        insert_at: u32,
        params: &FormalParameters<'_>,
        body: &FunctionBody<'_>,
    ) {
        let strict = *self.strict.last().unwrap_or(&false) || has_use_strict(body);
        let params = params
            .items
            .iter()
            .flat_map(|param| param.pattern.get_binding_identifiers())
            .chain(
                params
                    .rest
                    .iter()
                    .flat_map(|rest| rest.rest.argument.get_binding_identifiers()),
            )
            .map(|identifier| identifier.name.to_string())
            .collect();
        self.blocks
            .push((true, lexical_names(&body.statements, false)));
        self.functions.push(AnnexScope {
            insert_at,
            params,
            body: self.blocks.len() - 1,
            declared: Vec::new(),
        });
        self.strict.push(strict);
    }

    fn exit_function(&mut self) {
        self.strict.pop();
        self.blocks.pop();
        let scope = self.functions.pop().expect("entered");
        if scope.declared.is_empty() {
            return;
        }
        let mut text = String::new();
        for name in &scope.declared {
            let setter = annex_b_setter(name);
            let _ = write!(
                text,
                "var {name}; const {setter} = (__value) => {{ {name} = __value; }}; "
            );
        }
        self.edits.push((scope.insert_at, text));
    }

    fn block<T>(&mut self, names: Vec<String>, visit: impl FnOnce(&mut Self) -> T) -> T {
        self.blocks.push((false, names));
        let result = visit(self);
        self.blocks.pop();
        result
    }

    fn block_function(&mut self, function: &Function<'_>) {
        let Some(id) = &function.id else {
            return;
        };
        if *self.strict.last().unwrap_or(&true)
            || function.r#async
            || function.generator
            || !self.blocks.last().is_some_and(|(body, _)| !body)
        {
            return;
        }
        let name = id.name.as_str();
        let Some(scope) = self.functions.last() else {
            return;
        };
        if scope.params.iter().any(|param| param == name) {
            return;
        }
        let enclosing = &self.blocks[scope.body..self.blocks.len() - 1];
        if enclosing
            .iter()
            .any(|(_, names)| names.iter().any(|known| known == name))
        {
            return;
        }
        self.edits.push((
            function.span.end,
            format!(";{}({name});", annex_b_setter(name)),
        ));
        let scope = self.functions.last_mut().expect("checked");
        if !scope.declared.iter().any(|known| known == name) {
            scope.declared.push(name.to_owned());
        }
    }
}

impl<'a> Visit<'a> for AnnexB {
    fn visit_function(&mut self, function: &Function<'a>, flags: oxc::semantic::ScopeFlags) {
        if function.r#type == FunctionType::FunctionDeclaration {
            self.block_function(function);
        }
        let Some(body) = &function.body else {
            return;
        };
        self.enter_function(body.span.start + 1, &function.params, body);
        walk::walk_function(self, function, flags);
        self.exit_function();
    }

    fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        // An expression body declares nothing; functions in it have their own.
        let ArrowFunctionBody::FunctionBody(body) = &arrow.body else {
            walk::walk_arrow_function_expression(self, arrow);
            return;
        };
        self.enter_function(body.span.start + 1, &arrow.params, body);
        walk::walk_arrow_function_expression(self, arrow);
        self.exit_function();
    }

    fn visit_class(&mut self, class: &Class<'a>) {
        self.strict.push(true);
        walk::walk_class(self, class);
        self.strict.pop();
    }

    fn visit_block_statement(&mut self, block: &BlockStatement<'a>) {
        let names = lexical_names(&block.body, true);
        self.block(names, |this| walk::walk_block_statement(this, block));
    }

    fn visit_switch_statement(&mut self, statement: &SwitchStatement<'a>) {
        self.visit_expression(&statement.discriminant);
        let names = statement
            .cases
            .iter()
            .flat_map(|case| lexical_names(&case.consequent, true))
            .collect();
        self.block(names, |this| {
            for case in &statement.cases {
                this.visit_switch_case(case);
            }
        });
    }

    fn visit_for_statement(&mut self, statement: &ForStatement<'a>) {
        let names = match &statement.init {
            Some(ForStatementInit::VariableDeclaration(declaration))
                if declaration.kind != VariableDeclarationKind::Var =>
            {
                declaration
                    .declarations
                    .iter()
                    .flat_map(|declarator| declarator.id.get_binding_identifiers())
                    .map(|identifier| identifier.name.to_string())
                    .collect()
            }
            _ => Vec::new(),
        };
        self.block(names, |this| walk::walk_for_statement(this, statement));
    }

    fn visit_for_in_statement(&mut self, statement: &ForInStatement<'a>) {
        let names = for_left_names(&statement.left);
        self.block(names, |this| walk::walk_for_in_statement(this, statement));
    }

    fn visit_for_of_statement(&mut self, statement: &ForOfStatement<'a>) {
        let names = for_left_names(&statement.left);
        self.block(names, |this| walk::walk_for_of_statement(this, statement));
    }

    fn visit_catch_clause(&mut self, clause: &CatchClause<'a>) {
        let names = clause
            .param
            .iter()
            .flat_map(|param| param.pattern.get_binding_identifiers())
            .map(|identifier| identifier.name.to_string())
            .collect();
        self.block(names, |this| walk::walk_catch_clause(this, clause));
    }
}

fn for_left_names(left: &ForStatementLeft<'_>) -> Vec<String> {
    match left {
        ForStatementLeft::VariableDeclaration(declaration)
            if declaration.kind != VariableDeclarationKind::Var =>
        {
            declaration
                .declarations
                .iter()
                .flat_map(|declarator| declarator.id.get_binding_identifiers())
                .map(|identifier| identifier.name.to_string())
                .collect()
        }
        _ => Vec::new(),
    }
}

/// The Annex B insertions for a sloppy CommonJS module, by position.
fn annex_b_edits(program: &Program<'_>) -> Vec<(u32, String)> {
    let mut annex = AnnexB {
        functions: Vec::new(),
        blocks: Vec::new(),
        strict: vec![program.has_use_strict_directive()],
        edits: Vec::new(),
    };
    // The factory's own parameters and body.
    annex
        .blocks
        .push((true, lexical_names(&program.body, false)));
    annex.functions.push(AnnexScope {
        insert_at: 0,
        params: ["exports", "require", "module", "__filename", "__dirname"]
            .map(str::to_owned)
            .to_vec(),
        body: 0,
        declared: Vec::new(),
    });
    for statement in &program.body {
        annex.visit_statement(statement);
    }
    annex.exit_function();
    annex.edits.sort_by_key(|(at, _)| *at);
    annex.edits
}

pub(super) fn scan(
    path: &Path,
    original: String,
    options: &CompileOptions,
) -> Result<Record, Vec<Diagnostic>> {
    let allocator = Allocator::default();
    let source = Source {
        path,
        text: &original,
    };
    let source_type = SourceType::from_path(path)
        .map_err(|error| vec![source.error(Span::default(), &error.to_string())])?
        .with_commonjs(true);
    let parsed = Parser::new(&allocator, allocator.alloc_str(&original), source_type).parse();
    if parsed.diagnostics.has_errors() {
        return Err(source.diagnostics(parsed.diagnostics));
    }
    let strict_source = parsed.program.has_use_strict_directive();
    let insertions = if strict_source {
        Vec::new()
    } else {
        annex_b_edits(&parsed.program)
    };
    // Hashbangs are valid only at a file's beginning. A same-byte-length comment
    // retains every original position when placing that source in its factory.
    let mut body = if let Some(body) = original.strip_prefix("#!") {
        format!("//{body}")
    } else {
        original.clone()
    };
    for (at, text) in insertions.iter().rev() {
        body.insert_str(*at as usize, text);
    }
    let original = Original {
        text: original,
        edits: insertions
            .iter()
            .map(|(at, text)| (*at, text.len() as u32))
            .collect(),
    };
    let text = format!("{PREFIX}{body}\n}}\n");
    let mut record = super::scan(path, text, options).map_err(|mut errors| {
        for error in &mut errors {
            remap(error, &original);
        }
        errors
    })?;
    record.strict = strict_source;
    let wrapped = Source {
        path,
        text: &record.source,
    };
    let (program, scoping) = parse(&allocator, &wrapped, options).expect("wrapper was scanned");
    let mut collector = Requests {
        source: &wrapped,
        scoping: &scoping,
        require: None,
        module: None,
        module_rebound: false,
        functions: Vec::new(),
        allowed: HashSet::new(),
        argument_mappings: Vec::new(),
        argument_owners: Vec::new(),
        strict_functions: vec![strict_source],
        class_depth: 0,
        requests: Vec::new(),
        resolutions: Vec::new(),
        errors: Vec::new(),
        sloppy_differences: Vec::new(),
        current_variable_kind: None,
        defines: &options.defines,
        try_depth: 0,
        optional: HashSet::new(),
        require_operations: HashSet::new(),
        warnings: Vec::new(),
    };
    collector.visit_program(&program);
    if !strict_source {
        for (span, message) in collector.sloppy_differences.drain(..) {
            collector.errors.push(wrapped.error(span, message));
        }
    }
    for diagnostic in collector.errors.iter_mut().chain(&mut collector.warnings) {
        remap(diagnostic, &original);
    }
    record.errors = collector.errors;
    record.warnings = collector.warnings;
    record.requests = collector.requests;
    record.optional = collector.optional;
    record.resolutions = collector.resolutions;
    record.original = Some(original);
    record.named_exports = true;
    Ok(record)
}

/// Only direct, statically known requests may reach generated dependency code.
/// Shadowed identifiers are ordinary local values, not special module requests.
struct Requests<'s> {
    source: &'s Source<'s>,
    scoping: &'s oxc::semantic::Scoping,
    require: Option<SymbolId>,
    module: Option<SymbolId>,
    module_rebound: bool,
    functions: Vec<oxc::semantic::ScopeId>,
    allowed: HashSet<Span>,
    /// Whether the nearest function that owns `arguments` has unmapped
    /// semantics. Arrows inherit their enclosing function's arguments object.
    argument_mappings: Vec<Option<bool>>,
    /// Whether the nearest owner is the compiler-owned CommonJS wrapper.
    argument_owners: Vec<Option<bool>>,
    /// Strictness of original CommonJS functions. The wrapped parser sees an
    /// ES module, so its ScopeFlags incorrectly mark every nested function
    /// strict; track source directives and class bodies instead.
    strict_functions: Vec<bool>,
    class_depth: usize,
    requests: Vec<(String, Span, Option<usize>)>,
    resolutions: Vec<Resolution>,
    errors: Vec<Diagnostic>,
    /// Sloppy-only behavior not yet preserved by the native function model.
    sloppy_differences: Vec<(Span, &'static str)>,
    current_variable_kind: Option<VariableDeclarationKind>,
    /// Build-time defines, which decide some conditions: requests in the
    /// branches they rule out are never made.
    defines: &'s BTreeMap<String, crate::dynamic::CompileValue>,
    /// How many `try` blocks the visitor is in.
    try_depth: usize,
    /// Requests made inside `try` blocks.
    optional: HashSet<Span>,
    /// `require` references whose properties are read or that are assigned,
    /// which the factory's function does not provide.
    require_operations: HashSet<Span>,
    /// Uses that compile but fail when they run.
    warnings: Vec<Diagnostic>,
}

/// A value build-time defines and literals decide.
#[derive(Debug, Clone, PartialEq)]
enum Known {
    Undefined,
    Null,
    Boolean(bool),
    Number(f64),
    String(String),
}

impl Known {
    fn truthy(&self) -> bool {
        match self {
            Known::Undefined | Known::Null => false,
            Known::Boolean(value) => *value,
            Known::Number(value) => *value != 0.0 && !value.is_nan(),
            Known::String(value) => !value.is_empty(),
        }
    }
}

impl Requests<'_> {
    /// The value of an expression that literals and build-time defines
    /// decide, such as `process.env.NODE_ENV === 'production'`.
    fn known(&self, expression: &Expression<'_>) -> Option<Known> {
        use crate::dynamic::CompileValue;
        match expression.without_parentheses() {
            Expression::StringLiteral(literal) => Some(Known::String(literal.value.to_string())),
            Expression::NumericLiteral(literal) => Some(Known::Number(literal.value)),
            Expression::BooleanLiteral(literal) => Some(Known::Boolean(literal.value)),
            Expression::NullLiteral(_) => Some(Known::Null),
            Expression::Identifier(_) | Expression::StaticMemberExpression(_) => {
                let value = self.defines.get(&self.global_path(expression)?)?;
                Some(match value {
                    CompileValue::Undefined => Known::Undefined,
                    CompileValue::Null => Known::Null,
                    CompileValue::Boolean(value) => Known::Boolean(*value),
                    CompileValue::Number(value) => Known::Number(*value),
                    CompileValue::String(value) => Known::String(value.clone()),
                    CompileValue::GlobalThis => return None,
                })
            }
            Expression::UnaryExpression(unary) if unary.operator == UnaryOperator::LogicalNot => {
                Some(Known::Boolean(!self.known(&unary.argument)?.truthy()))
            }
            Expression::BinaryExpression(binary) => {
                let (left, right) = (self.known(&binary.left)?, self.known(&binary.right)?);
                let nullish = |value: &Known| matches!(value, Known::Undefined | Known::Null);
                let equal = match binary.operator {
                    BinaryOperator::StrictEquality | BinaryOperator::StrictInequality => {
                        left == right
                    }
                    // Loose equality is decided without conversions only.
                    _ if nullish(&left) || nullish(&right) => nullish(&left) && nullish(&right),
                    _ if core::mem::discriminant(&left) == core::mem::discriminant(&right) => {
                        left == right
                    }
                    _ => return None,
                };
                match binary.operator {
                    BinaryOperator::StrictEquality | BinaryOperator::Equality => {
                        Some(Known::Boolean(equal))
                    }
                    BinaryOperator::StrictInequality | BinaryOperator::Inequality => {
                        Some(Known::Boolean(!equal))
                    }
                    _ => None,
                }
            }
            Expression::LogicalExpression(logical) => {
                let left = self.known(&logical.left)?;
                match logical.operator {
                    LogicalOperator::And if !left.truthy() => Some(left),
                    LogicalOperator::Or if left.truthy() => Some(left),
                    LogicalOperator::And | LogicalOperator::Or => self.known(&logical.right),
                    LogicalOperator::Coalesce => None,
                }
            }
            _ => None,
        }
    }

    /// The dotted name of a global read, such as `process.env.NODE_ENV`,
    /// when its first name is not a local binding.
    fn global_path(&self, expression: &Expression<'_>) -> Option<String> {
        match expression.without_parentheses() {
            Expression::Identifier(identifier)
                if self
                    .scoping
                    .get_reference(identifier.reference_id())
                    .symbol_id()
                    .is_none() =>
            {
                Some(identifier.name.to_string())
            }
            Expression::StaticMemberExpression(member) if !member.optional => Some(format!(
                "{}.{}",
                self.global_path(&member.object)?,
                member.property.name
            )),
            _ => None,
        }
    }

    fn sloppy_difference(&mut self, span: Span, message: &'static str) {
        if self.sloppy_differences.is_empty() {
            self.sloppy_differences.push((span, message));
        }
    }

    /// Notes a `require` whose property an expression reads.
    fn note_require_operation(&mut self, object: &Expression) {
        if let Expression::Identifier(identifier) = object.without_parentheses()
            && self.is(identifier, self.require)
        {
            self.require_operations.insert(identifier.span);
        }
    }

    fn is(&self, identifier: &IdentifierReference<'_>, symbol: Option<SymbolId>) -> bool {
        self.scoping
            .get_reference(identifier.reference_id())
            .symbol_id()
            == symbol
    }

    fn arguments_identifier(&self, expression: &Expression<'_>) -> Option<Span> {
        let Expression::Identifier(identifier) = expression.without_parentheses() else {
            return None;
        };
        (identifier.name == "arguments"
            && self
                .scoping
                .get_reference(identifier.reference_id())
                .symbol_id()
                .is_none())
        .then_some(identifier.span)
    }

    fn current_arguments_are_unmapped(&self) -> bool {
        self.argument_mappings
            .iter()
            .rev()
            .find_map(|mapping| *mapping)
            .unwrap_or(false)
    }

    fn current_arguments_belong_to_wrapper(&self) -> bool {
        self.argument_owners
            .iter()
            .rev()
            .find_map(|owner| *owner)
            .unwrap_or(false)
    }

    fn function_arguments_are_unmapped(&self, function: &Function<'_>) -> bool {
        let parameters = &function.params;
        let simple = parameters.rest.is_none()
            && parameters.items.iter().all(|parameter| {
                parameter.initializer.is_none()
                    && matches!(parameter.pattern, BindingPattern::BindingIdentifier(_))
            });
        !simple
    }
}

impl<'a> Visit<'a> for Requests<'_> {
    fn visit_variable_declarator(&mut self, declaration: &VariableDeclarator<'a>) {
        let mut rebind_module = false;
        if declaration.init.is_some() {
            for identifier in declaration.id.get_binding_identifiers() {
                if Some(identifier.symbol_id()) == self.require {
                    self.errors.push(self.source.error(
                        identifier.span,
                        "CommonJS wrapper binding replacement is not compiled yet",
                    ));
                }
                // `var` may redeclare a function parameter in CommonJS. The
                // generated factory passes an ordinary mutable parameter, so
                // initialized `var module` has the same local binding behavior.
                // Once rebound, reads are ordinary JavaScript values rather
                // than operations on Node's wrapper module object.
                if Some(identifier.symbol_id()) == self.module
                    && self.current_variable_kind == Some(VariableDeclarationKind::Var)
                {
                    rebind_module = true;
                }
            }
        }
        walk::walk_variable_declarator(self, declaration);
        if rebind_module {
            self.module_rebound = true;
        }
    }

    fn visit_function(&mut self, function: &Function<'a>, flags: oxc::semantic::ScopeFlags) {
        // The first function is the compiler-owned wrapper. Identify its
        // bindings before walking user code; nested shadowing stays ordinary.
        if self.require.is_none() {
            self.require =
                Some(function.params.items[1].pattern.get_binding_identifiers()[0].symbol_id());
            self.module =
                Some(function.params.items[2].pattern.get_binding_identifiers()[0].symbol_id());
        }
        if let Some(identifier) = &function.id
            && (Some(identifier.symbol_id()) == self.require
                || Some(identifier.symbol_id()) == self.module)
        {
            self.errors.push(self.source.error(
                identifier.span,
                "CommonJS wrapper binding replacement is not compiled yet",
            ));
        }
        self.functions.push(function.scope_id());
        let strict = self.strict_functions.last().copied().unwrap_or(false)
            || self.class_depth > 0
            || function.body.as_ref().is_some_and(|body| {
                body.directives
                    .iter()
                    .any(|directive| directive.directive.as_str() == "use strict")
            });
        self.strict_functions.push(strict);
        let wrapper = self.functions.len() == 1;
        self.argument_mappings.push(Some(
            strict || self.function_arguments_are_unmapped(function),
        ));
        self.argument_owners.push(Some(wrapper));
        walk::walk_function(self, function, flags);
        self.argument_mappings.pop();
        self.argument_owners.pop();
        self.strict_functions.pop();
        self.functions.pop();
    }

    fn visit_arrow_function_expression(&mut self, function: &ArrowFunctionExpression<'a>) {
        self.functions.push(function.scope_id());
        self.argument_mappings.push(None);
        self.argument_owners.push(None);
        self.strict_functions
            .push(self.strict_functions.last().copied().unwrap_or(false) || self.class_depth > 0);
        walk::walk_arrow_function_expression(self, function);
        self.strict_functions.pop();
        self.argument_owners.pop();
        self.argument_mappings.pop();
        self.functions.pop();
    }

    fn visit_class(&mut self, class: &Class<'a>) {
        self.class_depth += 1;
        walk::walk_class(self, class);
        self.class_depth -= 1;
    }

    fn visit_this_expression(&mut self, expression: &ThisExpression) {
        // The emitter normalizes this for each sloppy ordinary function at its
        // call boundary. Arrows retain lexical this and strict functions skip it.
        walk::walk_this_expression(self, expression);
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        // `Object.defineProperty(module, 'exports', descriptor)`: the module
        // object is real, and its exports are read once the factory returns.
        if self.global_path(&call.callee).as_deref() == Some("Object.defineProperty")
            && let [
                Argument::Identifier(identifier),
                Argument::StringLiteral(name),
                _,
            ] = call.arguments.as_slice()
            && self.is(identifier, self.module)
            && name.value == "exports"
        {
            self.allowed.insert(identifier.span);
        }
        if let Expression::StaticMemberExpression(member) = call.callee.without_parentheses()
            && let Expression::Identifier(identifier) = member.object.without_parentheses()
            && self.is(identifier, self.require)
            && member.property.name == "resolve"
        {
            self.allowed.insert(identifier.span);
            match call.arguments.as_slice() {
                [Argument::StringLiteral(literal)] if !call.optional && !member.optional && !literal.lone_surrogates => {
                    self.resolutions.push(Resolution {
                        call: call.span,
                        request: literal.value.to_string(),
                        location: literal.span,
                        path: None,
                    });
                }
                // As a computed require, a computed resolution answers for
                // the modules the build compiled and throws MODULE_NOT_FOUND
                // for others when it runs.
                [argument]
                    if !call.optional
                        && !member.optional
                        && !matches!(
                            argument,
                            Argument::SpreadElement(_) | Argument::StringLiteral(_)
                        ) =>
                {
                    self.warnings.push(self.source.error(
                        call.span,
                        "this require.resolve is computed when the application runs and throws MODULE_NOT_FOUND then for modules the file does not require by name",
                    ));
                }
                _ => self.errors.push(self.source.error(call.span, "CommonJS require.resolve needs one static string literal; computed, optional or options-based resolution is not compiled yet")),
            }
        }
        if let Expression::Identifier(identifier) = call.callee.without_parentheses()
            && self.is(identifier, self.require)
        {
            self.allowed.insert(identifier.span);
            match call.arguments.as_slice() {
                [Argument::StringLiteral(literal)] if !call.optional && !literal.lone_surrogates => {
                    self.requests.push((literal.value.to_string(), literal.span, None));
                    if self.try_depth > 0 {
                        self.optional.insert(literal.span);
                    }
                }
                // A request computed when the application runs names no
                // module the build compiled: like a missing one, it throws
                // Node's MODULE_NOT_FOUND then, as bundlers leave it.
                _ => self.warnings.push(self.source.error(
                    call.span,
                    "this require is computed when the application runs and throws MODULE_NOT_FOUND then; only modules required by name are compiled",
                )),
            }
        } else if let Expression::Identifier(identifier) = call.callee.without_parentheses()
            && identifier.name == "require"
            && let [Argument::StringLiteral(literal)] = call.arguments.as_slice()
            && !call.optional
            && !literal.lone_surrogates
        {
            // A UMD factory receives the file's `require` as a parameter of
            // that name (`factory(require, exports)`): the modules it loads
            // by name are the file's requests too. They are optional, as the
            // parameter may be given another function.
            self.requests
                .push((literal.value.to_string(), literal.span, None));
            self.optional.insert(literal.span);
        }
        walk::walk_call_expression(self, call);
    }

    fn visit_static_member_expression(&mut self, member: &StaticMemberExpression<'a>) {
        if !self.current_arguments_are_unmapped()
            && member.property.name == "callee"
            && self.arguments_identifier(&member.object).is_some()
        {
            self.sloppy_difference(
                member.span,
                "sloppy arguments.callee identity is not compiled yet",
            );
        }
        if let Expression::Identifier(identifier) = member.object.without_parentheses()
            && self.is(identifier, self.module)
            && (member.property.name == "exports"
                || member.property.name == "filename"
                || member.property.name == "nodeType"
                || self.module_rebound)
            && !member.optional
        {
            self.allowed.insert(identifier.span);
        }
        self.note_require_operation(&member.object);
        if let Expression::Identifier(identifier) = member.object.without_parentheses()
            && self.is(identifier, self.require)
            && member.property.name == "main"
            && !member.optional
        {
            // `require.main` is defined for CommonJS entry points, and is
            // `undefined` when the application's entry point is ESM.
            self.allowed.insert(identifier.span);
        }
        walk::walk_static_member_expression(self, member);
    }

    fn visit_variable_declaration(&mut self, declaration: &VariableDeclaration<'a>) {
        let previous = self.current_variable_kind.replace(declaration.kind);
        walk::walk_variable_declaration(self, declaration);
        self.current_variable_kind = previous;
    }

    fn visit_if_statement(&mut self, statement: &IfStatement<'a>) {
        match self.known(&statement.test).map(|value| value.truthy()) {
            Some(true) => self.visit_statement(&statement.consequent),
            Some(false) => {
                if let Some(alternate) = &statement.alternate {
                    self.visit_statement(alternate);
                }
            }
            None => walk::walk_if_statement(self, statement),
        }
    }

    fn visit_conditional_expression(&mut self, expression: &ConditionalExpression<'a>) {
        match self.known(&expression.test).map(|value| value.truthy()) {
            Some(true) => self.visit_expression(&expression.consequent),
            Some(false) => self.visit_expression(&expression.alternate),
            None => walk::walk_conditional_expression(self, expression),
        }
    }

    fn visit_try_statement(&mut self, statement: &TryStatement<'a>) {
        self.try_depth += 1;
        self.visit_block_statement(&statement.block);
        self.try_depth -= 1;
        if let Some(handler) = &statement.handler {
            self.visit_catch_clause(handler);
        }
        if let Some(finalizer) = &statement.finalizer {
            self.visit_block_statement(finalizer);
        }
    }

    fn visit_logical_expression(&mut self, expression: &LogicalExpression<'a>) {
        if let Some(left) = self.known(&expression.left) {
            let decided = match expression.operator {
                LogicalOperator::And => !left.truthy(),
                LogicalOperator::Or => left.truthy(),
                LogicalOperator::Coalesce => false,
            };
            if decided {
                return;
            }
        }
        // UMD CommonJS detection treats the wrapper module as a truthy object
        // and returns that same object from the short-circuit chain.
        for operand in [&expression.left, &expression.right] {
            if let Expression::Identifier(identifier) = operand.without_parentheses()
                && self.is(identifier, self.module)
            {
                self.allowed.insert(identifier.span);
            }
        }
        walk::walk_logical_expression(self, expression);
    }

    fn visit_binary_expression(&mut self, expression: &BinaryExpression<'a>) {
        // `require.main === module`: the module object is real, and compares
        // by identity.
        if expression.operator.is_equality() {
            for operand in [&expression.left, &expression.right] {
                if let Expression::Identifier(identifier) = operand.without_parentheses()
                    && self.is(identifier, self.module)
                {
                    self.allowed.insert(identifier.span);
                }
            }
        }
        walk::walk_binary_expression(self, expression);
    }

    fn visit_unary_expression(&mut self, expression: &UnaryExpression<'a>) {
        if expression.operator == UnaryOperator::Typeof
            && let Expression::Identifier(identifier) = expression.argument.without_parentheses()
            && (self.is(identifier, self.module) || self.is(identifier, self.require))
        {
            self.allowed.insert(identifier.span);
        }
        if expression.operator == UnaryOperator::Delete {
            self.visit_expression(&expression.argument);
        } else {
            walk::walk_unary_expression(self, expression);
        }
    }

    fn visit_computed_member_expression(&mut self, member: &ComputedMemberExpression<'a>) {
        self.note_require_operation(&member.object);
        if !self.current_arguments_are_unmapped()
            && matches!(member.expression.without_parentheses(), Expression::StringLiteral(key) if key.value == "callee")
            && self.arguments_identifier(&member.object).is_some()
        {
            self.sloppy_difference(
                member.span,
                "sloppy arguments.callee identity is not compiled yet",
            );
        }
        walk::walk_computed_member_expression(self, member);
    }

    fn visit_assignment_expression(&mut self, expression: &AssignmentExpression<'a>) {
        if let AssignmentTarget::AssignmentTargetIdentifier(identifier) = &expression.left
            && self.is(identifier, self.require)
        {
            self.require_operations.insert(identifier.span);
        }
        self.visit_assignment_target(&expression.left);
        self.visit_expression(&expression.right);
    }

    fn visit_update_expression(&mut self, expression: &UpdateExpression<'a>) {
        if let SimpleAssignmentTarget::AssignmentTargetIdentifier(identifier) = &expression.argument
            && self.is(identifier, self.require)
        {
            self.require_operations.insert(identifier.span);
        }
        self.visit_simple_assignment_target(&expression.argument);
    }

    fn visit_identifier_reference(&mut self, identifier: &IdentifierReference<'a>) {
        if identifier.name == "arguments"
            && self
                .scoping
                .get_reference(identifier.reference_id())
                .symbol_id()
                .is_none()
            && self.current_arguments_belong_to_wrapper()
        {
            self.errors.push(self.source.error(
                identifier.span,
                "CommonJS wrapper arguments access is not compiled yet",
            ));
        }
        if !self.allowed.contains(&identifier.span)
            && !(self.module_rebound && self.is(identifier, self.module))
        {
            let message = if self.is(identifier, self.require) {
                if !self.require_operations.contains(&identifier.span) {
                    // Passed on or kept, as UMD and browserify bundles keep it
                    // for modules they lack, `require` is the factory's own
                    // function.
                    self.warnings.push(self.source.error(
                        identifier.span,
                        "require is used as a value here; it loads the modules this file requires by name, and other requests throw MODULE_NOT_FOUND when the application runs",
                    ));
                    return;
                }
                "CommonJS require properties and assignments are not compiled yet"
            } else {
                // `module` is the wrapper's module object when the file runs,
                // whatever the file does with it.
                return;
            };
            self.errors
                .push(self.source.error(identifier.span, message));
        }
    }
}

/// A module's path in the application folder, with `/` separators: the
/// application's own modules and its installed packages run from there, as
/// a packaged application's do, with the packages' files beside them.
pub(super) fn application_path(graph: &Graph, path: &Path) -> Option<String> {
    // A host module stands for its importer.
    let text = path.to_string_lossy();
    let path = match text.split_once(super::HOST) {
        Some((importer, _)) => std::borrow::Cow::Owned(std::path::PathBuf::from(importer)),
        None => std::borrow::Cow::Borrowed(path),
    };
    let path = path.as_ref();
    if let Some(emitted) = super::emitted_path(&graph.emitted_folders, path) {
        return Some(emitted);
    }
    let root = graph.application.as_deref()?;
    let relative = path.strip_prefix(root).ok()?;
    Some(relative.to_string_lossy().replace('\\', "/"))
}

/// Each module gets its own native factory evaluator. The shared cache holds
/// the module object before invoking its body, exposing partial exports in cycles.
pub(super) fn factory(graph: &Graph, index: usize) -> String {
    let record = &graph.records[index];
    let factory = graph.locals[index][&Local::Default];
    let exported = graph.commonjs[index].expect("CommonJS has an export cell");
    let mut cases = BTreeMap::new();
    for (name, _, target) in &record.requests {
        cases.insert(name, target.expect("requests resolved before generation"));
    }
    let mut dispatch = String::new();
    let mut resolutions = String::new();
    for (name, target) in &cases {
        let target_path = &graph.records[*target].path;
        let value = if target_path.is_absolute() {
            match application_path(graph, target_path) {
                Some(relative) => format!(
                    "::nativeboat_native::process_builtin::app_relative(realm, {relative:?})"
                ),
                None => format!("Ok({})", emit::path_value(target_path)),
            }
        } else {
            // Built-in modules resolve to their names.
            format!("Ok(Value::str({name:?}))")
        };
        let _ = writeln!(resolutions, "{name:?} => {value},");
    }
    for (name, target) in cases {
        if super::required_es_module(graph, index, target) {
            // An ES module, evaluated before this factory runs, gives its
            // namespace, as Node's `require(esm)` does.
            let _ = writeln!(
                dispatch,
                "{name:?} => binding::read(realm, &cells[{}], \"*namespace*\"),",
                graph.namespaces[target]
            );
        } else {
            let _ = writeln!(
                dispatch,
                "{name:?} => require_c{target}(realm, &cells, &cache),"
            );
        }
    }
    let (filename, dirname) = match application_path(graph, &record.path) {
        Some(relative) => {
            let directory = relative
                .rsplit_once('/')
                .map_or("", |(directory, _)| directory);
            (
                format!("::nativeboat_native::process_builtin::app_relative(realm, {relative:?})?"),
                format!(
                    "::nativeboat_native::process_builtin::app_relative(realm, {directory:?})?"
                ),
            )
        }
        None => (
            emit::path_value(&record.path),
            emit::path_value(record.path.parent().unwrap_or(Path::new(""))),
        ),
    };
    let main_module = if graph.commonjs[0].is_some() {
        "main_cache[0].borrow().as_ref().map(|module| Value::Object(module.clone())).unwrap_or(Value::Undefined)"
    } else {
        "Value::Undefined"
    };
    format!(
        "\n#[allow(unused_variables)]\n\
         fn require_c{index}(realm: &mut Realm, cells: &::alloc::rc::Rc<Vec<binding::Binding>>, cache: &::alloc::rc::Rc<Vec<::core::cell::RefCell<Option<Object>>>>) -> ::nativeboat_native::Completion {{\n\
         let cached = cache[{index}].borrow().clone();\n\
         if let Some(module) = cached {{ return ops::get(realm, &Value::Object(module), &Key::name(\"exports\")); }}\n\
         let exports = Value::Object(realm.object());\n\
         let module = Value::Object(realm.object());\n\
         ops::set(realm, &module, &Key::name(\"exports\"), exports.clone())?;\n\
         ops::set(realm, &module, &Key::name(\"filename\"), {filename})?;\n\
         *cache[{index}].borrow_mut() = module.as_object().cloned();\n\
         let require = {{ let cells = cells.clone(); let main_cache = cache.clone(); let cache = cache.clone();\n\
         let function = realm.function(\"require\", 1, false, move |realm, _, args| {{\n\
         let request = ops::to_string(realm, args.first().unwrap_or(&Value::Undefined))?;\n\
         match request.to_rust_lossy().as_str() {{\n{dispatch}\
         _ => {{ let parent = {filename}; ::nativeboat_native::modules::require_unlinked(realm, &Value::String(request), &parent) }}\n}}\n}});\n\
         function.define(Key::name(\"main\"), ::nativeboat_native::Property::data({main_module}));\n\
         let resolve = realm.function(\"resolve\", 1, false, move |realm, _, args| {{\n\
         let request = ops::to_string(realm, args.first().unwrap_or(&Value::Undefined))?;\n\
         match request.to_rust_lossy().as_str() {{\n{resolutions}\
         _ => {{ let message = ::alloc::format!(\"Cannot find module '{{}}'\", request.to_rust_lossy()); let error = realm.error(ErrorKind::Error, &message); error.define(Key::name(\"code\"), ::nativeboat_native::Property::data(Value::str(\"MODULE_NOT_FOUND\"))); Err(Thrown(Value::Object(error))) }}\n}}\n}});\n\
         function.define(Key::name(\"resolve\"), ::nativeboat_native::Property::data(Value::Object(resolve)));\n\
         Value::Object(function) }};\n\
         ::nativeboat_native::modules::define_commonjs_module(realm, &module, {dirname}, &require)?;\n\
         let factory = binding::read(realm, &cells[{factory}], \"*factory*\")?;\n\
         if let Err(error) = ops::call(realm, &factory, exports.clone(), &[exports, require, module.clone(), {filename}, {dirname}]) {{\n\
         *cache[{index}].borrow_mut() = None; return Err(error);\n}}\n\
         ops::set(realm, &module, &Key::name(\"loaded\"), Value::Boolean(true))?;\n\
         let value = ops::get(realm, &module, &Key::name(\"exports\"))?;\n\
         binding::initialize(&cells[{exported}], value.clone());\n\
         Ok(value)\n}}\n"
    )
}
