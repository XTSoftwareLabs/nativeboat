//! Rust generation for dynamic modules. Expressions become Rust block
//! expressions of type `Value` that evaluate operands into temporaries in
//! source order; fallible operations leave through the current exit
//! (`check!`, or the enclosing `try` region's slot and label). Locals are
//! borrowed in place, instead of copied into temporaries, where no later
//! operand can change them, and branches test Rust booleans.

use super::link::Link;
use super::{Analysis, CompileOptions, CompileValue, ambient_only, function_scope};
use crate::{Diagnostic, Source};
use oxc::ast::ast::*;
use oxc::semantic::{ScopeId, Scoping, SymbolId};
use oxc::span::{GetSpan, Span};
use oxc::syntax::operator::{BinaryOperator, LogicalOperator, UnaryOperator, UpdateOperator};
use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::path::Path;

mod class;
mod typescript;
pub(super) use typescript::instantiated;
mod loops;
mod patterns;

use patterns::Mode;

/// Generated code links no Rust `std`: collections and macros come from
/// `alloc`, which the program root declares with `extern crate alloc`.
pub(super) const PRELUDE: &str = "#[allow(unused_imports)]\n\
use ::nativeboat_native::{binding, ops, ErrorKind, Key, Object, Realm, Thrown, Value};\n\
#[allow(unused_imports)]\n\
use ::nativeboat_native::ops::{Arithmetic, Relation};\n\
#[allow(unused_imports)]\n\
use ::nativeboat_native::{check, string::{LookupCache, StaticStr}};\n\
#[allow(unused_imports)]\n\
use ::alloc::{vec, vec::Vec};\n";

/// How a fallible operation leaves the current region.
#[derive(Clone)]
enum Exit {
    /// Propagate from the Rust function with `?`.
    Return,
    /// Store the thrown value in `slot` and leave the labeled `try` block.
    Attempt { slot: String, label: String },
}

/// A loop that `break` and `continue` can target.
struct Loop {
    names: Vec<String>,
    exit: String,
    next: Option<String>,
    /// How many `finally` regions enclose the loop.
    guards: usize,
    /// The iterator a `for`-`of` loop closes when left early.
    iterator: Option<String>,
}

/// A non-loop labeled statement that `break label` can leave.
struct LabelBlock {
    name: String,
    exit: String,
    guards: usize,
    loops: usize,
}

/// The function being generated.
struct Frame {
    scope: ScopeId,
    exits: Vec<Exit>,
    loops: Vec<Loop>,
    /// Labels of the statement being generated, for the loop they name.
    labels: Vec<String>,
    /// Active labels on non-loop statements.
    label_blocks: Vec<LabelBlock>,
    guards: usize,
    /// Uncaptured `let`/`const` bindings whose declarations have run.
    declared: HashSet<SymbolId>,
    /// Whether this is a generator body, whose `catch` clauses pass on the
    /// signal with which `return()` unwinds it.
    generator: bool,
    /// Whether this is an async generator body, whose `yield` awaits.
    async_generator: bool,
    /// The `try` statements with `finally` blocks being generated, innermost
    /// last: jumps out of them run their `finally` block first.
    finally: Vec<Finally>,
}

/// A `try` statement with a `finally` block: `return`, `break` and
/// `continue` leaving its protected region record themselves, leave the
/// region and are performed again after the `finally` block.
struct Finally {
    id: usize,
    /// The labeled block the region leaves: the `try` block or the handler.
    label: String,
    /// How many loops enclose the statement.
    loops: usize,
    /// The jumps out of the region; the slot holds the index plus one.
    jumps: Vec<Pending>,
}

/// A jump waiting for a `finally` block.
#[derive(Clone)]
enum Pending {
    Return,
    Jump {
        span: Span,
        label: Option<String>,
        exit: bool,
    },
}

/// How a function's body runs when it is called.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Callable {
    Normal,
    Generator,
    Async,
    AsyncGenerator,
}

impl Callable {
    fn of(function: &Function<'_>) -> Callable {
        if function.generator && function.r#async {
            Callable::AsyncGenerator
        } else if function.generator {
            Callable::Generator
        } else if function.r#async {
            Callable::Async
        } else {
            Callable::Normal
        }
    }
}

pub(super) struct Emitter<'s> {
    source: &'s Source<'s>,
    scoping: &'s Scoping,
    analysis: &'s Analysis,
    link: &'s Link,
    options: &'s CompileOptions,
    /// Strictness inherited by function bodies in this source module.
    strict: bool,
    errors: Vec<Diagnostic>,
    frames: Vec<Frame>,
    /// Whether the next function body is an async generator's.
    async_body: bool,
    /// The module awaits at top level, so its body runs as an async function.
    module_awaits: bool,
    next: usize,
    /// Private names of the classes being generated, innermost last, with
    /// the Rust variables holding their keys.
    classes: Vec<Vec<(String, String)>>,
    /// Class methods, constructors and initializers with a `home{scope}`
    /// object for `super` property access.
    homes: HashSet<ScopeId>,
    /// Class constructors with a `target{scope}` new target, and whether each
    /// is derived (its `this{scope}` is a `class::FrameRef`).
    constructors: HashMap<ScopeId, bool>,
    /// Ordinary function scopes with invocation-local new target snapshots.
    function_targets: HashSet<ScopeId>,
    /// Formal parameters whose values are also exposed through a sloppy
    /// function's mapped `arguments` object.
    mapped_parameters: HashSet<SymbolId>,
    /// TypeScript constructor parameters that also initialize instance fields.
    constructor_parameter_properties: HashMap<ScopeId, Vec<(String, SymbolId)>>,
    /// TypeScript enums and namespaces whose object exists, which a later
    /// declaration of the name merges into.
    typescript_objects: HashSet<SymbolId>,
    /// Constructors that initialize their class's fields after assigning
    /// their parameter properties, with the Rust expression of the fields'
    /// initializer (TypeScript's `useDefineForClassFields: false`).
    deferred_fields: HashMap<ScopeId, String>,
    /// Whether the module is TypeScript, whose class options apply to it.
    typescript: bool,
    /// The names of the module's static strings, by their units: string
    /// literals and property names, which then cost no allocation where they
    /// are used.
    strings: HashMap<Vec<u16>, String>,
    /// The Rust items defining the current part's static strings and read
    /// caches.
    statics: String,
    /// How many property reads have a `LookupCache` of their own.
    lookups: usize,
    /// For a module split into parts, the size of Rust at which a part ends.
    /// Its function bodies then become Rust functions of their own, which
    /// take what they capture by reference, so that native linkers can
    /// compile its parts in separate crates.
    part_bytes: Option<usize>,
    /// The finished parts of a split module.
    parts: Vec<String>,
    /// The functions of the current part.
    functions: String,
    /// How many function bodies became Rust functions.
    outlined: usize,
}

/// The Rust source of a module: one part, or with `part_bytes`, parts of
/// about that size, each defining the items its `P<module>_<part>_` names
/// name and using those of earlier parts. The last part defines the module's
/// function.
pub(super) fn module(
    program: &Program<'_>,
    scoping: &Scoping,
    analysis: &Analysis,
    source: &Source<'_>,
    link: &Link,
    options: &CompileOptions,
    part_bytes: Option<usize>,
) -> Result<Vec<String>, Vec<Diagnostic>> {
    let mut emitter = Emitter {
        source,
        scoping,
        analysis,
        link,
        options,
        strict: link.strict,
        errors: Vec::new(),
        frames: Vec::new(),
        async_body: false,
        module_awaits: super::awaits_at_top_level(program),
        next: 0,
        classes: Vec::new(),
        homes: HashSet::new(),
        constructors: HashMap::new(),
        function_targets: HashSet::new(),
        mapped_parameters: HashSet::new(),
        constructor_parameter_properties: HashMap::new(),
        typescript_objects: HashSet::new(),
        deferred_fields: HashMap::new(),
        typescript: program.source_type.is_typescript(),
        strings: HashMap::new(),
        statics: String::new(),
        lookups: 0,
        part_bytes,
        parts: Vec::new(),
        functions: String::new(),
        outlined: 0,
    };
    let root = scoping.root_scope_id();
    emitter.frames.push(frame(root));
    let mut body = String::new();
    if let Some(relative) = emitter.application_relative(source.path) {
        let directory = relative.rsplit_once('/').map_or("", |(folder, _)| folder);
        let url = emitter.fail(&format!(
            "::nativeboat_native::process_builtin::app_relative_url(realm, {relative:?})"
        ));
        let filename = emitter.fail(&format!(
            "::nativeboat_native::process_builtin::app_relative(realm, {relative:?})"
        ));
        let dirname = emitter.fail(&format!(
            "::nativeboat_native::process_builtin::app_relative(realm, {directory:?})"
        ));
        let _ = writeln!(
            body,
            "let __import_meta = realm.object(); __import_meta.define(Key::name(\"url\"), ::nativeboat_native::Property::data({url})); __import_meta.define(Key::name(\"filename\"), ::nativeboat_native::Property::data({filename})); __import_meta.define(Key::name(\"dirname\"), ::nativeboat_native::Property::data({dirname}));"
        );
    } else {
        write_build_import_meta(&mut body, source.path);
    }
    if let Some(environment) = &options.import_meta_env {
        body.push_str("let __import_meta_env = realm.object();\n");
        for (name, value) in environment {
            let _ = writeln!(
                body,
                "__import_meta_env.define(Key::name({}), ::nativeboat_native::Property::data({}));",
                rust_str(name),
                Emitter::compile_value(value)
            );
        }
        body.push_str("__import_meta.define(Key::name(\"env\"), ::nativeboat_native::Property::data(Value::Object(__import_meta_env)));\n");
    }
    let _ = writeln!(body, "let this{} = Value::Undefined;", root.index());
    let mut statements = Vec::new();
    let mut anonymous = None;
    for statement in &program.body {
        match statement {
            // Imports and re-exports are linked to the exporter's cells.
            Statement::ImportDeclaration(_)
            | Statement::ExportNamedDeclaration(_)
            | Statement::ExportFromDeclaration(_)
            | Statement::ExportAllDeclaration(_) => {}
            Statement::ExportDeclaration(export) => match &export.declaration {
                declaration @ (Declaration::VariableDeclaration(_)
                | Declaration::FunctionDeclaration(_)
                | Declaration::ClassDeclaration(_)
                | Declaration::TSEnumDeclaration(_)
                | Declaration::TSNamespaceDeclaration(_)
                | Declaration::TSImportEqualsDeclaration(_)) => {
                    statements.push(Owned::Declaration(declaration));
                }
                Declaration::TSTypeAliasDeclaration(_) | Declaration::TSInterfaceDeclaration(_) => {
                }
                declaration => emitter.unsupported(
                    declaration.span(),
                    "this exported declaration is not compiled yet",
                ),
            },
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                ExportDefaultDeclarationKind::FunctionDeclaration(function)
                    if function.id.is_some() =>
                {
                    statements.push(Owned::Function(function));
                }
                ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                    anonymous = Some(&**function);
                }
                ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                    statements.push(Owned::Class(class));
                }
                ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => {}
                kind => statements.push(Owned::Default(kind.to_expression())),
            },
            other => statements.push(Owned::Statement(other)),
        }
    }
    let scope_code = emitter.scope_bindings(root, &[]);
    body.push_str(&scope_code);
    let hoisted = emitter.hoist(statements.iter().filter_map(|owned| match owned {
        Owned::Statement(Statement::FunctionDeclaration(function)) => Some(&**function),
        Owned::Function(function) => Some(*function),
        Owned::Declaration(Declaration::FunctionDeclaration(function)) => Some(&**function),
        _ => None,
    }));
    body.push_str(&hoisted);
    let default = link.default.map(|cell| format!("cells[{cell}]"));
    if let Some(function) = anonymous {
        let value = emitter.named_function(function, "default");
        let cell = default.as_deref().expect("default exports have a cell");
        let _ = writeln!(body, "binding::initialize(&{cell}, {value});");
    }
    for owned in &statements {
        let code = match owned {
            Owned::Statement(statement) => emitter.statement(statement),
            Owned::Declaration(Declaration::VariableDeclaration(declaration)) => {
                emitter.variables(declaration)
            }
            Owned::Declaration(Declaration::ClassDeclaration(class)) => {
                emitter.class_declaration(class)
            }
            Owned::Declaration(Declaration::TSEnumDeclaration(declaration)) => {
                emitter.enum_declaration(declaration)
            }
            Owned::Declaration(Declaration::TSNamespaceDeclaration(declaration)) => {
                emitter.namespace(declaration)
            }
            Owned::Declaration(Declaration::TSImportEqualsDeclaration(declaration)) => {
                emitter.import_equals(declaration, true)
            }
            Owned::Class(class) if class.id.is_some() => emitter.class_declaration(class),
            Owned::Class(class) => {
                let value = emitter.class(class, "default");
                let cell = default.as_deref().expect("default exports have a cell");
                format!("binding::initialize(&{cell}, {value});\n")
            }
            Owned::Default(expression) => {
                let value = emitter.expression(expression);
                let cell = default.as_deref().expect("default exports have a cell");
                format!("binding::initialize(&{cell}, {value});\n")
            }
            Owned::Declaration(_) | Owned::Function(_) => String::new(),
        };
        body.push_str(&code);
    }
    if !emitter.errors.is_empty() {
        return Err(emitter.errors);
    }
    if emitter.module_awaits {
        body = async_module_body(&body);
    }
    let mut last = emitter.functions;
    let _ = write!(
        last,
        "\n/// `{}`\n\
         #[allow(unused_mut, unused_variables, unused_assignments, unused_labels, unreachable_code, unused_braces, unused_parens, clippy::all)]\n\
         fn m{}(realm: &mut Realm, cells: &::alloc::rc::Rc<Vec<binding::Binding>>, cache: &::alloc::rc::Rc<Vec<::core::cell::RefCell<Option<Object>>>>) -> Result<(), Thrown> {{\n{body}Ok(())\n}}\n{}",
        source.path.display(),
        link.index,
        emitter.statics
    );
    let mut parts = emitter.parts;
    parts.push(last);
    Ok(parts)
}

/// A module body that awaits at top level, as an async function the module
/// starts: it runs until its first `await`, and the realm records its promise,
/// which a dynamic import of the module waits for. Its error rejects that
/// promise, which reports it as an unhandled rejection does.
fn async_module_body(body: &str) -> String {
    format!(
        "let __module_cells = cells.clone();\n\
         let __module_cache = cache.clone();\n\
         let __module_body = ::nativeboat_native::coroutine::async_function(realm, \"\", 0, move |realm: &mut Realm, this: Value, args: &[Value]| -> Result<::nativeboat_native::coroutine::Body, Thrown> {{\n\
         let cells = __module_cells.clone();\n\
         let cache = __module_cache.clone();\n\
         Ok(::alloc::boxed::Box::new(move |realm: &mut Realm| -> ::nativeboat_native::Completion {{\n\
         let cells = &cells;\n\
         let cache = &cache;\n\
         {body}Ok(Value::Undefined)\n\
         }}))\n\
         }});\n\
         let __evaluation = check!(::nativeboat_native::ops::call(realm, &Value::Object(__module_body), Value::Undefined, &[]));\n\
         realm.async_evaluation(__evaluation);\n"
    )
}

enum Owned<'r, 'a> {
    Statement(&'r Statement<'a>),
    Declaration(&'r Declaration<'a>),
    /// `export default function name() {}`, hoisted like a declaration.
    Function(&'r Function<'a>),
    /// `export default <expression>`.
    Default(&'r Expression<'a>),
    /// `export default class`, named or anonymous.
    Class(&'r Class<'a>),
}

fn frame(scope: ScopeId) -> Frame {
    Frame {
        scope,
        exits: vec![Exit::Return],
        loops: Vec::new(),
        labels: Vec::new(),
        label_blocks: Vec::new(),
        guards: 0,
        declared: HashSet::new(),
        generator: false,
        async_generator: false,
        finally: Vec::new(),
    }
}

