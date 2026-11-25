//! SQL statements, as SQLite parses them, for the statements the engine
//! runs: tables and indexes, inserts with conflict handling, queries,
//! updates, deletes, transactions, pragmas and `VACUUM`.

use super::SqlValue;
#[allow(unused_imports)]
use crate::prelude::*;
use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// A syntax error, with SQLite's wording where it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SyntaxError(pub(crate) String);

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Token {
    /// A keyword or bare identifier, as written.
    Word(String),
    /// A quoted identifier.
    Quoted(String),
    Text(String),
    Blob(Vec<u8>),
    Integer(i64),
    Real(f64),
    /// `?`, `?NNN`, `:name`, `@name` or `$name`.
    Parameter(Parameter),
    Symbol(&'static str),
}

/// A bound parameter: by position, or by name with its prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Parameter {
    /// `?`: the next position.
    Next,
    /// `?NNN`.
    Numbered(usize),
    /// `:name`, `@name` or `$name`, with its prefix.
    Named(String),
}

const SYMBOLS: [&str; 26] = [
    "||", "<<", ">>", "<=", ">=", "==", "!=", "<>", "(", ")", ",", ";", ".", "*", "+", "-", "/",
    "%", "<", ">", "=", "&", "|", "~", "!", "?",
];

pub(crate) fn tokens(sql: &str) -> Result<(Vec<Token>, Vec<usize>), SyntaxError> {
    let chars: Vec<char> = sql.chars().collect();
    let mut tokens = Vec::new();
    // Where each token starts: an iteration that pushed a token records
    // where it began at the start of the next.
    let mut starts = Vec::new();
    let mut began = 0;
    let mut at = 0;
    let unrecognized = |text: &str| SyntaxError(alloc::format!("unrecognized token: \"{text}\""));
    while at < chars.len() {
        if starts.len() < tokens.len() {
            starts.push(began);
        }
        began = at;
        let c = chars[at];
        if c.is_whitespace() {
            at += 1;
            continue;
        }
        if c == '-' && chars.get(at + 1) == Some(&'-') {
            while at < chars.len() && chars[at] != '\n' {
                at += 1;
            }
            continue;
        }
        if c == '/' && chars.get(at + 1) == Some(&'*') {
            at += 2;
            while at < chars.len() && !(chars[at] == '*' && chars.get(at + 1) == Some(&'/')) {
                at += 1;
            }
            at += 2;
            continue;
        }
        if matches!(c, '\'' | '"' | '`' | '[') {
            let close = match c {
                '[' => ']',
                other => other,
            };
            let mut text = String::new();
            at += 1;
            loop {
                let Some(&next) = chars.get(at) else {
                    return Err(unrecognized(
                        &chars[at.saturating_sub(text.len() + 1)..]
                            .iter()
                            .collect::<String>(),
                    ));
                };
                at += 1;
                if next == close {
                    // A doubled quote is one quote, except in brackets.
                    if close != ']' && chars.get(at) == Some(&close) {
                        text.push(close);
                        at += 1;
                        continue;
                    }
                    break;
                }
                text.push(next);
            }
            tokens.push(if c == '\'' {
                Token::Text(text)
            } else {
                Token::Quoted(text)
            });
            continue;
        }
        if matches!(c, 'x' | 'X') && chars.get(at + 1) == Some(&'\'') {
            let start = at + 2;
            let mut end = start;
            while end < chars.len() && chars[end] != '\'' {
                end += 1;
            }
            let hex: String = chars[start..end.min(chars.len())].iter().collect();
            if end >= chars.len()
                || !hex.len().is_multiple_of(2)
                || !hex.chars().all(|c| c.is_ascii_hexdigit())
            {
                return Err(unrecognized(
                    &chars[at..end.min(chars.len())].iter().collect::<String>(),
                ));
            }
            let bytes = (0..hex.len())
                .step_by(2)
                .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("hex digits"))
                .collect();
            tokens.push(Token::Blob(bytes));
            at = end + 1;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && chars.get(at + 1).is_some_and(char::is_ascii_digit)) {
            let start = at;
            if c == '0' && matches!(chars.get(at + 1), Some('x' | 'X')) {
                at += 2;
                while at < chars.len() && chars[at].is_ascii_hexdigit() {
                    at += 1;
                }
                let digits: String = chars[start + 2..at].iter().collect();
                let value = u64::from_str_radix(&digits, 16).map_err(|_| unrecognized(&digits))?;
                tokens.push(Token::Integer(value as i64));
                continue;
            }
            let mut real = false;
            while at < chars.len() && chars[at].is_ascii_digit() {
                at += 1;
            }
            if chars.get(at) == Some(&'.') {
                real = true;
                at += 1;
                while at < chars.len() && chars[at].is_ascii_digit() {
                    at += 1;
                }
            }
            if matches!(chars.get(at), Some('e' | 'E')) {
                real = true;
                at += 1;
                if matches!(chars.get(at), Some('+' | '-')) {
                    at += 1;
                }
                while at < chars.len() && chars[at].is_ascii_digit() {
                    at += 1;
                }
            }
            let text: String = chars[start..at].iter().collect();
            if !real && let Ok(integer) = text.parse::<i64>() {
                tokens.push(Token::Integer(integer));
            } else {
                tokens.push(Token::Real(
                    text.parse::<f64>().map_err(|_| unrecognized(&text))?,
                ));
            }
            continue;
        }
        if c == '?' {
            at += 1;
            let start = at;
            while at < chars.len() && chars[at].is_ascii_digit() {
                at += 1;
            }
            let digits: String = chars[start..at].iter().collect();
            tokens.push(Token::Parameter(if digits.is_empty() {
                Parameter::Next
            } else {
                Parameter::Numbered(digits.parse().map_err(|_| unrecognized(&digits))?)
            }));
            continue;
        }
        if matches!(c, ':' | '@' | '$') {
            let start = at;
            at += 1;
            while at < chars.len() && (chars[at].is_alphanumeric() || chars[at] == '_') {
                at += 1;
            }
            tokens.push(Token::Parameter(Parameter::Named(
                chars[start..at].iter().collect(),
            )));
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = at;
            while at < chars.len()
                && (chars[at].is_alphanumeric() || chars[at] == '_' || chars[at] == '$')
            {
                at += 1;
            }
            tokens.push(Token::Word(chars[start..at].iter().collect()));
            continue;
        }
        let rest: String = chars[at..(at + 2).min(chars.len())].iter().collect();
        if let Some(symbol) = SYMBOLS.iter().find(|symbol| rest.starts_with(**symbol)) {
            tokens.push(Token::Symbol(symbol));
            at += symbol.chars().count();
            continue;
        }
        return Err(unrecognized(&c.to_string()));
    }
    if starts.len() < tokens.len() {
        starts.push(began);
    }
    Ok((tokens, starts))
}

