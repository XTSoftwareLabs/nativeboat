//! Compilation of untyped modules over native dynamic values.
//!
//! Each source function becomes a Rust closure calling the operations of
//! `nativeboat-native`; there is no instruction dispatch and no source text
//! at run time. Bindings that no closure captures stay Rust locals; captured
//! and exported bindings live in shared binding cells. Every construct this
//! pass cannot compile yet is reported at its source location, all of them
//! in one pass, so a project's remaining blockers can be inventoried.

mod emit;
mod link;

use crate::Diagnostic;
use oxc::ast::ast::*;
use oxc::ast_visit::{Visit, walk};
use oxc::semantic::{ScopeFlags, ScopeId, Scoping, SymbolId};
use oxc::syntax::operator::BinaryOperator;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

/// A module graph compiled to Rust over dynamic native values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicModule {
    rust: String,
    warnings: Vec<Diagnostic>,
    module_rust: Vec<Vec<String>>,
    entry_rust: String,
    native_dependencies: BTreeSet<String>,
    entries: usize,
    scripts: Vec<(String, usize)>,
    host_importers: Vec<String>,
}

impl DynamicModule {
    /// Uses that compile but fail when the application runs them, such as
    /// `require` calls whose module is computed then.
    pub fn warnings(&self) -> &[Diagnostic] {
        &self.warnings
    }

    /// Native library namespaces used by imports or intrinsic global reads.
    /// Callers select linked libraries from this metadata, not generated text.
    pub fn native_dependencies(&self) -> &BTreeSet<String> {
        &self.native_dependencies
    }

    /// Rust source defining `pub fn module(realm: &mut Realm) ->
    /// Result<Object, Thrown>`, which links and evaluates the modules and
    /// returns the entry module's namespace. It needs only the
    /// `nativeboat_native` library.
    pub fn rust_source(&self) -> &str {
        &self.rust
    }

    /// Independently generated source for each module in the linked graph,
    /// in parts. Native linkers can place modules in separate crates to bound
    /// rustc's memory use while retaining the shared project-level module
    /// cache. A module larger than `CompileOptions::split_module_bytes` has
    /// several parts, which may go in separate crates too: each defines the
    /// items whose `P<module>_<part>_` names name it and uses those of
    /// earlier parts, and its last part defines the module's functions.
    pub fn module_rust_parts(&self) -> &[Vec<String>] {
        &self.module_rust
    }

    /// Project initializer and module-evaluation state machine for the graph.
    pub fn entry_rust_source(&self) -> &str {
        &self.entry_rust
    }

    /// Number of independent application entries registered in the shared
    /// module graph. Each entry is loaded once by the generated `Project`.
    pub fn entry_count(&self) -> usize {
        self.entries
    }

    /// Package scripts the graph may load while it runs (with
    /// `CompileOptions::package_scripts`): each one's path from its
    /// `node_modules` folder (`node_modules/<package>/<file>`), or for a
    /// worker script the path the program names it by, lowercase with `/`
    /// separators, and the entry that evaluates it.
    pub fn scripts(&self) -> &[(String, usize)] {
        &self.scripts
    }

    /// The application paths (with `/` separators, relative to the folder
    /// the application runs from) of modules that load, when they run, a
    /// module the build could not resolve, such as the API an editor serves
    /// its plugins: the program finds the importer's package by the real
    /// path of its file.
    pub fn host_importers(&self) -> &[String] {
        &self.host_importers
    }
}

/// Compiles one untyped JavaScript or TypeScript module, which may import
/// nothing, to native Rust over dynamic values. Every unsupported construct
/// is reported.
pub fn compile_dynamic(source: &str, path: &Path) -> Result<DynamicModule, Vec<Diagnostic>> {
    compile_project(path, &mut |_, _| None, &mut |_| Some(source.to_owned()))
}

/// Compiles the module graph reachable from `entry`. `resolve` maps an
/// import specifier, from the importing module's path, to a module path;
/// `load` reads a module. Imports become live bindings to the exporting
/// module's cells; modules evaluate once, dependencies first.
pub fn compile_project(
    entry: &Path,
    resolve: link::Resolve<'_>,
    load: link::Load<'_>,
) -> Result<DynamicModule, Vec<Diagnostic>> {
    compile_sources(
        entry,
        &mut |from, name, _| resolve(from, name),
        &mut |path| {
            load(path).map(|text| ModuleSource {
                text,
                kind: ModuleKind::EsModule,
            })
        },
    )
}

