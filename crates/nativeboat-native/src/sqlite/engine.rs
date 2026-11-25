//! A database: its tables and indexes in memory, the statements of [`sql`]
//! run over them as SQLite runs them, and its file read and written with
//! [`format`].
//!
//! Changes outside a transaction are written when each statement ends; a
//! transaction's when it commits, and `ROLLBACK` restores the state it
//! began with. Writing replaces the whole file (see `Storage`). Tables the
//! engine cannot run statements on (`WITHOUT ROWID` tables, or definitions
//! it cannot parse) are kept and written back unchanged.

use super::format::{self, Content, FormatError, Header, Object, SchemaEntry};
use super::sql::{
    self, ColumnDef, Conflict, Expr, OnConflict, Parameter, ResultColumn, Select, Statement,
};
use super::{SqlValue, compare_records};
#[allow(unused_imports)]
use crate::prelude::*;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Ordering;

/// An error a statement reports: SQLite's result code name and message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SqlError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

fn error(message: impl Into<String>) -> SqlError {
    SqlError {
        code: "SQLITE_ERROR",
        message: message.into(),
    }
}

fn constraint(message: impl Into<String>) -> SqlError {
    SqlError {
        code: "SQLITE_CONSTRAINT",
        message: message.into(),
    }
}

impl From<FormatError> for SqlError {
    fn from(found: FormatError) -> Self {
        match found {
            FormatError::NotADatabase => SqlError {
                code: "SQLITE_NOTADB",
                message: "file is not a database".into(),
            },
            FormatError::Corrupt(why) => SqlError {
                code: "SQLITE_CORRUPT",
                message: alloc::format!("database disk image is malformed: {why}"),
            },
            FormatError::Unsupported(what) => error(alloc::format!("{what} are not supported")),
        }
    }
}

impl From<sql::SyntaxError> for SqlError {
    fn from(found: sql::SyntaxError) -> Self {
        error(found.0)
    }
}

/// A database file's bytes and its write-ahead log's, if it has one.
pub(crate) type Stored = (Vec<u8>, Option<Vec<u8>>);

/// Where a database's file lives.
pub(crate) trait Storage {
    /// The file's bytes and its write-ahead log's, if they exist.
    fn read(&mut self) -> Result<Option<Stored>, SqlError>;
    /// Replaces the file with `bytes`, leaving no write-ahead log.
    fn write(&mut self, bytes: &[u8]) -> Result<(), SqlError>;
}

/// A database that lives in memory only.
pub(crate) struct Memory;

impl Storage for Memory {
    fn read(&mut self) -> Result<Option<Stored>, SqlError> {
        Ok(None)
    }

    fn write(&mut self, _: &[u8]) -> Result<(), SqlError> {
        Ok(())
    }
}

/// A column's type affinity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Affinity {
    Integer,
    Text,
    Blob,
    Real,
    Numeric,
}

/// The affinity SQLite gives a declared type.
fn affinity(declared: &str) -> Affinity {
    let upper = declared.to_ascii_uppercase();
    if upper.contains("INT") {
        Affinity::Integer
    } else if upper.contains("CHAR") || upper.contains("CLOB") || upper.contains("TEXT") {
        Affinity::Text
    } else if upper.contains("BLOB") || upper.is_empty() {
        Affinity::Blob
    } else if upper.contains("REAL") || upper.contains("FLOA") || upper.contains("DOUB") {
        Affinity::Real
    } else {
        Affinity::Numeric
    }
}

/// Text as a number, if all of it is one.
fn numeric_text(text: &str) -> Option<SqlValue> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(integer) = trimmed.parse::<i64>() {
        return Some(SqlValue::Integer(integer));
    }
    let real = trimmed.parse::<f64>().ok()?;
    if !real.is_finite() && !trimmed.to_ascii_lowercase().contains("inf") {
        return None;
    }
    Some(SqlValue::Real(real))
}

/// A value stored in a column of `affinity`.
fn apply_affinity(value: SqlValue, affinity: Affinity) -> SqlValue {
    match (affinity, value) {
        (Affinity::Text, SqlValue::Integer(integer)) => SqlValue::Text(integer.to_string()),
        (Affinity::Text, SqlValue::Real(real)) => SqlValue::Text(real_text(real)),
        (Affinity::Integer | Affinity::Numeric, SqlValue::Text(text)) => {
            match numeric_text(&text) {
                Some(SqlValue::Real(real))
                    if crate::float::fract(real) == 0.0 && real.abs() < 9.2e18 =>
                {
                    SqlValue::Integer(real as i64)
                }
                Some(number) => number,
                None => SqlValue::Text(text),
            }
        }
        (Affinity::Integer | Affinity::Numeric, SqlValue::Real(real))
            if crate::float::fract(real) == 0.0 && real.abs() < 9.2e18 =>
        {
            SqlValue::Integer(real as i64)
        }
        (Affinity::Real, SqlValue::Integer(integer)) => SqlValue::Real(integer as f64),
        (Affinity::Real, SqlValue::Text(text)) => match numeric_text(&text) {
            Some(SqlValue::Integer(integer)) => SqlValue::Real(integer as f64),
            Some(number) => number,
            None => SqlValue::Text(text),
        },
        (_, value) => value,
    }
}

/// A real as SQLite prints it: with a decimal point.
fn real_text(real: f64) -> String {
    if crate::float::fract(real) == 0.0 && real.abs() < 1e15 {
        alloc::format!("{real:.1}")
    } else {
        alloc::format!("{real}")
    }
}

/// A value as text, as SQLite casts it.
fn as_text(value: &SqlValue) -> Option<String> {
    match value {
        SqlValue::Null => None,
        SqlValue::Integer(integer) => Some(integer.to_string()),
        SqlValue::Real(real) => Some(real_text(*real)),
        SqlValue::Text(text) => Some(text.clone()),
        SqlValue::Blob(blob) => Some(String::from_utf8_lossy(blob).into_owned()),
    }
}

/// A value as a number for arithmetic: text and blobs by their numeric
/// prefix, as SQLite converts them.
fn as_number(value: &SqlValue) -> SqlValue {
    match value {
        SqlValue::Integer(_) | SqlValue::Real(_) | SqlValue::Null => value.clone(),
        SqlValue::Text(_) | SqlValue::Blob(_) => {
            let text = as_text(value).unwrap_or_default();
            let trimmed = text.trim_start();
            let mut end = 0;
            let bytes = trimmed.as_bytes();
            if end < bytes.len() && matches!(bytes[end], b'+' | b'-') {
                end += 1;
            }
            while end < bytes.len()
                && (bytes[end].is_ascii_digit()
                    || bytes[end] == b'.'
                    || bytes[end] == b'e'
                    || bytes[end] == b'E')
            {
                end += 1;
            }
            let mut prefix = &trimmed[..end];
            loop {
                if let Some(number) = numeric_text(prefix) {
                    return number;
                }
                if prefix.is_empty() {
                    return SqlValue::Integer(0);
                }
                prefix = &prefix[..prefix.len() - 1];
            }
        }
    }
}

fn truth(value: &SqlValue) -> Option<bool> {
    match as_number(value) {
        SqlValue::Null => None,
        SqlValue::Integer(integer) => Some(integer != 0),
        SqlValue::Real(real) => Some(real != 0.0),
        _ => Some(false),
    }
}

fn boolean(value: Option<bool>) -> SqlValue {
    value.map_or(SqlValue::Null, |value| SqlValue::Integer(i64::from(value)))
}

/// A key of an index, ordered as SQLite orders its records.
#[derive(Debug, Clone)]
struct Key(Vec<SqlValue>);

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Key {}

impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Key {
    fn cmp(&self, other: &Self) -> Ordering {
        compare_records(&self.0, &other.0)
    }
}

#[derive(Debug, Clone)]
struct Column {
    name: String,
    declared: String,
    affinity: Affinity,
    not_null: bool,
    default: Option<Expr>,
}

#[derive(Debug, Clone)]
struct Index {
    name: String,
    /// The `CREATE INDEX` statement; `None` for the automatic index of a
    /// table's `UNIQUE` or `PRIMARY KEY` constraint.
    sql: Option<String>,
    columns: Vec<usize>,
    unique: bool,
    conflict: Conflict,
    /// For unique indexes, the rowid of each key without NULLs.
    keys: BTreeMap<Key, i64>,
}

#[derive(Debug, Clone)]
struct Table {
    name: String,
    sql: String,
    columns: Vec<Column>,
    /// The column that is the rowid: `INTEGER PRIMARY KEY`.
    rowid_column: Option<usize>,
    autoincrement: bool,
    rows: BTreeMap<i64, Vec<SqlValue>>,
    indexes: Vec<Index>,
}

impl Table {
    fn column(&self, name: &str) -> Option<usize> {
        self.columns
            .iter()
            .position(|column| column.name.eq_ignore_ascii_case(name))
    }

    fn key(&self, index: &Index, row: &[SqlValue]) -> Key {
        Key(index
            .columns
            .iter()
            .map(|column| row[*column].clone())
            .collect())
    }

    /// The rows another row's keys conflict with in unique indexes, by
    /// index, ignoring `except`.
    fn conflicts(&self, row: &[SqlValue], except: Option<i64>) -> Vec<(usize, i64)> {
        let mut found = Vec::new();
        for (position, index) in self.indexes.iter().enumerate() {
            if !index.unique {
                continue;
            }
            let key = self.key(index, row);
            if key.0.iter().any(|value| matches!(value, SqlValue::Null)) {
                continue;
            }
            if let Some(rowid) = index.keys.get(&key)
                && Some(*rowid) != except
            {
                found.push((position, *rowid));
            }
        }
        found
    }

    fn insert_row(&mut self, rowid: i64, row: Vec<SqlValue>) {
        for position in 0..self.indexes.len() {
            if self.indexes[position].unique {
                let key = self.key(&self.indexes[position], &row);
                if !key.0.iter().any(|value| matches!(value, SqlValue::Null)) {
                    self.indexes[position].keys.insert(key, rowid);
                }
            }
        }
        self.rows.insert(rowid, row);
    }

