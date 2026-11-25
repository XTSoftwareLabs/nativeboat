//! Configuration files: `tsconfig.json`, Vite aliases and `.env` files.

pub(crate) mod evaluate;

use nativeboat_bundler::{Alias, AliasKind};
use oxc::allocator::Allocator;
use oxc::ast::ast::{
    Argument, ArrayExpressionElement, Expression, ObjectPropertyKind, PropertyKey,
};
use oxc::ast_visit::{Visit, walk};
use oxc::parser::Parser;
use oxc::span::SourceType;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The next character that is neither white space nor part of a comment.
fn next_token(characters: &[char]) -> Option<char> {
    let mut index = 0;
    while index < characters.len() {
        match (characters[index], characters.get(index + 1)) {
            ('/', Some('/')) => {
                while index < characters.len() && characters[index] != '\n' {
                    index += 1;
                }
            }
            ('/', Some('*')) => {
                index += 2;
                while index < characters.len()
                    && !(characters[index] == '*' && characters.get(index + 1) == Some(&'/'))
                {
                    index += 1;
                }
                index += 2;
            }
            (character, _) if character.is_whitespace() => index += 1,
            (character, _) => return Some(character),
        }
    }
    None
}

/// Removes comments and trailing commas so JSONC parses as JSON.
pub fn strip_jsonc(text: &str) -> String {
    let characters: Vec<char> = text.chars().collect();
    let mut output = String::with_capacity(text.len());
    let mut index = 0;
    let mut in_string = false;
    while index < characters.len() {
        let character = characters[index];
        if in_string {
            output.push(character);
            if character == '\\' && index + 1 < characters.len() {
                output.push(characters[index + 1]);
                index += 1;
            } else if character == '"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        match (character, characters.get(index + 1)) {
            ('"', _) => {
                in_string = true;
                output.push(character);
            }
            ('/', Some('/')) => {
                while index < characters.len() && characters[index] != '\n' {
                    index += 1;
                }
                continue;
            }
            ('/', Some('*')) => {
                index += 2;
                while index < characters.len()
                    && !(characters[index] == '*' && characters.get(index + 1) == Some(&'/'))
                {
                    index += 1;
                }
                index += 2;
                continue;
            }
            (',', _) => {
                if !matches!(next_token(&characters[index + 1..]), Some('}' | ']')) {
                    output.push(character);
                }
            }
            _ => output.push(character),
        }
        index += 1;
    }
    output
}

/// Reads a JSON or JSONC file.
pub fn read_jsonc(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&strip_jsonc(&text)).ok()
}

/// Reads YAML configuration, including anchors and merge keys, as static data.
pub fn read_yaml(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let mut value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&text)
        .map_err(|error| format!("invalid {}: {error}", path.display()))?;
    value
        .apply_merge()
        .map_err(|error| format!("invalid {}: {error}", path.display()))?;
    serde_json::to_value(value).map_err(|error| format!("invalid {}: {error}", path.display()))
}

/// Collects `compilerOptions.paths` aliases from the project's tsconfig files.
pub fn tsconfig_aliases(root: &Path) -> Vec<Alias> {
    let mut aliases = Vec::new();
    for name in TSCONFIGS {
        let Some(config) = read_jsonc(&root.join(name)) else {
            continue;
        };
        let options = &config["compilerOptions"];
        let base = options["baseUrl"].as_str().unwrap_or(".");
        let Some(paths) = options["paths"].as_object() else {
            continue;
        };
        for (pattern, targets) in paths {
            if let Some(target) = targets.get(0).and_then(Value::as_str) {
                let target = nativeboat_bundler::resolve::normalize(&Path::new(base).join(target))
                    .to_string_lossy()
                    .replace('\\', "/");
                if !aliases
                    .iter()
                    .any(|alias: &Alias| alias.pattern == *pattern)
                {
                    aliases.push(Alias {
                        pattern: pattern.clone(),
                        target,
                        kind: AliasKind::Path,
                    });
                }
            }
        }
    }
    aliases
}