/// The source module's semantics, determined by the original project/package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleKind {
    /// A strict ES module with statically linked live exports.
    EsModule,
    /// A CommonJS dependency evaluated by its compiled factory on first require.
    CommonJs,
    /// JSON source compiled into cached native data, never parsed by an app.
    Json,
    /// A module the runtime provides, named by the source text: its exports
    /// object comes from the native runtime, never from source.
    Native,
}

/// Original source text and its module format. Loading never executes source.
#[derive(Debug, Clone)]
pub struct ModuleSource {
    /// Original UTF-8 text; diagnostics retain these byte positions.
    pub text: String,
    /// The format selected by the source's physical package scope.
    pub kind: ModuleKind,
}

/// Which package-export conditions a source request selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    /// An ES import or re-export.
    Import,
    /// A static CommonJS require that loads a compiled factory.
    Require,
    /// A static require.resolve request: return a file path or builtin name,
    /// without loading, compiling or evaluating the resolved module.
    Resolve,
    /// A package script the program may load while it runs, named
    /// `<package>/<file>` from string literals given side by side to a call,
    /// as script loaders name them: the file in the installed package, loaded
    /// as a classic script.
    Script,
    /// A script a worker may run, named by a string literal a module gives
    /// a call or a constructor, as programs name the scripts they start
    /// workers with: `new Worker('worker.js')`, `new URL('./worker.js',
    /// import.meta.url)`, or a path a URL is made from. A relative path names
    /// a file next to the module; another names one in a folder above it.
    Worker,
}

/// A project build-time literal substituted before native code generation.
#[derive(Debug, Clone, PartialEq)]
pub enum CompileValue {
    /// The native realm global object (`globalThis`).
    GlobalThis,
    /// JavaScript `undefined`.
    Undefined,
    /// JavaScript `null`.
    Null,
    /// A boolean literal.
    Boolean(bool),
    /// A number literal.
    Number(f64),
    /// A string literal.
    String(String),
}

/// Statically known values supplied by the project's build configuration.
#[derive(Debug, Clone, Default)]
pub struct CompileOptions {
    /// Bare identifiers replaced by statically configured define literals.
    pub defines: BTreeMap<String, CompileValue>,
    /// `import.meta.env` values; `None` means this module format does not
    /// provide Vite's environment object.
    pub import_meta_env: Option<BTreeMap<String, CompileValue>>,
    /// How JSX in `.jsx` and `.tsx` modules compiles.
    pub jsx: Jsx,
    /// TypeScript's `verbatimModuleSyntax`: imports in TypeScript modules
    /// stay as written. Without it, imports that no value uses go, as
    /// TypeScript and esbuild drop them.
    pub verbatim_module_syntax: bool,
    /// TypeScript's `experimentalDecorators`: in TypeScript modules,
    /// decorators of class members and constructor parameters apply as `tsc`
    /// applies them, after the class is defined.
    pub legacy_decorators: bool,
    /// TypeScript's `useDefineForClassFields: false`: in TypeScript modules,
    /// a field without an initializer only declares a type, initializers
    /// assign (`[[Set]]`) rather than define, and constructor parameter
    /// properties are assigned before them.
    pub assign_class_fields: bool,
    /// The CommonJS bundle a bundler builds this process's ES sources into,
    /// such as electron-vite's `dist_electron/main/index.js`, relative to
    /// the application. The project's ES modules then have a `require` of
    /// their own, and their `__dirname` and `__filename` are the bundle's.
    pub bundle: Option<String>,
    /// The application's folder. Without a bundle, its own CommonJS modules
    /// (not installed packages) have the `__filename` and `__dirname` of
    /// their place in the folder the application runs from, as in a
    /// packaged Electron application, rather than the build's paths.
    pub application: Option<std::path::PathBuf>,
    /// The size of Rust above which a module compiles in parts of an eighth
    /// of it, as `DynamicModule::module_rust_parts` gives them; `None` keeps
    /// every module whole.
    pub split_module_bytes: Option<usize>,
    /// Whether the graph also compiles the package scripts it may load while
    /// it runs (see `RequestKind::Script`), as pages load them with script
    /// elements and workers with computed `import()`s, and, when it
    /// constructs workers, the scripts they may run (see
    /// `RequestKind::Worker`).
    pub package_scripts: bool,
    /// Source folders whose modules the application runs from the files its
    /// own build emits from them, with the app-relative folder each emits
    /// to: plugin packages' `rootDir` and `outDir`. Their modules'
    /// `__filename` and `__dirname`, and the parents of the requires they
    /// compute, are those emitted paths in the folder the application runs
    /// from.
    pub emitted_folders: Vec<(std::path::PathBuf, String)>,
    /// Source folders of TypeScript a package's own build compiles to
    /// CommonJS, as its `tsconfig.json` sets `module`: their modules have a
    /// `require` of their own, and a default import of a CommonJS module is
    /// its `default` when it sets `__esModule`, as `tsc`'s interop reads it.
    pub commonjs_folders: Vec<std::path::PathBuf>,
}

