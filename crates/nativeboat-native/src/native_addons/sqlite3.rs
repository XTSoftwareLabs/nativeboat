//! `@vscode/sqlite3`'s binding (`vscode-sqlite3.node`), over the runtime's
//! own SQLite engine (`crate::sqlite`): the `Database` and `Statement`
//! classes of node-sqlite3, whose package JavaScript adds the rest.
//!
//! As in node-sqlite3, a database's work is queued and runs in order after
//! the call that asks for it, and callbacks follow with the result:
//! `Database(file, [mode], [callback])` opens the file (`""` and `:memory:`
//! are in memory) and emits `open`; `exec`, `close`, `wait`, `serialize`,
//! `parallelize` and `configure` act on it; a `Statement` prepared on it
//! binds positional or named values and runs with `run`, `get`, `all` and
//! `each`. Failures are `Error`s with SQLite's code and errno, given to the
//! callback or emitted as `error` without one. A database's file is
//! replaced whole when its changes commit; `Backup` is not available.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::sqlite::SqlValue;
use crate::sqlite::engine::{Database, Memory, Outcome, Params, SqlError, Storage, Stored};
use crate::sqlite::sql;
use crate::{ErrorKind, Key, Object, Property, Realm, Thrown, Value, ops};
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

mod system;

const OPEN_READONLY: u32 = 0x1;
const OPEN_READWRITE: u32 = 0x2;
const OPEN_CREATE: u32 = 0x4;
const OPEN_FULLMUTEX: u32 = 0x1_0000;

/// SQLite's result codes, by name.
const CODES: &[(&str, u32)] = &[
    ("OK", 0),
    ("ERROR", 1),
    ("INTERNAL", 2),
    ("PERM", 3),
    ("ABORT", 4),
    ("BUSY", 5),
    ("LOCKED", 6),
    ("NOMEM", 7),
    ("READONLY", 8),
    ("INTERRUPT", 9),
    ("IOERR", 10),
    ("CORRUPT", 11),
    ("NOTFOUND", 12),
    ("FULL", 13),
    ("CANTOPEN", 14),
    ("PROTOCOL", 15),
    ("EMPTY", 16),
    ("SCHEMA", 17),
    ("TOOBIG", 18),
    ("CONSTRAINT", 19),
    ("MISMATCH", 20),
    ("MISUSE", 21),
    ("NOLFS", 22),
    ("AUTH", 23),
    ("FORMAT", 24),
    ("RANGE", 25),
    ("NOTADB", 26),
];

/// A database file on disk, replaced whole through a temporary file.
struct File {
    path: String,
    readonly: bool,
}

impl File {
    fn wide(path: &str) -> Vec<u16> {
        path.encode_utf16().chain(core::iter::once(0)).collect()
    }
}

impl Storage for File {
    fn read(&mut self) -> Result<Option<Stored>, SqlError> {
        Ok(crate::platform::read_file(&self.path).map(|bytes| {
            let wal = crate::platform::read_file(&alloc::format!("{}-wal", self.path));
            (bytes, wal.filter(|wal| !wal.is_empty()))
        }))
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), SqlError> {
        if self.readonly {
            return Err(SqlError {
                code: "SQLITE_READONLY",
                message: "attempt to write a readonly database".into(),
            });
        }
        let temporary = alloc::format!("{}.nativeboat-journal", self.path);
        let io = || SqlError {
            code: "SQLITE_IOERR",
            message: "disk I/O error".into(),
        };
        if !crate::platform::write_file(&temporary, bytes) {
            return Err(io());
        }
        crate::fs_win::rename(&File::wide(&temporary), &File::wide(&self.path))
            .map_err(|_| io())?;
        // The file now holds every commit: a log SQLite left is spent.
        for suffix in ["-wal", "-shm"] {
            let _ =
                crate::fs_win::remove(&File::wide(&alloc::format!("{}{suffix}", self.path)), false);
        }
        Ok(())
    }
}

