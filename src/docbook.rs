//! DocBook XML to Markdown converter.
//!
//! Converts DocBook XML documents to markdown.

use async_trait::async_trait;
use bytes::Bytes;
use object_store::ObjectStore;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::sync::Arc;

use crate::error::MarkitdownError;
use crate::model::{ContentBlock, ConversionOptions, Document, DocumentConverter, Page};

/// DocBook XML to Markdown converter
pub struct DocBookConverter;

impl DocBookConverter {
    fn convert_docbook(bytes: &[u8]) -> Result<Document, MarkitdownError> {
        let content = String::from_utf8_lossy(bytes);
        let mut document = Document::new();
        let mut page = Page::new(1);

        let markdown = Self::docbook_to_markdown(&content)?;
        page.add_content(ContentBlock::Markdown(markdown));
        document.add_page(page);

        Ok(document)
    }

    fn docbook_to_markdown(content: &str) -> Result<String, MarkitdownError> {
        let mut reader = Reader::from_str(content);
        reader.config_mut().trim_text(true);

        let mut result = String::new();
        let mut buf = Vec::new();
        let mut text_stack: Vec<String> = Vec::new();
        let mut in_para = false;
        let mut in_emphasis = false;
        let mut emphasis_role = String::new();
        let mut in_code = false;
        let mut in_link = false;
        let mut link_url = String::new();
        let mut list_depth: usize = 0;
        let mut in_listitem = false;

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    match e.name().as_ref() {
                        // Sections and titles
                        b"chapter" | b"article" | b"book" => {}
                        b"section" | b"sect1" | b"sect2" | b"sect3" | b"sect4" => {}
                        b"title" => {
                            text_stack.push(String::new());
                        }
                        b"para" | b"simpara" => {
                            in_para = true;
                            text_stack.push(String::new());
                        }

                        // Inline formatting
                        b"emphasis" => {
                            in_emphasis = true;
                            // Check for role attribute
                            let decoder = reader.decoder();
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"role" {
                                    emphasis_role = attr
                                        .decode_and_unescape_value(decoder)
                                        .map(|s| s.to_string())
                                        .unwrap_or_default();
                                }
                            }
                        }
                        b"literal" | b"code" | b"computeroutput" => {
                            in_code = true;
                        }
                        b"link" | b"ulink" | b"xref" => {
                            in_link = true;
                            let decoder = reader.decoder();
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"url"
                                    || attr.key.as_ref() == b"xlink:href"
                                    || attr.key.as_ref() == b"linkend"
                                {
                                    link_url = attr
                                        .decode_and_unescape_value(decoder)
                                        .map(|s| s.to_string())
                                        .unwrap_or_default();
                                }
                            }
                        }

                        // Lists
                        b"itemizedlist" | b"orderedlist" | b"variablelist" => {
                            list_depth += 1;
                        }
                        b"listitem" => {
                            in_listitem = true;
                        }

                        // Code blocks
                        b"programlisting" | b"screen" | b"literallayout" => {
                            result.push_str("```\n");
                        }

                        // Block quotes
                        b"blockquote" => {
                            result.push_str("> ");
                        }

                        _ => {}
                    }
                }
                Ok(Event::End(ref e)) => {
                    match e.name().as_ref() {
                        b"title" => {
                            if let Some(text) = text_stack.pop() {
                                // Determine heading level based on context
                                // For simplicity, use ## for all titles
                                result.push_str(&format!("## {}\n\n", text.trim()));
                            }
                        }
                        b"para" | b"simpara" => {
                            if let Some(text) = text_stack.pop() {
                                let prefix = if in_listitem && list_depth > 0 {
                                    let indent = "  ".repeat(list_depth.saturating_sub(1));
                                    format!("{}- ", indent)
                                } else {
                                    String::new()
                                };
                                result.push_str(&format!("{}{}\n\n", prefix, text.trim()));
                            }
                            in_para = false;
                        }
                        b"emphasis" => {
                            in_emphasis = false;
                            emphasis_role.clear();
                        }
                        b"literal" | b"code" | b"computeroutput" => {
                            in_code = false;
                        }
                        b"link" | b"ulink" | b"xref" => {
                            in_link = false;
                            link_url.clear();
                        }
                        b"itemizedlist" | b"orderedlist" | b"variablelist" => {
                            list_depth = list_depth.saturating_sub(1);
                        }
                        b"listitem" => {
                            in_listitem = false;
                        }
                        b"programlisting" | b"screen" | b"literallayout" => {
                            result.push_str("```\n\n");
                        }
                        _ => {}
                    }
                }
                Ok(Event::Text(e)) => {
                    let text = e.decode().map(|s| s.to_string()).unwrap_or_default();

                    // Apply formatting
                    let formatted = if in_code {
                        format!("`{}`", text)
                    } else if in_emphasis {
                        if emphasis_role == "bold" || emphasis_role == "strong" {
                            format!("**{}**", text)
                        } else {
                            format!("*{}*", text)
                        }
                    } else if in_link && !link_url.is_empty() {
                        format!("[{}]({})", text, link_url)
                    } else {
                        text
                    };

                    // Add to current context
                    if let Some(current) = text_stack.last_mut() {
                        current.push_str(&formatted);
                    } else if in_para || !formatted.trim().is_empty() {
                        result.push_str(&formatted);
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => {
                    return Err(MarkitdownError::ParseError(format!(
                        "DocBook parse error: {}",
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
impl DocumentConverter for DocBookConverter {
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
        Self::convert_docbook(&bytes)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".docbook", ".docbook4", ".docbook5", ".dbk"]
    }
}
