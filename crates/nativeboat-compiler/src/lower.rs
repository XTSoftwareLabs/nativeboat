//! Lexical binding, scalar typing and native control flow before code generation.

use crate::ir::{Block, Expr, Function as NativeFunction, Kind, NativeModule, Scalar, Stmt};
use crate::{Diagnostic, Source};
use oxc::ast::ast::*;
use oxc::semantic::{Scoping, SymbolId};
use oxc::span::{GetSpan, Span};
use oxc::syntax::operator::{AssignmentOperator, BinaryOperator, LogicalOperator, UnaryOperator};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy)]
enum Entry<'a> {
    Function(FunctionEntry<'a>, bool),
    Constant(&'a BindingIdentifier<'a>, &'a VariableDeclarator<'a>),
}

#[derive(Clone, Copy)]
enum FunctionEntry<'a> {
    Declaration(&'a Function<'a>),
    Arrow(&'a BindingIdentifier<'a>, &'a ArrowFunctionExpression<'a>),
}

#[derive(Clone, Copy)]
enum Body<'a> {
    Block(&'a FunctionBody<'a>),
    Expression(&'a Expression<'a>),
}

#[derive(Clone)]
struct Definition<'a> {
    binding: &'a BindingIdentifier<'a>,
    parameters: Vec<(usize, Scalar)>,
    result: Option<Scalar>,
    body: Body<'a>,
    exported: bool,
    span: Span,
}

#[derive(Clone, Copy)]
struct Binding {
    scalar: Scalar,
    mutable: bool,
}

struct Lower<'a, 's> {
    source: &'s Source<'s>,
    scoping: &'s Scoping,
    bindings: HashMap<SymbolId, Binding>,
    entries: Vec<Entry<'a>>,
    definitions: HashMap<SymbolId, Definition<'a>>,
    native: HashMap<SymbolId, NativeFunction>,
    active: HashSet<SymbolId>,
}

pub(crate) fn module<'a>(
    program: &'a Program<'a>,
    scoping: &Scoping,
    source: &Source<'_>,
) -> Result<NativeModule, Diagnostic> {
    let mut lower = Lower {
        source,
        scoping,
        bindings: HashMap::new(),
        entries: Vec::new(),
        definitions: HashMap::new(),
        native: HashMap::new(),
        active: HashSet::new(),
    };
    for statement in &program.body {
        match statement {
            Statement::VariableDeclaration(declaration) => lower.variables(declaration, false)?,
            Statement::FunctionDeclaration(function) => lower
                .entries
                .push(Entry::Function(FunctionEntry::Declaration(function), false)),
            Statement::ExportDeclaration(export) => match &export.declaration {
                Declaration::VariableDeclaration(declaration) => {
                    lower.variables(declaration, true)?
                }
                Declaration::FunctionDeclaration(function) => lower
                    .entries
                    .push(Entry::Function(FunctionEntry::Declaration(function), true)),
                _ => {
                    return Err(source.error(
                        export.span,
                        "declaration is outside the native scalar subset",
                    ));
                }
            },
            _ => {
                return Err(source.error(
                    statement.span(),
                    "statement is outside the native scalar subset",
                ));
            }
        }
    }
    let mut module = NativeModule {
        constants: Vec::new(),
        functions: Vec::new(),
    };
    let entries = std::mem::take(&mut lower.entries);
    let mut order = Vec::new();
    for entry in entries {
        match entry {
            Entry::Constant(binding, declaration) => {
                let expression = declaration.init.as_ref().expect("checked initializer");
                if !matches!(
                    expression,
                    Expression::NumericLiteral(_) | Expression::BooleanLiteral(_)
                ) {
                    return Err(source.error(
                        expression.span(),
                        "module constants must be numeric or boolean literals",
                    ));
                }
                let value = lower.expression(expression)?;
                lower.check_annotation(declaration.type_annotation.as_deref(), value.scalar)?;
                lower.bindings.insert(
                    binding.symbol_id(),
                    Binding {
                        scalar: value.scalar,
                        mutable: false,
                    },
                );
                module.constants.push((binding.symbol_id().index(), value));
            }
            Entry::Function(entry, exported) => {
                let definition = lower.definition(entry, exported)?;
                let id = definition.binding.symbol_id();
                order.push(id);
                lower.definitions.insert(id, definition);
            }
        }
    }
    for id in &order {
        lower.function(*id)?;
    }
    for id in order {
        module
            .functions
            .push(lower.native.remove(&id).expect("checked native function"));
    }
    Ok(module)
}

