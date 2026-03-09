//! PDF to Markdown converter with smart LLM fallback.
//!
//! This module provides PDF conversion with intelligent handling of:
//! - Regular PDFs with extractable text
//! - Scanned PDFs (rendered as images for LLM OCR)
//! - Complex pages with diagrams/charts (sent to LLM for description)
//! - Pages with embedded images + limited text (rendered for full context)
//! - Poor quality OCR/extraction results (detected and re-processed via LLM)
//!
//! Uses hayro for PDF rendering when LLM fallback is needed.

use async_trait::async_trait;
use bytes::Bytes;
use hayro::{render, RenderSettings};
use hayro::hayro_interpret::InterpreterSettings;
use hayro_syntax::object::dict::keys::{HEIGHT, SUBTYPE, WIDTH};
use hayro_syntax::object::{Name, Stream};
use hayro_syntax::Pdf;
use object_store::ObjectStore;
use pdf_extract;
use std::panic;
use std::sync::Arc;

use crate::error::MarkitdownError;
use crate::llm::LlmClient;
use crate::model::{ContentBlock, ConversionOptions, Document, DocumentConverter, Page};

/// Threshold for "low text" - if page has significant images and text below this, render whole page
/// This is set high because pages with good text extraction don't need LLM even with images
const LOW_TEXT_WORD_THRESHOLD: usize = 150;

/// Minimum number of XObjects to consider a page "image-heavy"
/// A single image (likely a logo/watermark) won't trigger LLM fallback
/// We need multiple images to suggest the page has meaningful visual content
const MIN_SIGNIFICANT_XOBJECTS: usize = 2;

/// Minimum image dimension (width or height) in pixels to be considered significant
/// Images smaller than this are likely logos, icons, or decorative elements
const MIN_SIGNIFICANT_IMAGE_SIZE: u32 = 100;

/// Minimum ratio of valid words to total words for acceptable quality
const MIN_VALID_WORD_RATIO: f64 = 0.6;

/// Minimum average word length for text to be considered valid
const MIN_AVG_WORD_LENGTH: f64 = 2.5;

/// Maximum ratio of special characters for text to be considered valid
const MAX_SPECIAL_CHAR_RATIO: f64 = 0.3;

/// Find a good break point in text near the target position (in characters, not bytes)
/// Prefers breaking at paragraph boundaries, then sentences, then words
fn find_text_break_point(text: &str, target_chars: usize) -> usize {
    let char_count = text.chars().count();
    if target_chars >= char_count {
        return char_count;
    }

    // Helper to convert char index to byte index
    fn char_to_byte_index(s: &str, char_idx: usize) -> usize {
        s.char_indices()
            .nth(char_idx)
            .map(|(byte_idx, _)| byte_idx)
            .unwrap_or(s.len())
    }

    // Helper to convert byte index to char index
    fn byte_to_char_index(s: &str, byte_idx: usize) -> usize {
        s[..byte_idx.min(s.len())].chars().count()
    }

    // Look for paragraph break (double newline) within 20% of target
    let search_start_chars = (target_chars as f64 * 0.8) as usize;
    let search_end_chars = (target_chars as f64 * 1.2).min(char_count as f64) as usize;

    let search_start_bytes = char_to_byte_index(text, search_start_chars);
    let search_end_bytes = char_to_byte_index(text, search_end_chars);

    if let Some(pos) = text[search_start_bytes..search_end_bytes].find("\n\n") {
        return byte_to_char_index(text, search_start_bytes + pos + 2);
    }

    // Look for sentence end
    for end_char in [". ", ".\n", "! ", "!\n", "? ", "?\n"] {
        if let Some(pos) = text[search_start_bytes..search_end_bytes].find(end_char) {
            return byte_to_char_index(text, search_start_bytes + pos + end_char.len());
        }
    }

    // Look for word boundary
    let target_bytes = char_to_byte_index(text, target_chars);
    if let Some(pos) = text[target_bytes..search_end_bytes].find(' ') {
        return byte_to_char_index(text, target_bytes + pos + 1);
    }

    target_chars
}

