use async_trait::async_trait;
use bytes::Bytes;
use object_store::ObjectStore;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::error::MarkitdownError;
use crate::llm::{LlmClient, SharedLlmClient};
use crate::summarize::{self, DocumentSummary, SummaryConfig};
use crate::table_merge;

/// Represents an extracted image from a document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedImage {
    /// Unique identifier for this image within the document
    pub id: String,
    /// Image data as bytes (raw image data, e.g., PNG, JPEG)
    #[serde(skip_serializing, skip_deserializing)]
    pub data: Bytes,
    /// MIME type of the image (e.g., "image/png", "image/jpeg")
    pub mime_type: String,
    /// Optional alt text or caption if available from source
    pub alt_text: Option<String>,
    /// Optional LLM-generated description
    pub description: Option<String>,
    /// Width in pixels if known
    pub width: Option<u32>,
    /// Height in pixels if known
    pub height: Option<u32>,
    /// Page number where the image appears (1-indexed)
    pub page_number: Option<u32>,
    /// Optional source path or context hint for this image
    pub source_path: Option<String>,
}

impl ExtractedImage {
    pub fn new(id: impl Into<String>, data: Bytes, mime_type: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            data,
            mime_type: mime_type.into(),
            alt_text: None,
            description: None,
            width: None,
            height: None,
            page_number: None,
            source_path: None,
        }
    }

    /// Get base64 encoded image data
    pub fn to_base64(&self) -> String {
        use base64::prelude::*;
        BASE64_STANDARD.encode(&self.data)
    }

    /// Get a markdown image reference (placeholder for replacement)
    pub fn as_markdown_placeholder(&self) -> String {
        format!("![Image: {}](image:{})", self.id, self.id)
    }

    /// Get the description or fallback to alt_text
    pub fn get_display_text(&self) -> Option<&str> {
        self.description.as_deref().or(self.alt_text.as_deref())
    }
}

/// Represents a block of content in a page
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContentBlock {
    /// Plain text content
    Text(String),
    /// A heading with level (1-6) and text
    Heading { level: u8, text: String },
    /// An image reference
    Image(ExtractedImage),
    /// A table with headers and rows
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    /// A list (ordered or unordered)
    List { ordered: bool, items: Vec<String> },
    /// A code block with optional language
    Code {
        language: Option<String>,
        code: String,
    },
    /// A blockquote
    Quote(String),
    /// Raw markdown (already formatted)
    Markdown(String),
}

impl ContentBlock {
    /// Convert this content block to markdown
    pub fn to_markdown(&self) -> String {
        match self {
            ContentBlock::Text(text) => format!("{}\n", text),
            ContentBlock::Heading { level, text } => {
                format!("{} {}\n", "#".repeat(*level as usize), text)
            }
            ContentBlock::Image(img) => {
                if let Some(desc) = img.get_display_text() {
                    format!("![{}]({})\n\n*{}*\n", img.id, img.id, desc)
                } else {
                    format!("![{}]({})\n", img.id, img.id)
                }
            }
            ContentBlock::Table { headers, rows } => {
                let mut md = String::new();
                md.push_str("| ");
                md.push_str(&headers.join(" | "));
                md.push_str(" |\n| ");
                md.push_str(
                    &headers
                        .iter()
                        .map(|_| "---")
                        .collect::<Vec<_>>()
                        .join(" | "),
                );
                md.push_str(" |\n");
                for row in rows {
                    md.push_str("| ");
                    md.push_str(&row.join(" | "));
                    md.push_str(" |\n");
                }
                md
            }
            ContentBlock::List { ordered, items } => {
                let mut md = String::new();
                for (i, item) in items.iter().enumerate() {
                    if *ordered {
                        md.push_str(&format!("{}. {}\n", i + 1, item));
                    } else {
                        md.push_str(&format!("- {}\n", item));
                    }
                }
                md
            }
            ContentBlock::Code { language, code } => {
                format!("```{}\n{}\n```\n", language.as_deref().unwrap_or(""), code)
            }
            ContentBlock::Quote(text) => {
                text.lines()
                    .map(|line| format!("> {}", line))
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n"
            }
            ContentBlock::Markdown(md) => md.clone(),
        }
    }
}

/// Represents a single page of content extracted from a document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    /// Page number (1-indexed)
    pub page_number: u32,
    /// Content blocks in order
    pub content: Vec<ContentBlock>,
    /// Optional rendered image of the entire page (for scanned PDFs, slides, complex layouts)
    pub rendered_image: Option<ExtractedImage>,
}

impl Page {
    pub fn new(page_number: u32) -> Self {
        Self {
            page_number,
            content: Vec::new(),
            rendered_image: None,
        }
    }

