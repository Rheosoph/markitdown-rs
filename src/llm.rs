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
//! let client = openai::Client::from_env().expect("OPENAI_API_KEY must be set");
//! let model = client.completion_model("gpt-4o");
//! let llm = LlmWrapper::new(model);
//! ```

use async_trait::async_trait;
use base64::prelude::*;
use bytes::Bytes;
use futures::future::join_all;
use futures::FutureExt;
use image::ImageFormat;
use rig::{
    completion::{AssistantContent, CompletionModel, CompletionRequest, CompletionRequestBuilder},
    message::{ImageDetail, ImageMediaType, Message, ReasoningContent, UserContent},
    OneOrMany,
};
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// Temperature for LLM responses (0.0 = deterministic, 1.0 = creative).
    /// `None` sends no temperature, so the provider's default applies.
    ///
    /// Some models accept only their default temperature (reasoning models, for example).
    /// When a request is rejected because of its temperature, [`LlmWrapper`] retries it
    /// without one and leaves the temperature out of later requests.
    pub temperature: Option<f64>,
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
            temperature: Some(0.1), // Low temperature for accurate extraction with minimal repetition
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
        self.temperature = Some(temp.clamp(0.0, 2.0));
        self
    }

    /// Send no temperature, so the provider's default applies. Use this for models whose
    /// documentation recommends their default (such as Gemini 3).
    pub fn without_temperature(mut self) -> Self {
        self.temperature = None;
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

/// One piece of a multimodal prompt. Parts are sent in order, so text and images can be
/// interleaved (e.g. a page label, then the page text, then the rendered page image).
#[derive(Debug, Clone)]
pub enum PromptPart {
    /// Plain text (may contain markdown or XML-style tags)
    Text(String),
    /// An image. Formats the provider cannot accept are replaced by a short text placeholder.
    Image {
        data: Bytes,
        mime_type: String,
        /// Request high-detail processing (used for rendered document pages)
        high_detail: bool,
    },
}

impl PromptPart {
    /// Create a text part
    pub fn text(text: impl Into<String>) -> Self {
        PromptPart::Text(text.into())
    }

    /// Create an image part with automatic detail selection
    pub fn image(data: Bytes, mime_type: impl Into<String>) -> Self {
        PromptPart::Image {
            data,
            mime_type: mime_type.into(),
            high_detail: false,
        }
    }
}

/// Token usage reported by the LLM provider, summed over one or more requests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Number of LLM requests made
    pub requests: u64,
    /// Input (prompt) tokens, including image tokens
    pub input_tokens: u64,
    /// Output (completion) tokens
    pub output_tokens: u64,
}

impl TokenUsage {
    /// Input plus output tokens
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

impl std::ops::AddAssign for TokenUsage {
    fn add_assign(&mut self, other: Self) {
        self.requests += other.requests;
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
    }
}

/// Text response of an LLM request together with its token usage.
#[derive(Debug, Clone, Default)]
pub struct LlmResponse {
    pub text: String,
    pub usage: TokenUsage,
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

    /// Run a prompt made of ordered text and image parts under the given system prompt.
    ///
    /// Used by summarization mode to read several pages (text, page renders, embedded
    /// images) in one request. `max_tokens` overrides the configured output limit.
    ///
    /// The default implementation sends only the text parts through [`complete`](Self::complete)
    /// and reports no token counts; clients with vision support should override it.
    async fn complete_multimodal(
        &self,
        system_prompt: &str,
        parts: &[PromptPart],
        _max_tokens: Option<u64>,
    ) -> Result<LlmResponse, MarkitdownError> {
        let mut prompt = system_prompt.to_string();
        for part in parts {
            prompt.push('\n');
            match part {
                PromptPart::Text(text) => prompt.push_str(text),
                PromptPart::Image { .. } => prompt.push_str("[Image omitted]"),
            }
        }
        let text = self.complete(&prompt).await?;
        Ok(LlmResponse {
            text,
            usage: TokenUsage {
                requests: 1,
                ..TokenUsage::default()
            },
        })
    }

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
/// let client = openai::Client::from_env().expect("OPENAI_API_KEY must be set");
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
    /// Set once the model rejected the configured temperature; later requests omit it
    omit_temperature: AtomicBool,
}

impl<M: CompletionModel> LlmWrapper<M> {
    /// Create a new LLM wrapper with default configuration
    pub fn new(model: M) -> Self {
        Self::with_config(model, LlmConfig::default())
    }