/// A connection's database: on the system's SQLite where Windows has a
/// recent one, else on the built-in engine.
enum Backend {
    System(system::Database),
    Engine(Database),
}

impl Backend {
    /// Every statement of `sql`, as `exec` runs them.
    fn execute(&mut self, sql: &str, params: &Params) -> Result<Outcome, SqlError> {
        match self {
            Backend::System(database) => database.execute(sql, params),
            Backend::Engine(database) => database.execute(sql, params),
        }
    }
}

type Operation = Box<dyn FnOnce(&mut Realm) -> Result<(), Thrown>>;

/// A database object's connection and its queue of work.
struct Connection {
    object: Object,
    database: Option<Backend>,
    queue: VecDeque<Operation>,
    scheduled: bool,
    trace: bool,
}

type Shared = Rc<RefCell<Connection>>;

/// Runs a connection's queued work in order.
fn schedule(realm: &mut Realm, connection: &Shared) {
    if connection.borrow().scheduled {
        return;
    }
    connection.borrow_mut().scheduled = true;
    let pump = connection.clone();
    realm.enqueue(Box::new(move |realm| {
        loop {
            let next = pump.borrow_mut().queue.pop_front();
            let Some(operation) = next else {
                break;
            };
            operation(realm)?;
        }
        pump.borrow_mut().scheduled = false;
        Ok(())
    }));
}

fn queue(realm: &mut Realm, connection: &Shared, operation: Operation) {
    connection.borrow_mut().queue.push_back(operation);
    schedule(realm, connection);
}

/// An `Error` as node-sqlite3 makes it: `CODE: message`, with `errno` and
/// `code`.
fn sql_error(realm: &mut Realm, found: &SqlError) -> Value {
    let error = realm.error(
        ErrorKind::Error,
        &alloc::format!("{}: {}", found.code, found.message),
    );
    let name = found.code.trim_start_matches("SQLITE_");
    let errno = CODES
        .iter()
        .find(|(known, _)| *known == name)
        .map_or(1, |(_, code)| *code);
    error.define(
        Key::name("errno"),
        Property::data(Value::Number(f64::from(errno))),
    );
    error.define(Key::name("code"), Property::data(Value::str(found.code)));
    Value::Object(error)
}

fn emit(realm: &mut Realm, target: &Object, event: &str, args: &[Value]) -> Result<(), Thrown> {
    let target = Value::Object(target.clone());
    let emit = ops::get_named(realm, &target, "emit")?;
    if !crate::array_builtin::is_callable(&emit) {
        return Ok(());
    }
    let mut arguments = alloc::vec![Value::str(event)];
    arguments.extend_from_slice(args);
    ops::call(realm, &emit, target, &arguments).map(drop)
}

/// Reports `result` to `callback`, or emits `error` on `target` when it
/// failed and there is no callback.
fn settle(
    realm: &mut Realm,
    target: &Object,
    callback: &Value,
    this: Value,
    result: Result<Vec<Value>, SqlError>,
) -> Result<(), Thrown> {
    match result {
        Ok(values) => {
            if crate::array_builtin::is_callable(callback) {
                let mut arguments = alloc::vec![Value::Null];
                arguments.extend(values);
                ops::call(realm, callback, this, &arguments)?;
            }
            Ok(())
        }
        Err(found) => {
            let error = sql_error(realm, &found);
            if crate::array_builtin::is_callable(callback) {
                ops::call(realm, callback, this, &[error]).map(drop)
            } else {
                emit(realm, target, "error", &[error])
            }
        }
    }
}