    /// Set the rendered image for this page
    pub fn with_rendered_image(mut self, image: ExtractedImage) -> Self {
        self.rendered_image = Some(image);
        self
    }

    /// Add a content block to the page
    pub fn add_content(&mut self, block: ContentBlock) {
        self.content.push(block);
    }

    /// Apply a source path hint to all images on this page (if not already set)
    pub fn apply_image_context_path(&mut self, path: &str) {
        for block in &mut self.content {
            if let ContentBlock::Image(img) = block {
                if img.source_path.is_none() {
                    img.source_path = Some(path.to_string());
                }
            }
        }

        if let Some(rendered_image) = &mut self.rendered_image {
            if rendered_image.source_path.is_none() {
                rendered_image.source_path = Some(path.to_string());
            }
        }
    }

    /// Get all images from this page
    pub fn images(&self) -> Vec<&ExtractedImage> {
        self.content
            .iter()
            .filter_map(|block| {
                if let ContentBlock::Image(img) = block {
                    Some(img)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get all text content (excluding images) as markdown
    pub fn to_markdown(&self) -> String {
        self.content
            .iter()
            .map(|block| block.to_markdown())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Create a new page with images replaced by their LLM descriptions
    /// Uses batch processing based on the LLM client's images_per_message setting
    pub async fn with_image_descriptions(
        &self,
        llm_client: &dyn LlmClient,
    ) -> Result<Page, MarkitdownError> {
        // Collect images that need descriptions
        let images_to_describe: Vec<(usize, &ExtractedImage)> = self
            .content
            .iter()
            .enumerate()
            .filter_map(|(i, block)| {
                if let ContentBlock::Image(img) = block {
                    if img.description.is_none() {
                        return Some((i, img));
                    }
                }
                None
            })
            .collect();

        // If no images need descriptions, return a clone
        if images_to_describe.is_empty() {
            return Ok(self.clone());
        }

        // Prepare image references for batch processing
        let image_refs: Vec<&ExtractedImage> =
            images_to_describe.iter().map(|(_, img)| *img).collect();

        // Get descriptions using the context-aware method
        let descriptions = llm_client.describe_extracted_images(&image_refs).await?;

        // Build the new page with descriptions
        let mut new_page = Page::new(self.page_number);
        let mut desc_iter = descriptions.into_iter();
        let mut image_indices: std::collections::HashSet<usize> =
            images_to_describe.iter().map(|(i, _)| *i).collect();

        for (i, block) in self.content.iter().enumerate() {
            match block {
                ContentBlock::Image(img) if image_indices.remove(&i) => {
                    let mut new_img = img.clone();
                    if let Some(desc) = desc_iter.next() {
                        new_img.description = Some(desc);
                    }
                    new_page.add_content(ContentBlock::Image(new_img));
                }
                other => new_page.add_content(other.clone()),
            }
        }

        Ok(new_page)
    }

    /// Convert this page to a text-only page, replacing images with their descriptions
    pub fn to_text_only(&self) -> Page {
        let mut new_page = Page::new(self.page_number);

        for block in &self.content {
            match block {
                ContentBlock::Image(img) => {
                    if let Some(text) = img.get_display_text() {
                        new_page.add_content(ContentBlock::Text(format!("[Image: {}]", text)));
                    } else {
                        new_page.add_content(ContentBlock::Text(format!("[Image: {}]", img.id)));
                    }
                }
                other => new_page.add_content(other.clone()),
            }
        }

        new_page
    }
}

/// Represents a complete document with multiple pages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    /// Optional document title
    pub title: Option<String>,
    /// Document pages
    pub pages: Vec<Page>,
    /// Document-level metadata
    pub metadata: std::collections::HashMap<String, String>,
    /// Document summary, set when the document was produced in summarization mode.
    /// The pages then hold the LLM's notes for each page instead of the full content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<DocumentSummary>,
}

impl Document {
    pub fn new() -> Self {
        Self {
            title: None,
            pages: Vec::new(),
            metadata: std::collections::HashMap::new(),
            summary: None,
        }
    }

    /// Create a document from a single page
    pub fn from_page(page: Page) -> Self {
        Self {
            title: None,
            pages: vec![page],
            metadata: std::collections::HashMap::new(),
            summary: None,
        }
    }

    /// Add a page to the document
    pub fn add_page(&mut self, page: Page) {
        self.pages.push(page);
    }

    /// Get all images from the document
    pub fn images(&self) -> Vec<&ExtractedImage> {
        self.pages.iter().flat_map(|p| p.images()).collect()
    }

    /// Apply a source path hint to all images in the document (if not already set)
    pub fn apply_image_context_path(&mut self, path: &str) {
        for page in &mut self.pages {
            page.apply_image_context_path(path);
        }
    }

    /// Convert the entire document to markdown
    ///
    /// For summarized documents this renders the summary and key points, followed by
    /// the notes for each page.
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        if let Some(title) = &self.title {
            md.push_str(&format!("# {}\n\n", title));
        }

        if let Some(summary) = &self.summary {
            md.push_str(&summary.to_markdown());
            if !self.pages.is_empty() {
                md.push_str("\n## Notes\n");
            }
            for page in &self.pages {
                if self.pages.len() > 1 {
                    md.push_str(&format!("\n### Page {}\n", page.page_number));
                }
                md.push('\n');
                md.push_str(&page.to_markdown());
            }
            return md;
        }

        for page in &self.pages {
            if self.pages.len() > 1 {
                md.push_str(&format!("\n---\n## Page {}\n\n", page.page_number));
            }
            md.push_str(&page.to_markdown());
        }

        md
    }

    /// Create a new document with all images replaced by LLM descriptions
    pub async fn with_image_descriptions(
        &self,
        llm_client: &dyn LlmClient,
    ) -> Result<Document, MarkitdownError> {
        let mut new_doc = Document::new();
        new_doc.title = self.title.clone();
        new_doc.metadata = self.metadata.clone();
        new_doc.summary = self.summary.clone();

        for page in &self.pages {
            new_doc.add_page(page.with_image_descriptions(llm_client).await?);
        }

        Ok(new_doc)
    }

    /// Summarize this document: concise notes for every page plus a document summary.
    ///
    /// Pages are sent to the LLM as input (text, rendered page images, embedded images)
    /// and only notes are generated, so no page is transcribed. The returned document's
    /// pages hold the notes and [`Document::summary`] holds the summary.
    /// See [`crate::summarize`] for the pipeline.
    pub async fn summarize(
        &self,
        llm_client: &dyn LlmClient,
        config: &SummaryConfig,
    ) -> Result<Document, MarkitdownError> {
        summarize::summarize_document(self, llm_client, config).await
    }

    /// Convert to text-only document
    pub fn to_text_only(&self) -> Document {
        let mut new_doc = Document::new();
        new_doc.title = self.title.clone();
        new_doc.metadata = self.metadata.clone();
        new_doc.summary = self.summary.clone();

        for page in &self.pages {
            new_doc.add_page(page.to_text_only());
        }

        new_doc
    }

    /// Merge tables that span multiple pages into single tables.
    ///
    /// This method detects tables at page boundaries and merges them when:
    /// - A table ends at the bottom of a page
    /// - The next page starts with table content (with or without header)
    /// - The column counts match
    ///
    /// Merged tables are placed on the first page where they start.
    pub fn with_merged_tables(&self) -> Document {
        if self.pages.len() <= 1 {
            return self.clone();
        }

        // Extract markdown content from each page
        let page_contents: Vec<(u32, String)> = self
            .pages
            .iter()
            .map(|p| (p.page_number, p.to_markdown()))
            .collect();

        // Merge tables across pages
        let merged_contents = table_merge::merge_tables_across_pages(&page_contents);

        // Rebuild the document with merged content
        let mut new_doc = Document::new();
        new_doc.title = self.title.clone();
        new_doc.metadata = self.metadata.clone();
        new_doc.summary = self.summary.clone();

        for merged in merged_contents {
            let mut page = Page::new(merged.page_number);
            // Use the merged markdown content
            page.add_content(ContentBlock::Markdown(merged.content));
            new_doc.add_page(page);
        }

        new_doc
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

/// Legacy result type for backward compatibility
#[derive(Debug, Serialize, Deserialize)]
pub struct DocumentConverterResult {
    pub title: Option<String>,
    pub text_content: String,
}

impl From<Document> for DocumentConverterResult {
    fn from(doc: Document) -> Self {
        Self {
            title: doc.title.clone(),
            text_content: doc.to_markdown(),
        }
    }
}

/// What a conversion produces when an LLM client is configured.
// Options are built once per conversion, so boxing the config would only make the API clunkier
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Default)]
pub enum ConversionMode {
    /// Full-content extraction (default). Every page becomes markdown: pages that need
    /// vision are transcribed by the LLM and embedded images get detailed descriptions.
    #[default]
    Extract,
    /// Summarization. Pages are sent to the LLM as input (extracted text, rendered page
    /// images, embedded images) and it writes only concise notes per page plus a document
    /// summary, so no page is ever transcribed and output tokens stay small.
    /// Requires an LLM client.
    Summarize(SummaryConfig),
}

/// Options for document conversion
#[derive(Clone)]
pub struct ConversionOptions {
    /// File extension hint (e.g., ".pdf", ".docx")
    pub file_extension: Option<String>,
    /// Source URL if applicable
    pub url: Option<String>,
    /// Optional LLM client for image descriptions
    pub llm_client: Option<SharedLlmClient>,
    /// Optional path context hint for LLM image descriptions
    pub image_context_path: Option<String>,
    /// Whether to extract images
    pub extract_images: bool,
    /// Force LLM OCR for all PDF pages (useful for PDFs with images)
    pub force_llm_ocr: bool,
    /// Merge tables that span multiple pages into a single table
    pub merge_multipage_tables: bool,
    /// Full extraction (default) or summarization, see [`ConversionMode`]
    pub mode: ConversionMode,
}

impl std::fmt::Debug for ConversionOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConversionOptions")
            .field("file_extension", &self.file_extension)
            .field("url", &self.url)
            .field(
                "llm_client",
                &self.llm_client.as_ref().map(|_| "<LlmClient>"),
            )
            .field("image_context_path", &self.image_context_path)
            .field("extract_images", &self.extract_images)
            .field("force_llm_ocr", &self.force_llm_ocr)
            .field("mode", &self.mode)
            .finish()
    }
}

impl Default for ConversionOptions {
    fn default() -> Self {
        Self {
            file_extension: None,
            url: None,
            llm_client: None,
            image_context_path: None,
            extract_images: true,
            force_llm_ocr: false,
            merge_multipage_tables: false,
            mode: ConversionMode::Extract,
        }
    }
}

impl ConversionOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_extension(mut self, ext: impl Into<String>) -> Self {
        self.file_extension = Some(ext.into());
        self
    }

