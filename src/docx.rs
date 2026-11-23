use async_trait::async_trait;
use bytes::Bytes;
use docx_rust::{
    document::{
        BodyContent, Paragraph, ParagraphContent, Run, RunContent, TableCellContent,
        TableRowContent,
    },
    rels::TargetMode,
    DocxFile,
};
use object_store::{ObjectStore, ObjectStoreExt};
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::Arc;

use crate::error::MarkitdownError;
use crate::model::{
    ContentBlock, ConversionOptions, Document, DocumentConverter, ExtractedImage, Page,
};

pub struct DocxConverter;

/// Pictures in a paragraph, in order: relationship id of the picture and its alt text
fn paragraph_pictures(paragraph: &Paragraph) -> Vec<(String, Option<String>)> {
    let runs = paragraph
        .content
        .iter()
        .filter_map(|content| match content {
            ParagraphContent::Run(run) => Some(run),
            ParagraphContent::Link(link) => link.content.as_ref(),
            _ => None,
        });
    let mut pictures = Vec::new();
    for run in runs {
        pictures.extend(run_pictures(run));
    }
    pictures
}

fn run_pictures(run: &Run) -> Vec<(String, Option<String>)> {
    let mut pictures = Vec::new();
    for content in &run.content {
        let RunContent::Drawing(drawing) = content else {
            continue;
        };
        let (doc_property, graphic) = match (&drawing.inline, &drawing.anchor) {
            (Some(inline), _) => (&inline.doc_property, inline.graphic.as_ref()),
            (None, Some(anchor)) => (&anchor.doc_property, anchor.graphic.as_ref()),
            (None, None) => continue,
        };
        let Some(graphic) = graphic else {
            continue;
        };
        for picture in &graphic.data.children {
            let alt = doc_property
                .descr
                .as_deref()
                .or_else(|| {
                    picture
                        .nv_pic_pr
                        .c_nv_pr
                        .as_ref()
                        .and_then(|c| c.descr.as_deref())
                })
                .map(str::trim)
                .filter(|alt| !alt.is_empty())
                .map(str::to_string);
            let embed = picture.fill.blip.embed.trim();
            if !embed.is_empty() {
                pictures.push((embed.to_string(), alt));
            }
        }
    }
    pictures
}

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

        // Pictures stand where the document shows them, found through the relationships of the
        // main part (not every file in word/media: some are unused, some belong to headers)
        let relationships: HashMap<String, String> = doc
            .document_rels
            .as_ref()
            .map(|rels| {
                rels.relationships
                    .iter()
                    .filter(|rel| !matches!(rel.target_mode, Some(TargetMode::External)))
                    .map(|rel| {
                        (
                            rel.id.to_string(),
                            crate::resolve_part_target("word", &rel.target),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut archive = if extract_images {
            Some(crate::open_zip(Cursor::new(bytes))?)
        } else {
            None
        };
        let mut picture_count = 0;

        // Process document content
        for content in &doc.document.body.content {
            match content {
                BodyContent::Paragraph(paragraph) => {
                    let mut text = String::new();
                    for t in paragraph.iter_text() {
                        text.push_str(t.as_ref());
                    }
                    if !text.trim().is_empty() {
                        page.add_content(ContentBlock::Text(text));
                    }
                    let Some(archive) = archive.as_mut() else {
                        continue;
                    };
                    for (rel_id, alt) in paragraph_pictures(paragraph) {
                        let Some(path) = relationships.get(&rel_id) else {
                            continue;
                        };
                        let mut data = Vec::new();
                        let Ok(mut file) = archive.by_name(path) else {
                            continue;
                        };
                        if file.read_to_end(&mut data).is_err() || data.is_empty() {
                            continue;
                        }
                        picture_count += 1;
                        let mime_type = mime_guess::from_path(path)
                            .first_or_octet_stream()
                            .to_string();
                        let mut image = ExtractedImage::new(
                            format!("image{}", picture_count),
                            Bytes::from(data),
                            mime_type,
                        );
                        image.alt_text = alt;
                        image.page_number = Some(1);
                        page.add_content(ContentBlock::Image(image));
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
        &[".docx"]
    }
}
