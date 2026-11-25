//! The system's SQLite: `winsqlite3.dll`, which Windows 10 and later keep
//! in System32. Databases opened on it run every statement and file format
//! of the real library: joins, subqueries, JSON functions, window
//! functions and the rest. It loads when first used; where it is missing or
//! older than 3.38 (the first with JSON built in), the built-in engine
//! serves databases instead.

use crate::sqlite::SqlValue;
use crate::sqlite::engine::{Outcome, Params, SqlError};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

type Handle = *mut c_void;

const ROW: i32 = 100;
const DONE: i32 = 101;
const INTEGER: i32 = 1;
const FLOAT: i32 = 2;
const TEXT: i32 = 3;
const BLOB: i32 = 4;
/// `SQLITE_TRANSIENT`: SQLite copies a bound value before the call returns.
const TRANSIENT: isize = -1;

/// SQLite's primary result codes, by number.
const NAMES: [&str; 27] = [
    "SQLITE_OK",
    "SQLITE_ERROR",
    "SQLITE_INTERNAL",
    "SQLITE_PERM",
    "SQLITE_ABORT",
    "SQLITE_BUSY",
    "SQLITE_LOCKED",
    "SQLITE_NOMEM",
    "SQLITE_READONLY",
    "SQLITE_INTERRUPT",
    "SQLITE_IOERR",
    "SQLITE_CORRUPT",
    "SQLITE_NOTFOUND",
    "SQLITE_FULL",
    "SQLITE_CANTOPEN",
    "SQLITE_PROTOCOL",
    "SQLITE_EMPTY",
    "SQLITE_SCHEMA",
    "SQLITE_TOOBIG",
    "SQLITE_CONSTRAINT",
    "SQLITE_MISMATCH",
    "SQLITE_MISUSE",
    "SQLITE_NOLFS",
    "SQLITE_AUTH",
    "SQLITE_FORMAT",
    "SQLITE_RANGE",
    "SQLITE_NOTADB",
];

/// The functions of `winsqlite3.dll` the binding calls.
struct Api {
    open_v2: unsafe extern "system" fn(*const u8, *mut Handle, i32, *const u8) -> i32,
    close_v2: unsafe extern "system" fn(Handle) -> i32,
    errmsg: unsafe extern "system" fn(Handle) -> *const u8,
    prepare_v2:
        unsafe extern "system" fn(Handle, *const u8, i32, *mut Handle, *mut *const u8) -> i32,
    finalize: unsafe extern "system" fn(Handle) -> i32,
    bind_parameter_index: unsafe extern "system" fn(Handle, *const u8) -> i32,
    bind_null: unsafe extern "system" fn(Handle, i32) -> i32,
    bind_int64: unsafe extern "system" fn(Handle, i32, i64) -> i32,
    bind_double: unsafe extern "system" fn(Handle, i32, f64) -> i32,
    bind_text: unsafe extern "system" fn(Handle, i32, *const u8, i32, isize) -> i32,
    bind_blob: unsafe extern "system" fn(Handle, i32, *const c_void, i32, isize) -> i32,
    step: unsafe extern "system" fn(Handle) -> i32,
    column_count: unsafe extern "system" fn(Handle) -> i32,
    column_name: unsafe extern "system" fn(Handle, i32) -> *const u8,
    column_type: unsafe extern "system" fn(Handle, i32) -> i32,
    column_int64: unsafe extern "system" fn(Handle, i32) -> i64,
    column_double: unsafe extern "system" fn(Handle, i32) -> f64,
    column_text: unsafe extern "system" fn(Handle, i32) -> *const u8,
    column_blob: unsafe extern "system" fn(Handle, i32) -> *const c_void,
    column_bytes: unsafe extern "system" fn(Handle, i32) -> i32,
    changes: unsafe extern "system" fn(Handle) -> i32,
    last_insert_rowid: unsafe extern "system" fn(Handle) -> i64,
}

static API: AtomicPtr<Api> = AtomicPtr::new(core::ptr::null_mut());
static UNAVAILABLE: AtomicBool = AtomicBool::new(false);

