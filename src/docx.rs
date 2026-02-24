use async_trait::async_trait;
use bytes::Bytes;
use docx_rust::{
    document::{BodyContent, TableCellContent, TableRowContent},
    DocxFile,
};
use object_store::ObjectStore;
use std::io::{Cursor, Read};
use std::sync::Arc;
use zip::ZipArchive;

use crate::error::MarkitdownError;
use crate::model::{
    ContentBlock, ConversionOptions, Document, DocumentConverter, ExtractedImage, Page,
};

pub struct DocxConverter;

impl DocxConverter {
    fn convert_docx_bytes(
        &self,
        bytes: &[u8],
        extract_images: bool,
    ) -> Result<Document, MarkitdownError> {
        let reader = Cursor::new(bytes);

        let docx_file = DocxFile::from_reader(reader)
            .map_err(|e| MarkitdownError::ParseError(format!("Failed to read DOCX file: {}", e)))?;
        let doc = docx_file.parse().map_err(|e| {
            MarkitdownError::ParseError(format!("Failed to parse DOCX file: {}", e))
        })?;

        let mut document = Document::new();
        let mut page = Page::new(1);

        // Extract images from the DOCX file if requested
        if extract_images {
            let images = self.extract_images_from_docx(bytes)?;
            for img in images {
                page.add_content(ContentBlock::Image(img));
            }
        }

        // Process document content
        for content in doc.document.body.content {
            match content {
                BodyContent::Paragraph(paragraph) => {
                    let mut text = String::new();
                    for t in paragraph.iter_text() {
                        text.push_str(t.as_ref());
                    }
                    if !text.trim().is_empty() {
                        page.add_content(ContentBlock::Text(text));
                    }
                }
                BodyContent::Table(table) => {
                    if !table.rows.is_empty() {
                        let mut headers: Vec<String> = Vec::new();
                        let mut rows: Vec<Vec<String>> = Vec::new();

                        // First row as headers
                        for cell in table.rows[0].cells.iter() {
                            if let TableRowContent::TableCell(tc) = cell {
                                let mut cell_text = String::new();
                                for content in &tc.content {
                                    let TableCellContent::Paragraph(paragraph) = content;
                                    for text in paragraph.iter_text() {
                                        cell_text.push_str(text.as_ref());
                                    }
                                }
                                headers.push(cell_text);
                            }
                        }

                        // Remaining rows as data
                        for row in table.rows.iter().skip(1) {
                            let mut row_data: Vec<String> = Vec::new();
                            for cell in row.cells.iter() {
                                if let TableRowContent::TableCell(tc) = cell {
                                    let mut cell_text = String::new();
                                    for content in &tc.content {
                                        let TableCellContent::Paragraph(paragraph) = content;
                                        for text in paragraph.iter_text() {
                                            cell_text.push_str(text.as_ref());
                                        }
                                    }
                                    row_data.push(cell_text);
                                }
                            }
                            rows.push(row_data);
                        }

                        page.add_content(ContentBlock::Table { headers, rows });
                    }
                }
                _ => {}
            }
        }

        document.add_page(page);
        Ok(document)
    }

    fn extract_images_from_docx(
        &self,
        bytes: &[u8],
    ) -> Result<Vec<ExtractedImage>, MarkitdownError> {
        let mut images = Vec::new();
        let cursor = Cursor::new(bytes);
        let mut archive = ZipArchive::new(cursor)?;

        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            let name = file.name().to_string();

            // Check if this is an image file in the media folder
            if name.starts_with("word/media/") {
                let mime_type = if name.ends_with(".png") {
                    "image/png"
                } else if name.ends_with(".jpg") || name.ends_with(".jpeg") {
                    "image/jpeg"
                } else if name.ends_with(".gif") {
                    "image/gif"
                } else if name.ends_with(".webp") {
                    "image/webp"
                } else if name.ends_with(".emf") || name.ends_with(".wmf") {
                    continue; // Skip Windows metafiles
                } else {
                    continue;
                };

                let mut data = Vec::new();
                file.read_to_end(&mut data)?;

                let image_name = name
                    .strip_prefix("word/media/")
                    .unwrap_or(&name)
                    .to_string();

                images.push(ExtractedImage::new(
                    image_name,
                    Bytes::from(data),
                    mime_type,
                ));
            }
        }

        Ok(images)
    }
}

#[async_trait]
impl DocumentConverter for DocxConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        if let Some(opts) = &options {
            if let Some(ext) = &opts.file_extension {
                if ext.to_lowercase() != ".docx" {
                    return Err(MarkitdownError::InvalidFile(format!(
                        "Expected .docx file, got {}",
                        ext
                    )));
                }
            }
        }

        let extract_images = options.as_ref().map(|o| o.extract_images).unwrap_or(true);

        let result = store.get(path).await?;
        let bytes = result.bytes().await?;
        let mut document = self.convert_docx_bytes(&bytes, extract_images)?;

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
                if ext.to_lowercase() != ".docx" {
                    return Err(MarkitdownError::InvalidFile(format!(
                        "Expected .docx file, got {}",
                        ext
                    )));
                }
            }
        }

        let extract_images = options.as_ref().map(|o| o.extract_images).unwrap_or(true);
        let mut document = self.convert_docx_bytes(&bytes, extract_images)?;

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
        &[".docx"]
    }
}
