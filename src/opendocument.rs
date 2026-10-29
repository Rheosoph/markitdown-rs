//! OpenDocument format converters (.odt, .ods, .odp).
//!
//! OpenDocument formats are ZIP-based XML formats used by LibreOffice, OpenOffice, etc.

use async_trait::async_trait;
use bytes::Bytes;
use object_store::{ObjectStore, ObjectStoreExt};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::io::{Cursor, Read};
use std::sync::Arc;

use crate::error::MarkitdownError;
use crate::model::{ContentBlock, ConversionOptions, Document, DocumentConverter, Page};

/// OpenDocument Text (.odt) converter
pub struct OdtConverter;

impl OdtConverter {
    fn convert_odt(bytes: &[u8]) -> Result<Document, MarkitdownError> {
        let cursor = Cursor::new(bytes);
        let mut archive = crate::open_zip(cursor)
            .map_err(|e| MarkitdownError::ParseError(format!("ODT parse error: {}", e)))?;

        let mut document = Document::new();
        let mut page = Page::new(1);
        let mut markdown = String::new();

        markdown.push_str("# OpenDocument Text\n\n");

        // Read content.xml
        if let Ok(mut content_file) = archive.by_name("content.xml") {
            let mut content = String::new();
            content_file
                .read_to_string(&mut content)
                .map_err(|e| MarkitdownError::ParseError(format!("Read error: {}", e)))?;

            let text = Self::extract_text_from_xml(&content)?;
            markdown.push_str(&text);
        } else {
            markdown.push_str("*Unable to read content from ODT file.*\n");
        }

        page.add_content(ContentBlock::Markdown(markdown));
        document.add_page(page);
        Ok(document)
    }

    fn extract_text_from_xml(xml: &str) -> Result<String, MarkitdownError> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut text = String::new();
        let mut in_text = false;
        let mut in_heading = false;
        let mut heading_level = 1;
        let mut current_text = String::new();

        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    match name.as_str() {
                        "text:p" => {
                            in_text = true;
                            current_text.clear();
                        }
                        "text:h" => {
                            in_heading = true;
                            current_text.clear();
                            // Try to get outline level
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"text:outline-level" {
                                    if let Ok(level) =
                                        String::from_utf8_lossy(&attr.value).parse::<u8>()
                                    {
                                        heading_level = level.min(6);
                                    }
                                }
                            }
                        }
                        "text:list-item" => {
                            current_text.push_str("- ");
                        }
                        _ => {}
                    }
                }
                Ok(Event::End(e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    match name.as_str() {
                        "text:p" => {
                            if in_text {
                                text.push_str(&current_text);
                                text.push_str("\n\n");
                                in_text = false;
                            }
                        }
                        "text:h" => {
                            if in_heading {
                                text.push_str(&"#".repeat(heading_level as usize));
                                text.push(' ');
                                text.push_str(&current_text);
                                text.push_str("\n\n");
                                in_heading = false;
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Event::Text(e)) => {
                    // Use into_inner to get the escaped bytes, then decode
                    let escaped_text = String::from_utf8_lossy(e.as_ref()).to_string();
                    current_text.push_str(&escaped_text);
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(MarkitdownError::ParseError(format!("XML error: {}", e))),
                _ => {}
            }
        }

        Ok(text.trim().to_string())
    }
}

#[async_trait]
impl DocumentConverter for OdtConverter {
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
        Self::convert_odt(&bytes)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".odt", ".ott"] // .ott is template
    }
}

/// OpenDocument Spreadsheet (.ods) converter
pub struct OdsConverter;

