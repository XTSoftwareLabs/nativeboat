//! Static JavaScript configuration data. Unknown expressions remain distinct
//! from literal null so required settings can be diagnosed without execution.

use indexmap::IndexMap;
use oxc::allocator::Allocator;
use oxc::ast::ast::*;
use oxc::parser::Parser;
use oxc::span::SourceType;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::Path;

mod paths;
use paths::Function;
pub(crate) use paths::{bindings, module};

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(untagged)]
pub(crate) enum Data {
    Object(IndexMap<String, Data>),
    Array(Vec<Data>),
    Literal(Value),
    #[serde(skip)]
    Function(Function),
    #[serde(skip)]
    Url(url::Url),
    #[default]
    #[serde(skip)]
    Unknown,
}

impl Data {
    /// Partial data is enough for discovery of unrelated static settings.
    pub(crate) fn json(&self, strict: bool) -> Option<Value> {
        match self {
            Self::Literal(value) => Some(value.clone()),
            Self::Url(url) => Some(Value::String(url.to_string())),
            Self::Object(fields) => fields
                .iter()
                .map(|(key, value)| Some((key.clone(), value.json(strict)?)))
                .collect::<Option<Map<_, _>>>()
                .map(Value::Object),
            Self::Array(items) => items
                .iter()
                .map(|item| item.json(strict))
                .collect::<Option<Vec<_>>>()
                .map(Value::Array),
            Self::Unknown | Self::Function(_) if strict => None,
            Self::Unknown | Self::Function(_) => Some(Value::Null),
        }
    }

    pub(crate) fn property(&self, key: &str) -> Option<Self> {
        match self {
            Self::Object(fields) => fields.get(key).cloned(),
            Self::Url(url) if key == "href" => Some(Self::Literal(Value::String(url.to_string()))),
            _ => None,
        }
    }

    pub(crate) fn fields(self) -> Option<IndexMap<String, Self>> {
        match self {
            Self::Object(fields) => Some(fields),
            _ => None,
        }
    }

    /// JSON.stringify-compatible data, retaining object insertion order and
    /// JavaScript's integer-key ordering and number formatting.
    pub(crate) fn stringify(&self) -> Option<String> {
        match self {
            Self::Literal(Value::Number(number)) => Some(
                ryu_js::Buffer::new()
                    .format_finite(number.as_f64().unwrap())
                    .to_owned(),
            ),
            Self::Literal(value) => Some(value.to_string()),
            Self::Url(url) => Some(Value::String(url.to_string()).to_string()),
            Self::Array(items) => Some(format!(
                "[{}]",
                items
                    .iter()
                    .map(Self::stringify)
                    .collect::<Option<Vec<_>>>()?
                    .join(",")
            )),
            Self::Object(fields) => {
                let mut fields: Vec<_> = fields.iter().collect();
                fields.sort_by_key(|(key, _)| {
                    let index = key
                        .parse::<u32>()
                        .ok()
                        .filter(|index| *index < u32::MAX && index.to_string() == **key);
                    (index.is_none(), index.unwrap_or_default())
                });
                let fields = fields
                    .into_iter()
                    .map(|(key, value)| {
                        Some(format!(
                            "{}:{}",
                            Value::String(key.clone()),
                            value.stringify()?
                        ))
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(format!("{{{}}}", fields.join(",")))
            }
            Self::Unknown | Self::Function(_) => None,
        }
    }
}

fn primitive(value: Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text),
        Value::Number(number) => Some(
            ryu_js::Buffer::new()
                .format_finite(number.as_f64().unwrap())
                .to_owned(),
        ),
        Value::Null | Value::Bool(_) => Some(value.to_string()),
        _ => None,
    }
}

