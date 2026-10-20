//! FictionBook (FB2) conversion tests

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

const TEST_DIR: &str = "tests/test_documents/fictionbook";

fn test_file(name: &str) -> String {
    format!("{}/{}", TEST_DIR, name)
}

#[tokio::test]
async fn test_fb2_basic() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("basic.fb2"), Some(default_options(".fb2")))
        .await;

    assert!(
        result.is_ok(),
        "FictionBook conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

#[tokio::test]
async fn test_fb2_emphasis() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("emphasis.fb2"), Some(default_options(".fb2")))
        .await;

    assert!(
        result.is_ok(),
        "FictionBook conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_fb2_titles() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("titles.fb2"), Some(default_options(".fb2")))
        .await;

    assert!(
        result.is_ok(),
        "FictionBook conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    // Should have section titles as headings
    assert!(
        content.contains("#"),
        "Should contain markdown headings from sections"
    );
}

#[tokio::test]
async fn test_fb2_bytes_conversion() {
    let md = MarkItDown::new();
    let bytes = fs::read(test_file("basic.fb2")).expect("Failed to read file");
    let result = md
        .convert_bytes(Bytes::from(bytes), Some(default_options(".fb2")))
        .await;

    assert!(
        result.is_ok(),
        "FictionBook bytes conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_fb2_meta() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("meta.fb2"), Some(default_options(".fb2")))
        .await;

    assert!(
        result.is_ok(),
        "FictionBook conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_fb2_images() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("images.fb2"), Some(default_options(".fb2")))
        .await;

    assert!(
        result.is_ok(),
        "FictionBook conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_fb2_notes() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("notes.fb2"), Some(default_options(".fb2")))
        .await;

    assert!(
        result.is_ok(),
        "FictionBook conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_fb2_poem() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("poem.fb2"), Some(default_options(".fb2")))
        .await;

    assert!(
        result.is_ok(),
        "FictionBook conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_fb2_tables() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("tables.fb2"), Some(default_options(".fb2")))
        .await;

    assert!(
        result.is_ok(),
        "FictionBook conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_fb2_epigraph() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("epigraph.fb2"), Some(default_options(".fb2")))
        .await;

    assert!(
        result.is_ok(),
        "FictionBook conversion failed: {:?}",
        result.err()
    );
}