impl OdsConverter {
    fn convert_ods(bytes: &[u8]) -> Result<Document, MarkitdownError> {
        // Use calamine which already supports .ods
        use calamine::{open_workbook_auto_from_rs, Reader};

        crate::open_zip(Cursor::new(bytes))?;
        let cursor = Cursor::new(bytes.to_vec());
        let mut workbook = open_workbook_auto_from_rs(cursor)
            .map_err(|e| MarkitdownError::ParseError(format!("ODS parse error: {}", e)))?;

        let mut document = Document::new();

        let sheet_names = workbook.sheet_names().to_vec();

        for (sheet_idx, sheet_name) in sheet_names.iter().enumerate() {
            let mut page = Page::new((sheet_idx + 1) as u32);

            // Add sheet name as heading
            page.add_content(ContentBlock::Heading {
                level: 2,
                text: format!("Sheet: {}", sheet_name),
            });

            if let Ok(range) = workbook.worksheet_range(sheet_name) {
                let rows: Vec<_> = range.rows().collect();
                if rows.is_empty() {
                    page.add_content(ContentBlock::Text("*Empty sheet*".to_string()));
                    document.add_page(page);
                    continue;
                }

                // Create table
                let num_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
                if num_cols == 0 {
                    document.add_page(page);
                    continue;
                }

                // Header row
                let headers = if let Some(first_row) = rows.first() {
                    first_row
                        .iter()
                        .map(|c| format!("{}", c).replace('|', "\\|"))
                        .collect()
                } else {
                    vec![]
                };

                // Data rows (skip header)
                let data_rows: Vec<Vec<String>> = rows
                    .iter()
                    .skip(1)
                    .take(100)
                    .map(|row| {
                        row.iter()
                            .map(|c| format!("{}", c).replace('|', "\\|"))
                            .collect()
                    })
                    .collect();

                page.add_content(ContentBlock::Table {
                    headers,
                    rows: data_rows,
                });

                if rows.len() > 101 {
                    page.add_content(ContentBlock::Text(format!(
                        "*... and {} more rows*",
                        rows.len() - 101
                    )));
                }
            }

            document.add_page(page);
        }

        // If no sheets found, create empty document
        if document.pages.is_empty() {
            document.add_page(Page::new(1));
        }

        Ok(document)
    }
}

#[async_trait]
impl DocumentConverter for OdsConverter {
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
        Self::convert_ods(&bytes)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".ods", ".ots"] // .ots is template
    }
}

/// OpenDocument Presentation (.odp) converter
pub struct OdpConverter;

impl OdpConverter {
    fn convert_odp(bytes: &[u8]) -> Result<Document, MarkitdownError> {
        let cursor = Cursor::new(bytes);
        let mut archive = crate::open_zip(cursor)
            .map_err(|e| MarkitdownError::ParseError(format!("ODP parse error: {}", e)))?;

        let mut document = Document::new();

        // Read content.xml
        if let Ok(mut content_file) = archive.by_name("content.xml") {
            let mut content = String::new();
            content_file
                .read_to_string(&mut content)
                .map_err(|e| MarkitdownError::ParseError(format!("Read error: {}", e)))?;

            let slides = Self::extract_slides_from_xml(&content)?;

            for (idx, slide_text) in slides.iter().enumerate() {
                let mut page = Page::new((idx + 1) as u32);
                page.add_content(ContentBlock::Heading {
                    level: 2,
                    text: format!("Slide {}", idx + 1),
                });
                page.add_content(ContentBlock::Text(slide_text.clone()));
                document.add_page(page);
            }
        }

        // If no slides found, create empty document
        if document.pages.is_empty() {
            let mut page = Page::new(1);
            page.add_content(ContentBlock::Text(
                "*Unable to read content from ODP file.*".to_string(),
            ));
            document.add_page(page);
        }

        Ok(document)
    }

    fn extract_slides_from_xml(xml: &str) -> Result<Vec<String>, MarkitdownError> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut slides = Vec::new();
        let mut current_slide = String::new();
        let mut in_page = false;
        let mut current_text = String::new();

        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    match name.as_str() {
                        "draw:page" => {
                            in_page = true;
                            current_slide.clear();
                        }
                        "text:p" | "text:span" => {
                            current_text.clear();
                        }
                        _ => {}
                    }
                }
                Ok(Event::End(e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    match name.as_str() {
                        "draw:page" => {
                            if in_page && !current_slide.is_empty() {
                                slides.push(current_slide.clone());
                            }
                            in_page = false;
                        }
                        "text:p" | "text:span" => {
                            if in_page && !current_text.is_empty() {
                                current_slide.push_str(&current_text);
                                current_slide.push('\n');
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Event::Text(e)) => {
                    if in_page {
                        let escaped_text = String::from_utf8_lossy(e.as_ref()).to_string();
                        current_text.push_str(&escaped_text);
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(MarkitdownError::ParseError(format!("XML error: {}", e))),
                _ => {}
            }
        }

        Ok(slides)
    }
}

#[async_trait]
impl DocumentConverter for OdpConverter {
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
        Self::convert_odp(&bytes)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".odp", ".otp"] // .otp is template
    }
}