/// What JSX elements become.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Jsx {
    /// Calls to `jsx`, `jsxs` and `Fragment` from `<import_source>/jsx-runtime`,
    /// as TypeScript's `react-jsx` and Vite's React plugin compile them.
    Automatic { import_source: String },
    /// Calls to a factory, such as `React.createElement`, as TypeScript's
    /// `react` compiles them.
    Classic { factory: String, fragment: String },
}

impl Default for Jsx {
    fn default() -> Self {
        Self::Automatic {
            import_source: "react".to_owned(),
        }
    }
}

/// Compiles a source graph with explicit module formats and request kinds.
/// CommonJS is compiled, never parsed or loaded at application run time.
pub fn compile_sources(
    entry: &Path,
    resolve: link::ResolveSource<'_>,
    load: link::LoadSource<'_>,
) -> Result<DynamicModule, Vec<Diagnostic>> {
    compile_sources_with_options(entry, resolve, load, &CompileOptions::default())
}

/// Compiles a source graph with project build-time literals and environment
/// values. Unsupported or dynamic source behavior remains diagnostic.
pub fn compile_sources_with_options(
    entry: &Path,
    resolve: link::ResolveSource<'_>,
    load: link::LoadSource<'_>,
    options: &CompileOptions,
) -> Result<DynamicModule, Vec<Diagnostic>> {
    compile_sources_with_entries_and_options(std::slice::from_ref(&entry), resolve, load, options)
}

/// Compiles multiple application entries into one shared module graph.
/// Entries retain the supplied order and share dependency evaluation, live
/// export cells and the CommonJS cache.
pub fn compile_sources_with_entries(
    entries: &[&Path],
    resolve: link::ResolveSource<'_>,
    load: link::LoadSource<'_>,
) -> Result<DynamicModule, Vec<Diagnostic>> {
    compile_sources_with_entries_and_options(entries, resolve, load, &CompileOptions::default())
}

/// Compiles multiple application entries with build-time literals and one
/// shared module graph/cache.
pub(crate) const PRELUDE: &str = emit::PRELUDE;

pub fn compile_sources_with_entries_and_options(
    entries: &[&Path],
    resolve: link::ResolveSource<'_>,
    load: link::LoadSource<'_>,
    options: &CompileOptions,
) -> Result<DynamicModule, Vec<Diagnostic>> {
    link::project(entries, resolve, load, options).map(|linked| {
        let rust = format!(
            "// Generated native application behavior.\n{}{modules}{entry}",
            emit::PRELUDE,
            modules = linked.module_sources.concat().concat(),
            entry = linked.entry_source,
        );
        DynamicModule {
            native_dependencies: linked.native_dependencies,
            entries: linked.entry_count,
            rust,
            module_rust: linked.module_sources,
            entry_rust: linked.entry_source,
            warnings: linked.warnings,
            scripts: linked.scripts,
            host_importers: linked.host_importers,
        }
    })
}

/// What code generation needs to know about bindings and `this`.
#[derive(Debug, Default)]
pub(crate) struct Analysis {
    /// Bindings that need cells: captured, exported, or initialized along
    /// runtime-dependent paths (patterns, parameter defaults and cases).
    pub captured: HashSet<SymbolId>,
    /// For each function scope, the captured bindings declared outside it
    /// that it or its nested functions use. Its closure clones these cells.
    pub free: HashMap<ScopeId, BTreeSet<SymbolId>>,
    /// For each function scope, the non-arrow functions whose `this` (and
    /// with it `super` and `new.target`) it reaches through arrow functions.
    /// Class field initializers count as functions scoped to their class.
    pub this: HashMap<ScopeId, BTreeSet<ScopeId>>,
    /// Functions that use their own implicit arguments binding.
    pub owns_arguments: HashSet<ScopeId>,
    /// Arrow scopes that must capture an enclosing arguments binding.
    pub arguments: HashMap<ScopeId, BTreeSet<ScopeId>>,
    /// Named function expressions whose bodies use their own name, by
    /// function scope: the name is a cell holding the function itself.
    pub own_names: HashMap<ScopeId, SymbolId>,
    /// Functions and arrow functions whose text the program reads, by the
    /// start of their span: `f.toString()`, `${f}`, `String(f)` and `'' + f`
    /// of the function or of a binding it initializes. They keep their
    /// source text for `Function.prototype.toString`.
    pub stringified: HashSet<u32>,
}

