//! Benchmarks for document conversion operations.
//!
//! Run with: cargo bench
//!
//! To generate HTML report: cargo bench -- --save-baseline main

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use markitdown::MarkItDown;
use std::fs;
use std::hint::black_box;
use std::path::PathBuf;

/// Get the path to document fixtures used by tests and benchmarks.
fn test_documents_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/test_documents")
}

/// Benchmark CSV conversion
fn bench_csv_conversion(c: &mut Criterion) {
    let converter = MarkItDown::new();
    let file_path = test_documents_path().join("spreadsheets/stanley_cups.csv");
    let file_path_str = file_path.to_string_lossy().to_string();
    let file_size = fs::metadata(&file_path).unwrap().len();

    let mut group = c.benchmark_group("csv");
    group.throughput(Throughput::Bytes(file_size));

    group.bench_function("convert", |b| {
        b.iter(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async { converter.convert(&file_path_str, None).await.unwrap() })
        })
    });

    group.finish();
}

/// Benchmark HTML conversion
fn bench_html_conversion(c: &mut Criterion) {
    let converter = MarkItDown::new();
    let file_path = test_documents_path().join("web/html.html");
    let file_path_str = file_path.to_string_lossy().to_string();
    let file_size = fs::metadata(&file_path).unwrap().len();

    let mut group = c.benchmark_group("html");
    group.throughput(Throughput::Bytes(file_size));

    group.bench_function("convert", |b| {
        b.iter(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async { converter.convert(&file_path_str, None).await.unwrap() })
        })
    });

    group.finish();
}

/// Benchmark DOCX conversion
fn bench_docx_conversion(c: &mut Criterion) {
    let converter = MarkItDown::new();
    let file_path = test_documents_path().join("office/document.docx");
    let file_path_str = file_path.to_string_lossy().to_string();
    let file_size = fs::metadata(&file_path).unwrap().len();

    let mut group = c.benchmark_group("docx");
    group.throughput(Throughput::Bytes(file_size));

    group.bench_function("convert", |b| {
        b.iter(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async { converter.convert(&file_path_str, None).await.unwrap() })
        })
    });

    group.finish();
}

/// Benchmark PDF conversion (text-based, no LLM)
fn bench_pdf_conversion(c: &mut Criterion) {
    let converter = MarkItDown::new();
    let file_path = test_documents_path().join("pdfs/fake_memo.pdf");
    let file_path_str = file_path.to_string_lossy().to_string();
    let file_size = fs::metadata(&file_path).unwrap().len();

    let mut group = c.benchmark_group("pdf");
    group.throughput(Throughput::Bytes(file_size));
    // PDF can be slow, reduce sample size
    group.sample_size(20);

    group.bench_function("convert_text", |b| {
        b.iter(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async { converter.convert(&file_path_str, None).await.unwrap() })
        })
    });

    group.finish();
}

/// Benchmark Excel conversion
fn bench_excel_conversion(c: &mut Criterion) {
    let converter = MarkItDown::new();
    let file_path = test_documents_path().join("office/excel.xlsx");
    let file_path_str = file_path.to_string_lossy().to_string();
    let file_size = fs::metadata(&file_path).unwrap().len();

    let mut group = c.benchmark_group("excel");
    group.throughput(Throughput::Bytes(file_size));

    group.bench_function("convert", |b| {
        b.iter(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async { converter.convert(&file_path_str, None).await.unwrap() })
        })
    });

    group.finish();
}

/// Benchmark PowerPoint conversion
fn bench_pptx_conversion(c: &mut Criterion) {
    let converter = MarkItDown::new();
    let file_path = test_documents_path().join("presentations/simple.pptx");
    let file_path_str = file_path.to_string_lossy().to_string();
    let file_size = fs::metadata(&file_path).unwrap().len();

    let mut group = c.benchmark_group("pptx");
    group.throughput(Throughput::Bytes(file_size));

    group.bench_function("convert", |b| {
        b.iter(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async { converter.convert(&file_path_str, None).await.unwrap() })
        })
    });

    group.finish();
}

