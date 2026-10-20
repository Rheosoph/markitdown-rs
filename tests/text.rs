//! Plain text conversion tests
//!
//! Tests for text file conversions.

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

const TEST_DIR: &str = "tests/test_documents/text";

fn test_file(name: &str) -> String {
    format!("{}/{}", TEST_DIR, name)
}

// ============================================================================
// Text File Conversion Tests
// ============================================================================

#[tokio::test]
async fn test_text_fake_text() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("fake_text.txt"), Some(default_options(".txt")))
        .await;

    assert!(result.is_ok(), "Text conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

#[tokio::test]
async fn test_text_contract() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("contract.txt"), Some(default_options(".txt")))
        .await;

    assert!(result.is_ok(), "Text conversion failed: {:?}", result.err());
}

#[tokio::test]
async fn test_text_book_war_and_peace() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            &test_file("book_war_and_peace_1p.txt"),
            Some(default_options(".txt")),
        )
        .await;

    assert!(result.is_ok(), "Text conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

#[tokio::test]
async fn test_text_bytes_conversion() {
    let md = MarkItDown::new();
    let bytes = fs::read(test_file("fake_text.txt")).expect("Failed to read file");
    let result = md
        .convert_bytes(Bytes::from(bytes), Some(default_options(".txt")))
        .await;

    assert!(
        result.is_ok(),
        "Text bytes conversion failed: {:?}",
        result.err()
    );
}

// ============================================================================
// Markdown Passthrough Tests
// ============================================================================

#[tokio::test]
async fn test_markdown_passthrough() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("birth_control.md"), Some(default_options(".md")))
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
async fn test_markdown_comprehensive() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            "tests/test_documents/markdown/comprehensive.md",
            Some(default_options(".md")),
        )
        .await;

    assert!(
        result.is_ok(),
        "Markdown conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_markdown_bytes_conversion() {
    let md = MarkItDown::new();
    let markdown_content = "# Hello\n\nThis is **bold** text.";
    let result = md
        .convert_bytes(
            Bytes::from(markdown_content.as_bytes().to_vec()),
            Some(default_options(".md")),
        )
        .await;

    assert!(result.is_ok());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(content.contains("Hello"), "Should contain content");
    assert!(content.contains("**bold**"), "Should preserve markdown");
}
