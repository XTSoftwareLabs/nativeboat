//! The module graph: discovering modules from an entry, resolving their
//! imports and exports to shared binding cells, and the native entry point
//! that creates namespaces and evaluates modules in dependency order.
//!
//! Each exported binding owns one cell in a project-wide table; importing
//! modules use the exporter's cell directly, so imports are live bindings and
//! cycles observe the temporal dead zone exactly as linked source modules do.

use super::{Analysis, CompileOptions, ModuleKind, ModuleSource, RequestKind, emit};
use crate::{Diagnostic, Source};
use oxc::allocator::Allocator;
use oxc::ast::ast::*;
use oxc::ast_visit::{Visit, walk};
use oxc::parser::Parser;
use oxc::semantic::{SemanticBuilder, SymbolId};
use oxc::span::{GetSpan, SourceType, Span};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Finds the module a specifier names, from the importing module's path.
pub type Resolve<'r> = &'r mut dyn FnMut(&Path, &str) -> Option<PathBuf>;
/// Reads a module's source text.
pub type Load<'r> = &'r mut dyn FnMut(&Path) -> Option<String>;
/// Resolves a request using the original import or require conditions.
pub type ResolveSource<'r> = &'r mut dyn FnMut(&Path, &str, RequestKind) -> Option<PathBuf>;
/// Loads original source and the format of its physical package scope.
pub type LoadSource<'r> = &'r mut dyn FnMut(&Path) -> Option<ModuleSource>;

mod commonjs;
mod json;

/// What an export or import refers to in another module.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Imported {
    Name(String),
    Namespace,
    /// `import x = require('m')`: a CommonJS module's `module.exports`, an
    /// ES module's namespace.
    Required,
}

/// A module-local exported binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Local {
    Symbol(SymbolId),
    /// The value of `export default <expression>` or an anonymous function.
    Default,
}

/// A module's static imports and exports, before linking.
struct Record {
    path: PathBuf,
    source: String,
    /// Whether source functions inherit strict `this` semantics. ES modules
    /// and explicitly strict CommonJS files set this; sloppy CommonJS files do not.
    strict: bool,
    /// CommonJS keeps original text and maps the compiler's wrapper positions.
    original: Option<commonjs::Original>,
    /// Native construction code for a parsed JSON module's data.
    data: Option<String>,
    /// CommonJS and native modules: ES imports read named exports from the
    /// exports object once it is evaluated. JSON only has a default export.
    named_exports: bool,
    /// Source-valid blockers do not stop discovery of other static requests.
    errors: Vec<Diagnostic>,
    /// Uses that compile but fail when the application runs them.
    warnings: Vec<Diagnostic>,
    /// A module that could not be resolved, read or parsed. It was reported
    /// once; every name imported from it links to a stand-in cell so that the
    /// importing modules still report their own blockers.
    placeholder: bool,
    /// Specifier, its location and the module it resolved to.
    requests: Vec<(String, Span, Option<usize>)>,
    /// CommonJS requests inside `try` blocks, by location: one that cannot
    /// be resolved or read throws Node's `MODULE_NOT_FOUND` when it runs,
    /// instead of stopping the build, as bundlers treat them.
    optional: HashSet<Span>,
    /// Statically named dynamic imports, resolved into lazy project entries.
    /// A computed specifier contributes each path it can be, by its site.
    dynamic_imports: Vec<(String, Span, Option<usize>)>,
    /// The sites of computed dynamic imports, whose candidates need not all
    /// resolve.
    computed_imports: HashSet<Span>,
    /// Computed imports whose paths read properties: they take the values
    /// the whole graph gives those properties, once it is known.
    deferred_imports: Vec<(Vec<Choice>, Span)>,
    /// The values this module gives properties, by property name, for the
    /// computed imports that read them.
    property_values: Vec<(String, Vec<Choice>)>,
    /// String literals given side by side to a call, which may name a
    /// package and a script in it, and how many were looked up.
    package_pairs: Vec<(String, String)>,
    package_pairs_done: usize,
    /// The package scripts those name, by key, with their modules.
    package_scripts: Vec<(String, usize)>,
    /// Paths of scripts string literals given to calls and constructors
    /// name, which workers may run, and how many were looked up.
    worker_paths: Vec<String>,
    worker_paths_done: usize,
    /// Whether the module constructs a `Worker` or `SharedWorker`.
    constructs_workers: bool,
    /// CommonJS requests through a module-scoped `createRequire`, with their
    /// factory binding and graph request index.
    create_require_calls: Vec<(Option<SymbolId>, String, Span, Option<usize>)>,
    /// Request slots resolved using Node's CommonJS conditions and evaluated
    /// only when their `createRequire` call executes.
    create_require_request_indexes: HashSet<usize>,
    /// Calls that construct a require function from `node:module`.
    create_require_constructors: HashSet<Span>,
    /// Those whose binding the module uses as a value.
    create_require_values: HashSet<Span>,
    /// The functions around lowered `require` calls.
    require_scopes: HashSet<oxc::semantic::ScopeId>,
    /// Static require.resolve calls never load or evaluate their target.
    resolutions: Vec<Resolution>,
    locals: Vec<(String, Local)>,
    /// Exported `var` bindings start as undefined during instantiation.
    hoisted_vars: HashSet<SymbolId>,
    indirect: Vec<(String, usize, Imported, Span)>,
    stars: Vec<usize>,
    imports: Vec<(SymbolId, usize, Imported, Span)>,
    /// Names a TypeScript module exports only as types, which TypeScript
    /// importers drop.
    type_exports: HashSet<String>,
}

struct Resolution {
    call: Span,
    request: String,
    location: Span,
    path: Option<PathBuf>,
}

/// The cells a module's code uses: its exported locals and its imports.
pub(super) struct Link {
    pub index: usize,
    pub strict: bool,
    pub native_dependencies: Rc<RefCell<BTreeSet<String>>>,
    pub cells: HashMap<SymbolId, usize>,
    pub default: Option<usize>,
    /// Native constants for compiler-resolved CommonJS files or builtin names.
    pub resolutions: HashMap<Span, PathBuf>,
    /// Dynamic import expressions mapped to lazy project-entry ids: one
    /// for a string literal; for a computed specifier, each entry with the
    /// path it is found by at run time.
    pub dynamic_imports: HashMap<Span, DynamicImport>,
    /// ESM `createRequire` calls lowered to synchronous CommonJS factories.
    pub create_require_calls: HashMap<Span, usize>,
    /// Lowered requires inside `try` whose module is missing: they throw
    /// Node's `MODULE_NOT_FOUND` when they run.
    pub missing_requires: HashSet<Span>,
    /// The functions those calls are in, and the functions around them,
    /// whose closures keep the module tables the calls need.
    pub require_scopes: HashSet<oxc::semantic::ScopeId>,
    /// `createRequire(import.meta.url)` calls erased after their require sites
    /// have been linked directly into the project graph.
    pub create_require_constructors: HashSet<Span>,
    /// Those whose binding the module uses as a value, such as esbuild's
    /// `__require` wrapper over it: a function that requires by name when
    /// the application runs.
    pub create_require_values: HashSet<Span>,
}

/// The symbols a declaration binds at module level.
pub(super) fn declared_symbols(declaration: &Declaration<'_>) -> Vec<SymbolId> {
    match declaration {
        Declaration::VariableDeclaration(declaration) => declaration
            .declarations
            .iter()
            .flat_map(|declarator| declarator.id.get_binding_identifiers())
            .map(BindingIdentifier::symbol_id)
            .collect(),
        Declaration::FunctionDeclaration(function) => function
            .id
            .as_ref()
            .map(BindingIdentifier::symbol_id)
            .into_iter()
            .collect(),
        Declaration::ClassDeclaration(class) => class
            .id
            .as_ref()
            .map(BindingIdentifier::symbol_id)
            .into_iter()
            .collect(),
        Declaration::TSEnumDeclaration(declaration) if !declaration.declare => {
            vec![declaration.id.symbol_id()]
        }
        // Namespaces with only types have no runtime value to export.
        Declaration::TSNamespaceDeclaration(declaration)
            if super::emit::instantiated(declaration) =>
        {
            vec![declaration.id.symbol_id()]
        }
        Declaration::TSImportEqualsDeclaration(declaration)
            if !declaration.import_kind.is_type() =>
        {
            vec![declaration.id.symbol_id()]
        }
        _ => Vec::new(),
    }
}

fn name(export: &ModuleExportName<'_>) -> String {
    export.name().to_string()
}

/// Inline TypeScript type specifiers have no runtime binding.
fn value_import(specifier: &ImportDeclarationSpecifier<'_>) -> bool {
    !matches!(specifier, ImportDeclarationSpecifier::ImportSpecifier(specifier) if specifier.import_kind.is_type())
}

fn runtime_reexport(export: &ExportFromDeclaration<'_>) -> bool {
    !export.export_kind.is_type()
        && (export.specifiers.is_empty()
            || export
                .specifiers
                .iter()
                .any(|specifier| !specifier.export_kind.is_type()))
}

/// The request `using` declarations import their disposal helper by, and
/// the Node library module the runtime provides it as.
pub(super) const USING_HELPER: &str = "nativeboat/helpers/usingCtx";
const USING_HELPER_MODULE: &str = "node:internal/using_ctx";

/// Whether a module declares `using` or `await using` bindings.
fn declares_using(program: &Program<'_>) -> bool {
    #[derive(Default)]
    struct Using(bool);
    impl<'a> Visit<'a> for Using {
        fn visit_variable_declaration(&mut self, declaration: &VariableDeclaration<'a>) {
            if matches!(
                declaration.kind,
                VariableDeclarationKind::Using | VariableDeclarationKind::AwaitUsing
            ) {
                self.0 = true;
            }
            walk::walk_variable_declaration(self, declaration);
        }
    }
    let mut using = Using::default();
    using.visit_program(program);
    using.0
}

/// Lowers `using` declarations to blocks that dispose their resources as
/// the explicit resource management proposal does, with the helper imported
/// from `USING_HELPER`. The transform sees a JavaScript program, so it
/// changes nothing else.
fn lower_using<'a>(
    allocator: &'a Allocator,
    program: &mut Program<'a>,
    scoping: oxc::semantic::Scoping,
    source: &Source<'_>,
) -> Result<(), Vec<Diagnostic>> {
    use oxc::transformer::{HelperLoaderMode, TransformOptions, Transformer};
    let mut options = TransformOptions::default();
    options.env.es2026.explicit_resource_management = true;
    options.helper_loader.mode = HelperLoaderMode::Runtime;
    options.helper_loader.module_name = "nativeboat".into();
    let source_type = program.source_type;
    program.source_type = SourceType::mjs();
    let transformed =
        Transformer::new(allocator, source.path, &options).build_with_scoping(scoping, program);
    program.source_type = source_type;
    if transformed.diagnostics.is_empty() {
        Ok(())
    } else {
        Err(source.diagnostics(transformed.diagnostics))
    }
}

/// Parses a module and reports syntax and early errors. JSX in `.jsx` and
/// `.tsx` modules becomes calls, as `options` says, and `using`
/// declarations become the blocks that dispose their resources.
fn parse<'a>(
    allocator: &'a Allocator,
    source: &Source<'_>,
    options: &CompileOptions,
) -> Result<(Program<'a>, oxc::semantic::Scoping), Vec<Diagnostic>> {
    let source_type = SourceType::from_path(source.path)
        .map_err(|error| vec![source.error(Span::default(), &error.to_string())])?
        .with_module(true);
    let text = allocator.alloc_str(source.text);
    let parsed = Parser::new(allocator, text, source_type).parse();
    if parsed.diagnostics.has_errors() {
        return Err(source.diagnostics(parsed.diagnostics));
    }
    let mut program = parsed.program;
    let semantic = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .build(&program);
    if semantic.diagnostics.has_errors() {
        return Err(source.diagnostics(semantic.diagnostics));
    }
    let mut scoping = semantic.semantic.into_scoping();
    let extension = source
        .path
        .extension()
        .and_then(|extension| extension.to_str());
    if matches!(extension, Some("jsx" | "tsx")) {
        compile_jsx(allocator, &mut program, scoping, source, &options.jsx)?;
        scoping = SemanticBuilder::new()
            .build(&program)
            .semantic
            .into_scoping();
    }
    if declares_using(&program) {
        lower_using(allocator, &mut program, scoping, source)?;
        scoping = SemanticBuilder::new()
            .build(&program)
            .semantic
            .into_scoping();
    }
    Ok((program, scoping))
}

/// Replaces JSX with the calls it stands for, leaving TypeScript to the
/// emitter: the transform sees a JavaScript program, so it changes nothing
/// else, and the nodes it keeps keep their positions.
fn compile_jsx<'a>(
    allocator: &'a Allocator,
    program: &mut Program<'a>,
    scoping: oxc::semantic::Scoping,
    source: &Source<'_>,
    jsx: &super::Jsx,
) -> Result<(), Vec<Diagnostic>> {
    use oxc::transformer::{JsxRuntime, TransformOptions, Transformer};
    let mut options = TransformOptions::default();
    match jsx {
        super::Jsx::Automatic { import_source } => {
            options.jsx.runtime = JsxRuntime::Automatic;
            options.jsx.import_source = Some(import_source.clone());
        }
        super::Jsx::Classic { factory, fragment } => {
            options.jsx.runtime = JsxRuntime::Classic;
            options.jsx.pragma = Some(factory.clone());
            options.jsx.pragma_frag = Some(fragment.clone());
        }
    }
    let source_type = program.source_type;
    program.source_type = SourceType::jsx().with_module(true);
    let transformed =
        Transformer::new(allocator, source.path, &options).build_with_scoping(scoping, program);
    program.source_type = source_type;
    if transformed.diagnostics.is_empty() {
        Ok(())
    } else {
        Err(source.diagnostics(transformed.diagnostics))
    }
}

