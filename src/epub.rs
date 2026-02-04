//! EPUB to Markdown converter.
//!
//! Supports conversion of EPUB ebooks to markdown text,
//! extracting chapters and content.
//!
//! Uses rbook (Apache-2.0 licensed) for EPUB parsing.

use async_trait::async_trait;
use bytes::Bytes;
use object_store::ObjectStore;
use rbook::prelude::*;
use rbook::reader::Reader;
use rbook::Epub;
use std::io::Cursor;
use std::sync::Arc;

use crate::error::MarkitdownError;
use crate::model::{ContentBlock, ConversionOptions, Document, DocumentConverter, Page};

/// EPUB document converter
pub struct EpubConverter;

impl EpubConverter {
    /// Convert EPUB content to markdown
    fn convert_epub(bytes: &[u8]) -> Result<Document, MarkitdownError> {
        let cursor = Cursor::new(bytes.to_vec());
        let epub = Epub::options()
            .strict(false)
            .read(cursor)
            .map_err(|e| MarkitdownError::ParseError(format!("EPUB parse error: {:?}", e)))?;

        let mut document = Document::new();

        // Extract metadata
        let metadata = epub.metadata();
        let title = metadata.title().map(|t| t.value().to_string());
        let author = metadata.creators().next().map(|c| c.value().to_string());

        // Add metadata as first page
        if title.is_some() || author.is_some() {
            let mut meta_page = Page::new(0);
            let mut meta = String::new();

            if let Some(t) = &title {
                meta.push_str(&format!("# {}\n\n", t));
            }
            if let Some(a) = &author {
                meta.push_str(&format!("**Author:** {}\n\n", a));
            }

            meta_page.add_content(ContentBlock::Markdown(meta));
            document.add_page(meta_page);
        }

        // Read content using the Reader API
        let mut reader = epub.reader();
        let mut page_num = 1u32;

        while let Some(result) = reader.read_next() {
            if let Ok(data) = result {
                let content = data.content();
                // Content is XHTML, convert to markdown
                let markdown = htmd::convert(content).unwrap_or_default();
                let cleaned = Self::clean_markdown(&markdown);

                if !cleaned.is_empty() {
                    let mut page = Page::new(page_num);
                    page.add_content(ContentBlock::Markdown(cleaned));
                    document.add_page(page);
                    page_num += 1;
                }
            }
        }

        // If no content was extracted, create a placeholder
        if document.pages.is_empty() {
            let mut page = Page::new(1);
            page.add_content(ContentBlock::Text(
                "[EPUB content could not be extracted]".to_string(),
            ));
            document.add_page(page);
        }

        Ok(document)
    }

    /// Clean up markdown output
    fn clean_markdown(md: &str) -> String {
        md.lines()
            .map(|line| line.trim_end())
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string()
    }
}

#[async_trait]
impl DocumentConverter for EpubConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        if let Some(opts) = &options {
            if let Some(ext) = &opts.file_extension {
                if ext != ".epub" {
                    return Err(MarkitdownError::InvalidFile(format!(
                        "Expected .epub file, got {}",
                        ext
                    )));
                }
            }
        }

        let result = store.get(path).await?;
        let bytes = result.bytes().await?;
        self.convert_bytes(bytes, options).await
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        if let Some(opts) = &options {
            if let Some(ext) = &opts.file_extension {
                if ext != ".epub" {
                    return Err(MarkitdownError::InvalidFile(format!(
                        "Expected .epub file, got {}",
                        ext
                    )));
                }
            }
        }

        Self::convert_epub(&bytes)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".epub"]
    }
}