/// Metrics for a PDF page to determine if LLM processing is needed
#[derive(Debug)]
struct PageMetrics {
    /// Extracted text content
    text: String,
    /// Number of characters
    char_count: usize,
    /// Number of words
    word_count: usize,
    /// Ratio of alphanumeric characters to total
    alpha_ratio: f64,
    /// Whether the text seems structured (has line breaks, paragraphs)
    is_structured: bool,
    /// Number of XObjects (images/forms) on this page
    xobject_count: usize,
    /// Ratio of "valid" words (reasonable length, mostly letters)
    valid_word_ratio: f64,
    /// Average word length
    avg_word_length: f64,
    /// Ratio of special/control characters
    special_char_ratio: f64,
}

impl PageMetrics {
    fn from_text_with_xobjects(text: &str, xobject_count: usize) -> Self {
        let text = text.trim().to_string();
        let char_count = text.chars().count();
        let words: Vec<&str> = text.split_whitespace().collect();
        let word_count = words.len();

        let alpha_count = text.chars().filter(|c| c.is_alphanumeric()).count();
        let alpha_ratio = if char_count > 0 {
            alpha_count as f64 / char_count as f64
        } else {
            0.0
        };

        let line_count = text.lines().count();
        let is_structured = line_count > 3 && word_count > 10;

        // Calculate valid word ratio (words that are 2-20 chars and mostly alphabetic)
        let valid_words = words
            .iter()
            .filter(|w| {
                let len = w.len();
                let alpha = w.chars().filter(|c| c.is_alphabetic()).count();
                (2..=20).contains(&len) && (alpha as f64 / len as f64) > 0.5
            })
            .count();
        let valid_word_ratio = if word_count > 0 {
            valid_words as f64 / word_count as f64
        } else {
            0.0
        };

        // Calculate average word length
        let total_word_chars: usize = words.iter().map(|w| w.len()).sum();
        let avg_word_length = if word_count > 0 {
            total_word_chars as f64 / word_count as f64
        } else {
            0.0
        };

        // Calculate special character ratio (non-alphanumeric, non-whitespace, non-punctuation)
        let special_chars = text
            .chars()
            .filter(|c| {
                !c.is_alphanumeric() && !c.is_whitespace() && !".,:;!?'-\"()[]{}".contains(*c)
            })
            .count();
        let special_char_ratio = if char_count > 0 {
            special_chars as f64 / char_count as f64
        } else {
            0.0
        };

        Self {
            text,
            char_count,
            word_count,
            alpha_ratio,
            is_structured,
            xobject_count,
            valid_word_ratio,
            avg_word_length,
            special_char_ratio,
        }
    }

    /// Determine if this page should be processed by LLM
    fn should_use_llm(&self) -> bool {
        // Very little text = likely scanned
        if self.word_count < 10 {
            return true;
        }

        // Low alphanumeric ratio = likely garbage/OCR artifacts
        if self.alpha_ratio < 0.5 {
            return true;
        }

        // Very short unstructured content = might be missing context
        if self.char_count < 50 && !self.is_structured {
            return true;
        }

        // Poor quality text detection:
        // - Low ratio of valid words (too many garbage tokens)
        if self.valid_word_ratio < MIN_VALID_WORD_RATIO && self.word_count > 20 {
            return true;
        }

        // - Average word length too short (fragmented extraction)
        if self.avg_word_length < MIN_AVG_WORD_LENGTH && self.word_count > 10 {
            return true;
        }

        // - Too many special characters (encoding issues or garbage)
        if self.special_char_ratio > MAX_SPECIAL_CHAR_RATIO {
            return true;
        }

        // Only use LLM for pages that appear to have content-bearing images:
        // - Multiple XObjects (not just a single logo/watermark)
        // - AND limited text that might benefit from image context
        // - AND text quality is not already high (avoid re-processing good extractions)
        if self.xobject_count >= MIN_SIGNIFICANT_XOBJECTS
            && self.word_count < LOW_TEXT_WORD_THRESHOLD
            && !self.is_high_quality()
        {
            return true;
        }

        // Special case: page has many XObjects (likely diagram-heavy) even with some text
        // but only if text quality is poor
        if self.xobject_count >= 5 && !self.is_high_quality() {
            return true;
        }

        false
    }

