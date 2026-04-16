//! Archive format converters with recursive extraction support.
//!
//! Supports: .zip, .tar, .gz, .gzip, .tar.gz, .tgz, .bz2, .tar.bz2, .xz, .tar.xz,
//! .zst, .tar.zst, .7z

use async_trait::async_trait;
use bytes::Bytes;
use bzip2::read::BzDecoder;
use flate2::read::GzDecoder;
use liblzma::read::XzDecoder;
use object_store::ObjectStore;
use std::io::{Cursor, Read};
use std::sync::Arc;
use zstd::stream::read::Decoder as ZstdDecoder;

use crate::error::MarkitdownError;
use crate::model::{ContentBlock, ConversionOptions, Document, DocumentConverter, Page};

/// Result containing multiple documents from archive extraction
#[derive(Debug, Clone)]
pub struct ArchiveExtractionResult {
    pub documents: Vec<(String, Document)>,
    /// Files that couldn't be converted (path, reason)
    pub skipped_files: Vec<(String, String)>,
    pub total_files: usize,
}

impl ArchiveExtractionResult {
    pub fn new() -> Self {
        Self {
            documents: Vec::new(),
            skipped_files: Vec::new(),
            total_files: 0,
        }
    }

    /// Convert to a single Document with summary
    pub fn to_document(&self, archive_name: &str) -> Document {
        let mut document = Document::new();
        let mut page = Page::new(1);
        let mut markdown = String::new();

        markdown.push_str(&format!("# Archive: {}\n\n", archive_name));
        markdown.push_str(&format!("**Total files:** {}\n", self.total_files));
        markdown.push_str(&format!("**Converted:** {}\n", self.documents.len()));
        markdown.push_str(&format!("**Skipped:** {}\n\n", self.skipped_files.len()));

        markdown.push_str("## Contents\n\n");

        for (path, doc) in &self.documents {
            markdown.push_str(&format!("### 📄 {}\n\n", path));
            markdown.push_str(&doc.to_markdown());
            markdown.push_str("\n---\n\n");
        }

        if !self.skipped_files.is_empty() {
            markdown.push_str("## Skipped Files\n\n");
            for (path, reason) in &self.skipped_files {
                markdown.push_str(&format!("- `{}`: {}\n", path, reason));
            }
        }

        page.add_content(ContentBlock::Markdown(markdown));
        document.add_page(page);
        document
    }
}

impl Default for ArchiveExtractionResult {
    fn default() -> Self {
        Self::new()
    }
}

/// ZIP archive converter
pub struct ZipConverter;

impl ZipConverter {
    fn extract_zip(bytes: &[u8]) -> Result<ArchiveExtractionResult, MarkitdownError> {
        let cursor = Cursor::new(bytes);
        let mut archive = zip::ZipArchive::new(cursor)
            .map_err(|e| MarkitdownError::ParseError(format!("ZIP parse error: {}", e)))?;

        let mut result = ArchiveExtractionResult::new();
        result.total_files = archive.len();

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| MarkitdownError::ParseError(format!("ZIP entry error: {}", e)))?;

            if file.is_dir() {
                continue;
            }

            let path = file.name().to_string();
            let mut contents = Vec::new();

            if file.read_to_end(&mut contents).is_ok() {
                result = Self::process_file(result, &path, &contents);
            }
        }

        Ok(result)
    }

    fn process_file(
        mut result: ArchiveExtractionResult,
        path: &str,
        contents: &[u8],
    ) -> ArchiveExtractionResult {
        // Try to convert the file based on extension
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{}", e.to_lowercase()))
            .unwrap_or_default();

        // For text-based files, create a simple document
        if is_text_extension(&ext) {
            if let Ok(text) = String::from_utf8(contents.to_vec()) {
                let mut doc = Document::new();
                let mut page = Page::new(1);
                page.add_content(ContentBlock::Markdown(format!("```\n{}\n```", text)));
                doc.add_page(page);
                result.documents.push((path.to_string(), doc));
            } else {
                result
                    .skipped_files
                    .push((path.to_string(), "Binary file".to_string()));
            }
        } else {
            result
                .skipped_files
                .push((path.to_string(), format!("Unsupported format: {}", ext)));
        }

        result
    }
}