impl<'a> Lower<'a, '_> {
    fn variables(
        &mut self,
        declaration: &'a VariableDeclaration<'a>,
        exported: bool,
    ) -> Result<(), Diagnostic> {
        if declaration.kind != VariableDeclarationKind::Const || declaration.declare {
            return Err(self.source.error(
                declaration.span,
                "only initialized const declarations are compiled yet",
            ));
        }
        for variable in &declaration.declarations {
            let BindingPattern::BindingIdentifier(binding) = &variable.id else {
                return Err(self.source.error(
                    variable.span,
                    "destructured declarations are not compiled yet",
                ));
            };
            let initializer = variable
                .init
                .as_ref()
                .expect("parser validates non-ambient const initialization");
            if let Expression::ArrowFunctionExpression(function) = initializer {
                if variable.type_annotation.is_some() {
                    return Err(self.source.error(
                        variable.span,
                        "function-valued binding annotations are not compiled yet",
                    ));
                }
                self.entries.push(Entry::Function(
                    FunctionEntry::Arrow(binding, function),
                    exported,
                ));
            } else {
                self.entries.push(Entry::Constant(binding, variable));
            }
        }
        Ok(())
    }

    fn definition(
        &self,
        entry: FunctionEntry<'a>,
        exported: bool,
    ) -> Result<Definition<'a>, Diagnostic> {
        let (binding, parameters, return_type, body, span) = match entry {
            FunctionEntry::Declaration(function) => {
                if function.r#async
                    || function.generator
                    || function.declare
                    || function.type_parameters.is_some()
                    || function.this_param.is_some()
                {
                    return Err(self.source.error(
                        function.span,
                        "async, generator, declared or generic functions are not compiled yet",
                    ));
                }
                let body = function.body.as_ref().ok_or(
                    self.source
                        .error(function.span, "native functions require a body"),
                )?;
                (
                    function.id.as_ref().expect("named declaration"),
                    &*function.params,
                    function.return_type.as_deref(),
                    Body::Block(body),
                    function.span,
                )
            }
            FunctionEntry::Arrow(binding, function) => {
                if function.r#async || function.type_parameters.is_some() {
                    return Err(self.source.error(
                        function.span,
                        "async or generic arrows are not compiled yet",
                    ));
                }
                let body = match &function.body {
                    ArrowFunctionBody::FunctionBody(body) => Body::Block(body),
                    body => Body::Expression(body.as_expression().expect("expression arrow")),
                };
                (
                    binding,
                    &*function.params,
                    function.return_type.as_deref(),
                    body,
                    function.span,
                )
            }
        };
        if parameters.rest.is_some() {
            return Err(self
                .source
                .error(parameters.span, "rest parameters are not compiled yet"));
        }
        let mut native_parameters = Vec::new();
        for parameter in &parameters.items {
            let BindingPattern::BindingIdentifier(binding) = &parameter.pattern else {
                return Err(self.source.error(
                    parameter.span,
                    "destructured parameters are not compiled yet",
                ));
            };
            if parameter.optional
                || parameter.initializer.is_some()
                || !parameter.decorators.is_empty()
            {
                return Err(self.source.error(
                    parameter.span,
                    "optional, defaulted or decorated parameters are not compiled yet",
                ));
            }
            let annotation = parameter
                .type_annotation
                .as_deref()
                .ok_or(self.source.error(
                    parameter.span,
                    "native scalar parameters require a number or boolean annotation",
                ))?;
            native_parameters.push((binding.symbol_id().index(), self.scalar(annotation)?));
        }
        Ok(Definition {
            binding,
            parameters: native_parameters,
            result: return_type
                .map(|annotation| self.scalar(annotation))
                .transpose()?,
            body,
            exported,
            span,
        })
    }

    fn function(&mut self, id: SymbolId) -> Result<Scalar, Diagnostic> {
        if let Some(function) = self.native.get(&id) {
            return Ok(function.scalar);
        }
        let definition = self.definitions[&id].clone();
        if self.active.contains(&id) {
            return Err(self.source.error(
                definition.span,
                "recursive native calls require native stack-limit and exception semantics, which are not compiled yet",
            ));
        }
        self.active.insert(id);
        for (id, scalar) in &definition.parameters {
            self.bindings.insert(
                SymbolId::from_usize(*id),
                Binding {
                    scalar: *scalar,
                    mutable: true,
                },
            );
        }
        let body = match definition.body {
            Body::Block(body) => self.block(&body.statements)?,
            Body::Expression(expression) => {
                let value = self.expression(expression)?;
                Block {
                    result: Some(value.scalar),
                    returns: true,
                    statements: vec![Stmt::Return(value)],
                }
            }
        };
        if !body.returns {
            return Err(self.source.error(
                definition.span,
                "native scalar functions must return a value on every path",
            ));
        }
        let scalar = body.result.expect("all paths return typed values");
        if definition.result.is_some_and(|result| result != scalar) {
            return Err(self.source.error(
                definition.span,
                "native expression does not match its declared type",
            ));
        }
        self.active.remove(&id);
        self.native.insert(
            id,
            NativeFunction {
                source_name: definition.binding.name.to_string(),
                id: id.index(),
                exported: definition.exported,
                parameters: definition.parameters,
                scalar,
                body,
            },
        );
        Ok(scalar)
    }

    fn block(&mut self, statements: &[Statement<'a>]) -> Result<Block, Diagnostic> {
        let mut body = Block::default();
        for statement in statements {
            if body.returns {
                return Err(self.source.error(
                    statement.span(),
                    "unreachable statements are not compiled yet",
                ));
            }
            let part = self.statement(statement)?;
            self.merge(&mut body, &part, statement.span())?;
            body.statements.extend(part.statements);
            body.returns = part.returns;
        }
        Ok(body)
    }

    fn merge(&self, target: &mut Block, part: &Block, span: Span) -> Result<(), Diagnostic> {
        if let Some(scalar) = part.result {
            if target.result.is_some_and(|result| result != scalar) {
                return Err(self
                    .source
                    .error(span, "native return paths have different scalar types"));
            }
            target.result = Some(scalar);
        }
        Ok(())
    }

    fn statement(&mut self, statement: &Statement<'a>) -> Result<Block, Diagnostic> {
        let mut body = Block::default();
        let native = match statement {
            Statement::ReturnStatement(statement) => {
                let expression = statement.argument.as_ref().ok_or(self.source.error(
                    statement.span,
                    "native scalar functions must return a value",
                ))?;
                let value = self.expression(expression)?;
                body.result = Some(value.scalar);
                body.returns = true;
                Stmt::Return(value)
            }
            Statement::VariableDeclaration(declaration) => {
                if !matches!(
                    declaration.kind,
                    VariableDeclarationKind::Const | VariableDeclarationKind::Let
                ) || declaration.declare
                {
                    return Err(self.source.error(
                        declaration.span,
                        "native locals require initialized const or let declarations",
                    ));
                }
                for variable in &declaration.declarations {
                    let BindingPattern::BindingIdentifier(binding) = &variable.id else {
                        return Err(self
                            .source
                            .error(variable.span, "destructured locals are not compiled yet"));
                    };
                    let initializer = variable.init.as_ref().ok_or(
                        self.source
                            .error(variable.span, "native locals require an initializer"),
                    )?;
                    let value = self.expression(initializer)?;
                    self.check_annotation(variable.type_annotation.as_deref(), value.scalar)?;
                    let mutable = declaration.kind == VariableDeclarationKind::Let;
                    self.bindings.insert(
                        binding.symbol_id(),
                        Binding {
                            scalar: value.scalar,
                            mutable,
                        },
                    );
                    body.statements
                        .push(Stmt::Bind(binding.symbol_id().index(), value, mutable));
                }
                return Ok(body);
            }
            Statement::ExpressionStatement(statement) => {
                if let Expression::AssignmentExpression(assignment) = &statement.expression {
                    if assignment.operator != AssignmentOperator::Assign {
                        return Err(self
                            .source
                            .error(assignment.span, "compound assignments are not compiled yet"));
                    }
                    let AssignmentTarget::AssignmentTargetIdentifier(identifier) = &assignment.left
                    else {
                        return Err(self.source.error(
                            assignment.span,
                            "only scalar binding assignments are compiled yet",
                        ));
                    };
                    let id = self
                        .scoping
                        .get_reference(identifier.reference_id())
                        .symbol_id()
                        .ok_or(self.source.error(
                            identifier.span,
                            "assignment target is not a compiled scalar binding",
                        ))?;
                    let binding = self.bindings.get(&id).copied().ok_or(
                        self.source
                            .error(identifier.span, "assignment target is not initialized"),
                    )?;
                    if !binding.mutable {
                        return Err(self.source.error(
                            identifier.span,
                            "assignment to a const binding is not allowed",
                        ));
                    }
                    let value = self.expression(&assignment.right)?;
                    self.require(&value, binding.scalar, assignment.span)?;
                    Stmt::Assign(id.index(), value)
                } else {
                    Stmt::Evaluate(self.expression(&statement.expression)?)
                }
            }
            Statement::IfStatement(statement) => {
                let test = self.expression(&statement.test)?;
                self.require(&test, Scalar::Boolean, statement.test.span())?;
                let yes = self.statement(&statement.consequent)?;
                let no = statement
                    .alternate
                    .as_ref()
                    .map(|statement| self.statement(statement))
                    .transpose()?;
                self.merge(&mut body, &yes, statement.span)?;
                if let Some(no) = &no {
                    self.merge(&mut body, no, statement.span)?;
                }
                body.returns = yes.returns && no.as_ref().is_some_and(|no| no.returns);
                Stmt::If(test, yes, no)
            }
            Statement::BlockStatement(statement) => {
                let nested = self.block(&statement.body)?;
                body.result = nested.result;
                body.returns = nested.returns;
                Stmt::Block(nested)
            }
            Statement::EmptyStatement(_) => Stmt::Empty,
            _ => {
                return Err(self.source.error(
                    statement.span(),
                    "statement is outside the native scalar subset",
                ));
            }
        };
        body.statements.push(native);
        Ok(body)
    }

    fn scalar(&self, annotation: &TSTypeAnnotation<'_>) -> Result<Scalar, Diagnostic> {
        match annotation.type_annotation {
            TSType::TSNumberKeyword(_) => Ok(Scalar::Number),
            TSType::TSBooleanKeyword(_) => Ok(Scalar::Boolean),
            _ => Err(self.source.error(
                annotation.span,
                "only number and boolean scalar types are compiled yet",
            )),
        }
    }

    fn check_annotation(
        &self,
        annotation: Option<&TSTypeAnnotation<'_>>,
        scalar: Scalar,
    ) -> Result<(), Diagnostic> {
        if let Some(annotation) = annotation
            && self.scalar(annotation)? != scalar
        {
            return Err(self.source.error(
                annotation.span,
                "native expression does not match its declared type",
            ));
        }
        Ok(())
    }

    fn require(&self, value: &Expr, scalar: Scalar, span: Span) -> Result<(), Diagnostic> {
        if value.scalar != scalar {
            return Err(self
                .source
                .error(span, "native operation requires a different scalar type"));
        }
        Ok(())
    }

    fn call(&mut self, call: &CallExpression<'_>) -> Result<Expr, Diagnostic> {
        if call.optional || call.type_arguments.is_some() {
            return Err(self
                .source
                .error(call.span, "optional or generic calls are not compiled yet"));
        }
        let mut callee = &call.callee;
        while let Expression::ParenthesizedExpression(expression) = callee {
            callee = &expression.expression;
        }
        if let Expression::Identifier(identifier) = callee {
            let id = self
                .scoping
                .get_reference(identifier.reference_id())
                .symbol_id()
                .filter(|id| self.definitions.contains_key(id))
                .ok_or(self.source.error(
                    identifier.span,
                    "call target is not a compiled native function",
                ))?;
            let definition = self.definitions[&id].clone();
            if call.arguments.len() != definition.parameters.len() {
                return Err(self.source.error(
                    call.span,
                    "native function argument count does not match its parameters",
                ));
            }
            let mut arguments = Vec::new();
            for (argument, (_, scalar)) in call.arguments.iter().zip(&definition.parameters) {
                let argument = argument.as_expression().ok_or(
                    self.source
                        .error(argument.span(), "spread arguments are not compiled yet"),
                )?;
                let value = self.expression(argument)?;
                self.require(&value, *scalar, argument.span())?;
                arguments.push(value);
            }
            let scalar = self.function(id)?;
            return Ok(Expr {
                kind: Kind::Call(id.index(), arguments),
                scalar,
            });
        }
        let Expression::StaticMemberExpression(member) = callee else {
            return Err(self
                .source
                .error(call.span, "call target is not a compiled native intrinsic"));
        };
        let Expression::Identifier(object) = &member.object else {
            return Err(self.source.error(
                call.span,
                "intrinsic object must be the unshadowed Math binding",
            ));
        };
        if member.optional
            || object.name != "Math"
            || !matches!(member.property.name.as_str(), "abs" | "min" | "max")
            || self
                .scoping
                .get_reference(object.reference_id())
                .symbol_id()
                .is_some()
        {
            return Err(self.source.error(
                call.span,
                "only unshadowed Math.abs, Math.min and Math.max are compiled yet",
            ));
        }
        if member.property.name == "abs" && call.arguments.len() != 1 {
            return Err(self.source.error(
                call.span,
                "native Math.abs requires exactly one numeric argument",
            ));
        }
        let mut arguments = Vec::new();
        for argument in &call.arguments {
            let expression = argument.as_expression().ok_or(
                self.source
                    .error(argument.span(), "spread arguments are not compiled yet"),
            )?;
            let value = self.expression(expression)?;
            self.require(&value, Scalar::Number, argument.span())?;
            arguments.push(value);
        }
        let kind = if member.property.name == "abs" {
            Kind::Abs(Box::new(arguments.remove(0)))
        } else {
            Kind::Extremum(member.property.name == "max", arguments)
        };
        Ok(Expr {
            kind,
            scalar: Scalar::Number,
        })
    }

    fn expression(&mut self, expression: &Expression<'_>) -> Result<Expr, Diagnostic> {
        let number = Scalar::Number;
        let boolean = Scalar::Boolean;
        let span = expression.span();
        let (kind, scalar) = match expression {
            Expression::NumericLiteral(literal) => (Kind::Number(literal.value), number),
            Expression::BooleanLiteral(literal) => (Kind::Boolean(literal.value), boolean),
            Expression::Identifier(identifier) => {
                let symbol = self
                    .scoping
                    .get_reference(identifier.reference_id())
                    .symbol_id();
                let scalar = symbol
                    .and_then(|symbol| self.bindings.get(&symbol))
                    .map(|binding| binding.scalar)
                    .ok_or(
                        self.source
                            .error(span, "reference is not a compiled scalar binding"),
                    )?;
                (
                    Kind::Binding(symbol.expect("resolved binding").index()),
                    scalar,
                )
            }
            Expression::ParenthesizedExpression(parenthesized) => {
                return self.expression(&parenthesized.expression);
            }
            Expression::UnaryExpression(unary) => {
                let value = self.expression(&unary.argument)?;
                let (operator, scalar) = match unary.operator {
                    UnaryOperator::UnaryPlus => {
                        self.require(&value, number, span)?;
                        return Ok(value);
                    }
                    UnaryOperator::UnaryNegation => ("-", number),
                    UnaryOperator::LogicalNot => ("!", boolean),
                    _ => {
                        return Err(self
                            .source
                            .error(span, "unary operator is not compiled yet"));
                    }
                };
                self.require(&value, scalar, span)?;
                (Kind::Unary(operator, Box::new(value)), scalar)
            }
            Expression::BinaryExpression(binary) => {
                let left = self.expression(&binary.left)?;
                let right = self.expression(&binary.right)?;
                let (operator, input, output) = match binary.operator {
                    BinaryOperator::Addition => ("+", number, number),
                    BinaryOperator::Subtraction => ("-", number, number),
                    BinaryOperator::Multiplication => ("*", number, number),
                    BinaryOperator::Division => ("/", number, number),
                    BinaryOperator::Remainder => ("%", number, number),
                    BinaryOperator::LessThan => ("<", number, boolean),
                    BinaryOperator::LessEqualThan => ("<=", number, boolean),
                    BinaryOperator::GreaterThan => (">", number, boolean),
                    BinaryOperator::GreaterEqualThan => (">=", number, boolean),
                    BinaryOperator::StrictEquality => ("==", left.scalar, boolean),
                    BinaryOperator::StrictInequality => ("!=", left.scalar, boolean),
                    _ => {
                        return Err(self
                            .source
                            .error(span, "binary operator is not compiled yet"));
                    }
                };
                self.require(&left, input, span)?;
                self.require(&right, input, span)?;
                (
                    Kind::Binary(operator, Box::new(left), Box::new(right)),
                    output,
                )
            }
            Expression::LogicalExpression(logical) => {
                let operator = match logical.operator {
                    LogicalOperator::And => "&&",
                    LogicalOperator::Or => "||",
                    _ => {
                        return Err(self
                            .source
                            .error(span, "nullish expressions are not compiled yet"));
                    }
                };
                let left = self.expression(&logical.left)?;
                let right = self.expression(&logical.right)?;
                self.require(&left, boolean, span)?;
                self.require(&right, boolean, span)?;
                (
                    Kind::Binary(operator, Box::new(left), Box::new(right)),
                    boolean,
                )
            }
            Expression::ConditionalExpression(conditional) => {
                let test = self.expression(&conditional.test)?;
                self.require(&test, boolean, span)?;
                let yes = self.expression(&conditional.consequent)?;
                let no = self.expression(&conditional.alternate)?;
                self.require(&no, yes.scalar, span)?;
                let scalar = yes.scalar;
                (
                    Kind::Conditional(Box::new(test), Box::new(yes), Box::new(no)),
                    scalar,
                )
            }
            Expression::CallExpression(call) => return self.call(call),
            Expression::ChainExpression(chain) => {
                return Err(self.source.error(
                    chain.span,
                    "optional calls and members are not compiled yet",
                ));
            }
            _ => {
                return Err(self
                    .source
                    .error(span, "expression is outside the native scalar subset"));
            }
        };
        Ok(Expr { kind, scalar })
    }
}