    /// Check if the extracted text appears to be high quality
    fn is_high_quality(&self) -> bool {
        self.word_count >= 10
            && self.alpha_ratio >= 0.6
            && self.valid_word_ratio >= MIN_VALID_WORD_RATIO
            && self.avg_word_length >= MIN_AVG_WORD_LENGTH
            && self.special_char_ratio <= MAX_SPECIAL_CHAR_RATIO
    }
}

/// PDF converter with smart LLM fallback for scanned/complex pages.
pub struct PdfConverter;

impl PdfConverter {
    /// Render a PDF page as PNG image using hayro
    fn render_page_as_image(pdf: &Pdf, page_index: usize) -> Result<Vec<u8>, MarkitdownError> {
        let pages = pdf.pages();
        let page = pages.get(page_index).ok_or_else(|| {
            MarkitdownError::ParseError(format!("Page {} not found", page_index + 1))
        })?;

        let interpreter_settings = InterpreterSettings::default();
        let render_settings = RenderSettings::default();

        let pixmap = render(page, &interpreter_settings, &render_settings);

        // Encode as PNG (into_png consumes the pixmap)
        let png_data = pixmap
            .into_png()
            .map_err(|e| MarkitdownError::ParseError(format!("Failed to encode PDF page as PNG: {:?}", e)))?;

        Ok(png_data)
    }

    /// Parse PDF using hayro
    fn parse_pdf(bytes: &[u8]) -> Result<Pdf, MarkitdownError> {
        let data: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(bytes.to_vec());
        Pdf::new(data)
            .map_err(|e| MarkitdownError::ParseError(format!("Failed to parse PDF: {:?}", e)))
    }

    /// Extract text from PDF and split by pages.
    /// Uses catch_unwind to handle panics from pdf-extract on malformed PDFs.
    fn extract_text_by_page(bytes: &[u8]) -> Result<Vec<String>, MarkitdownError> {
        let bytes_owned = bytes.to_vec();
        let text_content = panic::catch_unwind(move || {
            pdf_extract::extract_text_from_mem(&bytes_owned)
        })
        .map_err(|_| {
            MarkitdownError::ParseError(
                "PDF text extraction panicked (likely malformed content stream)".to_string(),
            )
        })?
        .map_err(|e| {
            MarkitdownError::ParseError(format!("Failed to extract text from PDF: {}", e))
        })?;

        // Split by form feed (page separator)
        let pages: Vec<String> = text_content.split('\x0c').map(|s| s.to_string()).collect();

        Ok(pages)
    }

    /// Extract text aligned to actual PDF page count
    /// If text extraction gives different page count than PDF structure, redistribute text
    fn extract_text_aligned_to_pages(bytes: &[u8], actual_page_count: usize) -> Vec<String> {
        let extracted = Self::extract_text_by_page(bytes).unwrap_or_default();

        if extracted.len() == actual_page_count {
            // Perfect alignment
            return extracted;
        }

        if extracted.is_empty() || actual_page_count == 0 {
            return vec![String::new(); actual_page_count];
        }

        // Merge all text and redistribute across actual page count
        // This handles both:
        // - Fewer text pages than actual (e.g., no form feeds, all text in one block)
        // - More text pages than actual (form feeds don't align with page boundaries)
        let total_text: String = extracted.join("\n\n");

        // If total text is very small, don't try to split it
        if total_text.trim().len() < 100 {
            let mut result = vec![String::new(); actual_page_count];
            result[0] = total_text;
            return result;
        }

        let total_char_count = total_text.chars().count();
        let chars_per_page = (total_char_count / actual_page_count).max(1);

        // Helper to split a string at a character index
        fn split_at_char(s: &str, char_idx: usize) -> (&str, &str) {
            let byte_idx = s
                .char_indices()
                .nth(char_idx)
                .map(|(i, _)| i)
                .unwrap_or(s.len());
            s.split_at(byte_idx)
        }

        let mut result = Vec::with_capacity(actual_page_count);
        let mut remaining = total_text.as_str();

        for i in 0..actual_page_count {
            if i == actual_page_count - 1 {
                // Last page gets all remaining text
                result.push(remaining.to_string());
            } else {
                // Find a good break point near the target length (in characters)
                let remaining_chars = remaining.chars().count();
                let target_end = chars_per_page.min(remaining_chars);
                let break_point = find_text_break_point(remaining, target_end);
                let (page_text, rest) = split_at_char(remaining, break_point);
                result.push(page_text.trim().to_string());
                remaining = rest.trim_start();
            }
        }

        result
    }