fn to_js(realm: &mut Realm, value: &SqlValue) -> Result<Value, Thrown> {
    Ok(match value {
        SqlValue::Null => Value::Null,
        SqlValue::Integer(integer) => Value::Number(*integer as f64),
        SqlValue::Real(real) => Value::Number(*real),
        SqlValue::Text(text) => Value::str(text),
        SqlValue::Blob(blob) => {
            if realm.buffer_prototype.is_none()
                && let Some(require) = realm.node_library
            {
                let _ = require(realm, "buffer")?;
            }
            let prototype = realm.buffer_prototype.clone();
            Value::Object(crate::typed_array_builtin::new_bytes(
                realm,
                blob.clone(),
                prototype,
            ))
        }
    })
}

fn from_js(realm: &mut Realm, value: &Value) -> Result<SqlValue, Thrown> {
    Ok(match value {
        Value::Null | Value::Undefined => SqlValue::Null,
        Value::Boolean(flag) => SqlValue::Integer(i64::from(*flag)),
        Value::Number(number) => {
            if crate::float::fract(*number) == 0.0 && number.abs() < 9_007_199_254_740_992.0 {
                SqlValue::Integer(*number as i64)
            } else {
                SqlValue::Real(*number)
            }
        }
        Value::String(text) => SqlValue::Text(text.to_rust_lossy()),
        Value::Object(_) => match crate::typed_array_builtin::view_bytes(value) {
            Some(bytes) => SqlValue::Blob(bytes),
            None => {
                // Dates bind as their time; other objects as their text.
                let time = ops::get_named(realm, value, "getTime")?;
                if crate::array_builtin::is_callable(&time) {
                    match ops::call(realm, &time, value.clone(), &[])? {
                        Value::Number(number) => SqlValue::Real(number),
                        other => SqlValue::Text(ops::to_string(realm, &other)?.to_rust_lossy()),
                    }
                } else {
                    SqlValue::Text(ops::to_string(realm, value)?.to_rust_lossy())
                }
            }
        },
        other => SqlValue::Text(ops::to_string(realm, other)?.to_rust_lossy()),
    })
}

/// Values given to a statement: a trailing callback apart, an array, an
/// object of names (`:a`, `$a`, `@a` or `a`), or the values themselves.
fn bindings(realm: &mut Realm, args: &[Value]) -> Result<(Option<Params>, Value), Thrown> {
    let (args, callback) = match args.last() {
        Some(last) if crate::array_builtin::is_callable(last) => {
            (&args[..args.len() - 1], last.clone())
        }
        _ => (args, Value::Undefined),
    };
    if args.is_empty() {
        return Ok((None, callback));
    }
    let mut params = Params::default();
    let single_object = args.len() == 1
        && matches!(&args[0], Value::Object(object) if crate::typed_array_builtin::view_bytes(&args[0]).is_none() && {
            let time = ops::get_named(realm, &args[0], "getTime")?;
            let _ = object;
            !crate::array_builtin::is_callable(&time)
        });
    if single_object {
        let Value::Object(object) = &args[0] else {
            unreachable!("checked above");
        };
        if object.is_array() {
            let length = match ops::get_named(realm, &args[0], "length")? {
                Value::Number(length) => length as u32,
                _ => 0,
            };
            for index in 0..length {
                let item = ops::get(realm, &args[0], &Key::Index(index))?;
                params.positional.push(from_js(realm, &item)?);
            }
        } else {
            for key in ops::own_property_keys(realm, object)? {
                let value = ops::get(realm, &args[0], &key)?;
                let value = from_js(realm, &value)?;
                match &key {
                    Key::Index(index) => {
                        let index = *index as usize;
                        if params.positional.len() < index {
                            params.positional.resize(index, SqlValue::Null);
                        }
                        if index >= 1 {
                            params.positional[index - 1] = value;
                        }
                    }
                    Key::String(name) => {
                        let name = name.to_rust_lossy();
                        let name = if name.starts_with([':', '$', '@']) {
                            name
                        } else {
                            alloc::format!(":{name}")
                        };
                        params.named.push((name, value));
                    }
                    Key::Symbol(_) => {}
                }
            }
        }
    } else {
        for value in args {
            params.positional.push(from_js(realm, value)?);
        }
    }
    Ok((Some(params), callback))
}

