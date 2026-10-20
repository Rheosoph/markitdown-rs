//! RSS/Atom feed conversion tests using kreuzberg test documents
//!
//! Note: Only RSS and Atom feed formats are supported.
//! Generic XML files are not converted to markdown.

use bytes::Bytes;
use markitdown::{model::ConversionOptions, MarkItDown};
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

// ============================================================================
// RSS Feed Tests
// ============================================================================

#[tokio::test]
async fn test_rss_feed() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/xml/rss_feed.xml",
            Some(default_options(".xml")),
        )
        .await;

    assert!(result.is_ok(), "RSS conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
    // Check for RSS feed structure
    assert!(
        content.contains("Project Gutenberg") || content.contains("Gutenberg"),
        "Content should contain feed title"
    );
}

#[tokio::test]
async fn test_rss_feed_bytes() {
    let markitdown = MarkItDown::new();
    let bytes = fs::read("tests/test_documents/xml/rss_feed.xml").expect("Failed to read file");
    let result = markitdown
        .convert_bytes(Bytes::from(bytes), Some(default_options(".xml")))
        .await;

    assert!(
        result.is_ok(),
        "RSS bytes conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

// ============================================================================
// RSS Extension Tests
// ============================================================================

#[tokio::test]
async fn test_rss_extension() {
    let markitdown = MarkItDown::new();
    let bytes = fs::read("tests/test_documents/xml/rss_feed.xml").expect("Failed to read file");
    let result = markitdown
        .convert_bytes(Bytes::from(bytes), Some(default_options(".rss")))
        .await;

    assert!(
        result.is_ok(),
        "RSS extension conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    assert!(!doc.to_markdown().is_empty(), "Content should not be empty");
}

#[tokio::test]
async fn test_atom_extension() {
    // Test that atom extension works with RSS content
    let markitdown = MarkItDown::new();
    let bytes = fs::read("tests/test_documents/xml/rss_feed.xml").expect("Failed to read file");
    let result = markitdown
        .convert_bytes(Bytes::from(bytes), Some(default_options(".atom")))
        .await;

    // Even with wrong format extension, feed_rs should parse it
    assert!(
        result.is_ok(),
        "Atom extension conversion failed: {:?}",
        result.err()
    );
}
