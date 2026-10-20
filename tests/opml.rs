//! OPML conversion tests

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

const TEST_DIR: &str = "tests/test_documents/opml";

fn test_file(name: &str) -> String {
    format!("{}/{}", TEST_DIR, name)
}

#[tokio::test]
async fn test_opml_feeds() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("feeds.opml"), Some(default_options(".opml")))
        .await;

    assert!(result.is_ok(), "OPML conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

#[tokio::test]
async fn test_opml_outline() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("outline.opml"), Some(default_options(".opml")))
        .await;

    assert!(result.is_ok(), "OPML conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    // Should have nested list items with indentation
    assert!(content.contains("-"), "Should contain list markers");
}

#[tokio::test]
async fn test_opml_podcasts() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("podcasts.opml"), Some(default_options(".opml")))
        .await;

    assert!(result.is_ok(), "OPML conversion failed: {:?}", result.err());
}

#[tokio::test]
async fn test_opml_bytes_conversion() {
    let md = MarkItDown::new();
    let bytes = fs::read(test_file("feeds.opml")).expect("Failed to read file");
    let result = md
        .convert_bytes(Bytes::from(bytes), Some(default_options(".opml")))
        .await;

    assert!(
        result.is_ok(),
        "OPML bytes conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_opml_reader() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            &test_file("opml-reader.opml"),
            Some(default_options(".opml")),
        )
        .await;

    assert!(result.is_ok(), "OPML conversion failed: {:?}", result.err());
}

#[tokio::test]
async fn test_opml_pandoc_writer() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            &test_file("pandoc-writer.opml"),
            Some(default_options(".opml")),
        )
        .await;

    assert!(result.is_ok(), "OPML conversion failed: {:?}", result.err());
}
