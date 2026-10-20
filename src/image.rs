use crate::error::MarkitdownError;
use crate::model::{
    ContentBlock, ConversionOptions, Document, DocumentConverter, ExtractedImage, Page,
};
use async_trait::async_trait;
use bytes::Bytes;
use exif::Reader;
use object_store::{ObjectStore, ObjectStoreExt};
use std::io::Cursor;
use std::sync::Arc;

pub struct ImageConverter;

impl ImageConverter {
    /// Detect MIME type from bytes
    fn detect_mime_type(bytes: &[u8]) -> String {
        if bytes.len() >= 8 {
            // PNG: 89 50 4E 47 0D 0A 1A 0A
            if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
                return "image/png".to_string();
            }
            // JPEG: FF D8 FF
            if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
                return "image/jpeg".to_string();
            }
            // GIF: GIF87a or GIF89a
            if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
                return "image/gif".to_string();
            }
            // WebP: RIFF....WEBP
            if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
                return "image/webp".to_string();
            }
            // BMP: BM
            if bytes.starts_with(b"BM") {
                return "image/bmp".to_string();
            }
            // TIFF: II or MM
            if bytes.starts_with(&[0x49, 0x49, 0x2A, 0x00])
                || bytes.starts_with(&[0x4D, 0x4D, 0x00, 0x2A])
            {
                return "image/tiff".to_string();
            }
        }
        "application/octet-stream".to_string()
    }

    /// Extract EXIF metadata from image bytes
    fn extract_exif_metadata(bytes: &[u8]) -> Option<String> {
        let exif = Reader::new()
            .read_from_container(&mut Cursor::new(bytes))
            .ok()?;

        let mut metadata = String::new();
        for field in exif.fields() {
            metadata.push_str(&format!(
                "{}: {}\n",
                field.tag,
                field.display_value().with_unit(&exif)
            ));
        }

        if metadata.is_empty() {
            None
        } else {
            Some(metadata)
        }
    }

    /// Convert bytes to Document
    async fn bytes_to_document(
        &self,
        bytes: Bytes,
        args: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let mime_type = Self::detect_mime_type(&bytes);
        let mut page = Page::new(1);

        // Extract EXIF metadata if available
        if let Some(exif_data) = Self::extract_exif_metadata(&bytes) {
            page.add_content(ContentBlock::Heading {
                level: 2,
                text: "EXIF Metadata".to_string(),
            });
            page.add_content(ContentBlock::Text(exif_data));
        }

        // Create image data
        let mut image = ExtractedImage::new("image_1", bytes.clone(), mime_type.clone());
        image.page_number = Some(1);

        if let Some(ref opts) = args {
            if let Some(path) = &opts.image_context_path {
                image.source_path = Some(path.clone());
            }
        }

        // If LLM client is provided, get description
        if let Some(ref opts) = args {
            if let Some(llm_client) = opts.image_description_llm() {
                if let Ok(mut descriptions) = llm_client.describe_extracted_images(&[&image]).await
                {
                    if let Some(description) = descriptions.pop() {
                        image.description = Some(description);
                    }
                }
            }
        }

        page.add_content(ContentBlock::Image(image));

        Ok(Document::from_page(page))
    }
}

#[async_trait]
impl DocumentConverter for ImageConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let result = store.get(path).await.map_err(|e| {
            MarkitdownError::ObjectStoreError(format!("Failed to get object: {}", e))
        })?;

        let bytes = result.bytes().await.map_err(|e| {
            MarkitdownError::ObjectStoreError(format!("Failed to read bytes: {}", e))
        })?;

        self.bytes_to_document(bytes, options).await
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        self.bytes_to_document(bytes, options).await
    }

    fn supported_extensions(&self) -> &[&str] {
        &["jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff", "tif"]
    }
}
