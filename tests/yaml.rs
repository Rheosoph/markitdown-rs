//! YAML format conversion tests
//!
//! Tests for YAML to markdown conversions.

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

const TEST_DIR: &str = "tests/test_documents/yaml";

fn test_file(name: &str) -> String {
    format!("{}/{}", TEST_DIR, name)
}

// ============================================================================
// YAML Conversion Tests
// ============================================================================

#[tokio::test]
async fn test_yaml_sample_config() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            &test_file("sample_config.yaml"),
            Some(default_options(".yaml")),
        )
        .await;

    assert!(result.is_ok(), "YAML conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(
        content.contains("```yaml"),
        "Should contain YAML code block"
    );
}

#[tokio::test]
async fn test_yaml_simple() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            "tests/test_documents/data_formats/simple.yaml",
            Some(default_options(".yaml")),
        )
        .await;

    assert!(result.is_ok(), "YAML conversion failed: {:?}", result.err());
}

#[tokio::test]
async fn test_yaml_bytes_conversion() {
    let md = MarkItDown::new();
    let bytes = fs::read(test_file("sample_config.yaml")).expect("Failed to read file");
    let result = md
        .convert_bytes(Bytes::from(bytes), Some(default_options(".yaml")))
        .await;

    assert!(
        result.is_ok(),
        "YAML bytes conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_yaml_yml_extension() {
    let md = MarkItDown::new();
    let yaml_content = "name: test\nvalue: 42\n";
    let result = md
        .convert_bytes(
            Bytes::from(yaml_content.as_bytes().to_vec()),
            Some(default_options(".yml")),
        )
        .await;

    assert!(result.is_ok(), "YML conversion failed: {:?}", result.err());
}