/// How V8 names a callee in `... is not a function`: identifiers, `this`,
/// member chains, literal keys and calls as written, else
/// `(intermediate value)`.
pub(super) fn call_site_text(callee: &Expression<'_>) -> String {
    match callee {
        Expression::Identifier(identifier) => identifier.name.to_string(),
        Expression::ThisExpression(_) => "this".to_owned(),
        Expression::Super(_) => "super".to_owned(),
        Expression::StaticMemberExpression(member) => format!(
            "{}{}{}",
            call_site_text(&member.object),
            if member.optional { "?." } else { "." },
            member.property.name
        ),
        Expression::PrivateFieldExpression(field) => {
            format!("{}[#{}]", call_site_text(&field.object), field.field.name)
        }
        Expression::ComputedMemberExpression(member) => {
            let key = match &member.expression {
                Expression::StringLiteral(text)
                    if text.value.chars().next().is_some_and(|first| {
                        first.is_alphabetic() || first == '_' || first == '$'
                    }) && text.value.chars().all(|character| {
                        character.is_alphanumeric() || character == '_' || character == '$'
                    }) =>
                {
                    let object = call_site_text(&member.object);
                    return format!("{object}.{}", text.value);
                }
                Expression::StringLiteral(text) => format!("\"{}\"", text.value),
                Expression::NumericLiteral(number) => number.value.to_string(),
                Expression::Identifier(identifier) => identifier.name.to_string(),
                _ => "...".to_owned(),
            };
            format!("{}[{key}]", call_site_text(&member.object))
        }
        Expression::CallExpression(call) => format!("{}(...)", call_site_text(&call.callee)),
        Expression::ParenthesizedExpression(inner) => call_site_text(&inner.expression),
        Expression::ChainExpression(_) => "(intermediate value)".to_owned(),
        _ => "(intermediate value)".to_owned(),
    }
}

/// A Rust string literal for Rust text.
pub(super) fn rust_str(text: &str) -> String {
    format!("{text:?}")
}

/// The JavaScript text of a function whose source is `text`, as
/// `Function.prototype.toString` gives it: TypeScript's types, `this`
/// parameters and other type-only syntax are removed, as `tsc` emits it.
/// `None` when the text does not parse on its own.
pub(super) fn function_source_text(text: &str, typescript: bool) -> Option<String> {
    use oxc::allocator::Allocator;
    use oxc::codegen::Codegen;
    use oxc::parser::Parser;
    use oxc::semantic::SemanticBuilder;
    use oxc::span::SourceType;
    use oxc::transformer::{TransformOptions, Transformer};
    if !typescript {
        return Some(text.to_owned());
    }
    // The function is an expression of a program of its own.
    let wrapped = format!("({text});");
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, &wrapped, SourceType::ts()).parse();
    if parsed.panicked || parsed.diagnostics.has_errors() {
        return None;
    }
    let mut program = parsed.program;
    let scoping = SemanticBuilder::new()
        .build(&program)
        .semantic
        .into_scoping();
    let transformed = Transformer::new(
        &allocator,
        Path::new("function.ts"),
        &TransformOptions::default(),
    )
    .build_with_scoping(scoping, &mut program);
    if transformed.diagnostics.has_errors() {
        return None;
    }
    let code = Codegen::new().build(&program).code;
    let code = code.trim_end();
    let code = code.strip_suffix(';').unwrap_or(code);
    let inner = code.strip_prefix('(')?.strip_suffix(')')?;
    Some(inner.to_owned())
}

/// A `Value::String` built from UTF-16 code units: a Rust string literal when
/// the units are well-formed UTF-16, else the units themselves.
pub(super) fn units_value(units: &[u16]) -> String {
    if let Ok(text) = String::from_utf16(units) {
        return format!("Value::str({})", rust_str(&text));
    }
    let list = units
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    format!("Value::String(::nativeboat_native::JsStr::from_units(&[{list}]))")
}

/// Physical Windows paths retain their original UTF-16, including surrogates.
pub(super) fn path_value(path: &std::path::Path) -> String {
    #[cfg(windows)]
    let units = {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str().encode_wide().collect::<Vec<_>>()
    };
    #[cfg(not(windows))]
    let units = path.to_string_lossy().encode_utf16().collect::<Vec<_>>();
    units_value(&units)
}

/// `import.meta` with the build's own paths, for modules outside an
/// application folder: the URL, and Node's `filename` and `dirname`.
fn write_build_import_meta(body: &mut String, path: &Path) {
    let _ = writeln!(
        body,
        "let __import_meta = realm.object(); __import_meta.define(Key::name(\"url\"), ::nativeboat_native::Property::data(Value::str({})));",
        rust_str(&import_meta_url(path))
    );
    let filename = std::path::absolute(path)
        .expect("source paths have a current directory")
        .to_string_lossy()
        .replace('/', "\\");
    let dirname = filename
        .rsplit_once('\\')
        .map_or(filename.as_str(), |(folder, _)| folder);
    let dirname = if dirname.ends_with(':') {
        format!("{dirname}\\")
    } else {
        dirname.to_owned()
    };
    let _ = writeln!(
        body,
        "__import_meta.define(Key::name(\"filename\"), ::nativeboat_native::Property::data(Value::str({}))); __import_meta.define(Key::name(\"dirname\"), ::nativeboat_native::Property::data(Value::str({})));",
        rust_str(&filename),
        rust_str(&dirname)
    );
}

/// The WHATWG file URL for a source module, computed while compiling it.
fn import_meta_url(path: &std::path::Path) -> String {
    let absolute = std::path::absolute(path).expect("source paths have a current directory");
    let path = absolute.to_string_lossy().replace('\\', "/");
    let unc = cfg!(windows) && path.starts_with("//");
    #[cfg(windows)]
    let prefix = if unc { "file:" } else { "file:///" };
    #[cfg(not(windows))]
    let prefix = "file://";
    let mut url = String::from(prefix);
    for (index, byte) in path.bytes().enumerate() {
        let drive_colon = cfg!(windows) && index == 1 && byte == b':';
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/')
            || drive_colon
        {
            url.push(char::from(byte));
        } else {
            let _ = write!(url, "%{byte:02X}");
        }
    }
    url
}

/// The UTF-16 code units of a parsed string literal value. The parser marks
/// lone surrogates as U+FFFD followed by four hex digits.
fn literal_units(value: &str, lone_surrogates: bool) -> Vec<u16> {
    if !lone_surrogates {
        return value.encode_utf16().collect();
    }
    let mut units = Vec::new();
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        if character == '\u{FFFD}' {
            let hex: String = chars.by_ref().take(4).collect();
            units.push(u16::from_str_radix(&hex, 16).expect("the parser encodes four hex digits"));
        } else {
            let mut buffer = [0; 2];
            units.extend_from_slice(character.encode_utf16(&mut buffer));
        }
    }
    units
}

/// The AST variant name of a node, such as `SwitchStatement`, read from its
/// debug form without formatting the whole subtree.
fn variant(node: &impl std::fmt::Debug) -> String {
    struct Name(String);
    impl Write for Name {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            match text.find(['(', ' ', '{']) {
                Some(end) => {
                    self.0.push_str(&text[..end]);
                    Err(std::fmt::Error)
                }
                None => {
                    self.0.push_str(text);
                    Ok(())
                }
            }
        }
    }
    let mut name = Name(String::new());
    let _ = write!(name, "{node:?}");
    name.0
}

/// The local a Rust expression copies, when it is exactly a copy of one:
/// `b3.clone()` reads `b3`, which an operation can borrow instead.
fn borrowed(code: &str) -> Option<&str> {
    let local = code.strip_suffix(".clone()")?;
    let mut characters = local.chars();
    (characters.next()?.is_ascii_lowercase()
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_'))
    .then_some(local)
}

/// Whether an expression is a literal, whose Rust code has no effects and
/// does not use the realm.
fn literal(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::NumericLiteral(_)
        | Expression::StringLiteral(_)
        | Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_) => true,
        Expression::ParenthesizedExpression(inner) => literal(&inner.expression),
        _ => false,
    }
}

/// The longest string literal kept in static memory.
const STATIC_STRING_UNITS: usize = 4096;

/// The text of a property key written as an identifier, string or number,
/// as functions defined under it are named.
fn static_property_name(key: &PropertyKey<'_>) -> Option<String> {
    match key {
        PropertyKey::StaticIdentifier(identifier) => Some(identifier.name.to_string()),
        PropertyKey::StringLiteral(literal) if !literal.lone_surrogates => {
            Some(literal.value.to_string())
        }
        PropertyKey::NumericLiteral(literal)
            if literal.value.fract() == 0.0 && literal.value.abs() < 1e15 =>
        {
            Some(format!("{}", literal.value as i64))
        }
        _ => None,
    }
}

/// The array index a property name denotes, if it is one.
fn canonical_index(name: &str) -> Option<u32> {
    let index: u32 = name.parse().ok()?;
    (index != u32::MAX && index.to_string() == name).then_some(index)
}

/// A canonical array index written as a number literal.
fn index_literal(expression: &Expression<'_>) -> Option<u32> {
    let Expression::NumericLiteral(literal) = expression else {
        return None;
    };
    let value = literal.value;
    (value.fract() == 0.0
        && (0.0..4_294_967_295.0).contains(&value)
        && !(value == 0.0 && value.is_sign_negative()))
    .then_some(value as u32)
}

pub(super) fn number(value: f64) -> String {
    format!("Value::Number(f64::from_bits({:#x}))", value.to_bits())
}

/// Native globals whose reads are implemented; binding writes remain explicit
/// diagnostics until global environment writes are lowered.
/// Node and Electron main-process `process` members the native process
/// object does not provide yet. Members absent on Windows, such as
/// `getuid`, read as `undefined` as they do in Node.
const UNSUPPORTED_PROCESS_MEMBERS: &[&str] = &[
    "_debugEnd",
    "_debugProcess",
    "_fatalException",
    "_getActiveHandles",
    "_getActiveRequests",
    "_kill",
    "_linkedBinding",
    "_preload_modules",
    "_rawDebug",
    "_startProfilerIdleNotifier",
    "_stopProfilerIdleNotifier",
    "_tickCallback",
    "abort",
    "allowedNodeEnvironmentFlags",
    "assert",
    "crash",
    "debugPort",
    "dlopen",
    "domain",
    "execve",
    "features",
    "finalization",
    "getActiveResourcesInfo",
    "getBlinkMemoryInfo",
    "getBuiltinModule",
    "getCPUUsage",
    "getCreationTime",
    "getHeapStatistics",
    "getSystemMemoryInfo",
    "getSystemVersion",
    "hang",
    "hasUncaughtExceptionCaptureCallback",
    "loadEnvFile",
    "mainModule",
    "moduleLoadList",
    "openStdin",
    "reallyExit",
    "ref",
    "setSourceMapsEnabled",
    "setUncaughtExceptionCaptureCallback",
    "sourceMapsEnabled",
    "takeHeapSnapshot",
    "title",
    "unref",
];

/// `process` members inherited from Node's EventEmitter. Reaching them
/// links the Node.js library, which makes `process` an EventEmitter.
const PROCESS_EMITTER_MEMBERS: &[&str] = &[
    "_events",
    "_eventsCount",
    "_maxListeners",
    "addListener",
    "emit",
    "eventNames",
    "getMaxListeners",
    "listenerCount",
    "listeners",
    "off",
    "on",
    "once",
    "prependListener",
    "prependOnceListener",
    "rawListeners",
    "removeAllListeners",
    "removeListener",
    "setMaxListeners",
];

fn native_global(name: &str) -> bool {
    matches!(
        name,
        "Math"
            | "Function"
            | "BigInt"
            | "process"
            | "Array"
            | "String"
            | "Number"
            | "Boolean"
            | "Map"
            | "Set"
            | "WeakMap"
            | "WeakSet"
            | "WeakRef"
            | "FinalizationRegistry"
            | "JSON"
            | "RegExp"
            | "Reflect"
            | "Date"
            | "ArrayBuffer"
            | "DataView"
            | "Int8Array"
            | "Uint8Array"
            | "Uint8ClampedArray"
            | "Int16Array"
            | "Uint16Array"
            | "Int32Array"
            | "Uint32Array"
            | "Float32Array"
            | "Float64Array"
            | "BigInt64Array"
            | "BigUint64Array"
            | "globalThis"
            | "global"
            | "encodeURI"
            | "encodeURIComponent"
            | "decodeURI"
            | "decodeURIComponent"
            | "escape"
            | "unescape"
            | "Promise"
            | "setTimeout"
            | "setInterval"
            | "setImmediate"
            | "clearTimeout"
            | "clearInterval"
            | "clearImmediate"
            | "queueMicrotask"
            | "console"
            | "Object"
            | "Symbol"
            | "parseInt"
            | "parseFloat"
            | "isNaN"
            | "isFinite"
            | "Error"
            | "EvalError"
            | "RangeError"
            | "ReferenceError"
            | "SyntaxError"
            | "TypeError"
            | "URIError"
            | "AggregateError"
            | "SuppressedError"
            | "AbortController"
            | "AbortSignal"
            | "TextEncoder"
            | "TextDecoder"
            | "crypto"
            | "structuredClone"
            | "Event"
            | "EventTarget"
            | "Proxy"
    )
}

fn library_global_module(name: &str) -> Option<&'static str> {
    match name {
        "Buffer" | "atob" | "btoa" | "Blob" | "File" => Some("node:buffer"),
        "URL" | "URLSearchParams" => Some("node:url"),
        "fetch" | "FormData" | "Headers" | "Request" | "Response" => Some("node:internal/fetch"),
        "WebAssembly" => Some("node:internal/webassembly"),
        "WebSocket" => Some("node:internal/websocket"),
        "navigator" => Some("node:internal/navigator"),
        "DOMException" => Some("node:internal/per_context/domexception"),
        "MessageChannel" | "MessagePort" | "MessageEvent" | "BroadcastChannel" => {
            Some("node:internal/worker/io")
        }
        "ReadableStream"
        | "ReadableStreamDefaultReader"
        | "ReadableStreamBYOBReader"
        | "ReadableStreamBYOBRequest"
        | "ReadableByteStreamController"
        | "ReadableStreamDefaultController"
        | "TransformStream"
        | "TransformStreamDefaultController"
        | "WritableStream"
        | "WritableStreamDefaultWriter"
        | "WritableStreamDefaultController"
        | "ByteLengthQueuingStrategy"
        | "CountQueuingStrategy"
        | "TextEncoderStream"
        | "TextDecoderStream"
        | "CompressionStream"
        | "DecompressionStream" => Some("node:stream/web"),
        _ => None,
    }
}

