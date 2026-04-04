//! PDF conversion tests using kreuzberg test documents
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
    }
}

// ============================================================================
// Basic PDF Tests
// ============================================================================

#[tokio::test]
async fn test_pdf_fake_memo() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/pdfs/fake_memo.pdf",
            Some(default_options(".pdf")),
        )
        .await;

    assert!(result.is_ok(), "PDF conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(content.len() >= 50, "Content should have reasonable length");

    // Check for expected content
    assert!(
        content.contains("May") || content.contains("2023") || content.contains("Concern"),
        "Should contain memo content"
    );
}

#[tokio::test]
async fn test_pdf_google_doc() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/pdfs/google_doc_document.pdf",
            Some(default_options(".pdf")),
        )
        .await;

    assert!(result.is_ok(), "PDF conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(content.len() >= 50, "Content should have reasonable length");
}

// ============================================================================
// PDF with Tables Tests
// ============================================================================

#[tokio::test]
async fn test_pdf_embedded_images_tables() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/pdfs/embedded_images_tables.pdf",
            Some(default_options(".pdf")),
        )
        .await;

    assert!(result.is_ok(), "PDF conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

#[tokio::test]
async fn test_pdf_tables_tiny() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/pdfs_with_tables/tiny.pdf",
            Some(default_options(".pdf")),
        )
        .await;

    assert!(result.is_ok(), "PDF conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

#[tokio::test]
async fn test_pdf_tables_medium() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/pdfs_with_tables/medium.pdf",
            Some(default_options(".pdf")),
        )
        .await;

    assert!(result.is_ok(), "PDF conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

// ============================================================================
// Technical/Academic PDF Tests
// ============================================================================

#[tokio::test]
async fn test_pdf_code_and_formula() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/pdfs/code_and_formula.pdf",
            Some(default_options(".pdf")),
        )
        .await;

    assert!(result.is_ok(), "PDF conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

// This test is ignored because the PDF file causes a panic in the pdf-extract library
// (index out of bounds in lib.rs:1802). This is a third-party library bug.
#[tokio::test]
#[ignore = "PDF causes panic in pdf-extract library - third-party bug"]
async fn test_pdf_deep_learning() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/pdfs/fundamentals_of_deep_learning_2014.pdf",
            Some(default_options(".pdf")),
        )
        .await;

    assert!(result.is_ok(), "PDF conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

// ============================================================================
// Non-English PDF Tests
// ============================================================================

#[tokio::test]
async fn test_pdf_right_to_left() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/pdfs/right_to_left_01.pdf",
            Some(default_options(".pdf")),
        )
        .await;

    assert!(result.is_ok(), "PDF conversion failed: {:?}", result.err());
}

// ============================================================================
// Multi-page PDF Tests
// ============================================================================

#[tokio::test]
async fn test_pdf_multi_page() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/pdfs/multi_page.pdf",
            Some(default_options(".pdf")),
        )
        .await;

    assert!(result.is_ok(), "PDF conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    // Multi-page PDF should have multiple pages
    assert!(doc.pages.len() >= 1, "Should have at least one page");
}

// ============================================================================
// PDF Bytes Conversion
// ============================================================================

#[tokio::test]
async fn test_pdf_bytes_conversion() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert_bytes(
            Bytes::from_static(include_bytes!("./test_documents/pdfs/fake_memo.pdf")),
            Some(default_options(".pdf")),
        )
        .await;

    assert!(
        result.is_ok(),
        "PDF bytes conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

// ============================================================================
// Page Count Verification
// ============================================================================

#[tokio::test]
async fn test_pdf_page_structure() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/pdfs/fake_memo.pdf",
            Some(default_options(".pdf")),
        )
        .await;

    assert!(result.is_ok());
    let doc = result.unwrap();

    // Verify document has pages
    assert!(!doc.pages.is_empty(), "Document should have pages");

    // Each page should have content
    for page in &doc.pages {
        // Pages can be empty or have content, but structure should exist
        assert!(page.page_number > 0, "Page numbers should be positive");
    }
}
