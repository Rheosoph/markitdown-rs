//! DOCX conversion tests using kreuzberg test documents
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
// Basic DOCX Tests
// ============================================================================

#[tokio::test]
async fn test_docx_basic() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/office/document.docx",
            Some(default_options(".docx")),
        )
        .await;

    assert!(result.is_ok(), "DOCX conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(content.len() >= 10, "Content should have reasonable length");
}

#[tokio::test]
async fn test_docx_fake() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/documents/fake.docx",
            Some(default_options(".docx")),
        )
        .await;

    assert!(result.is_ok(), "DOCX conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(content.len() >= 20, "Content should have reasonable length");

    // Should contain Lorem ipsum style content
    assert!(
        content.contains("Lorem")
            || content.contains("ipsum")
            || content.contains("document")
            || content.contains("text"),
        "Should contain expected text content"
    );
}

#[tokio::test]
async fn test_docx_lorem_ipsum() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/documents/lorem_ipsum.docx",
            Some(default_options(".docx")),
        )
        .await;

    assert!(result.is_ok(), "DOCX conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

// ============================================================================
// DOCX with Tables
// ============================================================================

#[tokio::test]
async fn test_docx_tables() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/documents/docx_tables.docx",
            Some(default_options(".docx")),
        )
        .await;

    assert!(result.is_ok(), "DOCX conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(content.len() >= 50, "Content should have reasonable length");

    // Check for table-related content
    assert!(
        content.contains("table") || content.contains("Table") || content.contains("|"),
        "Should contain table content"
    );
}

#[tokio::test]
async fn test_docx_word_tables() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/documents/word_tables.docx",
            Some(default_options(".docx")),
        )
        .await;

    assert!(result.is_ok(), "DOCX conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

// ============================================================================
// DOCX Formatting Tests
// ============================================================================

#[tokio::test]
async fn test_docx_formatting() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/documents/unit_test_formatting.docx",
            Some(default_options(".docx")),
        )
        .await;

    assert!(result.is_ok(), "DOCX conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(content.len() >= 20, "Content should have reasonable length");
}

#[tokio::test]
async fn test_docx_headers() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/documents/unit_test_headers.docx",
            Some(default_options(".docx")),
        )
        .await;

    assert!(result.is_ok(), "DOCX conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(content.len() >= 20, "Content should have reasonable length");
}

#[tokio::test]
async fn test_docx_lists() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/documents/unit_test_lists.docx",
            Some(default_options(".docx")),
        )
        .await;

    assert!(result.is_ok(), "DOCX conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(content.len() >= 20, "Content should have reasonable length");
}

// ============================================================================
// DOCX with Equations
// ============================================================================

#[tokio::test]
async fn test_docx_equations() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/documents/equations.docx",
            Some(default_options(".docx")),
        )
        .await;

    assert!(result.is_ok(), "DOCX conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(content.len() >= 20, "Content should have reasonable length");
}

// ============================================================================
// DOCX Bytes Conversion
// ============================================================================

#[tokio::test]
async fn test_docx_bytes_conversion() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert_bytes(
            Bytes::from_static(include_bytes!("./test_documents/office/document.docx")),
            Some(default_options(".docx")),
        )
        .await;

    assert!(
        result.is_ok(),
        "DOCX bytes conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

// ============================================================================
// DOCX with Images
// ============================================================================

#[tokio::test]
async fn test_docx_with_images() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/documents/word_image_anchors.docx",
            Some(default_options(".docx")),
        )
        .await;

    assert!(result.is_ok(), "DOCX conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

// ============================================================================
// Large DOCX Sample
// ============================================================================

#[tokio::test]
async fn test_docx_word_sample() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/documents/word_sample.docx",
            Some(default_options(".docx")),
        )
        .await;

    assert!(result.is_ok(), "DOCX conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}
