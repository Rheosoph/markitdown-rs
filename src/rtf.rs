//! RTF (Rich Text Format) to Markdown converter.
//!
//! Supports conversion of RTF documents to markdown text.

use async_trait::async_trait;
use bytes::Bytes;
use object_store::ObjectStore;
use rtf_parser::document::RtfDocument;
use std::sync::Arc;

use crate::error::MarkitdownError;
use crate::model::{ContentBlock, ConversionOptions, Document, DocumentConverter, Page};

/// RTF document converter
pub struct RtfConverter;

impl RtfConverter {
    /// Extract text content from RTF document
    fn extract_text(bytes: &[u8]) -> Result<String, MarkitdownError> {
        let content = String::from_utf8_lossy(bytes).to_string();

        let doc = RtfDocument::try_from(content)
            .map_err(|e| MarkitdownError::ParseError(format!("RTF parser error: {:?}", e)))?;

        // Use the built-in get_text method
        let text = doc.get_text();

        Ok(text.trim().to_string())
    }
}

#[async_trait]
impl DocumentConverter for RtfConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        if let Some(opts) = &options {
            if let Some(ext) = &opts.file_extension {
                if ext.to_lowercase() != ".rtf" {
                    return Err(MarkitdownError::InvalidFile(format!(
                        "Expected .rtf file, got {}",
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
                if ext.to_lowercase() != ".rtf" {
                    return Err(MarkitdownError::InvalidFile(format!(
                        "Expected .rtf file, got {}",
                        ext
                    )));
                }
            }
        }

        let text = Self::extract_text(&bytes)?;

        let mut document = Document::new();
        let mut page = Page::new(1);

        if !text.is_empty() {
            page.add_content(ContentBlock::Text(text));
        }

        document.add_page(page);
        Ok(document)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".rtf"]
    }
}