    fn remove_row(&mut self, rowid: i64) -> Option<Vec<SqlValue>> {
        let row = self.rows.remove(&rowid)?;
        for position in 0..self.indexes.len() {
            if self.indexes[position].unique {
                let key = self.key(&self.indexes[position], &row);
                if self.indexes[position].keys.get(&key) == Some(&rowid) {
                    self.indexes[position].keys.remove(&key);
                }
            }
        }
        Some(row)
    }

    fn constraint_name(&self, index: &Index) -> String {
        index
            .columns
            .iter()
            .map(|column| alloc::format!("{}.{}", self.name, self.columns[*column].name))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// A table or index the engine keeps as the file has it.
#[derive(Debug, Clone)]
struct Opaque {
    entry: SchemaEntry,
    rows: Vec<(i64, Vec<SqlValue>)>,
    keys: Vec<Vec<SqlValue>>,
}

#[derive(Debug, Clone, Default)]
struct State {
    header: Header,
    tables: Vec<Table>,
    /// Views, triggers, and tables and indexes kept as they are.
    others: Vec<Opaque>,
}

/// What a statement gives back.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Outcome {
    pub(crate) columns: Vec<String>,
    pub(crate) rows: Vec<Vec<SqlValue>>,
    /// Rows inserted, updated or deleted.
    pub(crate) changes: usize,
    pub(crate) last_insert_rowid: i64,
}

/// Values bound to a statement's parameters.
#[derive(Debug, Clone, Default)]
pub(crate) struct Params {
    pub(crate) positional: Vec<SqlValue>,
    pub(crate) named: Vec<(String, SqlValue)>,
}

impl Params {
    fn get(&self, parameter: &Parameter) -> SqlValue {
        match parameter {
            Parameter::Numbered(index) => self
                .positional
                .get(index.wrapping_sub(1))
                .cloned()
                .unwrap_or(SqlValue::Null),
            Parameter::Named(name) => self
                .named
                .iter()
                .find(|(known, _)| {
                    known == name || known.get(1..) == Some(name.get(1..).unwrap_or_default())
                })
                .map_or(SqlValue::Null, |(_, value)| value.clone()),
            Parameter::Next => SqlValue::Null,
        }
    }
}

/// The values an expression sees: a table's row, and the row an upsert
/// proposes as `excluded`.
struct Scope<'a> {
    table: Option<&'a Table>,
    rowid: i64,
    row: Option<&'a [SqlValue]>,
    excluded: Option<&'a [SqlValue]>,
    params: &'a Params,
    last_insert_rowid: i64,
    changes: usize,
}

pub(crate) struct Database {
    state: State,
    /// The state a transaction began with.
    snapshot: Option<Box<State>>,
    storage: Box<dyn Storage>,
    dirty: bool,
    last_insert_rowid: i64,
    changes: usize,
    busy_timeout: i64,
}

fn is_aggregate(name: &str, arguments: usize) -> bool {
    match name {
        "count" | "sum" | "total" | "avg" | "group_concat" => true,
        "max" | "min" => arguments == 1,
        _ => false,
    }
}

fn has_aggregate(expr: &Expr) -> bool {
    match expr {
        Expr::Call(name, arguments, _) => {
            is_aggregate(name, arguments.len()) || arguments.iter().any(has_aggregate)
        }
        Expr::Unary(_, inner) | Expr::Cast(inner, _) | Expr::IsNull(inner, _) => {
            has_aggregate(inner)
        }
        Expr::Binary(_, left, right) => has_aggregate(left) || has_aggregate(right),
        Expr::In(inner, list, _) => has_aggregate(inner) || list.iter().any(has_aggregate),
        Expr::Between(a, b, c, _) => has_aggregate(a) || has_aggregate(b) || has_aggregate(c),
        Expr::Like(a, b, c, _) => {
            has_aggregate(a) || has_aggregate(b) || c.as_ref().is_some_and(|c| has_aggregate(c))
        }
        Expr::Case(operand, arms, otherwise) => {
            operand.as_ref().is_some_and(|e| has_aggregate(e))
                || arms
                    .iter()
                    .any(|(a, b)| has_aggregate(a) || has_aggregate(b))
                || otherwise.as_ref().is_some_and(|e| has_aggregate(e))
        }
        _ => false,
    }
}

/// SQL `LIKE`: `%` any run, `_` any character, ASCII letters in any case.
fn like(text: &[char], pattern: &[char], escape: Option<char>) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }
    let (first, rest) = (pattern[0], &pattern[1..]);
    if Some(first) == escape && !rest.is_empty() {
        return !text.is_empty() && text[0] == rest[0] && like(&text[1..], &rest[1..], escape);
    }
    match first {
        '%' => (0..=text.len()).any(|skip| like(&text[skip..], rest, escape)),
        '_' => !text.is_empty() && like(&text[1..], rest, escape),
        c => !text.is_empty() && text[0].eq_ignore_ascii_case(&c) && like(&text[1..], rest, escape),
    }
}

impl Database {
    /// Opens the database `storage` holds, or an empty one.
    pub(crate) fn open(mut storage: Box<dyn Storage>) -> Result<Database, SqlError> {
        let state = match storage.read()? {
            Some((bytes, wal)) if !bytes.is_empty() => load(&bytes, wal.as_deref())?,
            _ => State::default(),
        };
        Ok(Database {
            state,
            snapshot: None,
            storage,
            dirty: false,
            last_insert_rowid: 0,
            changes: 0,
            busy_timeout: 0,
        })
    }

    pub(crate) fn in_transaction(&self) -> bool {
        self.snapshot.is_some()
    }

    /// Runs every statement of `sql`, giving the last one's outcome.
    pub(crate) fn execute(&mut self, sql: &str, params: &Params) -> Result<Outcome, SqlError> {
        let mut outcome = Outcome::default();
        for parsed in sql::parse(sql)? {
            outcome = self.run(&parsed, params)?;
        }
        Ok(outcome)
    }

    /// Runs one parsed statement.
    pub(crate) fn run(
        &mut self,
        parsed: &sql::Parsed,
        params: &Params,
    ) -> Result<Outcome, SqlError> {
        let autocommit = self.snapshot.is_none();
        // A failing statement leaves the database as it was.
        let before = autocommit.then(|| self.state.clone());
        let result = self.statement(parsed, params);
        match result {
            Ok(outcome) => {
                if self.snapshot.is_none() && self.dirty {
                    self.persist()?;
                }
                Ok(outcome)
            }
            Err(found) => {
                if let Some(before) = before {
                    self.state = before;
                    self.dirty = false;
                }
                Err(found)
            }
        }
    }

    fn persist(&mut self) -> Result<(), SqlError> {
        let bytes = image(&self.state);
        self.storage.write(&bytes)?;
        self.state.header.change_counter = self.state.header.change_counter.wrapping_add(1);
        self.dirty = false;
        Ok(())
    }