pub(crate) fn value(expression: &Expression<'_>, bindings: &BTreeMap<String, Data>) -> Data {
    match expression {
        Expression::StringLiteral(literal) => {
            Data::Literal(Value::String(literal.value.to_string()))
        }
        Expression::BooleanLiteral(literal) => Data::Literal(Value::Bool(literal.value)),
        Expression::NullLiteral(_) => Data::Literal(Value::Null),
        Expression::NumericLiteral(literal) => serde_json::Number::from_f64(literal.value)
            .map(Value::Number)
            .map(Data::Literal)
            .unwrap_or_default(),
        Expression::Identifier(identifier) => bindings
            .get(identifier.name.as_str())
            .cloned()
            .unwrap_or_default(),
        Expression::ImportMeta(_) => bindings.get("import.meta").cloned().unwrap_or_default(),
        Expression::StaticMemberExpression(member) => value(&member.object, bindings)
            .property(member.property.name.as_str())
            .unwrap_or_default(),
        Expression::ComputedMemberExpression(member) => {
            let Some(Value::String(key)) = value(&member.expression, bindings).json(true) else {
                return Data::Unknown;
            };
            value(&member.object, bindings)
                .property(&key)
                .unwrap_or_default()
        }
        Expression::TemplateLiteral(template) => {
            let mut text = String::new();
            for (index, quasi) in template.quasis.iter().enumerate() {
                let Some(cooked) = &quasi.value.cooked else {
                    return Data::Unknown;
                };
                text.push_str(cooked.as_str());
                if let Some(expression) = template.expressions.get(index) {
                    let Some(part) = value(expression, bindings).json(true).and_then(primitive)
                    else {
                        return Data::Unknown;
                    };
                    text.push_str(&part);
                }
            }
            Data::Literal(Value::String(text))
        }
        Expression::ObjectExpression(object) => {
            let mut result = IndexMap::new();
            for property in &object.properties {
                match property {
                    ObjectPropertyKind::ObjectProperty(property) => {
                        if let Some(name) = property.key.static_name() {
                            if name == "__proto__" && !property.computed && !property.shorthand {
                                return Data::Unknown;
                            }
                            result.insert(name.to_string(), value(&property.value, bindings));
                        } else {
                            return Data::Unknown;
                        }
                    }
                    ObjectPropertyKind::SpreadProperty(spread) => {
                        if let Some(fields) = value(&spread.argument, bindings).fields() {
                            result.extend(fields);
                        } else {
                            return Data::Unknown;
                        }
                    }
                }
            }
            Data::Object(result)
        }
        Expression::ArrayExpression(array) => Data::Array(
            array
                .elements
                .iter()
                .map(|element| {
                    element
                        .as_expression()
                        .map_or(Data::Unknown, |expression| value(expression, bindings))
                })
                .collect(),
        ),
        Expression::CallExpression(call) => {
            let Data::Function(function) = value(&call.callee, bindings) else {
                return Data::Unknown;
            };
            if function == Function::DefineConfig {
                return call
                    .arguments
                    .first()
                    .and_then(Argument::as_expression)
                    .map_or(Data::Unknown, |expression| value(expression, bindings));
            }
            if function == Function::Url {
                return Data::Unknown;
            }
            paths::call(function, &call.arguments, bindings)
        }
        Expression::NewExpression(new)
            if matches!(value(&new.callee, bindings), Data::Function(Function::Url)) =>
        {
            paths::call(Function::Url, &new.arguments, bindings)
        }
        Expression::ArrowFunctionExpression(arrow) => match &arrow.body {
            ArrowFunctionBody::FunctionBody(body) if body.statements.len() == 1 => {
                match &body.statements[0] {
                    Statement::ReturnStatement(statement) => statement
                        .argument
                        .as_ref()
                        .map_or(Data::Unknown, |expression| value(expression, bindings)),
                    _ => Data::Unknown,
                }
            }
            body => body
                .as_expression()
                .map_or(Data::Unknown, |expression| value(expression, bindings)),
        },
        Expression::ParenthesizedExpression(inner) => value(&inner.expression, bindings),
        Expression::TSAsExpression(inner) => value(&inner.expression, bindings),
        Expression::TSSatisfiesExpression(inner) => value(&inner.expression, bindings),
        _ => Data::Unknown,
    }
}

