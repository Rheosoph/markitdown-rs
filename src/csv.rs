use async_trait::async_trait;
use bytes::Bytes;
use csv::ReaderBuilder;
use object_store::ObjectStore;
use std::sync::Arc;

use crate::error::MarkitdownError;
use crate::model::{ContentBlock, ConversionOptions, Document, DocumentConverter, Page};

pub struct CsvConverter;

impl CsvConverter {
    fn convert_csv_bytes(&self, bytes: &[u8]) -> Result<Document, MarkitdownError> {
        let mut rdr = ReaderBuilder::new().has_headers(true).from_reader(bytes);

        let headers: Vec<String> = rdr
            .headers()
            .map_err(|e| MarkitdownError::ParseError(format!("Failed to read CSV headers: {}", e)))?
            .iter()
            .map(|s| s.to_string())
            .collect();

        let mut rows: Vec<Vec<String>> = Vec::new();
        for result in rdr.records() {
            match result {
                Ok(record) => {
                    let row: Vec<String> = record.iter().map(|s| s.to_string()).collect();
                    rows.push(row);
                }
                Err(err) => {
                    return Err(MarkitdownError::ParseError(format!(
                        "Failed to parse CSV row: {}",
                        err
                    )));
                }
            }
        }

        let mut page = Page::new(1);

        if !headers.is_empty() || !rows.is_empty() {
            page.add_content(ContentBlock::Table { headers, rows });
        }

        Ok(Document::from_page(page))
    }
}

#[async_trait]
impl DocumentConverter for CsvConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        if let Some(opts) = &options {
            if let Some(ext) = &opts.file_extension {
                if ext.to_lowercase() != ".csv" {
                    return Err(MarkitdownError::InvalidFile(format!(
                        "Expected .csv file, got {}",
                        ext
                    )));
                }
            }
        }

        let result = store.get(path).await?;
        let bytes = result.bytes().await?;
        self.convert_csv_bytes(&bytes)
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        if let Some(opts) = &options {
            if let Some(ext) = &opts.file_extension {
                if ext.to_lowercase() != ".csv" {
                    return Err(MarkitdownError::InvalidFile(format!(
                        "Expected .csv file, got {}",
                        ext
                    )));
                }
            }
        }

        self.convert_csv_bytes(&bytes)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".csv"]
    }
}