/// The TypeScript and JavaScript configurations of a project, in the order
/// their settings are taken.
const TSCONFIGS: [&str; 5] = [
    "tsconfig.json",
    "tsconfig.app.json",
    "tsconfig.web.json",
    "tsconfig.node.json",
    "jsconfig.json",
];

/// The compiler options of the configurations that compile the project's
/// TypeScript, each with its `extends` chain applied: those at the root, in
/// the order their settings are taken, or else the nearest `tsconfig.json`
/// above the main process's source, as VS Code keeps its configuration in
/// `src`.
pub fn compiler_options(root: &Path, source: &Path) -> Vec<Value> {
    let at_root: Vec<Value> = TSCONFIGS
        .iter()
        .filter_map(|name| crate::tsconfig::compiler_options(&root.join(name)))
        .collect();
    if !at_root.is_empty() {
        return at_root;
    }
    source
        .ancestors()
        .skip(1)
        .take_while(|directory| directory.starts_with(root))
        .find_map(|directory| crate::tsconfig::compiler_options(&directory.join("tsconfig.json")))
        .into_iter()
        .collect()
}

/// Whether the project compiles TypeScript's `experimentalDecorators`, and
/// with `emitDecoratorMetadata`, from the first configuration that enables
/// them.
pub fn tsconfig_decorators(configs: &[Value]) -> (bool, bool) {
    configs
        .iter()
        .find(|options| options["experimentalDecorators"] == true)
        .map_or((false, false), |options| {
            (true, options["emitDecoratorMetadata"] == true)
        })
}