fn open_database(path: &str, mode: u32) -> Result<Backend, SqlError> {
    if let Some(opened) = system::Database::open(path, mode) {
        return opened.map(Backend::System);
    }
    open_engine_database(path, mode).map(Backend::Engine)
}

/// A database on the built-in engine, for systems without a recent SQLite.
fn open_engine_database(path: &str, mode: u32) -> Result<Database, SqlError> {
    if path.is_empty() || path == ":memory:" {
        return Database::open(Box::new(Memory));
    }
    let exists = crate::platform::read_file(path).is_some();
    if !exists && mode & OPEN_CREATE == 0 {
        return Err(SqlError {
            code: "SQLITE_CANTOPEN",
            message: "unable to open database file".into(),
        });
    }
    let readonly = mode & OPEN_READWRITE == 0 && mode & OPEN_READONLY != 0;
    Database::open(Box::new(File {
        path: path.into(),
        readonly,
    }))
}

fn method(
    realm: &Realm,
    target: &Object,
    name: &'static str,
    body: impl Fn(&mut Realm, Value, &[Value]) -> crate::Completion + 'static,
) {
    let function = realm.function(name, 0, false, body);
    target.define(Key::name(name), Property::hidden(Value::Object(function)));
}

/// The connections of database objects, by the id each object keeps.
type Registry = Rc<RefCell<Vec<Shared>>>;

const ID: &str = "__nativeboat_sqlite";

fn connection_of(
    realm: &mut Realm,
    registry: &Registry,
    database: &Value,
) -> Result<Shared, Thrown> {
    let id = match database {
        Value::Object(_) => ops::get_named(realm, database, ID)?,
        _ => Value::Undefined,
    };
    match id {
        Value::Number(id) => registry
            .borrow()
            .get(id as usize)
            .cloned()
            .ok_or_else(|| realm.throw(ErrorKind::Type, "Database object expected")),
        _ => Err(realm.throw(ErrorKind::Type, "Database object expected")),
    }
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or(Value::Undefined)
}

fn database_class(realm: &mut Realm, registry: &Registry) -> Object {
    let prototype = realm.object();
    let instance_prototype = prototype.clone();
    let connections = registry.clone();
    let constructor = realm.function("Database", 3, true, move |realm, _, args| {
        if matches!(realm.new_target(), Value::Undefined) {
            return Err(realm.throw(
                ErrorKind::Type,
                "Class constructors cannot be invoked without 'new'",
            ));
        }
        let Value::String(file) = arg(args, 0) else {
            return Err(realm.throw(ErrorKind::Type, "Filename must be a string"));
        };
        let file = file.to_rust_lossy();
        let (mode, callback) = match (arg(args, 1), arg(args, 2)) {
            (Value::Number(mode), callback) => (mode as u32, callback),
            (callback, _) => (OPEN_READWRITE | OPEN_CREATE | OPEN_FULLMUTEX, callback),
        };
        let object = realm.object();
        object.set_prototype(Some(instance_prototype.clone()));
        object.define(Key::name("filename"), Property::data(Value::str(&file)));
        object.define(
            Key::name("mode"),
            Property::data(Value::Number(f64::from(mode))),
        );
        object.define(Key::name("open"), Property::data(Value::Boolean(false)));
        let connection = Rc::new(RefCell::new(Connection {
            object: object.clone(),
            database: None,
            queue: VecDeque::new(),
            scheduled: false,
            trace: false,
        }));
        let id = {
            let mut known = connections.borrow_mut();
            known.push(connection.clone());
            known.len() - 1
        };
        object.define(Key::name(ID), Property::hidden(Value::Number(id as f64)));
        let opening = connection.clone();
        queue(
            realm,
            &connection,
            Box::new(move |realm| {
                let opened = open_database(&file, mode);
                let object = opening.borrow().object.clone();
                match opened {
                    Ok(database) => {
                        opening.borrow_mut().database = Some(database);
                        object.define(Key::name("open"), Property::data(Value::Boolean(true)));
                        if crate::array_builtin::is_callable(&callback) {
                            ops::call(
                                realm,
                                &callback,
                                Value::Object(object.clone()),
                                &[Value::Null],
                            )?;
                        }
                        emit(realm, &object, "open", &[])
                    }
                    Err(found) => settle(
                        realm,
                        &object,
                        &callback,
                        Value::Object(object.clone()),
                        Err(found),
                    ),
                }
            }),
        );
        add_database_methods(realm, &object, &connection);
        Ok(Value::Object(object))
    });
    constructor.define(
        Key::name("prototype"),
        Property::hidden(Value::Object(prototype.clone())),
    );
    prototype.define(
        Key::name("constructor"),
        Property::hidden(Value::Object(constructor.clone())),
    );
    constructor
}

