use super::*;
use alloc::string::ToString;

fn text(value: &str) -> SqlValue {
    SqlValue::Text(value.to_string())
}

#[test]
fn varints_take_one_to_nine_bytes() {
    for value in [
        0,
        1,
        127,
        128,
        16_383,
        16_384,
        1 << 35,
        0x00FF_FFFF_FFFF_FFFF,
        u64::MAX,
        1 << 63,
    ] {
        let mut bytes = Vec::new();
        put_varint(&mut bytes, value);
        assert_eq!(varint(&bytes, 0).unwrap(), (value, bytes.len()), "{value}");
    }
    let mut nine = Vec::new();
    put_varint(&mut nine, u64::MAX);
    assert_eq!(nine.len(), 9);
}

#[test]
fn records_keep_each_storage_class() {
    let values = vec![
        SqlValue::Null,
        SqlValue::Integer(0),
        SqlValue::Integer(1),
        SqlValue::Integer(-1),
        SqlValue::Integer(300),
        SqlValue::Integer(-8_388_608),
        SqlValue::Integer(1 << 40),
        SqlValue::Integer(i64::MIN),
        SqlValue::Real(-2.5),
        text("héllo"),
        SqlValue::Blob(vec![0, 255]),
        text(&"long ".repeat(40)),
    ];
    assert_eq!(decode_record(&encode_record(&values)).unwrap(), values);
}

/// A file SQLite wrote: its schema, an automatic index, a value on
/// overflow pages, numbers of each width, a view and the user version.
#[test]
fn files_sqlite_wrote_are_read() {
    let file = File::read(include_bytes!("fixtures/items.db"), None).unwrap();
    assert_eq!(file.header.page_size, 1024);
    assert_eq!(file.header.user_version, 7);
    let schema = file.schema().unwrap();
    let names: Vec<(&str, &str)> = schema
        .iter()
        .map(|entry| (entry.kind.as_str(), entry.name.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            ("table", "ItemTable"),
            ("index", "sqlite_autoindex_ItemTable_1"),
            ("table", "numbers"),
            ("view", "keys"),
        ]
    );
    assert_eq!(schema[1].sql, None);
    let items = file.table(schema[0].root).unwrap();
    assert_eq!(items.len(), 41);
    assert_eq!(items[0], (1, vec![text("key.000"), text("value 0")]));
    assert_eq!(items[40].1, vec![text("large"), text(&"x".repeat(5000))]);
    let keys = file.index(schema[1].root).unwrap();
    assert_eq!(keys.len(), 41);
    assert_eq!(keys[0], vec![text("key.000"), SqlValue::Integer(1)]);
    assert_eq!(keys[40], vec![text("large"), SqlValue::Integer(41)]);
    let numbers = file.table(schema[2].root).unwrap();
    let integers: Vec<SqlValue> = numbers.iter().map(|(_, row)| row[0].clone()).collect();
    let expected = [
        0,
        1,
        -1,
        127,
        -128,
        300,
        -40_000,
        8_388_607,
        1 << 31,
        -(1_i64 << 40),
        1 << 62,
    ];
    for (value, expected) in integers.iter().zip(expected) {
        assert_eq!(*value, SqlValue::Integer(expected));
    }
    assert_eq!(numbers[3].1[1], SqlValue::Real(127.0 / 3.0));
    assert_eq!(numbers[11].1, vec![SqlValue::Null; 4]);
}

/// A database in WAL mode reads as of its log's last commit.
#[test]
fn write_ahead_logs_replace_the_pages_they_commit() {
    let bytes = include_bytes!("fixtures/wal.db");
    let without = File::read(bytes, None).unwrap();
    let root = without.schema().unwrap()[0].root;
    assert_eq!(without.table(root).unwrap().len(), 1);
    let with = File::read(bytes, Some(include_bytes!("fixtures/wal.db-wal"))).unwrap();
    let rows: Vec<SqlValue> = with
        .table(root)
        .unwrap()
        .into_iter()
        .map(|(_, row)| row[0].clone())
        .collect();
    assert_eq!(rows, [text("in the file"), text("in the log")]);
    // A log whose checksum fails is not applied.
    let mut broken = include_bytes!("fixtures/wal.db-wal").to_vec();
    let last = broken.len() - 1;
    broken[last] ^= 1;
    let damaged = File::read(bytes, Some(&broken)).unwrap();
    assert_eq!(damaged.table(root).unwrap().len(), 1);
}