    fn table(&self, name: &str) -> Result<usize, SqlError> {
        self.state
            .tables
            .iter()
            .position(|table| table.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                if self
                    .state
                    .others
                    .iter()
                    .any(|other| other.entry.name.eq_ignore_ascii_case(name))
                {
                    error(alloc::format!(
                        "table {name} may not be modified by this engine"
                    ))
                } else {
                    error(alloc::format!("no such table: {name}"))
                }
            })
    }

    fn exists(&self, name: &str) -> bool {
        self.state.tables.iter().any(|table| {
            table.name.eq_ignore_ascii_case(name)
                || table
                    .indexes
                    .iter()
                    .any(|index| index.name.eq_ignore_ascii_case(name))
        }) || self
            .state
            .others
            .iter()
            .any(|other| other.entry.name.eq_ignore_ascii_case(name))
    }

    fn statement(&mut self, parsed: &sql::Parsed, params: &Params) -> Result<Outcome, SqlError> {
        match &parsed.statement {
            Statement::CreateTable {
                name,
                if_not_exists,
                columns,
                keys,
                without_rowid,
            } => {
                if self.exists(name) {
                    if *if_not_exists {
                        return Ok(Outcome::default());
                    }
                    return Err(error(alloc::format!("table {name} already exists")));
                }
                if *without_rowid {
                    return Err(error("WITHOUT ROWID tables are not supported"));
                }
                let sql = stored_sql(&parsed.text);
                let table = define_table(name, &sql, columns, keys)?;
                if table.autoincrement && self.table("sqlite_sequence").is_err() {
                    let sequence = "CREATE TABLE sqlite_sequence(name,seq)";
                    let parsed = &sql::parse(sequence)?[0];
                    if let Statement::CreateTable { columns, keys, .. } = &parsed.statement {
                        self.state.tables.push(define_table(
                            "sqlite_sequence",
                            sequence,
                            columns,
                            keys,
                        )?);
                    }
                }
                self.state.tables.push(table);
                self.schema_changed();
                Ok(Outcome::default())
            }
            Statement::AlterTable { table, action } => self.alter_table(table, action),
            Statement::CreateIndex {
                name,
                table,
                columns,
                unique,
                if_not_exists,
            } => {
                if self.exists(name) {
                    if *if_not_exists {
                        return Ok(Outcome::default());
                    }
                    return Err(error(alloc::format!("index {name} already exists")));
                }
                let position = self.table(table)?;
                let target = &self.state.tables[position];
                let mut indexed = Vec::new();
                for column in columns {
                    indexed.push(
                        target
                            .column(column)
                            .ok_or_else(|| error(alloc::format!("no such column: {column}")))?,
                    );
                }
                let mut index = Index {
                    name: name.clone(),
                    sql: Some(stored_sql(&parsed.text)),
                    columns: indexed,
                    unique: *unique,
                    conflict: Conflict::Abort,
                    keys: BTreeMap::new(),
                };
                if index.unique {
                    for (rowid, row) in &target.rows {
                        let key = target.key(&index, row);
                        if key.0.iter().any(|value| matches!(value, SqlValue::Null)) {
                            continue;
                        }
                        if index.keys.insert(key, *rowid).is_some() {
                            return Err(constraint(alloc::format!(
                                "UNIQUE constraint failed: {}",
                                target.constraint_name(&index)
                            )));
                        }
                    }
                }
                self.state.tables[position].indexes.push(index);
                self.schema_changed();
                Ok(Outcome::default())
            }
            Statement::Drop {
                index,
                name,
                if_exists,
            } => {
                if *index {
                    for table in &mut self.state.tables {
                        if let Some(at) = table
                            .indexes
                            .iter()
                            .position(|known| known.name.eq_ignore_ascii_case(name))
                        {
                            if table.indexes[at].sql.is_none() {
                                return Err(error(
                                    "index associated with UNIQUE or PRIMARY KEY constraint cannot be dropped",
                                ));
                            }
                            table.indexes.remove(at);
                            self.schema_changed();
                            return Ok(Outcome::default());
                        }
                    }
                } else if let Some(at) = self
                    .state
                    .tables
                    .iter()
                    .position(|table| table.name.eq_ignore_ascii_case(name))
                {
                    self.state.tables.remove(at);
                    self.schema_changed();
                    return Ok(Outcome::default());
                }
                if let Some(at) = self
                    .state
                    .others
                    .iter()
                    .position(|other| other.entry.name.eq_ignore_ascii_case(name))
                {
                    self.state.others.remove(at);
                    self.schema_changed();
                    return Ok(Outcome::default());
                }
                if *if_exists {
                    return Ok(Outcome::default());
                }
                Err(error(alloc::format!(
                    "no such {}: {name}",
                    if *index { "index" } else { "table" }
                )))
            }
            Statement::Insert {
                table,
                columns,
                rows,
                conflict,
                upsert,
            } => self.insert(table, columns, rows, *conflict, upsert.as_ref(), params),
            Statement::Select(select) => self.select(select, params),
            Statement::Update {
                table,
                set,
                filter,
                conflict,
            } => self.update(table, set, filter.as_ref(), *conflict, params),
            Statement::Delete { table, filter } => self.delete(table, filter.as_ref(), params),
            Statement::Begin => {
                if self.snapshot.is_some() {
                    return Err(error("cannot start a transaction within a transaction"));
                }
                self.snapshot = Some(Box::new(self.state.clone()));
                Ok(Outcome::default())
            }
            Statement::Commit => {
                if self.snapshot.take().is_none() {
                    return Err(error("cannot commit - no transaction is active"));
                }
                if self.dirty {
                    self.persist()?;
                }
                Ok(Outcome::default())
            }
            Statement::Rollback => {
                let Some(snapshot) = self.snapshot.take() else {
                    return Err(error("cannot rollback - no transaction is active"));
                };
                self.state = *snapshot;
                self.dirty = false;
                Ok(Outcome::default())
            }
            Statement::Pragma { name, value } => self.pragma(name, value.as_ref(), params),
            Statement::Vacuum => {
                if self.snapshot.is_some() {
                    return Err(error("cannot VACUUM from within a transaction"));
                }
                self.dirty = true;
                Ok(Outcome::default())
            }
        }
    }

    /// `ALTER TABLE`: a column added, with its default in every row, or the
    /// table renamed, its stored statements rewritten as SQLite rewrites
    /// them.
    fn alter_table(&mut self, table: &str, action: &sql::Alter) -> Result<Outcome, SqlError> {
        let position = self.table(table)?;
        let name = self.state.tables[position].name.clone();
        if name.to_ascii_lowercase().starts_with("sqlite_") {
            return Err(error(alloc::format!("table {name} may not be altered")));
        }
        match action {
            sql::Alter::AddColumn { column, definition } => {
                let target = &self.state.tables[position];
                if target.column(&column.name).is_some() {
                    return Err(error(alloc::format!(
                        "duplicate column name: {}",
                        column.name
                    )));
                }
                if column.primary_key {
                    return Err(error("Cannot add a PRIMARY KEY column"));
                }
                if column.unique.is_some() {
                    return Err(error("Cannot add a UNIQUE column"));
                }
                let affinity = affinity(&column.declared);
                let value = match &column.default {
                    Some(default) => {
                        let params = Params::default();
                        let scope = self.scope(&params);
                        let value = eval(default, &scope)
                            .map_err(|_| error("Cannot add a column with non-constant default"))?;
                        apply_affinity(value, affinity)
                    }
                    None => SqlValue::Null,
                };
                if column.not_null && matches!(value, SqlValue::Null) {
                    return Err(error(
                        "Cannot add a NOT NULL column with default value NULL",
                    ));
                }
                let target = &mut self.state.tables[position];
                // The definition goes at the end of the column list, as
                // SQLite stores it.
                let Some(end) = target.sql.rfind(')') else {
                    return Err(error(alloc::format!("malformed schema of table {name}")));
                };
                target.sql = alloc::format!(
                    "{}, {}{}",
                    &target.sql[..end],
                    definition,
                    &target.sql[end..]
                );
                target.columns.push(Column {
                    name: column.name.clone(),
                    declared: column.declared.clone(),
                    affinity,
                    not_null: column.not_null,
                    default: column.default.clone(),
                });
                for row in target.rows.values_mut() {
                    row.push(value.clone());
                }
            }
            sql::Alter::RenameTable(new) => {
                if self.exists(new) {
                    return Err(error(alloc::format!(
                        "there is already another table or index with this name: {new}"
                    )));
                }
                let target = &mut self.state.tables[position];
                target.sql = renamed_target(&target.sql, "TABLE", new);
                target.name = new.clone();
                let mut automatic = 0;
                for index in &mut target.indexes {
                    match &mut index.sql {
                        Some(sql) => *sql = renamed_target(sql, "ON", new),
                        None => {
                            automatic += 1;
                            index.name = alloc::format!("sqlite_autoindex_{new}_{automatic}");
                        }
                    }
                }
                if let Ok(sequence) = self.table("sqlite_sequence") {
                    for row in self.state.tables[sequence].rows.values_mut() {
                        if matches!(row.first(), Some(SqlValue::Text(text)) if text.eq_ignore_ascii_case(&name))
                        {
                            row[0] = SqlValue::Text(new.clone());
                        }
                    }
                }
            }
            sql::Alter::RenameColumn(..) | sql::Alter::DropColumn(_) => {
                return Err(error(
                    "ALTER TABLE renaming or dropping a column is not supported by this engine",
                ));
            }
        }
        self.schema_changed();
        Ok(Outcome::default())
    }

    fn schema_changed(&mut self) {
        self.state.header.schema_cookie = self.state.header.schema_cookie.wrapping_add(1);
        self.dirty = true;
    }

    fn scope<'a>(&'a self, params: &'a Params) -> Scope<'a> {
        Scope {
            table: None,
            rowid: 0,
            row: None,
            excluded: None,
            params,
            last_insert_rowid: self.last_insert_rowid,
            changes: self.changes,
        }
    }

    fn insert(
        &mut self,
        table: &str,
        columns: &[String],
        rows: &[Vec<Expr>],
        conflict: Conflict,
        upsert: Option<&(Vec<String>, OnConflict)>,
        params: &Params,
    ) -> Result<Outcome, SqlError> {
        let position = self.table(table)?;
        let target_columns: Vec<usize> = if columns.is_empty() {
            (0..self.state.tables[position].columns.len()).collect()
        } else {
            let target = &self.state.tables[position];
            let mut found = Vec::new();
            for column in columns {
                found.push(target.column(column).ok_or_else(|| {
                    error(alloc::format!(
                        "table {} has no column named {column}",
                        target.name
                    ))
                })?);
            }
            found
        };
        let mut changes = 0;
        for values in rows {
            if !values.is_empty() && values.len() != target_columns.len() {
                return Err(error(alloc::format!(
                    "table {} has {} columns but {} values were supplied",
                    self.state.tables[position].name,
                    target_columns.len(),
                    values.len()
                )));
            }
            // The proposed row, with defaults and affinities.
            let mut proposed: Vec<SqlValue> = Vec::new();
            let mut rowid: Option<i64> = None;
            {
                let scope = self.scope(params);
                let target = &self.state.tables[position];
                for (index, column) in target.columns.iter().enumerate() {
                    let given = target_columns
                        .iter()
                        .position(|known| *known == index)
                        .and_then(|at| values.get(at));
                    let value = match (given, &column.default) {
                        (Some(expr), _) => eval(expr, &scope)?,
                        (None, Some(default)) => eval(default, &scope)?,
                        (None, None) => SqlValue::Null,
                    };
                    proposed.push(apply_affinity(value, column.affinity));
                }
                // A rowid named by `rowid` in the column list.
                if target.rowid_column.is_none()
                    && let Some(at) = columns
                        .iter()
                        .position(|name| is_rowid_name(name) && target.column(name).is_none())
                {
                    let value = eval(&values[at], &scope)?;
                    rowid = integer_rowid(&value)?;
                }
            }
            let mut position_rowid = None;
            if let Some(column) = self.state.tables[position].rowid_column {
                match &proposed[column] {
                    SqlValue::Null => {}
                    value => position_rowid = integer_rowid(value)?,
                }
            }
            let rowid = match rowid.or(position_rowid) {
                Some(rowid) => rowid,
                None => self.next_rowid(position)?,
            };
            if let Some(column) = self.state.tables[position].rowid_column {
                proposed[column] = SqlValue::Integer(rowid);
            }
            {
                let target = &self.state.tables[position];
                for (index, column) in target.columns.iter().enumerate() {
                    if column.not_null && matches!(proposed[index], SqlValue::Null) {
                        if conflict == Conflict::Ignore {
                            continue;
                        }
                        return Err(constraint(alloc::format!(
                            "NOT NULL constraint failed: {}.{}",
                            target.name,
                            column.name
                        )));
                    }
                }
            }
            // Conflicts: the rowid, then unique keys.
            let mut clashes = self.state.tables[position].conflicts(&proposed, None);
            let rowid_clash = self.state.tables[position].rows.contains_key(&rowid);
            if let Some((target_names, action)) = upsert {
                let target = &self.state.tables[position];
                let applies = |index: &Index| {
                    target_names.is_empty()
                        || (index.columns.len() == target_names.len()
                            && index
                                .columns
                                .iter()
                                .zip(target_names)
                                .all(|(column, name)| {
                                    target.columns[*column].name.eq_ignore_ascii_case(name)
                                }))
                };
                let hit = clashes
                    .iter()
                    .find(|(index, _)| applies(&target.indexes[*index]))
                    .map(|(_, rowid)| *rowid);
                let hit = hit.or_else(|| {
                    (rowid_clash
                        && (target_names.is_empty()
                            || target.rowid_column.is_some_and(|column| {
                                target_names.len() == 1
                                    && target.columns[column]
                                        .name
                                        .eq_ignore_ascii_case(&target_names[0])
                            })))
                    .then_some(rowid)
                });
                if let Some(existing) = hit {
                    match action {
                        OnConflict::Nothing => continue,
                        OnConflict::Update(set, filter) => {
                            let current = target.rows[&existing].clone();
                            let (updated, run) = {
                                let scope = Scope {
                                    table: Some(target),
                                    rowid: existing,
                                    row: Some(&current),
                                    excluded: Some(&proposed),
                                    params,
                                    last_insert_rowid: self.last_insert_rowid,
                                    changes: self.changes,
                                };
                                let run = match filter {
                                    Some(filter) => truth(&eval(filter, &scope)?) == Some(true),
                                    None => true,
                                };
                                let mut updated = current.clone();
                                if run {
                                    for (name, expr) in set {
                                        let column = target.column(name).ok_or_else(|| {
                                            error(alloc::format!("no such column: {name}"))
                                        })?;
                                        updated[column] = apply_affinity(
                                            eval(expr, &scope)?,
                                            target.columns[column].affinity,
                                        );
                                    }
                                }
                                (updated, run)
                            };
                            if run {
                                self.replace_row(position, existing, updated, Conflict::Abort)?;
                                changes += 1;
                            }
                            continue;
                        }
                    }
                }
            }
            if rowid_clash {
                let resolution = if conflict == Conflict::Abort {
                    Conflict::Abort
                } else {
                    conflict
                };
                match resolution {
                    Conflict::Replace => {
                        self.state.tables[position].remove_row(rowid);
                    }
                    Conflict::Ignore => continue,
                    _ => {
                        let target = &self.state.tables[position];
                        let name = target.rowid_column.map_or_else(
                            || alloc::format!("{}.rowid", target.name),
                            |column| {
                                alloc::format!("{}.{}", target.name, target.columns[column].name)
                            },
                        );
                        return Err(constraint(alloc::format!(
                            "UNIQUE constraint failed: {name}"
                        )));
                    }
                }
                clashes = self.state.tables[position].conflicts(&proposed, None);
            }
            let mut skip = false;
            for (index, existing) in clashes {
                let target = &self.state.tables[position];
                let resolution = if conflict != Conflict::Abort {
                    conflict
                } else {
                    target.indexes[index].conflict
                };
                match resolution {
                    Conflict::Replace => {
                        self.state.tables[position].remove_row(existing);
                    }
                    Conflict::Ignore => {
                        skip = true;
                        break;
                    }
                    _ => {
                        return Err(constraint(alloc::format!(
                            "UNIQUE constraint failed: {}",
                            target.constraint_name(&target.indexes[index])
                        )));
                    }
                }
            }
            if skip {
                continue;
            }
            self.state.tables[position].insert_row(rowid, proposed);
            self.note_sequence(position, rowid)?;
            self.last_insert_rowid = rowid;
            changes += 1;
        }
        self.changes = changes;
        if changes > 0 {
            self.dirty = true;
        }
        Ok(Outcome {
            changes,
            last_insert_rowid: self.last_insert_rowid,
            ..Outcome::default()
        })
    }

    /// The rowid a new row takes: one past the largest, or past the largest
    /// ever used for `AUTOINCREMENT` tables.
    fn next_rowid(&self, position: usize) -> Result<i64, SqlError> {
        let table = &self.state.tables[position];
        let mut largest = table.rows.keys().next_back().copied().unwrap_or(0);
        if table.autoincrement
            && let Ok(sequence) = self.table("sqlite_sequence")
        {
            for row in self.state.tables[sequence].rows.values() {
                if matches!(&row[0], SqlValue::Text(name) if name.eq_ignore_ascii_case(&table.name))
                    && let SqlValue::Integer(seq) = row[1]
                {
                    largest = largest.max(seq);
                }
            }
        }
        if largest == i64::MAX {
            return Err(SqlError {
                code: "SQLITE_FULL",
                message: "database or disk is full".into(),
            });
        }
        Ok(largest + 1)
    }

    fn note_sequence(&mut self, position: usize, rowid: i64) -> Result<(), SqlError> {
        if !self.state.tables[position].autoincrement {
            return Ok(());
        }
        let name = self.state.tables[position].name.clone();
        let sequence = self.table("sqlite_sequence")?;
        let table = &mut self.state.tables[sequence];
        let existing = table
            .rows
            .iter()
            .find(|(_, row)| matches!(&row[0], SqlValue::Text(known) if known.eq_ignore_ascii_case(&name)))
            .map(|(rowid, row)| (*rowid, row.clone()));
        match existing {
            Some((at, row)) => {
                if matches!(row[1], SqlValue::Integer(seq) if seq >= rowid) {
                    return Ok(());
                }
                table.remove_row(at);
                table.insert_row(at, vec![SqlValue::Text(name), SqlValue::Integer(rowid)]);
            }
            None => {
                let at = table.rows.keys().next_back().copied().unwrap_or(0) + 1;
                table.insert_row(at, vec![SqlValue::Text(name), SqlValue::Integer(rowid)]);
            }
        }
        Ok(())
    }

    /// Replaces row `rowid` with `row`, which may change its rowid, with
    /// unique conflicts resolved as `conflict` says.
    fn replace_row(
        &mut self,
        position: usize,
        rowid: i64,
        mut row: Vec<SqlValue>,
        conflict: Conflict,
    ) -> Result<bool, SqlError> {
        let new_rowid = match self.state.tables[position].rowid_column {
            Some(column) => match &row[column] {
                SqlValue::Null => {
                    row[column] = SqlValue::Integer(rowid);
                    rowid
                }
                value => integer_rowid(value)?.unwrap_or(rowid),
            },
            None => rowid,
        };
        let table = &self.state.tables[position];
        for (index, column) in table.columns.iter().enumerate() {
            if column.not_null && matches!(row[index], SqlValue::Null) {
                if conflict == Conflict::Ignore {
                    return Ok(false);
                }
                return Err(constraint(alloc::format!(
                    "NOT NULL constraint failed: {}.{}",
                    table.name,
                    column.name
                )));
            }
        }
        let mut clashes = table.conflicts(&row, Some(rowid));
        if new_rowid != rowid && table.rows.contains_key(&new_rowid) {
            clashes.push((usize::MAX, new_rowid));
        }
        for (index, existing) in clashes {
            let table = &self.state.tables[position];
            let resolution = if conflict != Conflict::Abort || index == usize::MAX {
                conflict
            } else {
                table.indexes[index].conflict
            };
            match resolution {
                Conflict::Replace => {
                    self.state.tables[position].remove_row(existing);
                }
                Conflict::Ignore => return Ok(false),
                _ => {
                    let name = if index == usize::MAX {
                        alloc::format!("{}.rowid", table.name)
                    } else {
                        table.constraint_name(&table.indexes[index])
                    };
                    return Err(constraint(alloc::format!(
                        "UNIQUE constraint failed: {name}"
                    )));
                }
            }
        }
        let table = &mut self.state.tables[position];
        table.remove_row(rowid);
        table.insert_row(new_rowid, row);
        Ok(true)
    }

    /// The rowids of a table's rows a filter keeps.
    fn matching(
        &self,
        position: usize,
        filter: Option<&Expr>,
        params: &Params,
    ) -> Result<Vec<i64>, SqlError> {
        let table = &self.state.tables[position];
        let mut found = Vec::new();
        for (rowid, row) in &table.rows {
            let keep = match filter {
                None => true,
                Some(filter) => {
                    let scope = Scope {
                        table: Some(table),
                        rowid: *rowid,
                        row: Some(row),
                        excluded: None,
                        params,
                        last_insert_rowid: self.last_insert_rowid,
                        changes: self.changes,
                    };
                    truth(&eval(filter, &scope)?) == Some(true)
                }
            };
            if keep {
                found.push(*rowid);
            }
        }
        Ok(found)
    }

    fn update(
        &mut self,
        table: &str,
        set: &[(String, Expr)],
        filter: Option<&Expr>,
        conflict: Conflict,
        params: &Params,
    ) -> Result<Outcome, SqlError> {
        let position = self.table(table)?;
        let rowids = self.matching(position, filter, params)?;
        let mut changes = 0;
        for rowid in rowids {
            let Some(current) = self.state.tables[position].rows.get(&rowid).cloned() else {
                continue;
            };
            let updated = {
                let target = &self.state.tables[position];
                let scope = Scope {
                    table: Some(target),
                    rowid,
                    row: Some(&current),
                    excluded: None,
                    params,
                    last_insert_rowid: self.last_insert_rowid,
                    changes: self.changes,
                };
                let mut updated = current.clone();
                for (name, expr) in set {
                    let column = target
                        .column(name)
                        .ok_or_else(|| error(alloc::format!("no such column: {name}")))?;
                    updated[column] =
                        apply_affinity(eval(expr, &scope)?, target.columns[column].affinity);
                }
                updated
            };
            if self.replace_row(position, rowid, updated, conflict)? {
                changes += 1;
            }
        }
        self.changes = changes;
        if changes > 0 {
            self.dirty = true;
        }
        Ok(Outcome {
            changes,
            last_insert_rowid: self.last_insert_rowid,
            ..Outcome::default()
        })
    }

    fn delete(
        &mut self,
        table: &str,
        filter: Option<&Expr>,
        params: &Params,
    ) -> Result<Outcome, SqlError> {
        let position = self.table(table)?;
        let rowids = self.matching(position, filter, params)?;
        for rowid in &rowids {
            self.state.tables[position].remove_row(*rowid);
        }
        self.changes = rowids.len();
        if !rowids.is_empty() {
            self.dirty = true;
        }
        Ok(Outcome {
            changes: rowids.len(),
            last_insert_rowid: self.last_insert_rowid,
            ..Outcome::default()
        })
    }

    fn select(&self, select: &Select, params: &Params) -> Result<Outcome, SqlError> {
        let position = match &select.from {
            Some(name) => Some(self.table(name)?),
            None => None,
        };
        let table = position.map(|position| &self.state.tables[position]);
        // The rows the query reads: a table's matching rows, or one row.
        let rows: Vec<(i64, Option<&Vec<SqlValue>>)> = match table {
            Some(table) => {
                let mut kept = Vec::new();
                for (rowid, row) in &table.rows {
                    let keep = match &select.filter {
                        None => true,
                        Some(filter) => {
                            let scope = Scope {
                                table: Some(table),
                                rowid: *rowid,
                                row: Some(row),
                                ..self.scope(params)
                            };
                            truth(&eval(filter, &scope)?) == Some(true)
                        }
                    };
                    if keep {
                        kept.push((*rowid, Some(row)));
                    }
                }
                kept
            }
            None => {
                let keep = match &select.filter {
                    None => true,
                    Some(filter) => truth(&eval(filter, &self.scope(params))?) == Some(true),
                };
                if keep { vec![(0, None)] } else { Vec::new() }
            }
        };
        let mut names = Vec::new();
        let mut exprs: Vec<Expr> = Vec::new();
        for column in &select.columns {
            match column {
                ResultColumn::All => {
                    let table = table.ok_or_else(|| error("no tables specified"))?;
                    for column in &table.columns {
                        names.push(column.name.clone());
                        exprs.push(Expr::Column(None, column.name.clone()));
                    }
                }
                ResultColumn::Expr(expr, alias) => {
                    names.push(alias.clone().unwrap_or_else(|| expression_name(expr)));
                    exprs.push(expr.clone());
                }
            }
        }
        let aggregate = exprs.iter().any(has_aggregate);
        let mut output: Vec<(Vec<SqlValue>, Vec<SqlValue>)> = Vec::new();
        if aggregate {
            let mut values = Vec::new();
            for expr in &exprs {
                values.push(aggregate_value(expr, table, &rows, &self.scope(params))?);
            }
            output.push((values, Vec::new()));
        } else {
            for (rowid, row) in &rows {
                let scope = Scope {
                    table,
                    rowid: *rowid,
                    row: row.map(Vec::as_slice),
                    ..self.scope(params)
                };
                let mut values = Vec::with_capacity(exprs.len());
                for expr in &exprs {
                    values.push(eval(expr, &scope)?);
                }
                let mut keys = Vec::with_capacity(select.order.len());
                for (expr, _) in &select.order {
                    // `ORDER BY 2` names the second result column.
                    let key = match expr {
                        Expr::Literal(SqlValue::Integer(at))
                            if *at >= 1 && (*at as usize) <= values.len() =>
                        {
                            values[*at as usize - 1].clone()
                        }
                        Expr::Column(None, name)
                            if names.iter().any(|known| known.eq_ignore_ascii_case(name))
                                && table.and_then(|table| table.column(name)).is_none() =>
                        {
                            let at = names
                                .iter()
                                .position(|known| known.eq_ignore_ascii_case(name))
                                .expect("found");
                            values[at].clone()
                        }
                        expr => eval(expr, &scope)?,
                    };
                    keys.push(key);
                }
                output.push((values, keys));
            }
        }
        if !select.order.is_empty() {
            output.sort_by(|(_, left), (_, right)| {
                for (index, (_, descending)) in select.order.iter().enumerate() {
                    let order = left[index].compare(&right[index]);
                    if order != Ordering::Equal {
                        return if *descending { order.reverse() } else { order };
                    }
                }
                Ordering::Equal
            });
        }
        let mut result: Vec<Vec<SqlValue>> = output.into_iter().map(|(values, _)| values).collect();
        if select.distinct {
            let mut seen: Vec<Vec<SqlValue>> = Vec::new();
            result.retain(|row| {
                if seen
                    .iter()
                    .any(|known| compare_records(known, row) == Ordering::Equal)
                {
                    false
                } else {
                    seen.push(row.clone());
                    true
                }
            });
        }
        let scope = self.scope(params);
        let offset = match &select.offset {
            Some(expr) => match as_number(&eval(expr, &scope)?) {
                SqlValue::Integer(offset) => offset.max(0) as usize,
                _ => 0,
            },
            None => 0,
        };
        let limit = match &select.limit {
            Some(expr) => match as_number(&eval(expr, &scope)?) {
                SqlValue::Integer(limit) if limit >= 0 => Some(limit as usize),
                _ => None,
            },
            None => None,
        };
        let result: Vec<Vec<SqlValue>> = result
            .into_iter()
            .skip(offset)
            .take(limit.unwrap_or(usize::MAX))
            .collect();
        Ok(Outcome {
            columns: names,
            rows: result,
            changes: 0,
            last_insert_rowid: self.last_insert_rowid,
        })
    }

    fn pragma(
        &mut self,
        name: &str,
        value: Option<&Expr>,
        params: &Params,
    ) -> Result<Outcome, SqlError> {
        let value = match value {
            Some(expr) => Some(eval(expr, &self.scope(params))?),
            None => None,
        };
        let integer = |value: &Option<SqlValue>| match value.as_ref().map(as_number) {
            Some(SqlValue::Integer(integer)) => Some(integer),
            Some(SqlValue::Real(real)) => Some(real as i64),
            _ => None,
        };
        let single = |name: &str, value: SqlValue| Outcome {
            columns: vec![name.to_string()],
            rows: vec![vec![value]],
            ..Outcome::default()
        };
        Ok(match name {
            "user_version" | "application_id" => {
                if let Some(set) = integer(&value) {
                    if name == "user_version" {
                        self.state.header.user_version = set as i32;
                    } else {
                        self.state.header.application_id = set as i32;
                    }
                    self.dirty = true;
                    Outcome::default()
                } else {
                    let current = if name == "user_version" {
                        self.state.header.user_version
                    } else {
                        self.state.header.application_id
                    };
                    single(name, SqlValue::Integer(i64::from(current)))
                }
            }
            "schema_version" => single(
                name,
                SqlValue::Integer(i64::from(self.state.header.schema_cookie)),
            ),
            "page_size" => {
                if let Some(size) = integer(&value) {
                    let size = size as u32;
                    if (512..=65_536).contains(&size)
                        && size.is_power_of_two()
                        && self.state.tables.is_empty()
                    {
                        self.state.header.page_size = size;
                    }
                    Outcome::default()
                } else {
                    single(
                        name,
                        SqlValue::Integer(i64::from(self.state.header.page_size)),
                    )
                }
            }
            // The file is always replaced whole, as a rollback journal
            // would leave it: the journal mode in effect is `delete`.
            "journal_mode" => single(name, SqlValue::Text("delete".into())),
            "busy_timeout" => {
                if let Some(timeout) = integer(&value) {
                    self.busy_timeout = timeout;
                }
                single("timeout", SqlValue::Integer(self.busy_timeout))
            }
            "integrity_check" | "quick_check" => single(name, SqlValue::Text("ok".into())),
            "encoding" => single(name, SqlValue::Text("UTF-8".into())),
            "table_info" => {
                let Some(SqlValue::Text(table)) = value else {
                    return Ok(Outcome::default());
                };
                let Ok(position) = self.table(&table) else {
                    return Ok(Outcome::default());
                };
                let table = &self.state.tables[position];
                Outcome {
                    columns: ["cid", "name", "type", "notnull", "dflt_value", "pk"]
                        .map(String::from)
                        .to_vec(),
                    rows: table
                        .columns
                        .iter()
                        .enumerate()
                        .map(|(index, column)| {
                            vec![
                                SqlValue::Integer(index as i64),
                                SqlValue::Text(column.name.clone()),
                                SqlValue::Text(column.declared.clone()),
                                SqlValue::Integer(i64::from(column.not_null)),
                                SqlValue::Null,
                                SqlValue::Integer(i64::from(table.rowid_column == Some(index))),
                            ]
                        })
                        .collect(),
                    ..Outcome::default()
                }
            }
            // Pragmas that tune SQLite itself are accepted and have no
            // effect here; unknown pragmas are ignored, as SQLite ignores them.
            _ => Outcome::default(),
        })
    }
}