#[cfg(test)]
mod tests;

/// The value a JavaScript or TypeScript configuration module exports, as far
/// as it can be known without running it: `export default` or
/// `module.exports`, with static JSON imports and known modules. `what`
/// names the configuration in errors.
pub(crate) fn exported(path: &Path, source: &str, root: &Path, what: &str) -> Result<Data, String> {
    let allocator = Allocator::default();
    let parsed = Parser::new(
        &allocator,
        source,
        SourceType::from_path(path).unwrap_or_default(),
    )
    .parse();
    if !parsed.diagnostics.is_empty() {
        return Err(format!("invalid {what} {}", path.display()));
    }
    let mut bindings = bindings(path);
    let mut result = Data::Unknown;
    for statement in &parsed.program.body {
        match statement {
            Statement::ImportDeclaration(import)
                if import.source.value.ends_with(".json")
                    && import.import_kind == ImportOrExportKind::Value
                    && (import.source.value.starts_with('.')
                        || Path::new(import.source.value.as_str()).is_absolute()) =>
            {
                let source = root.join(import.source.value.as_str());
                let text = std::fs::read_to_string(&source)
                    .map_err(|error| format!("cannot read {}: {error}", source.display()))?;
                let data: Data = serde_json::from_str(&text).map_err(|error| {
                    format!("invalid static JSON import {}: {error}", source.display())
                })?;
                for specifier in import.specifiers.iter().flatten() {
                    let (name, value) = match specifier {
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(default) => {
                            (&default.local.name, data.clone())
                        }
                        ImportDeclarationSpecifier::ImportSpecifier(named) => (
                            &named.local.name,
                            data.property(named.imported.name().as_str())
                                .unwrap_or_default(),
                        ),
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(namespace) => {
                            let mut fields = data.clone().fields().unwrap_or_default();
                            fields.insert("default".to_owned(), data.clone());
                            (&namespace.local.name, Data::Object(fields))
                        }
                    };
                    bindings.insert(name.to_string(), value);
                }
            }
            Statement::ImportDeclaration(import)
                if import.import_kind == ImportOrExportKind::Value =>
            {
                let data = module(import.source.value.as_str()).unwrap_or_default();
                for specifier in import.specifiers.iter().flatten() {
                    let (name, data) = match specifier {
                        ImportDeclarationSpecifier::ImportSpecifier(named) => (
                            &named.local.name,
                            data.property(named.imported.name().as_str())
                                .unwrap_or_default(),
                        ),
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(default) => {
                            (&default.local.name, data.clone())
                        }
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(namespace) => {
                            (&namespace.local.name, data.clone())
                        }
                    };
                    bindings.insert(name.to_string(), data);
                }
            }
            Statement::VariableDeclaration(declaration) => {
                for declaration in &declaration.declarations {
                    if let BindingPattern::BindingIdentifier(name) = &declaration.id
                        && let Some(initializer) = &declaration.init
                    {
                        bindings.insert(name.name.to_string(), value(initializer, &bindings));
                    }
                }
            }
            Statement::ExportDefaultDeclaration(export) => {
                result = export
                    .declaration
                    .as_expression()
                    .map_or(Data::Unknown, |expression| value(expression, &bindings));
            }
            Statement::ExpressionStatement(statement) => {
                if let Expression::AssignmentExpression(assignment) = &statement.expression
                    && let AssignmentTarget::StaticMemberExpression(member) = &assignment.left
                    && matches!(&member.object, Expression::Identifier(name) if name.name == "module")
                    && member.property.name == "exports"
                {
                    result = value(&assignment.right, &bindings);
                }
            }
            _ => {}
        }
    }
    Ok(result)
}
