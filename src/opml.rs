//! OPML (Outline Processor Markup Language) to Markdown converter.
//!
//! Converts OPML files (commonly used for podcast feeds and outlines) to markdown.

use async_trait::async_trait;
use bytes::Bytes;
use object_store::ObjectStore;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::sync::Arc;

use crate::error::MarkitdownError;
use crate::model::{ContentBlock, ConversionOptions, Document, DocumentConverter, Page};

/// OPML to Markdown converter
pub struct OpmlConverter;

impl OpmlConverter {
    fn convert_opml(bytes: &[u8]) -> Result<Document, MarkitdownError> {
        let content = String::from_utf8_lossy(bytes);
        let mut document = Document::new();
        let mut page = Page::new(1);

        let markdown = Self::opml_to_markdown(&content)?;
        page.add_content(ContentBlock::Markdown(markdown));
        document.add_page(page);

        Ok(document)
    }

    fn opml_to_markdown(content: &str) -> Result<String, MarkitdownError> {
        let mut reader = Reader::from_str(content);
        reader.config_mut().trim_text(true);

        let mut result = String::new();
        let mut title = String::new();
        let mut depth = 0;
        let mut buf = Vec::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                    match e.name().as_ref() {
                        b"title" => {
                            // Read title text
                            if let Ok(Event::Text(t)) = reader.read_event_into(&mut buf) {
                                title = t.decode().map(|s| s.to_string()).unwrap_or_default();
                            }
                        }
                        b"outline" => {
                            // Extract attributes
                            let mut text = String::new();
                            let mut url = String::new();
                            let mut outline_type = String::new();

                            let decoder = reader.decoder();
                            for attr in e.attributes().flatten() {
                                match attr.key.as_ref() {
                                    b"text" => {
                                        text = attr
                                            .decode_and_unescape_value(decoder)
                                            .map(|s| s.to_string())
                                            .unwrap_or_default();
                                    }
                                    b"xmlUrl" | b"htmlUrl" | b"url" => {
                                        if url.is_empty() {
                                            url = attr
                                                .decode_and_unescape_value(decoder)
                                                .map(|s| s.to_string())
                                                .unwrap_or_default();
                                        }
                                    }
                                    b"type" => {
                                        outline_type = attr
                                            .decode_and_unescape_value(decoder)
                                            .map(|s| s.to_string())
                                            .unwrap_or_default();
                                    }
                                    _ => {}
                                }
                            }

                            // Format as markdown list item
                            let indent = "  ".repeat(depth);
                            if !text.is_empty() {
                                if !url.is_empty() {
                                    result.push_str(&format!("{}- [{}]({})\n", indent, text, url));
                                } else {
                                    result.push_str(&format!("{}- {}\n", indent, text));
                                }

                                // Add type info if present
                                if !outline_type.is_empty() && outline_type != "rss" {
                                    result.push_str(&format!(
                                        "{}  *Type: {}*\n",
                                        indent, outline_type
                                    ));
                                }
                            }

                            // Increase depth for nested outlines (only for Start events)
                            if matches!(reader.read_event_into(&mut buf), Ok(Event::Start(_))) {
                                depth += 1;
                            }
                        }
                        b"body" => {
                            // Add title before body content
                            if !title.is_empty() {
                                result.push_str(&format!("# {}\n\n", title));
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Event::End(ref e)) => {
                    if e.name().as_ref() == b"outline" && depth > 0 {
                        depth -= 1;
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => {
                    return Err(MarkitdownError::ParseError(format!(
                        "OPML parse error: {}",
                        e
                    )));
                }
                _ => {}
            }
            buf.clear();
        }

        Ok(result)
    }
}

#[async_trait]
impl DocumentConverter for OpmlConverter {
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
        Self::convert_opml(&bytes)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".opml"]
    }
}