/// Whether a value uses an import binding, as TypeScript keeps it: in code,
/// JSX or an `export` list, not only in types.
fn used_as_value(scoping: &oxc::semantic::Scoping, symbol: SymbolId) -> bool {
    scoping
        .get_resolved_references(symbol)
        .any(|reference| reference.flags().is_value())
}

/// Collects a module's import and export records.
fn scan(path: &Path, text: String, options: &CompileOptions) -> Result<Record, Vec<Diagnostic>> {
    let allocator = Allocator::default();
    let source = Source { path, text: &text };
    let (program, scoping) = parse(&allocator, &source, options)?;
    // TypeScript drops imports that no value uses: types, and modules that
    // only declare types.
    let elide = program.source_type.is_typescript() && !options.verbatim_module_syntax;
    let kept = |specifier: &ImportDeclarationSpecifier<'_>| {
        value_import(specifier)
            && (!elide || used_as_value(&scoping, specifier.local().symbol_id()))
    };
    // Erases type-only requests, keeping ordinary empty and side-effect
    // imports and mixed declarations. No source rewriting changes diagnostics.
    let runtime_import = |import: &ImportDeclaration<'_>| {
        !import.import_kind.is_type()
            && import
                .specifiers
                .as_ref()
                .is_none_or(|specifiers| specifiers.is_empty() || specifiers.iter().any(kept))
    };
    let mut record = Record {
        path: path.to_path_buf(),
        source: String::new(),
        strict: true,
        original: None,
        data: None,
        named_exports: false,
        errors: Vec::new(),
        warnings: Vec::new(),
        placeholder: false,
        requests: Vec::new(),
        optional: HashSet::new(),
        dynamic_imports: Vec::new(),
        computed_imports: HashSet::new(),
        deferred_imports: Vec::new(),
        property_values: Vec::new(),
        package_pairs: Vec::new(),
        package_pairs_done: 0,
        package_scripts: Vec::new(),
        worker_paths: Vec::new(),
        worker_paths_done: 0,
        constructs_workers: false,
        create_require_calls: Vec::new(),
        create_require_request_indexes: HashSet::new(),
        create_require_constructors: HashSet::new(),
        create_require_values: HashSet::new(),
        require_scopes: HashSet::new(),
        resolutions: Vec::new(),
        locals: Vec::new(),
        hoisted_vars: HashSet::new(),
        indirect: Vec::new(),
        stars: Vec::new(),
        imports: Vec::new(),
        type_exports: HashSet::new(),
    };
    let request = |record: &mut Record, specifier: &StringLiteral<'_>| {
        record
            .requests
            .push((specifier.value.to_string(), specifier.span, None));
        record.requests.len() - 1
    };
    let mut create_require_factories = HashSet::new();
    // Module-level `const require = createRequire(import.meta.url)`
    // declarations, by the factory name they call: those uses are lowered.
    let mut lowered_factory_calls: HashMap<String, usize> = HashMap::new();
    for statement in &program.body {
        if let Statement::VariableDeclaration(declaration) = statement {
            for declarator in &declaration.declarations {
                if let Some(Expression::CallExpression(call)) = &declarator.init
                    && let Expression::Identifier(factory) = call.callee.without_parentheses()
                    && !call.optional
                    && call.arguments.len() == 1
                    && call
                        .arguments
                        .first()
                        .and_then(Argument::as_expression)
                        .is_some_and(|argument| is_import_meta_url(argument.without_parentheses()))
                {
                    *lowered_factory_calls
                        .entry(factory.name.to_string())
                        .or_default() += 1;
                }
            }
        }
    }
    for statement in &program.body {
        let (attributes, specifier) = match statement {
            Statement::ImportDeclaration(import) if runtime_import(import) => (
                import.with_clause.as_deref(),
                Some(import.source.value.as_str()),
            ),
            Statement::ExportFromDeclaration(export) if runtime_reexport(export) => (
                export.with_clause.as_deref(),
                Some(export.source.value.as_str()),
            ),
            Statement::ExportAllDeclaration(export) if !export.export_kind.is_type() => (
                export.with_clause.as_deref(),
                Some(export.source.value.as_str()),
            ),
            _ => (None, None),
        };
        if let Some(attributes) = attributes
            && !supports_json_attribute(attributes, specifier.unwrap_or_default())
        {
            record.errors.push(source.error(
                attributes.span,
                "module import attributes are not compiled yet",
            ));
        }
        match statement {
            Statement::ImportDeclaration(import) if runtime_import(import) => {
                let is_module_builtin =
                    matches!(import.source.value.as_str(), "module" | "node:module");
                let mut runtime_specifiers = Vec::new();
                for specifier in import
                    .specifiers
                    .iter()
                    .flatten()
                    .filter(|specifier| kept(specifier))
                {
                    // `createRequire(import.meta.url)` at module level is
                    // lowered to the module's own requires; other uses run
                    // the library's `createRequire`, imported for them.
                    if is_module_builtin
                        && let ImportDeclarationSpecifier::ImportSpecifier(named) = specifier
                        && name(&named.imported) == "createRequire"
                    {
                        let symbol = named.local.symbol_id();
                        create_require_factories.insert(symbol);
                        let lowered = lowered_factory_calls
                            .get(named.local.name.as_str())
                            .copied()
                            .unwrap_or(0);
                        if scoping.get_resolved_references(symbol).count() > lowered {
                            runtime_specifiers.push(specifier);
                        }
                    } else {
                        runtime_specifiers.push(specifier);
                    }
                }
                let has_runtime_request = !runtime_specifiers.is_empty()
                    || import
                        .specifiers
                        .as_ref()
                        .is_none_or(|specifiers| specifiers.is_empty());
                let index = has_runtime_request.then(|| request(&mut record, &import.source));
                for specifier in runtime_specifiers {
                    let (local, imported) = match specifier {
                        ImportDeclarationSpecifier::ImportSpecifier(specifier) => {
                            (&specifier.local, Imported::Name(name(&specifier.imported)))
                        }
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier) => {
                            (&specifier.local, Imported::Name("default".to_owned()))
                        }
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(specifier) => {
                            (&specifier.local, Imported::Namespace)
                        }
                    };
                    record.imports.push((
                        local.symbol_id(),
                        index.expect("runtime import has a graph request"),
                        imported,
                        specifier.span(),
                    ));
                }
            }
            Statement::ExportNamedDeclaration(export) if !export.export_kind.is_type() => {
                for specifier in export
                    .specifiers
                    .iter()
                    .filter(|specifier| !specifier.export_kind.is_type())
                {
                    if let ModuleExportName::IdentifierReference(local) = &specifier.local
                        && let Some(symbol) =
                            scoping.get_reference(local.reference_id()).symbol_id()
                    {
                        record
                            .locals
                            .push((name(&specifier.exported), Local::Symbol(symbol)));
                    }
                }
            }
            Statement::ExportDeclaration(export) => {
                for symbol in declared_symbols(&export.declaration) {
                    let exported = scoping.symbol_name(symbol).to_owned();
                    record.locals.push((exported, Local::Symbol(symbol)));
                }
            }
            Statement::ExportDefaultDeclaration(export) => {
                let local = match &export.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(function) => function
                        .id
                        .as_ref()
                        .map_or(Local::Default, |id| Local::Symbol(id.symbol_id())),
                    ExportDefaultDeclarationKind::ClassDeclaration(class) => class
                        .id
                        .as_ref()
                        .map_or(Local::Default, |id| Local::Symbol(id.symbol_id())),
                    _ => Local::Default,
                };
                record.locals.push(("default".to_owned(), local));
            }
            Statement::ExportFromDeclaration(export) if runtime_reexport(export) => {
                let index = request(&mut record, &export.source);
                for specifier in export
                    .specifiers
                    .iter()
                    .filter(|specifier| !specifier.export_kind.is_type())
                {
                    record.indirect.push((
                        name(&specifier.exported),
                        index,
                        Imported::Name(name(&specifier.local)),
                        specifier.span,
                    ));
                }
            }
            Statement::ExportAllDeclaration(export) if !export.export_kind.is_type() => {
                let index = request(&mut record, &export.source);
                match &export.exported {
                    Some(exported) => record.indirect.push((
                        name(exported),
                        index,
                        Imported::Namespace,
                        export.span,
                    )),
                    None => record.stars.push(index),
                }
            }
            // `import x = require('m')` imports the module as `tsc` emits it
            // for CommonJS output.
            Statement::TSImportEqualsDeclaration(declaration) => {
                if !declaration.import_kind.is_type()
                    && let TSModuleReference::ExternalModuleReference(reference) =
                        &declaration.module_reference
                    && (!elide || used_as_value(&scoping, declaration.id.symbol_id()))
                {
                    let index = request(&mut record, &reference.expression);
                    record.imports.push((
                        declaration.id.symbol_id(),
                        index,
                        Imported::Required,
                        declaration.span,
                    ));
                }
            }
            _ => {}
        }
        match statement {
            Statement::ExportDeclaration(export) => match &export.declaration {
                Declaration::TSTypeAliasDeclaration(declaration) => {
                    record.type_exports.insert(declaration.id.name.to_string());
                }
                Declaration::TSInterfaceDeclaration(declaration) => {
                    record.type_exports.insert(declaration.id.name.to_string());
                }
                _ => {}
            },
            Statement::ExportNamedDeclaration(export) => {
                for specifier in &export.specifiers {
                    if export.export_kind.is_type() || specifier.export_kind.is_type() {
                        record.type_exports.insert(name(&specifier.exported));
                    }
                }
            }
            _ => {}
        }
    }
    let mut created_require_symbols = HashSet::new();
    let mut constructors_by_symbol = HashMap::new();
    for statement in &program.body {
        let Statement::VariableDeclaration(declaration) = statement else {
            continue;
        };
        for declarator in &declaration.declarations {
            let BindingPattern::BindingIdentifier(binding) = &declarator.id else {
                continue;
            };
            let Some(Expression::CallExpression(call)) = declarator.init.as_ref() else {
                continue;
            };
            let Expression::Identifier(factory) = call.callee.without_parentheses() else {
                continue;
            };
            if !scoping
                .get_reference(factory.reference_id())
                .symbol_id()
                .is_some_and(|symbol| create_require_factories.contains(&symbol))
            {
                continue;
            }
            if call.optional
                || call.arguments.len() != 1
                || !call
                    .arguments
                    .first()
                    .and_then(Argument::as_expression)
                    .is_some_and(|argument| is_import_meta_url(argument.without_parentheses()))
            {
                continue;
            }
            let symbol = binding.symbol_id();
            created_require_symbols.insert(symbol);
            constructors_by_symbol.insert(symbol, call.span);
            record.create_require_constructors.insert(call.span);
        }
    }
    // Sources a bundler builds into CommonJS have the bundle's `require`, as
    // TypeScript a package's own build compiles to CommonJS has its own.
    let implicit = (options.bundle.is_some()
        && !path
            .components()
            .any(|component| component.as_os_str() == "node_modules"))
        || compiles_to_commonjs(options, path);
    let mut created_calls = CreateRequireCalls {
        source: &source,
        scoping: &scoping,
        factories: &create_require_factories,
        requires: &created_require_symbols,
        implicit,
        functions: Vec::new(),
        scopes: HashSet::new(),
        allowed_references: HashSet::new(),
        calls: Vec::new(),
        computed: Vec::new(),
        tries: 0,
        optional: Vec::new(),
        resolutions: Vec::new(),
        constants: static_const_initializers(&program),
        errors: Vec::new(),
        escaping: HashSet::new(),
        warnings: Vec::new(),
    };
    created_calls.visit_program(&program);
    record.errors.extend(created_calls.errors);
    record.warnings.extend(created_calls.warnings);
    for symbol in &created_calls.escaping {
        if let Some(span) = constructors_by_symbol.get(symbol) {
            record.create_require_values.insert(*span);
        }
    }
    record.resolutions.extend(created_calls.resolutions);
    record.require_scopes = created_calls.scopes;
    record.optional.extend(created_calls.optional);
    for (symbol, specifier, span) in created_calls.calls {
        let request_index = record.requests.len();
        record.requests.push((specifier, span, None));
        record.create_require_request_indexes.insert(request_index);
        record.create_require_calls.push((
            symbol,
            record.requests[request_index].0.clone(),
            span,
            Some(request_index),
        ));
    }
    for (symbol, span) in created_calls.computed {
        record.warnings.push(source.error(
            span,
            "this require is computed when the application runs and throws MODULE_NOT_FOUND then; only modules required by name are compiled",
        ));
        record
            .create_require_calls
            .push((symbol, String::new(), span, None));
    }
    let mut dynamic_imports = DynamicImports::default();
    dynamic_imports.visit_program(&program);
    record.package_pairs = core::mem::take(&mut dynamic_imports.package_pairs);
    record.worker_paths = core::mem::take(&mut dynamic_imports.worker_paths);
    record.constructs_workers = dynamic_imports.constructs_workers;
    let (candidates, deferred, properties) = dynamic_imports.candidates();
    record.property_values = properties;
    for (choices, span) in deferred {
        record.computed_imports.insert(span);
        record.deferred_imports.push((choices, span));
    }
    for (specifier, span, has_options) in candidates {
        match (specifier, has_options) {
            (Specifier::Literal(specifier), false) => {
                record.dynamic_imports.push((specifier, span, None));
            }
            (Specifier::Computed(paths), false) if !paths.is_empty() => {
                record.computed_imports.insert(span);
                for path in paths {
                    record.dynamic_imports.push((path, span, None));
                }
            }
            // A specifier only known at run time names no compiled module:
            // the import rejects when it runs, as a missing module does.
            (Specifier::Computed(_), false) => {
                record.computed_imports.insert(span);
                // Sites that read properties are judged once the graph's
                // values are known.
                if !record.deferred_imports.iter().any(|(_, site)| *site == span) {
                    record.warnings.push(source.error(
                        span,
                        "this dynamic import names no module the build can find; it rejects with ERR_MODULE_NOT_FOUND when it runs",
                    ));
                }
            }
            (Specifier::Computed(_), true) => record.errors.push(source.error(
                span,
                "dynamic import specifier must be a string literal, or a value built from string literals",
            )),
            (Specifier::Literal(_), true) => record
                .errors
                .push(source.error(span, "dynamic import attributes are not compiled yet")),
        }
    }
    for (_, local) in &record.locals {
        if let Local::Symbol(symbol) = local {
            let flags = scoping.symbol_flags(*symbol);
            if flags.is_function_scoped_declaration() && !flags.is_function() {
                record.hoisted_vars.insert(*symbol);
            }
        }
    }
    record.source = text;
    Ok(record)
}

