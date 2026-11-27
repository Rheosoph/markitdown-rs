//! SQLite conversion tests against a database built on the fly
use bytes::Bytes;
use markitdown::{model::ConversionOptions, MarkItDown};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};

async fn database_bytes() -> Bytes {
    let file = tempfile::NamedTempFile::new().expect("temp database file");
    let pool = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(file.path())
            .create_if_missing(true),
    )
    .await
    .expect("open temp database");
    for statement in [
        "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL, score REAL)",
        "INSERT INTO people (name, score) VALUES ('Ada', 9.5), ('Grace', NULL)",
        "CREATE TABLE \"odd \"\"name\"\"\" (label TEXT)",
        "INSERT INTO \"odd \"\"name\"\"\" (label) VALUES ('quoted')",
    ] {
        sqlx::query(statement)
            .execute(&pool)
            .await
            .expect("seed temp database");
    }
    pool.close().await;
    Bytes::from(std::fs::read(file.path()).expect("read temp database"))
}

#[tokio::test]
async fn test_sqlite_tables_schema_and_rows() {
    let options = ConversionOptions {
        file_extension: Some(".sqlite".to_string()),
        ..Default::default()
    };
    let doc = MarkItDown::new()
        .convert_bytes(database_bytes().await, Some(options))
        .await
        .expect("SQLite conversion");
    let content = doc.to_markdown();

    assert!(content.contains("**Tables:** 2"), "{content}");
    assert!(content.contains("## Table: `people`"), "{content}");
    assert!(content.contains("| name | TEXT | No | No |"), "{content}");
    assert!(content.contains("**Row count:** 2"), "{content}");
    assert!(content.contains("| 1 | Ada | 9.5 |"), "{content}");
    assert!(content.contains("| 2 | Grace | NULL |"), "{content}");
    assert!(content.contains("## Table: `odd \"name\"`"), "{content}");
    assert!(content.contains("| quoted |"), "{content}");
}