/// Check if extension is a text-based format
fn is_text_extension(ext: &str) -> bool {
    matches!(
        ext,
        ".txt"
            | ".md"
            | ".markdown"
            | ".json"
            | ".yaml"
            | ".yml"
            | ".toml"
            | ".xml"
            | ".html"
            | ".htm"
            | ".css"
            | ".js"
            | ".ts"
            | ".jsx"
            | ".tsx"
            | ".rs"
            | ".py"
            | ".go"
            | ".java"
            | ".c"
            | ".cpp"
            | ".h"
            | ".hpp"
            | ".sh"
            | ".bash"
            | ".zsh"
            | ".fish"
            | ".ps1"
            | ".bat"
            | ".cmd"
            | ".sql"
            | ".csv"
            | ".log"
            | ".ini"
            | ".cfg"
            | ".conf"
            | ".env"
            | ".gitignore"
            | ".dockerfile"
            | ".makefile"
            | ".cmake"
            | ".rb"
            | ".php"
            | ".swift"
            | ".kt"
            | ".scala"
            | ".clj"
            | ".ex"
            | ".exs"
            | ".lua"
            | ".r"
            | ".pl"
            | ".pm"
            | ".tcl"
            | ".awk"
            | ".sed"
            | ".vue"
            | ".svelte"
            | ".astro"
            | ".bib"
            | ".tex"
            | ".sty"
    )
}

#[async_trait]
impl DocumentConverter for ZipConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let result = store.get(path).await?;
        let bytes = result.bytes().await?;
        self.convert_bytes(bytes, options).await
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        _options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let result = Self::extract_zip(&bytes)?;
        Ok(result.to_document("archive.zip"))
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".zip"]
    }
}

/// TAR archive converter (uncompressed)
pub struct TarConverter;

impl TarConverter {
    fn extract_tar(bytes: &[u8]) -> Result<ArchiveExtractionResult, MarkitdownError> {
        let cursor = Cursor::new(bytes);
        let mut archive = tar::Archive::new(cursor);

        let mut result = ArchiveExtractionResult::new();

        let entries = archive
            .entries()
            .map_err(|e| MarkitdownError::ParseError(format!("TAR parse error: {}", e)))?;

        for entry in entries {
            let mut entry = entry
                .map_err(|e| MarkitdownError::ParseError(format!("TAR entry error: {}", e)))?;

            result.total_files += 1;

            if entry.header().entry_type().is_dir() {
                continue;
            }

            let path = entry
                .path()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| "unknown".to_string());

            let mut contents = Vec::new();
            if entry.read_to_end(&mut contents).is_ok() {
                result = ZipConverter::process_file(result, &path, &contents);
            }
        }

        Ok(result)
    }
}

#[async_trait]
impl DocumentConverter for TarConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let result = store.get(path).await?;
        let bytes = result.bytes().await?;
        self.convert_bytes(bytes, options).await
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        _options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let result = Self::extract_tar(&bytes)?;
        Ok(result.to_document("archive.tar"))
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".tar"]
    }
}

/// Gzip compressed file converter (handles .gz and .tar.gz)
pub struct GzipConverter;

impl GzipConverter {
    fn decompress_gz(bytes: &[u8]) -> Result<Vec<u8>, MarkitdownError> {
        let cursor = Cursor::new(bytes);
        let mut decoder = GzDecoder::new(cursor);
        let mut decompressed = Vec::new();
        decoder
            .read_to_end(&mut decompressed)
            .map_err(|e| MarkitdownError::ParseError(format!("Gzip decompress error: {}", e)))?;
        Ok(decompressed)
    }
}

#[async_trait]
impl DocumentConverter for GzipConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let result = store.get(path).await?;
        let bytes = result.bytes().await?;
        self.convert_bytes(bytes, options).await
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        _options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let decompressed = Self::decompress_gz(&bytes)?;

        // Check if it's a tar archive inside
        if let Ok(result) = TarConverter::extract_tar(&decompressed) {
            if result.total_files > 0 {
                return Ok(result.to_document("archive.tar.gz"));
            }
        }

        // Otherwise, treat as single file
        let mut doc = Document::new();
        let mut page = Page::new(1);

        if let Ok(text) = String::from_utf8(decompressed.clone()) {
            page.add_content(ContentBlock::Markdown(format!("```\n{}\n```", text)));
        } else {
            page.add_content(ContentBlock::Text(format!(
                "[Binary data: {} bytes]",
                decompressed.len()
            )));
        }

        doc.add_page(page);
        Ok(doc)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".gz", ".gzip", ".tgz"]
    }
}

/// Bzip2 compressed file converter
pub struct Bzip2Converter;

impl Bzip2Converter {
    fn decompress_bz2(bytes: &[u8]) -> Result<Vec<u8>, MarkitdownError> {
        let cursor = Cursor::new(bytes);
        let mut decoder = BzDecoder::new(cursor);
        let mut decompressed = Vec::new();
        decoder
            .read_to_end(&mut decompressed)
            .map_err(|e| MarkitdownError::ParseError(format!("Bzip2 decompress error: {}", e)))?;
        Ok(decompressed)
    }
}

