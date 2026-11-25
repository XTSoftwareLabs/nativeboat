use super::*;
use alloc::rc::Rc;
use core::cell::RefCell;

/// A file in memory that tests can read back.
#[derive(Clone, Default)]
struct Shared(Rc<RefCell<Option<Vec<u8>>>>);

impl Storage for Shared {
    fn read(&mut self) -> Result<Option<Stored>, SqlError> {
        Ok(self.0.borrow().clone().map(|bytes| (bytes, None)))
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), SqlError> {
        *self.0.borrow_mut() = Some(bytes.to_vec());
        Ok(())
    }
}

fn text(value: &str) -> SqlValue {
    SqlValue::Text(value.into())
}

fn rows(database: &mut Database, sql: &str) -> Vec<Vec<SqlValue>> {
    database.execute(sql, &Params::default()).unwrap().rows
}

/// The statement the schema stores for a table or index, and its table.
fn stored(database: &Database, name: &str) -> (String, String) {
    for table in &database.state.tables {
        if table.name == name {
            return (table.sql.clone(), table.name.clone());
        }
        for index in &table.indexes {
            if index.name == name {
                return (index.sql.clone().unwrap_or_default(), table.name.clone());
            }
        }
    }
    panic!("{name} is not in the schema")
}

fn with(values: &[SqlValue]) -> Params {
    Params {
        positional: values.to_vec(),
        named: Vec::new(),
    }
}

/// `ALTER TABLE` as VS Code's agent host migrates its databases: columns
/// added with their defaults in existing rows, a table renamed with its
/// indexes, the stored statements rewritten as SQLite rewrites them, and
/// all of it read back from the file.
#[test]
fn alter_table_adds_columns_and_renames_tables() {
    let file = Shared::default();
    let mut database = Database::open(Box::new(file.clone())).unwrap();
    database
        .execute(
            "CREATE TABLE turns (id INTEGER PRIMARY KEY AUTOINCREMENT, text TEXT); CREATE INDEX turns_text ON turns(text); INSERT INTO turns (text) VALUES ('a')",
            &Params::default(),
        )
        .unwrap();
    database
        .execute(
            "ALTER TABLE turns ADD COLUMN event_id TEXT; ALTER TABLE turns ADD kind TEXT NOT NULL DEFAULT 'local'; ALTER TABLE turns ADD COLUMN archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1))",
            &Params::default(),
        )
        .unwrap();
    assert_eq!(
        rows(
            &mut database,
            "SELECT id, text, event_id, kind, archived FROM turns"
        ),
        [vec![
            SqlValue::Integer(1),
            text("a"),
            SqlValue::Null,
            text("local"),
            SqlValue::Integer(0)
        ]]
    );
    assert_eq!(
        stored(&database, "turns").0,
        "CREATE TABLE turns (id INTEGER PRIMARY KEY AUTOINCREMENT, text TEXT, event_id TEXT, kind TEXT NOT NULL DEFAULT 'local', archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)))"
    );
    database
        .execute("ALTER TABLE turns RENAME TO turns_v2", &Params::default())
        .unwrap();
    database
        .execute(
            "INSERT INTO turns_v2 (text) VALUES ('b')",
            &Params::default(),
        )
        .unwrap();
    assert_eq!(
        stored(&database, "turns_v2").0,
        "CREATE TABLE \"turns_v2\" (id INTEGER PRIMARY KEY AUTOINCREMENT, text TEXT, event_id TEXT, kind TEXT NOT NULL DEFAULT 'local', archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)))"
    );
    assert_eq!(
        stored(&database, "turns_text"),
        (
            "CREATE INDEX turns_text ON \"turns_v2\"(text)".to_owned(),
            "turns_v2".to_owned()
        )
    );
    assert_eq!(
        rows(&mut database, "SELECT name, seq FROM sqlite_sequence"),
        [vec![text("turns_v2"), SqlValue::Integer(2)]]
    );
    // The file opens again with the new shape.
    let mut reopened = Database::open(Box::new(file.clone())).unwrap();
    assert_eq!(
        rows(&mut reopened, "SELECT text, kind FROM turns_v2 ORDER BY id"),
        [
            vec![text("a"), text("local")],
            vec![text("b"), text("local")]
        ]
    );
    assert_eq!(
        rows(&mut reopened, "PRAGMA integrity_check"),
        [vec![text("ok")]]
    );
    // SQLite's refusals.
    for (sql, message) in [
        (
            "ALTER TABLE turns_v2 ADD COLUMN kind TEXT",
            "duplicate column name: kind",
        ),
        (
            "ALTER TABLE turns_v2 ADD COLUMN must TEXT NOT NULL",
            "Cannot add a NOT NULL column with default value NULL",
        ),
        (
            "ALTER TABLE turns_v2 ADD COLUMN key INTEGER PRIMARY KEY",
            "Cannot add a PRIMARY KEY column",
        ),
        (
            "ALTER TABLE turns_v2 ADD COLUMN code TEXT UNIQUE",
            "Cannot add a UNIQUE column",
        ),
        ("ALTER TABLE missing ADD COLUMN x", "no such table: missing"),
        (
            "ALTER TABLE turns_v2 RENAME TO turns_text",
            "there is already another table or index with this name: turns_text",
        ),
    ] {
        let error = reopened.execute(sql, &Params::default()).unwrap_err();
        assert!(error.message.contains(message), "{sql}: {}", error.message);
    }
}

