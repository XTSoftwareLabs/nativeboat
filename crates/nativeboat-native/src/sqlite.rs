//! SQLite databases, in SQLite's own file format, without SQLite: files the
//! SQLite library writes open here, and files written here open in SQLite.
//!
//! - [`format`] reads a database file, with its write-ahead log when it has
//!   one, into its schema and the content of its tables and indexes, and
//!   writes a database's content as a new file.
//! - [`sql`] parses the SQL statements the engine runs.
//! - [`engine`] keeps a database in memory and runs statements over it, as
//!   SQLite runs them, writing its file when changes commit.

// The SQL engine and the `@vscode/sqlite3` binding build on this module.
#![allow(dead_code)]

pub(crate) mod engine;
pub(crate) mod format;
pub(crate) mod sql;

/// A value in a database: SQLite's five storage classes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SqlValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(alloc::string::String),
    Blob(alloc::vec::Vec<u8>),
}

impl SqlValue {
    /// The order SQLite sorts values in with the `BINARY` collation: NULLs,
    /// then numbers by value, then text and blobs bytewise.
    pub(crate) fn compare(&self, other: &SqlValue) -> core::cmp::Ordering {
        use core::cmp::Ordering;
        fn class(value: &SqlValue) -> u8 {
            match value {
                SqlValue::Null => 0,
                SqlValue::Integer(_) | SqlValue::Real(_) => 1,
                SqlValue::Text(_) => 2,
                SqlValue::Blob(_) => 3,
            }
        }
        match (self, other) {
            (SqlValue::Integer(left), SqlValue::Integer(right)) => left.cmp(right),
            (SqlValue::Integer(left), SqlValue::Real(right)) => compare_mixed(*left, *right),
            (SqlValue::Real(left), SqlValue::Integer(right)) => {
                compare_mixed(*right, *left).reverse()
            }
            (SqlValue::Real(left), SqlValue::Real(right)) => {
                left.partial_cmp(right).unwrap_or(Ordering::Equal)
            }
            (SqlValue::Text(left), SqlValue::Text(right)) => left.as_bytes().cmp(right.as_bytes()),
            (SqlValue::Blob(left), SqlValue::Blob(right)) => left.cmp(right),
            _ => class(self).cmp(&class(other)),
        }
    }
}

/// An integer and a real compared exactly, as SQLite compares them.
fn compare_mixed(integer: i64, real: f64) -> core::cmp::Ordering {
    use core::cmp::Ordering;
    if real.is_nan() {
        return Ordering::Greater;
    }
    if real >= 9_223_372_036_854_775_808.0 {
        return Ordering::Less;
    }
    if real < -9_223_372_036_854_775_808.0 {
        return Ordering::Greater;
    }
    let truncated = real as i64;
    match integer.cmp(&truncated) {
        Ordering::Equal => {
            let fraction = real - truncated as f64;
            if fraction > 0.0 {
                Ordering::Less
            } else if fraction < 0.0 {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        }
        other => other,
    }
}

/// Records compared field by field, as an index orders its keys.
pub(crate) fn compare_records(left: &[SqlValue], right: &[SqlValue]) -> core::cmp::Ordering {
    for (left, right) in left.iter().zip(right) {
        let order = left.compare(right);
        if order != core::cmp::Ordering::Equal {
            return order;
        }
    }
    left.len().cmp(&right.len())
}