/// Benchmark RSS/Atom feed conversion
fn bench_rss_conversion(c: &mut Criterion) {
    let converter = MarkItDown::new();
    let file_path = test_documents_path().join("xml/rss_feed.xml");
    let file_path_str = file_path.to_string_lossy().to_string();
    let file_size = fs::metadata(&file_path).unwrap().len();

    let mut group = c.benchmark_group("rss");
    group.throughput(Throughput::Bytes(file_size));

    group.bench_function("convert_atom", |b| {
        b.iter(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async { converter.convert(&file_path_str, None).await.unwrap() })
        })
    });

    group.finish();
}

/// Benchmark image EXIF extraction (no LLM)
fn bench_image_conversion(c: &mut Criterion) {
    let converter = MarkItDown::new();
    let file_path = test_documents_path().join("images/example.jpg");
    let file_path_str = file_path.to_string_lossy().to_string();
    let file_size = fs::metadata(&file_path).unwrap().len();

    let mut group = c.benchmark_group("image");
    group.throughput(Throughput::Bytes(file_size));

    group.bench_function("exif_extraction", |b| {
        b.iter(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async { converter.convert(&file_path_str, None).await.unwrap() })
        })
    });

    group.finish();
}

/// Benchmark ZIP file handling
fn bench_zip_conversion(c: &mut Criterion) {
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    let converter = MarkItDown::new();

    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buffer);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("benchmark.txt", options).unwrap();
        zip.write_all(b"Hello, this is zip archive text content for benchmarking.")
            .unwrap();
        zip.finish().unwrap();
    }

    let zip_bytes = bytes::Bytes::from(buffer.into_inner());
    let file_size = zip_bytes.len() as u64;

    let mut group = c.benchmark_group("zip");
    group.throughput(Throughput::Bytes(file_size));
    // ZIP can contain multiple files, reduce sample size
    group.sample_size(20);

    group.bench_function("convert_bytes", |b| {
        b.iter(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let options = markitdown::ConversionOptions::default().with_extension(".zip");
            let bytes = zip_bytes.clone();
            rt.block_on(async {
                converter
                    .convert_bytes(black_box(bytes), Some(options))
                    .await
                    .unwrap()
            })
        })
    });

    group.finish();
}

/// Comparative benchmark across all document types
fn bench_all_formats(c: &mut Criterion) {
    let converter = MarkItDown::new();
    let test_dir = test_documents_path();

    let files = vec![
        ("csv", "spreadsheets/stanley_cups.csv"),
        ("html", "web/html.html"),
        ("docx", "office/document.docx"),
        ("xlsx", "office/excel.xlsx"),
        ("pptx", "presentations/simple.pptx"),
        ("rss", "xml/rss_feed.xml"),
        ("jpg", "images/example.jpg"),
    ];

    let mut group = c.benchmark_group("format_comparison");

    for (format, filename) in files {
        let file_path = test_dir.join(filename);
        if file_path.exists() {
            let file_path_str = file_path.to_string_lossy().to_string();
            group.bench_with_input(
                BenchmarkId::new("convert", format),
                &file_path_str,
                |b, path| {
                    b.iter(|| {
                        let rt = tokio::runtime::Runtime::new().unwrap();
                        rt.block_on(async {
                            converter
                                .convert(black_box(path.as_str()), None)
                                .await
                                .unwrap()
                        })
                    })
                },
            );
        }
    }

    group.finish();
}

/// Benchmark in-memory conversion using bytes
fn bench_memory_conversion(c: &mut Criterion) {
    let converter = MarkItDown::new();
    let file_path = test_documents_path().join("spreadsheets/stanley_cups.csv");
    let data = fs::read(&file_path).unwrap();
    let bytes = bytes::Bytes::from(data);

    let mut group = c.benchmark_group("memory");
    group.throughput(Throughput::Bytes(bytes.len() as u64));

    group.bench_function("csv_from_bytes", |b| {
        b.iter(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let bytes_clone = bytes.clone();
            rt.block_on(async {
                let opts = markitdown::ConversionOptions::default().with_extension(".csv");
                converter
                    .convert_bytes(black_box(bytes_clone), Some(opts))
                    .await
                    .unwrap()
            })
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_csv_conversion,
    bench_html_conversion,
    bench_docx_conversion,
    bench_pdf_conversion,
    bench_excel_conversion,
    bench_pptx_conversion,
    bench_rss_conversion,
    bench_image_conversion,
    bench_zip_conversion,
    bench_all_formats,
    bench_memory_conversion,
);

criterion_main!(benches);