fn is_rowid_name(name: &str) -> bool {
    ["rowid", "oid", "_rowid_"]
        .iter()
        .any(|known| known.eq_ignore_ascii_case(name))
}

fn integer_rowid(value: &SqlValue) -> Result<Option<i64>, SqlError> {
    match value {
        SqlValue::Null => Ok(None),
        SqlValue::Integer(integer) => Ok(Some(*integer)),
        SqlValue::Real(real) if crate::float::fract(*real) == 0.0 => Ok(Some(*real as i64)),
        SqlValue::Text(text) => match numeric_text(text) {
            Some(SqlValue::Integer(integer)) => Ok(Some(integer)),
            _ => Err(SqlError {
                code: "SQLITE_MISMATCH",
                message: "datatype mismatch".into(),
            }),
        },
        _ => Err(SqlError {
            code: "SQLITE_MISMATCH",
            message: "datatype mismatch".into(),
        }),
    }
}

/// The name SQLite gives a result column: its expression's text.
fn expression_name(expr: &Expr) -> String {
    match expr {
        Expr::Column(_, name) => name.clone(),
        Expr::Literal(value) => match value {
            SqlValue::Null => "NULL".into(),
            SqlValue::Text(text) => alloc::format!("'{text}'"),
            other => as_text(other).unwrap_or_default(),
        },
        Expr::Call(name, arguments, _) => {
            let inner: Vec<String> = arguments.iter().map(expression_name).collect();
            if name == "count"
                && arguments.len() == 1
                && matches!(arguments[0], Expr::Literal(SqlValue::Integer(1)))
            {
                return "count(*)".into();
            }
            alloc::format!("{name}({})", inner.join(", "))
        }
        _ => "?column?".into(),
    }
}

