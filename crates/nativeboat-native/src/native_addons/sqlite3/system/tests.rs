use super::*;

fn text(value: &str) -> SqlValue {
    SqlValue::Text(value.into())
}

/// A private database on the system's SQLite; `None` on systems without one.
fn memory() -> Option<Database> {
    let database = Database::open(":memory:", 0x2 | 0x4).map(|opened| opened.unwrap());
    if database.is_none() {
        std::eprintln!("skipped: this system has no SQLite 3.38 or later");
    }
    database
}

/// The SQL VS Code's agent host runs, which the built-in engine does not
/// parse: joins, subqueries, compound selects, `INSERT … SELECT`, JSON
/// functions, `RETURNING`, and the schema table.
#[test]
fn system_sqlite_runs_joins_subqueries_and_json() {
    let Some(mut database) = memory() else {
        return;
    };
    database
        .execute(
            "CREATE TABLE sessions (uri TEXT PRIMARY KEY, start INTEGER);
             CREATE TABLE chats (uri TEXT, session TEXT, payload TEXT);
             CREATE TEMP TABLE pending (uri TEXT);",
            &Params::default(),
        )
        .unwrap();
    let insert = database
        .execute(
            "INSERT INTO sessions VALUES (?, ?), (?, ?)",
            &Params {
                positional: alloc::vec![
                    text("a"),
                    SqlValue::Integer(1),
                    text("b"),
                    SqlValue::Integer(2)
                ],
                named: Vec::new(),
            },
        )
        .unwrap();
    assert_eq!((insert.changes, insert.last_insert_rowid), (2, 2));
    database
        .execute(
            "INSERT INTO chats SELECT 'c1', uri, json_object('n', start) FROM sessions WHERE uri = :uri",
            &Params {
                positional: Vec::new(),
                named: alloc::vec![(String::from(":uri"), text("a"))],
            },
        )
        .unwrap();
    let joined = database
        .execute(
            "SELECT s.uri AS session, c.uri AS chat, json_extract(c.payload, '$.n') AS n
             FROM sessions AS s LEFT JOIN chats AS c ON c.session = s.uri
             WHERE EXISTS (SELECT 1 FROM sessions WHERE uri = s.uri)
             ORDER BY s.uri",
            &Params::default(),
        )
        .unwrap();
    assert_eq!(joined.columns, ["session", "chat", "n"]);
    assert_eq!(
        joined.rows,
        [
            alloc::vec![text("a"), text("c1"), SqlValue::Integer(1)],
            alloc::vec![text("b"), SqlValue::Null, SqlValue::Null],
        ]
    );
    let each = database
        .execute(
            "SELECT group_concat(value, ',') AS values_ FROM json_each('[1,2,3]')
             UNION ALL SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'chats'",
            &Params::default(),
        )
        .unwrap();
    assert_eq!(
        each.rows,
        [alloc::vec![text("1,2,3")], alloc::vec![text("chats")]]
    );
    let returned = database
        .run_first(
            "UPDATE sessions SET start = start + 10 RETURNING uri, start; SELECT 'not run'",
            &Params::default(),
        )
        .unwrap();
    assert_eq!(returned.rows.len(), 2);
    assert_eq!(returned.columns, ["uri", "start"]);
    // Blobs and reals keep their values.
    let values = database
        .execute(
            "SELECT ?, ?",
            &Params {
                positional: alloc::vec![SqlValue::Blob(alloc::vec![0, 255]), SqlValue::Real(1.5)],
                named: Vec::new(),
            },
        )
        .unwrap();
    assert_eq!(
        values.rows,
        [alloc::vec![
            SqlValue::Blob(alloc::vec![0, 255]),
            SqlValue::Real(1.5)
        ]]
    );
}

/// Errors carry SQLite's code names and messages, as node-sqlite3 reports
/// them, from preparing and from running.
#[test]
fn system_sqlite_reports_errors_as_sqlite_does() {
    let Some(mut database) = memory() else {
        return;
    };
    let syntax = database.check("SELEC 1").unwrap_err();
    assert_eq!(syntax.code, "SQLITE_ERROR");
    assert_eq!(syntax.message, "near \"SELEC\": syntax error");
    let missing = database.check("SELECT * FROM absent").unwrap_err();
    assert_eq!(missing.message, "no such table: absent");
    database
        .execute(
            "CREATE TABLE t (k TEXT UNIQUE); INSERT INTO t VALUES ('x')",
            &Params::default(),
        )
        .unwrap();
    let duplicate = database
        .execute("INSERT INTO t VALUES ('x')", &Params::default())
        .unwrap_err();
    assert_eq!(duplicate.code, "SQLITE_CONSTRAINT");
    assert_eq!(duplicate.message, "UNIQUE constraint failed: t.k");
    // A file that cannot be created reports SQLITE_CANTOPEN.
    let blocked = Database::open("nativeboat-missing-folder\\db.sqlite", 0x2 | 0x4)
        .unwrap()
        .err()
        .unwrap();
    assert_eq!(blocked.code, "SQLITE_CANTOPEN");
}
