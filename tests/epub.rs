//! EPUB ebook conversion tests
//!
//! Tests for EPUB format conversions.

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

const TEST_DIR: &str = "tests/test_documents/epub";

fn test_file(name: &str) -> String {
    format!("{}/{}", TEST_DIR, name)
}

// ============================================================================
// EPUB Conversion Tests
// ============================================================================

#[tokio::test]
async fn test_epub2_cover() {
    let md = MarkItDown::new();
    let result = md.convert(&test_file("epub2_cover.epub"), None).await;

    assert!(result.is_ok(), "EPUB conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

#[tokio::test]
async fn test_epub2_no_cover() {
    let md = MarkItDown::new();
    let result = md.convert(&test_file("epub2_no_cover.epub"), None).await;

    assert!(result.is_ok(), "EPUB conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

#[tokio::test]
async fn test_epub2_picture() {
    let md = MarkItDown::new();
    let result = md.convert(&test_file("epub2_picture.epub"), None).await;

    assert!(result.is_ok(), "EPUB conversion failed: {:?}", result.err());
}

#[tokio::test]
async fn test_epub_bytes_conversion() {
    let md = MarkItDown::new();
    let bytes = fs::read(test_file("epub2_cover.epub")).expect("Failed to read file");
    let result = md
        .convert_bytes(Bytes::from(bytes), Some(default_options(".epub")))
        .await;

    assert!(
        result.is_ok(),
        "EPUB bytes conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_epub_has_metadata() {
    let md = MarkItDown::new();
    let result = md.convert(&test_file("epub2_cover.epub"), None).await;

    assert!(result.is_ok());
    let doc = result.unwrap();

    // EPUB should have at least one page
    assert!(!doc.pages.is_empty(), "EPUB should have pages");
}

#[tokio::test]
async fn test_epub_misc_simple() {
    // Test the simple.epub from misc directory
    let md = MarkItDown::new();
    let result = md
        .convert("tests/test_documents/misc/simple.epub", None)
        .await;

    assert!(
        result.is_ok(),
        "Simple EPUB conversion failed: {:?}",
        result.err()
    );
}
