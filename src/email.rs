//! Email (EML/MSG) to Markdown converter.
//!
//! Supports conversion of email files to markdown,
//! including headers, body text, and attachment listings.

use async_trait::async_trait;
use bytes::Bytes;
use mail_parser::{MessageParser, MimeHeaders};
use object_store::ObjectStore;
use std::sync::Arc;

use crate::error::MarkitdownError;
use crate::model::{ContentBlock, ConversionOptions, Document, DocumentConverter, Page};

/// Email document converter
pub struct EmailConverter;

impl EmailConverter {
    /// Convert email content to markdown
    fn convert_email(bytes: &[u8]) -> Result<Document, MarkitdownError> {
        let message = MessageParser::default()
            .parse(bytes)
            .ok_or_else(|| MarkitdownError::ParseError("Failed to parse email".to_string()))?;

        let mut document = Document::new();
        let mut page = Page::new(1);
        let mut markdown = String::new();

        // Email headers
        markdown.push_str("# Email\n\n");

        // From
        if let Some(from) = message.from() {
            let from_str: Vec<String> = from
                .iter()
                .map(|addr| {
                    if let Some(name) = addr.name() {
                        format!("{} <{}>", name, addr.address().unwrap_or(""))
                    } else {
                        addr.address().unwrap_or("").to_string()
                    }
                })
                .collect();
            if !from_str.is_empty() {
                markdown.push_str(&format!("**From:** {}\n\n", from_str.join(", ")));
            }
        }

        // To
        if let Some(to) = message.to() {
            let to_str: Vec<String> = to
                .iter()
                .map(|addr| {
                    if let Some(name) = addr.name() {
                        format!("{} <{}>", name, addr.address().unwrap_or(""))
                    } else {
                        addr.address().unwrap_or("").to_string()
                    }
                })
                .collect();
            if !to_str.is_empty() {
                markdown.push_str(&format!("**To:** {}\n\n", to_str.join(", ")));
            }
        }

        // CC
        if let Some(cc) = message.cc() {
            let cc_str: Vec<String> = cc
                .iter()
                .map(|addr| {
                    if let Some(name) = addr.name() {
                        format!("{} <{}>", name, addr.address().unwrap_or(""))
                    } else {
                        addr.address().unwrap_or("").to_string()
                    }
                })
                .collect();
            if !cc_str.is_empty() {
                markdown.push_str(&format!("**CC:** {}\n\n", cc_str.join(", ")));
            }
        }

        // Subject
        if let Some(subject) = message.subject() {
            markdown.push_str(&format!("**Subject:** {}\n\n", subject));
        }

        // Date
        if let Some(date) = message.date() {
            markdown.push_str(&format!("**Date:** {}\n\n", date));
        }

        markdown.push_str("---\n\n");

        // Body - prefer plain text, fallback to HTML
        let body = message.body_text(0).map(|s| s.to_string()).or_else(|| {
            message
                .body_html(0)
                .and_then(|html| htmd::convert(&html).ok())
        });

        if let Some(body_text) = body {
            markdown.push_str(&body_text);
            markdown.push_str("\n\n");
        }

        // List attachments
        let attachments: Vec<_> = message.attachments().collect();
        if !attachments.is_empty() {
            markdown.push_str("---\n\n## Attachments\n\n");
            for attachment in attachments {
                let name = attachment.attachment_name().unwrap_or("unnamed");
                let size = attachment.len();
                let content_type = attachment
                    .content_type()
                    .map(|ct| ct.c_type.as_ref())
                    .unwrap_or("unknown");
                markdown.push_str(&format!(
                    "- **{}** ({}, {} bytes)\n",
                    name, content_type, size
                ));
            }
        }

        page.add_content(ContentBlock::Markdown(markdown.trim().to_string()));
        document.add_page(page);

        Ok(document)
    }
}

#[async_trait]
impl DocumentConverter for EmailConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        if let Some(opts) = &options {
            if let Some(ext) = &opts.file_extension {
                let ext_lower = ext.to_lowercase();
                if ext_lower != ".eml" && ext_lower != ".msg" {
                    return Err(MarkitdownError::InvalidFile(format!(
                        "Expected .eml or .msg file, got {}",
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
                let ext_lower = ext.to_lowercase();
                if ext_lower != ".eml" && ext_lower != ".msg" {
                    return Err(MarkitdownError::InvalidFile(format!(
                        "Expected .eml or .msg file, got {}",
                        ext
                    )));
                }
            }
        }

        Self::convert_email(&bytes)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".eml", ".msg"]
    }
}