    /// Create a new LLM wrapper with custom configuration
    pub fn with_config(model: M, config: LlmConfig) -> Self {
        Self::from_arc(Arc::new(model), config)
    }

    /// Create from an Arc'd model
    pub fn from_arc(model: Arc<M>, config: LlmConfig) -> Self {
        Self {
            model,
            config,
            omit_temperature: AtomicBool::new(false),
        }
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
        let temperature = self
            .config
            .temperature
            .filter(|_| !self.omit_temperature.load(Ordering::Relaxed));
        let mut builder = self
            .model
            .completion_request(system_prompt)
            .messages(vec![Message::User {
                content: user_content,
            }])
            .temperature_opt(temperature);

        if let Some(max_tokens) = self.config.max_tokens {
            builder = builder.max_tokens(max_tokens);
        }

        builder
    }

    /// Send a request and return the raw assistant content with its token usage.
    ///
    /// A request rejected because of its temperature (models that accept only their
    /// default) is retried once without it, and later requests leave it out.
    async fn send_raw(
        &self,
        request: CompletionRequest,
    ) -> Result<(OneOrMany<AssistantContent>, TokenUsage), MarkitdownError> {
        let retry = request.temperature.is_some().then(|| {
            let mut retry = request.clone();
            retry.temperature = None;
            retry
        });

        match (self.complete_once(request).await, retry) {
            (Err(error), Some(retry)) if is_temperature_rejection(&error) => {
                self.omit_temperature.store(true, Ordering::Relaxed);
                self.complete_once(retry).await
            }
            (result, _) => result,
        }
    }

    /// Send a request once.
    /// Uses catch_unwind to handle panics from rig-core on invalid API responses (e.g. 400 errors).
    async fn complete_once(
        &self,
        request: CompletionRequest,
    ) -> Result<(OneOrMany<AssistantContent>, TokenUsage), MarkitdownError> {
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
            .map_err(|e| MarkitdownError::LlmError(format!("LLM error: {}", e)))?;

        let usage = TokenUsage {
            requests: 1,
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
        };
        Ok((response.choice, usage))
    }

    /// Send a request and extract the response text, with post-processing for repetition detection.
    async fn send_request(&self, request: CompletionRequest) -> Result<String, MarkitdownError> {
        let (choice, _) = self.send_raw(request).await?;

        // Apply repetition detection and cleanup
        Ok(detect_and_truncate_repetition(&extract_text_from_response(
            &choice,
        )))
    }

    /// Describe a single ExtractedImage with context-aware prompts.
    ///
    /// An image the provider refuses is sent once more re-encoded. If that is refused too, or
    /// the image cannot be sent at all, the description is a skip note instead of an error, so
    /// one bad image does not fail the document.
    async fn describe_single_extracted_image(
        &self,
        img: &ExtractedImage,
    ) -> Result<String, MarkitdownError>
    where
        M: Send + Sync + 'static,
    {
        let prompt = single_image_prompt(img);

        for encoding in [ImageEncoding::Original, ImageEncoding::Normalized] {
            let image = match image_content(&img.data, &img.mime_type, ImageDetail::Auto, encoding)
            {
                Ok(image) => image,
                Err(reason) => return Ok(skipped_image(&reason)),
            };
            let mut content = OneOrMany::one(image);
            content.push(UserContent::text(prompt.clone()));
            let request = self
                .build_request(&self.config.image_description_prompt, content)
                .build();

            match self.send_request(request).await {
                Err(error) if is_image_rejection(&error) => continue,
                result => return result,
            }
        }

        Ok(skipped_image(UNREADABLE_IMAGE))
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

            // A refused image would fail the whole batch; described one by one, it is the
            // only one that gets a skip note
            match self.describe_extracted_image_batch(chunk).await {
                Err(error) if is_image_rejection(&error) => {
                    all_descriptions.extend(self.describe_extracted_images_individual(chunk).await?)
                }
                result => all_descriptions.extend(result?),
            }
        }

        Ok(all_descriptions)
    }