fn supports_json_attribute(attributes: &WithClause<'_>, specifier: &str) -> bool {
    let is_json = specifier
        .split(['?', '#'])
        .next()
        .is_some_and(|path| path.to_ascii_lowercase().ends_with(".json"));
    is_json
        && attributes.with_entries.len() == 1
        && attributes.with_entries.first().is_some_and(|attribute| {
            let is_type = match &attribute.key {
                ImportAttributeKey::Identifier(key) => key.name == "type",
                ImportAttributeKey::StringLiteral(key) => key.value == "type",
            };
            is_type && attribute.value.value == "json"
        })
}

fn is_import_meta_url(expression: &Expression<'_>) -> bool {
    matches!(
        expression,
        Expression::StaticMemberExpression(member)
            if !member.optional
                && member.property.name == "url"
                && matches!(member.object.without_parentheses(), Expression::ImportMeta(_))
    )
}

struct CreateRequireCalls<'s, 'p> {
    source: &'s Source<'p>,
    scoping: &'s oxc::semantic::Scoping,
    factories: &'s HashSet<SymbolId>,
    requires: &'s HashSet<SymbolId>,
    /// Whether an unbound `require` is the module's own, as in a bundle.
    implicit: bool,
    /// The functions the visitor is in, innermost last.
    functions: Vec<oxc::semantic::ScopeId>,
    /// The functions around lowered calls.
    scopes: HashSet<oxc::semantic::ScopeId>,
    allowed_references: HashSet<Span>,
    calls: Vec<(Option<SymbolId>, String, Span)>,
    /// Calls whose specifier is computed when the application runs.
    computed: Vec<(Option<SymbolId>, Span)>,
    /// How many `try` blocks the visitor is in.
    tries: usize,
    /// Lowered requires inside `try` blocks, which may name missing modules.
    optional: Vec<Span>,
    resolutions: Vec<Resolution>,
    constants: HashMap<SymbolId, &'p Expression<'p>>,
    errors: Vec<Diagnostic>,
    /// Bindings the module uses as values, beside the calls compiled.
    escaping: HashSet<SymbolId>,
    warnings: Vec<Diagnostic>,
}

impl<'a> Visit<'a> for CreateRequireCalls<'_, '_> {
    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        match call.callee.without_parentheses() {
            Expression::Identifier(identifier) => {
                let symbol = self
                    .scoping
                    .get_reference(identifier.reference_id())
                    .symbol_id();
                if symbol.is_some_and(|symbol| self.factories.contains(&symbol)) {
                    // Other calls run the library's createRequire.
                    self.allowed_references.insert(identifier.span);
                } else if symbol.is_some_and(|symbol| self.requires.contains(&symbol))
                    || (symbol.is_none() && self.implicit && identifier.name == "require")
                {
                    self.allowed_references.insert(identifier.span);
                    if call.optional {
                        self.errors.push(
                            self.source
                                .error(call.span, "optional require calls are not compiled yet"),
                        );
                    } else if let [Argument::StringLiteral(literal)] = call.arguments.as_slice()
                        && !literal.lone_surrogates
                    {
                        self.scopes.extend(self.functions.iter().copied());
                        if self.tries > 0 {
                            self.optional.push(call.span);
                        }
                        self.calls
                            .push((symbol, literal.value.to_string(), call.span));
                    } else if let [argument] = call.arguments.as_slice()
                        && !matches!(
                            argument,
                            Argument::SpreadElement(_) | Argument::StringLiteral(_)
                        )
                    {
                        // A request computed when the application runs names
                        // no module the build compiled: it throws
                        // MODULE_NOT_FOUND then.
                        self.scopes.extend(self.functions.iter().copied());
                        self.computed.push((symbol, call.span));
                    } else {
                        self.errors.push(self.source.error(
                            call.span,
                            "createRequire needs one static string module specifier",
                        ));
                    }
                }
            }
            Expression::StaticMemberExpression(member)
                if member.property.name == "resolve"
                    && let Expression::Identifier(identifier) =
                        member.object.without_parentheses()
                    && match self
                        .scoping
                        .get_reference(identifier.reference_id())
                        .symbol_id()
                    {
                        Some(symbol) => self.requires.contains(&symbol),
                        None => self.implicit && identifier.name == "require",
                    } =>
            {
                self.allowed_references.insert(identifier.span);
                if call.optional || member.optional {
                    self.errors.push(self.source.error(
                        call.span,
                        "optional require.resolve calls are not compiled yet",
                    ));
                } else if let [Argument::StringLiteral(literal)] = call.arguments.as_slice()
                    && !literal.lone_surrogates
                {
                    self.resolutions.push(Resolution {
                        call: call.span,
                        request: literal.value.to_string(),
                        location: literal.span,
                        path: None,
                    });
                } else if let [argument] = call.arguments.as_slice()
                    && let Some(argument) = argument.as_expression()
                    && let Some(request) = resolve_static_string(
                        argument.without_parentheses(),
                        self.scoping,
                        &static_const_initializers_before(&self.constants, call.span.start),
                        &mut HashSet::new(),
                    )
                {
                    self.resolutions.push(Resolution {
                        call: call.span,
                        request,
                        location: argument.span(),
                        path: None,
                    });
                } else {
                    self.errors.push(self.source.error(
                        call.span,
                        "createRequire.resolve needs one statically known string specifier",
                    ));
                }
            }
            _ => {}
        }
        walk::walk_call_expression(self, call);
    }

    fn visit_identifier_reference(&mut self, reference: &IdentifierReference<'a>) {
        let symbol = self
            .scoping
            .get_reference(reference.reference_id())
            .symbol_id();
        if let Some(symbol) = symbol
            && self.requires.contains(&symbol)
            && !self.allowed_references.contains(&reference.span)
            && self.escaping.insert(symbol)
        {
            // Passed on or kept, as esbuild's `__require` wrapper keeps it,
            // the binding is a function that requires by name.
            self.warnings.push(self.source.error(
                reference.span,
                "createRequire's require is used as a value here; it requires modules by name when the application runs, and those the build did not compile throw MODULE_NOT_FOUND",
            ));
        }
        walk::walk_identifier_reference(self, reference);
    }

    fn visit_try_statement(&mut self, statement: &TryStatement<'a>) {
        self.tries += 1;
        self.visit_block_statement(&statement.block);
        self.tries -= 1;
        if let Some(handler) = &statement.handler {
            self.visit_catch_clause(handler);
        }
        if let Some(finalizer) = &statement.finalizer {
            self.visit_block_statement(finalizer);
        }
    }

    fn visit_function(&mut self, function: &Function<'a>, flags: oxc::semantic::ScopeFlags) {
        self.functions.push(function.scope_id());
        walk::walk_function(self, function, flags);
        self.functions.pop();
    }

    fn visit_arrow_function_expression(&mut self, function: &ArrowFunctionExpression<'a>) {
        self.functions.push(function.scope_id());
        walk::walk_arrow_function_expression(self, function);
        self.functions.pop();
    }
}

fn static_const_initializers<'a>(
    program: &'a Program<'a>,
) -> HashMap<SymbolId, &'a Expression<'a>> {
    let mut constants = HashMap::new();
    let mut add_declaration = |declaration: &'a VariableDeclaration<'a>| {
        if declaration.kind != VariableDeclarationKind::Const {
            return;
        }
        for declarator in &declaration.declarations {
            if let (BindingPattern::BindingIdentifier(identifier), Some(initializer)) =
                (&declarator.id, &declarator.init)
            {
                constants.insert(identifier.symbol_id(), initializer);
            }
        }
    };
    for statement in &program.body {
        match statement {
            Statement::VariableDeclaration(declaration) => add_declaration(declaration),
            Statement::ExportDeclaration(export) => {
                if let Declaration::VariableDeclaration(declaration) = &export.declaration {
                    add_declaration(declaration);
                }
            }
            _ => {}
        }
    }
    constants
}

fn static_const_initializers_before<'a>(
    constants: &HashMap<SymbolId, &'a Expression<'a>>,
    position: u32,
) -> HashMap<SymbolId, &'a Expression<'a>> {
    constants
        .iter()
        .filter(|(_, initializer)| initializer.span().end <= position)
        .map(|(symbol, initializer)| (*symbol, *initializer))
        .collect()
}

#[derive(Clone)]
enum StaticValue {
    String(String),
    Boolean(bool),
    Undefined,
}

impl StaticValue {
    fn truthy(&self) -> bool {
        match self {
            Self::String(value) => !value.is_empty(),
            Self::Boolean(value) => *value,
            Self::Undefined => false,
        }
    }

    fn string(self) -> Option<String> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }
}

fn resolve_static_string(
    expression: &Expression<'_>,
    scoping: &oxc::semantic::Scoping,
    constants: &HashMap<SymbolId, &Expression<'_>>,
    resolving: &mut HashSet<SymbolId>,
) -> Option<String> {
    static_value(expression, scoping, constants, resolving)?.string()
}

fn static_value(
    expression: &Expression<'_>,
    scoping: &oxc::semantic::Scoping,
    constants: &HashMap<SymbolId, &Expression<'_>>,
    resolving: &mut HashSet<SymbolId>,
) -> Option<StaticValue> {
    match expression.without_parentheses() {
        Expression::StringLiteral(literal) if !literal.lone_surrogates => {
            Some(StaticValue::String(literal.value.to_string()))
        }
        Expression::BooleanLiteral(literal) => Some(StaticValue::Boolean(literal.value)),
        Expression::Identifier(identifier) => {
            if let Some(symbol) = scoping.get_reference(identifier.reference_id()).symbol_id() {
                if !resolving.insert(symbol) {
                    return None;
                }
                let value = constants.get(&symbol).and_then(|initializer| {
                    static_value(initializer, scoping, constants, resolving)
                });
                resolving.remove(&symbol);
                return value;
            }
            None
        }
        Expression::StaticMemberExpression(member) => {
            let process = matches!(
                member.object.without_parentheses(),
                Expression::Identifier(identifier)
                    if identifier.name == "process"
                        && scoping
                            .get_reference(identifier.reference_id())
                            .symbol_id()
                            .is_none()
            );
            if process {
                return match member.property.name.as_str() {
                    "platform" => Some(StaticValue::String(
                        match std::env::consts::OS {
                            "windows" => "win32",
                            "macos" => "darwin",
                            other => other,
                        }
                        .to_owned(),
                    )),
                    "arch" => Some(StaticValue::String(
                        match std::env::consts::ARCH {
                            "x86_64" => "x64",
                            "aarch64" => "arm64",
                            "x86" => "ia32",
                            other => other,
                        }
                        .to_owned(),
                    )),
                    "env" => Some(StaticValue::Undefined),
                    _ => None,
                };
            }
            if member.property.name == "npm_config_arch"
                && let Expression::StaticMemberExpression(env) = member.object.without_parentheses()
                && env.property.name == "env"
                && let Expression::Identifier(identifier) = env.object.without_parentheses()
                && identifier.name == "process"
                && scoping
                    .get_reference(identifier.reference_id())
                    .symbol_id()
                    .is_none()
            {
                // npm_config_arch is an installation-time override, not a
                // property of the generated executable's target environment.
                // Treat it as absent so the adjacent process.arch expression
                // resolves to the native target architecture.
                return Some(StaticValue::Undefined);
            }
            None
        }
        Expression::TemplateLiteral(template) => {
            if template.quasis.iter().any(|quasi| quasi.lone_surrogates) {
                return None;
            }
            let mut value = String::new();
            for (index, quasi) in template.quasis.iter().enumerate() {
                value.push_str(quasi.value.cooked.as_ref()?.as_str());
                if let Some(expression) = template.expressions.get(index) {
                    value.push_str(
                        &static_value(expression, scoping, constants, resolving)?.string()?,
                    );
                }
            }
            Some(StaticValue::String(value))
        }
        Expression::BinaryExpression(binary)
            if matches!(
                binary.operator,
                oxc::syntax::operator::BinaryOperator::StrictEquality
            ) =>
        {
            let left = static_value(&binary.left, scoping, constants, resolving)?;
            let right = static_value(&binary.right, scoping, constants, resolving)?;
            let equal = match (left, right) {
                (StaticValue::String(left), StaticValue::String(right)) => left == right,
                (StaticValue::Boolean(left), StaticValue::Boolean(right)) => left == right,
                _ => false,
            };
            Some(StaticValue::Boolean(equal))
        }
        Expression::LogicalExpression(logical)
            if logical.operator == oxc::syntax::operator::LogicalOperator::Or =>
        {
            let left = static_value(&logical.left, scoping, constants, resolving)?;
            if left.truthy() {
                Some(left)
            } else {
                static_value(&logical.right, scoping, constants, resolving)
            }
        }
        Expression::ConditionalExpression(conditional) => {
            let test = static_value(&conditional.test, scoping, constants, resolving)?;
            static_value(
                if test.truthy() {
                    &conditional.consequent
                } else {
                    &conditional.alternate
                },
                scoping,
                constants,
                resolving,
            )
        }
        _ => None,
    }
}