/// A stored `CREATE` statement naming `name` where it named its table: the
/// name after `keyword` (`TABLE`, or `ON` in an index), quoted as SQLite
/// writes a renamed table.
fn renamed_target(sql: &str, keyword: &str, name: &str) -> String {
    let Ok((tokens, starts)) = sql::tokens(sql) else {
        return sql.into();
    };
    let Some(at) = tokens.iter().position(
        |token| matches!(token, sql::Token::Word(word) if word.eq_ignore_ascii_case(keyword)),
    ) else {
        return sql.into();
    };
    let first = at + 1;
    // A schema-qualified name spans three tokens: `main` `.` `name`.
    let last = if matches!(tokens.get(first + 1), Some(sql::Token::Symbol("."))) {
        first + 2
    } else {
        first
    };
    let chars: Vec<char> = sql.chars().collect();
    let (Some(&start), Some(&next)) = (starts.get(first), starts.get(last + 1)) else {
        return sql.into();
    };
    let span: String = chars[start..next].iter().collect();
    let spacing = &span[span.trim_end().len()..];
    let quoted = name.replace('"', "\"\"");
    alloc::format!(
        "{}\"{quoted}\"{spacing}{}",
        chars[..start].iter().collect::<String>(),
        chars[next..].iter().collect::<String>()
    )
}

