//! SQLite database to Markdown converter.
//!
//! Converts SQLite databases to markdown by extracting table schemas and sample data.

use async_trait::async_trait;
use bytes::Bytes;
use object_store::{ObjectStore, ObjectStoreExt};
use sqlx::{AssertSqlSafe, Row, SqlitePool, ValueRef};
use std::io::Write;
use std::sync::Arc;
use tempfile::NamedTempFile;

use crate::error::MarkitdownError;
use crate::model::{ContentBlock, ConversionOptions, Document, DocumentConverter, Page};

/// SQLite database converter
pub struct SqliteConverter;

fn quoted_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

impl SqliteConverter {
    async fn convert_sqlite(bytes: &[u8]) -> Result<Document, MarkitdownError> {
        // Write bytes to a temporary file (SQLite needs file access)
        let mut temp_file = NamedTempFile::new().map_err(|e| {
            MarkitdownError::ParseError(format!("Failed to create temp file: {}", e))
        })?;
        temp_file.write_all(bytes).map_err(|e| {
            MarkitdownError::ParseError(format!("Failed to write temp file: {}", e))
        })?;

        let db_path = temp_file.path().to_string_lossy();
        let pool = SqlitePool::connect(&format!("sqlite:{}?mode=ro", db_path))
            .await
            .map_err(|e| MarkitdownError::ParseError(format!("SQLite connection error: {}", e)))?;

        let mut document = Document::new();
        let mut page = Page::new(1);
        let mut markdown = String::new();

        markdown.push_str("# SQLite Database\n\n");

        // Get list of tables
        let tables: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
        )
            .fetch_all(&pool)
            .await
            .map_err(|e| MarkitdownError::ParseError(format!("Failed to list tables: {}", e)))?;

        if tables.is_empty() {
            markdown.push_str("*No tables found in database.*\n");
        } else {
            markdown.push_str(&format!("**Tables:** {}\n\n", tables.len()));

            for (table_name,) in &tables {
                markdown.push_str(&format!("## Table: `{}`\n\n", table_name));
                let table = quoted_identifier(table_name);

                // Get table schema
                let schema_query = format!("PRAGMA table_info({table})");
                let columns: Vec<(i32, String, String, i32, Option<String>, i32)> =
                    sqlx::query_as(AssertSqlSafe(schema_query))
                        .fetch_all(&pool)
                        .await
                        .unwrap_or_default();

                if !columns.is_empty() {
                    markdown.push_str("### Schema\n\n");
                    markdown.push_str("| Column | Type | Nullable | Primary Key |\n");
                    markdown.push_str("|--------|------|----------|-------------|\n");

                    for (_, name, col_type, notnull, _, pk) in &columns {
                        markdown.push_str(&format!(
                            "| {} | {} | {} | {} |\n",
                            name,
                            col_type,
                            if *notnull == 0 { "Yes" } else { "No" },
                            if *pk > 0 { "Yes" } else { "No" }
                        ));
                    }
                    markdown.push('\n');
                }

                // Get row count
                let count_query = format!("SELECT COUNT(*) FROM {table}");
                let count: (i64,) = sqlx::query_as(AssertSqlSafe(count_query))
                    .fetch_one(&pool)
                    .await
                    .unwrap_or((0,));

                markdown.push_str(&format!("**Row count:** {}\n\n", count.0));

                // Get sample data (first 5 rows)
                if count.0 > 0 && !columns.is_empty() {
                    let sample_query = format!("SELECT * FROM {table} LIMIT 5");

                    if let Ok(rows) = sqlx::query(AssertSqlSafe(sample_query))
                        .fetch_all(&pool)
                        .await
                    {
                        if !rows.is_empty() {
                            markdown.push_str("### Sample Data (first 5 rows)\n\n");

                            // Header
                            let col_names: Vec<&str> = columns
                                .iter()
                                .map(|(_, n, _, _, _, _)| n.as_str())
                                .collect();
                            markdown.push_str(&format!("| {} |\n", col_names.join(" | ")));
                            markdown.push_str(&format!(
                                "|{}|\n",
                                col_names
                                    .iter()
                                    .map(|_| "---")
                                    .collect::<Vec<_>>()
                                    .join("|")
                            ));

                            // Rows
                            for row in &rows {
                                let values: Vec<String> = (0..columns.len())
                                    .map(|i| {
                                        if row.try_get_raw(i).map_or(true, |v| v.is_null()) {
                                            return "NULL".to_string();
                                        }
                                        row.try_get::<String, _>(i)
                                            .or_else(|_| {
                                                row.try_get::<i64, _>(i).map(|v| v.to_string())
                                            })
                                            .or_else(|_| {
                                                row.try_get::<f64, _>(i).map(|v| v.to_string())
                                            })
                                            .unwrap_or_else(|_| "NULL".to_string())
                                            .replace('|', "\\|")
                                    })
                                    .collect();
                                markdown.push_str(&format!("| {} |\n", values.join(" | ")));
                            }
                            markdown.push('\n');
                        }
                    }
                }
            }
        }

        pool.close().await;

        page.add_content(ContentBlock::Markdown(markdown));
        document.add_page(page);
        Ok(document)
    }
}

#[async_trait]
impl DocumentConverter for SqliteConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let valid_extensions = [".db", ".sqlite", ".sqlite3"];

        if let Some(opts) = &options {
            if let Some(ext) = &opts.file_extension {
                if !valid_extensions.contains(&ext.as_str()) {
                    return Err(MarkitdownError::InvalidFile(format!(
                        "Expected SQLite file (.db, .sqlite, .sqlite3), got {}",
                        ext
                    )));
                }
            }
        }

        let result = store.get(path).await?;
        let bytes = result.bytes().await?;
        self.convert_bytes(bytes, options).await
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let valid_extensions = [".db", ".sqlite", ".sqlite3"];

        if let Some(opts) = &options {
            if let Some(ext) = &opts.file_extension {
                if !valid_extensions.contains(&ext.as_str()) {
                    return Err(MarkitdownError::InvalidFile(format!(
                        "Expected SQLite file (.db, .sqlite, .sqlite3), got {}",
                        ext
                    )));
                }
            }
        }

        Self::convert_sqlite(&bytes).await
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".db", ".sqlite", ".sqlite3"]
    }
}