fn add_database_methods(realm: &mut Realm, object: &Object, connection: &Shared) {
    let exec = connection.clone();
    method(realm, object, "exec", move |realm, this, args| {
        let sql = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
        let callback = arg(args, 1);
        let work = exec.clone();
        queue(
            realm,
            &exec,
            Box::new(move |realm| {
                let (object, traced) = {
                    let connection = work.borrow();
                    (connection.object.clone(), connection.trace)
                };
                if traced {
                    emit(realm, &object, "trace", &[Value::str(&sql)])?;
                }
                let result = match work.borrow_mut().database.as_mut() {
                    Some(database) => database
                        .execute(&sql, &Params::default())
                        .map(|_| Vec::new()),
                    None => Err(SqlError {
                        code: "SQLITE_MISUSE",
                        message: "Database is closed".into(),
                    }),
                };
                settle(
                    realm,
                    &object,
                    &callback,
                    Value::Object(object.clone()),
                    result,
                )
            }),
        );
        Ok(this)
    });
    let close = connection.clone();
    method(realm, object, "close", move |realm, this, args| {
        let callback = arg(args, 0);
        let work = close.clone();
        queue(
            realm,
            &close,
            Box::new(move |realm| {
                let object = work.borrow().object.clone();
                let open = work.borrow_mut().database.take().is_some();
                object.define(Key::name("open"), Property::data(Value::Boolean(false)));
                let result = if open {
                    Ok(Vec::new())
                } else {
                    Err(SqlError {
                        code: "SQLITE_MISUSE",
                        message: "Database is closed".into(),
                    })
                };
                let ok = result.is_ok();
                settle(
                    realm,
                    &object,
                    &callback,
                    Value::Object(object.clone()),
                    result,
                )?;
                if ok {
                    emit(realm, &object, "close", &[])?;
                }
                Ok(())
            }),
        );
        Ok(this)
    });
    let wait = connection.clone();
    method(realm, object, "wait", move |realm, this, args| {
        let callback = arg(args, 0);
        let work = wait.clone();
        queue(
            realm,
            &wait,
            Box::new(move |realm| {
                let object = work.borrow().object.clone();
                settle(
                    realm,
                    &object,
                    &callback,
                    Value::Object(object.clone()),
                    Ok(Vec::new()),
                )
            }),
        );
        Ok(this)
    });
    // Work always runs in order, as `serialize` asks.
    for name in ["serialize", "parallelize"] {
        method(realm, object, name, |realm, this, args| {
            let callback = arg(args, 0);
            if crate::array_builtin::is_callable(&callback) {
                ops::call(realm, &callback, this.clone(), &[])?;
            }
            Ok(this)
        });
    }
    let configure = connection.clone();
    method(realm, object, "configure", move |realm, this, args| {
        let option = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
        if option == "trace" {
            configure.borrow_mut().trace = matches!(arg(args, 1), Value::Boolean(true));
        }
        Ok(this)
    });
    method(realm, object, "interrupt", |_, this, _| Ok(this));
    method(realm, object, "loadExtension", |realm, this, args| {
        let callback = args.last().cloned().unwrap_or(Value::Undefined);
        let error = sql_error(
            realm,
            &SqlError {
                code: "SQLITE_ERROR",
                message: "extensions are not available".into(),
            },
        );
        if crate::array_builtin::is_callable(&callback) {
            ops::call(realm, &callback, this.clone(), &[error])?;
        }
        Ok(this)
    });
}