#[async_trait]
impl DocumentConverter for Bzip2Converter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let result = store.get(path).await?;
        let bytes = result.bytes().await?;
        self.convert_bytes(bytes, options).await
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        _options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let decompressed = Self::decompress_bz2(&bytes)?;

        // Check if it's a tar archive inside
        if let Ok(result) = TarConverter::extract_tar(&decompressed) {
            if result.total_files > 0 {
                return Ok(result.to_document("archive.tar.bz2"));
            }
        }

        // Otherwise, treat as single file
        let mut doc = Document::new();
        let mut page = Page::new(1);

        if let Ok(text) = String::from_utf8(decompressed.clone()) {
            page.add_content(ContentBlock::Markdown(format!("```\n{}\n```", text)));
        } else {
            page.add_content(ContentBlock::Text(format!(
                "[Binary data: {} bytes]",
                decompressed.len()
            )));
        }

        doc.add_page(page);
        Ok(doc)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".bz2", ".tbz2", ".tbz"]
    }
}

/// XZ/LZMA compressed file converter
pub struct XzConverter;

impl XzConverter {
    fn decompress_xz(bytes: &[u8]) -> Result<Vec<u8>, MarkitdownError> {
        let cursor = Cursor::new(bytes);
        let mut decoder = XzDecoder::new(cursor);
        let mut decompressed = Vec::new();
        decoder
            .read_to_end(&mut decompressed)
            .map_err(|e| MarkitdownError::ParseError(format!("XZ decompress error: {}", e)))?;
        Ok(decompressed)
    }
}

#[async_trait]
impl DocumentConverter for XzConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let result = store.get(path).await?;
        let bytes = result.bytes().await?;
        self.convert_bytes(bytes, options).await
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        _options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let decompressed = Self::decompress_xz(&bytes)?;

        // Check if it's a tar archive inside
        if let Ok(result) = TarConverter::extract_tar(&decompressed) {
            if result.total_files > 0 {
                return Ok(result.to_document("archive.tar.xz"));
            }
        }

        // Otherwise, treat as single file
        let mut doc = Document::new();
        let mut page = Page::new(1);

        if let Ok(text) = String::from_utf8(decompressed.clone()) {
            page.add_content(ContentBlock::Markdown(format!("```\n{}\n```", text)));
        } else {
            page.add_content(ContentBlock::Text(format!(
                "[Binary data: {} bytes]",
                decompressed.len()
            )));
        }

        doc.add_page(page);
        Ok(doc)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".xz", ".txz", ".lzma"]
    }
}

/// Zstandard compressed file converter
pub struct ZstdConverter;

impl ZstdConverter {
    fn decompress_zstd(bytes: &[u8]) -> Result<Vec<u8>, MarkitdownError> {
        let cursor = Cursor::new(bytes);
        let mut decoder = ZstdDecoder::new(cursor)
            .map_err(|e| MarkitdownError::ParseError(format!("Zstd init error: {}", e)))?;
        let mut decompressed = Vec::new();
        decoder
            .read_to_end(&mut decompressed)
            .map_err(|e| MarkitdownError::ParseError(format!("Zstd decompress error: {}", e)))?;
        Ok(decompressed)
    }
}

#[async_trait]
impl DocumentConverter for ZstdConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let result = store.get(path).await?;
        let bytes = result.bytes().await?;
        self.convert_bytes(bytes, options).await
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        _options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let decompressed = Self::decompress_zstd(&bytes)?;

        // Check if it's a tar archive inside
        if let Ok(result) = TarConverter::extract_tar(&decompressed) {
            if result.total_files > 0 {
                return Ok(result.to_document("archive.tar.zst"));
            }
        }

        // Otherwise, treat as single file
        let mut doc = Document::new();
        let mut page = Page::new(1);

        if let Ok(text) = String::from_utf8(decompressed.clone()) {
            page.add_content(ContentBlock::Markdown(format!("```\n{}\n```", text)));
        } else {
            page.add_content(ContentBlock::Text(format!(
                "[Binary data: {} bytes]",
                decompressed.len()
            )));
        }

        doc.add_page(page);
        Ok(doc)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".zst", ".zstd", ".tzst"]
    }
}

/// 7-Zip archive converter
pub struct SevenZipConverter;

impl SevenZipConverter {
    fn extract_7z(bytes: &[u8]) -> Result<ArchiveExtractionResult, MarkitdownError> {
        use std::io::Write;

        // sevenz-rust needs a file path, so write to temp file
        let mut temp_file = tempfile::NamedTempFile::new()
            .map_err(|e| MarkitdownError::ParseError(format!("Temp file error: {}", e)))?;
        temp_file
            .write_all(bytes)
            .map_err(|e| MarkitdownError::ParseError(format!("Write error: {}", e)))?;

        let temp_dir = tempfile::TempDir::new()
            .map_err(|e| MarkitdownError::ParseError(format!("Temp dir error: {}", e)))?;

        sevenz_rust::decompress_file(temp_file.path(), temp_dir.path())
            .map_err(|e| MarkitdownError::ParseError(format!("7z decompress error: {}", e)))?;

        let mut result = ArchiveExtractionResult::new();

        // Walk the extracted directory
        fn walk_dir(
            dir: &std::path::Path,
            base: &std::path::Path,
            result: &mut ArchiveExtractionResult,
        ) {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        walk_dir(&path, base, result);
                    } else {
                        result.total_files += 1;
                        let rel_path = path
                            .strip_prefix(base)
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_else(|_| path.to_string_lossy().to_string());

                        if let Ok(contents) = std::fs::read(&path) {
                            *result = ZipConverter::process_file(
                                std::mem::take(result),
                                &rel_path,
                                &contents,
                            );
                        }
                    }
                }
            }
        }

        walk_dir(temp_dir.path(), temp_dir.path(), &mut result);
        Ok(result)
    }
}

