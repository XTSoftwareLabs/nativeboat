//! Scans main-process sources for the pages and preload scripts they load.

use nativeboat_bundler::{Resolved, Resolver};
use oxc::allocator::Allocator;
use oxc::ast::ast::{
    Argument, AssignmentExpression, AssignmentOperator, AssignmentTarget, BindingPattern,
    CallExpression, Expression, ImportDeclaration, ImportExpression, ObjectProperty, PropertyKey,
    StaticMemberExpression, StringLiteral, TemplateLiteral, VariableDeclarator,
};
use oxc::ast_visit::{Visit, walk};
use oxc::parser::Parser;
use oxc::span::SourceType;
use std::path::{Path, PathBuf};

/// A path literal found in a source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// Directory of the file containing the literal.
    pub directory: PathBuf,
    /// The literal text.
    pub literal: String,
}

/// What the main-process sources reference.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scan {
    /// Local scripts reachable from the entry, and the stylesheets they
    /// import.
    pub files: Vec<PathBuf>,
    /// String literals naming HTML files.
    pub pages: Vec<Reference>,
    /// Paths inside `preload` options and paths of files named like preload
    /// scripts.
    pub preloads: Vec<Reference>,
    /// Modules started as child processes: the paths given to `fork()`, as
    /// Electron's `utilityProcess.fork` and Node's `child_process.fork`
    /// take them.
    pub forks: Vec<Reference>,
    /// Identifiers the Electron Forge Vite plugin defines, such as `MAIN_WINDOW_VITE_NAME`.
    pub forge_globals: Vec<String>,
    /// Whether the sources name Electron's `BrowserWindow`, so the
    /// application opens windows.
    pub opens_windows: bool,
}

#[derive(Default)]
struct Collector {
    opens_windows: bool,
    specifiers: Vec<String>,
    pages: Vec<String>,
    preloads: Vec<String>,
    forge_globals: Vec<String>,
    /// Variables initialized with or assigned a path, by name.
    bindings: Vec<(String, String)>,
    /// Variables used as `preload` options.
    preload_variables: Vec<String>,
    /// Paths given to `fork()`.
    forks: Vec<String>,
    /// Variables given to `fork()`.
    fork_variables: Vec<String>,
    /// Variables `import()` loads, as in `import(workbenchUrl)`.
    import_variables: Vec<String>,
}

/// Whether a call joins path segments, such as `path.join(...)`,
/// `resolve(...)` or `require.resolve(...)`.
fn is_path_join(call: &CallExpression<'_>) -> bool {
    let name = match &call.callee {
        Expression::Identifier(callee) => callee.name.as_str(),
        Expression::StaticMemberExpression(member) => member.property.name.as_str(),
        _ => return false,
    };
    matches!(name, "join" | "resolve")
}

