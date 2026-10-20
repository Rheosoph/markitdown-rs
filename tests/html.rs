//! HTML conversion tests using kreuzberg test documents
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
// Basic HTML Tests
// ============================================================================

#[tokio::test]
async fn test_html_basic() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/web/html.html",
            Some(default_options(".html")),
        )
        .await;

    assert!(result.is_ok(), "HTML conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

#[tokio::test]
async fn test_html_simple_table() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/web/simple_table.html",
            Some(default_options(".html")),
        )
        .await;

    assert!(result.is_ok(), "HTML conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(content.len() >= 10, "Content should have reasonable length");

    // Check for markdown content
    assert!(
        content.contains("#")
            || content.contains("**")
            || content.contains("simple")
            || content.contains("HTML"),
        "Should contain markdown formatted content"
    );
}

#[tokio::test]
async fn test_html_complex_table() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/web/complex_table.html",
            Some(default_options(".html")),
        )
        .await;

    assert!(result.is_ok(), "HTML conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

// ============================================================================
// Wikipedia-style HTML Tests
// ============================================================================

#[tokio::test]
async fn test_html_taylor_swift() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/web/taylor_swift.html",
            Some(default_options(".html")),
        )
        .await;

    assert!(result.is_ok(), "HTML conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(
        content.len() > 1000,
        "Large HTML should produce substantial content"
    );
}

#[tokio::test]
async fn test_html_world_war_ii() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/web/world_war_ii.html",
            Some(default_options(".html")),
        )
        .await;

    assert!(result.is_ok(), "HTML conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(
        content.len() > 1000,
        "Large HTML should produce substantial content"
    );
}

// ============================================================================
// Non-English HTML Tests
// ============================================================================

#[tokio::test]
async fn test_html_german() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/web/germany_german.html",
            Some(default_options(".html")),
        )
        .await;

    assert!(result.is_ok(), "HTML conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

#[tokio::test]
async fn test_html_chinese() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/web/china_chinese.html",
            Some(default_options(".html")),
        )
        .await;

    assert!(result.is_ok(), "HTML conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

#[tokio::test]
async fn test_html_hebrew() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/web/israel_hebrew.html",
            Some(default_options(".html")),
        )
        .await;

    assert!(result.is_ok(), "HTML conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

// ============================================================================
// HTML Bytes Conversion
// ============================================================================

#[tokio::test]
async fn test_html_bytes_conversion() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert_bytes(
            Bytes::from_static(include_bytes!("./test_documents/web/simple_table.html")),
            Some(default_options(".html")),
        )
        .await;

    assert!(
        result.is_ok(),
        "HTML bytes conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

// ============================================================================
// Specialized HTML Content
// ============================================================================

#[tokio::test]
async fn test_html_consciousness() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/web/consciousness.html",
            Some(default_options(".html")),
        )
        .await;

    assert!(result.is_ok(), "HTML conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(
        content.len() > 500,
        "Content should have substantial length"
    );
}

#[tokio::test]
async fn test_html_medical() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/web/crohns_disease.html",
            Some(default_options(".html")),
        )
        .await;

    assert!(result.is_ok(), "HTML conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}
