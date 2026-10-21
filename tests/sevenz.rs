use bytes::Bytes;
use markitdown::{model::ConversionOptions, MarkItDown};
use sevenz_rust2::{ArchiveEntry, ArchiveWriter};
use std::io::Cursor;
use std::time::{SystemTime, UNIX_EPOCH};

fn build_7z(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer =
        ArchiveWriter::new(Cursor::new(Vec::new())).expect("Failed to create 7z writer");
    for (name, contents) in entries {
        writer
            .push_archive_entry(ArchiveEntry::new_file(name), Some(*contents))
            .expect("Failed to add 7z entry");
    }
    writer.finish().expect("Failed to finish 7z").into_inner()
}

fn options() -> ConversionOptions {
    ConversionOptions {
        file_extension: Some(".7z".to_string()),
        extract_images: false,
        ..Default::default()
    }
}

async fn convert(bytes: Vec<u8>) -> String {
    MarkItDown::new()
        .convert_bytes(Bytes::from(bytes), Some(options()))
        .await
        .expect("7z conversion failed")
        .to_markdown()
}

#[tokio::test]
async fn test_7z_bytes_conversion() {
    let archive = build_7z(&[
        ("notes/sample.txt", b"Hello 7z"),
        ("readme.md", b"# Readme"),
    ]);

    let markdown = convert(archive).await;

    assert!(markdown.contains("notes/sample.txt"), "{markdown}");
    assert!(markdown.contains("Hello 7z"), "{markdown}");
    assert!(markdown.contains("# Readme"), "{markdown}");
}

#[tokio::test]
async fn test_7z_entry_names_never_become_file_paths() {
    let unique_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("System time before UNIX_EPOCH")
        .as_nanos();
    let escape_path = std::env::temp_dir().join(format!("markitdown_7z_escape_{}.txt", unique_id));
    let absolute_name = escape_path.to_string_lossy().to_string();
    let archive = build_7z(&[
        (absolute_name.as_str(), b"absolute entry"),
        ("../../markitdown_7z_relative_escape.txt", b"relative entry"),
    ]);

    let markdown = convert(archive).await;

    assert!(markdown.contains("absolute entry"), "{markdown}");
    assert!(markdown.contains("relative entry"), "{markdown}");
    assert!(
        !escape_path.exists(),
        "7z entry was written to {}",
        escape_path.display()
    );
}
