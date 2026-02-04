//! Jupyter Notebook to Markdown converter.
//!
//! Converts .ipynb files to markdown, preserving code cells and outputs.

use async_trait::async_trait;
use bytes::Bytes;
use object_store::ObjectStore;
use serde::Deserialize;
use std::sync::Arc;

use crate::error::MarkitdownError;
use crate::model::{ContentBlock, ConversionOptions, Document, DocumentConverter, Page};

/// Jupyter Notebook to Markdown converter
pub struct JupyterConverter;

#[derive(Deserialize)]
struct Notebook {
    cells: Vec<Cell>,
    metadata: Option<NotebookMetadata>,
}

#[derive(Deserialize)]
struct NotebookMetadata {
    kernelspec: Option<KernelSpec>,
}

#[derive(Deserialize)]
struct KernelSpec {
    language: Option<String>,
}

#[derive(Deserialize)]
struct Cell {
    cell_type: String,
    source: CellSource,
    outputs: Option<Vec<Output>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum CellSource {
    String(String),
    Array(Vec<String>),
}

impl CellSource {
    fn to_string(&self) -> String {
        match self {
            CellSource::String(s) => s.clone(),
            CellSource::Array(arr) => arr.join(""),
        }
    }
}

#[derive(Deserialize)]
struct Output {
    output_type: String,
    text: Option<CellSource>,
    data: Option<OutputData>,
}

#[derive(Deserialize)]
struct OutputData {
    #[serde(rename = "text/plain")]
    text_plain: Option<CellSource>,
    #[serde(rename = "text/html")]
    text_html: Option<CellSource>,
}

impl JupyterConverter {
    fn convert_notebook(bytes: &[u8]) -> Result<Document, MarkitdownError> {
        let notebook: Notebook = serde_json::from_slice(bytes)
            .map_err(|e| MarkitdownError::ParseError(format!("Invalid notebook: {}", e)))?;

        let mut document = Document::new();

        // Determine language from metadata
        let language = notebook
            .metadata
            .as_ref()
            .and_then(|m| m.kernelspec.as_ref())
            .and_then(|k| k.language.as_ref())
            .map(|s| s.as_str())
            .unwrap_or("python");

        let mut page = Page::new(1);
        let mut markdown = String::new();

        for (i, cell) in notebook.cells.iter().enumerate() {
            match cell.cell_type.as_str() {
                "markdown" => {
                    // Markdown cells are passed through
                    let source = cell.source.to_string();
                    markdown.push_str(&source);
                    markdown.push_str("\n\n");
                }
                "code" => {
                    // Code cells are wrapped in code blocks
                    let source = cell.source.to_string();

                    // Add cell number comment
                    markdown.push_str(&format!("**In [{}]:**\n\n", i + 1));
                    markdown.push_str(&format!("```{}\n", language));
                    markdown.push_str(&source);
                    if !source.ends_with('\n') {
                        markdown.push('\n');
                    }
                    markdown.push_str("```\n\n");

                    // Add outputs if present
                    if let Some(outputs) = &cell.outputs {
                        for output in outputs {
                            let output_text = Self::extract_output(output);
                            if !output_text.is_empty() {
                                markdown.push_str("**Out:**\n\n");
                                markdown.push_str("```\n");
                                markdown.push_str(&output_text);
                                if !output_text.ends_with('\n') {
                                    markdown.push('\n');
                                }
                                markdown.push_str("```\n\n");
                            }
                        }
                    }
                }
                "raw" => {
                    // Raw cells are wrapped in code blocks
                    let source = cell.source.to_string();
                    markdown.push_str("```\n");
                    markdown.push_str(&source);
                    if !source.ends_with('\n') {
                        markdown.push('\n');
                    }
                    markdown.push_str("```\n\n");
                }
                _ => {}
            }
        }

        page.add_content(ContentBlock::Markdown(markdown));
        document.add_page(page);

        Ok(document)
    }

    fn extract_output(output: &Output) -> String {
        match output.output_type.as_str() {
            "stream" | "execute_result" | "display_data" => {
                // Try text first
                if let Some(text) = &output.text {
                    return text.to_string();
                }
                // Try data
                if let Some(data) = &output.data {
                    if let Some(text) = &data.text_plain {
                        return text.to_string();
                    }
                    if let Some(html) = &data.text_html {
                        // Convert HTML to text (simple approach)
                        return htmd::convert(&html.to_string()).unwrap_or_default();
                    }
                }
            }
            "error" => {
                return "[Error output]".to_string();
            }
            _ => {}
        }
        String::new()
    }
}

#[async_trait]
impl DocumentConverter for JupyterConverter {
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
        Self::convert_notebook(&bytes)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".ipynb"]
    }
}
