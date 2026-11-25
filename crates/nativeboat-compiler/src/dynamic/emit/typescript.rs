//! TypeScript's runtime declarations: enums and namespaces, emitted as
//! `tsc` emits them.
//!
//! An enum is an object of its members, with reverse entries from numeric
//! values to names. Member initializers that are constant expressions
//! (literals, arithmetic, bitwise and string operations, and earlier
//! members) are folded as `tsc` folds them; others run at run time.
//!
//! A namespace is an object bound before its body runs, as `tsc`'s
//! `(function (N) { … })(N || (N = {}))` binds it. Its body runs in its own
//! scope, and each exported declaration is assigned to the object as it is
//! declared. A namespace merging with an earlier class, function, enum or
//! namespace of its name adds to that object. Namespaces with only types
//! have no runtime value.

use super::{Emitter, number, rust_str};
use oxc::ast::ast::*;
use oxc::ast_visit::{Visit, walk};
use oxc::semantic::{Scoping, SymbolId};
use oxc::span::GetSpan;
use oxc::syntax::operator::{BinaryOperator, UnaryOperator};
use std::fmt::Write;

/// A folded enum member value.
#[derive(Debug, Clone, PartialEq)]
enum Constant {
    Number(f64),
    String(String),
}

impl Constant {
    fn rust(&self) -> String {
        match self {
            Self::Number(value) => number(*value),
            Self::String(text) => format!("Value::str({})", rust_str(text)),
        }
    }

    /// JavaScript's `ToString` for the values folding produces.
    fn text(&self) -> String {
        match self {
            Self::String(text) => text.clone(),
            Self::Number(value) => number_text(*value),
        }
    }
}

/// `Number::toString` for folded numbers: integers in plain decimal, others
/// as Rust prints them, which agrees for the values enums use.
fn number_text(value: f64) -> String {
    if value.is_nan() {
        "NaN".to_owned()
    } else if value.is_infinite() {
        if value > 0.0 { "Infinity" } else { "-Infinity" }.to_owned()
    } else if value == value.trunc() && value.abs() < 1e21 {
        format!("{}", value as i128)
    } else {
        format!("{value}")
    }
}

/// `ToInt32`.
fn int32(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    value.trunc().rem_euclid(4_294_967_296.0) as u32 as i32
}

/// `ToUint32`.
fn uint32(value: f64) -> u32 {
    int32(value) as u32
}

/// Folds a constant enum member expression as `tsc` does: literals, `+`,
/// `-`, `~`, the arithmetic, shift and bitwise operators, string
/// concatenation, template literals of constants, and references to the
/// enum's earlier members by name or as `Enum.member`.
fn fold(
    expression: &Expression<'_>,
    members: &[(String, Constant)],
    enum_name: &str,
) -> Option<Constant> {
    let member = |name: &str| {
        members
            .iter()
            .rev()
            .find(|(known, _)| known == name)
            .map(|(_, value)| value.clone())
    };
    match expression {
        Expression::NumericLiteral(literal) => Some(Constant::Number(literal.value)),
        Expression::StringLiteral(literal) => Some(Constant::String(literal.value.to_string())),
        Expression::TemplateLiteral(template) => {
            let mut text = String::new();
            for (index, quasi) in template.quasis.iter().enumerate() {
                text.push_str(quasi.value.cooked.as_ref()?.as_str());
                if let Some(expression) = template.expressions.get(index) {
                    text.push_str(&fold(expression, members, enum_name)?.text());
                }
            }
            Some(Constant::String(text))
        }
        Expression::ParenthesizedExpression(inner) => fold(&inner.expression, members, enum_name),
        Expression::Identifier(identifier) => member(identifier.name.as_str()),
        Expression::StaticMemberExpression(access) if matches!(&access.object, Expression::Identifier(object) if object.name == enum_name) => {
            member(access.property.name.as_str())
        }
        Expression::ComputedMemberExpression(access) if matches!(&access.object, Expression::Identifier(object) if object.name == enum_name) => {
            match &access.expression {
                Expression::StringLiteral(name) => member(name.value.as_str()),
                _ => None,
            }
        }
        Expression::UnaryExpression(unary) => {
            let Constant::Number(value) = fold(&unary.argument, members, enum_name)? else {
                return None;
            };
            Some(Constant::Number(match unary.operator {
                UnaryOperator::UnaryPlus => value,
                UnaryOperator::UnaryNegation => -value,
                UnaryOperator::BitwiseNot => f64::from(!int32(value)),
                _ => return None,
            }))
        }
        Expression::BinaryExpression(binary) => {
            let left = fold(&binary.left, members, enum_name)?;
            let right = fold(&binary.right, members, enum_name)?;
            if binary.operator == BinaryOperator::Addition
                && (matches!(left, Constant::String(_)) || matches!(right, Constant::String(_)))
            {
                return Some(Constant::String(left.text() + &right.text()));
            }
            let (Constant::Number(a), Constant::Number(b)) = (left, right) else {
                return None;
            };
            Some(Constant::Number(match binary.operator {
                BinaryOperator::Addition => a + b,
                BinaryOperator::Subtraction => a - b,
                BinaryOperator::Multiplication => a * b,
                BinaryOperator::Division => a / b,
                BinaryOperator::Remainder => a % b,
                BinaryOperator::Exponential => a.powf(b),
                BinaryOperator::ShiftLeft => f64::from(int32(a).wrapping_shl(uint32(b) & 31)),
                BinaryOperator::ShiftRight => f64::from(int32(a).wrapping_shr(uint32(b) & 31)),
                BinaryOperator::ShiftRightZeroFill => {
                    f64::from(uint32(a).wrapping_shr(uint32(b) & 31))
                }
                BinaryOperator::BitwiseAnd => f64::from(int32(a) & int32(b)),
                BinaryOperator::BitwiseOR => f64::from(int32(a) | int32(b)),
                BinaryOperator::BitwiseXOR => f64::from(int32(a) ^ int32(b)),
                _ => return None,
            }))
        }
        _ => None,
    }
}