fn placeholder(path: &Path) -> Record {
    Record {
        path: path.to_path_buf(),
        source: String::new(),
        strict: true,
        original: None,
        data: None,
        named_exports: false,
        errors: Vec::new(),
        warnings: Vec::new(),
        placeholder: true,
        requests: Vec::new(),
        optional: HashSet::new(),
        dynamic_imports: Vec::new(),
        computed_imports: HashSet::new(),
        deferred_imports: Vec::new(),
        property_values: Vec::new(),
        package_pairs: Vec::new(),
        package_pairs_done: 0,
        package_scripts: Vec::new(),
        worker_paths: Vec::new(),
        worker_paths_done: 0,
        constructs_workers: false,
        create_require_calls: Vec::new(),
        create_require_request_indexes: HashSet::new(),
        create_require_constructors: HashSet::new(),
        create_require_values: HashSet::new(),
        require_scopes: HashSet::new(),
        resolutions: Vec::new(),
        locals: Vec::new(),
        hoisted_vars: HashSet::new(),
        indirect: Vec::new(),
        stars: Vec::new(),
        imports: Vec::new(),
        type_exports: HashSet::new(),
    }
}

/// A native module: like JSON data, its default export is computed once
/// when first required, here by the runtime's module of that name.
fn native(path: &Path, name: &str) -> Record {
    Record {
        data: Some(if name == "electron" {
            "Ok(::nativeboat_native::Value::Object(::nativeboat_native::electron_builtin::module(realm)))?".to_owned()
        } else {
            format!(
                "::nativeboat_native::modules::load(realm, {})?",
                emit::rust_str(name)
            )
        }),
        locals: vec![("default".into(), Local::Default)],
        named_exports: true,
        placeholder: false,
        ..placeholder(path)
    }
}

/// How an export name resolves.
enum Found {
    Cell(usize),
    Missing,
    Ambiguous,
}

/// What a dynamic import site loads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DynamicImport {
    /// The entry its string literal names.
    Entry(usize),
    /// For a specifier computed at run time, the entries it can name, each
    /// with the path that ends a specifier naming it.
    Computed(Vec<(String, usize)>),
}

/// The path that ends a computed specifier naming a candidate: the
/// candidate without its leading `./` and `../` segments.
fn runtime_key(specifier: &str) -> String {
    let mut rest = specifier.trim();
    loop {
        if let Some(after) = rest.strip_prefix("./") {
            rest = after;
        } else if let Some(after) = rest.strip_prefix("../") {
            rest = after;
        } else {
            break;
        }
    }
    rest.split(['?', '#']).next().unwrap_or_default().to_owned()
}

/// A dynamic import's specifier: a string literal, or the paths a computed
/// one can be.
enum Specifier {
    Literal(String),
    Computed(Vec<String>),
}

/// How many paths a computed specifier may stand for.
const MAX_CANDIDATES: usize = 16;

/// The dynamic imports of a module, and the string literals each variable
/// is given, which computed specifiers name.
#[derive(Default)]
struct DynamicImports {
    sites: Vec<(Specifier, Span, bool)>,
    /// Pending computed sources: the variables or expressions they name.
    computed: Vec<(Vec<Choice>, Span, bool)>,
    bindings: Vec<(String, Vec<Choice>)>,
    /// The values the module gives properties.
    properties: Vec<(String, Vec<Choice>)>,
    /// String literals given side by side to calls.
    package_pairs: Vec<(String, String)>,
    /// String literals given to calls and constructors that are paths of
    /// scripts.
    worker_paths: Vec<String>,
    /// Whether the module constructs a `Worker` or `SharedWorker`.
    constructs_workers: bool,
}

/// A module's dynamic import sites; its computed sites that read
/// properties, with their localized choices; and the values it gives
/// properties.
type ImportSites = (
    Vec<(Specifier, Span, bool)>,
    Vec<(Vec<Choice>, Span)>,
    Vec<(String, Vec<Choice>)>,
);

/// How many paths a computed specifier that reads properties may stand
/// for: the values a property is given across a program include ones that
/// name no module, which are left out.
const MAX_PROPERTY_CANDIDATES: usize = 64;

/// A path a computed specifier may be.
#[derive(Clone, Debug, PartialEq)]
enum Choice {
    Text(String),
    /// The values the module gives a variable.
    Variable(String),
    /// The values the program gives a property of this name anywhere: an
    /// object literal's `name: value`, or an assignment to `x.name` or
    /// `x['name']`. `process.env.NAME` reads such values, as programs set
    /// environment variables for the processes they start.
    Property(String),
    /// Parts in order, each one of its alternatives.
    Concat(Vec<Vec<Choice>>),
}

/// The choices of `[a, b].join(separator)`.
fn joined_choices(call: &CallExpression<'_>) -> Vec<Choice> {
    let Expression::StaticMemberExpression(member) = &call.callee else {
        return Vec::new();
    };
    let Expression::ArrayExpression(array) = &member.object else {
        return Vec::new();
    };
    if member.property.name != "join" || call.arguments.len() > 1 {
        return Vec::new();
    }
    let separator = match call.arguments.first().map(Argument::as_expression) {
        None => ",".to_owned(),
        Some(Some(Expression::StringLiteral(separator))) => separator.value.to_string(),
        Some(_) => return Vec::new(),
    };
    let mut parts = Vec::new();
    for (index, element) in array.elements.iter().enumerate() {
        let Some(element) = element.as_expression() else {
            return Vec::new();
        };
        if index > 0 && !separator.is_empty() {
            parts.push(vec![Choice::Text(separator.clone())]);
        }
        let options = choices(element);
        if options.is_empty() {
            return Vec::new();
        }
        parts.push(options);
    }
    vec![Choice::Concat(parts)]
}

/// How many choices one localization may produce: a module whose variables
/// take many values that name each other would otherwise spell out every
/// combination. Past it the choices are unknown.
const LOCALIZE_BUDGET: usize = 4096;

/// The values a module gives its variables, by name.
type Bindings<'a> = HashMap<&'a str, Vec<&'a [Choice]>>;

fn index_bindings(bindings: &[(String, Vec<Choice>)]) -> Bindings<'_> {
    let mut index: Bindings<'_> = HashMap::new();
    for (name, values) in bindings {
        index.entry(name.as_str()).or_default().push(values);
    }
    index
}

/// Choices with the module's variables replaced by the values the module
/// gives them; variables it gives no value are left out. Empty when the
/// choices take more than [`LOCALIZE_BUDGET`] to spell.
fn localize(choices: &[Choice], bindings: &Bindings<'_>) -> Vec<Choice> {
    let mut budget = LOCALIZE_BUDGET;
    let local = localize_within(choices, bindings, &mut Vec::new(), &mut budget);
    if budget == 0 { Vec::new() } else { local }
}

fn localize_within(
    choices: &[Choice],
    bindings: &Bindings<'_>,
    seen: &mut Vec<String>,
    budget: &mut usize,
) -> Vec<Choice> {
    let mut out = Vec::new();
    for choice in choices {
        if *budget == 0 {
            return Vec::new();
        }
        *budget -= 1;
        match choice {
            Choice::Variable(name) => {
                if seen.contains(name) {
                    continue;
                }
                seen.push(name.clone());
                for values in bindings.get(name.as_str()).into_iter().flatten() {
                    for value in localize_within(values, bindings, seen, budget) {
                        if !out.contains(&value) {
                            out.push(value);
                        }
                    }
                }
                seen.pop();
            }
            Choice::Concat(parts) => {
                let parts: Vec<Vec<Choice>> = parts
                    .iter()
                    .map(|part| localize_within(part, bindings, seen, budget))
                    .collect();
                if parts.iter().all(|part| !part.is_empty()) {
                    out.push(Choice::Concat(parts));
                }
            }
            other => {
                if !out.contains(other) {
                    out.push(other.clone());
                }
            }
        }
    }
    out
}

fn reads_properties(choices: &[Choice]) -> bool {
    choices.iter().any(|choice| match choice {
        Choice::Property(_) => true,
        Choice::Concat(parts) => parts.iter().any(|part| reads_properties(part)),
        Choice::Text(_) | Choice::Variable(_) => false,
    })
}

/// The values properties are given, by name, across a graph.
type Properties = HashMap<String, Vec<Vec<Choice>>>;

/// How many steps one expansion may take: properties a program gives
/// thousands of values, read inside concatenations, would otherwise be
/// spelled out at every site. Past it the texts are unknown.
const EXPAND_BUDGET: usize = 50_000;

/// The texts localized choices spell, at most `limit`: properties take the
/// values `properties` gives them. Empty when spelling them takes more than
/// [`EXPAND_BUDGET`] steps.
fn expand(choices: &[Choice], properties: &Properties, limit: usize) -> Vec<String> {
    let mut budget = EXPAND_BUDGET;
    let texts = expand_within(choices, properties, &mut Vec::new(), limit, &mut budget);
    if budget == 0 { Vec::new() } else { texts }
}

fn expand_within(
    choices: &[Choice],
    properties: &Properties,
    seen: &mut Vec<String>,
    limit: usize,
    budget: &mut usize,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for choice in choices {
        if *budget == 0 {
            return Vec::new();
        }
        *budget -= 1;
        let texts = match choice {
            Choice::Text(text) => vec![text.clone()],
            Choice::Variable(_) => Vec::new(),
            Choice::Property(name) => {
                if seen.contains(name) {
                    continue;
                }
                seen.push(name.clone());
                let mut texts = Vec::new();
                for value in properties.get(name).into_iter().flatten() {
                    texts.extend(expand_within(value, properties, seen, limit, budget));
                    if *budget == 0 {
                        break;
                    }
                }
                seen.pop();
                texts
            }
            Choice::Concat(parts) => {
                let mut spelled = vec![String::new()];
                for part in parts {
                    let options = expand_within(part, properties, seen, limit, budget);
                    let product = spelled.len().saturating_mul(options.len()).min(limit);
                    if product > *budget {
                        *budget = 0;
                        return Vec::new();
                    }
                    *budget -= product;
                    spelled = spelled
                        .iter()
                        .flat_map(|prefix| {
                            options
                                .iter()
                                .map(move |option| format!("{prefix}{option}"))
                        })
                        .take(limit)
                        .collect();
                    if spelled.is_empty() {
                        break;
                    }
                }
                spelled
            }
        };
        for text in texts {
            if out.len() < limit && !out.contains(&text) {
                out.push(text);
            }
        }
    }
    out
}

/// The name a property key spells: an identifier or a string.
fn property_name(key: &PropertyKey<'_>) -> Option<String> {
    match key {
        PropertyKey::StaticIdentifier(name) => Some(name.name.to_string()),
        PropertyKey::StringLiteral(name) => Some(name.value.to_string()),
        _ => None,
    }
}

/// The paths an expression may be: string literals, templates of
/// literal choices, conditionals, `new URL(path, base)` and its `href`, and
/// variables (resolved once every assignment is known).
fn choices(expression: &Expression<'_>) -> Vec<Choice> {
    match expression {
        Expression::StringLiteral(literal) => vec![Choice::Text(literal.value.to_string())],
        Expression::TemplateLiteral(template) => {
            let mut parts = Vec::new();
            for (index, quasi) in template.quasis.iter().enumerate() {
                let Some(text) = quasi.value.cooked.as_ref() else {
                    return Vec::new();
                };
                if !text.is_empty() {
                    parts.push(vec![Choice::Text(text.to_string())]);
                }
                if let Some(expression) = template.expressions.get(index) {
                    let options = choices(expression);
                    if options.is_empty() {
                        return Vec::new();
                    }
                    parts.push(options);
                }
            }
            vec![Choice::Concat(parts)]
        }
        Expression::CallExpression(call) => joined_choices(call),
        Expression::ConditionalExpression(conditional) => {
            let mut all = choices(&conditional.consequent);
            all.extend(choices(&conditional.alternate));
            all
        }
        Expression::ParenthesizedExpression(inner) => choices(&inner.expression),
        Expression::TSAsExpression(inner) => choices(&inner.expression),
        Expression::TSSatisfiesExpression(inner) => choices(&inner.expression),
        Expression::TSNonNullExpression(inner) => choices(&inner.expression),
        Expression::NewExpression(new) if matches!(&new.callee, Expression::Identifier(callee) if callee.name == "URL") => {
            new.arguments
                .first()
                .and_then(Argument::as_expression)
                .map(choices)
                .unwrap_or_default()
        }
        Expression::StaticMemberExpression(member)
            if matches!(member.property.name.as_str(), "href" | "pathname")
                && matches!(member.object, Expression::NewExpression(_)) =>
        {
            choices(&member.object)
        }
        // `process.env.NAME`, `configuration.entryPoint`: the values the
        // program gives the property.
        Expression::StaticMemberExpression(member) => {
            vec![Choice::Property(member.property.name.to_string())]
        }
        Expression::ComputedMemberExpression(member) => match &member.expression {
            Expression::StringLiteral(key) => vec![Choice::Property(key.value.to_string())],
            _ => Vec::new(),
        },
        Expression::Identifier(identifier) => vec![Choice::Variable(identifier.name.to_string())],
        _ => Vec::new(),
    }
}