    pub fn with_llm(mut self, client: SharedLlmClient) -> Self {
        self.llm_client = Some(client);
        self
    }

    /// Provide a path context hint for LLM image descriptions
    pub fn with_image_context_path(mut self, path: impl Into<String>) -> Self {
        self.image_context_path = Some(path.into());
        self
    }

    pub fn with_images(mut self, extract: bool) -> Self {
        self.extract_images = extract;
        self
    }

    /// Force LLM OCR for all PDF pages, regardless of text quality.
    /// This is useful for PDFs with important images that need descriptions.
    pub fn with_force_llm_ocr(mut self, force: bool) -> Self {
        self.force_llm_ocr = force;
        self
    }

    /// Enable merging of tables that span multiple pages.
    /// When enabled, tables that continue from one page to the next will be
    /// merged into a single table on the first page.
    pub fn with_merge_multipage_tables(mut self, merge: bool) -> Self {
        self.merge_multipage_tables = merge;
        self
    }

    /// Choose between full extraction (default) and summarization.
    pub fn with_mode(mut self, mode: ConversionMode) -> Self {
        self.mode = mode;
        self
    }

    /// Switch to summarization mode with the given settings.
    /// Shorthand for `with_mode(ConversionMode::Summarize(config))`.
    pub fn with_summarization(self, config: SummaryConfig) -> Self {
        self.with_mode(ConversionMode::Summarize(config))
    }