/// The system's SQLite, loaded once; `None` where it is missing or old.
fn api() -> Option<&'static Api> {
    let loaded = API.load(Ordering::Acquire);
    if !loaded.is_null() {
        // SAFETY: a published `Api` is leaked and never freed.
        return Some(unsafe { &*loaded });
    }
    if UNAVAILABLE.load(Ordering::Acquire) {
        return None;
    }
    match load() {
        Some(api) => {
            let leaked: &'static Api = Box::leak(Box::new(api));
            API.store(core::ptr::from_ref(leaked).cast_mut(), Ordering::Release);
            Some(leaked)
        }
        None => {
            UNAVAILABLE.store(true, Ordering::Release);
            None
        }
    }
}

// Each export has the signature of the field it fills, as SQLite's C API
// declares it.
#[allow(clippy::missing_transmute_annotations)]
fn load() -> Option<Api> {
    macro_rules! function {
        ($name:literal) => {{
            let procedure = crate::platform::system_procedure("winsqlite3.dll", $name);
            if procedure.is_null() {
                return None;
            }
            // SAFETY: the export has the field's signature (see above).
            unsafe { core::mem::transmute(procedure) }
        }};
    }
    let version: unsafe extern "system" fn() -> i32 = function!(c"sqlite3_libversion_number");
    // SAFETY: the function takes no arguments and only reads a constant.
    if unsafe { version() } < 3_038_000 {
        return None;
    }
    Some(Api {
        open_v2: function!(c"sqlite3_open_v2"),
        close_v2: function!(c"sqlite3_close_v2"),
        errmsg: function!(c"sqlite3_errmsg"),
        prepare_v2: function!(c"sqlite3_prepare_v2"),
        finalize: function!(c"sqlite3_finalize"),
        bind_parameter_index: function!(c"sqlite3_bind_parameter_index"),
        bind_null: function!(c"sqlite3_bind_null"),
        bind_int64: function!(c"sqlite3_bind_int64"),
        bind_double: function!(c"sqlite3_bind_double"),
        bind_text: function!(c"sqlite3_bind_text"),
        bind_blob: function!(c"sqlite3_bind_blob"),
        step: function!(c"sqlite3_step"),
        column_count: function!(c"sqlite3_column_count"),
        column_name: function!(c"sqlite3_column_name"),
        column_type: function!(c"sqlite3_column_type"),
        column_int64: function!(c"sqlite3_column_int64"),
        column_double: function!(c"sqlite3_column_double"),
        column_text: function!(c"sqlite3_column_text"),
        column_blob: function!(c"sqlite3_column_blob"),
        column_bytes: function!(c"sqlite3_column_bytes"),
        changes: function!(c"sqlite3_changes"),
        last_insert_rowid: function!(c"sqlite3_last_insert_rowid"),
    })
}

/// The UTF-8 text of a nul-terminated string SQLite returned.
fn text(pointer: *const u8) -> String {
    if pointer.is_null() {
        return String::new();
    }
    // SAFETY: SQLite returns nul-terminated strings valid until the next
    // call on the same object, and the text is copied at once.
    let bytes = unsafe { core::ffi::CStr::from_ptr(pointer.cast()) }.to_bytes();
    String::from_utf8_lossy(bytes).into_owned()
}

/// A connection to a database on the system's SQLite.
pub(crate) struct Database {
    api: &'static Api,
    handle: Handle,
}

impl Drop for Database {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: the handle is this connection's, closed once; any
            // statement left is finalized by `close_v2` when it ends.
            unsafe { (self.api.close_v2)(self.handle) };
        }
    }
}