/// The text a statement stores in the schema: without `IF NOT EXISTS`.
fn stored_sql(text: &str) -> String {
    let upper = text.to_ascii_uppercase();
    match upper.find("IF NOT EXISTS") {
        Some(at) => {
            let mut stored = String::from(text[..at].trim_end());
            stored.push(' ');
            stored.push_str(text[at + "IF NOT EXISTS".len()..].trim_start());
            stored
        }
        None => text.into(),
    }
}

/// A table from its definition, with the automatic indexes of its `UNIQUE`
/// and `PRIMARY KEY` constraints, named as SQLite names them.
fn define_table(
    name: &str,
    sql: &str,
    columns: &[ColumnDef],
    keys: &[sql::TableKey],
) -> Result<Table, SqlError> {
    let mut table = Table {
        name: name.into(),
        sql: sql.into(),
        columns: columns
            .iter()
            .map(|column| Column {
                name: column.name.clone(),
                declared: column.declared.clone(),
                affinity: affinity(&column.declared),
                not_null: column.not_null,
                default: column.default.clone(),
            })
            .collect(),
        rowid_column: None,
        autoincrement: false,
        rows: BTreeMap::new(),
        indexes: Vec::new(),
    };
    let mut automatic = 0;
    let mut add_index = |table: &mut Table, indexed: Vec<usize>, conflict: Conflict| {
        automatic += 1;
        table.indexes.push(Index {
            name: alloc::format!("sqlite_autoindex_{name}_{automatic}"),
            sql: None,
            columns: indexed,
            unique: true,
            conflict,
            keys: BTreeMap::new(),
        });
    };
    for (index, column) in columns.iter().enumerate() {
        if column.primary_key {
            if column.declared.eq_ignore_ascii_case("INTEGER") {
                table.rowid_column = Some(index);
                table.autoincrement = column.autoincrement;
            } else {
                add_index(&mut table, vec![index], Conflict::Abort);
            }
        }
        if let Some(conflict) = column.unique {
            add_index(&mut table, vec![index], conflict);
        }
    }
    for key in keys {
        let mut indexed = Vec::new();
        for column in &key.columns {
            indexed.push(
                table
                    .column(column)
                    .ok_or_else(|| error(alloc::format!("no such column: {column}")))?,
            );
        }
        if key.primary
            && indexed.len() == 1
            && table.columns[indexed[0]]
                .declared
                .eq_ignore_ascii_case("INTEGER")
        {
            table.rowid_column = Some(indexed[0]);
            continue;
        }
        add_index(&mut table, indexed, key.conflict);
    }
    Ok(table)
}

/// A database's state from its file.
fn load(bytes: &[u8], wal: Option<&[u8]>) -> Result<State, SqlError> {
    let file = format::File::read(bytes, wal)?;
    let schema = file.schema()?;
    let mut state = State {
        header: file.header,
        tables: Vec::new(),
        others: Vec::new(),
    };
    let mut kept_tables: Vec<String> = Vec::new();
    for entry in &schema {
        if entry.kind != "table" {
            continue;
        }
        let defined = entry
            .sql
            .as_deref()
            .and_then(|sql| sql::parse(sql).ok())
            .and_then(
                |mut parsed| match parsed.pop().map(|parsed| parsed.statement) {
                    Some(Statement::CreateTable {
                        columns,
                        keys,
                        without_rowid: false,
                        ..
                    }) => define_table(
                        &entry.name,
                        entry.sql.as_deref().unwrap_or_default(),
                        &columns,
                        &keys,
                    )
                    .ok(),
                    _ => None,
                },
            );
        match defined {
            Some(mut table) => {
                for (rowid, mut row) in file.table(entry.root)? {
                    row.resize(table.columns.len(), SqlValue::Null);
                    if let Some(column) = table.rowid_column {
                        row[column] = SqlValue::Integer(rowid);
                    }
                    table.insert_row(rowid, row);
                }
                state.tables.push(table);
            }
            None => {
                kept_tables.push(entry.name.to_ascii_lowercase());
                state.others.push(Opaque {
                    entry: entry.clone(),
                    rows: file.table(entry.root)?,
                    keys: Vec::new(),
                });
            }
        }
    }
    for entry in &schema {
        match entry.kind.as_str() {
            "table" => {}
            "index" if kept_tables.contains(&entry.table.to_ascii_lowercase()) => {
                state.others.push(Opaque {
                    entry: entry.clone(),
                    rows: Vec::new(),
                    keys: file.index(entry.root)?,
                });
            }
            "index" => {
                let Some(table) = state
                    .tables
                    .iter_mut()
                    .find(|table| table.name.eq_ignore_ascii_case(&entry.table))
                else {
                    continue;
                };
                if entry.sql.is_none() {
                    // An automatic index the table's definition made.
                    continue;
                }
                let parsed = entry.sql.as_deref().and_then(|sql| sql::parse(sql).ok());
                let Some(Statement::CreateIndex {
                    columns, unique, ..
                }) = parsed
                    .and_then(|mut parsed| parsed.pop())
                    .map(|parsed| parsed.statement)
                else {
                    return Err(error(alloc::format!(
                        "cannot read the definition of index {}",
                        entry.name
                    )));
                };
                let mut indexed = Vec::new();
                for column in &columns {
                    indexed.push(
                        table
                            .column(column)
                            .ok_or_else(|| error(alloc::format!("no such column: {column}")))?,
                    );
                }
                let mut index = Index {
                    name: entry.name.clone(),
                    sql: entry.sql.clone(),
                    columns: indexed,
                    unique,
                    conflict: Conflict::Abort,
                    keys: BTreeMap::new(),
                };
                if unique {
                    for (rowid, row) in &table.rows {
                        let key = table.key(&index, row);
                        if !key.0.iter().any(|value| matches!(value, SqlValue::Null)) {
                            index.keys.insert(key, *rowid);
                        }
                    }
                }
                table.indexes.push(index);
            }
            _ => state.others.push(Opaque {
                entry: entry.clone(),
                rows: Vec::new(),
                keys: Vec::new(),
            }),
        }
    }
    Ok(state)
}

/// The file of a database's state.
fn image(state: &State) -> Vec<u8> {
    let mut objects = Vec::new();
    for table in &state.tables {
        let rows: Vec<(i64, Vec<SqlValue>)> = table
            .rows
            .iter()
            .map(|(rowid, row)| {
                let mut row = row.clone();
                // The rowid column is stored as NULL: the rowid holds it.
                if let Some(column) = table.rowid_column {
                    row[column] = SqlValue::Null;
                }
                (*rowid, row)
            })
            .collect();
        objects.push(Object {
            kind: "table".into(),
            name: table.name.clone(),
            table: table.name.clone(),
            sql: Some(table.sql.clone()),
            content: Content::Table(rows),
        });
        for index in &table.indexes {
            let mut keys: Vec<Vec<SqlValue>> = table
                .rows
                .iter()
                .map(|(rowid, row)| {
                    let mut key: Vec<SqlValue> = index
                        .columns
                        .iter()
                        .map(|column| row[*column].clone())
                        .collect();
                    key.push(SqlValue::Integer(*rowid));
                    key
                })
                .collect();
            keys.sort_by(|left, right| compare_records(left, right));
            objects.push(Object {
                kind: "index".into(),
                name: index.name.clone(),
                table: table.name.clone(),
                sql: index.sql.clone(),
                content: Content::Index(keys),
            });
        }
    }
    for other in &state.others {
        let content = match other.entry.kind.as_str() {
            "table" => Content::Table(other.rows.clone()),
            "index" => Content::Index(other.keys.clone()),
            _ => Content::None,
        };
        objects.push(Object {
            kind: other.entry.kind.clone(),
            name: other.entry.name.clone(),
            table: other.entry.table.clone(),
            sql: other.entry.sql.clone(),
            content,
        });
    }
    format::write(&state.header, &objects)
}

/// A column's value in a scope.
fn column_value(
    scope: &Scope<'_>,
    qualifier: Option<&str>,
    name: &str,
) -> Result<(SqlValue, Option<Affinity>), SqlError> {
    let Some(table) = scope.table else {
        return Err(error(alloc::format!("no such column: {name}")));
    };
    let row = if qualifier.is_some_and(|qualifier| qualifier.eq_ignore_ascii_case("excluded")) {
        scope
            .excluded
            .ok_or_else(|| error(alloc::format!("no such column: excluded.{name}")))?
    } else {
        scope
            .row
            .ok_or_else(|| error(alloc::format!("no such column: {name}")))?
    };
    match table.column(name) {
        Some(column) => Ok((row[column].clone(), Some(table.columns[column].affinity))),
        None if is_rowid_name(name) => {
            Ok((SqlValue::Integer(scope.rowid), Some(Affinity::Integer)))
        }
        None => Err(error(alloc::format!("no such column: {name}"))),
    }
}

fn eval(expr: &Expr, scope: &Scope<'_>) -> Result<SqlValue, SqlError> {
    eval_typed(expr, scope).map(|(value, _)| value)
}