struct Collector<'s> {
    scoping: &'s Scoping,
    analysis: Analysis,
    /// Enclosing function scopes, innermost last, with whether each is an arrow.
    functions: Vec<(ScopeId, bool)>,
    /// Enclosing class scopes, innermost last.
    classes: Vec<ScopeId>,
    /// Bindings whose value's text the program reads.
    stringified_bindings: HashSet<SymbolId>,
    /// The functions that function declarations and `const f = function`
    /// or arrow initializers bind, by the start of their span.
    function_bindings: HashMap<SymbolId, u32>,
}

impl Analysis {
    fn of(program: &Program<'_>, scoping: &Scoping) -> Analysis {
        let mut collector = Collector {
            scoping,
            analysis: Analysis::default(),
            functions: vec![(scoping.root_scope_id(), false)],
            classes: Vec::new(),
            stringified_bindings: HashSet::new(),
            function_bindings: HashMap::new(),
        };
        collector.visit_program(program);
        let mut analysis = collector.analysis;
        analysis.stringified.extend(
            collector
                .stringified_bindings
                .iter()
                .filter_map(|symbol| collector.function_bindings.get(symbol)),
        );
        analysis
    }
}

/// The function, static block or module scope that owns a scope's bindings.
pub(crate) fn function_scope(scoping: &Scoping, scope: ScopeId) -> ScopeId {
    scoping
        .scope_ancestors(scope)
        .find(|scope| scoping.scope_flags(*scope).is_var())
        .expect("every scope is inside the module scope")
}

/// Whether every declaration of a symbol is ambient (`declare function`,
/// `declare const`, `declare class`, ...). Such a name has no binding in the
/// emitted program: it describes a global the host provides, so references to
/// it read the global at run time.
pub(crate) fn ambient_only(scoping: &Scoping, symbol: SymbolId) -> bool {
    let redeclarations = scoping.symbol_redeclarations(symbol);
    if redeclarations.is_empty() {
        scoping.symbol_flags(symbol).is_ambient()
    } else {
        redeclarations
            .iter()
            .all(|redeclaration| redeclaration.flags.is_ambient())
    }
}

impl Collector<'_> {
    /// These bindings initialize along runtime-dependent paths, so their
    /// temporal dead zone needs a cell rather than emission-order tracking.
    fn lexical_cells(&mut self, scope: ScopeId) {
        self.analysis
            .captured
            .extend(
                self.scoping
                    .get_bindings(scope)
                    .values()
                    .copied()
                    .filter(|symbol| {
                        let flags = self.scoping.symbol_flags(*symbol);
                        flags.is_block_scoped() && !flags.is_function()
                    }),
            );
    }

    /// Notes that the program reads the text of `expression`'s value.
    fn stringified(&mut self, expression: &Expression<'_>) {
        match expression.get_inner_expression() {
            Expression::FunctionExpression(function) => {
                self.analysis.stringified.insert(function.span.start);
            }
            Expression::ArrowFunctionExpression(arrow) => {
                self.analysis.stringified.insert(arrow.span.start);
            }
            Expression::Identifier(identifier) => {
                if let Some(symbol) = self
                    .scoping
                    .get_reference(identifier.reference_id())
                    .symbol_id()
                {
                    self.stringified_bindings.insert(symbol);
                }
            }
            _ => {}
        }
    }

    fn enter(&mut self, scope: ScopeId, arrow: bool) {
        self.functions.push((scope, arrow));
    }

    fn reference(&mut self, symbol: SymbolId) {
        // Types (type parameters, interfaces, aliases) are erased: reading
        // one from a nested function's annotations captures nothing.
        if !self.scoping.symbol_flags(symbol).is_value() || ambient_only(self.scoping, symbol) {
            return;
        }
        let owner = function_scope(self.scoping, self.scoping.symbol_scope_id(symbol));
        for (function, _) in self.functions.iter().rev() {
            if *function == owner {
                break;
            }
            self.analysis.captured.insert(symbol);
            self.analysis
                .free
                .entry(*function)
                .or_default()
                .insert(symbol);
        }
    }
}