/// The path a path-joining call builds from its literal segments; leading
/// segments that are not literals, such as `__dirname` or
/// `app.getAppPath()`, are where the path starts.
fn joined(call: &CallExpression<'_>) -> Option<String> {
    let mut parts = Vec::new();
    for argument in &call.arguments {
        match argument.as_expression().and_then(literal_of) {
            Some(part) => parts.push(part),
            None if parts.is_empty() => {}
            None => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn literal_of(expression: &Expression<'_>) -> Option<String> {
    match expression {
        Expression::StringLiteral(literal) => Some(literal.value.to_string()),
        Expression::TemplateLiteral(template) => template_path(template),
        Expression::CallExpression(call) if is_path_join(call) => joined(call),
        Expression::CallExpression(call) => call
            .arguments
            .iter()
            .rev()
            .find_map(|argument| argument.as_expression().and_then(literal_of)),
        // `new URL('./preload.js', import.meta.url)`
        Expression::NewExpression(new) => new
            .arguments
            .first()
            .and_then(|argument| argument.as_expression().and_then(literal_of)),
        // `new URL('vs/main.js', base).href`, and the path of a URI a call
        // makes of a path, as VS Code's `FileAccess.asFileUri('x.js').fsPath`.
        Expression::StaticMemberExpression(member)
            if matches!(
                member.property.name.as_str(),
                "href" | "pathname" | "fsPath"
            ) && matches!(
                member.object,
                Expression::NewExpression(_) | Expression::CallExpression(_)
            ) =>
        {
            literal_of(&member.object)
        }
        _ => None,
    }
}

/// The path a template literal names: its text, or the text after the last
/// substitution, as in `` `${__dirname}/../renderer/index.html` ``.
fn template_path(template: &TemplateLiteral<'_>) -> Option<String> {
    if let Some(first) = template_choices(template).into_iter().next() {
        return Some(first);
    }
    let last = template.quasis.last()?.value.raw.to_string();
    Some(last.trim_start_matches('/').to_owned()).filter(|path| !path.is_empty())
}

/// How many strings a template literal's choices may spell.
const MAX_CHOICES: usize = 16;

/// The strings an expression may be when each of its branches is a literal:
/// `'a'`, `` `b` ``, `flag ? '' : '-dev'` and nestings of them.
fn choices(expression: &Expression<'_>) -> Option<Vec<String>> {
    match expression {
        Expression::StringLiteral(literal) => Some(vec![literal.value.to_string()]),
        Expression::TemplateLiteral(template) => {
            Some(template_choices(template)).filter(|choices| !choices.is_empty())
        }
        Expression::ParenthesizedExpression(inner) => choices(&inner.expression),
        Expression::ConditionalExpression(conditional) => {
            let mut all = choices(&conditional.consequent)?;
            for choice in choices(&conditional.alternate)? {
                if !all.contains(&choice) {
                    all.push(choice);
                }
            }
            Some(all)
        }
        _ => None,
    }
}

/// Every string a template literal spells when each substitution is one of
/// a few literals, as in `` `workbench${isBuilt ? '' : '-dev'}.html` ``;
/// empty when a substitution is anything else.
fn template_choices(template: &TemplateLiteral<'_>) -> Vec<String> {
    let mut spelled = vec![String::new()];
    for (index, quasi) in template.quasis.iter().enumerate() {
        let text = quasi.value.cooked.as_ref().unwrap_or(&quasi.value.raw);
        for prefix in &mut spelled {
            prefix.push_str(text);
        }
        let Some(expression) = template.expressions.get(index) else {
            continue;
        };
        let Some(options) = choices(expression) else {
            return Vec::new();
        };
        if spelled.len() * options.len() > MAX_CHOICES {
            return Vec::new();
        }
        spelled = spelled
            .iter()
            .flat_map(|prefix| {
                options
                    .iter()
                    .map(move |option| format!("{prefix}{option}"))
            })
            .collect();
    }
    spelled
}

/// Whether a path names a preload script, such as `preload.cjs` or
/// `preload/index.js`.
fn names_preload(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("preload")
        && [".js", ".cjs", ".mjs", ".ts", ".cts", ".mts"]
            .iter()
            .any(|extension| lower.ends_with(extension))
}

fn push_unique(list: &mut Vec<String>, item: String) {
    if !list.contains(&item) {
        list.push(item);
    }
}

/// The path an application's own URL names: `app://./index.html`, which a
/// handler registered with Electron's `protocol` serves from the
/// application's files, names `index.html`. Web, file and browser schemes
/// name no application path.
fn custom_url_path(literal: &str) -> Option<String> {
    let (scheme, rest) = literal.split_once("://")?;
    let custom = scheme.len() > 1
        && scheme.as_bytes()[0].is_ascii_alphabetic()
        && scheme
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"+-.".contains(&byte))
        && !matches!(
            scheme.to_ascii_lowercase().as_str(),
            "http" | "https" | "file" | "ws" | "wss" | "ftp" | "chrome" | "devtools" | "about"
        );
    if !custom {
        return None;
    }
    let (_, path) = rest.split_once('/')?;
    let path = path.split(['?', '#']).next().unwrap_or_default();
    (!path.is_empty()).then(|| path.to_owned())
}

impl Collector {
    /// Records a path the main process builds.
    fn path(&mut self, path: String) {
        let path = custom_url_path(&path).unwrap_or(path);
        if path.to_ascii_lowercase().ends_with(".html") {
            push_unique(&mut self.pages, path);
        } else if names_preload(&path) {
            push_unique(&mut self.preloads, path);
        }
    }
}

impl<'a> Visit<'a> for Collector {
    fn visit_import_declaration(&mut self, declaration: &ImportDeclaration<'a>) {
        self.specifiers.push(declaration.source.value.to_string());
    }

    fn visit_import_expression(&mut self, expression: &ImportExpression<'a>) {
        match &expression.source {
            Expression::StringLiteral(literal) => self.specifiers.push(literal.value.to_string()),
            Expression::Identifier(variable) => {
                push_unique(&mut self.import_variables, variable.name.to_string());
            }
            _ => {}
        }
        walk::walk_import_expression(self, expression);
    }

    fn visit_assignment_expression(&mut self, assignment: &AssignmentExpression<'a>) {
        if let AssignmentTarget::AssignmentTargetIdentifier(name) = &assignment.left
            && assignment.operator == AssignmentOperator::Assign
            && let Some(path) = literal_of(&assignment.right)
        {
            self.bindings.push((name.name.to_string(), path));
        }
        walk::walk_assignment_expression(self, assignment);
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        let forks = match &call.callee {
            Expression::Identifier(callee) => callee.name == "fork",
            Expression::StaticMemberExpression(member) => member.property.name == "fork",
            _ => false,
        };
        if forks && let Some(module) = call.arguments.first().and_then(Argument::as_expression) {
            match module {
                Expression::Identifier(variable) => {
                    push_unique(&mut self.fork_variables, variable.name.to_string());
                }
                module => {
                    if let Some(path) = literal_of(module) {
                        push_unique(&mut self.forks, path);
                    }
                }
            }
        }
        if let Expression::Identifier(callee) = &call.callee
            && callee.name == "require"
            && let Some(Argument::StringLiteral(literal)) = call.arguments.first()
        {
            self.specifiers.push(literal.value.to_string());
        }
        if is_path_join(call)
            && let Some(path) = joined(call)
        {
            self.path(path);
        }
        walk::walk_call_expression(self, call);
    }

    fn visit_variable_declarator(&mut self, declarator: &VariableDeclarator<'a>) {
        if let BindingPattern::BindingIdentifier(name) = &declarator.id
            && let Some(path) = declarator.init.as_ref().and_then(literal_of)
        {
            self.bindings.push((name.name.to_string(), path));
        }
        walk::walk_variable_declarator(self, declarator);
    }

    fn visit_object_property(&mut self, property: &ObjectProperty<'a>) {
        if let PropertyKey::StaticIdentifier(key) = &property.key
            && key.name == "preload"
        {
            if let Expression::Identifier(variable) = &property.value {
                self.preload_variables.push(variable.name.to_string());
            } else if let Some(literal) = literal_of(&property.value) {
                push_unique(&mut self.preloads, literal);
            }
        }
        walk::walk_object_property(self, property);
    }

    fn visit_string_literal(&mut self, literal: &StringLiteral<'a>) {
        self.path(literal.value.to_string());
    }

    fn visit_template_literal(&mut self, template: &TemplateLiteral<'a>) {
        let spelled = template_choices(template);
        if spelled.is_empty() {
            if let Some(path) = template_path(template) {
                self.path(path);
            }
        } else {
            for path in spelled {
                self.path(path);
            }
        }
        walk::walk_template_literal(self, template);
    }

    fn visit_identifier_reference(&mut self, identifier: &oxc::ast::ast::IdentifierReference<'a>) {
        let name = identifier.name.as_str();
        if name.ends_with("_VITE_DEV_SERVER_URL") || name.ends_with("_VITE_NAME") {
            self.forge_globals.push(name.to_owned());
        }
        self.opens_windows |= name == "BrowserWindow";
    }

    fn visit_static_member_expression(&mut self, member: &StaticMemberExpression<'a>) {
        self.opens_windows |= member.property.name == "BrowserWindow";
        walk::walk_static_member_expression(self, member);
    }
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
}

fn is_script(path: &Path) -> bool {
    matches!(
        extension(path).as_deref(),
        Some("js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "mts" | "cts")
    )
}

/// Scans the entry file and every local script it imports.
pub fn scan_main(entry: &Path, resolver: &Resolver) -> Scan {
    scan(entry, resolver, false)
}

/// Scans what an application ships from an entry: like [`scan_main`], and
/// also Vue components with the modules their scripts import and stylesheets
/// in preprocessor languages.
pub fn scan_shipped(entry: &Path, resolver: &Resolver) -> Scan {
    scan(entry, resolver, true)
}

/// The script blocks of a Vue component, or the whole file of a script.
fn programs(file: &Path, source: &str) -> Vec<(String, SourceType)> {
    if extension(file).as_deref() != Some("vue") {
        return vec![(
            source.to_owned(),
            SourceType::from_path(file).unwrap_or_default(),
        )];
    }
    crate::vue::blocks(source)
        .into_iter()
        .filter(|block| block.kind == crate::vue::BlockKind::Script)
        .map(|block| {
            let source_type = match block.lang.as_deref() {
                Some("ts") => SourceType::ts(),
                Some("tsx") => SourceType::tsx(),
                Some("jsx") => SourceType::jsx(),
                _ => SourceType::mjs(),
            };
            (block.content.to_owned(), source_type)
        })
        .collect()
}

fn scan(entry: &Path, resolver: &Resolver, shipped: bool) -> Scan {
    let mut scan = Scan::default();
    let mut queue = vec![entry.to_path_buf()];
    while let Some(file) = queue.pop() {
        if scan.files.contains(&file) {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&file) else {
            continue;
        };
        scan.files.push(file.clone());
        let mut collector = Collector::default();
        for (program, source_type) in programs(&file, &source) {
            let allocator = Allocator::default();
            let parsed = Parser::new(&allocator, &program, source_type).parse();
            collector.visit_program(&parsed.program);
        }
        for variable in &collector.preload_variables {
            if let Some((_, path)) = collector.bindings.iter().find(|(name, _)| name == variable) {
                push_unique(&mut collector.preloads, path.clone());
            }
        }
        for variable in &collector.fork_variables {
            for (name, path) in &collector.bindings {
                if name == variable {
                    push_unique(&mut collector.forks, path.clone());
                }
            }
        }
        // A variable `import()` loads may be any path it was given; those
        // that resolve from this file are the modules it can load.
        for variable in &collector.import_variables {
            for (name, path) in &collector.bindings {
                if name == variable {
                    collector.specifiers.push(path.clone());
                }
            }
        }
        let directory = file.parent().map(Path::to_path_buf).unwrap_or_default();
        let reference = |literal: String| Reference {
            directory: directory.clone(),
            literal,
        };
        scan.pages
            .extend(collector.pages.into_iter().map(reference));
        scan.preloads
            .extend(collector.preloads.into_iter().map(reference));
        scan.forks
            .extend(collector.forks.into_iter().map(reference));
        scan.opens_windows |= collector.opens_windows;
        for global in collector.forge_globals {
            if !scan.forge_globals.contains(&global) {
                scan.forge_globals.push(global);
            }
        }
        for specifier in collector.specifiers {
            // Workspace packages resolve to their sources, past the links
            // package managers make for them.
            let Ok(Resolved::File(path)) = resolver.resolve(&specifier, &file) else {
                continue;
            };
            if path
                .components()
                .any(|part| part.as_os_str() == "node_modules")
            {
                continue;
            }
            let kind = extension(&path);
            if is_script(&path) || (shipped && kind.as_deref() == Some("vue")) {
                queue.push(path);
            } else if (matches!(kind.as_deref(), Some("css"))
                || (shipped
                    && matches!(
                        kind.as_deref(),
                        Some("scss" | "sass" | "less" | "styl" | "stylus")
                    )))
                && !scan.files.contains(&path)
            {
                scan.files.push(path);
            }
        }
    }
    scan
}

#[cfg(test)]
mod tests;