    /// The summarization settings, if in summarization mode
    pub fn summary_config(&self) -> Option<&SummaryConfig> {
        match &self.mode {
            ConversionMode::Summarize(config) => Some(config),
            ConversionMode::Extract => None,
        }
    }

    /// The LLM client converters should use to describe embedded images.
    ///
    /// `None` in summarization mode: the summarizer looks at the images itself, so
    /// generating detailed descriptions up front would only add output tokens.
    pub fn image_description_llm(&self) -> Option<&SharedLlmClient> {
        match self.mode {
            ConversionMode::Extract => self.llm_client.as_ref(),
            ConversionMode::Summarize(_) => None,
        }
    }
}

/// Trait for document converters that work with ObjectStore
#[async_trait]
pub trait DocumentConverter: Send + Sync {
    /// Convert a file from ObjectStore to a Document
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError>;

    /// Convert bytes directly to a Document
    async fn convert_bytes(
        &self,
        bytes: Bytes,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError>;

    /// Get supported file extensions
    fn supported_extensions(&self) -> &[&str];

    /// Check if this converter can handle the given extension
    fn can_handle(&self, extension: &str) -> bool {
        let ext_normalized = extension.trim_start_matches('.');
        self.supported_extensions().iter().any(|supported| {
            supported
                .trim_start_matches('.')
                .eq_ignore_ascii_case(ext_normalized)
        })
    }
}