impl<'a> Visit<'a> for Collector<'_> {
    fn visit_binding_pattern(&mut self, pattern: &BindingPattern<'a>) {
        if !matches!(pattern, BindingPattern::BindingIdentifier(_)) {
            self.analysis.captured.extend(
                pattern
                    .get_binding_identifiers()
                    .iter()
                    .map(|binding| binding.symbol_id()),
            );
        }
        walk::walk_binding_pattern(self, pattern);
    }

    fn visit_formal_parameters(&mut self, params: &FormalParameters<'a>) {
        // Defaults may read themselves or parameters initialized later.
        if params.items.iter().any(|parameter| {
            parameter.initializer.is_some()
                || !matches!(parameter.pattern, BindingPattern::BindingIdentifier(_))
        }) {
            self.analysis
                .captured
                .extend(params.items.iter().flat_map(|parameter| {
                    parameter
                        .pattern
                        .get_binding_identifiers()
                        .into_iter()
                        .map(|binding| binding.symbol_id())
                }));
            if let Some(rest) = &params.rest {
                self.analysis.captured.extend(
                    rest.rest
                        .argument
                        .get_binding_identifiers()
                        .iter()
                        .map(|binding| binding.symbol_id()),
                );
            }
        }
        walk::walk_formal_parameters(self, params);
    }

    fn visit_switch_statement(&mut self, statement: &SwitchStatement<'a>) {
        self.lexical_cells(statement.scope_id());
        walk::walk_switch_statement(self, statement);
    }

    fn visit_for_in_statement(&mut self, statement: &ForInStatement<'a>) {
        self.lexical_cells(statement.scope_id());
        walk::walk_for_in_statement(self, statement);
    }

    fn visit_for_of_statement(&mut self, statement: &ForOfStatement<'a>) {
        self.lexical_cells(statement.scope_id());
        walk::walk_for_of_statement(self, statement);
    }

    fn visit_function(&mut self, function: &Function<'a>, flags: ScopeFlags) {
        // Overload signatures have no body and no text of their own.
        if function.r#type == FunctionType::FunctionDeclaration
            && function.body.is_some()
            && let Some(id) = &function.id
        {
            self.function_bindings
                .insert(id.symbol_id(), function.span.start);
        }
        if function.r#type == FunctionType::FunctionExpression
            && let Some(id) = &function.id
            && !self
                .scoping
                .get_resolved_reference_ids(id.symbol_id())
                .is_empty()
        {
            self.analysis.captured.insert(id.symbol_id());
            self.analysis
                .own_names
                .insert(function.scope_id(), id.symbol_id());
        }
        self.enter(function.scope_id(), false);
        walk::walk_function(self, function, flags);
        self.functions.pop();
    }

    fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        self.enter(arrow.scope_id(), true);
        walk::walk_arrow_function_expression(self, arrow);
        self.functions.pop();
    }

    fn visit_identifier_reference(&mut self, identifier: &IdentifierReference<'a>) {
        if let Some(symbol) = self
            .scoping
            .get_reference(identifier.reference_id())
            .symbol_id()
        {
            self.reference(symbol);
        } else if identifier.name == "arguments"
            && let Some((owner, _)) = self
                .functions
                .iter()
                .rev()
                .find(|(scope, arrow)| !arrow && self.scoping.scope_flags(*scope).is_function())
        {
            self.analysis.owns_arguments.insert(*owner);
            for (function, _) in self.functions.iter().rev() {
                if function == owner {
                    break;
                }
                self.analysis
                    .arguments
                    .entry(*function)
                    .or_default()
                    .insert(*owner);
            }
        }
    }

    fn visit_variable_declarator(&mut self, declarator: &VariableDeclarator<'a>) {
        if let BindingPattern::BindingIdentifier(id) = &declarator.id
            && let Some(init) = &declarator.init
        {
            match init.get_inner_expression() {
                Expression::FunctionExpression(function) => {
                    self.function_bindings
                        .insert(id.symbol_id(), function.span.start);
                }
                Expression::ArrowFunctionExpression(arrow) => {
                    self.function_bindings
                        .insert(id.symbol_id(), arrow.span.start);
                }
                _ => {}
            }
        }
        walk::walk_variable_declarator(self, declarator);
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        match call.callee.get_inner_expression() {
            // `f.toString()`
            Expression::StaticMemberExpression(member)
                if member.property.name == "toString" && call.arguments.is_empty() =>
            {
                self.stringified(&member.object);
            }
            // `String(f)` of the global `String`
            Expression::Identifier(callee)
                if callee.name == "String"
                    && self
                        .scoping
                        .get_reference(callee.reference_id())
                        .symbol_id()
                        .is_none() =>
            {
                if let Some(argument) = call.arguments.first().and_then(Argument::as_expression) {
                    self.stringified(argument);
                }
            }
            _ => {}
        }
        walk::walk_call_expression(self, call);
    }

    fn visit_template_literal(&mut self, template: &TemplateLiteral<'a>) {
        for expression in &template.expressions {
            self.stringified(expression);
        }
        walk::walk_template_literal(self, template);
    }

    fn visit_binary_expression(&mut self, binary: &BinaryExpression<'a>) {
        // `'' + f` and `f + ''`
        if binary.operator == BinaryOperator::Addition {
            let text = |expression: &Expression<'_>| {
                matches!(
                    expression.get_inner_expression(),
                    Expression::StringLiteral(_) | Expression::TemplateLiteral(_)
                )
            };
            if text(&binary.left) {
                self.stringified(&binary.right);
            }
            if text(&binary.right) {
                self.stringified(&binary.left);
            }
        }
        walk::walk_binary_expression(self, binary);
    }

    fn visit_this_expression(&mut self, _: &ThisExpression) {
        self.this();
    }

    fn visit_super(&mut self, _: &Super) {
        self.this();
    }

    fn visit_new_target(&mut self, _: &NewTarget) {
        self.this();
    }

    fn visit_class(&mut self, class: &Class<'a>) {
        self.classes.push(class.scope_id());
        walk::walk_class(self, class);
        self.classes.pop();
    }

    /// Field initializers run as functions of their class with their own
    /// `this`; computed keys run in the enclosing function.
    fn visit_property_definition(&mut self, property: &PropertyDefinition<'a>) {
        self.visit_property_key(&property.key);
        if let Some(value) = &property.value {
            let class = *self.classes.last().expect("fields are inside classes");
            self.enter(class, false);
            self.visit_expression(value);
            self.functions.pop();
        }
    }

    fn visit_static_block(&mut self, block: &StaticBlock<'a>) {
        self.enter(block.scope_id(), false);
        walk::walk_static_block(self, block);
        self.functions.pop();
    }
}

