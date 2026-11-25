use super::*;

fn one(sql: &str) -> Parsed {
    let mut parsed = parse(sql).unwrap();
    assert_eq!(parsed.len(), 1, "{sql}");
    parsed.pop().unwrap()
}

#[test]
fn vs_code_storage_statements_parse() {
    let create = one(
        "CREATE TABLE IF NOT EXISTS ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)",
    );
    let Statement::CreateTable {
        name,
        if_not_exists,
        columns,
        keys,
        without_rowid,
    } = create.statement
    else {
        panic!("a table");
    };
    assert_eq!(
        (name.as_str(), if_not_exists, without_rowid),
        ("ItemTable", true, false)
    );
    assert_eq!(columns[0].unique, Some(Conflict::Replace));
    assert_eq!(columns[1].declared, "BLOB");
    assert!(keys.is_empty());
    let insert = one(
        "INSERT INTO ItemTable VALUES (?,?),(?,?) ON CONFLICT (key) DO UPDATE SET value = excluded.value WHERE value != excluded.value",
    );
    assert_eq!(insert.parameters, 4);
    let Statement::Insert {
        rows,
        upsert: Some((target, OnConflict::Update(set, Some(_)))),
        ..
    } = insert.statement
    else {
        panic!("an upsert");
    };
    assert_eq!(rows.len(), 2);
    assert_eq!(target, ["key"]);
    assert_eq!(
        set[0].1,
        Expr::Column(Some("excluded".into()), "value".into())
    );
    let delete = one("DELETE FROM ItemTable WHERE key IN (?,?,?)");
    assert_eq!(delete.parameters, 3);
    let pragmas =
        parse("PRAGMA user_version = 1; PRAGMA journal_mode=WAL; PRAGMA busy_timeout=2000;")
            .unwrap();
    assert_eq!(pragmas.len(), 3);
    assert_eq!(
        pragmas[1].statement,
        Statement::Pragma {
            name: "journal_mode".into(),
            value: Some(Expr::Literal(SqlValue::Text("WAL".into())))
        }
    );
    assert_eq!(pragmas[0].text, "PRAGMA user_version = 1");
    assert!(matches!(
        one("BEGIN TRANSACTION").statement,
        Statement::Begin
    ));
    assert!(matches!(
        one("END TRANSACTION").statement,
        Statement::Commit
    ));
    assert!(matches!(one("VACUUM").statement, Statement::Vacuum));
}

#[test]
fn expressions_follow_sqlite_precedence() {
    let Statement::Select(select) = one("SELECT a + b * 2 AS x, -1, 'it''s', x'0aFF', NOT a = 1 OR b < 2 FROM t WHERE a IS NOT NULL ORDER BY 1 DESC LIMIT 3 OFFSET ?").statement else {
        panic!("a query");
    };
    let ResultColumn::Expr(first, Some(alias)) = &select.columns[0] else {
        panic!("an aliased column");
    };
    assert_eq!(alias, "x");
    assert!(
        matches!(first, Expr::Binary("+", _, right) if matches!(**right, Expr::Binary("*", _, _)))
    );
    assert_eq!(
        select.columns[1],
        ResultColumn::Expr(Expr::Literal(SqlValue::Integer(-1)), None)
    );
    assert_eq!(
        select.columns[2],
        ResultColumn::Expr(Expr::Literal(SqlValue::Text("it's".into())), None)
    );
    assert_eq!(
        select.columns[3],
        ResultColumn::Expr(Expr::Literal(SqlValue::Blob(vec![10, 255])), None)
    );
    assert!(
        matches!(&select.columns[4], ResultColumn::Expr(Expr::Binary("OR", left, _), None) if matches!(**left, Expr::Unary("NOT", _)))
    );
    assert!(matches!(select.filter, Some(Expr::IsNull(_, false))));
    assert_eq!(select.order.len(), 1);
    assert!(select.order[0].1);
    assert!(select.offset.is_some());
}

#[test]
fn errors_name_where_parsing_stopped() {
    assert_eq!(
        parse("SELEC 1").unwrap_err().0,
        "near \"SELEC\": syntax error"
    );
    assert_eq!(parse("SELECT (1").unwrap_err().0, "incomplete input");
    assert_eq!(
        parse("SELECT 'open").unwrap_err().0,
        "unrecognized token: \"'open\""
    );
    assert_eq!(
        parse("-- just a comment\n/* and another */").unwrap(),
        Vec::new()
    );
}
