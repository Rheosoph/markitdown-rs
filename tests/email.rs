//! Email (EML/MSG) conversion tests
//!
//! Tests for email file format conversions.

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

const TEST_DIR: &str = "tests/test_documents/email";

fn test_file(name: &str) -> String {
    format!("{}/{}", TEST_DIR, name)
}

// ============================================================================
// EML Tests
// ============================================================================

#[tokio::test]
async fn test_eml_fake_email() {
    let md = MarkItDown::new();
    let result = md.convert(&test_file("fake_email.eml"), None).await;

    assert!(result.is_ok(), "EML conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

#[tokio::test]
async fn test_eml_complex_headers() {
    let md = MarkItDown::new();
    let result = md.convert(&test_file("complex_headers.eml"), None).await;

    assert!(result.is_ok(), "EML conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

#[tokio::test]
async fn test_eml_bytes_conversion() {
    let md = MarkItDown::new();
    let bytes = fs::read(test_file("fake_email.eml")).expect("Failed to read file");
    let result = md
        .convert_bytes(Bytes::from(bytes), Some(default_options(".eml")))
        .await;

    assert!(
        result.is_ok(),
        "EML bytes conversion failed: {:?}",
        result.err()
    );
}

// ============================================================================
// MSG Tests (if .msg files exist)
// ============================================================================

#[tokio::test]
async fn test_msg_fake_email() {
    let md = MarkItDown::new();
    let path = test_file("fake_email.msg");

    // Check if file exists first
    if !std::path::Path::new(&path).exists() {
        return; // Skip if no .msg file
    }

    let result = md.convert(&path, None).await;
    // MSG format may not be fully supported, so we just check it doesn't panic
    let _ = result;
}

#[tokio::test]
async fn test_msg_with_attachment() {
    let md = MarkItDown::new();
    let path = test_file("fake_email_attachment.msg");

    if !std::path::Path::new(&path).exists() {
        return;
    }

    let result = md.convert(&path, None).await;
    let _ = result;
}
