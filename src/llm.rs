//! LLM abstraction for document conversion using any rig-core compatible model.
//!
//! This module provides a provider-agnostic LLM wrapper that works with ANY model
//! implementing rig-core's `CompletionModel` trait. This includes:
//! - OpenAI (GPT-4, GPT-4o, etc.)
//! - Anthropic (Claude)
//! - Google (Gemini)
//! - Cohere
//! - Any custom implementation
//!
//! # Example
//! ```ignore
//! use rig::providers::openai;
//! use markitdown::llm::{LlmWrapper, LlmConfig};
//!
//! let client = openai::Client::from_env();
//! let model = client.completion_model("gpt-4o");
//! let llm = LlmWrapper::new(model);
//! ```

use async_trait::async_trait;
use base64::prelude::*;
use futures::future::join_all;
use futures::FutureExt;
use image::ImageFormat;
use rig::{
    completion::{AssistantContent, CompletionModel, CompletionRequest, CompletionRequestBuilder},
    message::{ImageDetail, ImageMediaType, Message, UserContent},
    OneOrMany,
};
use std::io::Cursor;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use crate::error::MarkitdownError;
use crate::model::ExtractedImage;
use crate::prompts::{
    DEFAULT_BATCH_IMAGE_PROMPT, DEFAULT_IMAGE_DESCRIPTION_PROMPT, DEFAULT_PAGE_CONVERSION_PROMPT,
};

/// Configuration for LLM behavior during document conversion.
#[derive(Debug, Clone)]
pub struct LlmConfig {
    /// System prompt for single image description
    pub image_description_prompt: String,
    /// System prompt for full page conversion (PDF pages rendered as images)
    pub page_conversion_prompt: String,
    /// System prompt for batch image description (when images_per_message > 1)
    pub batch_image_prompt: String,
    /// Temperature for LLM responses (0.0 = deterministic, 1.0 = creative)
    pub temperature: f64,
    /// Number of images to send per LLM message for parallelization
    /// Default: 1 (one image per request)
    pub images_per_message: usize,
    /// Number of PDF pages to process in parallel
    /// Higher values speed up conversion but use more memory and API rate limit
    /// Default: 4 (process 4 pages concurrently)
    pub pages_per_batch: usize,
    /// Maximum tokens for LLM output (None = no limit)
    /// Helps prevent runaway generation from poorly-behaved models.
    ///
    /// **Important**: This value is passed directly to the LLM provider.
    /// If it exceeds the model's maximum output tokens, the API will return an error.
    /// Common limits:
    /// - GPT-4o: 16,384 tokens
    /// - Claude 3.5 Sonnet: 8,192 tokens  
    /// - Free/smaller models: often 2,048-4,096 tokens
    ///
    /// The default of `None` lets the model use its natural limit, which is
    /// generally safe but may allow runaway generation on some models.
    /// For free-tier or less reliable models, consider setting this explicitly.
    pub max_tokens: Option<u64>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            image_description_prompt: DEFAULT_IMAGE_DESCRIPTION_PROMPT.to_string(),
            page_conversion_prompt: DEFAULT_PAGE_CONVERSION_PROMPT.to_string(),
            batch_image_prompt: DEFAULT_BATCH_IMAGE_PROMPT.to_string(),
            temperature: 0.1, // Low temperature for accurate extraction with minimal repetition
            images_per_message: 1,
            pages_per_batch: 4, // Process 4 pages concurrently by default
            max_tokens: None, // Let model use its natural limit; set explicitly for unreliable models
        }
    }
}

impl LlmConfig {
    /// Create a new config with default prompts
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the image description prompt
    pub fn with_image_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.image_description_prompt = prompt.into();
        self
    }

    /// Set the page conversion prompt
    pub fn with_page_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.page_conversion_prompt = prompt.into();
        self
    }

    /// Set the batch image prompt
    pub fn with_batch_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.batch_image_prompt = prompt.into();
        self
    }

    /// Set the temperature
    pub fn with_temperature(mut self, temp: f64) -> Self {
        self.temperature = temp.clamp(0.0, 2.0);
        self
    }

    /// Set images per message for batching
    pub fn with_images_per_message(mut self, count: usize) -> Self {
        self.images_per_message = count.max(1);
        self
    }

    /// Set pages per batch for parallel PDF processing
    /// Higher values process more pages concurrently but use more memory
    pub fn with_pages_per_batch(mut self, count: usize) -> Self {
        self.pages_per_batch = count.max(1);
        self
    }

    /// Set maximum tokens for LLM output
    /// Use None for no limit (not recommended for production)
    pub fn with_max_tokens(mut self, tokens: Option<u64>) -> Self {
        self.max_tokens = tokens;
        self
    }
}

/// Trait for LLM clients that can describe images and convert pages.
///
/// This trait is implemented for any type that wraps a rig-core CompletionModel,
/// allowing provider-agnostic LLM usage.
#[async_trait]
pub trait LlmClient: Send + Sync {
    /// Generate a description for an image from raw bytes
    async fn describe_image(
        &self,
        image_data: &[u8],
        mime_type: &str,
    ) -> Result<String, MarkitdownError>;

    /// Generate a description for an image from base64
    async fn describe_image_base64(
        &self,
        base64_data: &str,
        mime_type: &str,
    ) -> Result<String, MarkitdownError>;