/// A prepared statement's state.
struct Prepared {
    /// The statement's text, which the system's SQLite prepares.
    sql: String,
    /// The built-in engine's parse of its first statement.
    parsed: Option<sql::Parsed>,
    params: Params,
    error: Option<SqlError>,
    finalized: bool,
}

fn statement_class(realm: &mut Realm, registry: &Registry) -> Object {
    let prototype = realm.object();
    let instance_prototype = prototype.clone();
    let connections = registry.clone();
    let constructor = realm.function("Statement", 3, true, move |realm, _, args| {
        if matches!(realm.new_target(), Value::Undefined) {
            return Err(realm.throw(
                ErrorKind::Type,
                "Class constructors cannot be invoked without 'new'",
            ));
        }
        let connection = connection_of(realm, &connections, &arg(args, 0))?;
        let sql = ops::to_string(realm, &arg(args, 1))?.to_rust_lossy();
        let error_back = arg(args, 2);
        let object = realm.object();
        object.set_prototype(Some(instance_prototype.clone()));
        object.define(Key::name("sql"), Property::data(Value::str(&sql)));
        let prepared = Rc::new(RefCell::new(Prepared {
            sql: sql.clone(),
            parsed: None,
            params: Params::default(),
            error: None,
            finalized: false,
        }));
        let preparing = prepared.clone();
        let target = object.clone();
        let opened = connection.clone();
        queue(
            realm,
            &connection,
            Box::new(move |realm| {
                // The first statement, as SQLite prepares it.
                let result = match opened.borrow_mut().database.as_mut() {
                    Some(Backend::System(database)) => database.check(&sql).map(|()| None),
                    _ => sql::parse(&sql)
                        .map_err(SqlError::from)
                        .map(|mut parsed| (!parsed.is_empty()).then(|| parsed.remove(0))),
                };
                match result {
                    Ok(parsed) => {
                        preparing.borrow_mut().parsed = parsed;
                        Ok(())
                    }
                    Err(found) => {
                        preparing.borrow_mut().error = Some(found.clone());
                        settle(
                            realm,
                            &target,
                            &error_back,
                            Value::Object(target.clone()),
                            Err(found),
                        )
                    }
                }
            }),
        );
        add_statement_methods(realm, &object, &connection, &prepared);
        Ok(Value::Object(object))
    });
    constructor.define(
        Key::name("prototype"),
        Property::hidden(Value::Object(prototype.clone())),
    );
    prototype.define(
        Key::name("constructor"),
        Property::hidden(Value::Object(constructor.clone())),
    );
    constructor
}

/// How a statement's rows are given back.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fetch {
    Run,
    Get,
    All,
    Each,
}

fn row_object(realm: &mut Realm, columns: &[String], row: &[SqlValue]) -> Result<Value, Thrown> {
    let object = realm.object();
    for (name, value) in columns.iter().zip(row) {
        let value = to_js(realm, value)?;
        object.define(Key::String(name.as_str().into()), Property::data(value));
    }
    Ok(Value::Object(object))
}