/// An expression's value with the affinity it carries for comparisons.
fn eval_typed(expr: &Expr, scope: &Scope<'_>) -> Result<(SqlValue, Option<Affinity>), SqlError> {
    Ok(match expr {
        Expr::Literal(value) => (value.clone(), None),
        Expr::Parameter(parameter) => (scope.params.get(parameter), None),
        Expr::Column(qualifier, name) => column_value(scope, qualifier.as_deref(), name)?,
        Expr::Unary(op, inner) => {
            let value = eval(inner, scope)?;
            (
                match *op {
                    "NOT" => boolean(truth(&value).map(|value| !value)),
                    "-" => match as_number(&value) {
                        SqlValue::Integer(integer) => integer
                            .checked_neg()
                            .map_or(SqlValue::Real(-(integer as f64)), SqlValue::Integer),
                        SqlValue::Real(real) => SqlValue::Real(-real),
                        other => other,
                    },
                    "~" => match as_number(&value) {
                        SqlValue::Integer(integer) => SqlValue::Integer(!integer),
                        SqlValue::Real(real) => SqlValue::Integer(!(real as i64)),
                        other => other,
                    },
                    _ => value,
                },
                None,
            )
        }
        Expr::Binary(op, left, right) => (binary(op, left, right, scope)?, None),
        Expr::IsNull(inner, null) => (
            boolean(Some(matches!(eval(inner, scope)?, SqlValue::Null) == *null)),
            None,
        ),
        Expr::In(inner, list, negated) => {
            let (value, value_affinity) = eval_typed(inner, scope)?;
            if matches!(value, SqlValue::Null) {
                return Ok((SqlValue::Null, None));
            }
            let mut saw_null = false;
            let mut found = false;
            for item in list {
                let (item, item_affinity) = eval_typed(item, scope)?;
                if matches!(item, SqlValue::Null) {
                    saw_null = true;
                    continue;
                }
                let (left, right) = comparable(value.clone(), value_affinity, item, item_affinity);
                if left.compare(&right) == Ordering::Equal {
                    found = true;
                    break;
                }
            }
            let result = if found {
                Some(true)
            } else if saw_null {
                None
            } else {
                Some(false)
            };
            (boolean(result.map(|result| result != *negated)), None)
        }
        Expr::Between(inner, low, high, negated) => {
            let lower = Expr::Binary(">=", inner.clone(), low.clone());
            let upper = Expr::Binary("<=", inner.clone(), high.clone());
            let both = binary("AND", &lower, &upper, scope)?;
            (
                if *negated {
                    boolean(truth(&both).map(|value| !value))
                } else {
                    both
                },
                None,
            )
        }
        Expr::Like(inner, pattern, escape, negated) => {
            let value = eval(inner, scope)?;
            let pattern = eval(pattern, scope)?;
            let escape = match escape {
                Some(escape) => as_text(&eval(escape, scope)?).and_then(|text| text.chars().next()),
                None => None,
            };
            match (as_text(&value), as_text(&pattern)) {
                (Some(text), Some(pattern)) => {
                    let text: Vec<char> = text.chars().collect();
                    let pattern: Vec<char> = pattern.chars().collect();
                    (
                        boolean(Some(like(&text, &pattern, escape) != *negated)),
                        None,
                    )
                }
                _ => (SqlValue::Null, None),
            }
        }
        Expr::Call(name, arguments, _) => {
            if is_aggregate(name, arguments.len()) {
                return Err(error(alloc::format!(
                    "misuse of aggregate function {name}()"
                )));
            }
            (call(name, arguments, scope)?, None)
        }
        Expr::Cast(inner, kind) => {
            let value = eval(inner, scope)?;
            let affinity = affinity(kind);
            let cast = match (affinity, value) {
                (_, SqlValue::Null) => SqlValue::Null,
                (Affinity::Text, value) => SqlValue::Text(as_text(&value).unwrap_or_default()),
                (Affinity::Blob, value) => match value {
                    SqlValue::Blob(blob) => SqlValue::Blob(blob),
                    other => SqlValue::Blob(as_text(&other).unwrap_or_default().into_bytes()),
                },
                (Affinity::Integer, value) => match as_number(&value) {
                    SqlValue::Real(real) => SqlValue::Integer(real as i64),
                    other => other,
                },
                (Affinity::Real, value) => match as_number(&value) {
                    SqlValue::Integer(integer) => SqlValue::Real(integer as f64),
                    other => other,
                },
                (Affinity::Numeric, value) => apply_affinity(as_number(&value), Affinity::Numeric),
            };
            (cast, Some(affinity))
        }
        Expr::Case(operand, arms, otherwise) => {
            let operand = match operand {
                Some(operand) => Some(eval(operand, scope)?),
                None => None,
            };
            for (when, then) in arms {
                let when = eval(when, scope)?;
                let hit = match &operand {
                    Some(operand) => {
                        !matches!(operand, SqlValue::Null)
                            && operand.compare(&when) == Ordering::Equal
                    }
                    None => truth(&when) == Some(true),
                };
                if hit {
                    return eval_typed(then, scope);
                }
            }
            match otherwise {
                Some(otherwise) => eval_typed(otherwise, scope)?,
                None => (SqlValue::Null, None),
            }
        }
    })
}

/// Operands converted for comparison by the affinities they carry, as
/// SQLite converts them.
fn comparable(
    left: SqlValue,
    left_affinity: Option<Affinity>,
    right: SqlValue,
    right_affinity: Option<Affinity>,
) -> (SqlValue, SqlValue) {
    let numeric = |affinity: Option<Affinity>| {
        matches!(
            affinity,
            Some(Affinity::Integer | Affinity::Real | Affinity::Numeric)
        )
    };
    let textual = |affinity: Option<Affinity>| matches!(affinity, Some(Affinity::Text));
    let blob_like = |affinity: Option<Affinity>| matches!(affinity, None | Some(Affinity::Blob));
    if numeric(left_affinity) && (textual(right_affinity) || blob_like(right_affinity)) {
        return (left, apply_affinity(right, Affinity::Numeric));
    }
    if numeric(right_affinity) && (textual(left_affinity) || blob_like(left_affinity)) {
        return (apply_affinity(left, Affinity::Numeric), right);
    }
    if textual(left_affinity) && blob_like(right_affinity) {
        return (left, apply_affinity(right, Affinity::Text));
    }
    if textual(right_affinity) && blob_like(left_affinity) {
        return (apply_affinity(left, Affinity::Text), right);
    }
    (left, right)
}

fn arithmetic(op: &str, left: SqlValue, right: SqlValue) -> SqlValue {
    let (left, right) = (as_number(&left), as_number(&right));
    match (left, right) {
        (SqlValue::Null, _) | (_, SqlValue::Null) => SqlValue::Null,
        (SqlValue::Integer(a), SqlValue::Integer(b)) => {
            let checked = match op {
                "+" => a.checked_add(b),
                "-" => a.checked_sub(b),
                "*" => a.checked_mul(b),
                "/" => {
                    if b == 0 {
                        return SqlValue::Null;
                    }
                    a.checked_div(b)
                }
                "%" => {
                    if b == 0 {
                        return SqlValue::Null;
                    }
                    a.checked_rem(b)
                }
                _ => None,
            };
            match checked {
                Some(value) => SqlValue::Integer(value),
                None => arithmetic_real(op, a as f64, b as f64),
            }
        }
        (a, b) => {
            let real = |value: SqlValue| match value {
                SqlValue::Integer(integer) => integer as f64,
                SqlValue::Real(real) => real,
                _ => 0.0,
            };
            arithmetic_real(op, real(a), real(b))
        }
    }
}

fn arithmetic_real(op: &str, a: f64, b: f64) -> SqlValue {
    match op {
        "+" => SqlValue::Real(a + b),
        "-" => SqlValue::Real(a - b),
        "*" => SqlValue::Real(a * b),
        "/" if b == 0.0 => SqlValue::Null,
        "/" => SqlValue::Real(a / b),
        "%" if (b as i64) == 0 => SqlValue::Null,
        "%" => SqlValue::Real(((a as i64) % (b as i64)) as f64),
        _ => SqlValue::Null,
    }
}

fn binary(op: &str, left: &Expr, right: &Expr, scope: &Scope<'_>) -> Result<SqlValue, SqlError> {
    if op == "AND" || op == "OR" {
        let left = truth(&eval(left, scope)?);
        if op == "AND" && left == Some(false) {
            return Ok(boolean(Some(false)));
        }
        if op == "OR" && left == Some(true) {
            return Ok(boolean(Some(true)));
        }
        let right = truth(&eval(right, scope)?);
        return Ok(boolean(match (op, left, right) {
            ("AND", _, Some(false)) => Some(false),
            ("AND", Some(true), Some(true)) => Some(true),
            ("OR", _, Some(true)) => Some(true),
            ("OR", Some(false), Some(false)) => Some(false),
            _ => None,
        }));
    }
    let (left, left_affinity) = eval_typed(left, scope)?;
    let (right, right_affinity) = eval_typed(right, scope)?;
    Ok(match op {
        "||" => match (as_text(&left), as_text(&right)) {
            (Some(left), Some(right)) => SqlValue::Text(left + &right),
            _ => SqlValue::Null,
        },
        "+" | "-" | "*" | "/" | "%" => arithmetic(op, left, right),
        "<<" | ">>" | "&" | "|" => match (as_number(&left), as_number(&right)) {
            (SqlValue::Null, _) | (_, SqlValue::Null) => SqlValue::Null,
            (a, b) => {
                let integer = |value: SqlValue| match value {
                    SqlValue::Integer(integer) => integer,
                    SqlValue::Real(real) => real as i64,
                    _ => 0,
                };
                let (a, b) = (integer(a), integer(b));
                SqlValue::Integer(match op {
                    "<<" if b >= 64 => 0,
                    "<<" if b >= 0 => a.wrapping_shl(b as u32),
                    "<<" => a.wrapping_shr((-b).min(63) as u32),
                    ">>" if b >= 0 => a.wrapping_shr(b.min(63) as u32),
                    ">>" => a.wrapping_shl((-b).min(63) as u32),
                    "&" => a & b,
                    _ => a | b,
                })
            }
        },
        "IS" | "IS NOT" => {
            let same = match (&left, &right) {
                (SqlValue::Null, SqlValue::Null) => true,
                (SqlValue::Null, _) | (_, SqlValue::Null) => false,
                _ => {
                    let (left, right) = comparable(left, left_affinity, right, right_affinity);
                    left.compare(&right) == Ordering::Equal
                }
            };
            boolean(Some(same == (op == "IS")))
        }
        _ => {
            if matches!(left, SqlValue::Null) || matches!(right, SqlValue::Null) {
                return Ok(SqlValue::Null);
            }
            let (left, right) = comparable(left, left_affinity, right, right_affinity);
            let order = left.compare(&right);
            boolean(Some(match op {
                "=" => order == Ordering::Equal,
                "!=" => order != Ordering::Equal,
                "<" => order == Ordering::Less,
                "<=" => order != Ordering::Greater,
                ">" => order == Ordering::Greater,
                ">=" => order != Ordering::Less,
                _ => return Err(error(alloc::format!("unknown operator {op}"))),
            }))
        }
    })
}

