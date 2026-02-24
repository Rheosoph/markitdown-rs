use async_trait::async_trait;
use bytes::Bytes;
use object_store::ObjectStore;
use regex::Regex;
use std::sync::Arc;

use crate::error::MarkitdownError;
use crate::model::{
    ContentBlock, ConversionOptions, Document, DocumentConverter, ExtractedImage, Page,
};

pub struct HtmlConverter;

impl HtmlConverter {
    fn convert_html_bytes(
        &self,
        bytes: &[u8],
        extract_images: bool,
    ) -> Result<Document, MarkitdownError> {
        let content = String::from_utf8(bytes.to_vec())
            .map_err(|e| MarkitdownError::ParseError(format!("Invalid UTF-8 encoding: {}", e)))?;

        let title = extract_title(&content);
        let mut document = Document::new();
        document.title = title;

        let mut page = Page::new(1);

        // Extract images if requested
        if extract_images {
            let images = extract_images_from_html(&content);
            for img in images {
                page.add_content(ContentBlock::Image(img));
            }
        }

        // Convert HTML to markdown
        let markdown = htmd::convert(&content).unwrap_or_default();
        page.add_content(ContentBlock::Markdown(markdown));

        document.add_page(page);
        Ok(document)
    }
}

#[async_trait]
impl DocumentConverter for HtmlConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        if let Some(opts) = &options {
            if let Some(ext) = &opts.file_extension {
                if ext.to_lowercase() != ".html" && ext.to_lowercase() != ".htm" {
                    return Err(MarkitdownError::InvalidFile(format!(
                        "Expected .html or .htm file, got {}",
                        ext
                    )));
                }
            }
        }

        let extract_images = options.as_ref().map(|o| o.extract_images).unwrap_or(true);

        let result = store.get(path).await?;
        let bytes = result.bytes().await?;
        let mut document = self.convert_html_bytes(&bytes, extract_images)?;

        // If LLM client is provided, get descriptions for all images
        if let Some(ref opts) = options {
            if let Some(ref llm_client) = opts.llm_client {
                if let Some(path) = opts.image_context_path.as_deref() {
                    document.apply_image_context_path(path);
                }
                document = document
                    .with_image_descriptions(llm_client.as_ref())
                    .await?;
            }
        }

        Ok(document)
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        if let Some(opts) = &options {
            if let Some(ext) = &opts.file_extension {
                if ext.to_lowercase() != ".html" && ext.to_lowercase() != ".htm" {
                    return Err(MarkitdownError::InvalidFile(format!(
                        "Expected .html or .htm file, got {}",
                        ext
                    )));
                }
            }
        }

        let extract_images = options.as_ref().map(|o| o.extract_images).unwrap_or(true);
        let mut document = self.convert_html_bytes(&bytes, extract_images)?;

        // If LLM client is provided, get descriptions for all images
        if let Some(ref opts) = options {
            if let Some(ref llm_client) = opts.llm_client {
                if let Some(path) = opts.image_context_path.as_deref() {
                    document.apply_image_context_path(path);
                }
                document = document
                    .with_image_descriptions(llm_client.as_ref())
                    .await?;
            }
        }

        Ok(document)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".html", ".htm"]
    }
}

fn extract_title(html: &str) -> Option<String> {
    let re = Regex::new(r"(?i)<title(?:\s[^>]*)?>(.*?)</title>").ok()?;
    if let Some(captures) = re.captures(html) {
        return Some(captures[1].trim().to_string());
    }
    None
}

fn extract_images_from_html(html: &str) -> Vec<ExtractedImage> {
    let mut images = Vec::new();

    // Match img tags and extract src and alt attributes
    let img_re = Regex::new(r#"<img[^>]*\s+src=["']([^"']+)["'][^>]*>"#).ok();
    let alt_re = Regex::new(r#"alt=["']([^"']*)["']"#).ok();

    if let Some(re) = img_re {
        for (idx, cap) in re.captures_iter(html).enumerate() {
            let src = cap.get(1).map(|m| m.as_str().to_string());
            let alt = alt_re.as_ref().and_then(|r| {
                r.captures(&cap[0])
                    .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
            });

            if let Some(src_url) = src {
                // For data URLs, extract the actual image data
                if src_url.starts_with("data:image/") {
                    if let Some(extracted) = extract_data_url_image(&src_url, idx) {
                        let mut img = extracted;
                        img.alt_text = alt;
                        images.push(img);
                    }
                } else {
                    // For external URLs, create a placeholder
                    let mut img = ExtractedImage::new(
                        format!("html_image_{}", idx),
                        Bytes::new(),
                        "image/unknown",
                    );
                    img.alt_text = alt.or(Some(src_url));
                    images.push(img);
                }
            }
        }
    }

    images
}

fn extract_data_url_image(data_url: &str, idx: usize) -> Option<ExtractedImage> {
    // Parse data URL format: data:image/type;base64,data
    let parts: Vec<&str> = data_url.splitn(2, ',').collect();
    if parts.len() != 2 {
        return None;
    }

    let header = parts[0];
    let data = parts[1];

    // Extract MIME type
    let mime_type = header.strip_prefix("data:")?.split(';').next()?.to_string();

    // Decode base64 data
    use base64::prelude::*;
    let bytes = BASE64_STANDARD.decode(data).ok()?;

    Some(ExtractedImage::new(
        format!("html_image_{}", idx),
        Bytes::from(bytes),
        mime_type,
    ))
}