/// Whether an expression refers to an enum member binding, which a run-time
/// initializer cannot read as a variable.
fn uses_enum_member(scoping: &Scoping, expression: &Expression<'_>) -> bool {
    struct Finder<'s> {
        scoping: &'s Scoping,
        found: bool,
    }
    impl<'a> Visit<'a> for Finder<'_> {
        fn visit_identifier_reference(&mut self, identifier: &IdentifierReference<'a>) {
            if let Some(symbol) = self
                .scoping
                .get_reference(identifier.reference_id())
                .symbol_id()
                && self.scoping.symbol_flags(symbol).is_enum_member()
            {
                self.found = true;
            }
        }
    }
    let mut finder = Finder {
        scoping,
        found: false,
    };
    walk::walk_expression(&mut finder, expression);
    finder.found
}

/// Whether a namespace has a runtime value: a statement other than types,
/// interfaces, ambient declarations and namespaces without one.
pub(in crate::dynamic) fn instantiated(declaration: &TSNamespaceDeclaration<'_>) -> bool {
    if declaration.declare {
        return false;
    }
    match &declaration.body {
        TSNamespaceDeclarationBody::TSNamespaceDeclaration(inner) => instantiated(inner),
        TSNamespaceDeclarationBody::TSModuleBlock(block) => block.body.iter().any(|statement| {
            let declaration = match statement {
                Statement::ExportDeclaration(export) => &export.declaration,
                Statement::TSTypeAliasDeclaration(_)
                | Statement::TSInterfaceDeclaration(_)
                | Statement::TSGlobalDeclaration(_)
                | Statement::TSExternalModuleDeclaration(_)
                | Statement::EmptyStatement(_) => return false,
                Statement::TSNamespaceDeclaration(inner) => return instantiated(inner),
                Statement::TSEnumDeclaration(declaration) => return !declaration.declare,
                Statement::VariableDeclaration(declaration) => return !declaration.declare,
                Statement::FunctionDeclaration(function) => return function.body.is_some(),
                Statement::ClassDeclaration(class) => return !class.declare,
                _ => return true,
            };
            match declaration {
                Declaration::TSTypeAliasDeclaration(_) | Declaration::TSInterfaceDeclaration(_) => {
                    false
                }
                Declaration::TSNamespaceDeclaration(inner) => instantiated(inner),
                Declaration::TSEnumDeclaration(declaration) => !declaration.declare,
                Declaration::VariableDeclaration(declaration) => !declaration.declare,
                Declaration::FunctionDeclaration(function) => function.body.is_some(),
                Declaration::ClassDeclaration(class) => !class.declare,
                _ => true,
            }
        }),
    }
}