/// An expression.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Expr {
    Literal(SqlValue),
    Parameter(Parameter),
    /// A column, with the table (or `excluded`) that qualifies it.
    Column(Option<String>, String),
    Unary(&'static str, Box<Expr>),
    Binary(&'static str, Box<Expr>, Box<Expr>),
    /// `expr [NOT] IN (list)`.
    In(Box<Expr>, Vec<Expr>, bool),
    /// `expr [NOT] BETWEEN low AND high`.
    Between(Box<Expr>, Box<Expr>, Box<Expr>, bool),
    /// `expr IS [NOT] NULL` and the like: whether it is NULL, or not.
    IsNull(Box<Expr>, bool),
    /// `expr [NOT] LIKE pattern [ESCAPE escape]`.
    Like(Box<Expr>, Box<Expr>, Option<Box<Expr>>, bool),
    /// A function call; `*` arguments for `count(*)`.
    Call(String, Vec<Expr>, bool),
    Cast(Box<Expr>, String),
    /// `CASE [operand] WHEN .. THEN .. [ELSE ..] END`.
    Case(Option<Box<Expr>>, Vec<(Expr, Expr)>, Option<Box<Expr>>),
}

/// What a constraint does when it is violated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Conflict {
    #[default]
    Abort,
    Fail,
    Ignore,
    Replace,
    Rollback,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ColumnDef {
    pub(crate) name: String,
    pub(crate) declared: String,
    pub(crate) primary_key: bool,
    pub(crate) autoincrement: bool,
    pub(crate) not_null: bool,
    /// A `UNIQUE` constraint, with its conflict clause.
    pub(crate) unique: Option<Conflict>,
    pub(crate) default: Option<Expr>,
}

/// A table constraint: `PRIMARY KEY (..)` or `UNIQUE (..)`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TableKey {
    pub(crate) primary: bool,
    pub(crate) columns: Vec<String>,
    pub(crate) conflict: Conflict,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum OnConflict {
    Nothing,
    Update(Vec<(String, Expr)>, Option<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ResultColumn {
    All,
    Expr(Expr, Option<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Select {
    pub(crate) distinct: bool,
    pub(crate) columns: Vec<ResultColumn>,
    pub(crate) from: Option<String>,
    pub(crate) filter: Option<Expr>,
    pub(crate) order: Vec<(Expr, bool)>,
    pub(crate) limit: Option<Expr>,
    pub(crate) offset: Option<Expr>,
}

/// What `ALTER TABLE` changes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Alter {
    /// `ADD [COLUMN]`, with the column definition's text as written.
    AddColumn {
        column: ColumnDef,
        definition: String,
    },
    RenameTable(String),
    RenameColumn(String, String),
    DropColumn(String),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Statement {
    CreateTable {
        name: String,
        if_not_exists: bool,
        columns: Vec<ColumnDef>,
        keys: Vec<TableKey>,
        without_rowid: bool,
    },
    CreateIndex {
        name: String,
        table: String,
        columns: Vec<String>,
        unique: bool,
        if_not_exists: bool,
    },
    Drop {
        index: bool,
        name: String,
        if_exists: bool,
    },
    Insert {
        table: String,
        columns: Vec<String>,
        rows: Vec<Vec<Expr>>,
        conflict: Conflict,
        upsert: Option<(Vec<String>, OnConflict)>,
    },
    Select(Select),
    Update {
        table: String,
        set: Vec<(String, Expr)>,
        filter: Option<Expr>,
        conflict: Conflict,
    },
    Delete {
        table: String,
        filter: Option<Expr>,
    },
    Begin,
    Commit,
    Rollback,
    Pragma {
        name: String,
        value: Option<Expr>,
    },
    Vacuum,
    AlterTable {
        table: String,
        action: Alter,
    },
}

struct Parser {
    tokens: Vec<Token>,
    /// Where each token starts in `chars`.
    starts: Vec<usize>,
    chars: Vec<char>,
    at: usize,
    /// The next `?` parameter's position.
    next_parameter: usize,
}

fn word_is(token: Option<&Token>, keyword: &str) -> bool {
    matches!(token, Some(Token::Word(word)) if word.eq_ignore_ascii_case(keyword))
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.at + offset)
    }

    fn error(&self) -> SyntaxError {
        match self.peek() {
            None => SyntaxError("incomplete input".into()),
            Some(token) => SyntaxError(alloc::format!("near \"{}\": syntax error", display(token))),
        }
    }

    fn keyword(&mut self, keyword: &str) -> bool {
        if word_is(self.peek(), keyword) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn expect_keyword(&mut self, keyword: &str) -> Result<(), SyntaxError> {
        if self.keyword(keyword) {
            Ok(())
        } else {
            Err(self.error())
        }
    }

    fn symbol(&mut self, symbol: &str) -> bool {
        if matches!(self.peek(), Some(Token::Symbol(found)) if *found == symbol) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn expect_symbol(&mut self, symbol: &str) -> Result<(), SyntaxError> {
        if self.symbol(symbol) {
            Ok(())
        } else {
            Err(self.error())
        }
    }

    fn name(&mut self) -> Result<String, SyntaxError> {
        match self.peek().cloned() {
            Some(Token::Word(word)) | Some(Token::Quoted(word)) | Some(Token::Text(word)) => {
                self.at += 1;
                Ok(word)
            }
            _ => Err(self.error()),
        }
    }

    /// A name, possibly qualified by a schema: `main.t` is `t`.
    fn object_name(&mut self) -> Result<String, SyntaxError> {
        let name = self.name()?;
        if self.symbol(".") {
            return self.name();
        }
        Ok(name)
    }

    /// The source text of the tokens from `first` to the current one.
    fn text_from(&self, first: usize) -> String {
        let end = self
            .starts
            .get(self.at)
            .copied()
            .unwrap_or(self.chars.len());
        self.chars[self.starts[first]..end]
            .iter()
            .collect::<String>()
            .trim_end()
            .into()
    }

    fn alter_table(&mut self) -> Result<Statement, SyntaxError> {
        self.expect_keyword("TABLE")?;
        let table = self.object_name()?;
        let action = if self.keyword("RENAME") {
            if self.keyword("TO") {
                Alter::RenameTable(self.name()?)
            } else {
                let _ = self.keyword("COLUMN");
                let from = self.name()?;
                self.expect_keyword("TO")?;
                Alter::RenameColumn(from, self.name()?)
            }
        } else if self.keyword("ADD") {
            let _ = self.keyword("COLUMN");
            let first = self.at;
            let column = self.column()?;
            Alter::AddColumn {
                column,
                definition: self.text_from(first),
            }
        } else if self.keyword("DROP") {
            let _ = self.keyword("COLUMN");
            Alter::DropColumn(self.name()?)
        } else {
            return Err(self.error());
        };
        Ok(Statement::AlterTable { table, action })
    }

    fn statement(&mut self) -> Result<Statement, SyntaxError> {
        if self.keyword("ALTER") {
            return self.alter_table();
        }
        if self.keyword("CREATE") {
            let unique = self.keyword("UNIQUE");
            if self.keyword("TABLE") {
                return self.create_table();
            }
            if self.keyword("INDEX") {
                return self.create_index(unique);
            }
            return Err(self.error());
        }
        if self.keyword("DROP") {
            let index = if self.keyword("INDEX") {
                true
            } else {
                self.expect_keyword("TABLE")?;
                false
            };
            let if_exists = self.keyword("IF") && {
                self.expect_keyword("EXISTS")?;
                true
            };
            let name = self.object_name()?;
            return Ok(Statement::Drop {
                index,
                name,
                if_exists,
            });
        }
        if self.keyword("INSERT") {
            let conflict = if self.keyword("OR") {
                self.conflict()?
            } else {
                Conflict::Abort
            };
            return self.insert(conflict);
        }
        if self.keyword("REPLACE") {
            return self.insert(Conflict::Replace);
        }
        if word_is(self.peek(), "SELECT") {
            return Ok(Statement::Select(self.select()?));
        }
        if self.keyword("UPDATE") {
            let conflict = if self.keyword("OR") {
                self.conflict()?
            } else {
                Conflict::Abort
            };
            let table = self.object_name()?;
            self.expect_keyword("SET")?;
            let set = self.assignments()?;
            let filter = if self.keyword("WHERE") {
                Some(self.expr()?)
            } else {
                None
            };
            return Ok(Statement::Update {
                table,
                set,
                filter,
                conflict,
            });
        }
        if self.keyword("DELETE") {
            self.expect_keyword("FROM")?;
            let table = self.object_name()?;
            let filter = if self.keyword("WHERE") {
                Some(self.expr()?)
            } else {
                None
            };
            return Ok(Statement::Delete { table, filter });
        }
        if self.keyword("BEGIN") {
            let _ =
                self.keyword("DEFERRED") || self.keyword("IMMEDIATE") || self.keyword("EXCLUSIVE");
            let _ = self.keyword("TRANSACTION");
            return Ok(Statement::Begin);
        }
        if self.keyword("COMMIT") || self.keyword("END") {
            let _ = self.keyword("TRANSACTION");
            return Ok(Statement::Commit);
        }
        if self.keyword("ROLLBACK") {
            let _ = self.keyword("TRANSACTION");
            return Ok(Statement::Rollback);
        }
        if self.keyword("PRAGMA") {
            let mut name = self.name()?;
            if self.symbol(".") {
                name = self.name()?;
            }
            let value = if self.symbol("=") {
                Some(self.pragma_value()?)
            } else if self.symbol("(") {
                let value = self.pragma_value()?;
                self.expect_symbol(")")?;
                Some(value)
            } else {
                None
            };
            return Ok(Statement::Pragma {
                name: name.to_ascii_lowercase(),
                value,
            });
        }
        if self.keyword("VACUUM") {
            return Ok(Statement::Vacuum);
        }
        Err(self.error())
    }

    /// A pragma's value: a name stands for itself.
    fn pragma_value(&mut self) -> Result<Expr, SyntaxError> {
        if let Some(Token::Word(word)) = self.peek().cloned()
            && !word.eq_ignore_ascii_case("NULL")
        {
            self.at += 1;
            return Ok(Expr::Literal(SqlValue::Text(word)));
        }
        self.expr()
    }

    fn conflict(&mut self) -> Result<Conflict, SyntaxError> {
        for (keyword, conflict) in [
            ("ROLLBACK", Conflict::Rollback),
            ("ABORT", Conflict::Abort),
            ("FAIL", Conflict::Fail),
            ("IGNORE", Conflict::Ignore),
            ("REPLACE", Conflict::Replace),
        ] {
            if self.keyword(keyword) {
                return Ok(conflict);
            }
        }
        Err(self.error())
    }

    /// `ON CONFLICT x` after a constraint, or the default.
    fn conflict_clause(&mut self) -> Result<Conflict, SyntaxError> {
        if word_is(self.peek(), "ON") && word_is(self.peek_at(1), "CONFLICT") {
            self.at += 2;
            return self.conflict();
        }
        Ok(Conflict::Abort)
    }

    fn create_table(&mut self) -> Result<Statement, SyntaxError> {
        let if_not_exists = self.if_not_exists()?;
        let name = self.object_name()?;
        self.expect_symbol("(")?;
        let mut columns = Vec::new();
        let mut keys = Vec::new();
        loop {
            if self.keyword("CONSTRAINT") {
                self.name()?;
            }
            if self.keyword("PRIMARY") {
                self.expect_keyword("KEY")?;
                keys.push(TableKey {
                    primary: true,
                    columns: self.indexed_columns()?,
                    conflict: self.conflict_clause()?,
                });
            } else if self.keyword("UNIQUE") {
                keys.push(TableKey {
                    primary: false,
                    columns: self.indexed_columns()?,
                    conflict: self.conflict_clause()?,
                });
            } else if self.keyword("CHECK") {
                self.skip_parenthesized()?;
            } else if self.keyword("FOREIGN") {
                self.expect_keyword("KEY")?;
                self.indexed_columns()?;
                self.references()?;
            } else {
                columns.push(self.column()?);
            }
            if self.symbol(",") {
                continue;
            }
            self.expect_symbol(")")?;
            break;
        }
        let mut without_rowid = false;
        loop {
            if self.keyword("WITHOUT") {
                if !self.keyword("ROWID") {
                    return Err(self.error());
                }
                without_rowid = true;
            } else if !self.keyword("STRICT") {
                break;
            }
            if !self.symbol(",") {
                break;
            }
        }
        Ok(Statement::CreateTable {
            name,
            if_not_exists,
            columns,
            keys,
            without_rowid,
        })
    }

    fn if_not_exists(&mut self) -> Result<bool, SyntaxError> {
        if self.keyword("IF") {
            self.expect_keyword("NOT")?;
            self.expect_keyword("EXISTS")?;
            return Ok(true);
        }
        Ok(false)
    }

    fn skip_parenthesized(&mut self) -> Result<(), SyntaxError> {
        self.expect_symbol("(")?;
        let mut depth = 1;
        while depth > 0 {
            match self.peek() {
                None => return Err(self.error()),
                Some(Token::Symbol("(")) => depth += 1,
                Some(Token::Symbol(")")) => depth -= 1,
                _ => {}
            }
            self.at += 1;
        }
        Ok(())
    }

    fn references(&mut self) -> Result<(), SyntaxError> {
        self.expect_keyword("REFERENCES")?;
        self.name()?;
        if matches!(self.peek(), Some(Token::Symbol("("))) {
            self.skip_parenthesized()?;
        }
        // ON DELETE/UPDATE actions, MATCH, DEFERRABLE: their words.
        while let Some(Token::Word(word)) = self.peek() {
            let word = word.to_ascii_uppercase();
            if matches!(
                word.as_str(),
                "ON" | "DELETE"
                    | "UPDATE"
                    | "SET"
                    | "NULL"
                    | "DEFAULT"
                    | "CASCADE"
                    | "RESTRICT"
                    | "NO"
                    | "ACTION"
                    | "MATCH"
                    | "DEFERRABLE"
                    | "INITIALLY"
                    | "DEFERRED"
                    | "IMMEDIATE"
                    | "NOT"
            ) {
                self.at += 1;
            } else {
                break;
            }
        }
        Ok(())
    }

    /// `(col [COLLATE x] [ASC|DESC], ...)`.
    fn indexed_columns(&mut self) -> Result<Vec<String>, SyntaxError> {
        self.expect_symbol("(")?;
        let mut columns = Vec::new();
        loop {
            columns.push(self.name()?);
            if self.keyword("COLLATE") {
                self.name()?;
            }
            let _ = self.keyword("ASC") || self.keyword("DESC");
            if self.symbol(",") {
                continue;
            }
            self.expect_symbol(")")?;
            return Ok(columns);
        }
    }

    fn column(&mut self) -> Result<ColumnDef, SyntaxError> {
        let name = self.name()?;
        // The declared type: words, and a size such as `(10, 2)`.
        let mut declared = Vec::new();
        while let Some(Token::Word(word)) = self.peek() {
            let upper = word.to_ascii_uppercase();
            if matches!(
                upper.as_str(),
                "CONSTRAINT"
                    | "PRIMARY"
                    | "NOT"
                    | "NULL"
                    | "UNIQUE"
                    | "CHECK"
                    | "DEFAULT"
                    | "COLLATE"
                    | "REFERENCES"
                    | "GENERATED"
                    | "AS"
            ) {
                break;
            }
            declared.push(word.clone());
            self.at += 1;
        }
        if !declared.is_empty() && matches!(self.peek(), Some(Token::Symbol("("))) {
            self.skip_parenthesized()?;
        }
        let mut column = ColumnDef {
            name,
            declared: declared.join(" "),
            primary_key: false,
            autoincrement: false,
            not_null: false,
            unique: None,
            default: None,
        };
        loop {
            if self.keyword("CONSTRAINT") {
                self.name()?;
            } else if self.keyword("PRIMARY") {
                self.expect_keyword("KEY")?;
                let _ = self.keyword("ASC") || self.keyword("DESC");
                self.conflict_clause()?;
                column.primary_key = true;
                column.autoincrement = self.keyword("AUTOINCREMENT");
            } else if self.keyword("NOT") {
                self.expect_keyword("NULL")?;
                self.conflict_clause()?;
                column.not_null = true;
            } else if self.keyword("NULL") {
            } else if self.keyword("UNIQUE") {
                column.unique = Some(self.conflict_clause()?);
            } else if self.keyword("CHECK") {
                self.skip_parenthesized()?;
            } else if self.keyword("DEFAULT") {
                column.default = Some(if self.symbol("(") {
                    let value = self.expr()?;
                    self.expect_symbol(")")?;
                    value
                } else {
                    self.primary()?
                });
            } else if self.keyword("COLLATE") {
                self.name()?;
            } else if word_is(self.peek(), "REFERENCES") {
                self.references()?;
            } else {
                return Ok(column);
            }
        }
    }

    fn create_index(&mut self, unique: bool) -> Result<Statement, SyntaxError> {
        let if_not_exists = self.if_not_exists()?;
        let name = self.object_name()?;
        self.expect_keyword("ON")?;
        let table = self.object_name()?;
        let columns = self.indexed_columns()?;
        if self.keyword("WHERE") {
            return Err(SyntaxError("partial indexes are not supported".into()));
        }
        Ok(Statement::CreateIndex {
            name,
            table,
            columns,
            unique,
            if_not_exists,
        })
    }

    fn insert(&mut self, conflict: Conflict) -> Result<Statement, SyntaxError> {
        self.expect_keyword("INTO")?;
        let table = self.object_name()?;
        let mut columns = Vec::new();
        if self.symbol("(") {
            loop {
                columns.push(self.name()?);
                if !self.symbol(",") {
                    break;
                }
            }
            self.expect_symbol(")")?;
        }
        let mut rows = Vec::new();
        if self.keyword("DEFAULT") {
            self.expect_keyword("VALUES")?;
            rows.push(Vec::new());
        } else {
            self.expect_keyword("VALUES")?;
            loop {
                self.expect_symbol("(")?;
                let mut row = Vec::new();
                loop {
                    row.push(self.expr()?);
                    if !self.symbol(",") {
                        break;
                    }
                }
                self.expect_symbol(")")?;
                rows.push(row);
                if !self.symbol(",") {
                    break;
                }
            }
        }
        let mut upsert = None;
        if self.keyword("ON") {
            self.expect_keyword("CONFLICT")?;
            let mut target = Vec::new();
            if self.symbol("(") {
                loop {
                    target.push(self.name()?);
                    if !self.symbol(",") {
                        break;
                    }
                }
                self.expect_symbol(")")?;
            }
            self.expect_keyword("DO")?;
            let action = if self.keyword("NOTHING") {
                OnConflict::Nothing
            } else {
                self.expect_keyword("UPDATE")?;
                self.expect_keyword("SET")?;
                let set = self.assignments()?;
                let filter = if self.keyword("WHERE") {
                    Some(self.expr()?)
                } else {
                    None
                };
                OnConflict::Update(set, filter)
            };
            upsert = Some((target, action));
        }
        Ok(Statement::Insert {
            table,
            columns,
            rows,
            conflict,
            upsert,
        })
    }

    fn assignments(&mut self) -> Result<Vec<(String, Expr)>, SyntaxError> {
        let mut set = Vec::new();
        loop {
            let column = self.name()?;
            self.expect_symbol("=")?;
            set.push((column, self.expr()?));
            if !self.symbol(",") {
                return Ok(set);
            }
        }
    }

    fn select(&mut self) -> Result<Select, SyntaxError> {
        self.expect_keyword("SELECT")?;
        let distinct = self.keyword("DISTINCT");
        let _ = distinct || self.keyword("ALL");
        let mut columns = Vec::new();
        loop {
            if self.symbol("*") {
                columns.push(ResultColumn::All);
            } else if matches!(self.peek_at(1), Some(Token::Symbol(".")))
                && matches!(self.peek_at(2), Some(Token::Symbol("*")))
            {
                self.at += 3;
                columns.push(ResultColumn::All);
            } else {
                let expr = self.expr()?;
                let alias = if self.keyword("AS") {
                    Some(self.name()?)
                } else if let Some(Token::Word(word)) = self.peek()
                    && !matches!(
                        word.to_ascii_uppercase().as_str(),
                        "FROM" | "WHERE" | "ORDER" | "LIMIT" | "GROUP"
                    )
                {
                    let word = word.clone();
                    self.at += 1;
                    Some(word)
                } else {
                    None
                };
                columns.push(ResultColumn::Expr(expr, alias));
            }
            if !self.symbol(",") {
                break;
            }
        }
        let from = if self.keyword("FROM") {
            let table = self.object_name()?;
            // An alias is not supported beyond being skipped.
            if self.keyword("AS") {
                self.name()?;
            }
            if self.keyword("INDEXED") {
                self.expect_keyword("BY")?;
                self.name()?;
            }
            Some(table)
        } else {
            None
        };
        let filter = if self.keyword("WHERE") {
            Some(self.expr()?)
        } else {
            None
        };
        let mut order = Vec::new();
        if self.keyword("ORDER") {
            self.expect_keyword("BY")?;
            loop {
                let expr = self.expr()?;
                if self.keyword("COLLATE") {
                    self.name()?;
                }
                let descending = self.keyword("DESC");
                if !descending {
                    self.keyword("ASC");
                }
                order.push((expr, descending));
                if !self.symbol(",") {
                    break;
                }
            }
        }
        let (mut limit, mut offset) = (None, None);
        if self.keyword("LIMIT") {
            let first = self.expr()?;
            if self.keyword("OFFSET") {
                limit = Some(first);
                offset = Some(self.expr()?);
            } else if self.symbol(",") {
                offset = Some(first);
                limit = Some(self.expr()?);
            } else {
                limit = Some(first);
            }
        }
        Ok(Select {
            distinct,
            columns,
            from,
            filter,
            order,
            limit,
            offset,
        })
    }

    fn expr(&mut self) -> Result<Expr, SyntaxError> {
        self.or()
    }

    fn or(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.and()?;
        while self.keyword("OR") {
            left = Expr::Binary("OR", Box::new(left), Box::new(self.and()?));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.not()?;
        while self.keyword("AND") {
            left = Expr::Binary("AND", Box::new(left), Box::new(self.not()?));
        }
        Ok(left)
    }

    fn not(&mut self) -> Result<Expr, SyntaxError> {
        if self.keyword("NOT") {
            return Ok(Expr::Unary("NOT", Box::new(self.not()?)));
        }
        self.equality()
    }

    fn equality(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.comparison()?;
        loop {
            let negated = word_is(self.peek(), "NOT")
                && (word_is(self.peek_at(1), "IN")
                    || word_is(self.peek_at(1), "LIKE")
                    || word_is(self.peek_at(1), "GLOB")
                    || word_is(self.peek_at(1), "BETWEEN"));
            if negated {
                self.at += 1;
            }
            if self.symbol("=") || self.symbol("==") {
                left = Expr::Binary("=", Box::new(left), Box::new(self.comparison()?));
            } else if self.symbol("!=") || self.symbol("<>") {
                left = Expr::Binary("!=", Box::new(left), Box::new(self.comparison()?));
            } else if self.keyword("IS") {
                let not = self.keyword("NOT");
                if self.keyword("NULL") {
                    left = Expr::IsNull(Box::new(left), !not);
                } else {
                    let op = if not { "IS NOT" } else { "IS" };
                    left = Expr::Binary(op, Box::new(left), Box::new(self.comparison()?));
                }
            } else if self.keyword("ISNULL") {
                left = Expr::IsNull(Box::new(left), true);
            } else if self.keyword("NOTNULL") {
                left = Expr::IsNull(Box::new(left), false);
            } else if word_is(self.peek(), "NOT") && word_is(self.peek_at(1), "NULL") {
                self.at += 2;
                left = Expr::IsNull(Box::new(left), false);
            } else if self.keyword("IN") {
                self.expect_symbol("(")?;
                let mut list = Vec::new();
                if !self.symbol(")") {
                    loop {
                        list.push(self.expr()?);
                        if !self.symbol(",") {
                            break;
                        }
                    }
                    self.expect_symbol(")")?;
                }
                left = Expr::In(Box::new(left), list, negated);
            } else if self.keyword("LIKE") {
                let pattern = self.comparison()?;
                let escape = if self.keyword("ESCAPE") {
                    Some(Box::new(self.comparison()?))
                } else {
                    None
                };
                left = Expr::Like(Box::new(left), Box::new(pattern), escape, negated);
            } else if self.keyword("BETWEEN") {
                let low = self.comparison()?;
                self.expect_keyword("AND")?;
                let high = self.comparison()?;
                left = Expr::Between(Box::new(left), Box::new(low), Box::new(high), negated);
            } else {
                if negated {
                    self.at -= 1;
                }
                return Ok(left);
            }
        }
    }

    fn comparison(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.bitwise()?;
        loop {
            let op = ["<=", ">=", "<", ">"]
                .into_iter()
                .find(|op| self.symbol(op));
            match op {
                Some(op) => left = Expr::Binary(op, Box::new(left), Box::new(self.bitwise()?)),
                None => return Ok(left),
            }
        }
    }

    fn bitwise(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.additive()?;
        loop {
            let op = ["<<", ">>", "&", "|"]
                .into_iter()
                .find(|op| self.symbol(op));
            match op {
                Some(op) => left = Expr::Binary(op, Box::new(left), Box::new(self.additive()?)),
                None => return Ok(left),
            }
        }
    }

    fn additive(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.multiplicative()?;
        loop {
            let op = ["+", "-"].into_iter().find(|op| self.symbol(op));
            match op {
                Some(op) => {
                    left = Expr::Binary(op, Box::new(left), Box::new(self.multiplicative()?))
                }
                None => return Ok(left),
            }
        }
    }

    fn multiplicative(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.concatenation()?;
        loop {
            let op = ["*", "/", "%"].into_iter().find(|op| self.symbol(op));
            match op {
                Some(op) => {
                    left = Expr::Binary(op, Box::new(left), Box::new(self.concatenation()?))
                }
                None => return Ok(left),
            }
        }
    }

    fn concatenation(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.unary()?;
        while self.symbol("||") {
            left = Expr::Binary("||", Box::new(left), Box::new(self.unary()?));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, SyntaxError> {
        if self.symbol("-") {
            return Ok(match self.unary()? {
                Expr::Literal(SqlValue::Integer(value)) => {
                    Expr::Literal(SqlValue::Integer(value.wrapping_neg()))
                }
                Expr::Literal(SqlValue::Real(value)) => Expr::Literal(SqlValue::Real(-value)),
                other => Expr::Unary("-", Box::new(other)),
            });
        }
        if self.symbol("+") {
            return self.unary();
        }
        if self.symbol("~") {
            return Ok(Expr::Unary("~", Box::new(self.unary()?)));
        }
        let value = self.primary()?;
        while self.keyword("COLLATE") {
            self.name()?;
        }
        Ok(value)
    }

    fn primary(&mut self) -> Result<Expr, SyntaxError> {
        let Some(token) = self.peek().cloned() else {
            return Err(self.error());
        };
        self.at += 1;
        Ok(match token {
            Token::Integer(value) => Expr::Literal(SqlValue::Integer(value)),
            Token::Real(value) => Expr::Literal(SqlValue::Real(value)),
            Token::Text(text) => Expr::Literal(SqlValue::Text(text)),
            Token::Blob(bytes) => Expr::Literal(SqlValue::Blob(bytes)),
            Token::Parameter(Parameter::Next) => {
                self.next_parameter += 1;
                Expr::Parameter(Parameter::Numbered(self.next_parameter))
            }
            Token::Parameter(Parameter::Numbered(index)) => {
                self.next_parameter = self.next_parameter.max(index);
                Expr::Parameter(Parameter::Numbered(index))
            }
            Token::Parameter(parameter) => Expr::Parameter(parameter),
            Token::Symbol("(") => {
                let inner = self.expr()?;
                self.expect_symbol(")")?;
                inner
            }
            Token::Word(word) if word.eq_ignore_ascii_case("NULL") => Expr::Literal(SqlValue::Null),
            Token::Word(word) if word.eq_ignore_ascii_case("TRUE") => {
                Expr::Literal(SqlValue::Integer(1))
            }
            Token::Word(word) if word.eq_ignore_ascii_case("FALSE") => {
                Expr::Literal(SqlValue::Integer(0))
            }
            Token::Word(word) if word.eq_ignore_ascii_case("CAST") => {
                self.expect_symbol("(")?;
                let value = self.expr()?;
                self.expect_keyword("AS")?;
                let mut kind = Vec::new();
                while let Some(Token::Word(word)) = self.peek() {
                    kind.push(word.clone());
                    self.at += 1;
                }
                if matches!(self.peek(), Some(Token::Symbol("("))) {
                    self.skip_parenthesized()?;
                }
                self.expect_symbol(")")?;
                Expr::Cast(Box::new(value), kind.join(" "))
            }
            Token::Word(word) if word.eq_ignore_ascii_case("CASE") => {
                let operand = if word_is(self.peek(), "WHEN") {
                    None
                } else {
                    Some(Box::new(self.expr()?))
                };
                let mut arms = Vec::new();
                while self.keyword("WHEN") {
                    let when = self.expr()?;
                    self.expect_keyword("THEN")?;
                    arms.push((when, self.expr()?));
                }
                let otherwise = if self.keyword("ELSE") {
                    Some(Box::new(self.expr()?))
                } else {
                    None
                };
                self.expect_keyword("END")?;
                Expr::Case(operand, arms, otherwise)
            }
            Token::Word(name) | Token::Quoted(name) => {
                if matches!(self.peek(), Some(Token::Symbol("("))) {
                    self.at += 1;
                    let distinct = self.keyword("DISTINCT");
                    let mut arguments = Vec::new();
                    if self.symbol("*") {
                        arguments.push(Expr::Literal(SqlValue::Integer(1)));
                    } else if !matches!(self.peek(), Some(Token::Symbol(")"))) {
                        loop {
                            arguments.push(self.expr()?);
                            if !self.symbol(",") {
                                break;
                            }
                        }
                    }
                    self.expect_symbol(")")?;
                    Expr::Call(name.to_ascii_lowercase(), arguments, distinct)
                } else if self.symbol(".") {
                    let column = self.name()?;
                    Expr::Column(Some(name), column)
                } else {
                    Expr::Column(None, name)
                }
            }
            _ => {
                self.at -= 1;
                return Err(self.error());
            }
        })
    }
}

fn display(token: &Token) -> String {
    match token {
        Token::Word(word) => word.clone(),
        Token::Quoted(word) => alloc::format!("\"{word}\""),
        Token::Text(text) => alloc::format!("'{text}'"),
        Token::Blob(_) => "x''".into(),
        Token::Integer(value) => value.to_string(),
        Token::Real(value) => alloc::format!("{value}"),
        Token::Parameter(_) => "?".into(),
        Token::Symbol(symbol) => (*symbol).into(),
    }
}

/// A parsed statement: what it does, the `?` parameters it takes, and its
/// text without a trailing `;`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Parsed {
    pub(crate) statement: Statement,
    pub(crate) parameters: usize,
    pub(crate) text: String,
}

/// The statements of `sql`, in order.
pub(crate) fn parse(sql: &str) -> Result<Vec<Parsed>, SyntaxError> {
    let (tokens, starts) = tokens(sql)?;
    let chars: Vec<char> = sql.chars().collect();
    let mut parser = Parser {
        tokens,
        starts,
        chars,
        at: 0,
        next_parameter: 0,
    };
    let mut statements = Vec::new();
    loop {
        while parser.symbol(";") {}
        if parser.peek().is_none() {
            return Ok(statements);
        }
        parser.next_parameter = 0;
        let first = parser.at;
        let statement = parser.statement()?;
        let text = parser.text_from(first);
        statements.push(Parsed {
            statement,
            parameters: parser.next_parameter,
            text,
        });
        if parser.peek().is_some() && !parser.symbol(";") {
            return Err(parser.error());
        }
    }
}

#[cfg(test)]
mod tests;
