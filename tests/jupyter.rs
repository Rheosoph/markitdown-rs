//! Jupyter Notebook conversion tests

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

const TEST_DIR: &str = "tests/test_documents/jupyter";

fn test_file(name: &str) -> String {
    format!("{}/{}", TEST_DIR, name)
}

#[tokio::test]
async fn test_jupyter_simple() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("simple.ipynb"), Some(default_options(".ipynb")))
        .await;

    assert!(
        result.is_ok(),
        "Jupyter conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

#[tokio::test]
async fn test_jupyter_mime() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("mime.ipynb"), Some(default_options(".ipynb")))
        .await;

    assert!(
        result.is_ok(),
        "Jupyter conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_jupyter_rank() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("rank.ipynb"), Some(default_options(".ipynb")))
        .await;

    assert!(
        result.is_ok(),
        "Jupyter conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_jupyter_bytes_conversion() {
    let md = MarkItDown::new();
    let bytes = fs::read(test_file("simple.ipynb")).expect("Failed to read file");
    let result = md
        .convert_bytes(Bytes::from(bytes), Some(default_options(".ipynb")))
        .await;

    assert!(
        result.is_ok(),
        "Jupyter bytes conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_jupyter_mime_out() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            &test_file("mime.out.ipynb"),
            Some(default_options(".ipynb")),
        )
        .await;

    assert!(
        result.is_ok(),
        "Jupyter conversion failed: {:?}",
        result.err()
    );
}