impl Database {
    /// Opens `path` (`:memory:` or an empty path for a private one) with
    /// SQLite's open flags; `None` when the system has no usable SQLite.
    pub(crate) fn open(path: &str, flags: u32) -> Option<Result<Database, SqlError>> {
        let api = api()?;
        let mut name = Vec::with_capacity(path.len() + 1);
        name.extend_from_slice(path.as_bytes());
        name.push(0);
        let mut handle: Handle = core::ptr::null_mut();
        // SAFETY: the name is nul-terminated UTF-8 and `handle` is writable.
        let code =
            unsafe { (api.open_v2)(name.as_ptr(), &mut handle, flags as i32, core::ptr::null()) };
        let database = Database { api, handle };
        if code != 0 {
            return Some(Err(database.error(code)));
        }
        Some(Ok(database))
    }

    fn error(&self, code: i32) -> SqlError {
        let name = NAMES
            .get((code & 0xff) as usize)
            .copied()
            .unwrap_or("SQLITE_ERROR");
        let message = if self.handle.is_null() {
            String::from("out of memory")
        } else {
            // SAFETY: the handle is open; the message is copied at once.
            text(unsafe { (self.api.errmsg)(self.handle) })
        };
        SqlError {
            code: name,
            message,
        }
    }

    /// Every statement of `sql` in order, as `exec` runs them: the last
    /// one's rows.
    pub(crate) fn execute(&mut self, sql: &str, params: &Params) -> Result<Outcome, SqlError> {
        self.statements(sql, params, true)
    }

    /// The first statement of `sql`, as a prepared statement runs.
    pub(crate) fn run_first(&mut self, sql: &str, params: &Params) -> Result<Outcome, SqlError> {
        self.statements(sql, params, false)
    }

    /// Whether the first statement of `sql` prepares: its syntax error, or
    /// one naming a missing table or column, as SQLite reports when it
    /// prepares a statement.
    pub(crate) fn check(&mut self, sql: &str) -> Result<(), SqlError> {
        let mut text = Vec::with_capacity(sql.len() + 1);
        text.extend_from_slice(sql.as_bytes());
        text.push(0);
        let mut statement: Handle = core::ptr::null_mut();
        // SAFETY: the text is nul-terminated and outlives the call.
        let code = unsafe {
            (self.api.prepare_v2)(
                self.handle,
                text.as_ptr(),
                sql.len() as i32,
                &mut statement,
                core::ptr::null_mut(),
            )
        };
        if !statement.is_null() {
            // SAFETY: the statement was prepared above and is finalized once.
            unsafe { (self.api.finalize)(statement) };
        }
        if code != 0 {
            return Err(self.error(code));
        }
        Ok(())
    }

    fn statements(&mut self, sql: &str, params: &Params, all: bool) -> Result<Outcome, SqlError> {
        let mut text = Vec::with_capacity(sql.len() + 1);
        text.extend_from_slice(sql.as_bytes());
        text.push(0);
        let start = text.as_ptr();
        let mut at = 0_usize;
        let mut outcome = Outcome::default();
        while at < sql.len() {
            let mut statement: Handle = core::ptr::null_mut();
            let mut tail: *const u8 = core::ptr::null();
            // SAFETY: `at` is within the nul-terminated text, which outlives
            // the statement; `tail` points into it.
            let code = unsafe {
                (self.api.prepare_v2)(
                    self.handle,
                    start.add(at),
                    (sql.len() - at) as i32,
                    &mut statement,
                    &mut tail,
                )
            };
            if code != 0 {
                return Err(self.error(code));
            }
            at = if tail.is_null() {
                sql.len()
            } else {
                // SAFETY: SQLite's tail points into the same text.
                (unsafe { tail.offset_from(start) }) as usize
            };
            if statement.is_null() {
                // Only whitespace or a comment was left.
                continue;
            }
            let result = self.step_all(statement, params);
            // SAFETY: the statement was prepared above and is finalized once.
            unsafe { (self.api.finalize)(statement) };
            outcome = result?;
            if !all {
                break;
            }
        }
        Ok(outcome)
    }

