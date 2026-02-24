// vCard (.vcf) to Markdown converter.
//
// Supports conversion of vCard contact files to markdown.

use async_trait::async_trait;
use bytes::Bytes;
use object_store::ObjectStore;
use std::sync::Arc;
use vcard_parser::parse_vcards;
use vcard_parser::traits::HasValue;

use crate::error::MarkitdownError;
use crate::model::{ContentBlock, ConversionOptions, Document, DocumentConverter, Page};

/// vCard (.vcf) converter
pub struct VCardConverter;

impl VCardConverter {
    fn convert_vcf(bytes: &[u8]) -> Result<Document, MarkitdownError> {
        let content = String::from_utf8_lossy(bytes);
        let vcards = parse_vcards(&content)
            .map_err(|e| MarkitdownError::ParseError(format!("vCard parse error: {:?}", e)))?;

        let mut document = Document::new();
        let mut page = Page::new(1);
        let mut markdown = String::new();

        markdown.push_str("# Contacts\n\n");

        if vcards.is_empty() {
            markdown.push_str("*No contacts found.*\n");
        } else {
            for vcard in &vcards {
                markdown.push_str("## 👤 Contact\n\n");

                // Get formatted name (FN property)
                if let Some(fn_prop) = vcard.get_property_by_name("FN") {
                    markdown.push_str(&format!("**{}**\n\n", fn_prop.get_value()));
                }

                // Organization
                if let Some(org) = vcard.get_property_by_name("ORG") {
                    markdown.push_str(&format!("- **Organization:** {}\n", org.get_value()));
                }

                // Title
                if let Some(title) = vcard.get_property_by_name("TITLE") {
                    markdown.push_str(&format!("- **Title:** {}\n", title.get_value()));
                }

                // Email addresses
                for email in vcard.get_properties_by_name("EMAIL") {
                    markdown.push_str(&format!("- **Email:** {}\n", email.get_value()));
                }

                // Phone numbers
                for tel in vcard.get_properties_by_name("TEL") {
                    markdown.push_str(&format!("- **Phone:** {}\n", tel.get_value()));
                }

                // Address - Value needs to be converted to string first
                if let Some(adr) = vcard.get_property_by_name("ADR") {
                    let addr_str = format!("{}", adr.get_value());
                    let addr = addr_str
                        .replace(';', ", ")
                        .trim_matches(',')
                        .trim()
                        .to_string();
                    if !addr.is_empty() && addr != ", , , , , ," {
                        markdown.push_str(&format!("- **Address:** {}\n", addr));
                    }
                }

                // URL
                if let Some(url) = vcard.get_property_by_name("URL") {
                    markdown.push_str(&format!("- **Website:** {}\n", url.get_value()));
                }

                // Note
                if let Some(note) = vcard.get_property_by_name("NOTE") {
                    markdown.push_str(&format!("\n> {}\n", note.get_value()));
                }

                markdown.push('\n');
            }

            markdown.push_str(&format!("---\n*{} contact(s)*\n", vcards.len()));
        }

        page.add_content(ContentBlock::Markdown(markdown));
        document.add_page(page);
        Ok(document)
    }
}

#[async_trait]
impl DocumentConverter for VCardConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        if let Some(opts) = &options {
            if let Some(ext) = &opts.file_extension {
                if ext.to_lowercase() != ".vcf" && ext.to_lowercase() != ".vcard" {
                    return Err(MarkitdownError::InvalidFile(format!(
                        "Expected .vcf file, got {}",
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
                if ext.to_lowercase() != ".vcf" && ext.to_lowercase() != ".vcard" {
                    return Err(MarkitdownError::InvalidFile(format!(
                        "Expected .vcf file, got {}",
                        ext
                    )));
                }
            }
        }

        Self::convert_vcf(&bytes)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".vcf", ".vcard"]
    }
}
