//! PowerPoint conversion tests using kreuzberg test documents
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
// Basic PowerPoint Tests
// ============================================================================

#[tokio::test]
async fn test_pptx_simple() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/presentations/simple.pptx",
            Some(default_options(".pptx")),
        )
        .await;

    assert!(
        result.is_ok(),
        "PowerPoint conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

#[tokio::test]
async fn test_pptx_sample() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/presentations/powerpoint_sample.pptx",
            Some(default_options(".pptx")),
        )
        .await;

    assert!(
        result.is_ok(),
        "PowerPoint conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

// ============================================================================
// PowerPoint with Images
// ============================================================================

#[tokio::test]
async fn test_pptx_with_images() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/presentations/powerpoint_with_image.pptx",
            Some(default_options(".pptx")),
        )
        .await;

    assert!(
        result.is_ok(),
        "PowerPoint conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

// ============================================================================
// Pitch Deck PowerPoint
// ============================================================================

#[tokio::test]
async fn test_pptx_pitch_deck() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/presentations/pitch_deck_presentation.pptx",
            Some(default_options(".pptx")),
        )
        .await;

    assert!(
        result.is_ok(),
        "PowerPoint conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

// ============================================================================
// PowerPoint with Text Issues
// ============================================================================

#[tokio::test]
async fn test_pptx_bad_text() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/presentations/powerpoint_bad_text.pptx",
            Some(default_options(".pptx")),
        )
        .await;

    assert!(
        result.is_ok(),
        "PowerPoint conversion failed: {:?}",
        result.err()
    );
}

// ============================================================================
// PowerPoint Bytes Conversion
// ============================================================================

#[tokio::test]
async fn test_pptx_bytes_conversion() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert_bytes(
            Bytes::from_static(include_bytes!("./test_documents/presentations/simple.pptx")),
            Some(default_options(".pptx")),
        )
        .await;

    assert!(
        result.is_ok(),
        "PowerPoint bytes conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

// ============================================================================
// PowerPoint Slide Structure
// ============================================================================

#[tokio::test]
async fn test_pptx_slide_structure() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/presentations/powerpoint_sample.pptx",
            Some(default_options(".pptx")),
        )
        .await;

    assert!(result.is_ok());
    let doc = result.unwrap();

    // PowerPoint should have multiple slides/pages
    assert!(!doc.pages.is_empty(), "Document should have slides");
}