    /// Generate descriptions for multiple images in batch
    /// Returns one description per image in the same order
    async fn describe_images_batch(
        &self,
        images: &[(&[u8], &str)], // (data, mime_type) pairs
    ) -> Result<Vec<String>, MarkitdownError>;

    /// Generate descriptions for multiple ExtractedImage references in batch.
    /// This is the preferred method as it provides access to image metadata
    /// (alt_text, dimensions, page_number) for better context-aware descriptions.
    /// Returns one description per image in the same order.
    async fn describe_extracted_images(
        &self,
        images: &[&ExtractedImage],
    ) -> Result<Vec<String>, MarkitdownError> {
        // Default implementation delegates to describe_images_batch
        let data: Vec<(&[u8], &str)> = images
            .iter()
            .map(|img| (img.data.as_ref(), img.mime_type.as_str()))
            .collect();
        self.describe_images_batch(&data).await
    }

    /// Convert a page image to markdown (for PDF/scanned document conversion)
    async fn convert_page_image(
        &self,
        image_data: &[u8],
        mime_type: &str,
    ) -> Result<String, MarkitdownError>;

    /// Convert multiple page images to markdown in parallel
    /// Returns results in the same order as input, with None for failed conversions
    async fn convert_page_images_batch(
        &self,
        pages: &[(&[u8], &str)], // (image_data, mime_type) pairs
    ) -> Vec<Option<String>>;

    /// Generate a text completion
    async fn complete(&self, prompt: &str) -> Result<String, MarkitdownError>;

    /// Get the current configuration
    fn config(&self) -> &LlmConfig;

    /// Get number of images per message setting
    fn images_per_message(&self) -> usize {
        self.config().images_per_message
    }
}

/// Universal LLM wrapper that works with any rig-core CompletionModel.
///
/// This wrapper accepts any model implementing `CompletionModel`, making it
/// compatible with all rig-core providers (OpenAI, Anthropic, Google, etc.)
///
/// # Example
/// ```ignore
/// use rig::providers::openai;
/// use markitdown::llm::{LlmWrapper, LlmConfig};
///
/// // Create with any provider
/// let client = openai::Client::from_env();
/// let model = client.completion_model("gpt-4o");
/// let llm = LlmWrapper::new(model);
///
/// // Or with custom config
/// let llm = LlmWrapper::with_config(
///     model,
///     LlmConfig::default().with_images_per_message(3)
/// );
/// ```
pub struct LlmWrapper<M: CompletionModel> {
    model: Arc<M>,
    config: LlmConfig,
}

impl<M: CompletionModel> LlmWrapper<M> {
    /// Create a new LLM wrapper with default configuration
    pub fn new(model: M) -> Self {
        Self {
            model: Arc::new(model),
            config: LlmConfig::default(),
        }
    }

    /// Create a new LLM wrapper with custom configuration
    pub fn with_config(model: M, config: LlmConfig) -> Self {
        Self {
            model: Arc::new(model),
            config,
        }
    }

    /// Create from an Arc'd model
    pub fn from_arc(model: Arc<M>, config: LlmConfig) -> Self {
        Self { model, config }
    }

    /// Get a reference to the underlying model
    pub fn model(&self) -> &M {
        &self.model
    }

    /// Get a mutable reference to the config
    pub fn config_mut(&mut self) -> &mut LlmConfig {
        &mut self.config
    }

    /// Build a completion request with the given system prompt and user content
    fn build_request(
        &self,
        system_prompt: &str,
        user_content: OneOrMany<UserContent>,
    ) -> CompletionRequestBuilder<M> {
        let mut builder = self
            .model
            .completion_request(system_prompt)
            .messages(vec![Message::User {
                content: user_content,
            }])
            .temperature(self.config.temperature);

        if let Some(max_tokens) = self.config.max_tokens {
            builder = builder.max_tokens(max_tokens);
        }

        builder
    }

    /// Send a request and extract the response text, with post-processing for repetition detection.
    /// Uses catch_unwind to handle panics from rig-core on invalid API responses (e.g. 400 errors).
    async fn send_request(&self, request: CompletionRequest) -> Result<String, MarkitdownError> {
        let response = AssertUnwindSafe(self.model.completion(request))
            .catch_unwind()
            .await
            .map_err(|panic_info| {
                let msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    "Unknown panic in LLM completion".to_string()
                };
                MarkitdownError::LlmError(format!("LLM completion panicked: {}", msg))
            })?
            .map(|r| extract_text_from_response(&r.choice))
            .map_err(|e| MarkitdownError::LlmError(format!("LLM error: {}", e)))?;