fn call(name: &str, arguments: &[Expr], scope: &Scope<'_>) -> Result<SqlValue, SqlError> {
    let mut values = Vec::with_capacity(arguments.len());
    for argument in arguments {
        values.push(eval(argument, scope)?);
    }
    let argument = |index: usize| values.get(index).cloned().unwrap_or(SqlValue::Null);
    let count = |expected: usize| {
        if values.len() == expected {
            Ok(())
        } else {
            Err(error(alloc::format!(
                "wrong number of arguments to function {name}()"
            )))
        }
    };
    Ok(match name {
        "length" => {
            count(1)?;
            match argument(0) {
                SqlValue::Null => SqlValue::Null,
                SqlValue::Blob(blob) => SqlValue::Integer(blob.len() as i64),
                other => {
                    SqlValue::Integer(as_text(&other).unwrap_or_default().chars().count() as i64)
                }
            }
        }
        "lower" | "upper" => {
            count(1)?;
            match as_text(&argument(0)) {
                Some(text) if name == "lower" => SqlValue::Text(text.to_ascii_lowercase()),
                Some(text) => SqlValue::Text(text.to_ascii_uppercase()),
                None => SqlValue::Null,
            }
        }
        "coalesce" | "ifnull" => values
            .into_iter()
            .find(|value| !matches!(value, SqlValue::Null))
            .unwrap_or(SqlValue::Null),
        "nullif" => {
            count(2)?;
            if argument(0).compare(&argument(1)) == Ordering::Equal {
                SqlValue::Null
            } else {
                argument(0)
            }
        }
        "iif" => {
            count(3)?;
            if truth(&argument(0)) == Some(true) {
                argument(1)
            } else {
                argument(2)
            }
        }
        "abs" => {
            count(1)?;
            match as_number(&argument(0)) {
                SqlValue::Integer(integer) => integer
                    .checked_abs()
                    .map_or(SqlValue::Real((integer as f64).abs()), SqlValue::Integer),
                SqlValue::Real(real) => SqlValue::Real(real.abs()),
                other => other,
            }
        }
        "typeof" => {
            count(1)?;
            SqlValue::Text(
                match argument(0) {
                    SqlValue::Null => "null",
                    SqlValue::Integer(_) => "integer",
                    SqlValue::Real(_) => "real",
                    SqlValue::Text(_) => "text",
                    SqlValue::Blob(_) => "blob",
                }
                .into(),
            )
        }
        "max" | "min" => {
            if values.iter().any(|value| matches!(value, SqlValue::Null)) {
                return Ok(SqlValue::Null);
            }
            let mut best = argument(0);
            for value in values.into_iter().skip(1) {
                let order = value.compare(&best);
                if (name == "max" && order == Ordering::Greater)
                    || (name == "min" && order == Ordering::Less)
                {
                    best = value;
                }
            }
            best
        }
        "substr" | "substring" => {
            let Some(text) = as_text(&argument(0)) else {
                return Ok(SqlValue::Null);
            };
            let chars: Vec<char> = text.chars().collect();
            let start = match as_number(&argument(1)) {
                SqlValue::Integer(start) => start,
                SqlValue::Real(start) => start as i64,
                _ => 1,
            };
            let length = match values.get(2).map(as_number) {
                Some(SqlValue::Integer(length)) => Some(length),
                Some(SqlValue::Real(length)) => Some(length as i64),
                _ => None,
            };
            let total = chars.len() as i64;
            let begin = if start > 0 {
                start - 1
            } else if start < 0 {
                (total + start).max(0)
            } else {
                0
            };
            let end = match length {
                Some(length) if length >= 0 => begin + length - i64::from(start == 0),
                Some(_) => begin,
                None => total,
            };
            let begin = begin.clamp(0, total) as usize;
            let end = end.clamp(begin as i64, total) as usize;
            SqlValue::Text(chars[begin..end].iter().collect())
        }
        "instr" => {
            count(2)?;
            match (as_text(&argument(0)), as_text(&argument(1))) {
                (Some(text), Some(needle)) => SqlValue::Integer(
                    text.find(&needle)
                        .map_or(0, |at| text[..at].chars().count() as i64 + 1),
                ),
                _ => SqlValue::Null,
            }
        }
        "replace" => {
            count(3)?;
            match (
                as_text(&argument(0)),
                as_text(&argument(1)),
                as_text(&argument(2)),
            ) {
                (Some(text), Some(from), Some(to)) if !from.is_empty() => {
                    SqlValue::Text(text.replace(&from, &to))
                }
                (Some(text), Some(_), Some(_)) => SqlValue::Text(text),
                _ => SqlValue::Null,
            }
        }
        "trim" | "ltrim" | "rtrim" => {
            let Some(text) = as_text(&argument(0)) else {
                return Ok(SqlValue::Null);
            };
            let set: Vec<char> = match values.get(1) {
                Some(value) => as_text(value).unwrap_or_default().chars().collect(),
                None => vec![' '],
            };
            let trimmed = match name {
                "trim" => text.trim_matches(|c| set.contains(&c)),
                "ltrim" => text.trim_start_matches(|c| set.contains(&c)),
                _ => text.trim_end_matches(|c| set.contains(&c)),
            };
            SqlValue::Text(trimmed.into())
        }
        "hex" => {
            count(1)?;
            let bytes = match argument(0) {
                SqlValue::Blob(blob) => blob,
                other => as_text(&other).unwrap_or_default().into_bytes(),
            };
            SqlValue::Text(
                bytes
                    .iter()
                    .map(|byte| alloc::format!("{byte:02X}"))
                    .collect(),
            )
        }
        "round" => {
            let digits = match values.get(1).map(as_number) {
                Some(SqlValue::Integer(digits)) => digits.clamp(0, 30) as i32,
                _ => 0,
            };
            match as_number(&argument(0)) {
                SqlValue::Null => SqlValue::Null,
                SqlValue::Integer(integer) => SqlValue::Real(integer as f64),
                SqlValue::Real(real) => {
                    // Half away from zero, as SQLite rounds.
                    let scale = crate::float::pow(10.0, f64::from(digits));
                    let magnitude = crate::float::floor(real.abs() * scale + 0.5) / scale;
                    SqlValue::Real(if real < 0.0 { -magnitude } else { magnitude })
                }
                other => other,
            }
        }
        "last_insert_rowid" => SqlValue::Integer(scope.last_insert_rowid),
        "changes" => SqlValue::Integer(scope.changes as i64),
        "sqlite_version" => SqlValue::Text("3.45.0".into()),
        _ => return Err(error(alloc::format!("no such function: {name}"))),
    })
}

/// An aggregate expression's value over `rows`.
fn aggregate_value(
    expr: &Expr,
    table: Option<&Table>,
    rows: &[(i64, Option<&Vec<SqlValue>>)],
    base: &Scope<'_>,
) -> Result<SqlValue, SqlError> {
    match expr {
        Expr::Call(name, arguments, distinct) if is_aggregate(name, arguments.len()) => {
            let mut values = Vec::new();
            for (rowid, row) in rows {
                let scope = Scope {
                    table,
                    rowid: *rowid,
                    row: row.map(Vec::as_slice),
                    excluded: None,
                    params: base.params,
                    last_insert_rowid: base.last_insert_rowid,
                    changes: base.changes,
                };
                let value = match arguments.first() {
                    Some(argument) => eval(argument, &scope)?,
                    None => SqlValue::Integer(1),
                };
                if !matches!(value, SqlValue::Null) {
                    values.push(value);
                }
            }
            if *distinct {
                let mut unique: Vec<SqlValue> = Vec::new();
                for value in values {
                    if !unique
                        .iter()
                        .any(|known| known.compare(&value) == Ordering::Equal)
                    {
                        unique.push(value);
                    }
                }
                values = unique;
            }
            Ok(match name.as_str() {
                "count" => SqlValue::Integer(values.len() as i64),
                "sum" | "total" | "avg" => {
                    if values.is_empty() {
                        return Ok(match name.as_str() {
                            "total" => SqlValue::Real(0.0),
                            _ => SqlValue::Null,
                        });
                    }
                    let mut sum = SqlValue::Integer(0);
                    for value in &values {
                        sum = arithmetic("+", sum, value.clone());
                    }
                    match name.as_str() {
                        "sum" => sum,
                        "total" => arithmetic("+", sum, SqlValue::Real(0.0)),
                        _ => arithmetic(
                            "/",
                            arithmetic("+", sum, SqlValue::Real(0.0)),
                            SqlValue::Integer(values.len() as i64),
                        ),
                    }
                }
                "max" | "min" => {
                    let mut best: Option<SqlValue> = None;
                    for value in values {
                        let better = match &best {
                            None => true,
                            Some(best) => {
                                let order = value.compare(best);
                                (name == "max" && order == Ordering::Greater)
                                    || (name == "min" && order == Ordering::Less)
                            }
                        };
                        if better {
                            best = Some(value);
                        }
                    }
                    best.unwrap_or(SqlValue::Null)
                }
                _ => {
                    let separator = match arguments.get(1) {
                        Some(separator) => as_text(&eval(separator, base)?).unwrap_or_default(),
                        None => ",".into(),
                    };
                    if values.is_empty() {
                        SqlValue::Null
                    } else {
                        SqlValue::Text(
                            values
                                .iter()
                                .map(|value| as_text(value).unwrap_or_default())
                                .collect::<Vec<_>>()
                                .join(&separator),
                        )
                    }
                }
            })
        }
        other => {
            // A plain expression beside aggregates takes the first row's
            // value, as SQLite gives a row of the group.
            let (rowid, row) = rows.first().copied().unwrap_or((0, None));
            let scope = Scope {
                table,
                rowid,
                row: row.map(Vec::as_slice),
                excluded: None,
                params: base.params,
                last_insert_rowid: base.last_insert_rowid,
                changes: base.changes,
            };
            match other {
                Expr::Binary(op, left, right) if has_aggregate(other) => {
                    let left = aggregate_value(left, table, rows, base)?;
                    let right = aggregate_value(right, table, rows, base)?;
                    binary(op, &Expr::Literal(left), &Expr::Literal(right), &scope)
                }
                _ => eval(other, &scope),
            }
        }
    }
}

#[cfg(test)]
mod tests;
