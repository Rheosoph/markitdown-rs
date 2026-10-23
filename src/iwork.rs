//! Apple iWork format converters (.pages, .numbers, .key).
//!
//! iWork formats are ZIP-based with protobuf data and preview images.
//! We extract what text we can from the IWA files or fall back to metadata.

use async_trait::async_trait;
use bytes::Bytes;
use object_store::{ObjectStore, ObjectStoreExt};
use std::io::{Cursor, Read};
use std::sync::Arc;
use zip::ZipArchive;

use crate::error::MarkitdownError;
use crate::model::{ContentBlock, ConversionOptions, Document, DocumentConverter, Page};

/// Apple Pages document converter
pub struct PagesConverter;

impl PagesConverter {
    fn convert_iwork(bytes: &[u8], format_name: &str) -> Result<Document, MarkitdownError> {
        let cursor = Cursor::new(bytes);
        let mut archive = ZipArchive::new(cursor).map_err(|e| {
            MarkitdownError::ParseError(format!("{} parse error: {}", format_name, e))
        })?;

        let mut document = Document::new();
        let mut page = Page::new(1);
        let mut markdown = String::new();

        markdown.push_str(&format!("# Apple {} Document\n\n", format_name));

        // Try to read preview text or metadata
        let mut found_text = false;

        // Look for preview/preview.pdf or QuickLook/Preview.pdf
        let preview_paths = [
            "preview.pdf",
            "QuickLook/Preview.pdf",
            "QuickLook/Thumbnail.jpg",
        ];

        for path in preview_paths {
            if archive.by_name(path).is_ok() {
                markdown.push_str(&format!("*Preview available: {}*\n\n", path));
                break;
            }
        }

        // Try to extract text from Index files (older format)
        for i in 0..archive.len() {
            if let Ok(mut file) = archive.by_index(i) {
                let name = file.name().unwrap_or_default().into_owned();

                // Look for text content in various locations
                if name.ends_with(".txt") || name.contains("Text") {
                    let mut content = String::new();
                    if file.read_to_string(&mut content).is_ok() && !content.is_empty() {
                        markdown.push_str(&content);
                        markdown.push_str("\n\n");
                        found_text = true;
                    }
                }

                // Try to read from buildVersionHistory.plist for metadata
                if name == "buildVersionHistory.plist"
                    || name.ends_with("buildVersionHistory.plist")
                {
                    let mut content = String::new();
                    if file.read_to_string(&mut content).is_ok() {
                        markdown.push_str("**Version Info:**\n```xml\n");
                        markdown.push_str(&content);
                        markdown.push_str("\n```\n\n");
                    }
                }
            }
        }

        // Try to extract any readable strings from IWA files
        if !found_text {
            let mut all_text = String::new();

            for i in 0..archive.len() {
                if let Ok(mut file) = archive.by_index(i) {
                    let name = file.name().unwrap_or_default().into_owned();

                    if name.ends_with(".iwa") {
                        let mut data = Vec::new();
                        if file.read_to_end(&mut data).is_ok() {
                            // IWA files are compressed protobuf, extract readable strings
                            if let Some(text) = Self::extract_strings_from_binary(&data) {
                                all_text.push_str(&text);
                                all_text.push('\n');
                            }
                        }
                    }
                }
            }

            if !all_text.is_empty() {
                markdown.push_str("**Extracted Text:**\n\n");
                markdown.push_str(&all_text);
                found_text = true;
            }
        }

        if !found_text {
            markdown.push_str(&format!(
                "*Unable to extract text from {} file.*\n\n\
                Apple iWork files use a proprietary format. \
                Consider exporting to PDF, DOCX, or another format for better extraction.\n\n",
                format_name
            ));
        }

        // List archive contents
        markdown.push_str("---\n\n**Archive Contents:**\n");
        for i in 0..archive.len() {
            if let Ok(file) = archive.by_index(i) {
                if let Ok(name) = file.name() {
                    markdown.push_str(&format!("- `{}`\n", name));
                }
            }
        }

        page.add_content(ContentBlock::Markdown(markdown));
        document.add_page(page);
        Ok(document)
    }

    fn extract_strings_from_binary(data: &[u8]) -> Option<String> {
        // Try to decompress if it's snappy-compressed (common in iWork)
        let decompressed = Self::try_decompress(data);
        let data = decompressed.as_deref().unwrap_or(data);

        let mut text = String::new();
        let mut consecutive_printable = 0;
        let mut buffer = String::new();

        for &byte in data {
            if (0x20..0x7F).contains(&byte) {
                buffer.push(byte as char);
                consecutive_printable += 1;
            } else if byte == b'\n' || byte == b'\r' || byte == b'\t' {
                buffer.push(if byte == b'\t' { ' ' } else { '\n' });
                consecutive_printable += 1;
            } else {
                // Only keep runs of printable text longer than 8 chars
                if consecutive_printable >= 8 {
                    // Filter out likely binary garbage
                    let clean: String = buffer
                        .chars()
                        .filter(|c| {
                            c.is_alphanumeric()
                                || c.is_whitespace()
                                || ".,!?;:'-\"()[]{}".contains(*c)
                        })
                        .collect();
                    if clean.len() >= 8 {
                        text.push_str(&clean);
                        text.push(' ');
                    }
                }
                buffer.clear();
                consecutive_printable = 0;
            }
        }

        if consecutive_printable >= 8 {
            let clean: String = buffer
                .chars()
                .filter(|c| {
                    c.is_alphanumeric() || c.is_whitespace() || ".,!?;:'-\"()[]{}".contains(*c)
                })
                .collect();
            if clean.len() >= 8 {
                text.push_str(&clean);
            }
        }

        // Clean up
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");

        if text.len() > 30 {
            Some(text)
        } else {
            None
        }
    }

    fn try_decompress(data: &[u8]) -> Option<Vec<u8>> {
        // Try gzip decompression
        use flate2::read::GzDecoder;
        let cursor = Cursor::new(data);
        let mut decoder = GzDecoder::new(cursor);
        let mut decompressed = Vec::new();
        if decoder.read_to_end(&mut decompressed).is_ok() && !decompressed.is_empty() {
            return Some(decompressed);
        }
        None
    }
}

#[async_trait]
impl DocumentConverter for PagesConverter {
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
        Self::convert_iwork(&bytes, "Pages")
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".pages"]
    }
}

/// Apple Numbers spreadsheet converter
pub struct NumbersConverter;

#[async_trait]
impl DocumentConverter for NumbersConverter {
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
        PagesConverter::convert_iwork(&bytes, "Numbers")
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".numbers"]
    }
}

/// Apple Keynote presentation converter
pub struct KeynoteConverter;

#[async_trait]
impl DocumentConverter for KeynoteConverter {
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
        PagesConverter::convert_iwork(&bytes, "Keynote")
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".key"]
    }
}
