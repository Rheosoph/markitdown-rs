use async_trait::async_trait;
use bytes::Bytes;
use object_store::{ObjectStore, ObjectStoreExt};
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

        // Embedded images (data URLs) become images where they stand; the HTML gets a marker in
        // their place, so their base64 never ends up in the text. Linked images stay links.
        let (content, images) = if extract_images {
            embedded_images(&content)
        } else {
            (content, Vec::new())
        };

        // Convert HTML to markdown, then put the images back at their markers
        let markdown = htmd::convert(&content).unwrap_or_default();
        let marker = Regex::new(r"MARKITDOWNIMAGE(\d+)END").expect("valid regex");
        let mut images: Vec<Option<ExtractedImage>> = images.into_iter().map(Some).collect();
        let mut last = 0;
        for found in marker.captures_iter(&markdown) {
            let whole = found.get(0).expect("whole match");
            let text = &markdown[last..whole.start()];
            if !text.trim().is_empty() {
                page.add_content(ContentBlock::Markdown(text.to_string()));
            }
            last = whole.end();
            let index: usize = found[1].parse().unwrap_or(usize::MAX);
            if let Some(image) = images.get_mut(index).and_then(Option::take) {
                page.add_content(ContentBlock::Image(image));
            }
        }
        let rest = &markdown[last..];
        if !rest.trim().is_empty() || page.content.is_empty() {
            page.add_content(ContentBlock::Markdown(rest.to_string()));
        }

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
            if let Some(llm_client) = opts.image_description_llm() {
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
            if let Some(llm_client) = opts.image_description_llm() {
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

/// Replace every `<img>` with a data URL by a marker (`MARKITDOWNIMAGE<n>END`) and return the
/// images in marker order. Images with other sources are left in the HTML.
fn embedded_images(html: &str) -> (String, Vec<ExtractedImage>) {
    let img_re =
        Regex::new(r#"<img[^>]*\s+src=["'](data:image/[^"']+)["'][^>]*>"#).expect("valid regex");
    let alt_re = Regex::new(r#"alt=["']([^"']*)["']"#).expect("valid regex");
    let mut images = Vec::new();
    let replaced = img_re.replace_all(html, |caps: &regex::Captures| {
        let Some(mut image) = extract_data_url_image(&caps[1], images.len() + 1) else {
            return caps[0].to_string();
        };
        image.alt_text = alt_re
            .captures(&caps[0])
            .map(|c| c[1].trim().to_string())
            .filter(|alt| !alt.is_empty());
        let marker = format!(" MARKITDOWNIMAGE{}END ", images.len());
        images.push(image);
        marker
    });
    (replaced.into_owned(), images)
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