/// Written files read back: deep tables and indexes, values on overflow
/// pages, a schema larger than page 1, views and the header's fields.
#[test]
fn written_files_read_back() {
    let rows: Vec<(i64, Vec<SqlValue>)> = (1..=3000)
        .map(|rowid| {
            let value = if rowid % 500 == 0 {
                SqlValue::Blob(vec![rowid as u8; 3000])
            } else {
                SqlValue::Integer(rowid * 7)
            };
            (rowid, vec![text(&alloc::format!("key {rowid:05}")), value])
        })
        .chain(core::iter::once((
            9999,
            vec![text(&"k".repeat(2000)), SqlValue::Null],
        )))
        .collect();
    let mut keys: Vec<Vec<SqlValue>> = rows
        .iter()
        .map(|(rowid, row)| vec![row[0].clone(), SqlValue::Integer(*rowid)])
        .collect();
    keys.sort_by(|left, right| crate::sqlite::compare_records(left, right));
    let mut objects = vec![
        Object {
            kind: "table".into(),
            name: "items".into(),
            table: "items".into(),
            sql: Some("CREATE TABLE items (key TEXT, value)".into()),
            content: Content::Table(rows.clone()),
        },
        Object {
            kind: "index".into(),
            name: "items_key".into(),
            table: "items".into(),
            sql: Some("CREATE INDEX items_key ON items (key)".into()),
            content: Content::Index(keys.clone()),
        },
        Object {
            kind: "view".into(),
            name: "v".into(),
            table: "v".into(),
            sql: Some("CREATE VIEW v AS SELECT 1".into()),
            content: Content::None,
        },
    ];
    for index in 0..30 {
        objects.push(Object {
            kind: "table".into(),
            name: alloc::format!("empty_{index}"),
            table: alloc::format!("empty_{index}"),
            sql: Some(alloc::format!(
                "CREATE TABLE empty_{index} (a, b, c, d, e, f, g, h)"
            )),
            content: Content::Table(Vec::new()),
        });
    }
    let header = Header {
        page_size: 1024,
        change_counter: 41,
        schema_cookie: 9,
        user_version: 3,
        application_id: 12,
    };
    let bytes = write(&header, &objects);
    assert_eq!(bytes.len() % 1024, 0);
    // For checking with SQLite itself: `PRAGMA integrity_check` on the file.
    if let Some(path) = std::env::var_os("NATIVEBOAT_SQLITE_DUMP") {
        std::fs::write(path, &bytes).unwrap();
    }
    let file = File::read(&bytes, None).unwrap();
    assert_eq!(
        file.header,
        Header {
            change_counter: 42,
            ..header
        }
    );
    let schema = file.schema().unwrap();
    assert_eq!(schema.len(), 33);
    assert_eq!(schema[2].root, 0);
    assert_eq!(
        schema[1].sql.as_deref(),
        Some("CREATE INDEX items_key ON items (key)")
    );
    assert_eq!(file.table(schema[0].root).unwrap(), rows);
    assert_eq!(file.index(schema[1].root).unwrap(), keys);
    assert!(file.table(schema[32].root).unwrap().is_empty());
}

/// Each page size SQLite allows writes files that read back, with keys and
/// values around the overflow limits.
#[test]
fn every_page_size_writes_readable_files() {
    for page_size in [512_u32, 4096, 65_536] {
        let rows: Vec<(i64, Vec<SqlValue>)> = (1..=600)
            .map(|rowid| {
                let size = (rowid as usize * 37) % (page_size as usize * 2);
                (
                    rowid,
                    vec![
                        SqlValue::Blob(vec![7; size]),
                        text(&alloc::format!("{rowid:04}")),
                    ],
                )
            })
            .collect();
        let mut keys: Vec<Vec<SqlValue>> = rows
            .iter()
            .map(|(rowid, row)| vec![row[1].clone(), row[0].clone(), SqlValue::Integer(*rowid)])
            .collect();
        keys.sort_by(|left, right| crate::sqlite::compare_records(left, right));
        let objects = [
            Object {
                kind: "table".into(),
                name: "t".into(),
                table: "t".into(),
                sql: Some("CREATE TABLE t (a BLOB, b TEXT)".into()),
                content: Content::Table(rows.clone()),
            },
            Object {
                kind: "index".into(),
                name: "t_ba".into(),
                table: "t".into(),
                sql: Some("CREATE INDEX t_ba ON t (b, a)".into()),
                content: Content::Index(keys.clone()),
            },
        ];
        let header = Header {
            page_size,
            ..Header::default()
        };
        let bytes = write(&header, &objects);
        if let Some(path) = std::env::var_os("NATIVEBOAT_SQLITE_DUMP") {
            let mut path = std::path::PathBuf::from(path);
            path.set_extension(alloc::format!("{page_size}.db"));
            std::fs::write(path, &bytes).unwrap();
        }
        let file = File::read(&bytes, None).unwrap();
        let schema = file.schema().unwrap();
        assert_eq!(file.table(schema[0].root).unwrap(), rows, "{page_size}");
        assert_eq!(file.index(schema[1].root).unwrap(), keys, "{page_size}");
    }
}
