//! CSV conversion tests using kreuzberg test documents
use bytes::Bytes;
use markitdown::{model::ConversionOptions, MarkItDown};

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

#[tokio::test]
async fn test_csv_stanley_cups() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/spreadsheets/stanley_cups.csv",
            Some(default_options(".csv")),
        )
        .await;

    assert!(result.is_ok(), "CSV conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();

    // Check for expected content
    assert!(
        content.contains("Team") || content.contains("Stanley"),
        "CSV should contain table data"
    );
    assert!(
        content.len() > 50,
        "CSV content should have reasonable length"
    );
}

#[tokio::test]
async fn test_csv_bytes_conversion() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert_bytes(
            Bytes::from_static(include_bytes!(
                "./test_documents/spreadsheets/stanley_cups.csv"
            )),
            Some(default_options(".csv")),
        )
        .await;

    assert!(
        result.is_ok(),
        "CSV bytes conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "CSV content should not be empty");
}

#[tokio::test]
async fn test_csv_table_structure() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/spreadsheets/stanley_cups.csv",
            Some(default_options(".csv")),
        )
        .await;

    assert!(result.is_ok());
    let doc = result.unwrap();
    let content = doc.to_markdown();

    // CSV should be converted to markdown table format
    assert!(
        content.contains("|") || content.contains("---"),
        "CSV should be converted to markdown table format"
    );
}