impl DynamicImports {
    /// Each site's specifier, with whether it has import options. Computed
    /// sites that read properties are also returned with their choices, as
    /// are the values the module gives properties.
    fn candidates(self) -> ImportSites {
        let mut sites = self.sites;
        let mut deferred = Vec::new();
        let none = Properties::new();
        let bindings = index_bindings(&self.bindings);
        for (pending, span, options) in self.computed {
            let local = localize(&pending, &bindings);
            let paths = expand(&local, &none, MAX_CANDIDATES);
            if !options && reads_properties(&local) {
                deferred.push((local, span));
            }
            sites.push((Specifier::Computed(paths), span, options));
        }
        let properties = self
            .properties
            .into_iter()
            .filter_map(|(name, values)| {
                let values = localize(&values, &bindings);
                (!values.is_empty()).then_some((name, values))
            })
            .collect();
        (sites, deferred, properties)
    }
}

impl DynamicImports {
    /// Notes the arguments of a call or constructor that are paths of
    /// scripts: string literals, and template literals whose text before
    /// their first substitution is a path and its query, as in
    /// `` `sw.js?v=${version}` ``. A query or fragment is not part of the
    /// path.
    fn note_worker_paths(&mut self, arguments: &[Argument<'_>]) {
        for argument in arguments {
            let text = match argument {
                Argument::StringLiteral(value) => value.value.as_str(),
                Argument::TemplateLiteral(template) => {
                    let Some(head) = template
                        .quasis
                        .first()
                        .and_then(|quasi| quasi.value.cooked.as_ref())
                    else {
                        continue;
                    };
                    if template.expressions.is_empty() || head.contains(['?', '#']) {
                        head.as_str()
                    } else {
                        continue;
                    }
                }
                _ => continue,
            };
            let text = text.split(['?', '#']).next().unwrap_or_default();
            if let Some(path) = worker_path(text)
                && !self.worker_paths.contains(&path)
            {
                self.worker_paths.push(path);
            }
        }
    }
}

/// The script path a string literal may be: a relative or folder path with
/// a JavaScript extension, without a scheme, query or fragment.
fn worker_path(text: &str) -> Option<String> {
    let path = text.trim_start_matches("./");
    let script = [".js", ".mjs", ".cjs"]
        .iter()
        .any(|extension| path.len() > extension.len() && path.ends_with(extension));
    (script
        && !path.starts_with(['/', '\\', '.'])
        && !path.contains([':', '?', '#', '*', ' ', '\n', '\\']))
    .then(|| text.to_owned())
}

impl<'a> Visit<'a> for DynamicImports {
    fn visit_new_expression(&mut self, new: &NewExpression<'a>) {
        let constructor = match &new.callee {
            Expression::Identifier(callee) => Some(callee.name.as_str()),
            Expression::StaticMemberExpression(member) => Some(member.property.name.as_str()),
            _ => None,
        };
        if matches!(constructor, Some("Worker" | "SharedWorker")) {
            self.constructs_workers = true;
        }
        self.note_worker_paths(&new.arguments);
        walk::walk_new_expression(self, new);
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        // `navigator.serviceWorker.register(url)` starts a worker too.
        if let Expression::StaticMemberExpression(member) = &call.callee
            && member.property.name == "register"
            && let Expression::StaticMemberExpression(container) = &member.object
            && container.property.name == "serviceWorker"
        {
            self.constructs_workers = true;
        }
        self.note_worker_paths(&call.arguments);
        for pair in call.arguments.windows(2) {
            if let [
                Argument::StringLiteral(package),
                Argument::StringLiteral(file),
            ] = pair
            {
                let pair = (package.value.to_string(), file.value.to_string());
                if !self.package_pairs.contains(&pair) {
                    self.package_pairs.push(pair);
                }
            }
        }
        walk::walk_call_expression(self, call);
    }

    fn visit_import_expression(&mut self, expression: &ImportExpression<'a>) {
        let options = expression.options.is_some() || expression.phase.is_some();
        match &expression.source {
            Expression::StringLiteral(value) => {
                self.sites.push((
                    Specifier::Literal(value.value.to_string()),
                    expression.span,
                    options,
                ));
            }
            source => self
                .computed
                .push((choices(source), expression.span, options)),
        }
        walk::walk_import_expression(self, expression);
    }

    fn visit_variable_declarator(&mut self, declarator: &VariableDeclarator<'a>) {
        if let BindingPattern::BindingIdentifier(name) = &declarator.id
            && let Some(init) = &declarator.init
        {
            let values = choices(init);
            if !values.is_empty() {
                self.bindings.push((name.name.to_string(), values));
            }
        }
        walk::walk_variable_declarator(self, declarator);
    }

    fn visit_assignment_expression(&mut self, assignment: &AssignmentExpression<'a>) {
        if assignment.operator == oxc::syntax::operator::AssignmentOperator::Assign {
            let target = match &assignment.left {
                AssignmentTarget::AssignmentTargetIdentifier(name) => {
                    Some((false, name.name.to_string()))
                }
                AssignmentTarget::StaticMemberExpression(member) => {
                    Some((true, member.property.name.to_string()))
                }
                AssignmentTarget::ComputedMemberExpression(member) => match &member.expression {
                    Expression::StringLiteral(key) => Some((true, key.value.to_string())),
                    _ => None,
                },
                _ => None,
            };
            if let Some((property, name)) = target {
                let values = choices(&assignment.right);
                if !values.is_empty() {
                    if property {
                        self.properties.push((name, values));
                    } else {
                        self.bindings.push((name, values));
                    }
                }
            }
        }
        walk::walk_assignment_expression(self, assignment);
    }

    fn visit_object_property(&mut self, property: &ObjectProperty<'a>) {
        if property.kind == PropertyKind::Init
            && !property.computed
            && let Some(name) = property_name(&property.key)
        {
            let values = choices(&property.value);
            if !values.is_empty() {
                self.properties.push((name, values));
            }
        }
        walk::walk_object_property(self, property);
    }
}

/// The export name of the default a TypeScript module compiled to CommonJS
/// imports from a CommonJS module: its `default` when it sets `__esModule`,
/// else `module.exports`, as `tsc`'s `__importDefault` reads it.
const INTEROP_DEFAULT: &str = "\0default";

/// Whether a module is TypeScript a package's own build compiles to
/// CommonJS (see `CompileOptions::commonjs_folders`).
fn compiles_to_commonjs(options: &CompileOptions, path: &Path) -> bool {
    SourceType::from_path(path).is_ok_and(|kind| kind.is_typescript())
        && options
            .commonjs_folders
            .iter()
            .any(|folder| path.starts_with(folder))
}

/// What the path of a host module (see `link`) puts between its importer's
/// path and the name it requires (followed by `.cjs`).
pub(super) const HOST: &str = "?host=";

/// The app-relative path a module of an emitted source folder has where its
/// package's own build emits it: TypeScript extensions become JavaScript.
pub(super) fn emitted_path(folders: &[(PathBuf, String)], path: &Path) -> Option<String> {
    folders.iter().find_map(|(source, emitted)| {
        let rest = path.strip_prefix(source).ok()?;
        let rest = rest.to_string_lossy().replace('\\', "/");
        let rest = match rest.rsplit_once('.') {
            Some((stem, "ts" | "tsx")) => format!("{stem}.js"),
            Some((stem, "mts")) => format!("{stem}.mjs"),
            Some((stem, "cts")) => format!("{stem}.cjs"),
            _ => rest,
        };
        Some(format!("{emitted}/{rest}"))
    })
}

/// Whether a request names a package rather than a path.
fn is_bare(request: &str) -> bool {
    !(request.starts_with('.')
        || request.starts_with('/')
        || request.starts_with('\\')
        || request.as_bytes().get(1) == Some(&b':')
        || request.contains("://"))
}

struct Graph {
    /// The application folder whose CommonJS modules take their path
    /// globals from where the application runs.
    application: Option<std::path::PathBuf>,
    /// Source folders whose modules run from the files emitted from them
    /// (`CompileOptions::emitted_folders`).
    emitted_folders: Vec<(PathBuf, String)>,
    records: Vec<Record>,
    /// The roots of this project graph, in application lifecycle order.
    entries: Vec<usize>,
    /// Dynamic import source sites mapped to lazy project entries.
    dynamic_imports: HashMap<(usize, Span), DynamicImport>,
    /// CommonJS request slots reached through ESM `createRequire` calls.
    create_require_requests: HashSet<(usize, usize)>,
    /// The cell of each module's namespace object.
    namespaces: Vec<usize>,
    /// The cells of each module's exported locals.
    locals: Vec<HashMap<Local, usize>>,
    /// CommonJS default exports are distinct from the private factory cell.
    commonjs: Vec<Option<usize>>,
    /// Named exports imported from CommonJS or native modules, by module and
    /// name: snapshots taken when the module is evaluated, as in Node.
    named: BTreeMap<(usize, String), usize>,
    cells: usize,
}

const ELECTRON_NATIVE_EXPORTS: &[&str] = &[
    "app",
    "autoUpdater",
    "BrowserWindow",
    "clipboard",
    "contentTracing",
    "contextBridge",
    "crashReporter",
    "desktopCapturer",
    "dialog",
    "ipcMain",
    "ipcRenderer",
    "Menu",
    "MenuItem",
    "MessageChannelMain",
    "globalShortcut",
    "nativeImage",
    "nativeTheme",
    "net",
    "Notification",
    "powerMonitor",
    "powerSaveBlocker",
    "protocol",
    "safeStorage",
    "screen",
    "session",
    "shell",
    "systemPreferences",
    "Tray",
    "utilityProcess",
    "WebContentsView",
    "webContents",
    "webFrame",
    "webUtils",
];

/// Exports a native application cannot provide, by module and name, with
/// why: a program that imports one compiles with a warning, and using it
/// throws when the application runs.
const UNAVAILABLE_EXPORTS: &[(&str, &str, &str)] = &[(
    "electron",
    "WebContentsView",
    "web content views need Chromium's compositor; constructing one throws when the application runs",
)];

/// Exports of native modules the Node library implements, by module and
/// name: importing one links its library module, which the runtime's
/// export loads.
const LIBRARY_EXPORTS: &[(&str, &str, &str)] = &[
    (
        "electron",
        "MessageChannelMain",
        "node:internal/electron/utility_process",
    ),
    (
        "electron",
        "utilityProcess",
        "node:internal/electron/utility_process",
    ),
    ("electron", "contentTracing", "node:internal/electron/media"),
    (
        "electron",
        "desktopCapturer",
        "node:internal/electron/media",
    ),
    (
        "child_process",
        "fork",
        "node:internal/electron/utility_process",
    ),
];

const CHILD_PROCESS_NATIVE_EXPORTS: &[&str] = &[
    "exec",
    "execFile",
    "execFileSync",
    "execSync",
    "fork",
    "spawn",
    "spawnSync",
];

impl Graph {
    /// The module a request names; linking runs only when all resolved.
    fn target(&self, module: usize, request: usize) -> usize {
        self.records[module].requests[request]
            .2
            .expect("requests resolve before linking")
    }

    /// `ResolveExport`: follows indirect and star exports to a cell.
    /// What `import x = require()` binds from `target`: a CommonJS module's
    /// `module.exports` (its default export), an ES module's namespace.
    fn required(&self, target: usize, visiting: &mut Vec<(usize, String)>) -> Found {
        if self.commonjs[target].is_some() {
            self.resolve(target, "default", visiting)
        } else {
            Found::Cell(self.namespaces[target])
        }
    }

    fn resolve(&self, module: usize, name: &str, visiting: &mut Vec<(usize, String)>) -> Found {
        if visiting.contains(&(module, name.to_owned())) {
            return Found::Missing;
        }
        visiting.push((module, name.to_owned()));
        let record = &self.records[module];
        if record.placeholder {
            return Found::Cell(self.namespaces[module]);
        }
        if let Some(cell) = self.commonjs[module] {
            return if name == "default" {
                Found::Cell(cell)
            } else if (self.records[module].path == Path::new("electron")
                && !ELECTRON_NATIVE_EXPORTS.contains(&name))
                || (self.records[module].path == Path::new("child_process")
                    && !CHILD_PROCESS_NATIVE_EXPORTS.contains(&name))
            {
                Found::Missing
            } else {
                self.named
                    .get(&(module, name.to_owned()))
                    .map_or(Found::Missing, |cell| Found::Cell(*cell))
            };
        }
        if let Some((_, local)) = record.locals.iter().find(|(exported, _)| exported == name) {
            if let Local::Symbol(symbol) = local
                && let Some((_, request, imported, _)) = record
                    .imports
                    .iter()
                    .find(|(imported, ..)| imported == symbol)
            {
                let target = self.target(module, *request);
                return match imported {
                    Imported::Namespace => Found::Cell(self.namespaces[target]),
                    Imported::Name(name) => self.resolve(target, name, visiting),
                    Imported::Required => self.required(target, visiting),
                };
            }
            return Found::Cell(self.locals[module][local]);
        }
        if let Some((_, request, imported, _)) = record
            .indirect
            .iter()
            .find(|(exported, ..)| exported == name)
        {
            let target = self.target(module, *request);
            return match imported {
                Imported::Namespace => Found::Cell(self.namespaces[target]),
                Imported::Name(imported) => self.resolve(target, imported, visiting),
                Imported::Required => self.required(target, visiting),
            };
        }
        if name == "default" {
            return Found::Missing;
        }
        let mut found = None;
        for request in &record.stars {
            match self.resolve(self.target(module, *request), name, visiting) {
                Found::Cell(cell) if found.is_some_and(|found| found != cell) => {
                    return Found::Ambiguous;
                }
                Found::Cell(cell) => found = Some(cell),
                Found::Ambiguous => return Found::Ambiguous,
                Found::Missing => {}
            }
        }
        found.map_or(Found::Missing, Found::Cell)
    }