fn add_statement_methods(
    realm: &mut Realm,
    object: &Object,
    connection: &Shared,
    prepared: &Rc<RefCell<Prepared>>,
) {
    for (name, fetch) in [
        ("run", Fetch::Run),
        ("get", Fetch::Get),
        ("all", Fetch::All),
        ("each", Fetch::Each),
    ] {
        let connection = connection.clone();
        let prepared = prepared.clone();
        let target = object.clone();
        method(realm, object, name, move |realm, this, args| {
            // `each(params, row, complete)`: the row callback is not last.
            let (args, complete) = if fetch == Fetch::Each
                && args.len() >= 2
                && crate::array_builtin::is_callable(&args[args.len() - 1])
                && crate::array_builtin::is_callable(&args[args.len() - 2])
            {
                (&args[..args.len() - 1], args[args.len() - 1].clone())
            } else {
                (args, Value::Undefined)
            };
            let (bound, callback) = bindings(realm, args)?;
            let work = prepared.clone();
            let target = target.clone();
            let running = connection.clone();
            queue(
                realm,
                &connection,
                Box::new(move |realm| {
                    if let Some(bound) = bound {
                        work.borrow_mut().params = bound;
                    }
                    let result = run_statement(&running, &work);
                    match fetch {
                        Fetch::Run => {
                            let (this, result) = match result {
                                Ok(outcome) => {
                                    let context = realm.object();
                                    context.define(
                                        Key::name("lastID"),
                                        Property::data(Value::Number(
                                            outcome.last_insert_rowid as f64,
                                        )),
                                    );
                                    context.define(
                                        Key::name("changes"),
                                        Property::data(Value::Number(outcome.changes as f64)),
                                    );
                                    (Value::Object(context), Ok(Vec::new()))
                                }
                                Err(found) => (Value::Object(target.clone()), Err(found)),
                            };
                            settle(realm, &target, &callback, this, result)
                        }
                        Fetch::Get => {
                            let result = match result {
                                Ok(outcome) => match outcome.rows.first() {
                                    Some(row) => {
                                        Ok(alloc::vec![row_object(realm, &outcome.columns, row)?])
                                    }
                                    None => Ok(alloc::vec![Value::Undefined]),
                                },
                                Err(found) => Err(found),
                            };
                            settle(
                                realm,
                                &target,
                                &callback,
                                Value::Object(target.clone()),
                                result,
                            )
                        }
                        Fetch::All => {
                            let result = match result {
                                Ok(outcome) => {
                                    let mut rows = Vec::with_capacity(outcome.rows.len());
                                    for row in &outcome.rows {
                                        rows.push(row_object(realm, &outcome.columns, row)?);
                                    }
                                    Ok(alloc::vec![Value::Object(realm.array(rows))])
                                }
                                Err(found) => Err(found),
                            };
                            settle(
                                realm,
                                &target,
                                &callback,
                                Value::Object(target.clone()),
                                result,
                            )
                        }
                        Fetch::Each => match result {
                            Ok(outcome) => {
                                for row in &outcome.rows {
                                    let row = row_object(realm, &outcome.columns, row)?;
                                    settle(
                                        realm,
                                        &target,
                                        &callback,
                                        Value::Object(target.clone()),
                                        Ok(alloc::vec![row]),
                                    )?;
                                }
                                if crate::array_builtin::is_callable(&complete) {
                                    ops::call(
                                        realm,
                                        &complete,
                                        Value::Object(target.clone()),
                                        &[Value::Null, Value::Number(outcome.rows.len() as f64)],
                                    )?;
                                }
                                Ok(())
                            }
                            Err(found) => {
                                let callback = if crate::array_builtin::is_callable(&complete) {
                                    complete
                                } else {
                                    callback
                                };
                                settle(
                                    realm,
                                    &target,
                                    &callback,
                                    Value::Object(target.clone()),
                                    Err(found),
                                )
                            }
                        },
                    }
                }),
            );
            Ok(this)
        });
    }
    let binder = prepared.clone();
    let bind_connection = connection.clone();
    let bind_target = object.clone();
    method(realm, object, "bind", move |realm, this, args| {
        let (bound, callback) = bindings(realm, args)?;
        let work = binder.clone();
        let target = bind_target.clone();
        queue(
            realm,
            &bind_connection,
            Box::new(move |realm| {
                work.borrow_mut().params = bound.unwrap_or_default();
                settle(
                    realm,
                    &target,
                    &callback,
                    Value::Object(target.clone()),
                    Ok(Vec::new()),
                )
            }),
        );
        Ok(this)
    });
    for name in ["reset", "finalize"] {
        let finishing = prepared.clone();
        let connection = connection.clone();
        let target = object.clone();
        method(realm, object, name, move |realm, this, args| {
            let callback = arg(args, 0);
            let work = finishing.clone();
            let target = target.clone();
            queue(
                realm,
                &connection,
                Box::new(move |realm| {
                    if name == "finalize" {
                        work.borrow_mut().finalized = true;
                    }
                    if crate::array_builtin::is_callable(&callback) {
                        ops::call(
                            realm,
                            &callback,
                            Value::Object(target.clone()),
                            &[Value::Null],
                        )?;
                    }
                    Ok(())
                }),
            );
            Ok(this)
        });
    }
}

