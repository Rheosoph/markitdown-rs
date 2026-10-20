//! Org-mode conversion tests

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

const TEST_DIR: &str = "tests/test_documents/orgmode";

fn test_file(name: &str) -> String {
    format!("{}/{}", TEST_DIR, name)
}

#[tokio::test]
async fn test_org_comprehensive() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            &test_file("comprehensive.org"),
            Some(default_options(".org")),
        )
        .await;

    assert!(
        result.is_ok(),
        "Org-mode conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

#[tokio::test]
async fn test_org_code_blocks() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("code-blocks.org"), Some(default_options(".org")))
        .await;

    assert!(
        result.is_ok(),
        "Org-mode conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    // Code blocks should be preserved
    assert!(content.contains("```"), "Should contain code block markers");
}

#[tokio::test]
async fn test_org_links() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("links.org"), Some(default_options(".org")))
        .await;

    assert!(
        result.is_ok(),
        "Org-mode conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_org_bytes_conversion() {
    let md = MarkItDown::new();
    let bytes = fs::read(test_file("comprehensive.org")).expect("Failed to read file");
    let result = md
        .convert_bytes(Bytes::from(bytes), Some(default_options(".org")))
        .await;

    assert!(
        result.is_ok(),
        "Org-mode bytes conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_org_pandoc_writer() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            &test_file("pandoc-writer.org"),
            Some(default_options(".org")),
        )
        .await;

    assert!(
        result.is_ok(),
        "Org-mode conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_org_pandoc_tables() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            &test_file("pandoc-tables.org"),
            Some(default_options(".org")),
        )
        .await;

    assert!(
        result.is_ok(),
        "Org-mode conversion failed: {:?}",
        result.err()
    );
}
