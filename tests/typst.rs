//! Typst document conversion tests

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

const TEST_DIR: &str = "tests/test_documents/typst";

fn test_file(name: &str) -> String {
    format!("{}/{}", TEST_DIR, name)
}

#[tokio::test]
async fn test_typst_simple() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("simple.typ"), Some(default_options(".typ")))
        .await;

    assert!(
        result.is_ok(),
        "Typst conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

#[tokio::test]
async fn test_typst_minimal() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("minimal.typ"), Some(default_options(".typ")))
        .await;

    assert!(
        result.is_ok(),
        "Typst conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_typst_headings() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("headings.typ"), Some(default_options(".typ")))
        .await;

    assert!(
        result.is_ok(),
        "Typst conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    // Should have headings
    assert!(content.contains("#"), "Should contain headings");
}

#[tokio::test]
async fn test_typst_bytes_conversion() {
    let md = MarkItDown::new();
    let bytes = fs::read(test_file("simple.typ")).expect("Failed to read file");
    let result = md
        .convert_bytes(Bytes::from(bytes), Some(default_options(".typ")))
        .await;

    assert!(
        result.is_ok(),
        "Typst bytes conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_typst_advanced() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("advanced.typ"), Some(default_options(".typ")))
        .await;

    assert!(
        result.is_ok(),
        "Typst conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_typst_reader() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            &test_file("typst-reader.typ"),
            Some(default_options(".typ")),
        )
        .await;

    assert!(
        result.is_ok(),
        "Typst conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_typst_metadata() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("metadata.typ"), Some(default_options(".typ")))
        .await;

    assert!(
        result.is_ok(),
        "Typst conversion failed: {:?}",
        result.err()
    );
}