    /// Count significant XObjects (images, forms) on a page
    /// Attempts to filter out small images (logos, icons, watermarks) by checking dimensions.
    /// Falls back to counting all XObjects if dimension extraction fails.
    fn count_significant_xobjects(pdf: &Pdf, page_index: usize) -> usize {
        let pages = pdf.pages();
        let page = match pages.get(page_index) {
            Some(p) => p,
            None => return 0,
        };

        let x_objects = &page.resources().x_objects;
        let total_count = x_objects.len();

        // If there are no XObjects or just one, return early
        // (single XObject is likely a logo anyway)
        if total_count <= 1 {
            return total_count;
        }

        // Try to count only significant XObjects by checking their dimensions
        // This is a best-effort approach - if anything fails, we count it as significant
        let significant_count = x_objects
            .keys()
            .filter(|name| {
                // Try to get the XObject as a Stream to check its dimensions
                // If we can't get it as a stream, or can't read dimensions,
                // assume it's significant (conservative approach)
                Self::is_xobject_significant(x_objects, name)
            })
            .count();

        significant_count
    }

    /// Check if an XObject is "significant" (not a small logo/icon/watermark).
    /// Uses a conservative approach: if we can't determine size, assume it's significant.
    fn is_xobject_significant(x_objects: &hayro_syntax::object::Dict, name: &Name) -> bool {
        // Try to get the XObject stream
        // Name implements Deref<Target = [u8]>, so we can pass &*name for the lookup
        let stream: Option<Stream> = x_objects.get(&**name);

        let stream = match stream {
            Some(s) => s,
            None => {
                // Can't get stream - assume significant (conservative)
                return true;
            }
        };

        // Get the stream's dictionary to access Width/Height
        let dict = stream.dict();

        // Try to read Width and Height
        // Note: These are standard PDF image XObject keys
        let width: Option<i32> = dict.get(WIDTH);
        let height: Option<i32> = dict.get(HEIGHT);

        match (width, height) {
            (Some(w), Some(h)) => {
                // Check if the image is large enough to be significant
                let w = w.unsigned_abs();
                let h = h.unsigned_abs();
                w >= MIN_SIGNIFICANT_IMAGE_SIZE || h >= MIN_SIGNIFICANT_IMAGE_SIZE
            }
            _ => {
                // Can't read dimensions - might be a Form XObject or other type
                // Check if it's an Image subtype, otherwise assume significant
                let subtype: Option<Name> = dict.get(SUBTYPE);
                match subtype {
                    Some(st) if &*st == b"Image" => {
                        // It's an image but we can't read dimensions - assume significant
                        true
                    }
                    Some(_) => {
                        // It's a Form or other XObject type - assume significant
                        true
                    }
                    None => {
                        // No subtype - assume significant
                        true
                    }
                }
            }
        }
    }

