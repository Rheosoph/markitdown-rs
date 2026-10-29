use crate::error::MarkitdownError;
use crate::model::{
    ContentBlock, ConversionOptions, Document, DocumentConverter, ExtractedImage, Page,
};
use async_trait::async_trait;
use bytes::Bytes;
use object_store::{ObjectStore, ObjectStoreExt};
use quick_xml::{events::Event, reader::Reader};
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::Arc;
use zip::ZipArchive;

pub struct PptxConverter;

/// Represents an image reference found in a slide
#[derive(Debug, Clone)]
struct SlideImageRef {
    /// Relationship ID (e.g., "rId2")
    rel_id: String,
    /// Optional description text from the slide
    description: Option<String>,
}

impl PptxConverter {
    /// Detect MIME type from bytes
    fn detect_mime_type(bytes: &[u8]) -> String {
        if bytes.len() >= 8 {
            if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
                return "image/png".to_string();
            }
            if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
                return "image/jpeg".to_string();
            }
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
            // EMF (Enhanced Metafile)
            if bytes.starts_with(&[0x01, 0x00, 0x00, 0x00]) {
                return "image/emf".to_string();
            }
            // WMF (Windows Metafile)
            if bytes.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) {
                return "image/wmf".to_string();
            }
        }
        "application/octet-stream".to_string()
    }

    /// Extract images from ppt/media folder
    fn extract_images(archive: &mut ZipArchive<Cursor<&[u8]>>) -> HashMap<String, Vec<u8>> {
        let mut images = HashMap::new();

        for i in 0..archive.len() {
            if let Ok(mut file) = archive.by_index(i) {
                let name = file.name().unwrap_or_default().into_owned();
                if name.starts_with("ppt/media/") {
                    let mut data = Vec::new();
                    if file.read_to_end(&mut data).is_ok() {
                        // Extract just the filename
                        let filename = name.rsplit('/').next().unwrap_or(&name).to_string();
                        images.insert(filename, data);
                    }
                }
            }
        }

        images
    }

    /// Parse a slide's relationship file to map rIds to media filenames
    fn parse_slide_relationships(
        archive: &mut ZipArchive<Cursor<&[u8]>>,
        slide_num: u32,
    ) -> HashMap<String, String> {
        let mut relationships = HashMap::new();
        let rels_path = format!("ppt/slides/_rels/slide{}.xml.rels", slide_num);

        if let Ok(mut file) = archive.by_name(&rels_path) {
            let mut content = String::new();
            if file.read_to_string(&mut content).is_ok() {
                let mut reader = Reader::from_str(&content);
                reader.config_mut().trim_text(true);
                let mut buf = Vec::new();

                loop {
                    buf.clear();
                    match reader.read_event_into(&mut buf) {
                        Ok(Event::Empty(element)) | Ok(Event::Start(element)) => {
                            if element.name().as_ref() == b"Relationship" {
                                let mut id = None;
                                let mut target = None;

                                for attr in element.attributes().flatten() {
                                    match attr.key.as_ref() {
                                        b"Id" => {
                                            id = String::from_utf8(attr.value.to_vec()).ok();
                                        }
                                        b"Target" => {
                                            target = String::from_utf8(attr.value.to_vec()).ok();
                                        }
                                        _ => {}
                                    }
                                }

                                if let (Some(id), Some(target)) = (id, target) {
                                    // Target is like "../media/image1.png"
                                    if target.contains("/media/") {
                                        let filename = target
                                            .rsplit('/')
                                            .next()
                                            .unwrap_or(&target)
                                            .to_string();
                                        relationships.insert(id, filename);
                                    }
                                }
                            }
                        }
                        Ok(Event::Eof) => break,
                        Err(_) => break,
                        _ => {}
                    }
                }
            }
        }

        relationships
    }

    /// Extract image references from slide XML
    fn extract_image_refs(content: &str) -> Vec<SlideImageRef> {
        let mut image_refs = Vec::new();
        let mut reader = Reader::from_str(content);
        reader.config_mut().trim_text(true);
        let mut buf = Vec::new();

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(element)) | Ok(Event::Empty(element)) => {
                    if element.name().as_ref() == b"a:blip" {
                        for attr in element.attributes().flatten() {
                            if attr.key.as_ref() == b"r:embed" {
                                if let Ok(rel_id) = String::from_utf8(attr.value.to_vec()) {
                                    image_refs.push(SlideImageRef {
                                        rel_id,
                                        description: None,
                                    });
                                }
                            }
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Err(_) => break,
                _ => {}
            }
        }

        image_refs
    }

    /// Extract text content from slide XML
    fn extract_text_content(content: &str) -> Result<(String, Vec<ContentBlock>), MarkitdownError> {
        let mut markdown = String::new();
        let mut blocks: Vec<ContentBlock> = Vec::new();
        let mut reader = Reader::from_str(content);
        reader.config_mut().trim_text(true);

        let mut buf = Vec::new();
        let mut count: u8 = 0;

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf).map_err(|e| {
                MarkitdownError::ParseError(format!("Failed to read XML event: {}", e))
            })? {
                Event::Start(element) => {
                    Self::process_element(
                        &element,
                        &mut reader,
                        &mut markdown,
                        &mut blocks,
                        &mut count,
                    )?;
                }
                Event::Eof => break,
                _ => {}
            }
        }

        Ok((markdown, blocks))
    }

    /// Process a start element and extract content
    fn process_element(
        element: &quick_xml::events::BytesStart,
        reader: &mut Reader<&[u8]>,
        markdown: &mut String,
        blocks: &mut Vec<ContentBlock>,
        count: &mut u8,
    ) -> Result<(), MarkitdownError> {
        match element.name().as_ref() {
            b"p:txBody" => {
                Self::process_text_body(reader, markdown, blocks)?;
            }
            b"a:tbl" => {
                Self::process_table(reader, markdown, blocks, count)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Process a text body element
    fn process_text_body(
        reader: &mut Reader<&[u8]>,
        markdown: &mut String,
        blocks: &mut Vec<ContentBlock>,
    ) -> Result<(), MarkitdownError> {
        let mut text_content = String::new();
        let mut buf = Vec::new();

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf).map_err(|e| {
                MarkitdownError::ParseError(format!("Failed to read XML event: {}", e))
            })? {
                Event::Start(el) if el.name().as_ref() == b"a:t" => {
                    Self::extract_text_element(reader, &mut text_content)?;
                }
                Event::End(el) if el.name().as_ref() == b"p:txBody" => {
                    if !text_content.is_empty() {
                        markdown.push_str(&text_content);
                        markdown.push_str("\n\n");
                        blocks.push(ContentBlock::Text(text_content));
                    }
                    break;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Extract text from an a:t element
    fn extract_text_element(
        reader: &mut Reader<&[u8]>,
        text_content: &mut String,
    ) -> Result<(), MarkitdownError> {
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_event_into(&mut buf).map_err(|e| {
                MarkitdownError::ParseError(format!("Failed to read XML event: {}", e))
            })? {
                Event::Text(text) => {
                    let decoded = text.decode().map_err(|e| {
                        MarkitdownError::ParseError(format!("Failed to decode text: {}", e))
                    })?;
                    text_content.push_str(&decoded);
                }
                Event::End(el) if el.name().as_ref() == b"a:t" => break,
                _ => {}
            }
        }
        Ok(())
    }

    /// Process a table element
    fn process_table(
        reader: &mut Reader<&[u8]>,
        markdown: &mut String,
        blocks: &mut Vec<ContentBlock>,
        count: &mut u8,
    ) -> Result<(), MarkitdownError> {
        *count += 1;
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut buf = Vec::new();

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf).map_err(|e| {
                MarkitdownError::ParseError(format!("Failed to read XML event: {}", e))
            })? {
                Event::Start(el) => match el.name().as_ref() {
                    b"a:tr" => rows.push(Vec::new()),
                    b"a:tc" => {
                        if let Some(row) = rows.last_mut() {
                            Self::extract_table_cell(reader, row)?;
                        }
                    }
                    _ => {}
                },
                Event::End(el) if el.name().as_ref() == b"a:tbl" => break,
                _ => {}
            }
        }

        if !rows.is_empty() {
            let headers = rows[0].clone();
            let data_rows: Vec<Vec<String>> = rows.into_iter().skip(1).collect();
            blocks.push(ContentBlock::Table {
                headers: headers.clone(),
                rows: data_rows.clone(),
            });

            // Add to markdown
            Self::append_table_markdown(markdown, &headers, &data_rows);
        }

        Ok(())
    }

    /// Extract text from a table cell
    fn extract_table_cell(
        reader: &mut Reader<&[u8]>,
        row: &mut Vec<String>,
    ) -> Result<(), MarkitdownError> {
        let mut buf = Vec::new();
        let mut cell_text = String::new();

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf).map_err(|e| {
                MarkitdownError::ParseError(format!("Failed to read XML event: {}", e))
            })? {
                Event::Text(text) => {
                    let decoded = text.decode().map_err(|e| {
                        MarkitdownError::ParseError(format!("Failed to decode text: {}", e))
                    })?;
                    cell_text.push_str(&decoded);
                }
                Event::End(el) if el.name().as_ref() == b"a:tc" => {
                    row.push(cell_text);
                    break;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Append table as markdown
    fn append_table_markdown(markdown: &mut String, headers: &[String], rows: &[Vec<String>]) {
        markdown.push('|');
        for cell in headers {
            markdown.push_str(&format!(" {} |", cell));
        }
        markdown.push_str("\n|");
        for _ in headers {
            markdown.push_str("---|");
        }
        markdown.push('\n');
        for row in rows {
            markdown.push('|');
            for cell in row {
                markdown.push_str(&format!(" {} |", cell));
            }
            markdown.push('\n');
        }
        markdown.push('\n');
    }

    /// Parse a single slide XML and extract text content and image references
    fn parse_slide_content(
        content: &str,
    ) -> Result<(String, Vec<ContentBlock>, Vec<SlideImageRef>), MarkitdownError> {
        let (markdown, blocks) = Self::extract_text_content(content)?;
        let image_refs = Self::extract_image_refs(content);
        Ok((markdown, blocks, image_refs))
    }

    /// Convert bytes to Document - splits into helper functions
    fn bytes_to_document(
        &self,
        bytes: &[u8],
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let extract_images = options.as_ref().map(|o| o.extract_images).unwrap_or(true);

        // Extract images from media folder
        let cursor = Cursor::new(bytes);
        let mut archive = crate::open_zip(cursor)?;
        let images = if extract_images {
            Self::extract_images(&mut archive)
        } else {
            HashMap::new()
        };

        // Collect and sort slide files
        let slide_files = Self::collect_slide_files(bytes)?;

        let mut document = Document::new();

        // Process each slide
        for (slide_num, slide_path) in slide_files.iter().enumerate() {
            let slide_num = (slide_num + 1) as u32;
            let page = Self::process_slide(bytes, slide_path, slide_num, &images)?;
            document.add_page(page);
        }

        Ok(document)
    }

    /// Collect and sort slide file names from the archive
    fn collect_slide_files(bytes: &[u8]) -> Result<Vec<String>, MarkitdownError> {
        let cursor = Cursor::new(bytes);
        let mut archive = crate::open_zip(cursor)?;
        let mut slide_files: Vec<String> = Vec::new();

        for i in 0..archive.len() {
            let file = archive
                .by_index(i)
                .map_err(|e| MarkitdownError::Zip(format!("Failed to access ZIP: {}", e)))?;
            let name = file
                .name()
                .map_err(|e| MarkitdownError::Zip(format!("Failed to read ZIP entry name: {}", e)))?
                .into_owned();
            if name.starts_with("ppt/slides/slide") && name.ends_with(".xml") {
                slide_files.push(name);
            }
        }
        slide_files.sort();
        Ok(slide_files)
    }

    /// Process a single slide and return a Page
    fn process_slide(
        bytes: &[u8],
        slide_path: &str,
        slide_num: u32,
        images: &HashMap<String, Vec<u8>>,
    ) -> Result<Page, MarkitdownError> {
        let cursor = Cursor::new(bytes);
        let mut archive = crate::open_zip(cursor)?;

        // Read slide content
        let mut file = archive
            .by_name(slide_path)
            .map_err(|e| MarkitdownError::Zip(format!("Failed to access slide: {}", e)))?;
        let mut content = String::new();
        file.read_to_string(&mut content)
            .map_err(|e| MarkitdownError::ParseError(format!("Failed to read slide: {}", e)))?;

        let (_, blocks, image_refs) = Self::parse_slide_content(&content)?;

        // Get relationships for this slide to map image references
        let cursor2 = Cursor::new(bytes);
        let mut archive2 = crate::open_zip(cursor2)?;
        let relationships = Self::parse_slide_relationships(&mut archive2, slide_num);

        let mut page = Page::new(slide_num);

        // Add slide header
        page.add_content(ContentBlock::Markdown(format!(
            "<!-- Slide {} -->\n",
            slide_num
        )));

        // Add parsed content blocks
        for block in blocks {
            page.add_content(block);
        }

        // Add images that belong to this slide
        for (img_idx, img_ref) in image_refs.iter().enumerate() {
            if let Some(filename) = relationships.get(&img_ref.rel_id) {
                if let Some(data) = images.get(filename) {
                    let mime_type = Self::detect_mime_type(data);
                    let mut image = ExtractedImage::new(
                        format!("slide{}_image{}", slide_num, img_idx + 1),
                        Bytes::from(data.clone()),
                        mime_type,
                    );
                    image.alt_text = Some(filename.clone());
                    image.page_number = Some(slide_num);
                    if let Some(desc) = &img_ref.description {
                        image.description = Some(desc.clone());
                    }
                    page.add_content(ContentBlock::Image(image));
                }
            }
        }

        Ok(page)
    }
}

#[async_trait]
impl DocumentConverter for PptxConverter {
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

        let mut document = self.bytes_to_document(&bytes, options.clone())?;

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
        let mut document = self.bytes_to_document(&bytes, options.clone())?;

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
        &["pptx"]
    }
}
