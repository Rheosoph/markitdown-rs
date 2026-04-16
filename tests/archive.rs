//! Tests for archive format converters
//!
//! Tests use in-memory archives since no archive test files are provided
//! in the test_documents directory.

use bytes::Bytes;
use markitdown::{ConversionOptions, MarkItDown};

// TAR tests (create minimal in-memory test)
#[tokio::test]
async fn test_tar_bytes_conversion() {
    use tar::Builder;

    let md = MarkItDown::new();

    // Create tar content in memory
    let mut builder = Builder::new(Vec::new());

    // Add a text file to the archive
    let content = b"Hello, this is tar archive text content for testing.";
    let mut header = tar::Header::new_gnu();
    header.set_path("test.txt").unwrap();
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();

    builder.append(&header, &content[..]).unwrap();
    let archive_data = builder.into_inner().unwrap();

    let options = ConversionOptions::default().with_extension(".tar");
    let result = md
        .convert_bytes(Bytes::from(archive_data), Some(options))
        .await;
    assert!(
        result.is_ok(),
        "TAR bytes conversion failed: {:?}",
        result.err()
    );
}

// ZIP archive tests (create minimal in-memory test)
#[tokio::test]
async fn test_zip_bytes_conversion() {
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    let md = MarkItDown::new();

    // Create zip content in memory
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buffer);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("test.txt", options).unwrap();
        zip.write_all(b"Hello, this is zip archive text content for testing.")
            .unwrap();
        zip.finish().unwrap();
    }

    let options = ConversionOptions::default().with_extension(".zip");
    let result = md
        .convert_bytes(Bytes::from(buffer.into_inner()), Some(options))
        .await;
    assert!(
        result.is_ok(),
        "ZIP bytes conversion failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_zip_with_multiple_files() {
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    let md = MarkItDown::new();

    // Create zip with multiple files
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buffer);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

        zip.start_file("file1.txt", options).unwrap();
        zip.write_all(b"Content of file 1").unwrap();

        zip.start_file("file2.txt", options).unwrap();
        zip.write_all(b"Content of file 2").unwrap();

        zip.start_file("subdir/file3.txt", options).unwrap();
        zip.write_all(b"Content of file 3 in subdir").unwrap();

        zip.finish().unwrap();
    }

    let options = ConversionOptions::default().with_extension(".zip");
    let result = md
        .convert_bytes(Bytes::from(buffer.into_inner()), Some(options))
        .await;
    assert!(result.is_ok(), "ZIP conversion failed: {:?}", result.err());

    let doc = result.unwrap();
    let markdown = doc.to_markdown();
    assert!(!markdown.is_empty(), "ZIP should produce output");
}

// GZIP tests (create minimal in-memory test)
#[tokio::test]
async fn test_gzip_bytes_conversion() {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    let md = MarkItDown::new();

    // Create gzipped content in memory
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(b"Hello, this is gzipped text content for testing.")
        .unwrap();
    let compressed = encoder.finish().unwrap();

    let options = ConversionOptions::default().with_extension(".gz");
    let result = md
        .convert_bytes(Bytes::from(compressed), Some(options))
        .await;
    assert!(
        result.is_ok(),
        "GZIP bytes conversion failed: {:?}",
        result.err()
    );

    let doc = result.unwrap();
    let markdown = doc.to_markdown();
    assert!(
        markdown.contains("Hello") || markdown.contains("gzipped"),
        "GZIP content should be readable"
    );
}

// BZIP2 tests (create minimal in-memory test)
#[tokio::test]
async fn test_bzip2_bytes_conversion() {
    use bzip2::write::BzEncoder;
    use bzip2::Compression;
    use std::io::Write;

    let md = MarkItDown::new();

    // Create bzip2 content in memory
    let mut encoder = BzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(b"Hello, this is bzip2 compressed text content for testing.")
        .unwrap();
    let compressed = encoder.finish().unwrap();

    let options = ConversionOptions::default().with_extension(".bz2");
    let result = md
        .convert_bytes(Bytes::from(compressed), Some(options))
        .await;
    assert!(
        result.is_ok(),
        "BZIP2 bytes conversion failed: {:?}",
        result.err()
    );

    let doc = result.unwrap();
    let markdown = doc.to_markdown();
    assert!(
        markdown.contains("Hello") || markdown.contains("bzip2"),
        "BZIP2 content should be readable"
    );
}

// XZ tests (create minimal in-memory test)
#[tokio::test]
async fn test_xz_bytes_conversion() {
    use std::io::Write;
    use liblzma::write::XzEncoder;

    let md = MarkItDown::new();

    // Create xz content in memory
    let mut encoder = XzEncoder::new(Vec::new(), 6);
    encoder
        .write_all(b"Hello, this is xz compressed text content for testing.")
        .unwrap();
    let compressed = encoder.finish().unwrap();

    let options = ConversionOptions::default().with_extension(".xz");
    let result = md
        .convert_bytes(Bytes::from(compressed), Some(options))
        .await;
    assert!(
        result.is_ok(),
        "XZ bytes conversion failed: {:?}",
        result.err()
    );

    let doc = result.unwrap();
    let markdown = doc.to_markdown();
    assert!(
        markdown.contains("Hello") || markdown.contains("xz"),
        "XZ content should be readable"
    );
}

// ZSTD tests (create minimal in-memory test)
#[tokio::test]
async fn test_zstd_bytes_conversion() {
    let md = MarkItDown::new();

    // Create zstd content in memory
    let original = b"Hello, this is zstd compressed text content for testing.";
    let compressed = zstd::encode_all(&original[..], 3).unwrap();

    let options = ConversionOptions::default().with_extension(".zst");
    let result = md
        .convert_bytes(Bytes::from(compressed), Some(options))
        .await;
    assert!(
        result.is_ok(),
        "ZSTD bytes conversion failed: {:?}",
        result.err()
    );

    let doc = result.unwrap();
    let markdown = doc.to_markdown();
    assert!(
        markdown.contains("Hello") || markdown.contains("zstd"),
        "ZSTD content should be readable"
    );
}