fn run_statement(
    connection: &Shared,
    prepared: &Rc<RefCell<Prepared>>,
) -> Result<Outcome, SqlError> {
    let state = prepared.borrow();
    if let Some(found) = &state.error {
        return Err(found.clone());
    }
    if state.finalized {
        return Err(SqlError {
            code: "SQLITE_MISUSE",
            message: "Statement is already finalized".into(),
        });
    }
    let mut connection = connection.borrow_mut();
    match connection.database.as_mut() {
        Some(Backend::System(_)) if state.sql.trim().is_empty() => Ok(Outcome::default()),
        Some(Backend::System(database)) => database.run_first(&state.sql, &state.params),
        Some(Backend::Engine(database)) => match &state.parsed {
            Some(parsed) => database.run(parsed, &state.params),
            None => Ok(Outcome::default()),
        },
        None => Err(SqlError {
            code: "SQLITE_MISUSE",
            message: "Database is closed".into(),
        }),
    }
}

pub(crate) fn module(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let registry: Registry = Rc::default();
    let database = database_class(realm, &registry);
    exports.define(
        Key::name("Database"),
        Property::data(Value::Object(database)),
    );
    let statement = statement_class(realm, &registry);
    exports.define(
        Key::name("Statement"),
        Property::data(Value::Object(statement)),
    );
    let backup = realm.function("Backup", 0, true, |realm, _, _| {
        Err(realm.throw(
            ErrorKind::Error,
            "Database backups are not available in native applications",
        ))
    });
    let backup_prototype = realm.object();
    backup.define(
        Key::name("prototype"),
        Property::hidden(Value::Object(backup_prototype)),
    );
    exports.define(Key::name("Backup"), Property::data(Value::Object(backup)));
    for (name, value) in [
        ("OPEN_READONLY", OPEN_READONLY),
        ("OPEN_READWRITE", OPEN_READWRITE),
        ("OPEN_CREATE", OPEN_CREATE),
        ("OPEN_FULLMUTEX", OPEN_FULLMUTEX),
        ("OPEN_URI", 0x40),
        ("OPEN_SHAREDCACHE", 0x2_0000),
        ("OPEN_PRIVATECACHE", 0x4_0000),
    ] {
        exports.define(
            Key::name(name),
            Property::data(Value::Number(f64::from(value))),
        );
    }
    for (name, code) in CODES {
        exports.define(
            Key::String((*name).into()),
            Property::data(Value::Number(f64::from(*code))),
        );
    }
    exports.define(Key::name("VERSION"), Property::data(Value::str("3.45.0")));
    exports.define(
        Key::name("VERSION_NUMBER"),
        Property::data(Value::Number(3_045_000.0)),
    );
    exports.define(
        Key::name("SOURCE_ID"),
        Property::data(Value::str("nativeboat")),
    );
    exports
}