    /// Every name a module exports, with stars expanded and ambiguous or
    /// missing names left out, in code unit order.
    fn names(&self, module: usize, visited: &mut HashSet<usize>, names: &mut Vec<String>) {
        if !visited.insert(module) {
            return;
        }
        let record = &self.records[module];
        names.extend(record.locals.iter().map(|(name, _)| name.clone()));
        names.extend(record.indirect.iter().map(|(name, ..)| name.clone()));
        for request in &record.stars {
            let mut starred = Vec::new();
            self.names(self.target(module, *request), visited, &mut starred);
            names.extend(starred.into_iter().filter(|name| name != "default"));
        }
    }
}

/// The request a pair of literals makes when it may name a package and a
/// script in it: `<package>/<file>`.
fn package_script(package: &str, file: &str) -> Option<String> {
    let scoped = package.starts_with('@');
    let named = !package.is_empty()
        && !package.starts_with('.')
        && !package.contains(['\\', ':', ' '])
        && package.split('/').count() == if scoped { 2 } else { 1 };
    let file = file.trim_start_matches("./");
    (named && !file.is_empty() && !file.starts_with("..") && file.ends_with(".js"))
        .then(|| format!("{package}/{file}"))
}

/// Looks up the package scripts the scanned modules name, adding those
/// that exist as modules of the graph. Whether any module was added.
fn resolve_package_scripts(
    records: &mut Vec<Option<Record>>,
    indexes: &mut HashMap<PathBuf, usize>,
    pending: &mut Vec<(PathBuf, ModuleSource)>,
    resolve: ResolveSource<'_>,
    load: LoadSource<'_>,
) -> bool {
    let mut added = false;
    for index in 0..records.len() {
        let Some(record) = records[index].as_ref() else {
            continue;
        };
        if record.package_pairs_done == record.package_pairs.len() {
            continue;
        }
        let from = record.path.clone();
        let pairs = record.package_pairs[record.package_pairs_done..].to_vec();
        let mut found = Vec::new();
        for (package, file) in pairs {
            let Some(request) = package_script(&package, &file) else {
                continue;
            };
            if let Some(target) = resolve(&from, &request, RequestKind::Script)
                && let Some(text) = load(&target)
            {
                found.push((
                    format!("node_modules/{request}").to_ascii_lowercase(),
                    target,
                    text,
                ));
            }
        }
        let record = records[index].as_mut().expect("the record was read above");
        record.package_pairs_done = record.package_pairs.len();
        for (key, target, text) in found {
            let next = match indexes.get(&target) {
                Some(next) => *next,
                None => {
                    let next = records.len();
                    indexes.insert(target.clone(), next);
                    records.push(None);
                    pending.push((target, text));
                    added = true;
                    next
                }
            };
            records[index]
                .as_mut()
                .expect("the record was read above")
                .package_scripts
                .push((key, next));
        }
    }
    added
}

/// Looks up the scripts the scanned modules name by path when the graph
/// constructs workers, adding those that exist as modules of the graph,
/// keyed by the path the program names them by. Whether any module was
/// added.
fn resolve_worker_scripts(
    records: &mut Vec<Option<Record>>,
    indexes: &mut HashMap<PathBuf, usize>,
    pending: &mut Vec<(PathBuf, ModuleSource)>,
    resolve: ResolveSource<'_>,
    load: LoadSource<'_>,
) -> bool {
    if !records
        .iter()
        .flatten()
        .any(|record| record.constructs_workers)
    {
        return false;
    }
    let mut added = false;
    for index in 0..records.len() {
        let Some(record) = records[index].as_ref() else {
            continue;
        };
        if record.worker_paths_done == record.worker_paths.len() {
            continue;
        }
        let from = record.path.clone();
        let paths = record.worker_paths[record.worker_paths_done..].to_vec();
        let mut found = Vec::new();
        for path in paths {
            if let Some(target) = resolve(&from, &path, RequestKind::Worker)
                && let Some(text) = load(&target)
            {
                let key = path.trim_start_matches("./").replace('\\', "/");
                found.push((key.to_ascii_lowercase(), target, text));
            }
        }
        let record = records[index].as_mut().expect("the record was read above");
        record.worker_paths_done = record.worker_paths.len();
        for (key, target, text) in found {
            let next = match indexes.get(&target) {
                Some(next) => *next,
                None => {
                    let next = records.len();
                    indexes.insert(target.clone(), next);
                    records.push(None);
                    pending.push((target, text));
                    added = true;
                    next
                }
            };
            records[index]
                .as_mut()
                .expect("the record was read above")
                .package_scripts
                .push((key, next));
        }
    }
    added
}

/// Adds the modules computed imports that read properties can name, from
/// the values the scanned graph gives those properties, as lazy entries.
/// Whether any site gained a module.
fn resolve_deferred_imports(
    records: &mut Vec<Option<Record>>,
    indexes: &mut HashMap<PathBuf, usize>,
    pending: &mut Vec<(PathBuf, ModuleSource)>,
    resolve: ResolveSource<'_>,
    load: LoadSource<'_>,
) -> bool {
    if !records
        .iter()
        .flatten()
        .any(|record| !record.deferred_imports.is_empty())
    {
        return false;
    }
    let mut properties = Properties::new();
    for record in records.iter().flatten() {
        for (name, values) in &record.property_values {
            let known = properties.entry(name.clone()).or_default();
            if !known.contains(values) {
                known.push(values.clone());
            }
        }
    }
    let mut added = false;
    for index in 0..records.len() {
        let Some(record) = records[index].as_ref() else {
            continue;
        };
        let mut found = Vec::new();
        for (choices, site) in &record.deferred_imports {
            for specifier in expand(choices, &properties, MAX_PROPERTY_CANDIDATES) {
                if record
                    .dynamic_imports
                    .iter()
                    .any(|request| request.0 == specifier && request.1 == *site)
                    || found
                        .iter()
                        .any(|(known, at, _)| *known == specifier && at == site)
                {
                    continue;
                }
                if let Some(target) = resolve(&record.path, &specifier, RequestKind::Import)
                    && let Some(text) = load(&target)
                {
                    found.push((specifier, *site, (target, text)));
                }
            }
        }
        for (specifier, site, (target, text)) in found {
            added = true;
            let next = match indexes.get(&target) {
                Some(next) => *next,
                None => {
                    let next = records.len();
                    indexes.insert(target.clone(), next);
                    records.push(None);
                    pending.push((target, text));
                    next
                }
            };
            records[index]
                .as_mut()
                .expect("the record was read above")
                .dynamic_imports
                .push((specifier, site, Some(next)));
        }
    }
    added
}

/// Compiles the module graph reachable from an entry module.
pub(super) struct ProjectSource {
    pub module_sources: Vec<Vec<String>>,
    pub entry_source: String,
    pub native_dependencies: BTreeSet<String>,
    pub entry_count: usize,
    pub warnings: Vec<Diagnostic>,
    /// Package scripts by key, with their entries.
    pub scripts: Vec<(String, usize)>,
    /// The application paths of modules that load, when they run, a module
    /// the build could not resolve: one the running program serves (see
    /// `HOST`) or a missing one.
    pub host_importers: Vec<String>,
}

pub(super) fn project(
    entries: &[&Path],
    resolve: ResolveSource<'_>,
    load: LoadSource<'_>,
    options: &CompileOptions,
) -> Result<ProjectSource, Vec<Diagnostic>> {
    let native_dependencies = Rc::new(RefCell::new(BTreeSet::new()));
    // Modules whose requires of missing modules load, when they run,
    // whatever the program then serves by those names.
    let mut late_importers: BTreeSet<PathBuf> = BTreeSet::new();
    let outer_resolve = resolve;
    let outer_load = load;
    let resolve: ResolveSource<'_> = &mut |from: &Path, request: &str, kind| {
        if request == USING_HELPER {
            Some(PathBuf::from(USING_HELPER))
        } else {
            outer_resolve(from, request, kind)
        }
    };
    let load: LoadSource<'_> = &mut |path: &Path| {
        if path == Path::new(USING_HELPER) {
            Some(ModuleSource {
                text: USING_HELPER_MODULE.to_owned(),
                kind: ModuleKind::Native,
            })
        } else {
            outer_load(path)
        }
    };
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    if entries.is_empty() {
        let source = Source {
            path: Path::new(""),
            text: "",
        };
        return Err(vec![source.error(
            Span::default(),
            "a project needs at least one entry module",
        )]);
    }
    let mut records: Vec<Option<Record>> = Vec::new();
    let mut indexes: HashMap<PathBuf, usize> = HashMap::new();
    let mut roots = Vec::new();
    let mut pending = Vec::new();
    for entry in entries {
        let path = entry.to_path_buf();
        if let Some(index) = indexes.get(&path) {
            roots.push(*index);
            continue;
        }
        let index = records.len();
        indexes.insert(path.clone(), index);
        records.push(None);
        roots.push(index);
        match load(entry) {
            Some(text) => pending.push((path, text)),
            None => {
                let source = Source {
                    path: entry,
                    text: "",
                };
                errors.push(source.error(Span::default(), "cannot read the module"));
                records[index] = Some(placeholder(entry));
            }
        }
    }
    // Scripts the project names, which the program loads by URLs it builds.
    let mut named_scripts = Vec::new();
    for (key, source) in &options.named_scripts {
        let index = match indexes.get(source) {
            Some(index) => *index,
            None => {
                let index = records.len();
                indexes.insert(source.clone(), index);
                records.push(None);
                match load(source) {
                    Some(text) => pending.push((source.clone(), text)),
                    None => {
                        let file = Source {
                            path: source,
                            text: "",
                        };
                        errors.push(file.error(Span::default(), "cannot read the script"));
                        records[index] = Some(placeholder(source));
                    }
                }
                index
            }
        };
        named_scripts.push((key.replace('\\', "/").to_ascii_lowercase(), index));
    }
    loop {
        while let Some((path, text)) = pending.pop() {
            let index = indexes[&path];
            let mut record = match text.kind {
                ModuleKind::EsModule => scan(&path, text.text, options),
                ModuleKind::CommonJs => commonjs::scan(&path, text.text, options),
                ModuleKind::Json => json::scan(&path, text.text),
                ModuleKind::Native => {
                    native_dependencies.borrow_mut().insert(text.text.clone());
                    Ok(native(&path, &text.text))
                }
            }
            .unwrap_or_else(|found| {
                errors.extend(found);
                placeholder(&path)
            });
            errors.append(&mut record.errors);
            warnings.append(&mut record.warnings);
            let first_error = errors.len();
            for resolution in &mut record.resolutions {
                resolution.path = resolve(&path, &resolution.request, RequestKind::Resolve);
                if resolution.path.is_none() {
                    errors.push(
                        Source {
                            path: &path,
                            text: &record.source,
                        }
                        .error(
                            resolution.location,
                            &format!("cannot resolve module '{}'", resolution.request),
                        ),
                    );
                }
            }
            let mut dropped = HashSet::new();
            for (request_index, request) in record.requests.iter_mut().enumerate() {
                let source = Source {
                    path: &path,
                    text: &record.source,
                };
                let kind = if record.original.is_some()
                    || record
                        .create_require_request_indexes
                        .contains(&request_index)
                {
                    RequestKind::Require
                } else {
                    RequestKind::Import
                };
                let resolved = resolve(&path, &request.0, kind);
                let mut loaded = None;
                // A require whose module is missing throws MODULE_NOT_FOUND
                // when it runs, as in Node: programs guard requires of
                // optional packages and other platforms' binaries. Outside a
                // `try`, the build warns.
                let guarded = record.optional.contains(&request.1);
                if guarded || kind == RequestKind::Require {
                    let missing = match resolved.as_deref() {
                        None => true,
                        Some(target) if indexes.contains_key(target) => false,
                        Some(target) => {
                            loaded = load(target);
                            loaded.is_none()
                        }
                    };
                    if missing {
                        if !guarded && !path.to_string_lossy().contains(HOST) {
                            let found = if resolved.is_some() {
                                "read"
                            } else {
                                "resolve"
                            };
                            warnings.push(source.error(
                                request.1,
                                &format!(
                                    "cannot {found} module '{}'; requiring it throws MODULE_NOT_FOUND when it runs",
                                    request.0
                                ),
                            ));
                        }
                        dropped.insert(request_index);
                        late_importers.insert(path.clone());
                        continue;
                    }
                }
                // A bare name a plugin module imports that the build cannot
                // resolve is one the running program serves, as an editor
                // serves its API to its extensions: a module of its own for
                // each importer requires it when the importer loads, through
                // `Module._load` with the importer as parent.
                if resolved.is_none()
                    && is_bare(&request.0)
                    && options
                        .emitted_folders
                        .iter()
                        .any(|(folder, _)| path.starts_with(folder))
                {
                    // The key ends in `.cjs`, the kind of module it is.
                    let key = PathBuf::from(format!("{}{HOST}{}.cjs", path.display(), request.0));
                    let next = *indexes.entry(key.clone()).or_insert(records.len());
                    request.2 = Some(next);
                    if next == records.len() {
                        records.push(None);
                        pending.push((
                            key,
                            ModuleSource {
                                text: format!("module.exports = require({:?});\n", request.0),
                                kind: ModuleKind::CommonJs,
                            },
                        ));
                    }
                    continue;
                }
                if resolved.is_none() {
                    errors.push(
                        source.error(request.1, &format!("cannot resolve module '{}'", request.0)),
                    );
                }
                // Unresolved specifiers share one placeholder per specifier.
                let key = resolved
                    .clone()
                    .unwrap_or_else(|| PathBuf::from(format!("\0unresolved:{}", request.0)));
                let next = *indexes.entry(key.clone()).or_insert(records.len());
                request.2 = Some(next);
                if next < records.len() {
                    continue;
                }
                records.push(None);
                match resolved
                    .as_deref()
                    .map(|target| (target, loaded.take().or_else(|| load(target))))
                {
                    Some((target, Some(text))) => pending.push((target.to_path_buf(), text)),
                    Some((_, None)) => {
                        errors.push(
                            source.error(request.1, &format!("cannot read module '{}'", request.0)),
                        );
                        records[next] = Some(placeholder(&key));
                    }
                    None => records[next] = Some(placeholder(&key)),
                }
            }
            if !dropped.is_empty() {
                let mut index = 0;
                record.requests.retain(|_| {
                    index += 1;
                    !dropped.contains(&(index - 1))
                });
                // createRequire calls keep their requests' new places; one whose
                // module is missing throws MODULE_NOT_FOUND when it runs.
                let moved = |old: usize| old - dropped.iter().filter(|gone| **gone < old).count();
                for call in &mut record.create_require_calls {
                    call.3 = call.3.filter(|old| !dropped.contains(old)).map(moved);
                }
                record.create_require_request_indexes = record
                    .create_require_request_indexes
                    .iter()
                    .filter(|old| !dropped.contains(old))
                    .map(|old| moved(*old))
                    .collect();
            }
            // A computed import's candidates that do not resolve are paths it
            // cannot take; the site needs one that does.
            let computed = record.computed_imports.clone();
            record.dynamic_imports.retain(|request| {
                !computed.contains(&request.1)
                    || resolve(&path, &request.0, RequestKind::Import)
                        .is_some_and(|target| load(&target).is_some())
            });
            for site in &computed {
                // Sites that read properties are judged once the graph is known.
                if record
                    .deferred_imports
                    .iter()
                    .any(|(_, deferred)| deferred == site)
                {
                    continue;
                }
                // Sites without candidates were reported when the module was
                // scanned; the others report candidates that all failed.
                let named = warnings
                    .iter()
                    .any(|warning: &Diagnostic| warning.span == *site && warning.path == path);
                if !named
                    && !record
                        .dynamic_imports
                        .iter()
                        .any(|request| request.1 == *site)
                {
                    let source = Source {
                        path: &path,
                        text: &record.source,
                    };
                    warnings.push(source.error(
                    *site,
                    "no module this dynamic import can name resolves; it rejects with ERR_MODULE_NOT_FOUND when it runs",
                ));
                }
            }
            for request in &mut record.dynamic_imports {
                let source = Source {
                    path: &path,
                    text: &record.source,
                };
                let resolved = resolve(&path, &request.0, RequestKind::Import);
                if resolved.is_none() {
                    errors.push(source.error(
                        request.1,
                        &format!("cannot resolve dynamic import module '{}'", request.0),
                    ));
                }
                let key = resolved
                    .clone()
                    .unwrap_or_else(|| PathBuf::from(format!("\0unresolved:{}", request.0)));
                let next = *indexes.entry(key.clone()).or_insert(records.len());
                request.2 = Some(next);
                if next < records.len() {
                    continue;
                }
                records.push(None);
                match resolved.as_deref().map(|target| (target, load(target))) {
                    Some((target, Some(text))) => pending.push((target.to_path_buf(), text)),
                    Some((_, None)) => {
                        errors.push(source.error(
                            request.1,
                            &format!("cannot read dynamic import module '{}'", request.0),
                        ));
                        records[next] = Some(placeholder(&key));
                    }
                    None => records[next] = Some(placeholder(&key)),
                }
            }
            if let Some(original) = &record.original {
                for error in &mut errors[first_error..] {
                    commonjs::remap(error, original);
                }
            }
            records[index] = Some(record);
        }
        // Computed imports that read properties take the values the graph
        // gives them; modules those name may give more, so this repeats until
        // no site gains a module.
        let scripts = options.package_scripts
            && (resolve_package_scripts(
                &mut records,
                &mut indexes,
                &mut pending,
                &mut *resolve,
                &mut *load,
            ) | resolve_worker_scripts(
                &mut records,
                &mut indexes,
                &mut pending,
                &mut *resolve,
                &mut *load,
            ));
        if !resolve_deferred_imports(
            &mut records,
            &mut indexes,
            &mut pending,
            &mut *resolve,
            &mut *load,
        ) && !scripts
        {
            break;
        }
    }
    for record in records.iter().flatten() {
        let source = Source {
            path: &record.path,
            text: &record.source,
        };
        for (_, site) in &record.deferred_imports {
            if !record
                .dynamic_imports
                .iter()
                .any(|request| request.1 == *site)
            {
                warnings.push(source.error(
                    *site,
                    "no module this dynamic import can name resolves; it rejects with ERR_MODULE_NOT_FOUND when it runs",
                ));
            }
        }
    }
    let records: Vec<Record> = records
        .into_iter()
        .map(|record| record.expect("every module is scanned or a placeholder"))
        .collect();
    // Cells: a namespace per module, then one per exported local.
    let mut cells = 0;
    let mut namespaces = Vec::new();
    let mut locals = Vec::new();
    let mut commonjs = Vec::new();
    for record in &records {
        namespaces.push(cells);
        cells += 1;
        let mut map = HashMap::new();
        for (_, local) in &record.locals {
            map.entry(*local).or_insert_with(|| {
                cells += 1;
                cells - 1
            });
        }
        locals.push(map);
        commonjs.push(
            (record.original.is_some() || record.data.is_some()).then(|| {
                cells += 1;
                cells - 1
            }),
        );
    }
    let mut named = BTreeMap::new();
    for record in &records {
        let imports = record
            .imports
            .iter()
            .map(|(_, request, imported, _)| (*request, imported));
        let indirect = record
            .indirect
            .iter()
            .map(|(_, request, imported, _)| (*request, imported));
        for (request, imported) in imports.chain(indirect) {
            // Unresolved requests link to placeholders, so every one has a
            // target here.
            let target = record.requests[request]
                .2
                .expect("requests have targets before linking");
            if let Imported::Name(name) = imported
                && records[target].named_exports
                && (name != "default" || compiles_to_commonjs(options, &record.path))
            {
                let name = if name == "default" {
                    INTEROP_DEFAULT.to_owned()
                } else {
                    name.clone()
                };
                named.entry((target, name)).or_insert_with(|| {
                    cells += 1;
                    cells - 1
                });
            }
        }
    }
    let mut entry_roots = roots;
    let mut dynamic_imports = HashMap::new();
    let create_require_requests = records
        .iter()
        .enumerate()
        .flat_map(|(module, record)| {
            record
                .create_require_calls
                .iter()
                .filter_map(move |(_, _, _, request)| request.map(|request| (module, request)))
        })
        .collect();
    let mut scripts: Vec<(String, usize)> = Vec::new();
    // The scripts the project names first, then those the graph found.
    let found = records.iter().flat_map(|record| &record.package_scripts);
    for (key, target) in named_scripts.iter().chain(found) {
        if scripts.iter().any(|(known, _)| known == key) {
            continue;
        }
        let entry = entry_roots
            .iter()
            .position(|root| root == target)
            .unwrap_or_else(|| {
                entry_roots.push(*target);
                entry_roots.len() - 1
            });
        scripts.push((key.clone(), entry));
    }
    for (module, record) in records.iter().enumerate() {
        // Computed imports without candidates only reject.
        for span in &record.computed_imports {
            dynamic_imports
                .entry((module, *span))
                .or_insert_with(|| DynamicImport::Computed(Vec::new()));
        }
        for (specifier, span, target) in &record.dynamic_imports {
            let target = target.expect("dynamic import requests have targets before linking");
            let entry = entry_roots
                .iter()
                .position(|root| *root == target)
                .unwrap_or_else(|| {
                    entry_roots.push(target);
                    entry_roots.len() - 1
                });
            if record.computed_imports.contains(span) {
                let found = dynamic_imports
                    .entry((module, *span))
                    .or_insert_with(|| DynamicImport::Computed(Vec::new()));
                if let DynamicImport::Computed(candidates) = found {
                    let key = runtime_key(specifier);
                    if !candidates.contains(&(key.clone(), entry)) {
                        candidates.push((key, entry));
                    }
                }
            } else {
                dynamic_imports.insert((module, *span), DynamicImport::Entry(entry));
            }
        }
    }
    let graph = Graph {
        application: options
            .application
            .clone()
            .filter(|_| options.bundle.is_none()),
        emitted_folders: options.emitted_folders.clone(),
        records,
        entries: entry_roots,
        dynamic_imports,
        create_require_requests,
        namespaces,
        locals,
        commonjs,
        named,
        cells,
    };
    let mut module_sources = Vec::with_capacity(graph.records.len());
    for (index, record) in graph.records.iter().enumerate() {
        if record.placeholder {
            module_sources.push(vec![String::new()]);
            continue;
        }
        if let Some(data) = &record.data {
            let mut code = json::factory(index, graph.locals[index][&Local::Default], data);
            code.push_str(&commonjs::factory(&graph, index));
            module_sources.push(vec![code]);
            continue;
        }
        let first_error = errors.len();
        let source = Source {
            path: &record.path,
            text: &record.source,
        };
        let mut create_require_calls = HashMap::new();
        let mut missing_requires = HashSet::new();
        for (_, name, span, request) in &record.create_require_calls {
            let Some(request) = request else {
                missing_requires.insert(*span);
                continue;
            };
            let target = graph.target(index, *request);
            if graph.commonjs[target].is_none() {
                errors.push(source.error(
                    *span,
                    &format!(
                        "createRequire cannot synchronously load non-CommonJS module '{name}'"
                    ),
                ));
            } else {
                create_require_calls.insert(*span, target);
            }
        }
        let mut link = Link {
            index,
            strict: record.strict,
            native_dependencies: native_dependencies.clone(),
            cells: HashMap::new(),
            default: graph.locals[index].get(&Local::Default).copied(),
            resolutions: record
                .resolutions
                .iter()
                .filter_map(|resolution| {
                    resolution
                        .path
                        .as_ref()
                        .map(|path| (resolution.call, path.clone()))
                })
                .collect(),
            dynamic_imports: graph
                .dynamic_imports
                .iter()
                .filter(|((module, _), _)| *module == index)
                .map(|((_, span), entry)| (*span, entry.clone()))
                .collect(),
            create_require_calls,
            missing_requires,
            create_require_constructors: record.create_require_constructors.clone(),
            create_require_values: record.create_require_values.clone(),
            require_scopes: record.require_scopes.clone(),
        };
        for (local, cell) in &graph.locals[index] {
            if let Local::Symbol(symbol) = local {
                link.cells.insert(*symbol, *cell);
            }
        }
        let typescript = SourceType::from_path(&record.path).is_ok_and(|kind| kind.is_typescript());
        for (symbol, request, imported, span) in &record.imports {
            let target = graph.target(index, *request);
            if graph.commonjs[target].is_some()
                && !graph.records[target].named_exports
                && *imported == Imported::Namespace
            {
                errors.push(source.error(
                    *span,
                    "JSON module namespace imports are not compiled yet; only default imports are supported",
                ));
            }
            let (found, name) = match imported {
                Imported::Namespace => (Found::Cell(graph.namespaces[target]), "*"),
                Imported::Name(name)
                    if name == "default"
                        && graph.commonjs[target].is_some()
                        && graph.records[target].named_exports
                        && compiles_to_commonjs(options, &record.path) =>
                {
                    (
                        graph.resolve(target, INTEROP_DEFAULT, &mut Vec::new()),
                        "default",
                    )
                }
                Imported::Name(name) => {
                    (graph.resolve(target, name, &mut Vec::new()), name.as_str())
                }
                Imported::Required => (graph.required(target, &mut Vec::new()), "default"),
            };
            // A TypeScript module drops what it imports of another's types,
            // as `tsc` does, even when it re-exports the name.
            let only_type =
                |name: &str| typescript && graph.records[target].type_exports.contains(name);
            let specifier = &record.requests[*request].0;
            if let Some((_, _, reason)) = UNAVAILABLE_EXPORTS.iter().find(|(native, export, _)| {
                *export == name && graph.records[target].path == Path::new(native)
            }) {
                warnings.push(source.error(
                    *span,
                    &format!("'{name}' from '{specifier}' is not available: {reason}"),
                ));
            }
            if let Some((_, _, module)) = LIBRARY_EXPORTS.iter().find(|(native, export, _)| {
                *export == name && graph.records[target].path == Path::new(native)
            }) {
                native_dependencies
                    .borrow_mut()
                    .insert((*module).to_owned());
            }
            match found {
                Found::Cell(cell) => {
                    link.cells.insert(*symbol, cell);
                }
                Found::Missing if only_type(name) => {}
                Found::Missing => errors.push(source.error(
                    *span,
                    &format!("module '{specifier}' has no export '{name}'"),
                )),
                Found::Ambiguous => errors.push(source.error(
                    *span,
                    &format!("export '{name}' of module '{specifier}' is ambiguous"),
                )),
            }
        }
        for (exported, request, imported, span) in &record.indirect {
            let target = graph.target(index, *request);
            if graph.commonjs[target].is_some()
                && !graph.records[target].named_exports
                && *imported == Imported::Namespace
            {
                errors.push(source.error(
                    *span,
                    &format!("JSON module namespace re-export '{exported}' is not compiled yet"),
                ));
            }
            if let Imported::Name(imported) = imported
                && !matches!(
                    graph.resolve(target, imported, &mut Vec::new()),
                    Found::Cell(_)
                )
                && !(typescript && graph.records[target].type_exports.contains(imported))
            {
                errors.push(source.error(
                    *span,
                    &format!("cannot re-export '{imported}' as '{exported}'"),
                ));
            }
        }
        for request in &record.stars {
            if graph.commonjs[graph.target(index, *request)].is_some() {
                errors.push(source.error(
                    record.requests[*request].1,
                    "CommonJS star re-export interop is not compiled yet",
                ));
            }
        }
        let allocator = Allocator::default();
        let (program, scoping) = parse(&allocator, &source, options).expect("parsed when scanned");
        let analysis = Analysis::of(&program, &scoping);
        let emit = |part_bytes| {
            emit::module(
                &program, &scoping, &analysis, &source, &link, options, part_bytes,
            )
        };
        let mut parts = match emit(None) {
            // A module too large for one crate compiles again in parts.
            Ok(parts) => match options.split_module_bytes {
                Some(limit) if parts[0].len() > limit => {
                    emit(Some(limit / 8)).expect("a module that compiled whole compiles in parts")
                }
                _ => parts,
            },
            Err(found) => {
                errors.extend(found);
                vec![String::new()]
            }
        };
        if let Some(original) = &record.original {
            for error in &mut errors[first_error..] {
                commonjs::remap(error, original);
            }
            let last = parts.last_mut().expect("modules have a part");
            last.push_str(&commonjs::factory(&graph, index));
        }
        module_sources.push(parts);
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let entry = entry_point(&graph);
    let dependencies = native_dependencies.borrow().clone();
    // A module loaded at run time by a name the build could not resolve,
    // such as the API an editor serves its plugins, is loaded with its
    // importer's path as parent; the program resolves the real path of that
    // file, so it names a file when the program runs.
    let host_importers = graph
        .records
        .iter()
        .filter(|record| !record.placeholder)
        .filter_map(|record| {
            let text = record.path.to_string_lossy();
            if let Some((importer, _)) = text.split_once(HOST) {
                return commonjs::application_path(&graph, Path::new(importer));
            }
            let unresolved = late_importers.contains(&record.path)
                || record.requests.iter().any(|(_, _, target)| {
                    target.is_some_and(|target| {
                        graph.records[target]
                            .path
                            .to_string_lossy()
                            .starts_with("\0unresolved:")
                    })
                });
            if unresolved {
                commonjs::application_path(&graph, &record.path)
            } else {
                None
            }
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(ProjectSource {
        module_sources,
        entry_source: entry,
        native_dependencies: dependencies,
        entry_count: graph.entries.len(),
        warnings,
        scripts,
        host_importers,
    })
}

/// Evaluation order: dependencies before dependents, each module once.
/// The modules a module's static imports evaluate before it, in import
/// order, each once. ES import order includes CommonJS leaves; their own
/// requires execute only when the factory reaches the call, never during
/// static traversal.
fn static_dependencies(graph: &Graph, module: usize) -> Vec<usize> {
    let mut dependencies = Vec::new();
    if graph.commonjs[module].is_none() {
        for request in 0..graph.records[module].requests.len() {
            let target = graph.target(module, request);
            if !graph.create_require_requests.contains(&(module, request))
                && !dependencies.contains(&target)
            {
                dependencies.push(target);
            }
        }
    }
    // ES modules that CommonJS code run from here requires are evaluated
    // first, as bundlers hoist them: a factory cannot evaluate an ES module
    // when it runs, and `require` gives the module's namespace.
    let mut required = Vec::new();
    let mut seen = HashSet::new();
    let mut pending: Vec<usize> = core::iter::once(module)
        .chain(dependencies.iter().copied())
        .filter(|candidate| graph.commonjs[*candidate].is_some())
        .collect();
    while let Some(current) = pending.pop() {
        if !seen.insert(current) {
            continue;
        }
        for request in 0..graph.records[current].requests.len() {
            let target = graph.target(current, request);
            if graph.commonjs[target].is_some() {
                pending.push(target);
            } else if required_es_module(graph, current, target)
                && target != module
                && !required.contains(&target)
                && !dependencies.contains(&target)
            {
                required.push(target);
            }
        }
    }
    required.extend(dependencies);
    required
}

/// Whether CommonJS module `from` requires `target` as an ES module, whose
/// namespace its `require` gives.
fn required_es_module(graph: &Graph, from: usize, target: usize) -> bool {
    graph.records[from].original.is_some()
        && graph.commonjs[target].is_none()
        && !graph.records[target].placeholder
}

fn entry_point(graph: &Graph) -> String {
    let mut code = format!(
        "#[allow(dead_code)]\n\
         /// One realm's lazily evaluated application graph and shared module cache.\n\
         pub struct Project {{\n\
         cells: ::alloc::rc::Rc<Vec<binding::Binding>>,\n\
         cache: ::alloc::rc::Rc<Vec<::core::cell::RefCell<Option<Object>>>>,\n\
         states: Vec<::core::cell::Cell<u8>>,\n\
         errors: Vec<::core::cell::RefCell<Option<Thrown>>>,\n\
         }}\n\
         #[allow(dead_code)]\n\
         impl Project {{\n\
         /// Creates shared binding cells, module caches and namespaces without\n\
         /// evaluating any application entry.\n\
         pub fn new(realm: &mut Realm) -> Result<Self, Thrown> {{\n\
         let cells = ::alloc::rc::Rc::new((0..{}).map(|_| binding::new(None)).collect::<Vec<_>>());\n\
         let cache = ::alloc::rc::Rc::new((0..{}).map(|_| ::core::cell::RefCell::new(None)).collect::<Vec<_>>());\n",
        graph.cells,
        graph.records.len()
    );
    // Hoisted `var` cells start undefined: one loop over a static table,
    // not code per cell.
    let mut hoisted = Vec::new();
    for (module, record) in graph.records.iter().enumerate() {
        let mut cells: Vec<_> = record
            .hoisted_vars
            .iter()
            .map(|symbol| graph.locals[module][&Local::Symbol(*symbol)])
            .collect();
        cells.sort_unstable();
        hoisted.extend(cells.into_iter().map(|cell| cell.to_string()));
    }
    if !hoisted.is_empty() {
        let _ = writeln!(
            code,
            "for &cell in &[{}u32] {{ binding::initialize(&cells[cell as usize], Value::Undefined); }}",
            hoisted.join("u32, ")
        );
    }
    for (index, cell) in graph.commonjs.iter().enumerate() {
        if cell.is_some() {
            let _ = writeln!(code, "m{index}(realm, &cells, &cache)?;");
        }
    }
    for (module, namespace) in graph.namespaces.iter().enumerate() {
        // CommonJS namespaces list the exports object's names once evaluated.
        if graph.records[module].named_exports {
            continue;
        }
        let mut names = Vec::new();
        graph.names(module, &mut HashSet::new(), &mut names);
        let mut entries = BTreeMap::new();
        for name in names {
            if let Found::Cell(cell) = graph.resolve(module, &name, &mut Vec::new()) {
                let units: Vec<u16> = name.encode_utf16().collect();
                entries.insert(units, (name, cell));
            }
        }
        let list = entries
            .values()
            .map(|(name, cell)| format!("({name:?}, {cell})"))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            code,
            "binding::initialize(&cells[{namespace}], Value::Object(binding::namespace_of(realm, &cells, &[{list}])));"
        );
    }
    let _ = writeln!(
        code,
        "Ok(Self {{ cells, cache, states: (0..{}).map(|_| ::core::cell::Cell::new(0)).collect(), errors: (0..{}).map(|_| ::core::cell::RefCell::new(None)).collect() }})\n}}\n\
         /// Number of application entries in this graph.\n\
         #[allow(dead_code)]\n\
         pub fn entry_count(&self) -> usize {{ {} }}\n",
        graph.records.len(),
        graph.records.len(),
        graph.entries.len()
    );

    // Evaluation state is project-local: an entry requested a second time
    // returns its existing namespace, shared dependencies run once, and a
    // failed module rethrows the original value on later entry loads. The
    // state is per module behind a shared reference, so an entry a module
    // requires while it evaluates loads inside that evaluation.
    for module in 0..graph.records.len() {
        let path = emit::rust_str(&graph.records[module].path.to_string_lossy());
        let _ = writeln!(
            code,
            "fn evaluate_{module}(&self, realm: &mut Realm) -> Result<(), Thrown> {{\n\
             ::nativeboat_native::platform::trace_module_evaluation({path});\n\
             match self.states[{module}].get() {{\n\
             1 | 2 => return Ok(()),\n\
             3 => return Err(self.errors[{module}].borrow().as_ref().expect(\"failed module retains its error\").clone()),\n\
             _ => {{}}\n\
             }}\n\
             self.states[{module}].set(1);\n\
             let result = (|| -> Result<(), Thrown> {{"
        );
        for dependency in static_dependencies(graph, module) {
            let _ = writeln!(code, "self.evaluate_{dependency}(realm)?;");
        }
        if graph.records[module].named_exports {
            let _ = writeln!(
                code,
                "let exports = require_c{module}(realm, &self.cells, &self.cache)?;"
            );
            for ((target, name), cell) in &graph.named {
                if *target == module && name == INTEROP_DEFAULT {
                    let _ = writeln!(
                        code,
                        "binding::initialize(&self.cells[{cell}], if ops::to_boolean(&ops::get(realm, &exports, &Key::name(\"__esModule\"))?) {{ ops::get(realm, &exports, &Key::name(\"default\"))? }} else {{ exports.clone() }});"
                    );
                } else if *target == module {
                    let _ = writeln!(
                        code,
                        "binding::initialize(&self.cells[{cell}], ops::get(realm, &exports, &Key::name({}))?);",
                        emit::rust_str(name)
                    );
                }
            }
            let _ = writeln!(
                code,
                "binding::initialize(&self.cells[{}], Value::Object(binding::commonjs_namespace(realm, &exports)?));",
                graph.namespaces[module]
            );
        } else if graph.commonjs[module].is_some() {
            let _ = writeln!(code, "require_c{module}(realm, &self.cells, &self.cache)?;");
        } else {
            let _ = writeln!(code, "m{module}(realm, &self.cells, &self.cache)?;");
        }
        code.push_str(
            "Ok(())\n})();\n\
             match result {\n\
             Ok(()) => { self.states[",
        );
        let _ = write!(code, "{module}");
        code.push_str(
            "].set(2); Ok(()) },\n\
             Err(error) => { self.states[",
        );
        let _ = write!(code, "{module}");
        code.push_str("].set(3); *self.errors[");
        let _ = write!(code, "{module}");
        code.push_str(
            "].borrow_mut() = Some(error.clone()); Err(error) }\n\
             }\n\
             }\n",
        );
    }

    code.push_str(
        "/// Evaluates one entry and returns its namespace. Entry ids retain the\n\
         /// order passed to the compiler; entries share all dependency state.\n\
         pub fn entry(&self, realm: &mut Realm, entry: usize) -> Result<Object, Thrown> {\n\
         match entry {\n",
    );
    for (entry, root) in graph.entries.iter().enumerate() {
        let _ = writeln!(code, "{entry} => {{");
        let _ = writeln!(code, "self.evaluate_{root}(realm)?;");
        let _ = writeln!(
            code,
            "Ok(binding::read(realm, &self.cells[{}], \"*namespace*\")?.as_object().cloned().ok_or_else(|| realm.throw(ErrorKind::Type, \"module namespace is not an object\"))?)",
            graph.namespaces[*root]
        );
        code.push_str("}\n");
    }
    code.push_str(
        "_ => Err(realm.throw(ErrorKind::Range, \"Invalid native project entry\")),\n\
         }\n\
         }\n\
         }\n\
         /// Evaluates selected roots in one realm and shares their module cache.\n\
         /// Dynamic imports remain lazy and use the same project instance.\n\
         pub fn module_entries(realm: &mut Realm, entries: &[usize]) -> Result<Vec<Object>, Thrown> {\n",
    );
    // Entries other than the first may load lazily, as the scripts a page or
    // a worker loads by URL do, through the loader.
    if graph.dynamic_imports.is_empty() && graph.entries.len() == 1 {
        code.push_str(
            "let project = Project::new(realm)?;\n\
             let mut modules = Vec::with_capacity(entries.len());\n\
             for entry in entries { modules.push(project.entry(realm, *entry)?); realm.script_checkpoint()?; }\n\
             Ok(modules)\n\
             }\n",
        );
    } else {
        code.push_str(
            "let project = ::alloc::rc::Rc::new(Project::new(realm)?);\n\
             let dynamic_project = project.clone();\n\
             realm.set_dynamic_import_loader(move |realm, entry| {\n\
             dynamic_project.entry(realm, entry).map(Value::Object)\n\
             });\n\
             let mut modules = Vec::with_capacity(entries.len());\n\
             for entry in entries { modules.push(project.entry(realm, *entry)?); realm.script_checkpoint()?; }\n\
             Ok(modules)\n\
             }\n",
        );
    }
    code.push_str(
        "/// Loads the first entry for single-entry compiler callers.\n\
         #[allow(dead_code)]\n\
         pub fn module(realm: &mut Realm) -> Result<Object, Thrown> {\n\
         module_entries(realm, &[0])?.pop().ok_or_else(|| realm.throw(ErrorKind::Range, \"Project has no entry\"))\n\
         }\n",
    );
    code
}