    /// Describe several images in one request
    async fn describe_extracted_image_batch(
        &self,
        chunk: &[&ExtractedImage],
    ) -> Result<Vec<String>, MarkitdownError>
    where
        M: Send + Sync + 'static,
    {
        let mut content = OneOrMany::one(UserContent::text(&self.config.batch_image_prompt));
        let mut processed_count = 0;
        let mut skipped_descriptions = Vec::new();

        for (i, img) in chunk.iter().enumerate() {
            let image = match image_content(
                &img.data,
                &img.mime_type,
                ImageDetail::Auto,
                ImageEncoding::Original,
            ) {
                Ok(image) => image,
                Err(reason) => {
                    skipped_descriptions.push((i, skipped_image(&reason)));
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
            content.push(image);
        }

        if processed_count == 0 {
            return Ok(skipped_descriptions
                .into_iter()
                .map(|(_, placeholder)| placeholder)
                .collect());
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

        Ok(descriptions)
    }
}

/// Whether an LLM error rejects the temperature parameter, such as OpenAI's "Unsupported
/// value: 'temperature' does not support 0.1 with this model"
fn is_temperature_rejection(error: &MarkitdownError) -> bool {
    error.to_string().to_lowercase().contains("temperature")
}

/// Extract the answer text, ignoring reasoning content unless the model returned nothing else.
fn extract_answer_text(content: &OneOrMany<AssistantContent>) -> String {
    let answer = content
        .iter()
        .filter_map(|c| match c {
            AssistantContent::Text(text) if !text.text.trim().is_empty() => {
                Some(text.text.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    if answer.is_empty() {
        extract_text_from_response(content)
    } else {
        answer
    }
}

/// Extract text content from assistant response
fn extract_text_from_response(content: &OneOrMany<AssistantContent>) -> String {
    content
        .iter()
        .filter_map(|c| match c {
            AssistantContent::Text(text) => Some(text.text.clone()),
            AssistantContent::Reasoning(reasoning) => {
                let text = reasoning
                    .content
                    .iter()
                    .filter_map(|item| match item {
                        ReasoningContent::Text { text, .. } => Some(text.as_str()),
                        ReasoningContent::Summary(summary) => Some(summary.as_str()),
                        ReasoningContent::Encrypted(_) => None,
                        ReasoningContent::Redacted { .. } => None,
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                if text.is_empty() {
                    None
                } else {
                    Some(text)
                }
            }
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

/// Whether an image in this format can be sent to the LLM, directly or after conversion to PNG
pub(crate) fn can_send_image(mime_type: &str) -> bool {
    parse_mime_to_image_type(mime_type).is_some() || is_convertible_format(mime_type)
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

/// Media type providers accept for an image format, as detected from the image bytes
fn sendable_media_type(format: ImageFormat) -> Option<ImageMediaType> {
    match format {
        ImageFormat::Png => Some(ImageMediaType::PNG),
        ImageFormat::Jpeg => Some(ImageMediaType::JPEG),
        ImageFormat::Gif => Some(ImageMediaType::GIF),
        ImageFormat::WebP => Some(ImageMediaType::WEBP),
        _ => None,
    }
}

/// Convert an image in a format providers do not accept (BMP, TIFF) to PNG bytes.
/// Returns None if the image cannot be decoded.
fn convert_to_png(image_data: &[u8], format: ImageFormat) -> Option<Vec<u8>> {
    let img = image::load_from_memory_with_format(image_data, format).ok()?;

    let mut png_bytes = Vec::new();
    let mut cursor = Cursor::new(&mut png_bytes);
    img.write_to(&mut cursor, ImageFormat::Png).ok()?;

    Some(png_bytes)
}

/// Longest edge of a re-encoded image. Providers downscale larger images anyway (Anthropic to
/// 1568 px, OpenAI to 2048 px), and the smaller file stays under per-image size limits.
const NORMALIZED_MAX_EDGE: u32 = 2048;
const NORMALIZED_JPEG_QUALITY: u8 = 90;

/// Decode an image and re-encode it as a plain 8-bit image no larger than
/// [`NORMALIZED_MAX_EDGE`]: PNG when it has transparency, JPEG otherwise. Providers read these
/// even when they refuse the original encoding (CMYK JPEGs, 16-bit PNGs, oversized files).
fn normalize_image(image_data: &[u8]) -> Option<(Vec<u8>, ImageMediaType)> {
    let image = image::load_from_memory(image_data).ok()?;
    let image = if image.width().max(image.height()) > NORMALIZED_MAX_EDGE {
        image.resize(
            NORMALIZED_MAX_EDGE,
            NORMALIZED_MAX_EDGE,
            image::imageops::FilterType::Triangle,
        )
    } else {
        image
    };

    let mut out = Cursor::new(Vec::new());
    if image.color().has_alpha() {
        image::DynamicImage::ImageRgba8(image.to_rgba8())
            .write_to(&mut out, ImageFormat::Png)
            .ok()?;
        Some((out.into_inner(), ImageMediaType::PNG))
    } else {
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, NORMALIZED_JPEG_QUALITY)
            .encode_image(&image::DynamicImage::ImageRgb8(image.to_rgb8()))
            .ok()?;
        Some((out.into_inner(), ImageMediaType::JPEG))
    }
}

/// Result of attempting to prepare an image for LLM API.
enum PreparedImage {
    /// The original bytes can be sent with the given type
    Ready(ImageMediaType),
    /// Re-encoded bytes to send instead of the original
    Converted(Vec<u8>, ImageMediaType),
    /// The image cannot be sent, for the given reason
    Unsupported(String),
}

/// Prepare image data for LLM API, converting if necessary.
///
/// The format is read from the bytes: a declared MIME type comes from a file name or a
/// container entry and is often wrong, and a provider refuses an image whose data does not
/// match the media type it was sent with.
fn prepare_image_for_llm(image_data: &[u8], mime_type: &str) -> PreparedImage {
    if is_legacy_metafile(mime_type) {
        return PreparedImage::Unsupported(format!(
            "Windows Metafile format ({}) cannot be converted - these are vector graphics formats with no pure Rust support",
            mime_type
        ));
    }

    let Ok(format) = image::guess_format(image_data) else {
        return PreparedImage::Unsupported(format!(
            "Unsupported image format: {} (the data is not a recognized image)",
            mime_type
        ));
    };

    if let Some(media_type) = sendable_media_type(format) {
        return PreparedImage::Ready(media_type);
    }

    match convert_to_png(image_data, format) {
        Some(png_data) => PreparedImage::Converted(png_data, ImageMediaType::PNG),
        None => PreparedImage::Unsupported(format!(
            "Failed to convert {} image to PNG",
            format.to_mime_type()
        )),
    }
}

/// Note for an image the provider refused in every encoding
const UNREADABLE_IMAGE: &str = "the model could not read this image";

/// How images are put into a request. When the provider refuses an image, the request is
/// sent again with the next encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageEncoding {
    /// The original bytes in a format providers accept, other formats converted to PNG
    Original,
    /// Decoded and re-encoded with [`normalize_image`]
    Normalized,
    /// Left out, with a text note in its place
    Omitted,
}

fn prepare_image(image_data: &[u8], mime_type: &str, encoding: ImageEncoding) -> PreparedImage {
    if encoding == ImageEncoding::Omitted {
        return PreparedImage::Unsupported(UNREADABLE_IMAGE.to_string());
    }

    match prepare_image_for_llm(image_data, mime_type) {
        PreparedImage::Ready(_) if encoding == ImageEncoding::Normalized => {
            match normalize_image(image_data) {
                Some((data, media_type)) => PreparedImage::Converted(data, media_type),
                None => PreparedImage::Unsupported(
                    "the image data is damaged and could not be decoded".to_string(),
                ),
            }
        }
        prepared => prepared,
    }
}

/// The request content for an image, or the reason it cannot be sent
fn image_content(
    image_data: &[u8],
    mime_type: &str,
    detail: ImageDetail,
    encoding: ImageEncoding,
) -> Result<UserContent, String> {
    let (data, media_type) = match prepare_image(image_data, mime_type, encoding) {
        PreparedImage::Ready(media_type) => (BASE64_STANDARD.encode(image_data), media_type),
        PreparedImage::Converted(data, media_type) => (BASE64_STANDARD.encode(&data), media_type),
        PreparedImage::Unsupported(reason) => return Err(reason),
    };
    Ok(UserContent::image_base64(
        data,
        Some(media_type),
        Some(detail),
    ))
}

fn skipped_image(reason: &str) -> String {
    format!("[Image skipped: {}]", reason)
}

/// Whether the provider refused an image in the request, such as "the image could not be
/// read", "Could not process image", "image exceeds 5 MB maximum" or "Invalid image".
/// Rate limits and quota errors are never treated as a refused image.
fn is_image_rejection(error: &MarkitdownError) -> bool {
    let message = error.to_string().to_lowercase();
    let transient = ["429", "rate limit", "quota", "overloaded"]
        .iter()
        .any(|marker| message.contains(marker));
    let client_error = [
        "400",
        "413",
        "422",
        "bad request",
        "invalid_request",
        "invalid_argument",
        "validationexception",
    ]
    .iter()
    .any(|marker| message.contains(marker));

    !transient && client_error && message.contains("image")
}

/// User content for a multimodal prompt. Adjacent text parts, and notes for images that are
/// not sent, are merged so the provider receives fewer, larger text blocks.
fn multimodal_content(
    parts: &[PromptPart],
    encoding: ImageEncoding,
) -> Result<OneOrMany<UserContent>, MarkitdownError> {
    fn push_line(text: &mut String, line: &str) {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(line);
    }

    let mut contents: Vec<UserContent> = Vec::with_capacity(parts.len());
    let mut pending_text = String::new();

    for part in parts {
        match part {
            PromptPart::Text(text) => push_line(&mut pending_text, text),
            PromptPart::Image {
                data,
                mime_type,
                high_detail,
            } => {
                let detail = if *high_detail {
                    ImageDetail::High
                } else {
                    ImageDetail::Auto
                };
                match image_content(data, mime_type, detail, encoding) {
                    Ok(image) => {
                        if !pending_text.is_empty() {
                            contents.push(UserContent::text(std::mem::take(&mut pending_text)));
                        }
                        contents.push(image);
                    }
                    Err(reason) => push_line(&mut pending_text, &skipped_image(&reason)),
                }
            }
        }
    }
    if !pending_text.is_empty() {
        contents.push(UserContent::text(pending_text));
    }

    OneOrMany::many(contents).map_err(|_| {
        MarkitdownError::LlmError("Multimodal prompt must contain at least one part".into())
    })
}

/// Prompt for describing one image, with what is known about it as context
fn single_image_prompt(img: &ExtractedImage) -> String {
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
    prompt
}

#[async_trait]
impl<M: CompletionModel + Send + Sync + 'static> LlmClient for LlmWrapper<M> {
    async fn describe_image(
        &self,
        image_data: &[u8],
        mime_type: &str,
    ) -> Result<String, MarkitdownError> {
        let image = ExtractedImage::new("image", Bytes::copy_from_slice(image_data), mime_type);
        self.describe_single_extracted_image(&image).await
    }

    async fn describe_image_base64(
        &self,
        base64_data: &str,
        mime_type: &str,
    ) -> Result<String, MarkitdownError> {
        match BASE64_STANDARD.decode(base64_data) {
            Ok(image_data) => self.describe_image(&image_data, mime_type).await,
            Err(_) => Ok(skipped_image("the image data is not valid base64")),
        }
    }

    async fn describe_images_batch(
        &self,
        images: &[(&[u8], &str)],
    ) -> Result<Vec<String>, MarkitdownError> {
        let images: Vec<ExtractedImage> = images
            .iter()
            .enumerate()
            .map(|(i, (data, mime_type))| {
                ExtractedImage::new(
                    format!("image_{}", i + 1),
                    Bytes::copy_from_slice(data),
                    *mime_type,
                )
            })
            .collect();
        let image_refs: Vec<&ExtractedImage> = images.iter().collect();
        self.describe_extracted_images(&image_refs).await
    }

    async fn convert_page_image(
        &self,
        image_data: &[u8],
        mime_type: &str,
    ) -> Result<String, MarkitdownError> {
        let mut rejection = None;
        for encoding in [ImageEncoding::Original, ImageEncoding::Normalized] {
            let image = match image_content(image_data, mime_type, ImageDetail::High, encoding) {
                Ok(image) => image,
                Err(reason) => {
                    return Err(rejection.unwrap_or_else(|| {
                        MarkitdownError::LlmError(format!("Cannot convert page image: {}", reason))
                    }))
                }
            };
            let mut content = OneOrMany::one(image);
            content.push(UserContent::text(
                "Convert this page to markdown. Output only the content, no commentary.",
            ));
            let request = self
                .build_request(&self.config.page_conversion_prompt, content)
                .build();

            match self.send_request(request).await {
                Err(error) if is_image_rejection(&error) => rejection = Some(error),
                result => return result,
            }
        }

        Err(rejection.unwrap_or_else(|| MarkitdownError::LlmError(UNREADABLE_IMAGE.into())))
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

    async fn complete_multimodal(
        &self,
        system_prompt: &str,
        parts: &[PromptPart],
        max_tokens: Option<u64>,
    ) -> Result<LlmResponse, MarkitdownError> {
        let has_images = parts
            .iter()
            .any(|part| matches!(part, PromptPart::Image { .. }));
        let encodings: &[ImageEncoding] = if has_images {
            &[
                ImageEncoding::Original,
                ImageEncoding::Normalized,
                ImageEncoding::Omitted,
            ]
        } else {
            &[ImageEncoding::Original]
        };

        let mut rejection = None;
        for &encoding in encodings {
            let mut builder =
                self.build_request(system_prompt, multimodal_content(parts, encoding)?);
            if let Some(max_tokens) = max_tokens {
                builder = builder.max_tokens(max_tokens);
            }

            match self.send_raw(builder.build()).await {
                Err(error) if is_image_rejection(&error) => rejection = Some(error),
                result => {
                    let (choice, usage) = result?;
                    return Ok(LlmResponse {
                        text: detect_and_truncate_repetition(&extract_answer_text(&choice)),
                        usage,
                    });
                }
            }
        }

        Err(rejection.unwrap_or_else(|| MarkitdownError::LlmError(UNREADABLE_IMAGE.into())))
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

    use rig::completion::Usage;
    use rig::test_utils::{MockCompletionModel, MockTurn};

    const TEMPERATURE_REJECTED: &str = "Unsupported value: 'temperature' does not support 0.1 \
                                        with this model. Only the default (1) value is supported.";

    fn temperatures(model: &MockCompletionModel) -> Vec<Option<f64>> {
        model.requests().iter().map(|r| r.temperature).collect()
    }

    #[tokio::test]
    async fn rejected_temperature_is_retried_without_it_and_then_omitted() {
        let model = MockCompletionModel::new([
            MockTurn::error(TEMPERATURE_REJECTED),
            MockTurn::text("first"),
            MockTurn::text("second"),
        ]);
        let llm = LlmWrapper::new(model.clone());

        assert_eq!(llm.complete("hello").await.unwrap(), "first");
        assert_eq!(llm.complete("again").await.unwrap(), "second");
        assert_eq!(temperatures(&model), vec![Some(0.1), None, None]);
    }

    #[tokio::test]
    async fn other_errors_are_not_retried() {
        let model =
            MockCompletionModel::new([MockTurn::error("rate limit"), MockTurn::text("unused")]);
        let llm = LlmWrapper::new(model.clone());

        assert!(llm.complete("hello").await.is_err());
        assert_eq!(model.request_count(), 1);
    }

    #[tokio::test]
    async fn temperature_can_be_left_to_the_provider() {
        let config = LlmConfig::default().without_temperature();

        let model = MockCompletionModel::new([MockTurn::text("ok")]);
        let llm = LlmWrapper::with_config(model.clone(), config.clone());
        llm.complete("hello").await.unwrap();
        assert_eq!(temperatures(&model), vec![None]);

        // Without a temperature, an error mentioning it is not retried
        let model = MockCompletionModel::new([MockTurn::error(TEMPERATURE_REJECTED)]);
        let llm = LlmWrapper::with_config(model.clone(), config);
        assert!(llm.complete("hello").await.is_err());
        assert_eq!(model.request_count(), 1);
    }

    #[tokio::test]
    async fn complete_multimodal_sends_parts_in_order_and_reports_usage() {
        let mut usage = Usage::new();
        usage.input_tokens = 1_200;
        usage.output_tokens = 85;
        let turn = MockTurn::from_contents([
            AssistantContent::reasoning("thinking that must not leak into the answer"),
            AssistantContent::text("<notes page=\"1\">- A note</notes>"),
        ])
        .unwrap()
        .with_usage(usage);
        let model = MockCompletionModel::new([turn]);
        let llm = LlmWrapper::new(model.clone());

        let parts = vec![
            PromptPart::text("<page number=\"1\">"),
            PromptPart::Image {
                data: Bytes::from(photo(ImageFormat::Png)),
                mime_type: "image/png".to_string(),
                high_detail: true,
            },
            PromptPart::text("Page text"),
            PromptPart::image(Bytes::from_static(b"emf bytes"), "image/emf"),
            PromptPart::text("</page>"),
        ];
        let response = llm
            .complete_multimodal("System rules", &parts, Some(500))
            .await
            .unwrap();

        assert_eq!(response.text, "<notes page=\"1\">- A note</notes>");
        assert_eq!(
            response.usage,
            TokenUsage {
                requests: 1,
                input_tokens: 1_200,
                output_tokens: 85
            }
        );

        let request = &model.requests()[0];
        assert_eq!(request.max_tokens, Some(500));
        let Message::User { content } = request.chat_history.first_ref() else {
            panic!("expected the user content first");
        };
        let content: Vec<&UserContent> = content.iter().collect();
        assert_eq!(content.len(), 3, "adjacent text parts are merged");
        assert!(matches!(content[0], UserContent::Text(t) if t.text == "<page number=\"1\">"));
        assert!(matches!(
            content[1],
            UserContent::Image(image) if image.detail == Some(ImageDetail::High)
        ));
        match content[2] {
            UserContent::Text(t) => {
                assert!(t
                    .text
                    .starts_with("Page text\n[Image skipped: Windows Metafile"));
                assert!(t.text.ends_with("\n</page>"));
            }
            other => panic!("expected merged text, got {:?}", other),
        }
    }

    /// Provider error for an embedded document image the model could not decode
    const IMAGE_UNREADABLE: &str = "HttpError: Invalid status code 400 Bad Request with message: \
        {\"error\": {\"type\": \"invalid_request_error\", \"message\": \"the image could not \
        be read: cannot read the image fdd3aed3c33c164121722afedc3ce15c.img\"}}";

    fn encoded_image(format: ImageFormat, image: image::DynamicImage) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        image.write_to(&mut out, format).unwrap();
        out.into_inner()
    }

    fn photo(format: ImageFormat) -> Vec<u8> {
        encoded_image(format, image::DynamicImage::new_rgb8(8, 8))
    }

    /// Media types of the images in each request
    fn sent_image_types(model: &MockCompletionModel) -> Vec<Vec<Option<ImageMediaType>>> {
        model
            .requests()
            .iter()
            .map(|request| {
                request
                    .chat_history
                    .iter()
                    .flat_map(|message| match message {
                        Message::User { content } => content
                            .iter()
                            .filter_map(|c| match c {
                                UserContent::Image(image) => Some(image.media_type.clone()),
                                _ => None,
                            })
                            .collect(),
                        _ => Vec::new(),
                    })
                    .collect()
            })
            .collect()
    }

    fn sent_text(request: &CompletionRequest) -> String {
        request
            .chat_history
            .iter()
            .flat_map(|message| match message {
                Message::User { content } => content
                    .iter()
                    .filter_map(|c| match c {
                        UserContent::Text(text) => Some(text.text.clone()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn image_type_is_read_from_the_bytes_not_the_declared_mime_type() {
        assert!(matches!(
            prepare_image_for_llm(&photo(ImageFormat::Jpeg), "image/png"),
            PreparedImage::Ready(ImageMediaType::JPEG)
        ));
        assert!(matches!(
            prepare_image_for_llm(&photo(ImageFormat::Bmp), "image/png"),
            PreparedImage::Converted(_, ImageMediaType::PNG)
        ));
        match prepare_image_for_llm(b"not an image at all", "image/jpeg") {
            PreparedImage::Unsupported(reason) => {
                assert!(reason.contains("not a recognized image"), "{}", reason)
            }
            _ => panic!("expected bytes that are no image to be unsupported"),
        }
    }

    #[test]
    fn normalized_images_are_8_bit_and_bounded() {
        let wide_16_bit = encoded_image(
            ImageFormat::Png,
            image::DynamicImage::new_rgb16(3_000, 1_000),
        );
        let (data, media_type) = normalize_image(&wide_16_bit).unwrap();
        assert_eq!(media_type, ImageMediaType::JPEG);
        let decoded = image::load_from_memory(&data).unwrap();
        assert_eq!(decoded.width(), NORMALIZED_MAX_EDGE);
        assert!(decoded.height() < NORMALIZED_MAX_EDGE);

        let transparent = encoded_image(ImageFormat::Png, image::DynamicImage::new_rgba8(4, 4));
        assert!(matches!(
            normalize_image(&transparent),
            Some((_, ImageMediaType::PNG))
        ));
    }

    #[test]
    fn image_rejections_are_told_apart_from_other_errors() {
        let rejected =
            |message: &str| is_image_rejection(&MarkitdownError::LlmError(message.to_string()));

        assert!(rejected(IMAGE_UNREADABLE));
        assert!(rejected(
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"Could not process image"}}"#
        ));
        assert!(rejected(
            r#"{"error":{"code":400,"message":"Unable to process input image.","status":"INVALID_ARGUMENT"}}"#
        ));
        assert!(!rejected(
            "429 Too Many Requests: rate limit reached for image inputs"
        ));
        assert!(!rejected(TEMPERATURE_REJECTED));
        assert!(!rejected("Invalid status code 401 Unauthorized"));
    }

    #[tokio::test]
    async fn refused_image_is_sent_again_re_encoded() {
        let model = MockCompletionModel::new([
            MockTurn::error(IMAGE_UNREADABLE),
            MockTurn::text("A bar chart"),
        ]);
        let llm = LlmWrapper::new(model.clone());
        let png_16_bit = encoded_image(ImageFormat::Png, image::DynamicImage::new_rgb16(8, 8));

        let description = llm.describe_image(&png_16_bit, "image/png").await.unwrap();

        assert_eq!(description, "A bar chart");
        assert_eq!(
            sent_image_types(&model),
            vec![
                vec![Some(ImageMediaType::PNG)],
                vec![Some(ImageMediaType::JPEG)]
            ]
        );
    }

    #[tokio::test]
    async fn image_refused_in_every_encoding_is_skipped_instead_of_failing() {
        let model = MockCompletionModel::new([
            MockTurn::error(IMAGE_UNREADABLE),
            MockTurn::error(IMAGE_UNREADABLE),
        ]);
        let llm = LlmWrapper::new(model.clone());

        let description = llm
            .describe_image(&photo(ImageFormat::Jpeg), "image/jpeg")
            .await
            .unwrap();

        assert_eq!(description, skipped_image(UNREADABLE_IMAGE));
        assert_eq!(model.request_count(), 2);
    }

    #[tokio::test]
    async fn refused_damaged_image_is_skipped_without_a_second_request() {
        let model = MockCompletionModel::new([MockTurn::error(IMAGE_UNREADABLE)]);
        let llm = LlmWrapper::new(model.clone());
        let mut damaged = photo(ImageFormat::Jpeg);
        damaged.truncate(16);

        let description = llm.describe_image(&damaged, "image/jpeg").await.unwrap();

        assert!(
            description.contains("could not be decoded"),
            "{}",
            description
        );
        assert_eq!(model.request_count(), 1);
    }

    #[tokio::test]
    async fn errors_that_are_not_about_the_image_still_fail_the_description() {
        let model = MockCompletionModel::new([MockTurn::error(
            "429 Too Many Requests: rate limit reached for image inputs",
        )]);
        let llm = LlmWrapper::new(model.clone());

        assert!(llm
            .describe_image(&photo(ImageFormat::Jpeg), "image/jpeg")
            .await
            .is_err());
        assert_eq!(model.request_count(), 1);
    }

    #[tokio::test]
    async fn refused_batch_is_described_one_image_at_a_time() {
        let model = MockCompletionModel::new([
            MockTurn::error(IMAGE_UNREADABLE),
            MockTurn::text("A logo"),
            MockTurn::error(IMAGE_UNREADABLE),
            MockTurn::error(IMAGE_UNREADABLE),
        ]);
        let llm = LlmWrapper::with_config(
            model.clone(),
            LlmConfig::default().with_images_per_message(2),
        );
        let readable = photo(ImageFormat::Png);
        let unreadable = photo(ImageFormat::Jpeg);

        let descriptions = llm
            .describe_images_batch(&[(&readable, "image/png"), (&unreadable, "image/jpeg")])
            .await
            .unwrap();

        assert_eq!(
            descriptions,
            vec!["A logo".to_string(), skipped_image(UNREADABLE_IMAGE)]
        );
        assert_eq!(model.request_count(), 4);
    }

    #[tokio::test]
    async fn complete_multimodal_reads_the_text_when_no_image_encoding_is_accepted() {
        let model = MockCompletionModel::new([
            MockTurn::error(IMAGE_UNREADABLE),
            MockTurn::error(IMAGE_UNREADABLE),
            MockTurn::text("<notes page=\"1\">- A note</notes>"),
        ]);
        let llm = LlmWrapper::new(model.clone());
        let parts = vec![
            PromptPart::text("<page number=\"1\">"),
            PromptPart::image(Bytes::from(photo(ImageFormat::Png)), "image/png"),
            PromptPart::text("</page>"),
        ];

        let response = llm
            .complete_multimodal("System rules", &parts, None)
            .await
            .unwrap();

        assert_eq!(response.text, "<notes page=\"1\">- A note</notes>");
        assert_eq!(
            sent_image_types(&model),
            vec![
                vec![Some(ImageMediaType::PNG)],
                vec![Some(ImageMediaType::JPEG)],
                vec![]
            ]
        );
        assert!(sent_text(&model.requests()[2]).contains(&skipped_image(UNREADABLE_IMAGE)));
    }
}
