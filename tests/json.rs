//! JSON format conversion tests
//!
//! Tests for JSON to markdown conversions.

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

const TEST_DIR: &str = "tests/test_documents/json";

fn test_file(name: &str) -> String {
    format!("{}/{}", TEST_DIR, name)
}

// ============================================================================
// JSON Conversion Tests
// ============================================================================

#[tokio::test]
async fn test_json_simple() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            "tests/test_documents/data_formats/simple.json",
            Some(default_options(".json")),
        )
        .await;

    assert!(result.is_ok(), "JSON conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(
        content.contains("```json"),
        "Should contain JSON code block"
    );
}

#[tokio::test]
async fn test_json_complex_nested() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            &test_file("complex_nested.json"),
            Some(default_options(".json")),
        )
        .await;

    assert!(result.is_ok(), "JSON conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(
        content.contains("```json"),
        "Should contain JSON code block"
    );
}

#[tokio::test]
async fn test_json_sample_document() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            &test_file("sample_document.json"),
            Some(default_options(".json")),
        )
        .await;

    assert!(result.is_ok(), "JSON conversion failed: {:?}", result.err());
}

#[tokio::test]
async fn test_json_schema_test() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            &test_file("schema_test.json"),
            Some(default_options(".json")),
        )
        .await;

    assert!(result.is_ok(), "JSON conversion failed: {:?}", result.err());
}

#[tokio::test]
async fn test_json_bytes_conversion() {
    let md = MarkItDown::new();
    let bytes =
        fs::read("tests/test_documents/data_formats/simple.json").expect("Failed to read file");
    let result = md
        .convert_bytes(Bytes::from(bytes), Some(default_options(".json")))
        .await;

    assert!(
        result.is_ok(),
        "JSON bytes conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_json_pretty_formatted() {
    let md = MarkItDown::new();
    let json_content = r#"{"name": "test", "value": 42}"#;
    let result = md
        .convert_bytes(
            Bytes::from(json_content.as_bytes().to_vec()),
            Some(default_options(".json")),
        )
        .await;

    assert!(result.is_ok());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    // Should be pretty-formatted with indentation
    assert!(
        content.contains("  ") || content.contains("\n"),
        "JSON should be formatted"
    );
}