impl Emitter<'_> {
    fn unsupported(&mut self, span: Span, message: &str) {
        self.errors.push(self.source.error(span, message));
    }

    fn id(&mut self) -> usize {
        self.next += 1;
        self.next
    }

    fn frame(&mut self) -> &mut Frame {
        self.frames
            .last_mut()
            .expect("a function is being generated")
    }

    /// The binding an identifier names; `None` for globals, including names
    /// only declared ambiently, which the emitted program never binds.
    fn symbol(&self, identifier: &IdentifierReference<'_>) -> Option<SymbolId> {
        self.scoping
            .get_reference(identifier.reference_id())
            .symbol_id()
            .filter(|symbol| !ambient_only(self.scoping, *symbol))
    }

    fn arguments_owner(&self) -> Option<ScopeId> {
        self.frames
            .iter()
            .rev()
            .find(|frame| {
                let flags = self.scoping.scope_flags(frame.scope);
                flags.is_function() && !flags.is_arrow()
            })
            .map(|frame| frame.scope)
    }

    fn captured(&self, symbol: SymbolId) -> bool {
        self.analysis.captured.contains(&symbol)
            || self.link.cells.contains_key(&symbol)
            || self.mapped_parameters.contains(&symbol)
    }

    /// The name of the module's Rust item of `kind` numbered `id`: in a
    /// module split into parts, `P<module>_<part>_<kind><id>`, which names the
    /// part defining it.
    fn item_name(&self, kind: &str, id: usize) -> String {
        match self.part_bytes {
            Some(_) => format!("P{}_{}_{kind}{id}", self.link.index, self.parts.len()),
            None => format!("{kind}{}_{id}", self.link.index),
        }
    }

    /// How the module's statics are declared: other parts use them.
    fn static_item(&self) -> &'static str {
        if self.part_bytes.is_some() {
            "pub static"
        } else {
            "static"
        }
    }

    /// The header of the module's static string with these units.
    fn static_str(&mut self, units: &[u16]) -> String {
        if let Some(name) = self.strings.get(units) {
            return format!("{name}.header");
        }
        let name = self.item_name("S", self.strings.len());
        self.strings.insert(units.to_vec(), name.clone());
        let value = match String::from_utf16(units) {
            Ok(text) => format!("new({})", rust_str(&text)),
            Err(_) => {
                let list = units.iter().map(u16::to_string).collect::<Vec<_>>();
                format!("from_units([{}])", list.join(", "))
            }
        };
        let _ = writeln!(
            self.statics,
            "{} {name}: StaticStr<{}> = StaticStr::{value};",
            self.static_item(),
            units.len()
        );
        format!("{name}.header")
    }

    /// A string literal's value: a static string, or for a long literal,
    /// whose constant encoding would slow the Rust compiler, a string built
    /// when evaluated.
    fn string_value(&mut self, units: &[u16]) -> String {
        if units.len() > STATIC_STRING_UNITS {
            return units_value(units);
        }
        format!("{}.value()", self.static_str(units))
    }

    /// A Rust `Key` for a property name fixed in the program.
    fn name_key(&mut self, name: &str) -> String {
        match canonical_index(name) {
            Some(index) => format!("Key::Index({index})"),
            None if name.len() > STATIC_STRING_UNITS => format!("Key::name({})", rust_str(name)),
            None => {
                let units: Vec<u16> = name.encode_utf16().collect();
                format!("{}.key()", self.static_str(&units))
            }
        }
    }

    /// A new cache of where one property read found the property, and the
    /// Rust static holding it. Code emitted and then dropped leaves its
    /// cache unused.
    fn lookup_cache(&mut self) -> String {
        let cache = self.item_name("L", self.lookups);
        self.lookups += 1;
        let _ = writeln!(
            self.statics,
            "#[allow(dead_code)]\n{} {cache}: LookupCache = LookupCache::new();",
            self.static_item()
        );
        cache
    }

    /// Reads a property key, from a borrowed object and a borrowed key,
    /// where the read last found it.
    fn get_key(&mut self, object: &str, key: &str) -> String {
        let cache = self.lookup_cache();
        self.fail(&format!("ops::get_at(realm, {object}, {key}, &{cache})"))
    }

    /// Reads a property name fixed in the program from a borrowed object.
    fn get_name(&mut self, object: &str, name: &str) -> String {
        match canonical_index(name) {
            Some(index) => self.fail(&format!("ops::get(realm, {object}, &Key::Index({index}))")),
            None if name.len() > STATIC_STRING_UNITS => self.fail(&format!(
                "ops::get_named(realm, {object}, {})",
                rust_str(name)
            )),
            None => {
                let units: Vec<u16> = name.encode_utf16().collect();
                let header = self.static_str(&units);
                // Each read remembers where it found the property.
                let cache = self.lookup_cache();
                self.fail(&format!(
                    "ops::get_static_at(realm, {object}, &{header}, &{cache})"
                ))
            }
        }
    }

    /// The runtime operation assigning a property: sloppy code ignores
    /// assignments that strict code reports as errors.
    fn setter(&self) -> &'static str {
        if self.strict { "set" } else { "set_sloppy" }
    }

    /// The runtime operation of `delete`.
    fn deleter(&self) -> &'static str {
        if self.strict {
            "delete"
        } else {
            "delete_sloppy"
        }
    }

    /// Wraps a Rust expression of type `Result<T, Thrown>` so that it yields
    /// `T` or leaves through the current exit.
    fn fail(&mut self, operation: &str) -> String {
        match self.frame().exits.last().expect("an exit") {
            Exit::Return => format!("check!({operation})"),
            Exit::Attempt { slot, label } => {
                format!("::nativeboat_native::attempt!({operation}, {slot}, {label})")
            }
        }
    }

    /// The app-relative path a module of an emitted source folder has where
    /// its package's own build emits it (see `CompileOptions::emitted_folders`).
    fn emitted_path(&self, path: &Path) -> Option<String> {
        super::link::emitted_path(&self.options.emitted_folders, path)
    }

    /// The path a module has in the folder the application runs from, as a
    /// packaged application's modules have: the file its package's build
    /// emits for a plugin source, else its place in the application folder,
    /// installed packages included. `None` outside an application folder,
    /// and for sources a bundler builds into one CommonJS file.
    fn application_relative(&self, path: &Path) -> Option<String> {
        if let Some(emitted) = self.emitted_path(path) {
            return Some(emitted);
        }
        if self.options.bundle.is_some() {
            return None;
        }
        let root = self.options.application.as_deref()?;
        let relative = path.strip_prefix(root).ok()?;
        Some(relative.to_string_lossy().replace('\\', "/"))
    }

    /// The path of this module when the program runs, as a value: its path
    /// in the folder the application runs from, or its source.
    fn runtime_filename(&mut self) -> String {
        if let Some(relative) = self.application_relative(self.source.path) {
            return self.fail(&format!(
                "::nativeboat_native::process_builtin::app_relative(realm, {relative:?})"
            ));
        }
        format!(
            "Value::str({})",
            rust_str(&self.source.path.to_string_lossy())
        )
    }

    /// A statement that throws a Rust expression of type `Thrown`.
    fn throw(&mut self, thrown: &str) -> String {
        match self.frame().exits.last().expect("an exit") {
            Exit::Return => format!("return Err({thrown});\n"),
            Exit::Attempt { slot, label } => {
                format!("{{ {slot} = Some({thrown}); break {label}; }}\n")
            }
        }
    }

    /// A Rust expression that throws a new error of the given kind.
    fn error_value(&mut self, kind: &str, message: &str) -> String {
        self.fail(&format!(
            "Err::<Value, Thrown>(realm.throw(ErrorKind::{kind}, {}))",
            rust_str(message)
        ))
    }

    /// Bindings declared directly in a scope: shared cells for captured
    /// bindings and `var` locals. `skip` lists parameters the caller creates;
    /// `catch` parameters are created by their handler.
    fn scope_bindings(&mut self, scope: ScopeId, skip: &[SymbolId]) -> String {
        let mut symbols: Vec<SymbolId> = self
            .scoping
            .get_bindings(scope)
            .values()
            .copied()
            .filter(|symbol| {
                !skip.contains(symbol)
                    && !self.scoping.symbol_flags(*symbol).is_catch_variable()
                    && !ambient_only(self.scoping, *symbol)
                    && self.analysis.own_names.get(&scope) != Some(symbol)
            })
            .collect();
        symbols.sort_by_key(|symbol| symbol.index());
        let mut code = String::new();
        for symbol in symbols {
            let flags = self.scoping.symbol_flags(symbol);
            let var = flags.is_function_scoped_declaration() && !flags.is_function();
            if let Some(cell) = self.link.cells.get(&symbol) {
                let _ = writeln!(code, "let c{} = cells[{cell}].clone();", symbol.index());
            } else if self.captured(symbol) {
                let initial = if var {
                    "Some(Value::Undefined)"
                } else {
                    "None"
                };
                let _ = writeln!(code, "let c{} = binding::new({initial});", symbol.index());
            } else if var {
                let _ = writeln!(code, "let mut b{} = Value::Undefined;", symbol.index());
            }
        }
        code
    }

    /// Creates hoisted function declarations at the start of their scope.
    fn hoist<'r, 'a: 'r>(&mut self, functions: impl Iterator<Item = &'r Function<'a>>) -> String {
        let mut code = String::new();
        // Overload signatures and `declare function` have no runtime behavior.
        for function in functions.filter(|function| function.body.is_some()) {
            let binding = function.id.as_ref().expect("declarations are named");
            let value = self.function(function);
            let symbol = binding.symbol_id();
            if self.captured(symbol) {
                let _ = writeln!(code, "binding::initialize(&c{}, {value});", symbol.index());
            } else {
                let _ = writeln!(code, "let mut b{} = {value};", symbol.index());
            }
        }
        code
    }

    fn block(&mut self, scope: ScopeId, statements: &[Statement<'_>]) -> String {
        let mut code = String::from("{\n");
        code.push_str(&self.scope_bindings(scope, &[]));
        code.push_str(
            &self.hoist(statements.iter().filter_map(|statement| match statement {
                Statement::FunctionDeclaration(function) => Some(&**function),
                _ => None,
            })),
        );
        for statement in statements {
            code.push_str(&self.statement(statement));
        }
        code.push_str("}\n");
        code
    }

    fn statement(&mut self, statement: &Statement<'_>) -> String {
        match statement {
            Statement::VariableDeclaration(declaration) => self.variables(declaration),
            Statement::TSEnumDeclaration(declaration) => self.enum_declaration(declaration),
            Statement::TSNamespaceDeclaration(declaration) => self.namespace(declaration),
            Statement::TSImportEqualsDeclaration(declaration) => {
                self.import_equals(declaration, false)
            }
            // No debugger is attached to a compiled program.
            Statement::DebuggerStatement(_) => String::new(),
            // Types and ambient declarations have no runtime behavior.
            Statement::FunctionDeclaration(_)
            | Statement::EmptyStatement(_)
            | Statement::TSTypeAliasDeclaration(_)
            | Statement::TSInterfaceDeclaration(_)
            | Statement::TSGlobalDeclaration(_)
            | Statement::TSExternalModuleDeclaration(_) => String::new(),
            Statement::ExpressionStatement(statement) => self.effect(&statement.expression),
            Statement::BlockStatement(block) => self.block(block.scope_id(), &block.body),
            Statement::IfStatement(statement) => {
                let test = self.condition(&statement.test);
                let yes = self.nested(&statement.consequent);
                let no = statement
                    .alternate
                    .as_ref()
                    .map(|alternate| format!(" else {}", self.nested(alternate)))
                    .unwrap_or_default();
                format!("if {test} {yes}{no}\n")
            }
            Statement::ReturnStatement(statement) => {
                let value = statement
                    .argument
                    .as_ref()
                    .map_or("Value::Undefined".to_owned(), |argument| {
                        self.expression(argument)
                    });
                let value = if self.frame().async_generator && statement.argument.is_some() {
                    let awaited =
                        self.fail("::nativeboat_native::coroutine::await_value(realm, __returned)");
                    format!("{{ let __returned = {value}; {awaited} }}")
                } else {
                    value
                };
                self.return_value(&value)
            }
            Statement::ThrowStatement(statement) => {
                let value = self.expression(&statement.argument);
                self.throw(&format!("::nativeboat_native::ops::thrown({value})"))
            }
            Statement::WhileStatement(statement) => {
                let labels = std::mem::take(&mut self.frame().labels);
                let test = statement.test.span();
                self.r#loop(
                    labels,
                    None,
                    Some(test),
                    |emitter| emitter.condition(&statement.test),
                    &statement.body,
                    None,
                )
            }
            Statement::DoWhileStatement(statement) => {
                let labels = std::mem::take(&mut self.frame().labels);
                self.do_while(labels, statement)
            }
            Statement::ForStatement(statement) => self.r#for(statement),
            Statement::ForInStatement(statement) => self.for_in(statement),
            Statement::ForOfStatement(statement) => self.for_of(statement),
            Statement::SwitchStatement(statement) => self.switch(statement),
            Statement::BreakStatement(statement) => {
                let label = statement.label.as_ref().map(|label| label.name.to_string());
                self.jump(statement.span, label, true)
            }
            Statement::ContinueStatement(statement) => {
                let label = statement.label.as_ref().map(|label| label.name.to_string());
                self.jump(statement.span, label, false)
            }
            Statement::LabeledStatement(statement) => {
                if matches!(
                    statement.body,
                    Statement::WhileStatement(_)
                        | Statement::DoWhileStatement(_)
                        | Statement::ForStatement(_)
                        | Statement::ForInStatement(_)
                        | Statement::ForOfStatement(_)
                        | Statement::SwitchStatement(_)
                        | Statement::LabeledStatement(_)
                ) {
                    self.frame().labels.push(statement.label.name.to_string());
                    self.statement(&statement.body)
                } else {
                    let id = self.id();
                    let guards = self.frame().guards;
                    let loops = self.frame().loops.len();
                    self.frame().label_blocks.push(LabelBlock {
                        name: statement.label.name.to_string(),
                        exit: format!("'b{id}"),
                        guards,
                        loops,
                    });
                    let body = self.statement(&statement.body);
                    self.frame().label_blocks.pop();
                    format!("'b{id}: {{\n{body}}}\n")
                }
            }
            Statement::TryStatement(statement) => self.r#try(statement),
            Statement::ClassDeclaration(class) => self.class_declaration(class),
            other => {
                let message = format!("{} is not compiled yet", variant(other));
                self.unsupported(other.span(), &message);
                String::new()
            }
        }
    }

    /// A nested statement as a Rust block.
    fn nested(&mut self, statement: &Statement<'_>) -> String {
        match statement {
            Statement::BlockStatement(block) => self.block(block.scope_id(), &block.body),
            other => format!("{{\n{}}}", self.statement(other)),
        }
    }

    fn variables(&mut self, declaration: &VariableDeclaration<'_>) -> String {
        let mut code = String::new();
        if declaration.declare {
            return code;
        }
        let var = declaration.kind == VariableDeclarationKind::Var;
        // `using` declarations were lowered when the module was parsed;
        // those left are in positions the lowering does not reach.
        if !matches!(
            declaration.kind,
            VariableDeclarationKind::Var
                | VariableDeclarationKind::Let
                | VariableDeclarationKind::Const
        ) {
            self.unsupported(
                declaration.span,
                "this declaration kind is not compiled yet",
            );
            return code;
        }
        for declarator in &declaration.declarations {
            let BindingPattern::BindingIdentifier(binding) = &declarator.id else {
                // Destructuring always has an initializer.
                let mode = if var { Mode::Var } else { Mode::Lexical };
                let init = declarator.init.as_ref().expect("patterns are initialized");
                let value = self.expression(init);
                let name = self.temporary("p");
                code.push_str(&self.predeclare(&declarator.id, mode));
                let _ = writeln!(code, "let {name} = {value};");
                code.push_str(&self.bind(&declarator.id, &name, mode));
                continue;
            };
            let symbol = binding.symbol_id();
            let value = match &declarator.init {
                Some(init) => self.named_expression(init, &binding.name),
                // `var x;` keeps its value; `let x;` initializes to undefined.
                None if var => continue,
                None => "Value::Undefined".to_owned(),
            };
            let index = symbol.index();
            if self.captured(symbol) {
                if var {
                    let write = self.fail(&format!(
                        "binding::write(realm, &c{index}, {}, __value)",
                        rust_str(&binding.name)
                    ));
                    let _ = writeln!(code, "{{ let __value = {value}; {write}; }}");
                } else {
                    let _ = writeln!(code, "binding::initialize(&c{index}, {value});");
                }
            } else if var {
                let _ = writeln!(code, "b{index} = {value};");
                self.frame().declared.insert(symbol);
            } else {
                let _ = writeln!(code, "let mut b{index} = {value};");
                self.frame().declared.insert(symbol);
            }
        }
        code
    }

    fn r#loop(
        &mut self,
        labels: Vec<String>,
        setup: Option<String>,
        test_span: Option<Span>,
        test: impl FnOnce(&mut Self) -> String,
        body: &Statement<'_>,
        update: Option<&Expression<'_>>,
    ) -> String {
        let id = self.id();
        let guards = self.frame().guards;
        self.frame().loops.push(Loop {
            names: labels,
            exit: format!("'l{id}"),
            next: Some(format!("'n{id}")),
            guards,
            iterator: None,
        });
        let test = test_span.map(|_| test(self));
        let body = self.nested(body);
        let update = update.map(|update| self.effect(update)).unwrap_or_default();
        self.frame().loops.pop();
        let test = test
            .map(|test| format!("if !({test}) {{ break 'l{id}; }}\n"))
            .unwrap_or_default();
        format!(
            "{{\n{}'l{id}: loop {{\n{test}'n{id}: {body}\n{update}}}\n}}\n",
            setup.unwrap_or_default()
        )
    }

    fn do_while(&mut self, labels: Vec<String>, statement: &DoWhileStatement<'_>) -> String {
        let id = self.id();
        let guards = self.frame().guards;
        self.frame().loops.push(Loop {
            names: labels,
            exit: format!("'l{id}"),
            next: Some(format!("'n{id}")),
            guards,
            iterator: None,
        });
        let body = self.nested(&statement.body);
        self.frame().loops.pop();
        let test = self.condition(&statement.test);
        format!("'l{id}: loop {{\n'n{id}: {body}\nif !({test}) {{ break 'l{id}; }}\n}}\n")
    }

    fn r#for(&mut self, statement: &ForStatement<'_>) -> String {
        let labels = std::mem::take(&mut self.frame().labels);
        let mut setup = self.scope_bindings(statement.scope_id(), &[]);
        // `let` loop variables that closures capture get a fresh binding per
        // iteration.
        let mut captured: Vec<SymbolId> = statement
            .scope_id
            .get()
            .map(|scope| {
                self.scoping
                    .get_bindings(scope)
                    .values()
                    .copied()
                    .filter(|symbol| self.captured(*symbol))
                    .collect()
            })
            .unwrap_or_default();
        captured.sort_by_key(|symbol| symbol.index());
        if let (false, Some(ForStatementInit::VariableDeclaration(declaration))) =
            (captured.is_empty(), &statement.init)
            && declaration.kind == VariableDeclarationKind::Let
        {
            return self.per_iteration(statement, declaration, labels, &captured, setup);
        }
        match &statement.init {
            Some(ForStatementInit::VariableDeclaration(declaration)) => {
                setup.push_str(&self.variables(declaration));
            }
            Some(init) => {
                let expression = init.to_expression();
                setup.push_str(&format!("let _ = {};\n", self.expression(expression)));
            }
            None => {}
        }
        let test_span = statement.test.as_ref().map(GetSpan::span);
        self.r#loop(
            labels,
            Some(setup),
            test_span,
            |emitter| emitter.condition(statement.test.as_ref().expect("a test")),
            &statement.body,
            statement.update.as_ref(),
        )
    }

    fn jump(&mut self, span: Span, label: Option<String>, exit: bool) -> String {
        let guards = self.frame().guards;
        // Syntax checks guarantee a target. A labeled block is the target only
        // when no enclosing loop or switch owns that label.
        let position = self.frame().loops.iter().rposition(|target| {
            label
                    .as_ref()
                    .is_none_or(|label| target.names.contains(label))
                    // `continue` skips `switch`, which has no next iteration.
                    && (exit || target.next.is_some())
        });
        if position.is_none()
            && exit
            && let Some(label) = label.as_deref()
        {
            let target = self
                .frame()
                .label_blocks
                .iter()
                .rfind(|target| target.name == label)
                .map(|target| (target.guards, target.loops, target.exit.clone()))
                .expect("break labels have an enclosing statement target");
            if target.0 != guards {
                return self.defer(
                    Pending::Jump {
                        span,
                        label: Some(label.to_owned()),
                        exit,
                    },
                    "",
                );
            }
            let close = self.close_iterators(target.1);
            return format!("{{ {close}break {}; }}\n", target.2);
        }
        let position = position.expect("jump targets are enclosing loops");
        let target = &self.frame().loops[position];
        if target.guards != guards {
            let pending = Pending::Jump { span, label, exit };
            return self.defer(pending, "");
        }
        let destination = if exit {
            target.exit.clone()
        } else {
            target.next.clone().expect("loops have a continue label")
        };
        // `break` leaves the target loop too; `continue` stays in it.
        let close = self.close_iterators(if exit { position } else { position + 1 });
        format!("{{ {close}break {destination}; }}\n")
    }

    /// `return value` from here: through the innermost `finally` block, or
    /// out of the function. The value is computed before `for`-`of`
    /// iterators close.
    fn return_value(&mut self, value: &str) -> String {
        if self.frame().finally.is_empty() {
            let close = self.close_iterators(0);
            return format!("{{ let __returned = {value}; {close}return Ok(__returned); }}\n");
        }
        let id = self
            .frame()
            .finally
            .last()
            .map(|finally| finally.id)
            .unwrap_or_default();
        self.defer(
            Pending::Return,
            &format!("let __returned = {value}; __r{id} = __returned; "),
        )
    }

    /// Records a jump out of the innermost `finally` region and leaves the
    /// region, closing `for`-`of` iterators opened inside it.
    fn defer(&mut self, pending: Pending, prefix: &str) -> String {
        let (id, label, loops) = {
            let finally = self.frame().finally.last().expect("a finally region");
            (finally.id, finally.label.clone(), finally.loops)
        };
        let close = self.close_iterators(loops);
        let finally = self.frame().finally.last_mut().expect("a finally region");
        finally.jumps.push(pending);
        let index = finally.jumps.len();
        format!("{{ {prefix}{close}__j{id} = {index}; break {label}; }}\n")
    }

    fn r#try(&mut self, statement: &TryStatement<'_>) -> String {
        let id = self.id();
        let slot = format!("t{id}");
        let outer = self.frame().exits.last().cloned().expect("an exit");
        let finally = statement.finalizer.is_some();
        let mut code = format!("{{\nlet mut {slot}: Option<Thrown> = None;\n");
        if finally {
            self.frame().guards += 1;
            let loops = self.frame().loops.len();
            self.frame().finally.push(Finally {
                id,
                label: format!("'t{id}"),
                loops,
                jumps: Vec::new(),
            });
        }
        self.frame().exits.push(Exit::Attempt {
            slot: slot.clone(),
            label: format!("'t{id}"),
        });
        let block = self.block(statement.block.scope_id(), &statement.block.body);
        self.frame().exits.pop();
        let _ = writeln!(code, "'t{id}: {block}");
        if let Some(handler) = &statement.handler {
            if finally {
                let region = self.frame().finally.last_mut();
                region.expect("this statement's finally region").label = format!("'u{id}");
                self.frame().exits.push(Exit::Attempt {
                    slot: slot.clone(),
                    label: format!("'u{id}"),
                });
            }
            let mut body = String::new();
            if let Some(parameter) = &handler.param {
                body.push_str(&self.predeclare(&parameter.pattern, Mode::Fresh));
                body.push_str("let __caught = __thrown.0;\n");
                body.push_str(&self.bind(&parameter.pattern, "__caught", Mode::Fresh));
            }
            body.push_str(&self.block(handler.body.scope_id(), &handler.body.body));
            if finally {
                self.frame().exits.pop();
            }
            // With a finally block, a throw from the handler leaves its own
            // labeled block so the finally block still runs.
            let body = if finally {
                format!("'u{id}: {{\n{body}}}\n")
            } else {
                body
            };
            let body = if self.frame().generator {
                let pass = if finally {
                    format!("{slot} = Some(__thrown);")
                } else {
                    match &outer {
                        Exit::Return => "return Err(__thrown);".to_owned(),
                        Exit::Attempt { slot, label } => {
                            format!("{{ {slot} = Some(__thrown); break {label}; }}")
                        }
                    }
                };
                format!(
                    "if ::nativeboat_native::coroutine::is_return(realm, &__thrown) {{ {pass} }} else {{\n{body}}}\n"
                )
            } else {
                body
            };
            let _ = writeln!(code, "if let Some(__thrown) = {slot}.take() {{\n{body}}}");
        }
        if let Some(finalizer) = &statement.finalizer {
            self.frame().guards -= 1;
            let region = self.frame().finally.pop().expect("this try's region");
            let block = self.block(finalizer.scope_id(), &finalizer.body);
            code.push_str(&block);
            let rethrow = match outer {
                Exit::Return => "return Err(__pending);".to_owned(),
                Exit::Attempt { slot, label } => {
                    format!("{{ {slot} = Some(__pending); break {label}; }}")
                }
            };
            let _ = writeln!(code, "if let Some(__pending) = {slot} {{ {rethrow} }}");
            // Jumps out of the region continue from here, outside it.
            if !region.jumps.is_empty() {
                let mut slots = format!("let mut __j{id}: u32 = 0;\n");
                if region
                    .jumps
                    .iter()
                    .any(|pending| matches!(pending, Pending::Return))
                {
                    let _ = writeln!(slots, "let mut __r{id} = Value::Undefined;");
                }
                code.insert_str(code.find('\n').expect("a block start") + 1, &slots);
            }
            for (index, pending) in region.jumps.iter().enumerate() {
                let jump = match pending {
                    Pending::Return => self.return_value(&format!("__r{id}.clone()")),
                    Pending::Jump { span, label, exit } => self.jump(*span, label.clone(), *exit),
                };
                let _ = writeln!(code, "if __j{id} == {} {{ {jump} }}", index + 1);
            }
        }
        code.push_str("}\n");
        code
    }

    /// A function or arrow function expression creating a native closure.
    fn function(&mut self, function: &Function<'_>) -> String {
        let name = function
            .id
            .as_ref()
            .map_or(String::new(), |id| id.name.to_string());
        self.named_function(function, &name)
    }

    /// A function whose `name` property is `name`.
    fn named_function(&mut self, function: &Function<'_>, name: &str) -> String {
        let body = function
            .body
            .as_ref()
            .expect("declared functions are not compiled");
        let strict = self.strict
            || body
                .directives
                .iter()
                .any(|directive| directive.directive.as_str() == "use strict");
        let callable = Callable::of(function);
        let closure = self.closure(
            name,
            function.scope_id(),
            &function.params,
            Body::Block(&body.statements),
            false,
            callable == Callable::Normal,
            None,
            callable,
            strict,
        );
        let closure = self.with_source_text(function.span, closure);
        // A function expression's own name is an immutable binding of the
        // function itself, in a cell its body captures.
        match self.analysis.own_names.get(&function.scope_id()) {
            Some(own) => format!(
                "{{ let c{0} = binding::new(None); let __function = {closure}; binding::initialize(&c{0}, __function.clone()); __function }}",
                own.index()
            ),
            None => closure,
        }
    }

    /// A function whose text the program reads keeps its source text, which
    /// `Function.prototype.toString` returns.
    fn with_source_text(&self, span: Span, function: String) -> String {
        if !self.analysis.stringified.contains(&span.start) || span.is_empty() {
            return function;
        }
        let Some(text) = self
            .source
            .text
            .get(span.start as usize..span.end as usize)
            .and_then(|text| function_source_text(text, self.typescript))
        else {
            return function;
        };
        format!(
            "::nativeboat_native::with_source_text(realm, {function}, {})",
            rust_str(&text)
        )
    }

    fn arrow(&mut self, arrow: &ArrowFunctionExpression<'_>) -> String {
        self.named_arrow(arrow, "")
    }

    /// An expression's value, where an anonymous function, arrow or class
    /// takes `name`, as the language names those defined in variable
    /// initializers, assignments and property definitions.
    fn named_expression(&mut self, expression: &Expression<'_>, name: &str) -> String {
        match expression {
            Expression::ParenthesizedExpression(inner) => {
                self.named_expression(&inner.expression, name)
            }
            Expression::FunctionExpression(function) if function.id.is_none() => {
                self.named_function(function, name)
            }
            Expression::ArrowFunctionExpression(arrow) => self.named_arrow(arrow, name),
            Expression::ClassExpression(class) if class.id.is_none() => self.class(class, name),
            expression => self.expression(expression),
        }
    }

    /// An arrow function whose `name` property is `name`.
    fn named_arrow(&mut self, arrow: &ArrowFunctionExpression<'_>, name: &str) -> String {
        let callable = if arrow.r#async {
            Callable::Async
        } else {
            Callable::Normal
        };
        let body = match &arrow.body {
            ArrowFunctionBody::FunctionBody(body) => Body::Block(&body.statements),
            expression => Body::Expression(expression.to_expression()),
        };
        let own_strict = if let ArrowFunctionBody::FunctionBody(body) = &arrow.body {
            body.directives
                .iter()
                .any(|directive| directive.directive.as_str() == "use strict")
        } else {
            false
        };
        let strict = self.strict || own_strict;
        let closure = self.closure(
            name,
            arrow.scope_id(),
            &arrow.params,
            body,
            true,
            false,
            None,
            callable,
            strict,
        );
        self.with_source_text(arrow.span, closure)
    }

    /// Clones what a closure for `scope` captures into fresh Rust bindings
    /// ahead of its `move` closure.
    fn captures(&self, scope: ScopeId) -> String {
        let mut captures = String::new();
        if let Some(own) = self.analysis.own_names.get(&scope) {
            let _ = writeln!(captures, "let c{0} = c{0}.clone();", own.index());
        }
        if let Some(free) = self.analysis.free.get(&scope) {
            for symbol in free {
                let _ = writeln!(captures, "let c{0} = c{0}.clone();", symbol.index());
            }
        }
        if let Some(owners) = self.analysis.this.get(&scope) {
            for owner in owners {
                let _ = writeln!(captures, "let this{0} = this{0}.clone();", owner.index());
                if self.homes.contains(owner) {
                    let _ = writeln!(captures, "let home{0} = home{0}.clone();", owner.index());
                }
                if self.constructors.contains_key(owner) || self.function_targets.contains(owner) {
                    let _ = writeln!(
                        captures,
                        "let target{0} = target{0}.clone();",
                        owner.index()
                    );
                }
            }
        }
        if let Some(owners) = self.analysis.arguments.get(&scope) {
            for owner in owners {
                let _ = writeln!(captures, "let a{0} = a{0}.clone();", owner.index());
            }
        }
        for (_, key) in self.classes.iter().flatten() {
            let _ = writeln!(captures, "let {key} = {key}.clone();");
        }
        // Functions with lowered `require` calls keep the module tables.
        if self.link.require_scopes.contains(&scope) {
            captures.push_str("let cells = cells.clone();\nlet cache = cache.clone();\n");
        }
        captures
    }

    /// In a module split into parts, what a closure for `scope` captures, as
    /// the names and types of the parameters of the Rust function its body
    /// becomes. Closures capturing class state stay closures.
    fn outlined_parameters(&self, scope: ScopeId) -> Option<Vec<(String, &'static str)>> {
        self.part_bytes?;
        if self.classes.iter().any(|keys| !keys.is_empty()) {
            return None;
        }
        let mut parameters = Vec::new();
        let cells = self.analysis.own_names.get(&scope).into_iter();
        for symbol in cells.chain(self.analysis.free.get(&scope).into_iter().flatten()) {
            parameters.push((format!("c{}", symbol.index()), "binding::Binding"));
        }
        for owner in self.analysis.this.get(&scope).into_iter().flatten() {
            let flags = self.scoping.scope_flags(*owner);
            if self.homes.contains(owner)
                || self.constructors.contains_key(owner)
                || !(flags.is_function() || flags.is_top())
            {
                return None;
            }
            parameters.push((format!("this{}", owner.index()), "Value"));
            if self.function_targets.contains(owner) {
                parameters.push((format!("target{}", owner.index()), "Value"));
            }
        }
        for owner in self.analysis.arguments.get(&scope).into_iter().flatten() {
            parameters.push((format!("a{}", owner.index()), "Value"));
        }
        if self.link.require_scopes.contains(&scope) {
            parameters.push(("cells".to_owned(), "::alloc::rc::Rc<Vec<binding::Binding>>"));
            parameters.push((
                "cache".to_owned(),
                "::alloc::rc::Rc<Vec<::core::cell::RefCell<Option<Object>>>>",
            ));
        }
        let mut seen = HashSet::new();
        parameters.retain(|(name, _)| seen.insert(name.clone()));
        Some(parameters)
    }

    /// A function value of a split module whose body became the Rust
    /// function `body` returning `returns`. A Rust function beside it makes
    /// the value with `make` (`realm.function` or a coroutine maker) from a
    /// closure calling the body, so the code creating it holds no closure of
    /// its own. The part ends once it is large enough.
    fn outlined_closure(
        &mut self,
        captures: &str,
        parameters: &[(String, &str)],
        make: &str,
        returns: &str,
        body: &str,
    ) -> String {
        let function = self.item_name("f", self.outlined);
        let maker = self.item_name("v", self.outlined);
        self.outlined += 1;
        let declared = parameters
            .iter()
            .map(|(name, kind)| format!(", {name}: &{kind}"))
            .collect::<String>();
        let passed = parameters
            .iter()
            .map(|(name, _)| format!(", &{name}"))
            .collect::<String>();
        let allow = "#[allow(unused_mut, unused_variables, unused_assignments, unused_labels, unreachable_code, unused_braces, unused_parens, non_snake_case, clippy::all)]";
        let _ = write!(
            self.functions,
            "\n{allow}\npub fn {function}(realm: &mut Realm, this: Value, args: &[Value]{declared}) -> {returns} {{\n{body}\n}}\n\
             {allow}\npub fn {maker}(realm: &mut Realm{declared}) -> Value {{\n{captures}Value::Object({make}move |realm: &mut Realm, this: Value, args: &[Value]| -> {returns} {{ {function}(realm, this, args{passed}) }}))\n}}\n"
        );
        if let Some(limit) = self.part_bytes
            && self.functions.len() + self.statics.len() >= limit
        {
            let mut part = core::mem::take(&mut self.functions);
            part.push_str(&core::mem::take(&mut self.statics));
            self.parts.push(part);
        }
        format!("{maker}(realm{passed})")
    }

    /// A closure creating a function object; `home` names the home object of
    /// a class method.
    #[allow(clippy::too_many_arguments)]
    fn closure(
        &mut self,
        name: &str,
        scope: ScopeId,
        params: &FormalParameters<'_>,
        body: Body<'_, '_>,
        arrow: bool,
        constructor: bool,
        home: Option<&str>,
        callable: Callable,
        strict: bool,
    ) -> String {
        let mut captures = self.captures(scope);
        // A coroutine body owns fresh copies of what each call captures.
        let mut recaptured = captures.clone();
        if let Some(home) = home {
            let _ = writeln!(captures, "let home{} = {home}.clone();", scope.index());
            // The async/generator factory is an `Fn` and may be called more
            // than once. Its returned body is also `move`, so give that body
            // a fresh copy of the method home before constructing it.
            let _ = writeln!(recaptured, "let home{0} = home{0}.clone();", scope.index());
            self.homes.insert(scope);
        }
        let prologue = if arrow {
            String::new()
        } else {
            self.function_targets.insert(scope);
            format!(
                "let this{0} = this;\nlet target{0} = realm.new_target();\n",
                scope.index()
            )
        };
        let generator = matches!(callable, Callable::Generator | Callable::AsyncGenerator);
        self.async_body = callable == Callable::AsyncGenerator;
        let previous_strict = self.strict;
        self.strict = strict;
        let (head, code, result, length) =
            self.function_body(scope, params, body, &prologue, generator);
        self.strict = previous_strict;
        let name = rust_str(name);
        let sloppy_this = if !arrow && !strict {
            "let this = ops::sloppy_this(realm, this);\n"
        } else {
            ""
        };
        // A function's body may reach module state only through what it
        // captures to become a Rust function of its own.
        let uses =
            |name: &&str| head.contains(name) || code.contains(name) || result.contains(name);
        // `import.meta` is the module's one object: each function that reads
        // it, and each coroutine body, takes a reference of its own.
        if uses(&"__import_meta") {
            captures.push_str("let __import_meta = __import_meta.clone();\n");
            recaptured.push_str("let __import_meta = __import_meta.clone();\n");
        }
        let outlined = self.outlined_parameters(scope).filter(|parameters| {
            let tables = parameters.iter().any(|(name, _)| name == "cells");
            home.is_none()
                && !uses(&"__import_meta")
                && (tables
                    || ![
                        "cells[",
                        "&cells",
                        "&cache",
                        "cells.clone()",
                        "cache.clone()",
                    ]
                    .iter()
                    .any(uses))
        });
        if let Some(parameters) = &outlined
            && callable == Callable::Normal
        {
            return self.outlined_closure(
                &captures,
                parameters,
                &format!("realm.function({name}, {length}, {constructor}, "),
                "::nativeboat_native::Completion",
                &format!("{sloppy_this}{head}{code}{result}"),
            );
        }
        // Generator and async bodies run as coroutines after the parameters
        // are bound at call time.
        let maker = match callable {
            Callable::Normal => {
                return format!(
                    "{{\n{captures}Value::Object(realm.function({name}, {length}, {constructor}, move |realm: &mut Realm, this: Value, args: &[Value]| -> ::nativeboat_native::Completion {{\n{sloppy_this}{head}{code}{result}\n}}))\n}}"
                );
            }
            Callable::Generator => "generator_function",
            Callable::Async => "async_function",
            Callable::AsyncGenerator => "async_generator_function",
        };
        if let Some(parameters) = &outlined {
            return self.outlined_closure(
                &captures,
                parameters,
                &format!("::nativeboat_native::coroutine::{maker}(realm, {name}, {length}, "),
                "Result<::nativeboat_native::coroutine::Body, Thrown>",
                &format!(
                    "{sloppy_this}{head}{recaptured}Ok(::alloc::boxed::Box::new(move |realm: &mut Realm| -> ::nativeboat_native::Completion {{\n{code}{result}\n}}))"
                ),
            );
        }
        format!(
            "{{\n{captures}Value::Object(::nativeboat_native::coroutine::{maker}(realm, {name}, {length}, move |realm: &mut Realm, this: Value, args: &[Value]| -> Result<::nativeboat_native::coroutine::Body, Thrown> {{\n{sloppy_this}{head}{recaptured}Ok(::alloc::boxed::Box::new(move |realm: &mut Realm| -> ::nativeboat_native::Completion {{\n{code}{result}\n}}))\n}}))\n}}"
        )
    }

    /// A function's statements for `scope` over `args`: its arguments object,
    /// `prologue`, parameters and body; the Rust expression completing it; and
    /// its `length`.
    fn function_body(
        &mut self,
        scope: ScopeId,
        params: &FormalParameters<'_>,
        body: Body<'_, '_>,
        prologue: &str,
        generator: bool,
    ) -> (String, String, String, u32) {
        self.frames.push(frame(scope));
        self.frame().generator = generator;
        self.frame().async_generator = core::mem::take(&mut self.async_body);
        let mut code = String::new();
        let simple_parameters = params.rest.is_none()
            && params.items.iter().all(|parameter| {
                parameter.initializer.is_none()
                    && matches!(parameter.pattern, BindingPattern::BindingIdentifier(_))
            });
        let mapped_arguments =
            self.analysis.owns_arguments.contains(&scope) && !self.strict && simple_parameters;
        let mapped = if mapped_arguments {
            let mut mapped = Vec::new();
            for (index, parameter) in params.items.iter().enumerate().rev() {
                // `simple_parameters` above proves this list contains exactly
                // one binding identifier in every parameter pattern.
                let binding = &parameter.pattern.get_binding_identifiers()[0];
                let symbol = binding.symbol_id();
                self.mapped_parameters.insert(symbol);
                mapped.push((index, symbol.index()));
            }
            mapped.reverse();
            mapped
        } else {
            Vec::new()
        };
        if self.analysis.owns_arguments.contains(&scope) && !mapped_arguments {
            let _ = writeln!(
                code,
                "let a{} = Value::Object(realm.arguments(args));",
                scope.index()
            );
        }
        code.push_str(prologue);
        let length = params
            .items
            .iter()
            .take_while(|parameter| parameter.initializer.is_none())
            .count() as u32;
        let mut parameters = Vec::new();
        // Create every parameter cell before evaluating any default.
        for parameter in &params.items {
            code.push_str(&self.predeclare(&parameter.pattern, Mode::Fresh));
        }
        if let Some(rest) = &params.rest {
            code.push_str(&self.predeclare(&rest.rest.argument, Mode::Fresh));
        }
        for (index, parameter) in params.items.iter().enumerate() {
            parameters.extend(
                parameter
                    .pattern
                    .get_binding_identifiers()
                    .iter()
                    .map(|binding| binding.symbol_id()),
            );
            let mut value = format!("args.get({index}).cloned().unwrap_or_default()");
            if let Some(default) = &parameter.initializer {
                let fallback = self.expression(default);
                value = format!(
                    "{{ let __argument = {value}; if matches!(__argument, Value::Undefined) {{ {fallback} }} else {{ __argument }} }}"
                );
            }
            if let BindingPattern::BindingIdentifier(binding) = &parameter.pattern {
                code.push_str(&self.store_value(binding, &value, Mode::Fresh));
                continue;
            }
            let name = self.temporary("p");
            let _ = writeln!(code, "let {name} = {value};");
            code.push_str(&self.bind(&parameter.pattern, &name, Mode::Fresh));
        }
        if let Some(rest) = &params.rest {
            let pattern = &rest.rest.argument;
            parameters.extend(
                pattern
                    .get_binding_identifiers()
                    .iter()
                    .map(|binding| binding.symbol_id()),
            );
            let count = params.items.len();
            let _ = writeln!(
                code,
                "let __rest = Value::Object(realm.array(args.get({count}..).map(<[Value]>::to_vec).unwrap_or_default()));"
            );
            code.push_str(&self.bind(pattern, "__rest", Mode::Fresh));
        }
        if self.constructors.get(&scope) != Some(&true)
            && let Some(properties) = self.constructor_parameter_properties.get(&scope).cloned()
        {
            let target = format!("this{}", scope.index());
            for (name, symbol) in properties {
                code.push_str(&self.parameter_property(&name, symbol, &target));
            }
            if let Some(fields) = self.deferred_fields.get(&scope).cloned() {
                let initialize = self.fail(&format!(
                    "::nativeboat_native::class::initialize(realm, {fields}, &{target})"
                ));
                let _ = writeln!(code, "{initialize};");
            }
        }
        if mapped_arguments {
            let mappings = mapped
                .iter()
                .map(|(index, symbol)| format!("({index}, c{symbol}.clone())"))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(
                code,
                "let a{} = Value::Object(realm.mapped_arguments(args, &[{mappings}]));",
                scope.index()
            );
        }
        // Parameters are bound at call time; the rest is the body.
        let head = core::mem::take(&mut code);
        let result = match body {
            Body::Block(statements) => {
                code.push_str(&self.scope_bindings(scope, &parameters));
                code.push_str(&self.hoist(statements.iter().filter_map(
                    |statement| match statement {
                        Statement::FunctionDeclaration(function) => Some(&**function),
                        _ => None,
                    },
                )));
                for statement in statements {
                    code.push_str(&self.statement(statement));
                }
                "Ok(Value::Undefined)".to_owned()
            }
            Body::Expression(expression) => {
                code.push_str(&self.scope_bindings(scope, &parameters));
                format!("Ok({})", self.expression(expression))
            }
        };
        self.frames.pop();
        (head, code, result, length)
    }

    /// Reads an identifier.
    fn read(&mut self, identifier: &IdentifierReference<'_>) -> String {
        let Some(symbol) = self.symbol(identifier) else {
            // Sources a bundler builds into CommonJS have its path globals:
            // the bundle's, in the application.
            if let Some(bundle) = &self.options.bundle
                && !self
                    .source
                    .path
                    .components()
                    .any(|component| component.as_os_str() == "node_modules")
                && matches!(identifier.name.as_str(), "__dirname" | "__filename")
            {
                let relative = if identifier.name == "__dirname" {
                    bundle
                        .rsplit_once('/')
                        .map_or("", |(directory, _)| directory)
                } else {
                    bundle.as_str()
                };
                return self.fail(&format!(
                    "::nativeboat_native::process_builtin::app_relative(realm, {})",
                    rust_str(relative)
                ));
            }
            // Electron's TypeScript sources are loaded from CommonJS entry
            // points and expect the path globals that TypeScript's runtime
            // transpilation would normally place in the module wrapper.
            if self
                .source
                .path
                .extension()
                .is_some_and(|extension| extension == "ts")
            {
                match identifier.name.as_str() {
                    "__filename" => return self.runtime_filename(),
                    "__dirname" => {
                        if let Some(relative) = self.application_relative(self.source.path) {
                            let directory =
                                relative.rsplit_once('/').map_or("", |(folder, _)| folder);
                            return self.fail(&format!(
                                "::nativeboat_native::process_builtin::app_relative(realm, {directory:?})"
                            ));
                        }
                        let directory = self.source.path.parent().unwrap_or_else(|| Path::new(""));
                        return format!("Value::str({})", rust_str(&directory.to_string_lossy()));
                    }
                    _ => {}
                }
            }
            if let Some(value) = self.options.defines.get(identifier.name.as_str()) {
                if *value == CompileValue::GlobalThis {
                    return self.native_global("globalThis");
                }
                return Self::compile_value(value);
            }
            if identifier.name == "arguments"
                && let Some(owner) = self.arguments_owner()
            {
                return format!("a{}.clone()", owner.index());
            }
            if let Some(module) = library_global_module(identifier.name.as_str()) {
                return self.library_global(module, identifier.name.as_str());
            }
            return match identifier.name.as_str() {
                "undefined" => "Value::Undefined".to_owned(),
                "NaN" => number(f64::NAN),
                "Infinity" => number(f64::INFINITY),
                // `process` used as a value may reach its emitter members.
                "process" => self.library_global("node:events", "process"),
                name if native_global(name) => self.native_global(name),
                name => self.fail(&format!("ops::get_global(realm, {})", rust_str(name))),
            };
        };
        let index = symbol.index();
        if self.captured(symbol) {
            return self.fail(&format!(
                "binding::read(realm, &c{index}, {})",
                rust_str(&identifier.name)
            ));
        }
        let flags = self.scoping.symbol_flags(symbol);
        let lexical = flags.is_block_scoped() && !flags.is_function();
        if lexical
            && !self.frame().declared.contains(&symbol)
            && function_scope(self.scoping, self.scoping.symbol_scope_id(symbol))
                == self.frame().scope
            && !flags.is_catch_variable()
        {
            return self.error_value(
                "Reference",
                &format!("Cannot access '{}' before initialization", identifier.name),
            );
        }
        format!("b{index}.clone()")
    }

    /// Assigns a value held in the Rust variable `__value` to an identifier.
    fn assign_identifier(&mut self, identifier: &IdentifierReference<'_>) -> String {
        let Some(symbol) = self.symbol(identifier) else {
            // Unbound ambient names are routed through GlobalReference by
            // identifier_target; this is only reached for native globals,
            // whose members compiled code calls directly, so a replacement
            // could not take effect. Programs that replace one only when it
            // is missing, as Emscripten's glue does with `console`, compile;
            // a replacement that runs throws.
            let thrown = self.error_value(
                "Type",
                &format!(
                    "Replacing the global {} is not supported natively",
                    identifier.name
                ),
            );
            return format!("let _ = {thrown};\n");
        };
        let flags = self.scoping.symbol_flags(symbol);
        let own_name = self.analysis.own_names.values().any(|own| *own == symbol);
        if flags.is_const_variable() || flags.is_import() || own_name {
            // An uninitialized constant fails its TDZ check before the
            // immutable-binding error (including defaults inside patterns).
            let read = self.read(identifier);
            let thrown = self.error_value("Type", "Assignment to constant variable.");
            return format!("let _ = {read}; let _ = {thrown};\n");
        }
        let index = symbol.index();
        if self.captured(symbol) {
            let write = self.fail(&format!(
                "binding::write(realm, &c{index}, {}, __value.clone())",
                rust_str(&identifier.name)
            ));
            format!("{write};\n")
        } else {
            format!("b{index} = __value.clone();\n")
        }
    }

    fn expression(&mut self, expression: &Expression<'_>) -> String {
        self.expression_in(expression, None)
    }

    /// An expression; `chain` is the label of an enclosing optional chain.
    fn expression_in(&mut self, expression: &Expression<'_>, chain: Option<&str>) -> String {
        if let Expression::CallExpression(call) = expression {
            if let Some(path) = self.link.resolutions.get(&call.span) {
                return path_value(path);
            }
            if self.link.create_require_constructors.contains(&call.span) {
                return "Value::Undefined".to_owned();
            }
            if let Some(target) = self.link.create_require_calls.get(&call.span) {
                return self.fail(&format!("require_c{target}(realm, &cells, &cache)"));
            }
            if self.link.missing_requires.contains(&call.span) {
                // A computed request is required when the program runs: a
                // compiled entry by its path, or through `Module._load`.
                let request = match call
                    .arguments
                    .first()
                    .and_then(|argument| argument.as_expression())
                {
                    Some(argument) => self.expression(argument),
                    None => "Value::Undefined".to_owned(),
                };
                let parent = self.runtime_filename();
                return self.fail(&format!(
                    "{{ let __request = {request}; let __parent = {parent}; ::nativeboat_native::modules::require_unlinked(realm, &__request, &__parent) }}"
                ));
            }
        }
        match expression {
            Expression::NumericLiteral(literal) => number(literal.value),
            Expression::BigIntLiteral(literal) => format!(
                "::nativeboat_native::bigint::literal({})",
                rust_str(&literal.value)
            ),
            Expression::StringLiteral(literal) => {
                let units = literal_units(&literal.value, literal.lone_surrogates);
                self.string_value(&units)
            }
            Expression::BooleanLiteral(literal) => format!("Value::Boolean({})", literal.value),
            Expression::NullLiteral(_) => "Value::Null".to_owned(),
            Expression::TemplateLiteral(template) => self.template(template),
            Expression::RegExpLiteral(literal) => self.regexp(literal),
            Expression::YieldExpression(expression) => {
                let value = match &expression.argument {
                    Some(argument) => self.expression(argument),
                    None => "Value::Undefined".to_owned(),
                };
                let operation = match (expression.delegate, self.frame().async_generator) {
                    (true, true) => "async_yield_delegate",
                    (true, false) => "yield_delegate",
                    (false, true) => "async_yield",
                    (false, false) => "yield_value",
                };
                let call = self.fail(&format!(
                    "::nativeboat_native::coroutine::{operation}(realm, __yielded)"
                ));
                format!("{{ let __yielded = {value}; {call} }}")
            }
            Expression::AwaitExpression(expression) => {
                let value = self.expression(&expression.argument);
                let call =
                    self.fail("::nativeboat_native::coroutine::await_value(realm, __awaited)");
                format!("{{ let __awaited = {value}; {call} }}")
            }
            Expression::Identifier(identifier) => self.read(identifier),
            Expression::ThisExpression(_) => self.this_value(),
            Expression::ImportMeta(_) => "Value::Object(__import_meta.clone())".to_owned(),
            Expression::ImportExpression(import) => {
                match self.link.dynamic_imports.get(&import.span).cloned() {
                    Some(super::link::DynamicImport::Entry(entry)) => {
                        self.fail(&format!("realm.dynamic_import({entry})"))
                    }
                    // A specifier computed at run time loads the candidate
                    // whose path ends it.
                    Some(super::link::DynamicImport::Computed(candidates)) => {
                        let specifier = self.expression(&import.source);
                        let candidates = candidates
                            .iter()
                            .map(|(key, entry)| format!("({}, {entry})", rust_str(key)))
                            .collect::<Vec<_>>()
                            .join(", ");
                        let load = self.fail(&format!(
                            "realm.dynamic_import_matching(&__specifier, &[{candidates}])"
                        ));
                        format!("{{ let __specifier = {specifier}; {load} }}")
                    }
                    None => {
                        self.fail("Err(realm.throw(ErrorKind::Type, \"unlinked dynamic import\"))")
                    }
                }
            }
            Expression::NewTarget(_) => self.new_target(),
            Expression::ClassExpression(class) => {
                let name = class.id.as_ref().map_or("", |id| id.name.as_str());
                self.class(class, name)
            }
            Expression::PrivateFieldExpression(field) => {
                format!("{{ {}__member }}", self.private_member_parts(field, chain))
            }
            Expression::PrivateInExpression(test) => self.private_in(test),
            Expression::ParenthesizedExpression(inner) => {
                self.expression_in(&inner.expression, chain)
            }
            Expression::TSAsExpression(inner) => self.expression_in(&inner.expression, chain),
            Expression::TSSatisfiesExpression(inner) => {
                self.expression_in(&inner.expression, chain)
            }
            Expression::TSNonNullExpression(inner) => self.expression_in(&inner.expression, chain),
            Expression::TSTypeAssertion(inner) => self.expression_in(&inner.expression, chain),
            Expression::TSInstantiationExpression(inner) => {
                self.expression_in(&inner.expression, chain)
            }
            Expression::ArrayExpression(array) => self.array(array),
            Expression::ObjectExpression(object) => self.object(object),
            Expression::FunctionExpression(function) => self.function(function),
            Expression::ArrowFunctionExpression(arrow) => self.arrow(arrow),
            Expression::UnaryExpression(unary) => self.unary(unary),
            Expression::UpdateExpression(update) => self.update(update),
            Expression::BinaryExpression(binary) => self.binary(binary),
            Expression::LogicalExpression(logical) => {
                let left = self.expression(&logical.left);
                let right = self.expression(&logical.right);
                let test = match logical.operator {
                    LogicalOperator::And => "ops::to_boolean(&__left)",
                    LogicalOperator::Or => "!ops::to_boolean(&__left)",
                    LogicalOperator::Coalesce => "__left.is_nullish()",
                };
                format!("{{ let __left = {left}; if {test} {{ {right} }} else {{ __left }} }}")
            }
            Expression::ConditionalExpression(conditional) => {
                let test = self.condition(&conditional.test);
                let yes = self.expression(&conditional.consequent);
                let no = self.expression(&conditional.alternate);
                format!("if {test} {{ {yes} }} else {{ {no} }}")
            }
            Expression::SequenceExpression(sequence) => {
                let mut code = String::from("{ ");
                let count = sequence.expressions.len();
                for (index, item) in sequence.expressions.iter().enumerate() {
                    let value = self.expression(item);
                    if index + 1 == count {
                        code.push_str(&value);
                    } else {
                        let _ = write!(code, "let _ = {value}; ");
                    }
                }
                code.push_str(" }");
                code
            }
            Expression::AssignmentExpression(assignment) => self.assignment(assignment),
            Expression::StaticMemberExpression(member) => self.static_member(member, chain),
            Expression::ComputedMemberExpression(member) => self.computed_member(member, chain),
            Expression::CallExpression(call) => self.call(call, chain),
            Expression::TaggedTemplateExpression(tagged) => self.tagged(tagged),
            Expression::NewExpression(new) => {
                let callee = self.expression(&new.callee);
                let arguments = self.arguments(&new.arguments);
                let site = rust_str(&call_site_text(&new.callee));
                let construct = self.fail(&format!(
                    "ops::construct_site(realm, &__callee, &__arguments, {site})"
                ));
                format!("{{ let __callee = {callee}; {arguments}{construct} }}")
            }
            Expression::ChainExpression(chain_expression) => {
                let id = self.id();
                let label = format!("'o{id}");
                let inner = match &chain_expression.expression {
                    ChainElement::CallExpression(call) => self.call(call, Some(&label)),
                    ChainElement::StaticMemberExpression(member) => {
                        self.static_member(member, Some(&label))
                    }
                    ChainElement::ComputedMemberExpression(member) => {
                        self.computed_member(member, Some(&label))
                    }
                    ChainElement::PrivateFieldExpression(field) => format!(
                        "{{ {}__member }}",
                        self.private_member_parts(field, Some(&label))
                    ),
                    ChainElement::TSNonNullExpression(inner) => {
                        self.expression_in(&inner.expression, Some(&label))
                    }
                };
                format!("{label}: {{ {inner} }}")
            }
            other => {
                let message = format!("{} is not compiled yet", variant(other));
                self.unsupported(other.span(), &message);
                "Value::Undefined".to_owned()
            }
        }
    }

    /// Reports direct reads of Node and Electron `process` members, and of
    /// standard stream members, that the native process object does not
    /// provide yet, instead of letting them read as `undefined`.
    fn check_process_member(&mut self, span: Span, object: &Expression<'_>, name: &str) {
        // `globalThis.Request` and the like read library globals too: link
        // their modules, which the globals' accessors load.
        if let Expression::Identifier(root) = object
            && self.symbol(root).is_none()
            && matches!(
                root.name.as_str(),
                "globalThis" | "global" | "self" | "window"
            )
            && let Some(module) = library_global_module(name)
        {
            self.link
                .native_dependencies
                .borrow_mut()
                .insert(module.to_owned());
        }
        let path = if self.global_process(object) {
            if !UNSUPPORTED_PROCESS_MEMBERS.contains(&name) {
                return;
            }
            format!("process.{name}")
        } else if let Expression::StaticMemberExpression(stream) = object
            && self.global_process(&stream.object)
            && matches!(stream.property.name.as_str(), "stdout" | "stderr")
            // Columns and rows are undefined unless the stream is a
            // terminal, which a windowed application's is not.
            && !matches!(
                name,
                "write" | "fd" | "isTTY" | "columns" | "rows" | "destroyed" | "writable"
            )
            && !PROCESS_EMITTER_MEMBERS.contains(&name)
        {
            format!("process.{}.{name}", stream.property.name)
        } else if let Expression::StaticMemberExpression(stream) = object
            && self.global_process(&stream.object)
            && matches!(stream.property.name.as_str(), "stdout" | "stderr")
            && PROCESS_EMITTER_MEMBERS.contains(&name)
        {
            // Node's standard streams inherit EventEmitter methods. Loading the
            // library is required so the native stream receives those methods
            // before compiled code registers its listeners.
            self.link
                .native_dependencies
                .borrow_mut()
                .insert("node:events".to_owned());
            return;
        } else {
            return;
        };
        self.unsupported(span, &format!("{path} is not compiled yet"));
    }

    fn library_global(&mut self, module: &str, name: &str) -> String {
        self.link
            .native_dependencies
            .borrow_mut()
            .insert(module.to_owned());
        self.fail(&format!(
            "::nativeboat_native::modules::global(realm, {}, {})",
            rust_str(module),
            rust_str(name)
        ))
    }

    /// Reads a global the runtime provides.
    fn native_global(&mut self, name: &str) -> String {
        if matches!(name, "global" | "globalThis") {
            // Computed reads can select any library-backed global. Preserve
            // bootstrap semantics without guessing from generated Rust text.
            self.link
                .native_dependencies
                .borrow_mut()
                .extend(["node:buffer", "node:url", "node:events"].map(str::to_owned));
        }
        let read = self.get_name("&global", name);
        format!("{{ let global = Value::Object(realm.global.clone()); {read} }}")
    }

    /// The object of a member access. `process` reads without linking the
    /// Node.js library when the member is known not to be an emitter's.
    fn member_object(
        &mut self,
        object: &Expression<'_>,
        chain: Option<&str>,
        name: Option<&str>,
    ) -> String {
        if self.global_process(object)
            && name.is_some_and(|name| !PROCESS_EMITTER_MEMBERS.contains(&name))
        {
            return self.native_global("process");
        }
        self.expression_in(object, chain)
    }

    fn global_process(&self, expression: &Expression<'_>) -> bool {
        matches!(expression, Expression::Identifier(reference)
            if reference.name == "process" && self.symbol(reference).is_none())
    }

    /// `object.name` into `__member`, keeping `__object` for a method call.
    fn static_member_parts(
        &mut self,
        member: &StaticMemberExpression<'_>,
        chain: Option<&str>,
    ) -> String {
        if let Expression::Super(_) = &member.object {
            let key = self.name_key(&member.property.name);
            return self.super_member_parts(member.span, &key);
        }
        self.check_process_member(member.span, &member.object, &member.property.name);
        let name = Some(member.property.name.as_str());
        let object = self.member_object(&member.object, chain, name);
        let guard = self.optional(member.optional, chain);
        let get = self.get_name("&__object", &member.property.name);
        format!("let __object = {object}; {guard}let __member = {get}; ")
    }

    /// `object[key]` into `__member`, keeping `__object` for a method call.
    fn computed_member_parts(
        &mut self,
        member: &ComputedMemberExpression<'_>,
        chain: Option<&str>,
    ) -> String {
        if let Expression::Super(_) = &member.object {
            let key = self.expression(&member.expression);
            let to_key = self.fail("ops::to_key(realm, &__key)");
            let key = format!("{{ let __key = {key}; {to_key} }}");
            return self.super_member_parts(member.span, &key);
        }
        let name = match &member.expression {
            Expression::StringLiteral(key) => Some(key.value.as_str()),
            _ => None,
        };
        if let Some(name) = name {
            self.check_process_member(member.span, &member.object, name);
        }
        let object = self.member_object(&member.object, chain, name);
        let guard = self.optional(member.optional, chain);
        let key = self.expression(&member.expression);
        let to_key = self.fail("ops::to_key(realm, &__key)");
        let get = self.get_key("&__object", "&__name");
        format!(
            "let __object = {object}; {guard}let __key = {key}; let __name = {to_key}; let __member = {get}; "
        )
    }

    fn static_member(
        &mut self,
        member: &StaticMemberExpression<'_>,
        chain: Option<&str>,
    ) -> String {
        if matches!(&member.object, Expression::ImportMeta(_))
            && !matches!(
                member.property.name.as_str(),
                "url" | "env" | "dirname" | "filename"
            )
        {
            self.unsupported(
                member.span,
                &format!("import.meta.{} is not compiled yet", member.property.name),
            );
            return "Value::Undefined".to_owned();
        }
        if let Some(value) = self.import_meta_env_member(member) {
            return value;
        }
        if let Some(value) = self.dotted_define(member) {
            return value;
        }
        if let Expression::Super(_) = &member.object {
            return format!("{{ {}__member }}", self.static_member_parts(member, chain));
        }
        self.check_process_member(member.span, &member.object, &member.property.name);
        let name = Some(member.property.name.as_str());
        let object = self.member_object(&member.object, chain, name);
        if !member.optional
            && let Some(local) = borrowed(&object)
        {
            return self.get_name(&format!("&{local}"), &member.property.name);
        }
        let guard = self.optional(member.optional, chain);
        let get = self.get_name("&__object", &member.property.name);
        format!("{{ let __object = {object}; {guard}{get} }}")
    }

    /// A build-time define for a dotted global path, such as Vite's
    /// `process.env.NODE_ENV`, when its first name is not a local binding.
    fn dotted_define(&self, member: &StaticMemberExpression<'_>) -> Option<String> {
        if self.options.defines.is_empty() || member.optional {
            return None;
        }
        let mut names = vec![member.property.name.as_str()];
        let mut object = &member.object;
        loop {
            match object {
                Expression::StaticMemberExpression(inner) if !inner.optional => {
                    names.push(inner.property.name.as_str());
                    object = &inner.object;
                }
                Expression::Identifier(root) if self.symbol(root).is_none() => {
                    names.push(root.name.as_str());
                    break;
                }
                _ => return None,
            }
        }
        names.reverse();
        let value = self.options.defines.get(&names.join("."))?;
        Some(Self::compile_value(value))
    }

    /// Compiles a statically known `import.meta.env.NAME` read.
    fn import_meta_env_member(&self, member: &StaticMemberExpression<'_>) -> Option<String> {
        let Expression::StaticMemberExpression(env) = &member.object else {
            return None;
        };
        if env.property.name != "env" || !matches!(&env.object, Expression::ImportMeta(_)) {
            return None;
        }
        let environment = self.options.import_meta_env.as_ref()?;
        Some(
            environment
                .get(member.property.name.as_str())
                .map_or_else(|| "Value::Undefined".to_owned(), Self::compile_value),
        )
    }

    fn compile_value(value: &CompileValue) -> String {
        match value {
            CompileValue::GlobalThis => "Value::Object(realm.global.clone())".to_owned(),
            CompileValue::Undefined => "Value::Undefined".to_owned(),
            CompileValue::Null => "Value::Null".to_owned(),
            CompileValue::Boolean(value) => format!("Value::Boolean({value})"),
            CompileValue::Number(value) => number(*value),
            CompileValue::String(value) => format!("Value::str({})", rust_str(value)),
        }
    }

    fn computed_member(
        &mut self,
        member: &ComputedMemberExpression<'_>,
        chain: Option<&str>,
    ) -> String {
        let name = match &member.expression {
            Expression::StringLiteral(literal) if !literal.lone_surrogates => {
                Some(literal.value.to_string())
            }
            expression => index_literal(expression).map(|index| index.to_string()),
        };
        if !member.optional
            && !matches!(&member.object, Expression::Super(_))
            && let Some(name) = name
        {
            self.check_process_member(member.span, &member.object, &name);
            let object = self.member_object(&member.object, chain, Some(&name));
            return match borrowed(&object) {
                Some(local) => self.get_name(&format!("&{local}"), &name),
                None => {
                    let get = self.get_name("&__object", &name);
                    format!("{{ let __object = {object}; {get} }}")
                }
            };
        }
        format!(
            "{{ {}__member }}",
            self.computed_member_parts(member, chain)
        )
    }

    /// The short circuit of an optional `?.` step on `__object`.
    fn optional(&mut self, optional: bool, chain: Option<&str>) -> String {
        match (optional, chain) {
            (true, Some(label)) => {
                format!("if __object.is_nullish() {{ break {label} Value::Undefined; }} ")
            }
            _ => String::new(),
        }
    }

    fn template(&mut self, template: &TemplateLiteral<'_>) -> String {
        let mut code = String::from("{ let mut __text = ::nativeboat_native::JsStr::default(); ");
        for (index, quasi) in template.quasis.iter().enumerate() {
            let cooked = quasi
                .value
                .cooked
                .as_ref()
                .expect("untagged templates are cooked");
            let units = literal_units(cooked, quasi.lone_surrogates);
            if !units.is_empty() {
                let part = if units.len() > STATIC_STRING_UNITS {
                    let list = units.iter().map(u16::to_string).collect::<Vec<_>>();
                    format!(
                        "::nativeboat_native::JsStr::from_units(&[{}])",
                        list.join(", ")
                    )
                } else {
                    format!("{}.str()", self.static_str(&units))
                };
                let _ = write!(code, "__text = __text.concat(&{part}); ");
            }
            if let Some(expression) = template.expressions.get(index) {
                let value = self.expression(expression);
                let text = self.fail("ops::to_string(realm, &__part)");
                let _ = write!(
                    code,
                    "{{ let __part = {value}; let __part = {text}; __text = __text.concat(&__part); }} "
                );
            }
        }
        code.push_str("Value::String(__text) }");
        code
    }

    fn array(&mut self, array: &ArrayExpression<'_>) -> String {
        // Without spreads or holes the elements are a list of values.
        if array.elements.iter().all(|element| {
            !matches!(
                element,
                ArrayExpressionElement::SpreadElement(_) | ArrayExpressionElement::Elision(_)
            )
        }) {
            let values = array
                .elements
                .iter()
                .map(|element| self.expression(element.to_expression()))
                .collect::<Vec<_>>();
            return format!(
                "{{ let __elements = vec![{}]; Value::Object(realm.array(__elements)) }}",
                values.join(", ")
            );
        }
        let mut code = String::from("{ let mut __elements: Vec<Option<Value>> = Vec::new(); ");
        for element in &array.elements {
            match element {
                ArrayExpressionElement::SpreadElement(spread) => {
                    let value = self.expression(&spread.argument);
                    let values = self.fail("::nativeboat_native::iter::collect(realm, &__spread)");
                    let _ = write!(
                        code,
                        "{{ let __spread = {value}; __elements.extend({values}.into_iter().map(Some)); }} "
                    );
                }
                ArrayExpressionElement::Elision(_) => code.push_str("__elements.push(None); "),
                element => {
                    let value = self.expression(element.to_expression());
                    let _ = write!(code, "__elements.push(Some({value})); ");
                }
            }
        }
        code.push_str("Value::Object(realm.sparse_array(__elements)) }");
        code
    }

    /// A public property key.
    fn property_key(&mut self, key: &PropertyKey<'_>) -> String {
        match key {
            PropertyKey::StaticIdentifier(identifier) => self.name_key(&identifier.name),
            PropertyKey::StringLiteral(literal) if !literal.lone_surrogates => {
                self.name_key(&literal.value)
            }
            key => {
                let value = self.expression(key.to_expression());
                let key = self.fail("ops::to_key(realm, &__key)");
                format!("{{ let __key = {value}; {key} }}")
            }
        }
    }

    fn object(&mut self, object: &ObjectExpression<'_>) -> String {
        // The literal's own properties are room enough for most.
        let mut code = match object.properties.len() {
            0 => String::from("{ let __target = realm.object(); "),
            count => format!("{{ let __target = realm.object_sized({}); ", count.min(64)),
        };
        for property in &object.properties {
            let property = match property {
                ObjectPropertyKind::ObjectProperty(property) => property,
                ObjectPropertyKind::SpreadProperty(spread) => {
                    let value = self.expression(&spread.argument);
                    let copy = self.fail(
                        "::nativeboat_native::iter::copy_properties(realm, &__target, &__spread, &[])",
                    );
                    let _ = write!(code, "{{ let __spread = {value}; {copy}; }} ");
                    continue;
                }
            };
            let proto_key = match &property.key {
                PropertyKey::StaticIdentifier(name) => name.name == "__proto__",
                PropertyKey::StringLiteral(name) => name.value == "__proto__",
                _ => false,
            };
            // `__proto__: value` sets the prototype to an object or null and
            // is otherwise ignored; it defines no property.
            if proto_key
                && !property.computed
                && property.kind == PropertyKind::Init
                && !property.method
                && !property.shorthand
            {
                let value = self.expression(&property.value);
                let _ = write!(
                    code,
                    "{{ match {value} {{ Value::Object(__proto) => {{ __target.set_prototype(Some(__proto)); }} Value::Null => {{ __target.set_prototype(None); }} _ => {{}} }} }} "
                );
                continue;
            }
            let key = self.property_key(&property.key);
            // Functions defined by a property take its name; accessors are
            // `get name` and `set name`.
            let name = if property.computed {
                None
            } else {
                static_property_name(&property.key)
            };
            let name = name.map(|name| match property.kind {
                PropertyKind::Init => name,
                PropertyKind::Get => format!("get {name}"),
                PropertyKind::Set => format!("set {name}"),
            });
            let value = match &property.value {
                Expression::FunctionExpression(function)
                    if property.method || property.kind != PropertyKind::Init =>
                {
                    self.method(function, name.as_deref().unwrap_or(""))
                }
                value => match &name {
                    Some(name) => self.named_expression(value, name),
                    None => self.expression(value),
                },
            };
            // Functions under computed keys are named when the key is known.
            let anonymous = property.computed
                && match &property.value {
                    Expression::FunctionExpression(function) => function.id.is_none(),
                    Expression::ArrowFunctionExpression(_) => true,
                    Expression::ClassExpression(class) => class.id.is_none(),
                    _ => false,
                };
            if anonymous {
                let prefix = match property.kind {
                    PropertyKind::Init => "",
                    PropertyKind::Get => "get ",
                    PropertyKind::Set => "set ",
                };
                let define = match property.kind {
                    PropertyKind::Init => "ops::define_data(&__target, __key, __value);",
                    PropertyKind::Get => {
                        "ops::define_accessor(&__target, __key, __value.as_object().cloned(), None);"
                    }
                    PropertyKind::Set => {
                        "ops::define_accessor(&__target, __key, None, __value.as_object().cloned());"
                    }
                };
                let _ = write!(
                    code,
                    "{{ let __key = {key}; let __value = {value}; ops::name_function(&__value, &__key, {prefix:?}); {define} }} "
                );
                continue;
            }
            if property.kind == PropertyKind::Init {
                let _ = write!(code, "ops::define_data(&__target, {key}, {value}); ");
                continue;
            }
            let define = match property.kind {
                PropertyKind::Init => unreachable!("data properties are defined above"),
                PropertyKind::Get => {
                    "ops::define_accessor(&__target, __key, __value.as_object().cloned(), None);"
                        .to_owned()
                }
                PropertyKind::Set => {
                    "ops::define_accessor(&__target, __key, None, __value.as_object().cloned());"
                        .to_owned()
                }
            };
            let _ = write!(
                code,
                "{{ let __key = {key}; let __value = {value}; {define} }} "
            );
        }
        code.push_str("Value::Object(__target) }");
        code
    }

    /// Object literal methods and accessors are not constructors.
    fn method(&mut self, function: &Function<'_>, name: &str) -> String {
        let body = function.body.as_ref().expect("methods have bodies");
        self.closure(
            name,
            function.scope_id(),
            &function.params,
            Body::Block(&body.statements),
            false,
            false,
            None,
            Callable::of(function),
            true,
        )
    }

    fn arguments(&mut self, arguments: &[Argument<'_>]) -> String {
        // Without spreads the count is fixed: an array, evaluated in order.
        if !arguments
            .iter()
            .any(|argument| matches!(argument, Argument::SpreadElement(_)))
        {
            let values = arguments
                .iter()
                .map(|argument| self.expression(argument.to_expression()))
                .collect::<Vec<_>>();
            return if values.is_empty() {
                "let __arguments: [Value; 0] = []; ".to_owned()
            } else {
                format!("let __arguments = [{}]; ", values.join(", "))
            };
        }
        let mut code = String::from("let mut __arguments: Vec<Value> = Vec::new(); ");
        for argument in arguments {
            match argument {
                Argument::SpreadElement(spread) => {
                    let value = self.expression(&spread.argument);
                    let values = self.fail("::nativeboat_native::iter::collect(realm, &__spread)");
                    let _ = write!(
                        code,
                        "{{ let __spread = {value}; __arguments.extend({values}); }} "
                    );
                }
                argument => {
                    let value = self.expression(argument.to_expression());
                    let _ = write!(code, "__arguments.push({value}); ");
                }
            }
        }
        code
    }

    /// Evaluates a callee into `__callee`, with a member's object as `__this`.
    fn callee(&mut self, callee: &Expression<'_>, chain: Option<&str>) -> String {
        // Parentheses and TypeScript's type wrappers keep a member call's
        // `this`, as `(a.b)()` and `a.b!()` call `b` on `a`.
        match callee.get_inner_expression() {
            Expression::StaticMemberExpression(member)
                if !member.optional && !matches!(&member.object, Expression::Super(_)) =>
            {
                self.check_process_member(member.span, &member.object, &member.property.name);
                let name = Some(member.property.name.as_str());
                let object = self.member_object(&member.object, chain, name);
                let get = self.get_name("&__this", &member.property.name);
                format!("let __this = {object}; let __callee = {get}; ")
            }
            Expression::StaticMemberExpression(member) => format!(
                "{}let __this = __object; let __callee = __member; ",
                self.static_member_parts(member, chain)
            ),
            Expression::ComputedMemberExpression(member) => format!(
                "{}let __this = __object; let __callee = __member; ",
                self.computed_member_parts(member, chain)
            ),
            Expression::PrivateFieldExpression(field) => format!(
                "{}let __this = __object; let __callee = __member; ",
                self.private_member_parts(field, chain)
            ),
            callee => {
                let value = self.expression_in(callee, chain);
                format!("let __this = Value::Undefined; let __callee = {value}; ")
            }
        }
    }

    fn call(&mut self, call: &CallExpression<'_>, chain: Option<&str>) -> String {
        if let Expression::Super(_) = &call.callee {
            return self.super_call(&call.arguments);
        }
        let callee = self.callee(&call.callee, chain);
        let guard = match (call.optional, chain) {
            (true, Some(label)) => {
                format!("if __callee.is_nullish() {{ break {label} Value::Undefined; }} ")
            }
            _ => String::new(),
        };
        let arguments = self.arguments(&call.arguments);
        let site = rust_str(&call_site_text(&call.callee));
        let invoke = self.fail(&format!(
            "ops::call_site(realm, &__callee, __this, &__arguments, {site})"
        ));
        format!("{{ {callee}{guard}{arguments}{invoke} }}")
    }

    /// A regular expression literal: a new RegExp object per evaluation,
    /// sharing its compiled pattern. Invalid patterns are early errors.
    fn regexp(&mut self, literal: &RegExpLiteral<'_>) -> String {
        let units: Vec<u16> = literal.regex.pattern.text.encode_utf16().collect();
        let flags = literal.regex.flags;
        let options = nativeboat_regexp::Flags {
            ignore_case: flags.contains(RegExpFlags::I),
            multiline: flags.contains(RegExpFlags::M),
            dot_all: flags.contains(RegExpFlags::S),
            unicode: flags.contains(RegExpFlags::U),
            unicode_sets: flags.contains(RegExpFlags::V),
        };
        if let Err(message) = nativeboat_regexp::Pattern::compile(&units, options) {
            let message = format!("invalid regular expression: {message}");
            self.unsupported(literal.span, &message);
        }
        let list = units
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        self.fail(&format!(
            "::nativeboat_native::regexp_literal(realm, &[{list}], {})",
            rust_str(&flags.to_string())
        ))
    }

    /// A tagged template call: the tag receives the call site's cached
    /// strings object and the substitution values.
    fn tagged(&mut self, tagged: &TaggedTemplateExpression<'_>) -> String {
        let callee = self.callee(&tagged.tag, None);
        let site = ((self.link.index as u64) << 32) | self.id() as u64;
        let units = |units: &[u16]| {
            let list = units
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            format!("&[{list}][..]")
        };
        let mut cooked = Vec::new();
        let mut raw = Vec::new();
        for quasi in &tagged.quasi.quasis {
            cooked.push(match &quasi.value.cooked {
                Some(text) => format!(
                    "Some({})",
                    units(&literal_units(text, quasi.lone_surrogates))
                ),
                None => "None".to_owned(),
            });
            // Raw strings normalize CR LF and CR line terminators to LF.
            let text = quasi.value.raw.replace("\r\n", "\n").replace('\r', "\n");
            raw.push(units(&text.encode_utf16().collect::<Vec<u16>>()));
        }
        let mut code = format!(
            "{{ {callee}let mut __arguments: Vec<Value> = vec![Value::Object(realm.template_object({site}, &[{}], &[{}]))]; ",
            cooked.join(", "),
            raw.join(", ")
        );
        for expression in &tagged.quasi.expressions {
            let value = self.expression(expression);
            let _ = write!(code, "__arguments.push({value}); ");
        }
        let invoke = self.fail("ops::call(realm, &__callee, __this, &__arguments)");
        let _ = write!(code, "{invoke} }}");
        code
    }

    fn unary(&mut self, unary: &UnaryExpression<'_>) -> String {
        if unary.operator == UnaryOperator::Delete {
            return match &unary.argument {
                Expression::StaticMemberExpression(member) => {
                    let name = Some(member.property.name.as_str());
                    let object = self.member_object(&member.object, None, name);
                    let key = self.name_key(&member.property.name);
                    let delete = self.fail(&format!(
                        "ops::{}(realm, &__object, &{key})",
                        self.deleter()
                    ));
                    format!("{{ let __object = {object}; Value::Boolean({delete}) }}")
                }
                Expression::ComputedMemberExpression(member) => {
                    let object = self.expression(&member.object);
                    let key = self.expression(&member.expression);
                    let to_key = self.fail("ops::to_key(realm, &__key)");
                    let delete = self.fail(&format!(
                        "ops::{}(realm, &__object, &__name)",
                        self.deleter()
                    ));
                    format!(
                        "{{ let __object = {object}; let __key = {key}; let __name = {to_key}; Value::Boolean({delete}) }}"
                    )
                }
                other => {
                    // Deleting anything else evaluates it and yields true.
                    let value = self.expression(other);
                    format!("{{ let _ = {value}; Value::Boolean(true) }}")
                }
            };
        }
        if unary.operator == UnaryOperator::Typeof
            && let Expression::Identifier(reference) = &unary.argument
            && self.symbol(reference).is_none()
            && !matches!(reference.name.as_str(), "undefined" | "NaN" | "Infinity")
            && library_global_module(reference.name.as_str()).is_none()
            && !self.options.defines.contains_key(reference.name.as_str())
            && !native_global(reference.name.as_str())
        {
            // This helper already returns the JavaScript result of `typeof`.
            // Wrapping it in the ordinary `typeof` emitter would produce
            // "string" instead of values such as "undefined" or "number".
            return self.fail(&format!(
                "ops::type_of_global(realm, {})",
                rust_str(&reference.name)
            ));
        }
        let value = match &unary.argument {
            // `typeof process` does not reach its members.
            argument
                if unary.operator == UnaryOperator::Typeof && self.global_process(argument) =>
            {
                self.native_global("process")
            }
            argument => self.expression(argument),
        };
        let result = match unary.operator {
            UnaryOperator::UnaryNegation => self.fail("ops::negate(realm, &__operand)"),
            UnaryOperator::UnaryPlus => {
                let number = self.fail("ops::to_number(realm, &__operand)");
                format!("Value::Number({number})")
            }
            UnaryOperator::LogicalNot => {
                return format!("Value::Boolean(!{})", self.condition(&unary.argument));
            }
            UnaryOperator::BitwiseNot => self.fail("ops::bitwise_not(realm, &__operand)"),
            UnaryOperator::Typeof => "Value::str(ops::type_of(&__operand))".to_owned(),
            // `delete` returned above.
            UnaryOperator::Void | UnaryOperator::Delete => "Value::Undefined".to_owned(),
        };
        format!("{{ let __operand = {value}; {result} }}")
    }

    fn binary(&mut self, binary: &BinaryExpression<'_>) -> String {
        if let Some(test) = self.type_test(binary) {
            return format!("Value::Boolean({test})");
        }
        if let Some(chain) = self.long_chain(binary) {
            return chain;
        }
        let (setup, left, right) = self.operands(&binary.left, &binary.right);
        let result = match self.comparison(binary.operator, &left, &right) {
            Some(test) => format!("Value::Boolean({test})"),
            None => self.binary_operation(binary.operator, &left, &right),
        };
        if setup.is_empty() {
            result
        } else {
            format!("{{ {setup}{result} }}")
        }
    }

    /// A long left-nested chain of operators producing values, as in the
    /// `a + b + c + ...` of generated tables, as a sequence of statements:
    /// blocks nested as deep as the chain overflow the Rust compiler's
    /// stack. Each operand is evaluated, and each operation applied, in
    /// JavaScript's left-to-right order.
    fn long_chain(&mut self, binary: &BinaryExpression<'_>) -> Option<String> {
        const LONG_CHAIN: usize = 32;
        if Self::compares(binary.operator) {
            return None;
        }
        let mut links = vec![(binary.operator, &binary.right)];
        let mut first = &binary.left;
        while let Expression::BinaryExpression(inner) = first
            && !Self::compares(inner.operator)
        {
            links.push((inner.operator, &inner.right));
            first = &inner.left;
        }
        if links.len() < LONG_CHAIN {
            return None;
        }
        let mut code = format!("{{ let mut __chain = {}; ", self.expression(first));
        for (operator, right) in links.into_iter().rev() {
            let right = self.expression(right);
            let value = self.binary_operation(operator, "&__chain", "&__right");
            let _ = write!(code, "{{ let __right = {right}; __chain = {value}; }} ");
        }
        code.push_str("__chain }");
        Some(code)
    }

    /// Code evaluating two operands in order, and Rust expressions
    /// borrowing them. A local is borrowed in place when the code after it
    /// cannot change it: a literal or another borrowed local.
    fn operands(
        &mut self,
        left: &Expression<'_>,
        right: &Expression<'_>,
    ) -> (String, String, String) {
        let left_code = self.expression(left);
        let right_code = self.expression(right);
        match (borrowed(&left_code), borrowed(&right_code)) {
            (Some(left), Some(right)) => (String::new(), format!("&{left}"), format!("&{right}")),
            (Some(left), None) if literal(right) => {
                (String::new(), format!("&{left}"), format!("&{right_code}"))
            }
            (None, Some(right)) => (
                format!("let __left = {left_code}; "),
                "&__left".to_owned(),
                format!("&{right}"),
            ),
            _ => (
                format!("let __left = {left_code}; let __right = {right_code}; "),
                "&__left".to_owned(),
                "&__right".to_owned(),
            ),
        }
    }

    /// A JavaScript test as a Rust `bool`: comparisons, `!` and the
    /// short-circuit operators test directly instead of building Boolean
    /// values to convert back.
    fn condition(&mut self, expression: &Expression<'_>) -> String {
        match expression {
            Expression::ParenthesizedExpression(inner) => self.condition(&inner.expression),
            Expression::UnaryExpression(unary) if unary.operator == UnaryOperator::LogicalNot => {
                format!("!{}", self.condition(&unary.argument))
            }
            Expression::LogicalExpression(logical)
                if logical.operator != LogicalOperator::Coalesce =>
            {
                let left = self.condition(&logical.left);
                let right = self.condition(&logical.right);
                let operator = if logical.operator == LogicalOperator::And {
                    "&&"
                } else {
                    "||"
                };
                format!("({left} {operator} {right})")
            }
            Expression::BinaryExpression(binary) if let Some(test) = self.type_test(binary) => test,
            Expression::BinaryExpression(binary) if Self::compares(binary.operator) => {
                let (setup, left, right) = self.operands(&binary.left, &binary.right);
                let test = self
                    .comparison(binary.operator, &left, &right)
                    .expect("comparison operators test");
                if setup.is_empty() {
                    test
                } else {
                    format!("{{ {setup}{test} }}")
                }
            }
            other => {
                let value = self.expression(other);
                match borrowed(&value) {
                    Some(local) => format!("ops::to_boolean(&{local})"),
                    None => format!("ops::to_boolean(&{value})"),
                }
            }
        }
    }

    /// `typeof x == "name"` and its variants as a Rust `bool` comparing the
    /// type name, for operands other than unbound names.
    fn type_test(&mut self, binary: &BinaryExpression<'_>) -> Option<String> {
        let negated = match binary.operator {
            BinaryOperator::StrictEquality | BinaryOperator::Equality => false,
            BinaryOperator::StrictInequality | BinaryOperator::Inequality => true,
            _ => return None,
        };
        let (unary, name) = match (&binary.left, &binary.right) {
            (Expression::UnaryExpression(unary), Expression::StringLiteral(name))
            | (Expression::StringLiteral(name), Expression::UnaryExpression(unary))
                if unary.operator == UnaryOperator::Typeof =>
            {
                (unary, name)
            }
            _ => return None,
        };
        if let Expression::Identifier(reference) = &unary.argument
            && self.symbol(reference).is_none()
        {
            return None;
        }
        let value = self.expression(&unary.argument);
        let operator = if negated { "!=" } else { "==" };
        let name = rust_str(&name.value);
        Some(match borrowed(&value) {
            Some(local) => format!("(ops::type_of(&{local}) {operator} {name})"),
            None => {
                format!("{{ let __operand = {value}; ops::type_of(&__operand) {operator} {name} }}")
            }
        })
    }

    fn compares(operator: BinaryOperator) -> bool {
        operator.is_equality() || operator.is_compare() || operator.is_relational()
    }

    /// A comparison of two borrowed operands as a Rust `bool`, or `None` for
    /// operators producing other values.
    fn comparison(&mut self, operator: BinaryOperator, left: &str, right: &str) -> Option<String> {
        let relation =
            |name: &str| format!("ops::relational(realm, Relation::{name}, {left}, {right})");
        Some(match operator {
            BinaryOperator::StrictEquality => format!("ops::strict_equals({left}, {right})"),
            BinaryOperator::StrictInequality => format!("!ops::strict_equals({left}, {right})"),
            BinaryOperator::Equality => {
                self.fail(&format!("ops::loose_equals(realm, {left}, {right})"))
            }
            BinaryOperator::Inequality => format!(
                "!{}",
                self.fail(&format!("ops::loose_equals(realm, {left}, {right})"))
            ),
            BinaryOperator::LessThan => self.fail(&relation("Less")),
            BinaryOperator::GreaterThan => self.fail(&relation("Greater")),
            BinaryOperator::LessEqualThan => self.fail(&relation("LessOrEqual")),
            BinaryOperator::GreaterEqualThan => self.fail(&relation("GreaterOrEqual")),
            BinaryOperator::In => self.fail(&format!("ops::in_operator(realm, {left}, {right})")),
            BinaryOperator::Instanceof => {
                self.fail(&format!("ops::instance_of(realm, {left}, {right})"))
            }
            _ => return None,
        })
    }

    /// A binary operator applied to two borrowed operands.
    fn binary_operation(&mut self, operator: BinaryOperator, left: &str, right: &str) -> String {
        if let Some(test) = self.comparison(operator, left, right) {
            return format!("Value::Boolean({test})");
        }
        let arithmetic =
            |name: &str| format!("ops::arithmetic(realm, Arithmetic::{name}, {left}, {right})");
        match operator {
            BinaryOperator::Addition => self.fail(&format!("ops::add(realm, {left}, {right})")),
            BinaryOperator::Subtraction => self.fail(&arithmetic("Subtract")),
            BinaryOperator::Multiplication => self.fail(&arithmetic("Multiply")),
            BinaryOperator::Division => self.fail(&arithmetic("Divide")),
            BinaryOperator::Remainder => self.fail(&arithmetic("Remainder")),
            BinaryOperator::Exponential => self.fail(&arithmetic("Exponent")),
            BinaryOperator::BitwiseAnd => self.fail(&arithmetic("And")),
            BinaryOperator::BitwiseOR => self.fail(&arithmetic("Or")),
            BinaryOperator::BitwiseXOR => self.fail(&arithmetic("Xor")),
            BinaryOperator::ShiftLeft => self.fail(&arithmetic("ShiftLeft")),
            BinaryOperator::ShiftRight => self.fail(&arithmetic("ShiftRight")),
            BinaryOperator::ShiftRightZeroFill => self.fail(&arithmetic("ShiftRightUnsigned")),
            _ => unreachable!("comparisons return above"),
        }
    }

    /// The Rust code reading a simple target into `__old` and the code
    /// writing `__value` back. Member targets evaluate their object and key
    /// once, into `__object` and `__name`.
    fn target(&mut self, target: &SimpleAssignmentTarget<'_>) -> Option<(String, String, String)> {
        match target {
            SimpleAssignmentTarget::AssignmentTargetIdentifier(identifier) => {
                Some(self.identifier_target(identifier))
            }
            SimpleAssignmentTarget::StaticMemberExpression(member) => {
                Some(self.static_member_target(member))
            }
            SimpleAssignmentTarget::ComputedMemberExpression(member) => {
                Some(self.computed_member_target(member))
            }
            SimpleAssignmentTarget::PrivateFieldExpression(field) => {
                let object = self.expression(&field.object);
                let name = self.private_name(&field.field.name);
                let setup = format!("let __object = {object}; ");
                let read = self.fail(&format!(
                    "::nativeboat_native::class::private_get(realm, &__object, &{name})"
                ));
                let write = self.fail(&format!(
                    "::nativeboat_native::class::private_set(realm, &__object, &{name}, __value.clone())"
                ));
                Some((setup, read, format!("{write};\n")))
            }
            // TypeScript wrappers.
            other => other
                .get_expression()
                .and_then(|inner| self.wrapped_target(inner)),
        }
    }

    /// `object.name` as an assignment target, or `super.name`.
    fn static_member_target(
        &mut self,
        member: &StaticMemberExpression<'_>,
    ) -> (String, String, String) {
        if matches!(member.object, Expression::Super(_)) {
            let key = self.name_key(&member.property.name);
            return self.super_target(member.span, key);
        }
        let name = Some(member.property.name.as_str());
        let object = self.member_object(&member.object, None, name);
        let key = self.name_key(&member.property.name);
        let setup = format!("let __object = {object}; let __name = {key}; ");
        let read = self.get_name("&__object", &member.property.name);
        let cache = self.lookup_cache();
        let write = self.fail(&format!(
            "ops::{}_at(realm, &__object, &__name, __value.clone(), &{cache})",
            self.setter()
        ));
        (setup, read, format!("{write};\n"))
    }

    /// `object[key]` as an assignment target, or `super[key]`.
    fn computed_member_target(
        &mut self,
        member: &ComputedMemberExpression<'_>,
    ) -> (String, String, String) {
        if matches!(member.object, Expression::Super(_)) {
            let key = self.expression(&member.expression);
            let to_key = self.fail("ops::to_key(realm, &__key)");
            return self.super_target(member.span, format!("{{ let __key = {key}; {to_key} }}"));
        }
        let object = self.expression(&member.object);
        let key = self.expression(&member.expression);
        let to_key = self.fail("ops::to_key(realm, &__key)");
        let setup = format!("let __object = {object}; let __key = {key}; let __name = {to_key}; ");
        let read = self.get_key("&__object", "&__name");
        let cache = self.lookup_cache();
        let write = self.fail(&format!(
            "ops::{}_at(realm, &__object, &__name, __value.clone(), &{cache})",
            self.setter()
        ));
        (setup, read, format!("{write};\n"))
    }

    fn wrapped_target(&mut self, expression: &Expression<'_>) -> Option<(String, String, String)> {
        match expression {
            Expression::Identifier(identifier) => Some(self.identifier_target(identifier)),
            Expression::ParenthesizedExpression(inner) => self.wrapped_target(&inner.expression),
            Expression::TSAsExpression(inner) => self.wrapped_target(&inner.expression),
            Expression::TSSatisfiesExpression(inner) => self.wrapped_target(&inner.expression),
            Expression::TSNonNullExpression(inner) => self.wrapped_target(&inner.expression),
            Expression::TSTypeAssertion(inner) => self.wrapped_target(&inner.expression),
            Expression::StaticMemberExpression(member) => Some(self.static_member_target(member)),
            Expression::ComputedMemberExpression(member) => {
                Some(self.computed_member_target(member))
            }
            other => {
                self.unsupported(other.span(), "this assignment target is not compiled yet");
                None
            }
        }
    }

    /// Capture an ambient global name without reading its value. Compound
    /// assignments read before the RHS; stores check the current property,
    /// matching Node 22/V8 when the RHS creates or deletes a global binding.
    fn identifier_target(
        &mut self,
        identifier: &IdentifierReference<'_>,
    ) -> (String, String, String) {
        if self.symbol(identifier).is_none() {
            let reference = format!(
                "ops::global_reference(realm, {}, {})",
                rust_str(identifier.name.as_str()),
                self.strict,
            );
            let read = self.fail("__global_reference.get(realm)");
            // Native globals cannot be replaced: their write throws.
            let write = if native_global(identifier.name.as_str()) {
                self.assign_identifier(identifier)
            } else {
                let write = self.fail("__global_reference.set(realm, __value.clone())");
                format!("{write};\n")
            };
            return (
                format!("let __global_reference = {reference};\n"),
                read,
                write,
            );
        }
        (
            String::new(),
            self.read(identifier),
            self.assign_identifier(identifier),
        )
    }

    fn assignment(&mut self, assignment: &AssignmentExpression<'_>) -> String {
        let Some(target) = assignment.left.as_simple_assignment_target() else {
            let right = self.expression(&assignment.right);
            let pattern = self.assign_target(&assignment.left, "__assigned");
            return format!("{{ let __assigned = {right}; {pattern}__assigned }}");
        };
        let Some((setup, read, write)) = self.target(target) else {
            return "Value::Undefined".to_owned();
        };
        let operator = assignment.operator;
        // A function assigned to a name, also by `||=` and its kin, takes
        // that name.
        let right = match target {
            SimpleAssignmentTarget::AssignmentTargetIdentifier(identifier)
                if operator == AssignmentOperator::Assign || operator.is_logical() =>
            {
                self.named_expression(&assignment.right, &identifier.name)
            }
            _ => self.expression(&assignment.right),
        };
        match (
            operator.to_logical_operator(),
            operator.to_binary_operator(),
        ) {
            (Some(logical), _) => {
                let keep = match logical {
                    LogicalOperator::And => "!ops::to_boolean(&__left)",
                    LogicalOperator::Or => "ops::to_boolean(&__left)",
                    LogicalOperator::Coalesce => "!__left.is_nullish()",
                };
                format!(
                    "{{ {setup}let __left = {read}; if {keep} {{ __left }} else {{ let __value = {right}; {write}__value }} }}"
                )
            }
            (None, Some(binary)) => {
                let combined = self.binary_operation(binary, "&__left", "&__right");
                format!(
                    "{{ {setup}let __left = {read}; let __right = {right}; let __value = {combined}; {write}__value }}"
                )
            }
            // Identifier targets resolve before the right side; member
            // targets evaluate object and key first, as the language does.
            (None, None) => format!("{{ {setup}let __value = {right}; {write}__value }}"),
        }
    }

    /// An expression evaluated for its effects. Assignments and updates of
    /// locals write them in place instead of producing a value to discard.
    fn effect(&mut self, expression: &Expression<'_>) -> String {
        match expression {
            Expression::ParenthesizedExpression(inner) => self.effect(&inner.expression),
            Expression::AssignmentExpression(assignment) => {
                if let Some(SimpleAssignmentTarget::AssignmentTargetIdentifier(identifier)) =
                    assignment.left.as_simple_assignment_target()
                    && self.symbol(identifier).is_some()
                {
                    let (setup, read, write) = self.identifier_target(identifier);
                    let local = borrowed(&read).map(str::to_owned);
                    if let Some(local) = local
                        && setup.is_empty()
                        && write == format!("{local} = __value.clone();\n")
                    {
                        let operator = assignment.operator;
                        if operator == AssignmentOperator::Assign {
                            let right = self.named_expression(&assignment.right, &identifier.name);
                            return format!("{local} = {right};\n");
                        }
                        if let Some(binary) = operator.to_binary_operator() {
                            let right = self.expression(&assignment.right);
                            let borrowed_right = borrowed(&right).map(str::to_owned);
                            let (setup, right) = match borrowed_right {
                                Some(other) => (String::new(), format!("&{other}")),
                                None if literal(&assignment.right) => {
                                    (String::new(), format!("&{right}"))
                                }
                                None => (
                                    format!(
                                        "let __left = {local}.clone(); let __right = {right}; "
                                    ),
                                    "&__right".to_owned(),
                                ),
                            };
                            let left = if setup.is_empty() {
                                format!("&{local}")
                            } else {
                                "&__left".to_owned()
                            };
                            let combined = self.binary_operation(binary, &left, &right);
                            return if setup.is_empty() {
                                format!("{local} = {combined};\n")
                            } else {
                                format!("{local} = {{ {setup}{combined} }};\n")
                            };
                        }
                    }
                }
                format!("let _ = {};\n", self.expression(expression))
            }
            Expression::UpdateExpression(update) => {
                if let SimpleAssignmentTarget::AssignmentTargetIdentifier(identifier) =
                    &update.argument
                    && self.symbol(identifier).is_some()
                {
                    let (setup, read, write) = self.identifier_target(identifier);
                    let local = borrowed(&read).map(str::to_owned);
                    if let Some(local) = local
                        && setup.is_empty()
                        && write == format!("{local} = __value.clone();\n")
                    {
                        let step = match update.operator {
                            UpdateOperator::Increment => 1,
                            UpdateOperator::Decrement => -1,
                        };
                        let increment =
                            self.fail(&format!("ops::increment(realm, &{local}, {step})"));
                        return format!("{local} = {increment}.1;\n");
                    }
                }
                format!("let _ = {};\n", self.expression(expression))
            }
            other => format!("let _ = {};\n", self.expression(other)),
        }
    }

    fn update(&mut self, update: &UpdateExpression<'_>) -> String {
        let Some((setup, read, write)) = self.target(&update.argument) else {
            return "Value::Undefined".to_owned();
        };
        let step = match update.operator {
            UpdateOperator::Increment => 1,
            UpdateOperator::Decrement => -1,
        };
        let increment = self.fail(&format!("ops::increment(realm, &__old, {step})"));
        let result = if update.prefix {
            "__value"
        } else {
            "__numeric"
        };
        format!(
            "{{ {setup}let __old = {read}; let (__numeric, __value) = {increment}; {write}{result} }}"
        )
    }
}

/// A function body: statements, or an arrow's single expression.
enum Body<'r, 'a> {
    Block(&'r [Statement<'a>]),
    Expression(&'r Expression<'a>),
}

#[cfg(test)]
mod import_meta_tests {
    use super::import_meta_url;
    use std::path::Path;

    #[test]
    fn file_urls_escape_path_bytes_and_serialize_unc_authorities() {
        assert_eq!(
            import_meta_url(Path::new(r"\\server\share\Mark Text #?.js")),
            "file://server/share/Mark%20Text%20%23%3F.js"
        );
    }
}
