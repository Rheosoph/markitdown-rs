use bytes::Bytes;
use markitdown::{model::ConversionOptions, MarkItDown};
use std::fs;
use std::io::{Cursor, Write};
use std::time::{SystemTime, UNIX_EPOCH};
use zip::write::SimpleFileOptions;

fn build_test_zip_bytes() -> Vec<u8> {
    let mut buffer = Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(&mut buffer);
    let options = SimpleFileOptions::default();

    writer
        .start_file("sample.txt", options)
        .expect("Failed to start zip file entry");
    writer
        .write_all(b"Hello zip")
        .expect("Failed to write zip file entry");

    writer.finish().expect("Failed to finish zip");
    buffer.into_inner()
}

#[tokio::test]
async fn test_zip_conversion() {
    let zip_bytes = build_test_zip_bytes();
    let unique_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("System time before UNIX_EPOCH")
        .as_nanos();
    let zip_path = std::env::temp_dir().join(format!("markitdown_test_{}.zip", unique_id));
    fs::write(&zip_path, &zip_bytes).expect("Failed to write temp zip file");

    let options = ConversionOptions {
        file_extension: Some(".zip".to_string()),
        url: None,
        llm_client: None,
        image_context_path: None,
        extract_images: false,
        force_llm_ocr: false,
        merge_multipage_tables: false,
        ..Default::default()
    };

    let markitdown = MarkItDown::new();

    let result = markitdown
        .convert(
            zip_path.to_str().expect("Invalid temp zip path"),
            Some(options),
        )
        .await;
    let _ = fs::remove_file(&zip_path);
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_zip_bytes_conversion() {
    let zip_bytes = build_test_zip_bytes();
    let options = ConversionOptions {
        file_extension: Some(".zip".to_string()),
        url: None,
        llm_client: None,
        image_context_path: None,
        extract_images: false,
        force_llm_ocr: false,
        merge_multipage_tables: false,
        ..Default::default()
    };

    let markitdown = MarkItDown::new();

    let result = markitdown
        .convert_bytes(Bytes::from(zip_bytes), Some(options))
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn conflicting_zip_header_names_are_rejected_by_converters() {
    use markitdown::model::DocumentConverter;
    let mut cloaked = build_test_zip_bytes();
    assert_eq!(&cloaked[30..40], b"sample.txt");
    cloaked[30] = b'x';
    let converters: Vec<Box<dyn DocumentConverter>> = vec![
        Box::new(markitdown::archive::ZipConverter),
        Box::new(markitdown::docx::DocxConverter),
        Box::new(markitdown::excel::ExcelConverter),
        Box::new(markitdown::epub::EpubConverter),
        Box::new(markitdown::opendocument::OdtConverter),
        Box::new(markitdown::opendocument::OdsConverter),
        Box::new(markitdown::opendocument::OdpConverter),
        Box::new(markitdown::legacy_office::XlsConverter),
        Box::new(markitdown::iwork::PagesConverter),
        Box::new(markitdown::pptx::PptxConverter),
    ];
    for converter in converters {
        let error = converter
            .convert_bytes(Bytes::from(cloaked.clone()), None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("filename mismatch"), "{error}");
    }
    let error = MarkItDown::new()
        .convert_bytes(
            Bytes::from(cloaked),
            Some(ConversionOptions {
                file_extension: Some(".zip".into()),
                ..Default::default()
            }),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("filename mismatch"), "{error}");
}
