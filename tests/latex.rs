//! LaTeX conversion tests

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

const TEST_DIR: &str = "tests/test_documents/latex";

fn test_file(name: &str) -> String {
    format!("{}/{}", TEST_DIR, name)
}

#[tokio::test]
async fn test_latex_basic_sections() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            &test_file("basic_sections.tex"),
            Some(default_options(".tex")),
        )
        .await;

    assert!(
        result.is_ok(),
        "LaTeX conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

#[tokio::test]
async fn test_latex_math() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("math.tex"), Some(default_options(".tex")))
        .await;

    assert!(
        result.is_ok(),
        "LaTeX conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    // Math content should be preserved
    assert!(
        content.contains("$") || content.contains("\\"),
        "Math notation should be preserved"
    );
}

#[tokio::test]
async fn test_latex_minimal() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("minimal.tex"), Some(default_options(".tex")))
        .await;

    assert!(
        result.is_ok(),
        "LaTeX conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_latex_bytes_conversion() {
    let md = MarkItDown::new();
    let bytes = fs::read(test_file("minimal.tex")).expect("Failed to read file");
    let result = md
        .convert_bytes(Bytes::from(bytes), Some(default_options(".tex")))
        .await;

    assert!(
        result.is_ok(),
        "LaTeX bytes conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_latex_tables() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("tables.tex"), Some(default_options(".tex")))
        .await;

    assert!(
        result.is_ok(),
        "LaTeX conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_latex_formatting() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("formatting.tex"), Some(default_options(".tex")))
        .await;

    assert!(
        result.is_ok(),
        "LaTeX conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_latex_lists() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("lists.tex"), Some(default_options(".tex")))
        .await;

    assert!(
        result.is_ok(),
        "LaTeX conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_latex_reader() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            &test_file("latex-reader.latex"),
            Some(default_options(".latex")),
        )
        .await;

    assert!(
        result.is_ok(),
        "LaTeX conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_latex_comprehensive() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            &test_file("comprehensive_rustex.tex"),
            Some(default_options(".tex")),
        )
        .await;

    assert!(
        result.is_ok(),
        "LaTeX conversion failed: {:?}",
        result.err()
    );
}
