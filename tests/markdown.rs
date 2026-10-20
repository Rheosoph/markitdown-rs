//! Markdown conversion tests (passthrough and normalization)

use bytes::Bytes;
use markitdown::error::MarkitdownError;
use markitdown::{ConversionOptions, MarkItDown};
use object_store::{path::Path as ObjectPath, ObjectStoreExt};
use std::fs;

fn default_options(ext: &str) -> ConversionOptions {
    ConversionOptions {
        file_extension: Some(ext.to_string()),
        url: None,
        llm_client: None,
        image_context_path: None,
        extract_images: true,
        force_llm_ocr: false,
        merge_multipage_tables: false,
        ..Default::default()
    }
}

const TEST_DIR: &str = "tests/test_documents/markdown";

fn test_file(name: &str) -> String {
    format!("{}/{}", TEST_DIR, name)
}

#[tokio::test]
async fn test_markdown_comprehensive() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("comprehensive.md"), Some(default_options(".md")))
        .await;

    assert!(
        result.is_ok(),
        "Markdown conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

#[tokio::test]
async fn test_markdown_tables() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            &test_file("tables.markdown"),
            Some(default_options(".markdown")),
        )
        .await;

    assert!(
        result.is_ok(),
        "Markdown conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    // Tables should be preserved (pandoc simple table format uses dashes)
    assert!(content.contains("-"), "Should contain table markup");
}

#[tokio::test]
async fn test_markdown_bytes_conversion() {
    let md = MarkItDown::new();
    let bytes = fs::read(test_file("comprehensive.md")).expect("Failed to read file");
    let result = md
        .convert_bytes(Bytes::from(bytes), Some(default_options(".md")))
        .await;

    assert!(
        result.is_ok(),
        "Markdown bytes conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_markdown_object_store_conversion() {
    let temp_dir = tempfile::tempdir().unwrap();
    let local_path = temp_dir.path().join("stored.md");
    assert!(!local_path.exists());
    let path = local_path.to_str().unwrap();
    let object_path = ObjectPath::from(path);
    let md = MarkItDown::in_memory();
    md.store()
        .put(
            &object_path,
            Bytes::from_static(b"# Stored document\n\nRead from object storage.").into(),
        )
        .await
        .unwrap();

    let doc = md.convert(path, None).await.unwrap();

    assert_eq!(
        doc.to_markdown(),
        "# Stored document\n\nRead from object storage."
    );
}

#[tokio::test]
async fn test_markdown_missing_object_store_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    let local_path = temp_dir.path().join("missing.md");
    assert!(!local_path.exists());
    let path = local_path.to_str().unwrap();
    let md = MarkItDown::in_memory();

    let error = md.convert(path, None).await.unwrap_err();

    match error {
        MarkitdownError::ObjectStoreError(message) => {
            assert!(message.contains(ObjectPath::from(path).as_ref()));
        }
        other => panic!("Expected object store error, got {other:?}"),
    }
}