        // Apply repetition detection and cleanup
        Ok(detect_and_truncate_repetition(&response))
    }

    /// Describe a single ExtractedImage with context-aware prompts
    async fn describe_single_extracted_image(
        &self,
        img: &ExtractedImage,
    ) -> Result<String, MarkitdownError>
    where
        M: Send + Sync + 'static,
    {
        // Prepare the image, converting if necessary
        let (base64_data, image_type) = match prepare_image_for_llm(&img.data, &img.mime_type) {
            PreparedImage::Ready(media_type) => (img.to_base64(), media_type),
            PreparedImage::Converted(png_data) => {
                (BASE64_STANDARD.encode(&png_data), ImageMediaType::PNG)
            }
            PreparedImage::Unsupported(reason) => {
                return Ok(format!("[Image skipped: {}]", reason));
            }
        };

        let mut content = OneOrMany::one(UserContent::image_base64(
            base64_data,
            Some(image_type),
            Some(ImageDetail::Auto),
        ));

        // Build a context-aware prompt
        let mut prompt = String::from("Describe this image in detail.");
        if let Some(alt) = &img.alt_text {
            prompt.push_str(&format!(
                " The existing alt text is: '{}'. Build upon or improve this description.",
                alt
            ));
        }
        if let (Some(w), Some(h)) = (img.width, img.height) {
            prompt.push_str(&format!(" The image is {}x{} pixels.", w, h));
        }
        if let Some(page) = img.page_number {
            prompt.push_str(&format!(
                " This image appears on page {} of the document.",
                page
            ));
        }
        if let Some(path) = &img.source_path {
            prompt.push_str(&format!(
                " The source path is: '{}'. Use this as a contextual hint if relevant.",
                path
            ));
        }

        content.push(UserContent::text(prompt));

        let request = self
            .build_request(&self.config.image_description_prompt, content)
            .build();

        self.send_request(request).await
    }

    async fn describe_extracted_images_individual(
        &self,
        images: &[&ExtractedImage],
    ) -> Result<Vec<String>, MarkitdownError>
    where
        M: Send + Sync + 'static,
    {
        let futures: Vec<_> = images
            .iter()
            .map(|img| self.describe_single_extracted_image(img))
            .collect();

        let results = join_all(futures).await;
        results.into_iter().collect()
    }

    async fn describe_extracted_images_batched(
        &self,
        images: &[&ExtractedImage],
    ) -> Result<Vec<String>, MarkitdownError>
    where
        M: Send + Sync + 'static,
    {
        let mut all_descriptions = Vec::with_capacity(images.len());
        let batch_size = self.config.images_per_message;

        for chunk in images.chunks(batch_size) {
            if chunk.len() == 1 {
                let desc = self.describe_single_extracted_image(chunk[0]).await?;
                all_descriptions.push(desc);
                continue;
            }

            let mut content = OneOrMany::one(UserContent::text(&self.config.batch_image_prompt));
            let mut processed_count = 0;
            let mut skipped_descriptions = Vec::new();

            for (i, img) in chunk.iter().enumerate() {
                // Prepare the image, converting if necessary
                let (base64_data, image_type) =
                    match prepare_image_for_llm(&img.data, &img.mime_type) {
                        PreparedImage::Ready(media_type) => (img.to_base64(), media_type),
                        PreparedImage::Converted(png_data) => {
                            (BASE64_STANDARD.encode(&png_data), ImageMediaType::PNG)
                        }
                        PreparedImage::Unsupported(reason) => {
                            skipped_descriptions.push((i, format!("[Image skipped: {}]", reason)));
                            continue;
                        }
                    };

                processed_count += 1;
                let mut context = format!("\n--- Image {} ---", processed_count);
                if let Some(alt) = &img.alt_text {
                    context.push_str(&format!("\nExisting alt text: {}", alt));
                }
                if let (Some(w), Some(h)) = (img.width, img.height) {
                    context.push_str(&format!("\nDimensions: {}x{} pixels", w, h));
                }
                if let Some(page) = img.page_number {
                    context.push_str(&format!("\nFrom page: {}", page));
                }
                if let Some(path) = &img.source_path {
                    context.push_str(&format!("\nSource path hint: {}", path));
                }

                content.push(UserContent::text(context));
                content.push(UserContent::image_base64(
                    base64_data,
                    Some(image_type),
                    Some(ImageDetail::Auto),
                ));
            }

            // If all images were skipped, just add the placeholders
            if processed_count == 0 {
                for (_, desc) in skipped_descriptions {
                    all_descriptions.push(desc);
                }
                continue;
            }

            let request = self
                .build_request(&self.config.batch_image_prompt, content)
                .build();

            let response = self.send_request(request).await?;
            let mut descriptions = parse_batch_response(&response, processed_count);

            // Insert skipped placeholders at their original positions
            for (original_idx, placeholder) in skipped_descriptions {
                let insert_pos = original_idx.min(descriptions.len());
                descriptions.insert(insert_pos, placeholder);
            }

            all_descriptions.extend(descriptions);
        }

        Ok(all_descriptions)
    }
}

