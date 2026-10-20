//! Excel conversion tests using kreuzberg test documents
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

// ============================================================================
// Basic Excel Tests
// ============================================================================

#[tokio::test]
async fn test_xlsx_basic() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/office/excel.xlsx",
            Some(default_options(".xlsx")),
        )
        .await;

    assert!(
        result.is_ok(),
        "Excel conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(content.len() >= 10, "Content should have reasonable length");
}

#[tokio::test]
async fn test_xlsx_stanley_cups() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/spreadsheets/stanley_cups.xlsx",
            Some(default_options(".xlsx")),
        )
        .await;

    assert!(
        result.is_ok(),
        "Excel conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(
        content.len() >= 100,
        "Content should have reasonable length"
    );

    // Check for expected content
    assert!(
        content.contains("Team") || content.contains("Location") || content.contains("Stanley"),
        "Should contain expected table data"
    );
}

// ============================================================================
// Multi-Sheet Excel Tests
// ============================================================================

#[tokio::test]
async fn test_xlsx_multi_sheet() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/spreadsheets/excel_multi_sheet.xlsx",
            Some(default_options(".xlsx")),
        )
        .await;

    assert!(
        result.is_ok(),
        "Excel conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(content.len() >= 20, "Content should have reasonable length");
}

#[tokio::test]
async fn test_xlsx_test_01() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/spreadsheets/test_01.xlsx",
            Some(default_options(".xlsx")),
        )
        .await;

    assert!(
        result.is_ok(),
        "Excel conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

// ============================================================================
// Excel Bytes Conversion
// ============================================================================

#[tokio::test]
async fn test_xlsx_bytes_conversion() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert_bytes(
            Bytes::from_static(include_bytes!("./test_documents/office/excel.xlsx")),
            Some(default_options(".xlsx")),
        )
        .await;

    assert!(
        result.is_ok(),
        "Excel bytes conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

// ============================================================================
// Excel Table Structure Tests
// ============================================================================

#[tokio::test]
async fn test_xlsx_table_format() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/spreadsheets/stanley_cups.xlsx",
            Some(default_options(".xlsx")),
        )
        .await;

    assert!(result.is_ok());
    let doc = result.unwrap();
    let content = doc.to_markdown();

    // Excel should be converted to markdown table format
    assert!(
        content.contains("|") || content.contains("---"),
        "Excel should be converted to markdown table format"
    );
}