    /// Binds `params` to a prepared statement and steps it to its end.
    fn step_all(&mut self, statement: Handle, params: &Params) -> Result<Outcome, SqlError> {
        for (index, value) in params.positional.iter().enumerate() {
            self.bind(statement, index as i32 + 1, value)?;
        }
        for (name, value) in &params.named {
            let bare = name.trim_start_matches([':', '@', '$']);
            let index = [name.clone()]
                .into_iter()
                .chain([':', '@', '$'].map(|prefix| alloc::format!("{prefix}{bare}")))
                .find_map(|candidate| {
                    let mut candidate = candidate.into_bytes();
                    candidate.push(0);
                    // SAFETY: the statement is live and the name terminated.
                    let index =
                        unsafe { (self.api.bind_parameter_index)(statement, candidate.as_ptr()) };
                    (index > 0).then_some(index)
                });
            if let Some(index) = index {
                self.bind(statement, index, value)?;
            }
        }
        // SAFETY: the statement is live for the reads below.
        let count = unsafe { (self.api.column_count)(statement) };
        let columns = (0..count)
            // SAFETY: as above, for each column index below the count.
            .map(|column| text(unsafe { (self.api.column_name)(statement, column) }))
            .collect::<Vec<_>>();
        let mut rows = Vec::new();
        loop {
            // SAFETY: the statement is live.
            match unsafe { (self.api.step)(statement) } {
                ROW => rows.push(
                    (0..count)
                        .map(|column| self.column(statement, column))
                        .collect(),
                ),
                DONE => break,
                code => return Err(self.error(code)),
            }
        }
        Ok(Outcome {
            columns,
            rows,
            // SAFETY: the connection is open.
            changes: unsafe { (self.api.changes)(self.handle) }.max(0) as usize,
            // SAFETY: the connection is open.
            last_insert_rowid: unsafe { (self.api.last_insert_rowid)(self.handle) },
        })
    }

    fn bind(&self, statement: Handle, index: i32, value: &SqlValue) -> Result<(), SqlError> {
        // SAFETY: the statement is live; SQLite copies text and blobs
        // (`SQLITE_TRANSIENT`) before the calls return.
        let code = unsafe {
            match value {
                SqlValue::Null => (self.api.bind_null)(statement, index),
                SqlValue::Integer(value) => (self.api.bind_int64)(statement, index, *value),
                SqlValue::Real(value) => (self.api.bind_double)(statement, index, *value),
                SqlValue::Text(text) => (self.api.bind_text)(
                    statement,
                    index,
                    text.as_ptr(),
                    text.len() as i32,
                    TRANSIENT,
                ),
                SqlValue::Blob(bytes) => (self.api.bind_blob)(
                    statement,
                    index,
                    bytes.as_ptr().cast(),
                    bytes.len() as i32,
                    TRANSIENT,
                ),
            }
        };
        if code != 0 {
            return Err(self.error(code));
        }
        Ok(())
    }

    fn column(&self, statement: Handle, column: i32) -> SqlValue {
        // SAFETY: the statement is on a row and `column` is below its
        // column count; values are copied before the next step.
        unsafe {
            match (self.api.column_type)(statement, column) {
                INTEGER => SqlValue::Integer((self.api.column_int64)(statement, column)),
                FLOAT => SqlValue::Real((self.api.column_double)(statement, column)),
                TEXT => {
                    let pointer = (self.api.column_text)(statement, column);
                    let length = (self.api.column_bytes)(statement, column).max(0) as usize;
                    if pointer.is_null() {
                        SqlValue::Text(String::new())
                    } else {
                        let bytes = core::slice::from_raw_parts(pointer, length);
                        SqlValue::Text(String::from_utf8_lossy(bytes).into_owned())
                    }
                }
                BLOB => {
                    let pointer = (self.api.column_blob)(statement, column);
                    let length = (self.api.column_bytes)(statement, column).max(0) as usize;
                    if pointer.is_null() {
                        SqlValue::Blob(Vec::new())
                    } else {
                        SqlValue::Blob(
                            core::slice::from_raw_parts(pointer.cast::<u8>(), length).to_vec(),
                        )
                    }
                }
                _ => SqlValue::Null,
            }
        }
    }
}

#[cfg(test)]
mod tests;