/// Extract text content from assistant response
fn extract_text_from_response(content: &OneOrMany<AssistantContent>) -> String {
    content
        .iter()
        .filter_map(|c| match c {
            AssistantContent::Text(text) => Some(text.text.clone()),
            AssistantContent::Reasoning(r) => Some(r.reasoning.join("\n")),
            AssistantContent::ToolCall(_) => None,
            AssistantContent::Image(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Detect and truncate repetitive content from LLM output.
///
/// Some models (especially free tier ones) can get stuck in loops, generating
/// the same phrase thousands of times. This function detects such patterns
/// and truncates the output at the first repetition.
///
/// This function is intentionally conservative to avoid truncating legitimate content
/// that may have natural repetition (table headers, code patterns, etc.).
fn detect_and_truncate_repetition(text: &str) -> String {
    // If text is short, no need to check - increased threshold to avoid false positives
    if text.len() < 2000 {
        return text.to_string();
    }

    // Helper to safely get a substring by character indices (not byte indices)
    // Returns None if the range is invalid
    fn safe_char_slice(s: &str, char_start: usize, char_end: usize) -> Option<&str> {
        if char_start >= char_end {
            return None;
        }
        let mut char_indices = s.char_indices();
        let start_byte = char_indices.nth(char_start).map(|(i, _)| i)?;
        // We need to advance (char_end - char_start - 1) more positions, then get the end
        let end_byte = if char_end - char_start == 1 {
            // If only one character, end is after this char
            char_indices.next().map(|(i, _)| i).unwrap_or(s.len())
        } else {
            char_indices
                .nth(char_end - char_start - 2)
                .map(|(_, _)| ())
                .and_then(|_| char_indices.next().map(|(i, _)| i))
                .unwrap_or(s.len())
        };
        Some(&s[start_byte..end_byte])
    }

    // Helper to safely truncate a string to a maximum number of characters
    fn safe_truncate_chars(s: &str, max_chars: usize) -> &str {
        match s.char_indices().nth(max_chars) {
            Some((byte_pos, _)) => &s[..byte_pos],
            None => s,
        }
    }

    // Strategy 1: Check for repeating substrings using sliding window
    // Look for patterns that repeat many times (indicating a stuck model)
    // Use larger window sizes and higher thresholds to avoid false positives
    let char_count = text.chars().count();
    let check_sizes = [50usize, 80, 100, 150];

    for &window_size in &check_sizes {
        if char_count > window_size * 8 {
            // Sample multiple positions throughout the text
            let sample_count = 8.min(char_count / window_size);
            for i in 0..sample_count {
                let char_start = (char_count / sample_count) * i;
                let char_end = char_start + window_size;

                if let Some(sample) = safe_char_slice(text, char_start, char_end) {
                    // Skip if sample is mostly whitespace
                    let non_ws_count = sample.chars().filter(|c| !c.is_whitespace()).count();
                    if non_ws_count < window_size / 3 {
                        continue;
                    }

                    // Skip samples that look like common structural patterns
                    // (markdown headers, table separators, list markers, etc.)
                    let trimmed = sample.trim();
                    if trimmed.starts_with('#')
                        || trimmed.starts_with('|')
                        || trimmed.starts_with('-')
                        || trimmed.starts_with('*')
                        || trimmed.starts_with('>')
                    {
                        continue;
                    }

                    let count = text.matches(sample).count();
                    // Require at least 5 repetitions (was 3) to avoid false positives
                    if count >= 5 {
                        // Found likely repetition - but keep more content
                        if let Some(first_byte_pos) = text.find(sample) {
                            // Find character position of first occurrence
                            let first_char_pos = text[..first_byte_pos].chars().count();
                            // Keep more content: first occurrence + 4x window size (was 2x)
                            let truncate_at = first_char_pos + window_size * 4;
                            let truncated = safe_truncate_chars(text, truncate_at);
                            return format!(
                                "{}\n\n[Note: Repetitive content detected and truncated]",
                                truncated
                            );
                        }
                    }
                }
            }
        }
    }

    // Strategy 2: Look for consecutive identical lines (fallback)
    // Require more consecutive lines and longer content to avoid false positives
    let lines: Vec<&str> = text.lines().collect();
    let mut seen_consecutive = 0;
    let mut last_line = "";
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        // Use char count instead of byte length for comparison
        // Increased minimum line length to 20 chars (was 10)
        if trimmed.is_empty() || trimmed.chars().count() < 20 {
            continue;
        }
        // Skip common structural patterns that legitimately repeat
        if trimmed.starts_with('#')
            || trimmed.starts_with('|')
            || trimmed.starts_with('-')
            || trimmed.starts_with('*')
            || trimmed.starts_with('>')
            || trimmed.chars().all(|c| c == '-' || c == '=' || c == '_')
        {
            continue;
        }
        if trimmed == last_line {
            seen_consecutive += 1;
            // Increased threshold from 4 to 6 consecutive identical lines
            if seen_consecutive >= 6 {
                // Keep the first occurrence of the repeated content
                let truncate_at = i.saturating_sub(seen_consecutive - 1);
                let truncated: Vec<&str> = lines[..truncate_at].to_vec();
                return format!(
                    "{}\n\n[Note: Repetitive content detected and truncated]",
                    truncated.join("\n")
                );
            }
        } else {
            seen_consecutive = 1;
            last_line = trimmed;
        }
    }

    // Strategy 3: Check if output is suspiciously long for a single page
    // Normal pages are typically 500-3000 words; but some documents can be longer
    // Increased threshold significantly to avoid truncating legitimate dense content
    let word_count = text.split_whitespace().count();
    if word_count > 10000 {
        // Truncate to approximately 8000 words (was 3000)
        let words: Vec<&str> = text.split_whitespace().take(8000).collect();
        let truncated = words.join(" ");
        return format!(
            "{}\n\n[Note: Output truncated due to excessive length - possible repetition]",
            truncated
        );
    }

    text.to_string()
}

/// Parse MIME type string to rig ImageMediaType.
/// Returns None for unsupported formats (e.g., EMF, WMF) that would cause API failures.
fn parse_mime_to_image_type(mime_type: &str) -> Option<ImageMediaType> {
    match mime_type.to_lowercase().as_str() {
        "image/jpeg" | "image/jpg" => Some(ImageMediaType::JPEG),
        "image/png" => Some(ImageMediaType::PNG),
        "image/gif" => Some(ImageMediaType::GIF),
        "image/webp" => Some(ImageMediaType::WEBP),
        _ => None, // Unsupported formats like EMF/WMF return None
    }
}

/// Check if a MIME type can be converted to a supported format.
fn is_convertible_format(mime_type: &str) -> bool {
    matches!(
        mime_type.to_lowercase().as_str(),
        "image/bmp" | "image/tiff" | "image/x-tiff" | "image/x-ms-bmp"
    )
}

/// Check if a MIME type is a legacy Windows metafile format that cannot be converted.
fn is_legacy_metafile(mime_type: &str) -> bool {
    matches!(
        mime_type.to_lowercase().as_str(),
        "image/emf"
            | "image/wmf"
            | "image/x-emf"
            | "image/x-wmf"
            | "application/x-msmetafile"
            | "application/emf"
            | "application/wmf"
    )
}

/// Convert an image from a legacy format (BMP, TIFF) to PNG bytes.
/// Returns None if conversion fails or format is not supported.
fn convert_to_png(image_data: &[u8], mime_type: &str) -> Option<Vec<u8>> {
    let format = match mime_type.to_lowercase().as_str() {
        "image/bmp" | "image/x-ms-bmp" => ImageFormat::Bmp,
        "image/tiff" | "image/x-tiff" => ImageFormat::Tiff,
        _ => return None,
    };

    // Try to load the image
    let img = image::load_from_memory_with_format(image_data, format).ok()?;

    // Convert to PNG
    let mut png_bytes = Vec::new();
    let mut cursor = Cursor::new(&mut png_bytes);
    img.write_to(&mut cursor, ImageFormat::Png).ok()?;

    Some(png_bytes)
}

/// Result of attempting to prepare an image for LLM API.
enum PreparedImage {
    /// Image is ready to use with the given type
    Ready(ImageMediaType),
    /// Image was converted to PNG, new data provided
    Converted(Vec<u8>),
    /// Image format is unsupported and cannot be converted
    Unsupported(String),
}

/// Prepare image data for LLM API, converting if necessary.
fn prepare_image_for_llm(image_data: &[u8], mime_type: &str) -> PreparedImage {
    // First, check if it's already a supported format
    if let Some(media_type) = parse_mime_to_image_type(mime_type) {
        return PreparedImage::Ready(media_type);
    }

    // Check if it's a legacy metafile format (EMF/WMF) - these cannot be converted
    if is_legacy_metafile(mime_type) {
        return PreparedImage::Unsupported(format!(
            "Windows Metafile format ({}) cannot be converted - these are vector graphics formats with no pure Rust support",
            mime_type
        ));
    }

    // Try to convert convertible formats (BMP, TIFF)
    if is_convertible_format(mime_type) {
        if let Some(png_data) = convert_to_png(image_data, mime_type) {
            return PreparedImage::Converted(png_data);
        } else {
            return PreparedImage::Unsupported(format!(
                "Failed to convert {} image to PNG",
                mime_type
            ));
        }
    }

    // Unknown format
    PreparedImage::Unsupported(format!("Unsupported image format: {}", mime_type))
}

#[async_trait]
impl<M: CompletionModel + Send + Sync + 'static> LlmClient for LlmWrapper<M> {
    async fn describe_image(
        &self,
        image_data: &[u8],
        mime_type: &str,
    ) -> Result<String, MarkitdownError> {
        // Prepare the image, converting if necessary
        let (base64_data, image_type) = match prepare_image_for_llm(image_data, mime_type) {
            PreparedImage::Ready(media_type) => (BASE64_STANDARD.encode(image_data), media_type),
            PreparedImage::Converted(png_data) => {
                (BASE64_STANDARD.encode(&png_data), ImageMediaType::PNG)
            }
            PreparedImage::Unsupported(reason) => {
                return Ok(format!("[Image skipped: {}]", reason));
            }
        };

        let mut content = OneOrMany::one(UserContent::image_base64(
            base64_data,
            Some(image_type),
            Some(ImageDetail::Auto),
        ));
        content.push(UserContent::text("Describe this image in detail."));

        let request = self
            .build_request(&self.config.image_description_prompt, content)
            .build();

        self.send_request(request).await
    }

    async fn describe_image_base64(
        &self,
        base64_data: &str,
        mime_type: &str,
    ) -> Result<String, MarkitdownError> {
        // Note: For base64 input, we cannot convert - caller must provide supported format
        let image_type = match parse_mime_to_image_type(mime_type) {
            Some(t) => t,
            None => {
                // Try to decode and convert if it's a convertible format
                if is_convertible_format(mime_type) {
                    if let Ok(decoded) = BASE64_STANDARD.decode(base64_data) {
                        if let Some(png_data) = convert_to_png(&decoded, mime_type) {
                            let png_base64 = BASE64_STANDARD.encode(&png_data);
                            let mut content = OneOrMany::one(UserContent::image_base64(
                                png_base64,
                                Some(ImageMediaType::PNG),
                                Some(ImageDetail::Auto),
                            ));
                            content.push(UserContent::text("Describe this image in detail."));

                            let request = self
                                .build_request(&self.config.image_description_prompt, content)
                                .build();

                            return self.send_request(request).await;
                        }
                    }
                }

                if is_legacy_metafile(mime_type) {
                    return Ok(format!(
                        "[Image skipped: Windows Metafile format ({}) cannot be converted]",
                        mime_type
                    ));
                }

                return Ok(format!("[Image skipped: Unsupported format {}]", mime_type));
            }
        };

        let mut content = OneOrMany::one(UserContent::image_base64(
            base64_data.to_string(),
            Some(image_type),
            Some(ImageDetail::Auto),
        ));
        content.push(UserContent::text("Describe this image in detail."));

        let request = self
            .build_request(&self.config.image_description_prompt, content)
            .build();

        self.send_request(request).await
    }

    async fn describe_images_batch(
        &self,
        images: &[(&[u8], &str)],
    ) -> Result<Vec<String>, MarkitdownError> {
        if images.is_empty() {
            return Ok(Vec::new());
        }

        let batch_size = self.config.images_per_message;

        if batch_size == 1 {
            // Process one at a time in parallel
            let futures: Vec<_> = images
                .iter()
                .map(|(data, mime)| self.describe_image(data, mime))
                .collect();

            let results = join_all(futures).await;
            results.into_iter().collect()
        } else {
            // Process in batches
            let mut all_descriptions = Vec::with_capacity(images.len());

            for chunk in images.chunks(batch_size) {
                if chunk.len() == 1 {
                    // Single image, use regular method
                    let desc = self.describe_image(chunk[0].0, chunk[0].1).await?;
                    all_descriptions.push(desc);
                } else {
                    // Multiple images in one request
                    let mut content =
                        OneOrMany::one(UserContent::text(&self.config.batch_image_prompt));

                    let mut skipped_descriptions = Vec::new();
                    let mut processed_count = 0;

                    for (i, (data, mime)) in chunk.iter().enumerate() {
                        // Prepare the image, converting if necessary
                        let (base64_data, image_type) = match prepare_image_for_llm(data, mime) {
                            PreparedImage::Ready(media_type) => {
                                (BASE64_STANDARD.encode(data), media_type)
                            }
                            PreparedImage::Converted(png_data) => {
                                (BASE64_STANDARD.encode(&png_data), ImageMediaType::PNG)
                            }
                            PreparedImage::Unsupported(reason) => {
                                skipped_descriptions
                                    .push((i, format!("[Image skipped: {}]", reason)));
                                continue;
                            }
                        };

                        processed_count += 1;
                        content.push(UserContent::text(format!(
                            "\n--- Image {} ---",
                            processed_count
                        )));
                        content.push(UserContent::image_base64(
                            base64_data,
                            Some(image_type),
                            Some(ImageDetail::Auto),
                        ));
                    }

                    // If all images in this chunk were skipped, add placeholders and continue
                    if processed_count == 0 {
                        for (_, desc) in skipped_descriptions {
                            all_descriptions.push(desc);
                        }
                        continue;
                    }

                    let request = self
                        .build_request(&self.config.batch_image_prompt, content)
                        .build();

                    let response = self.send_request(request).await?;

                    // Parse the response to extract individual descriptions
                    let mut descriptions = parse_batch_response(&response, processed_count);

                    // Insert skipped placeholders at their original positions
                    for (original_idx, placeholder) in skipped_descriptions {
                        let insert_pos = original_idx.min(descriptions.len());
                        descriptions.insert(insert_pos, placeholder);
                    }

                    all_descriptions.extend(descriptions);
                }
            }

            Ok(all_descriptions)
        }
    }

    async fn convert_page_image(
        &self,
        image_data: &[u8],
        mime_type: &str,
    ) -> Result<String, MarkitdownError> {
        // Prepare the image, converting if necessary
        let (base64_data, image_type) = match prepare_image_for_llm(image_data, mime_type) {
            PreparedImage::Ready(media_type) => (BASE64_STANDARD.encode(image_data), media_type),
            PreparedImage::Converted(png_data) => {
                (BASE64_STANDARD.encode(&png_data), ImageMediaType::PNG)
            }
            PreparedImage::Unsupported(reason) => {
                return Err(MarkitdownError::LlmError(format!(
                    "Cannot convert page image: {}",
                    reason
                )));
            }
        };

        let mut content = OneOrMany::one(UserContent::image_base64(
            base64_data,
            Some(image_type),
            Some(ImageDetail::High), // Use high detail for page conversion
        ));
        content.push(UserContent::text(
            "Convert this page to markdown. Output only the content, no commentary.",
        ));

        let request = self
            .build_request(&self.config.page_conversion_prompt, content)
            .build();

        self.send_request(request).await
    }

    async fn convert_page_images_batch(&self, pages: &[(&[u8], &str)]) -> Vec<Option<String>> {
        if pages.is_empty() {
            return Vec::new();
        }

        // Process pages in parallel, respecting pages_per_batch limit
        let batch_size = self.config.pages_per_batch;
        let mut all_results = Vec::with_capacity(pages.len());

        for chunk in pages.chunks(batch_size) {
            let futures: Vec<_> = chunk
                .iter()
                .map(|(data, mime)| async move {
                    match self.convert_page_image(data, mime).await {
                        Ok(markdown) if !markdown.trim().is_empty() => Some(markdown),
                        _ => None,
                    }
                })
                .collect();

            let results = join_all(futures).await;
            all_results.extend(results);
        }

        all_results
    }

    async fn complete(&self, prompt: &str) -> Result<String, MarkitdownError> {
        let content = OneOrMany::one(UserContent::text(prompt));
        let request = self
            .build_request("You are a helpful assistant.", content)
            .build();

        self.send_request(request).await
    }

    fn config(&self) -> &LlmConfig {
        &self.config
    }

    async fn describe_extracted_images(
        &self,
        images: &[&ExtractedImage],
    ) -> Result<Vec<String>, MarkitdownError> {
        if images.is_empty() {
            return Ok(Vec::new());
        }

        let batch_size = self.config.images_per_message;

        if batch_size == 1 {
            self.describe_extracted_images_individual(images).await
        } else {
            self.describe_extracted_images_batched(images).await
        }
    }
}

/// Parse a batch response to extract individual image descriptions
fn parse_batch_response(response: &str, expected_count: usize) -> Vec<String> {
    let descriptions = parse_batch_response_with_headers(response, expected_count)
        .or_else(|| parse_batch_response_with_separators(response, expected_count))
        .unwrap_or_else(|| vec![response.to_string(); expected_count]);

    normalize_batch_descriptions(descriptions, response, expected_count)
}

fn parse_batch_response_with_headers(response: &str, expected_count: usize) -> Option<Vec<String>> {
    let parts: Vec<&str> = response.split("## Image").collect();
    if parts.len() <= 1 {
        return None;
    }

    let mut descriptions = Vec::with_capacity(expected_count);
    for part in parts.iter().skip(1) {
        let content = part
            .trim()
            .split_once('\n')
            .map(|x| x.1)
            .unwrap_or(part.trim())
            .trim();
        if !content.is_empty() {
            descriptions.push(content.to_string());
        }
    }

    if descriptions.is_empty() {
        None
    } else {
        Some(descriptions)
    }
}

fn parse_batch_response_with_separators(
    response: &str,
    expected_count: usize,
) -> Option<Vec<String>> {
    let mut descriptions = Vec::with_capacity(expected_count);
    for part in response.split("---") {
        let trimmed = part.trim();
        if !trimmed.is_empty() && !trimmed.starts_with("Image") {
            descriptions.push(trimmed.to_string());
        }
    }

    if descriptions.is_empty() {
        None
    } else {
        Some(descriptions)
    }
}

fn normalize_batch_descriptions(
    mut descriptions: Vec<String>,
    response: &str,
    expected_count: usize,
) -> Vec<String> {
    if descriptions.len() < expected_count {
        return vec![response.to_string(); expected_count];
    }

    if descriptions.len() > expected_count {
        descriptions.truncate(expected_count);
    }

    descriptions
}

/// A mock LLM client for testing
pub struct MockLlmClient {
    pub image_response: String,
    pub text_response: String,
    config: LlmConfig,
}

impl MockLlmClient {
    pub fn new() -> Self {
        Self {
            image_response: "A mock image description".to_string(),
            text_response: "A mock text response".to_string(),
            config: LlmConfig::default(),
        }
    }

    pub fn with_image_response(mut self, response: impl Into<String>) -> Self {
        self.image_response = response.into();
        self
    }

    pub fn with_text_response(mut self, response: impl Into<String>) -> Self {
        self.text_response = response.into();
        self
    }

    pub fn with_config(mut self, config: LlmConfig) -> Self {
        self.config = config;
        self
    }
}

impl Default for MockLlmClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LlmClient for MockLlmClient {
    async fn describe_image(
        &self,
        _image_data: &[u8],
        _mime_type: &str,
    ) -> Result<String, MarkitdownError> {
        Ok(self.image_response.clone())
    }

    async fn describe_image_base64(
        &self,
        _base64_data: &str,
        _mime_type: &str,
    ) -> Result<String, MarkitdownError> {
        Ok(self.image_response.clone())
    }

    async fn describe_images_batch(
        &self,
        images: &[(&[u8], &str)],
    ) -> Result<Vec<String>, MarkitdownError> {
        Ok(vec![self.image_response.clone(); images.len()])
    }

    async fn convert_page_image(
        &self,
        _image_data: &[u8],
        _mime_type: &str,
    ) -> Result<String, MarkitdownError> {
        Ok(self.text_response.clone())
    }

    async fn convert_page_images_batch(&self, pages: &[(&[u8], &str)]) -> Vec<Option<String>> {
        vec![Some(self.text_response.clone()); pages.len()]
    }

    async fn complete(&self, _prompt: &str) -> Result<String, MarkitdownError> {
        Ok(self.text_response.clone())
    }

    fn config(&self) -> &LlmConfig {
        &self.config
    }
}

/// Type alias for a boxed LLM client that can be shared across threads
pub type SharedLlmClient = Arc<dyn LlmClient>;

/// Helper function to create a shared LLM client from any CompletionModel
pub fn create_llm_client<M: CompletionModel + Send + Sync + 'static>(model: M) -> SharedLlmClient {
    Arc::new(LlmWrapper::new(model))
}

/// Helper function to create a shared LLM client with custom config
pub fn create_llm_client_with_config<M: CompletionModel + Send + Sync + 'static>(
    model: M,
    config: LlmConfig,
) -> SharedLlmClient {
    Arc::new(LlmWrapper::with_config(model, config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mime_to_image_type() {
        assert!(matches!(
            parse_mime_to_image_type("image/png"),
            Some(ImageMediaType::PNG)
        ));
        assert!(matches!(
            parse_mime_to_image_type("image/jpeg"),
            Some(ImageMediaType::JPEG)
        ));
        assert!(matches!(
            parse_mime_to_image_type("image/jpg"),
            Some(ImageMediaType::JPEG)
        ));
        assert!(matches!(
            parse_mime_to_image_type("image/gif"),
            Some(ImageMediaType::GIF)
        ));
        assert!(matches!(
            parse_mime_to_image_type("image/webp"),
            Some(ImageMediaType::WEBP)
        ));
        // Unsupported formats return None
        assert!(parse_mime_to_image_type("image/bmp").is_none());
        assert!(parse_mime_to_image_type("image/tiff").is_none());
        assert!(parse_mime_to_image_type("image/emf").is_none());
        assert!(parse_mime_to_image_type("image/wmf").is_none());
    }

    #[test]
    fn test_is_convertible_format() {
        assert!(is_convertible_format("image/bmp"));
        assert!(is_convertible_format("image/tiff"));
        assert!(is_convertible_format("image/x-tiff"));
        assert!(is_convertible_format("image/x-ms-bmp"));
        // Not convertible
        assert!(!is_convertible_format("image/emf"));
        assert!(!is_convertible_format("image/wmf"));
        assert!(!is_convertible_format("image/png"));
    }

    #[test]
    fn test_is_legacy_metafile() {
        assert!(is_legacy_metafile("image/emf"));
        assert!(is_legacy_metafile("image/wmf"));
        assert!(is_legacy_metafile("image/x-emf"));
        assert!(is_legacy_metafile("image/x-wmf"));
        assert!(is_legacy_metafile("application/x-msmetafile"));
        // Not metafiles
        assert!(!is_legacy_metafile("image/png"));
        assert!(!is_legacy_metafile("image/bmp"));
    }

    #[test]
    fn test_prepare_image_for_llm_supported_format() {
        // PNG is directly supported
        let png_header = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        match prepare_image_for_llm(&png_header, "image/png") {
            PreparedImage::Ready(ImageMediaType::PNG) => {}
            _ => panic!("Expected Ready(PNG)"),
        }
    }

    #[test]
    fn test_prepare_image_for_llm_metafile() {
        // EMF/WMF should return Unsupported with descriptive message
        let dummy_data = [0x01, 0x00, 0x00, 0x00];
        match prepare_image_for_llm(&dummy_data, "image/emf") {
            PreparedImage::Unsupported(msg) => {
                assert!(msg.contains("Windows Metafile"));
                assert!(msg.contains("cannot be converted"));
            }
            _ => panic!("Expected Unsupported for EMF"),
        }

        match prepare_image_for_llm(&dummy_data, "image/wmf") {
            PreparedImage::Unsupported(msg) => {
                assert!(msg.contains("Windows Metafile"));
            }
            _ => panic!("Expected Unsupported for WMF"),
        }
    }

    #[test]
    fn test_prepare_image_for_llm_unknown_format() {
        let dummy_data = [0x00, 0x00, 0x00, 0x00];
        match prepare_image_for_llm(&dummy_data, "image/unknown") {
            PreparedImage::Unsupported(msg) => {
                assert!(msg.contains("Unsupported image format"));
            }
            _ => panic!("Expected Unsupported for unknown format"),
        }
    }

    #[test]
    fn test_detect_and_truncate_repetition_with_multibyte_chars() {
        // Test with German text containing umlauts (multi-byte UTF-8 characters)
        let text_with_umlauts = "Prüfung bei RT im Neuzustand. ".repeat(20);
        // Should not panic even with multi-byte characters
        let result = detect_and_truncate_repetition(&text_with_umlauts);
        assert!(result.contains("Prüfung"));

        // Test with mixed content including special characters
        let mixed_text = "Hochspannung: 2,1kVAC/48S62Hz gefordert. Größe: 100mm. ".repeat(15);
        let result2 = detect_and_truncate_repetition(&mixed_text);
        assert!(result2.contains("Hochspannung"));

        // Test with Japanese characters (3-byte UTF-8)
        let japanese_text = "これはテストです。".repeat(50);
        let result3 = detect_and_truncate_repetition(&japanese_text);
        assert!(result3.contains("これは"));

        // Test with emoji (4-byte UTF-8)
        let emoji_text = "Hello 👋 World! ".repeat(40);
        let result4 = detect_and_truncate_repetition(&emoji_text);
        assert!(result4.contains("👋"));
    }

    #[test]
    fn test_detect_and_truncate_repetition_short_text() {
        // Short text should be returned as-is
        let short = "Hello world";
        assert_eq!(detect_and_truncate_repetition(short), short);
    }
}