    /// Convert PDF with optional LLM fallback for complex/scanned pages
    async fn convert_with_llm(
        &self,
        bytes: &[u8],
        llm_client: Option<&dyn LlmClient>,
        force_llm: bool,
    ) -> Result<Document, MarkitdownError> {
        // Try to parse PDF structure first to get accurate page count
        let pdf = Self::parse_pdf(bytes).ok();
        let actual_page_count = pdf.as_ref().map(|p| p.pages().len()).unwrap_or(0);

        // Get text aligned to actual page count if we have it
        let page_texts = if actual_page_count > 0 {
            Self::extract_text_aligned_to_pages(bytes, actual_page_count)
        } else {
            Self::extract_text_by_page(bytes).unwrap_or_default()
        };

        let page_count = if actual_page_count > 0 {
            actual_page_count
        } else {
            page_texts.len()
        };

        // Analyze which pages need LLM processing
        let mut page_metrics: Vec<(usize, PageMetrics)> = Vec::with_capacity(page_count);
        let mut pages_needing_llm: Vec<usize> = Vec::new();

        for idx in 0..page_count {
            let page_text = page_texts.get(idx).map(|s| s.as_str()).unwrap_or("");
            let xobject_count = pdf
                .as_ref()
                .map(|p| Self::count_significant_xobjects(p, idx))
                .unwrap_or(0);

            let metrics = PageMetrics::from_text_with_xobjects(page_text, xobject_count);
            let needs_llm =
                (force_llm || metrics.should_use_llm()) && llm_client.is_some() && pdf.is_some();

            if needs_llm {
                pages_needing_llm.push(idx);
            }
            page_metrics.push((idx, metrics));
        }

        // Batch render and process pages that need LLM
        let llm_results = if !pages_needing_llm.is_empty() {
            if let (Some(llm), Some(ref pdf_ref)) = (llm_client, &pdf) {
                Self::batch_llm_convert(&pages_needing_llm, pdf_ref, llm).await
            } else {
                std::collections::HashMap::new()
            }
        } else {
            std::collections::HashMap::new()
        };

        // Build document with results
        let mut document = Document::new();

        for (idx, metrics) in page_metrics {
            let mut page = Page::new((idx + 1) as u32);

            // Check if we have LLM result for this page
            if let Some(Some(markdown)) = llm_results.get(&idx) {
                page.add_content(ContentBlock::Markdown(markdown.clone()));
            } else if !metrics.text.is_empty() {
                // Use extracted text
                if !metrics.is_high_quality() && llm_client.is_none() {
                    page.add_content(ContentBlock::Text(format!(
                        "[Note: Text extraction quality may be poor]\n\n{}",
                        metrics.text
                    )));
                } else {
                    page.add_content(ContentBlock::Text(metrics.text.clone()));
                }
            }

            if !page.content.is_empty() {
                document.add_page(page);
            }
        }

        // Handle empty document case
        if document.pages.is_empty() && page_count > 0 {
            Self::try_llm_fallback_for_all_pages(&mut document, page_count, llm_client, &pdf).await;
        }

        // Final fallback
        if document.pages.is_empty() {
            Self::add_fallback_page(&mut document);
        }

        Ok(document)
    }

    /// Batch convert pages using LLM with parallel processing
    async fn batch_llm_convert(
        pages_needing_llm: &[usize],
        pdf: &Pdf,
        llm: &dyn LlmClient,
    ) -> std::collections::HashMap<usize, Option<String>> {
        // Render all pages that need LLM processing
        let rendered_pages: Vec<(usize, Vec<u8>)> = pages_needing_llm
            .iter()
            .filter_map(|&idx| {
                Self::render_page_as_image(pdf, idx)
                    .ok()
                    .map(|png| (idx, png))
            })
            .collect();

        if rendered_pages.is_empty() {
            return std::collections::HashMap::new();
        }

        // Prepare batch for LLM
        let page_data: Vec<(&[u8], &str)> = rendered_pages
            .iter()
            .map(|(_, png)| (png.as_slice(), "image/png"))
            .collect();

        // Process in parallel batches
        let results = llm.convert_page_images_batch(&page_data).await;

        // Map results back to page indices
        rendered_pages
            .into_iter()
            .zip(results)
            .map(|((idx, _), result)| (idx, result))
            .collect()
    }

    /// Try LLM fallback for all pages when initial extraction failed
    async fn try_llm_fallback_for_all_pages(
        document: &mut Document,
        page_count: usize,
        llm_client: Option<&dyn LlmClient>,
        pdf: &Option<Pdf>,
    ) {
        if let (Some(llm), Some(ref pdf_ref)) = (llm_client, pdf) {
            // Use batch processing for all pages
            let all_pages: Vec<usize> = (0..page_count).collect();
            let results = Self::batch_llm_convert(&all_pages, pdf_ref, llm).await;

            for idx in 0..page_count {
                let mut page = Page::new((idx + 1) as u32);

                if let Some(Some(markdown)) = results.get(&idx) {
                    page.add_content(ContentBlock::Markdown(markdown.clone()));
                }

                if !page.content.is_empty() {
                    document.add_page(page);
                }
            }
        }
    }

    /// Add a fallback page when nothing could be extracted
    fn add_fallback_page(document: &mut Document) {
        let mut page = Page::new(1);
        page.add_content(ContentBlock::Text(
            "[PDF content could not be extracted. The document may be scanned, protected, or use unsupported encoding.]"
                .to_string(),
        ));
        document.add_page(page);
    }

