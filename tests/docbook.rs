//! DocBook XML conversion tests

use bytes::Bytes;
use markitdown::{ConversionOptions, MarkItDown};
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

const TEST_DIR: &str = "tests/test_documents/docbook";

fn test_file(name: &str) -> String {
    format!("{}/{}", TEST_DIR, name)
}

#[tokio::test]
async fn test_docbook_chapter() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            &test_file("docbook-chapter.docbook"),
            Some(default_options(".docbook")),
        )
        .await;

    assert!(
        result.is_ok(),
        "DocBook conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

#[tokio::test]
async fn test_docbook_reader() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            &test_file("docbook-reader.docbook"),
            Some(default_options(".docbook")),
        )
        .await;

    assert!(
        result.is_ok(),
        "DocBook conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_docbook_xref() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            &test_file("docbook-xref.docbook"),
            Some(default_options(".docbook")),
        )
        .await;

    assert!(
        result.is_ok(),
        "DocBook conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_docbook_bytes_conversion() {
    let md = MarkItDown::new();
    let bytes = fs::read(test_file("docbook-chapter.docbook")).expect("Failed to read file");
    let result = md
        .convert_bytes(Bytes::from(bytes), Some(default_options(".docbook")))
        .await;

    assert!(
        result.is_ok(),
        "DocBook bytes conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_docbook_tables4() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            &test_file("tables.docbook4"),
            Some(default_options(".docbook4")),
        )
        .await;

    assert!(
        result.is_ok(),
        "DocBook conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_docbook_tables5() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            &test_file("tables.docbook5"),
            Some(default_options(".docbook5")),
        )
        .await;

    assert!(
        result.is_ok(),
        "DocBook conversion failed: {:?}",
        result.err()
    );
}