/// Whether TypeScript classes assign their fields, as with
/// `useDefineForClassFields: false`: the first configuration's setting, or
/// else as `tsc` and esbuild default it, defining fields for an `ES2022`
/// or later `target` and assigning them for an earlier or missing one.
/// Without a configuration, fields are defined as in JavaScript.
pub fn tsconfig_assigns_fields(configs: &[Value]) -> bool {
    if let Some(define) = configs
        .iter()
        .find_map(|options| options["useDefineForClassFields"].as_bool())
    {
        return !define;
    }
    if configs.is_empty() {
        return false;
    }
    let target = configs
        .iter()
        .find_map(|options| options["target"].as_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let year = target
        .strip_prefix("es")
        .and_then(|version| version.parse::<u32>().ok());
    !(target == "esnext" || year.is_some_and(|year| year >= 2022))
}

/// How the project's TypeScript configuration compiles modules: each setting
/// from the first configuration that gives it.
pub fn tsconfig_modules(configs: &[Value]) -> crate::TypeScript {
    let setting = |name: &str| {
        configs
            .iter()
            .find_map(|options| options[name].as_str().map(str::to_owned))
    };
    crate::TypeScript {
        jsx: setting("jsx"),
        jsx_import_source: setting("jsxImportSource"),
        jsx_factory: setting("jsxFactory"),
        jsx_fragment_factory: setting("jsxFragmentFactory"),
        verbatim_module_syntax: configs
            .iter()
            .find_map(|options| options["verbatimModuleSyntax"].as_bool())
            .unwrap_or(false),
        assign_class_fields: tsconfig_assigns_fields(configs),
    }
}

/// Finds the last string literal inside an expression such as
/// `path.resolve(__dirname, './src')` or `new URL('./src', import.meta.url)`.
fn path_literal(expression: &Expression<'_>) -> Option<String> {
    match expression {
        Expression::StringLiteral(literal) => Some(literal.value.to_string()),
        Expression::CallExpression(call) => call.arguments.iter().rev().find_map(argument_literal),
        Expression::NewExpression(new) => new.arguments.iter().find_map(argument_literal),
        _ => None,
    }
}

fn argument_literal(argument: &Argument<'_>) -> Option<String> {
    argument.as_expression().and_then(path_literal)
}

fn key_name(key: &PropertyKey<'_>) -> Option<String> {
    match key {
        PropertyKey::StaticIdentifier(identifier) => Some(identifier.name.to_string()),
        PropertyKey::StringLiteral(literal) => Some(literal.value.to_string()),
        _ => None,
    }
}

#[derive(Default)]
struct AliasCollector {
    aliases: Vec<(String, String)>,
}

impl AliasCollector {
    fn object(&mut self, expression: &Expression<'_>) {
        match expression {
            Expression::ObjectExpression(object) => {
                for property in &object.properties {
                    if let ObjectPropertyKind::ObjectProperty(property) = property
                        && let (Some(key), Some(target)) =
                            (key_name(&property.key), path_literal(&property.value))
                    {
                        self.aliases.push((key, target));
                    }
                }
            }
            Expression::ArrayExpression(array) => {
                for element in &array.elements {
                    let ArrayExpressionElement::ObjectExpression(object) = element else {
                        continue;
                    };
                    let mut find = None;
                    let mut replacement = None;
                    for property in &object.properties {
                        if let ObjectPropertyKind::ObjectProperty(property) = property {
                            match key_name(&property.key).as_deref() {
                                Some("find") => find = path_literal(&property.value),
                                Some("replacement") => replacement = path_literal(&property.value),
                                _ => {}
                            }
                        }
                    }
                    if let (Some(find), Some(replacement)) = (find, replacement) {
                        self.aliases.push((find, replacement));
                    }
                }
            }
            _ => {}
        }
    }
}

impl<'a> Visit<'a> for AliasCollector {
    fn visit_object_property(&mut self, property: &oxc::ast::ast::ObjectProperty<'a>) {
        if key_name(&property.key).as_deref() == Some("alias") {
            self.object(&property.value);
        }
        walk::walk_object_property(self, property);
    }
}

/// Extracts `resolve.alias` entries from a Vite configuration file.
///
/// Each alias `@` → `./src` becomes the patterns `@` and `@/*`.
pub fn vite_aliases(config: &Path, root: &Path) -> Vec<Alias> {
    let Ok(source) = std::fs::read_to_string(config) else {
        return Vec::new();
    };
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(config).unwrap_or_default();
    let parsed = Parser::new(&allocator, &source, source_type).parse();
    let mut collector = AliasCollector::default();
    collector.visit_program(&parsed.program);
    let directory = config.parent().unwrap_or(root);
    let mut aliases = Vec::new();
    for (find, replacement) in collector.aliases {
        let target = directory.join(&replacement);
        let target = target
            .strip_prefix(root)
            .map(Path::to_path_buf)
            .unwrap_or(target);
        let target = nativeboat_bundler::resolve::normalize(&target)
            .to_string_lossy()
            .replace('\\', "/");
        let find = find.trim_end_matches('/').to_owned();
        aliases.push(Alias {
            pattern: find.clone(),
            target: target.clone(),
            kind: AliasKind::Path,
        });
        aliases.push(Alias {
            pattern: format!("{find}/*"),
            target: format!("{target}/*"),
            kind: AliasKind::Path,
        });
    }
    aliases
}

/// Parses `.env` file content.
pub fn parse_env(text: &str) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        let value = match value.chars().next() {
            Some(quote @ ('"' | '\'')) if value.len() >= 2 && value.ends_with(quote) => {
                value[1..value.len() - 1].to_owned()
            }
            _ => value.split(" #").next().unwrap_or(value).trim().to_owned(),
        };
        values.insert(key.trim().to_owned(), value);
    }
    values
}

/// Loads the production `.env` files in Vite's order; later files win.
pub fn load_env(directory: &Path) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for name in [
        ".env",
        ".env.local",
        ".env.production",
        ".env.production.local",
    ] {
        if let Ok(text) = std::fs::read_to_string(directory.join(name)) {
            values.extend(parse_env(&text));
        }
    }
    values
}

/// Returns the first existing file among the candidates.
pub fn first_existing(root: &Path, candidates: &[&str]) -> Option<PathBuf> {
    candidates
        .iter()
        .map(|candidate| root.join(candidate))
        .find(|path| path.is_file())
}

#[cfg(test)]
mod tests;
