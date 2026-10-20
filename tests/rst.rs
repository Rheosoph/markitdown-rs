//! reStructuredText conversion tests

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

const TEST_DIR: &str = "tests/test_documents/rst";

fn test_file(name: &str) -> String {
    format!("{}/{}", TEST_DIR, name)
}

#[tokio::test]
async fn test_rst_reader() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("rst-reader.rst"), Some(default_options(".rst")))
        .await;

    assert!(result.is_ok(), "RST conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

#[tokio::test]
async fn test_rst_bytes_conversion() {
    let md = MarkItDown::new();
    let bytes = fs::read(test_file("rst-reader.rst")).expect("Failed to read file");
    let result = md
        .convert_bytes(Bytes::from(bytes), Some(default_options(".rst")))
        .await;

    assert!(
        result.is_ok(),
        "RST bytes conversion failed: {:?}",
        result.err()
    );
}
