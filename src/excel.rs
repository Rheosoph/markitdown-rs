use async_trait::async_trait;
use bytes::Bytes;
use calamine::{Reader, Xlsx};
use object_store::ObjectStore;
use std::io::Cursor;
use std::sync::Arc;

use crate::error::MarkitdownError;
use crate::model::{ContentBlock, ConversionOptions, Document, DocumentConverter, Page};

pub struct ExcelConverter;

impl ExcelConverter {
    fn convert_excel_bytes(&self, bytes: &[u8]) -> Result<Document, MarkitdownError> {
        let reader = Cursor::new(bytes);
        let mut workbook: Xlsx<_> = Xlsx::new(reader).map_err(|e| {
            MarkitdownError::ParseError(format!("Failed to open Excel file: {}", e))
        })?;

        let mut document = Document::new();
        let sheet_names: Vec<String> = workbook.sheet_names().to_vec();

        for (sheet_idx, sheet_name) in sheet_names.iter().enumerate() {
            let mut page = Page::new((sheet_idx + 1) as u32);

            // Add sheet name as heading
            page.add_content(ContentBlock::Heading {
                level: 2,
                text: sheet_name.clone(),
            });

            if let Ok(range) = workbook.worksheet_range(sheet_name) {
                let rows: Vec<Vec<String>> = range
                    .rows()
                    .map(|row| row.iter().map(|cell| cell.to_string()).collect())
                    .collect();

                if !rows.is_empty() {
                    let headers = rows[0].clone();
                    let data_rows: Vec<Vec<String>> = rows.into_iter().skip(1).collect();

                    page.add_content(ContentBlock::Table {
                        headers,
                        rows: data_rows,
                    });
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
impl DocumentConverter for ExcelConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        if let Some(opts) = &options {
            if let Some(ext) = &opts.file_extension {
                if ext.to_lowercase() != ".xlsx" && ext.to_lowercase() != ".xls" {
                    return Err(MarkitdownError::InvalidFile(format!(
                        "Expected .xlsx or .xls file, got {}",
                        ext
                    )));
                }
            }
        }

        let result = store.get(path).await?;
        let bytes = result.bytes().await?;
        self.convert_excel_bytes(&bytes)
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        if let Some(opts) = &options {
            if let Some(ext) = &opts.file_extension {
                if ext.to_lowercase() != ".xlsx" && ext.to_lowercase() != ".xls" {
                    return Err(MarkitdownError::InvalidFile(format!(
                        "Expected .xlsx or .xls file, got {}",
                        ext
                    )));
                }
            }
        }

        self.convert_excel_bytes(&bytes)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".xlsx", ".xls"]
    }
}