#[async_trait]
impl DocumentConverter for SevenZipConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let result = store.get(path).await?;
        let bytes = result.bytes().await?;
        self.convert_bytes(bytes, options).await
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        _options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let result = Self::extract_7z(&bytes)?;
        Ok(result.to_document("archive.7z"))
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".7z"]
    }
}

/// Unified archive converter that handles all supported formats
pub struct ArchiveConverter {
    zip: ZipConverter,
    tar: TarConverter,
    gzip: GzipConverter,
    bzip2: Bzip2Converter,
    xz: XzConverter,
    zstd: ZstdConverter,
    sevenz: SevenZipConverter,
}

impl ArchiveConverter {
    pub fn new() -> Self {
        Self {
            zip: ZipConverter,
            tar: TarConverter,
            gzip: GzipConverter,
            bzip2: Bzip2Converter,
            xz: XzConverter,
            zstd: ZstdConverter,
            sevenz: SevenZipConverter,
        }
    }
}

impl Default for ArchiveConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DocumentConverter for ArchiveConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let result = store.get(path).await?;
        let bytes = result.bytes().await?;
        self.convert_bytes(bytes, options).await
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        // Determine format from options or try to detect
        let ext = options
            .as_ref()
            .and_then(|o| o.file_extension.as_ref())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        match ext.as_str() {
            ".zip" => self.zip.convert_bytes(bytes, options).await,
            ".tar" => self.tar.convert_bytes(bytes, options).await,
            ".gz" | ".gzip" | ".tgz" | ".tar.gz" => self.gzip.convert_bytes(bytes, options).await,
            ".bz2" | ".tbz2" | ".tbz" | ".tar.bz2" => {
                self.bzip2.convert_bytes(bytes, options).await
            }
            ".xz" | ".txz" | ".lzma" | ".tar.xz" => self.xz.convert_bytes(bytes, options).await,
            ".zst" | ".zstd" | ".tzst" | ".tar.zst" => {
                self.zstd.convert_bytes(bytes, options).await
            }
            ".7z" => self.sevenz.convert_bytes(bytes, options).await,
            _ => {
                // Try to detect format from magic bytes
                if bytes.len() >= 4 {
                    // ZIP magic: PK\x03\x04
                    if bytes[0..4] == [0x50, 0x4B, 0x03, 0x04] {
                        return self.zip.convert_bytes(bytes, options).await;
                    }
                    // GZIP magic: \x1f\x8b
                    if bytes[0..2] == [0x1f, 0x8b] {
                        return self.gzip.convert_bytes(bytes, options).await;
                    }
                    // BZ2 magic: BZ
                    if bytes[0..2] == [0x42, 0x5a] {
                        return self.bzip2.convert_bytes(bytes, options).await;
                    }
                    // XZ magic: \xfd7zXZ\x00
                    if bytes.len() >= 6 && bytes[0..6] == [0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00] {
                        return self.xz.convert_bytes(bytes, options).await;
                    }
                    // ZSTD magic: 0x28 0xb5 0x2f 0xfd
                    if bytes[0..4] == [0x28, 0xb5, 0x2f, 0xfd] {
                        return self.zstd.convert_bytes(bytes, options).await;
                    }
                    // 7z magic: 7z\xbc\xaf\x27\x1c
                    if bytes.len() >= 6 && bytes[0..6] == [0x37, 0x7a, 0xbc, 0xaf, 0x27, 0x1c] {
                        return self.sevenz.convert_bytes(bytes, options).await;
                    }
                }

                Err(MarkitdownError::UnsupportedFormat(
                    "Could not detect archive format".to_string(),
                ))
            }
        }
    }

    fn supported_extensions(&self) -> &[&str] {
        &[
            ".zip", ".tar", ".gz", ".gzip", ".tgz", ".tar.gz", ".bz2", ".tbz2", ".tbz", ".tar.bz2",
            ".xz", ".txz", ".lzma", ".tar.xz", ".zst", ".zstd", ".tzst", ".tar.zst", ".7z",
        ]
    }
}