/// VS Code's storage, as `storage.ts` runs it, persisted and read back.
#[test]
fn vs_code_storage_round_trips() {
    let file = Shared::default();
    let mut database = Database::open(Box::new(file.clone())).unwrap();
    database
        .execute("PRAGMA user_version = 1; PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)", &Params::default())
        .unwrap();
    let before = file.0.borrow().clone();
    database
        .execute("BEGIN TRANSACTION", &Params::default())
        .unwrap();
    let upsert = "INSERT INTO ItemTable VALUES (?,?),(?,?) ON CONFLICT (key) DO UPDATE SET value = excluded.value WHERE value != excluded.value";
    database
        .execute(upsert, &with(&[text("a"), text("1"), text("b"), text("2")]))
        .unwrap();
    assert_eq!(
        *file.0.borrow(),
        before,
        "a transaction writes when it commits"
    );
    database
        .execute("END TRANSACTION", &Params::default())
        .unwrap();
    assert_ne!(*file.0.borrow(), before);
    let outcome = database
        .execute(upsert, &with(&[text("a"), text("1"), text("b"), text("3")]))
        .unwrap();
    assert_eq!(outcome.changes, 1, "an unchanged value is not updated");
    database
        .execute(
            "DELETE FROM ItemTable WHERE key IN (?)",
            &with(&[text("a")]),
        )
        .unwrap();
    database
        .execute(
            "INSERT INTO ItemTable VALUES (?, ?)",
            &with(&[text("b"), text("4")]),
        )
        .unwrap();
    assert_eq!(
        rows(&mut database, "SELECT * FROM ItemTable"),
        [vec![text("b"), text("4")]]
    );
    // The file opens again with its rows, its unique key and its version.
    let mut reopened = Database::open(Box::new(file.clone())).unwrap();
    assert_eq!(
        rows(&mut reopened, "SELECT * FROM ItemTable"),
        [vec![text("b"), text("4")]]
    );
    assert_eq!(
        rows(&mut reopened, "PRAGMA user_version"),
        [vec![SqlValue::Integer(1)]]
    );
    reopened
        .execute(
            "INSERT INTO ItemTable VALUES ('b', '5')",
            &Params::default(),
        )
        .unwrap();
    assert_eq!(
        rows(&mut reopened, "SELECT key, value FROM ItemTable"),
        [vec![text("b"), text("5")]]
    );
    assert_eq!(
        rows(&mut reopened, "PRAGMA integrity_check"),
        [vec![text("ok")]]
    );
    reopened.execute("VACUUM", &Params::default()).unwrap();
    // For checking with SQLite itself.
    if let Some(path) = std::env::var_os("NATIVEBOAT_SQLITE_DUMP") {
        std::fs::write(path, file.0.borrow().as_ref().unwrap()).unwrap();
    }
}

#[test]
fn constraints_conflicts_and_rollback() {
    let mut database = Database::open(Box::new(Memory)).unwrap();
    database
        .execute("CREATE TABLE t (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, n INTEGER DEFAULT 7)", &Params::default())
        .unwrap();
    database
        .execute(
            "INSERT INTO t (name) VALUES ('a'), ('b')",
            &Params::default(),
        )
        .unwrap();
    let duplicate = database
        .execute("INSERT INTO t (name) VALUES ('a')", &Params::default())
        .unwrap_err();
    assert_eq!(
        (duplicate.code, duplicate.message.as_str()),
        ("SQLITE_CONSTRAINT", "UNIQUE constraint failed: t.name")
    );
    let null = database
        .execute("INSERT INTO t (name) VALUES (NULL)", &Params::default())
        .unwrap_err();
    assert_eq!(null.message, "NOT NULL constraint failed: t.name");
    database
        .execute(
            "INSERT OR IGNORE INTO t (name) VALUES ('a')",
            &Params::default(),
        )
        .unwrap();
    database
        .execute(
            "INSERT OR REPLACE INTO t (name, n) VALUES ('a', 9)",
            &Params::default(),
        )
        .unwrap();
    assert_eq!(
        rows(&mut database, "SELECT id, name, n FROM t ORDER BY id"),
        [
            vec![SqlValue::Integer(2), text("b"), SqlValue::Integer(7)],
            vec![SqlValue::Integer(3), text("a"), SqlValue::Integer(9)],
        ]
    );
    database
        .execute("DELETE FROM t WHERE id = 3", &Params::default())
        .unwrap();
    // AUTOINCREMENT does not reuse rowids.
    let outcome = database
        .execute("INSERT INTO t (name) VALUES ('c')", &Params::default())
        .unwrap();
    assert_eq!(outcome.last_insert_rowid, 4);
    database.execute("BEGIN", &Params::default()).unwrap();
    database
        .execute("UPDATE t SET n = n + 1", &Params::default())
        .unwrap();
    database.execute("ROLLBACK", &Params::default()).unwrap();
    assert_eq!(
        rows(&mut database, "SELECT sum(n), count(*), max(name) FROM t"),
        [vec![SqlValue::Integer(14), SqlValue::Integer(2), text("c")]]
    );
    let renamed = database
        .execute("UPDATE t SET name = 'b' WHERE id = 4", &Params::default())
        .unwrap_err();
    assert_eq!(renamed.message, "UNIQUE constraint failed: t.name");
    // A failing statement leaves no change behind.
    assert_eq!(
        rows(&mut database, "SELECT name FROM t WHERE id = 4"),
        [vec![text("c")]]
    );
}