impl Collector<'_> {
    /// Records a use of the innermost non-arrow function's `this`.
    fn this(&mut self) {
        let owner = self
            .functions
            .iter()
            .rev()
            .find(|(_, arrow)| !arrow)
            .map(|(scope, _)| *scope)
            .expect("the module scope is never an arrow");
        for (function, _) in self.functions.iter().rev() {
            if *function == owner {
                break;
            }
            self.analysis
                .this
                .entry(*function)
                .or_default()
                .insert(owner);
        }
    }
}

#[cfg(test)]
mod tests;

/// Whether a module awaits outside every function: `await` or `for await` in
/// its body, which then evaluates as an async function.
pub(crate) fn awaits_at_top_level(program: &Program<'_>) -> bool {
    struct Finder {
        functions: usize,
        found: bool,
    }
    impl<'a> Visit<'a> for Finder {
        fn visit_function(&mut self, function: &Function<'a>, flags: ScopeFlags) {
            self.functions += 1;
            walk::walk_function(self, function, flags);
            self.functions -= 1;
        }
        fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
            self.functions += 1;
            walk::walk_arrow_function_expression(self, arrow);
            self.functions -= 1;
        }
        fn visit_await_expression(&mut self, expression: &AwaitExpression<'a>) {
            self.found |= self.functions == 0;
            walk::walk_await_expression(self, expression);
        }
        fn visit_for_of_statement(&mut self, statement: &ForOfStatement<'a>) {
            self.found |= statement.r#await && self.functions == 0;
            walk::walk_for_of_statement(self, statement);
        }
    }
    let mut finder = Finder {
        functions: 0,
        found: false,
    };
    finder.visit_program(program);
    finder.found
}
