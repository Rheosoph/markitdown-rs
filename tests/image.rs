//! Image conversion tests using kreuzberg test documents
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
// PNG Image Tests
// ============================================================================

#[tokio::test]
async fn test_image_png_sample() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/images/sample.png",
            Some(default_options(".png")),
        )
        .await;

    assert!(result.is_ok(), "PNG conversion failed: {:?}", result.err());
    let doc = result.unwrap();
    // Images without OCR should still return metadata
    assert!(doc.pages.len() >= 1 || !doc.to_markdown().is_empty());
}

#[tokio::test]
async fn test_image_png_hello_world() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/images/test_hello_world.png",
            Some(default_options(".png")),
        )
        .await;

    assert!(result.is_ok(), "PNG conversion failed: {:?}", result.err());
}

// ============================================================================
// JPEG Image Tests
// ============================================================================

#[tokio::test]
async fn test_image_jpeg() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/images/example.jpg",
            Some(default_options(".jpg")),
        )
        .await;

    assert!(result.is_ok(), "JPEG conversion failed: {:?}", result.err());
}

#[tokio::test]
async fn test_image_jpeg_flower() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/images/flower_no_text.jpg",
            Some(default_options(".jpg")),
        )
        .await;

    assert!(result.is_ok(), "JPEG conversion failed: {:?}", result.err());
}

// ============================================================================
// OCR-related Image Tests
// ============================================================================

#[tokio::test]
async fn test_image_ocr_sample() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/images/ocr_image.jpg",
            Some(default_options(".jpg")),
        )
        .await;

    assert!(
        result.is_ok(),
        "OCR image conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_image_chinese() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/images/chi_sim_image.jpeg",
            Some(default_options(".jpeg")),
        )
        .await;

    assert!(
        result.is_ok(),
        "Chinese image conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_image_japanese() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/images/jpn_vert.jpeg",
            Some(default_options(".jpeg")),
        )
        .await;

    assert!(
        result.is_ok(),
        "Japanese image conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_image_korean() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/images/english_and_korean.png",
            Some(default_options(".png")),
        )
        .await;

    assert!(
        result.is_ok(),
        "Korean image conversion failed: {:?}",
        result.err()
    );
}

// ============================================================================
// BMP Image Tests
// ============================================================================

#[tokio::test]
async fn test_image_bmp() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/images/bmp_24.bmp",
            Some(default_options(".bmp")),
        )
        .await;

    assert!(result.is_ok(), "BMP conversion failed: {:?}", result.err());
}

// ============================================================================
// Image Bytes Conversion
// ============================================================================

#[tokio::test]
async fn test_image_bytes_conversion() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert_bytes(
            Bytes::from_static(include_bytes!("./test_documents/images/sample.png")),
            Some(default_options(".png")),
        )
        .await;

    assert!(
        result.is_ok(),
        "Image bytes conversion failed: {:?}",
        result.err()
    );
}

// ============================================================================
// Table Images
// ============================================================================

#[tokio::test]
async fn test_image_simple_table() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/tables/simple_table.png",
            Some(default_options(".png")),
        )
        .await;

    assert!(
        result.is_ok(),
        "Table image conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_image_borderless_table() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/tables/borderless_table.png",
            Some(default_options(".png")),
        )
        .await;

    assert!(
        result.is_ok(),
        "Borderless table image conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_image_complex_document() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/tables/complex_document.png",
            Some(default_options(".png")),
        )
        .await;

    assert!(
        result.is_ok(),
        "Complex document image conversion failed: {:?}",
        result.err()
    );
}

// ============================================================================
// Document Layout Images
// ============================================================================

#[tokio::test]
async fn test_image_layout_parser() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/images/layout_parser_ocr.jpg",
            Some(default_options(".jpg")),
        )
        .await;

    assert!(
        result.is_ok(),
        "Layout parser image conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_image_invoice() {
    let markitdown = MarkItDown::new();
    let result = markitdown
        .convert(
            "tests/test_documents/images/invoice_image.png",
            Some(default_options(".png")),
        )
        .await;

    assert!(
        result.is_ok(),
        "Invoice image conversion failed: {:?}",
        result.err()
    );
}