/// Whether an `import X = Y` alias is used as a value, which `tsc` keeps;
/// one used only in types is elided, as its target may be a type.
pub(in crate::dynamic) fn alias_used(
    scoping: &Scoping,
    declaration: &TSImportEqualsDeclaration<'_>,
) -> bool {
    !declaration.import_kind.is_type()
        && scoping
            .get_resolved_references(declaration.id.symbol_id())
            .any(|reference| reference.flags().is_value())
}

impl Emitter<'_> {
    /// `import X = Y.Z`: a constant binding of the value the name reaches,
    /// as `tsc` emits `var X = Y.Z`. `exported` aliases are kept whether or
    /// not this module uses them.
    pub(super) fn import_equals(
        &mut self,
        declaration: &TSImportEqualsDeclaration<'_>,
        exported: bool,
    ) -> String {
        if declaration.import_kind.is_type()
            || (!exported && !alias_used(self.scoping, declaration))
        {
            return String::new();
        }
        let value = match &declaration.module_reference {
            TSModuleReference::IdentifierReference(identifier) => self.read(identifier),
            TSModuleReference::QualifiedName(name) => {
                self.qualified_value(&name.left, &name.right.name)
            }
            TSModuleReference::ExternalModuleReference(reference) => {
                // The linker binds the alias as an import of the module.
                if exported {
                    self.unsupported(
                        reference.span,
                        "exported TypeScript import = require() is not compiled yet",
                    );
                }
                return String::new();
            }
        };
        self.bind_declaration(declaration.id.symbol_id(), &value)
    }

    /// The value of `left.right`, a TypeScript qualified name.
    fn qualified_value(&mut self, left: &TSTypeName<'_>, right: &str) -> String {
        let object = match left {
            TSTypeName::IdentifierReference(identifier) => self.read(identifier),
            TSTypeName::QualifiedName(name) => self.qualified_value(&name.left, &name.right.name),
            TSTypeName::ThisExpression(_) => self.this_value(),
        };
        let get = self.get_name("&__object", right);
        format!("{{ let __object = {object}; {get} }}")
    }

    /// The current value of a symbol's binding, which is initialized.
    fn binding_value(&mut self, symbol: SymbolId, name: &str) -> String {
        if self.captured(symbol) {
            self.fail(&format!(
                "binding::read(realm, &c{}, {})",
                symbol.index(),
                rust_str(name)
            ))
        } else {
            format!("b{}.clone()", symbol.index())
        }
    }

    /// Initializes a declaration's binding with the Rust value `value`.
    fn bind_declaration(&mut self, symbol: SymbolId, value: &str) -> String {
        if self.captured(symbol) {
            format!("binding::initialize(&c{}, {value});\n", symbol.index())
        } else {
            self.frame().declared.insert(symbol);
            format!("let mut b{} = {value};\n", symbol.index())
        }
    }

    /// The object of an earlier declaration a new one merges with (an enum,
    /// a namespace, or a class or function a namespace extends).
    fn merged(&mut self, symbol: SymbolId) -> bool {
        let flags = self.scoping.symbol_flags(symbol);
        self.typescript_objects.contains(&symbol) || flags.is_class() || flags.is_function()
    }

    /// A TypeScript enum: an object of its members, with reverse entries
    /// for numeric values.
    pub(super) fn enum_declaration(&mut self, declaration: &TSEnumDeclaration<'_>) -> String {
        if declaration.declare {
            return String::new();
        }
        let symbol = declaration.id.symbol_id();
        let enum_name = declaration.id.name.as_str();
        let id = self.next;
        self.next += 1;
        let existing = self.typescript_objects.contains(&symbol);
        let mut code = if existing {
            let value = self.binding_value(symbol, enum_name);
            format!("let __enum{id} = {value};\n{{\nlet __enum = {value};\n")
        } else {
            format!("let __enum{id} = {{\nlet __enum = Value::Object(realm.object());\n")
        };
        let mut members: Vec<(String, Constant)> = Vec::new();
        let mut next = Some(0.0f64);
        let mut valid = true;
        for member in &declaration.body.members {
            let name = match &member.id {
                TSEnumMemberName::Identifier(name) => name.name.to_string(),
                TSEnumMemberName::String(name) | TSEnumMemberName::ComputedString(name) => {
                    name.value.to_string()
                }
                TSEnumMemberName::ComputedTemplateString(template)
                    if template.expressions.is_empty() && template.quasis.len() == 1 =>
                {
                    template.quasis[0]
                        .value
                        .cooked
                        .as_ref()
                        .map_or_else(String::new, |value| value.to_string())
                }
                TSEnumMemberName::ComputedTemplateString(_) => {
                    self.unsupported(
                        member.id.span(),
                        "computed TypeScript enum member names must be constant strings",
                    );
                    valid = false;
                    continue;
                }
            };
            let key = rust_str(&name);
            let folded = match &member.initializer {
                None => match next {
                    Some(value) => Some(Constant::Number(value)),
                    None => {
                        self.unsupported(
                            member.span,
                            "auto-incremented TypeScript enum members require a numeric predecessor",
                        );
                        valid = false;
                        continue;
                    }
                },
                Some(initializer) => fold(initializer, &members, enum_name),
            };
            match folded {
                Some(constant) => {
                    next = match &constant {
                        Constant::Number(value) => Some(value + 1.0),
                        Constant::String(_) => None,
                    };
                    let _ = writeln!(
                        code,
                        "ops::set(realm, &__enum, &Key::name({key}), {})?;",
                        constant.rust()
                    );
                    // Integers name themselves in decimal; other numbers by
                    // the runtime's `ToString`.
                    if let Constant::Number(value) = constant {
                        if value == value.trunc() && value.abs() < 1e21 {
                            let _ = writeln!(
                                code,
                                "ops::set(realm, &__enum, &Key::name({}), Value::str({key}))?;",
                                rust_str(&number_text(value))
                            );
                        } else {
                            let _ = writeln!(
                                code,
                                "{{ let __name = ops::to_string(realm, &{})?; ops::set(realm, &__enum, &Key::from_js(&__name), Value::str({key}))?; }}",
                                number(value)
                            );
                        }
                    }
                    members.push((name, constant));
                }
                None => {
                    let initializer = member.initializer.as_ref().expect("folded otherwise");
                    if uses_enum_member(self.scoping, initializer) {
                        self.unsupported(
                            initializer.span(),
                            "TypeScript enum initializers that read other members at run time are not compiled yet",
                        );
                        valid = false;
                        continue;
                    }
                    // `E[E["A"] = value] = "A"`: a value that is not a
                    // string also names the member.
                    let value = self.expression(initializer);
                    let _ = writeln!(
                        code,
                        "{{ let __value = {value}; ops::set(realm, &__enum, &Key::name({key}), __value.clone())?; if !matches!(__value, Value::String(_)) {{ let __name = ops::to_string(realm, &__value)?; ops::set(realm, &__enum, &Key::from_js(&__name), Value::str({key}))?; }} }}"
                    );
                    next = None;
                }
            }
        }
        if !valid {
            return String::new();
        }
        if existing {
            code.push_str("}\n");
            return code;
        }
        code.push_str("__enum\n};\n");
        code.push_str(&self.bind_declaration(symbol, &format!("__enum{id}")));
        self.typescript_objects.insert(symbol);
        code
    }

    /// A TypeScript namespace with a runtime value.
    pub(super) fn namespace(&mut self, declaration: &TSNamespaceDeclaration<'_>) -> String {
        if !instantiated(declaration) {
            return String::new();
        }
        let symbol = declaration.id.symbol_id();
        let name = declaration.id.name.to_string();
        let id = self.next;
        self.next += 1;
        let target = format!("__namespace{id}");
        let mut code = String::new();
        if self.merged(symbol) {
            let value = self.binding_value(symbol, &name);
            let _ = writeln!(code, "let {target} = {value};");
        } else {
            let _ = writeln!(code, "let {target} = Value::Object(realm.object());");
            code.push_str(&self.bind_declaration(symbol, &format!("{target}.clone()")));
            self.typescript_objects.insert(symbol);
        }
        let scope = declaration.scope_id();
        match &declaration.body {
            TSNamespaceDeclarationBody::TSModuleBlock(block) => {
                code.push_str(&self.namespace_body(scope, &block.body, &target));
            }
            // `namespace A.B { … }` is `namespace A { export namespace B { … } }`.
            TSNamespaceDeclarationBody::TSNamespaceDeclaration(inner) => {
                code.push_str("{\n");
                code.push_str(&self.scope_bindings(scope, &[]));
                code.push_str(&self.namespace(inner));
                let inner_symbol = inner.id.symbol_id();
                let inner_name = inner.id.name.to_string();
                code.push_str(&self.export_to(&target, inner_symbol, &inner_name));
                code.push_str("}\n");
            }
        }
        code
    }

    /// Assigns a binding's value to the namespace object `target`.
    fn export_to(&mut self, target: &str, symbol: SymbolId, name: &str) -> String {
        let value = self.binding_value(symbol, name);
        let set = self.fail(&format!(
            "ops::set(realm, &{target}, &Key::name({}), __value)",
            rust_str(name)
        ));
        format!("{{ let __value = {value}; {set}; }}\n")
    }

    /// A namespace's statements in its own scope, each exported declaration
    /// assigned to `target` where it is declared.
    fn namespace_body(
        &mut self,
        scope: oxc::semantic::ScopeId,
        statements: &[Statement<'_>],
        target: &str,
    ) -> String {
        let mut code = String::from("{\n");
        code.push_str(&self.scope_bindings(scope, &[]));
        code.push_str(
            &self.hoist(statements.iter().filter_map(|statement| match statement {
                Statement::FunctionDeclaration(function) => Some(&**function),
                Statement::ExportDeclaration(export) => match &export.declaration {
                    Declaration::FunctionDeclaration(function) => Some(&**function),
                    _ => None,
                },
                _ => None,
            })),
        );
        for statement in statements {
            let Statement::ExportDeclaration(export) = statement else {
                code.push_str(&self.statement(statement));
                continue;
            };
            let exported: Vec<(SymbolId, String)> = match &export.declaration {
                Declaration::VariableDeclaration(declaration) => {
                    if declaration.declare {
                        continue;
                    }
                    code.push_str(&self.variables(declaration));
                    declaration
                        .declarations
                        .iter()
                        .flat_map(|declarator| declarator.id.get_binding_identifiers())
                        .map(|binding| (binding.symbol_id(), binding.name.to_string()))
                        .collect()
                }
                Declaration::FunctionDeclaration(function) => match &function.id {
                    Some(binding) if function.body.is_some() => {
                        vec![(binding.symbol_id(), binding.name.to_string())]
                    }
                    _ => Vec::new(),
                },
                Declaration::ClassDeclaration(class) => {
                    if class.declare {
                        continue;
                    }
                    code.push_str(&self.class_declaration(class));
                    class
                        .id
                        .iter()
                        .map(|binding| (binding.symbol_id(), binding.name.to_string()))
                        .collect()
                }
                Declaration::TSEnumDeclaration(declaration) => {
                    if declaration.declare {
                        continue;
                    }
                    code.push_str(&self.enum_declaration(declaration));
                    vec![(declaration.id.symbol_id(), declaration.id.name.to_string())]
                }
                Declaration::TSNamespaceDeclaration(declaration) => {
                    if !instantiated(declaration) {
                        continue;
                    }
                    code.push_str(&self.namespace(declaration));
                    vec![(declaration.id.symbol_id(), declaration.id.name.to_string())]
                }
                Declaration::TSImportEqualsDeclaration(declaration) => {
                    if declaration.import_kind.is_type() {
                        continue;
                    }
                    code.push_str(&self.import_equals(declaration, true));
                    vec![(declaration.id.symbol_id(), declaration.id.name.to_string())]
                }
                Declaration::TSTypeAliasDeclaration(_) | Declaration::TSInterfaceDeclaration(_) => {
                    continue;
                }
                declaration => {
                    self.unsupported(
                        declaration.span(),
                        "this exported namespace declaration is not compiled yet",
                    );
                    continue;
                }
            };
            for (symbol, name) in exported {
                code.push_str(&self.export_to(target, symbol, &name));
            }
        }
        code.push_str("}\n");
        code
    }
}