#[test]
fn expressions_compare_and_compute_as_sqlite_does() {
    let mut database = Database::open(Box::new(Memory)).unwrap();
    database
        .execute(
            "CREATE TABLE v (i INTEGER, t TEXT, r REAL, b)",
            &Params::default(),
        )
        .unwrap();
    database
        .execute(
            "INSERT INTO v VALUES ('12', 34, '1.5', x'00')",
            &Params::default(),
        )
        .unwrap();
    assert_eq!(
        rows(
            &mut database,
            "SELECT typeof(i), typeof(t), typeof(r), typeof(b), i = '12', t = 34, t < 4 FROM v"
        ),
        [vec![
            text("integer"),
            text("text"),
            text("real"),
            text("blob"),
            SqlValue::Integer(1),
            SqlValue::Integer(1),
            SqlValue::Integer(1),
        ]]
    );
    assert_eq!(
        rows(
            &mut database,
            "SELECT 7 / 2, 7 / 2.0, 7 % 0, 'a' || 1, NULL = NULL, NULL IS NULL, 2 BETWEEN 1 AND 3, 'Abc' LIKE 'a%', upper('x'), substr('hello', 2, 3), coalesce(NULL, 5), CASE WHEN 1 > 2 THEN 'no' ELSE 'yes' END"
        ),
        [vec![
            SqlValue::Integer(3),
            SqlValue::Real(3.5),
            SqlValue::Null,
            text("a1"),
            SqlValue::Null,
            SqlValue::Integer(1),
            SqlValue::Integer(1),
            SqlValue::Integer(1),
            text("X"),
            text("ell"),
            SqlValue::Integer(5),
            text("yes"),
        ]]
    );
    let named = Params {
        positional: Vec::new(),
        named: vec![(":who".into(), text("me"))],
    };
    assert_eq!(
        database.execute("SELECT :who", &named).unwrap().rows,
        [vec![text("me")]]
    );
    let error = database
        .execute("SELECT nope FROM v", &Params::default())
        .unwrap_err();
    assert_eq!(error.message, "no such column: nope");
    assert_eq!(
        database
            .execute("SELECT * FROM missing", &Params::default())
            .unwrap_err()
            .message,
        "no such table: missing"
    );
}

/// A file SQLite wrote opens, and its tables run statements.
#[test]
fn databases_sqlite_wrote_open() {
    let bytes = include_bytes!("../format/fixtures/items.db").to_vec();
    let file = Shared(Rc::new(RefCell::new(Some(bytes))));
    let mut database = Database::open(Box::new(file.clone())).unwrap();
    assert_eq!(
        rows(&mut database, "SELECT count(*) FROM ItemTable"),
        [vec![SqlValue::Integer(41)]]
    );
    assert_eq!(
        rows(
            &mut database,
            "SELECT length(value) FROM ItemTable WHERE key = 'large'"
        ),
        [vec![SqlValue::Integer(5000)]]
    );
    database
        .execute(
            "INSERT INTO ItemTable VALUES ('key.000', 'replaced')",
            &Params::default(),
        )
        .unwrap();
    assert_eq!(
        rows(&mut database, "SELECT count(*) FROM ItemTable"),
        [vec![SqlValue::Integer(41)]]
    );
    // The view survives the rewrite.
    let reopened = format::File::read(file.0.borrow().as_ref().unwrap(), None).unwrap();
    assert!(
        reopened
            .schema()
            .unwrap()
            .iter()
            .any(|entry| entry.kind == "view" && entry.name == "keys")
    );
}