    /// Basic conversion without LLM (original behavior)
    fn convert_basic(&self, bytes: &[u8]) -> Result<Document, MarkitdownError> {
        // Try to get actual page count from PDF structure
        let pdf = Self::parse_pdf(bytes).ok();
        let actual_page_count = pdf.as_ref().map(|p| p.pages().len()).unwrap_or(0);

        // Get text aligned to actual pages if possible
        let page_texts = if actual_page_count > 0 {
            Self::extract_text_aligned_to_pages(bytes, actual_page_count)
        } else {
            // Use catch_unwind to guard against panics in pdf-extract on malformed PDFs
            Self::extract_text_by_page(bytes).unwrap_or_default()
        };

        let mut document = Document::new();

        for (idx, page_text) in page_texts.iter().enumerate() {
            let trimmed = page_text.trim();
            if !trimmed.is_empty() {
                let mut page = Page::new((idx + 1) as u32);
                page.add_content(ContentBlock::Text(trimmed.to_string()));
                document.add_page(page);
            }
        }

        // If we have actual page count but no content, create empty pages
        if document.pages.is_empty() && actual_page_count > 0 {
            for idx in 0..actual_page_count {
                let mut page = Page::new((idx + 1) as u32);
                page.add_content(ContentBlock::Text(
                    "[Page content could not be extracted]".to_string(),
                ));
                document.add_page(page);
            }
        } else if document.pages.is_empty() {
            // Fallback: single page with all text
            let bytes_owned = bytes.to_vec();
            let text_content = panic::catch_unwind(move || {
                pdf_extract::extract_text_from_mem(&bytes_owned)
            })
            .ok()
            .and_then(|r| r.ok())
            .map(|t| t.trim().to_string())
            .unwrap_or_default();
            let mut page = Page::new(1);
            if text_content.is_empty() {
                page.add_content(ContentBlock::Text(
                    "[PDF content could not be extracted]".to_string(),
                ));
            } else {
                page.add_content(ContentBlock::Text(text_content));
            }
            document.add_page(page);
        }

        Ok(document)
    }
}

#[async_trait]
impl DocumentConverter for PdfConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        if let Some(opts) = &options {
            if let Some(ext) = &opts.file_extension {
                if ext.to_lowercase() != ".pdf" {
                    return Err(MarkitdownError::InvalidFile(format!(
                        "Expected .pdf file, got {}",
                        ext
                    )));
                }
            }
        }

        let result = store.get(path).await?;
        let bytes = result.bytes().await?;

        // Check if we have an LLM client for smart conversion
        if let Some(opts) = &options {
            if let Some(llm) = &opts.llm_client {
                let force_llm = opts.force_llm_ocr;
                return self
                    .convert_with_llm(&bytes, Some(llm.as_ref()), force_llm)
                    .await;
            }
        }

        self.convert_basic(&bytes)
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        if let Some(opts) = &options {
            if let Some(ext) = &opts.file_extension {
                if ext.to_lowercase() != ".pdf" {
                    return Err(MarkitdownError::InvalidFile(format!(
                        "Expected .pdf file, got {}",
                        ext
                    )));
                }
            }
        }

        // Check if we have an LLM client for smart conversion
        if let Some(opts) = &options {
            if let Some(llm) = &opts.llm_client {
                let force_llm = opts.force_llm_ocr;
                return self
                    .convert_with_llm(&bytes, Some(llm.as_ref()), force_llm)
                    .await;
            }
        }

        self.convert_basic(&bytes)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".pdf"]
    }
}

/// Configuration for PDF conversion behavior
#[derive(Debug, Clone)]
pub struct PdfConversionConfig {
    /// Minimum words required before considering LLM fallback unnecessary
    pub min_words_threshold: usize,
    /// Minimum alphanumeric ratio before text is considered garbage
    pub min_alpha_ratio: f64,
    /// Whether to always use LLM for all pages
    pub always_use_llm: bool,
}

impl Default for PdfConversionConfig {
    fn default() -> Self {
        Self {
            min_words_threshold: 10,
            min_alpha_ratio: 0.5,
            always_use_llm: false,
        }
    }
}
