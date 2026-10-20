//! Summarization mode: page notes and a document summary instead of a full transcription.
//!
//! In extraction mode, every page that needs vision is transcribed by the LLM, so the LLM
//! *generates* roughly as many tokens as the document contains. Output tokens cost several
//! times more than input tokens and dominate latency, so summarization mode turns this
//! around: pages go in as *input* (extracted text, rendered page images, embedded images)
//! and the LLM writes only short notes and the summary.
//!
//! The pipeline follows current long-document summarization research:
//!
//! 1. **Read**: pages are packed into windows (about 10 pages or 16K tokens), and for each
//!    window the LLM writes concise notes per page, most important point first (page
//!    "gists" as in ReadAgent). Notes get shorter per page for long documents so that all
//!    of them fit the final request. With [`SummaryStrategy::MapReduce`] windows are read
//!    concurrently; with [`SummaryStrategy::Sequential`] they are read in order and each
//!    window receives a running summary of everything before it (Chain of Agents).
//! 2. **Reduce**: if the notes still exceed the final request's budget, the least important
//!    points of each page are dropped first. This is deterministic, so it cannot add
//!    hallucinations; only when it is not enough are consecutive notes condensed by the
//!    LLM (hierarchical merging, at most two levels).
//! 3. **Summarize**: one final request writes the summary and key points with page citations.
//!
//! A document that fits into a single window is read and summarized in one request, from
//! the pages themselves. Running headers and footers repeated across pages are removed
//! before reading.
//!
//! The result is a [`Document`] whose pages hold the notes and whose
//! [`summary`](Document::summary) holds the [`DocumentSummary`].

use futures::stream::{self, StreamExt};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Cursor;
use std::sync::OnceLock;
use std::time::Duration;

use crate::error::MarkitdownError;
use crate::llm::{can_send_image, LlmClient, PromptPart, TokenUsage};
use crate::model::{ContentBlock, Document, ExtractedImage, Page};
use crate::prompts::{
    DEFAULT_SUMMARY_DIRECT_PROMPT, DEFAULT_SUMMARY_MERGE_PROMPT, DEFAULT_SUMMARY_NOTES_PROMPT,
    DEFAULT_SUMMARY_PROMPT,
};

/// Rough input-token cost of one image. A document page rendered at ~1,500 px costs
/// about 1,000–1,600 tokens on current OpenAI, Anthropic and Gemini models.
const IMAGE_TOKEN_ESTIMATE: usize = 1_600;

/// Approximate tokens per word, used to turn word budgets into token estimates
const TOKENS_PER_WORD: f64 = 1.4;

/// Text tokens of a dense page; longer segments get a proportionally larger note budget
const TOKENS_PER_PAGE: usize = 800;

/// Shortest per-page note budget, however long the document
const MIN_NOTE_WORDS: usize = 25;

/// Typical words per note bullet and per summary sentence, for count-based length hints
/// (models follow counts more reliably than word targets)
const WORDS_PER_BULLET: usize = 16;
const WORDS_PER_SENTENCE: usize = 20;

/// Embedded images above this size are downscaled before they are sent
const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;

/// Upper bound on LLM merge levels; every level risks compounding hallucinations
const MAX_MERGE_LEVELS: usize = 2;

/// Lines checked at the top and bottom of each page for running headers and footers
const EDGE_LINES: usize = 2;

/// How the windows of a document are read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SummaryStrategy {
    /// Read all windows concurrently, then combine their notes. Fastest; the default.
    #[default]
    MapReduce,
    /// Read windows in document order, handing a running summary from each window to the
    /// next (Chain of Agents). Slower, but notes can resolve references to earlier pages,
    /// which helps narratives and documents that build on themselves.
    Sequential,
}

/// Settings for summarization mode.
#[derive(Debug, Clone)]
pub struct SummaryConfig {
    /// How windows are read
    pub strategy: SummaryStrategy,
    /// What the summary should concentrate on (e.g. "financial risks"); `None` for a
    /// general summary
    pub focus: Option<String>,
    /// Output language (e.g. "English"); `None` writes in the document's language
    pub language: Option<String>,
    /// Target length of the summary in words
    pub summary_words: usize,
    /// Maximum number of key points (0 disables them)
    pub max_key_points: usize,
    /// Longest notes for one page, in words. Long documents get shorter notes per page so
    /// that all notes fit `max_notes_tokens`.
    pub note_words_per_page: usize,
    /// Maximum pages read in one request
    pub max_pages_per_window: usize,
    /// Approximate input-token budget of one reading request
    pub max_window_tokens: usize,
    /// Maximum images (page renders and embedded images) in one reading request
    pub max_images_per_window: usize,
    /// Approximate token budget for all notes read by the final request. Notes above it
    /// are pruned (least important points first) and, if still too long, condensed.
    pub max_notes_tokens: usize,
    /// Send embedded images (figures, photos, slide graphics) to the LLM
    pub include_images: bool,
    /// Embedded images whose width and height are both below this (in pixels) are
    /// skipped as icons or decoration
    pub min_image_size: u32,
    /// Longest edge, in pixels, of rendered PDF pages; larger embedded images are
    /// downscaled to it
    pub page_image_max_edge: u32,
    /// Maximum concurrent LLM requests; `None` uses
    /// [`LlmConfig::pages_per_batch`](crate::LlmConfig)
    pub concurrency: Option<usize>,
    /// Retries for a failed or empty LLM response
    pub max_retries: usize,
    /// Output-token limit per request; `None` uses [`LlmConfig::max_tokens`](crate::LlmConfig).
    /// Leave headroom for reasoning models, whose thinking counts as output.
    pub max_output_tokens: Option<u64>,
    /// System prompt for reading pages into notes
    pub notes_prompt: String,
    /// System prompt for condensing notes into section summaries
    pub merge_prompt: String,
    /// System prompt for the final summary written from notes
    pub summary_prompt: String,
    /// System prompt for documents read and summarized in a single request
    pub direct_prompt: String,
}

impl Default for SummaryConfig {
    fn default() -> Self {
        Self {
            strategy: SummaryStrategy::MapReduce,
            focus: None,
            language: None,
            summary_words: 250,
            max_key_points: 8,
            note_words_per_page: 80,
            max_pages_per_window: 10,
            max_window_tokens: 16_000,
            max_images_per_window: 8,
            max_notes_tokens: 16_000,
            include_images: true,
            min_image_size: 100,
            page_image_max_edge: 1_288,
            concurrency: None,
            max_retries: 1,
            max_output_tokens: None,
            notes_prompt: DEFAULT_SUMMARY_NOTES_PROMPT.to_string(),
            merge_prompt: DEFAULT_SUMMARY_MERGE_PROMPT.to_string(),
            summary_prompt: DEFAULT_SUMMARY_PROMPT.to_string(),
            direct_prompt: DEFAULT_SUMMARY_DIRECT_PROMPT.to_string(),
        }
    }
}

impl SummaryConfig {
    /// Create a config with default settings
    pub fn new() -> Self {
        Self::default()
    }

    /// Set how windows are read
    pub fn with_strategy(mut self, strategy: SummaryStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// Concentrate notes and summary on a topic or question
    pub fn with_focus(mut self, focus: impl Into<String>) -> Self {
        self.focus = Some(focus.into());
        self
    }

    /// Write notes and summary in this language instead of the document's language
    pub fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    /// Set the target summary length in words
    pub fn with_summary_words(mut self, words: usize) -> Self {
        self.summary_words = words.max(20);
        self
    }

    /// Set the maximum number of key points (0 disables them)
    pub fn with_max_key_points(mut self, count: usize) -> Self {
        self.max_key_points = count;
        self
    }

    /// Set the longest notes for one page, in words
    pub fn with_note_words_per_page(mut self, words: usize) -> Self {
        self.note_words_per_page = words.max(10);
        self
    }

    /// Set the maximum pages per reading request
    pub fn with_max_pages_per_window(mut self, pages: usize) -> Self {
        self.max_pages_per_window = pages.max(1);
        self
    }

    /// Set the approximate input-token budget of one reading request
    pub fn with_max_window_tokens(mut self, tokens: usize) -> Self {
        self.max_window_tokens = tokens.max(1_000);
        self
    }

    /// Set the maximum images per reading request
    pub fn with_max_images_per_window(mut self, images: usize) -> Self {
        self.max_images_per_window = images.max(1);
        self
    }

    /// Set the approximate token budget for the notes read by the final request
    pub fn with_max_notes_tokens(mut self, tokens: usize) -> Self {
        self.max_notes_tokens = tokens.max(2_000);
        self
    }

    /// Send embedded images to the LLM (default) or only their alt text
    pub fn with_images(mut self, include: bool) -> Self {
        self.include_images = include;
        self
    }

    /// Skip embedded images whose width and height are both below this size
    pub fn with_min_image_size(mut self, pixels: u32) -> Self {
        self.min_image_size = pixels;
        self
    }

    /// Set the longest edge of rendered PDF pages and downscaled images
    pub fn with_page_image_max_edge(mut self, pixels: u32) -> Self {
        self.page_image_max_edge = pixels.max(256);
        self
    }

    /// Set the maximum number of concurrent LLM requests
    pub fn with_concurrency(mut self, requests: usize) -> Self {
        self.concurrency = Some(requests.max(1));
        self
    }

    /// Set the number of retries for a failed request
    pub fn with_max_retries(mut self, retries: usize) -> Self {
        self.max_retries = retries;
        self
    }

    /// Limit the output tokens of every request
    pub fn with_max_output_tokens(mut self, tokens: Option<u64>) -> Self {
        self.max_output_tokens = tokens;
        self
    }

    /// Set the system prompt for reading pages into notes
    pub fn with_notes_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.notes_prompt = prompt.into();
        self
    }

    /// Set the system prompt for condensing notes
    pub fn with_merge_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.merge_prompt = prompt.into();
        self
    }

    /// Set the system prompt for the final summary written from notes
    pub fn with_summary_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.summary_prompt = prompt.into();
        self
    }

    /// Set the system prompt for documents read and summarized in a single request
    pub fn with_direct_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.direct_prompt = prompt.into();
        self
    }
}

/// Summary of a document produced in summarization mode.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DocumentSummary {
    /// The summary as markdown prose
    pub summary: String,
    /// The most important points, each citing its pages
    pub key_points: Vec<String>,
    /// How the document was read
    pub strategy: SummaryStrategy,
    /// Pages that could not be read completely (a request failed), so the summary may
    /// not cover them
    pub failed_pages: Vec<u32>,
    /// Token usage of all summarization requests
    pub usage: TokenUsage,
}

impl DocumentSummary {
    /// Render the summary and key points as markdown
    pub fn to_markdown(&self) -> String {
        let mut md = format!("## Summary\n\n{}\n", self.summary.trim());

        if !self.key_points.is_empty() {
            md.push_str("\n## Key Points\n\n");
            for point in &self.key_points {
                md.push_str(&format!("- {}\n", point));
            }
        }

        if !self.failed_pages.is_empty() {
            let (pages, them) = if self.failed_pages.len() == 1 {
                ("page", "it")
            } else {
                ("pages", "them")
            };
            md.push_str(&format!(
                "\n> Note: {} {} could not be read completely; the summary may not cover {}.\n",
                pages,
                page_list(&self.failed_pages),
                them
            ));
        }

        md
    }
}

/// Summarize a document: concise notes for every page plus a document summary.
/// See the [module documentation](self) for the pipeline.
pub async fn summarize_document(
    document: &Document,
    llm: &dyn LlmClient,
    config: &SummaryConfig,
) -> Result<Document, MarkitdownError> {
    let boilerplate = repeated_edge_lines(document);
    let segments: Vec<Segment> = document
        .pages
        .iter()
        .flat_map(|page| {
            split_page(
                page.page_number,
                page_items(page, config, &boilerplate),
                config,
            )
        })
        .collect();
    if segments.is_empty() {
        return Err(MarkitdownError::Conversion(
            "Document has no readable content to summarize".to_string(),
        ));
    }

    let page_equivalents = segments.iter().map(Segment::page_equivalents).sum();
    let words_per_page = note_words_per_page(config, page_equivalents);
    let windows = pack_windows(segments, config);
    let info = DocumentInfo::new(document);
    let concurrency = config
        .concurrency
        .unwrap_or_else(|| llm.config().pages_per_batch)
        .max(1);

    let mut usage = TokenUsage::default();
    let mut notes: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    let mut failed_pages: Vec<u32> = Vec::new();
    let mut running_context: Option<String> = None;
    let mut summary = None;

    if windows.len() == 1 {
        // Short document: notes and summary in one request, written from the pages
        let (result, call_usage) =
            read_and_summarize(llm, config, &info, &windows[0], words_per_page).await;
        usage += call_usage;
        let (page_notes, direct_summary) = result?;
        for (page, text) in page_notes {
            notes.entry(page).or_default().push(text);
        }
        summary = direct_summary;
    } else {
        let outcomes: Vec<ReadOutcome> = match config.strategy {
            SummaryStrategy::MapReduce => {
                // Futures are created up front: a reference-taking closure inside the
                // stream would make this future !Send (rustc's higher-ranked closure limits)
                let reads: Vec<_> = windows
                    .iter()
                    .map(|window| read_window(llm, config, &info, window, None, words_per_page))
                    .collect();
                stream::iter(reads).buffered(concurrency).collect().await
            }
            SummaryStrategy::Sequential => {
                let mut outcomes = Vec::with_capacity(windows.len());
                for window in &windows {
                    let context = Some(running_context.as_deref().unwrap_or(""));
                    let outcome =
                        read_window(llm, config, &info, window, context, words_per_page).await;
                    if let Ok(WindowRead {
                        context: Some(updated),
                        ..
                    }) = &outcome.0
                    {
                        running_context = Some(updated.clone());
                    }
                    outcomes.push(outcome);
                }
                outcomes
            }
        };

        let mut last_error = None;
        for (window, (result, window_usage)) in windows.iter().zip(outcomes) {
            usage += window_usage;
            match result {
                Ok(read) => {
                    for (page, text) in read.notes {
                        notes.entry(page).or_default().push(text);
                    }
                }
                Err(e) => {
                    failed_pages.extend(window.page_numbers());
                    last_error = Some(e);
                }
            }
        }
        if notes.is_empty() {
            return Err(last_error.unwrap_or_else(|| {
                MarkitdownError::LlmError("The LLM returned no notes for the document".into())
            }));
        }
    }

    failed_pages.sort_unstable();
    failed_pages.dedup();

    let summary = match summary {
        Some(summary) => summary,
        None => {
            if notes.is_empty() {
                return Err(MarkitdownError::LlmError(
                    "The LLM returned neither notes nor a summary".to_string(),
                ));
            }
            let entries = notes
                .iter()
                .map(|(&page, blocks)| NoteEntry::page(page, blocks.join("\n")))
                .collect();
            let entries = reduce(llm, config, &info, entries, concurrency, &mut usage).await?;
            write_summary(
                llm,
                config,
                &info,
                &entries,
                running_context.as_deref(),
                &failed_pages,
                &mut usage,
            )
            .await?
        }
    };

    let mut summarized = Document::new();
    summarized.title = document.title.clone().or(summary.title);
    summarized.metadata = document.metadata.clone();

    for page in &document.pages {
        let mut notes_page = Page::new(page.page_number);
        if let Some(blocks) = notes.remove(&page.page_number) {
            notes_page.add_content(ContentBlock::Markdown(blocks.join("\n")));
        } else if failed_pages.contains(&page.page_number) {
            notes_page.add_content(ContentBlock::Text(
                "[Notes unavailable: this page could not be read]".to_string(),
            ));
        } else {
            continue;
        }
        summarized.add_page(notes_page);
    }

    summarized.summary = Some(DocumentSummary {
        summary: summary.summary,
        key_points: summary.key_points,
        strategy: config.strategy,
        failed_pages,
        usage,
    });

    Ok(summarized)
}

// ============================================================================
// Reading units
// ============================================================================

/// A page, or part of a page too large for one request, prepared for reading
#[derive(Debug)]
struct Segment {
    page_number: u32,
    /// `(part, total)` when the page had to be split
    part: Option<(usize, usize)>,
    /// Prompt content in reading order
    items: Vec<PromptPart>,
    text_tokens: usize,
    images: usize,
}

impl Segment {
    fn tokens(&self) -> usize {
        self.text_tokens + self.images * IMAGE_TOKEN_ESTIMATE
    }

    /// How many dense pages of text this segment holds, at least one and at most four.
    /// Parts of unpaginated documents (DOCX, HTML) count as several pages.
    fn page_equivalents(&self) -> usize {
        self.text_tokens.div_ceil(TOKENS_PER_PAGE).clamp(1, 4)
    }

    /// Note budget as `(bullets, words)`
    fn note_budget(&self, words_per_page: usize) -> (usize, usize) {
        let words = words_per_page * self.page_equivalents();
        (words.div_ceil(WORDS_PER_BULLET).max(1), words)
    }
}

/// Pages read together in one request
#[derive(Debug, Default)]
struct Window {
    segments: Vec<Segment>,
}

impl Window {
    fn tokens(&self) -> usize {
        self.segments.iter().map(Segment::tokens).sum()
    }

    fn images(&self) -> usize {
        self.segments.iter().map(|s| s.images).sum()
    }

    /// Distinct page numbers in reading order
    fn page_numbers(&self) -> Vec<u32> {
        let mut pages: Vec<u32> = Vec::new();
        for segment in &self.segments {
            if !pages.contains(&segment.page_number) {
                pages.push(segment.page_number);
            }
        }
        pages
    }
}

/// Document-level context shared by every request
struct DocumentInfo {
    title: Option<String>,
    total_pages: usize,
    /// Whether page numbers are meaningful (more than one page)
    paginated: bool,
    /// Headings with their pages, giving concurrently read windows a view of the whole
    outline: Vec<String>,
}

impl DocumentInfo {
    const MAX_OUTLINE_ENTRIES: usize = 40;

    fn new(document: &Document) -> Self {
        let paginated = document.pages.len() > 1;
        let headings: Vec<(u8, &str, u32)> = document
            .pages
            .iter()
            .flat_map(|page| {
                page.content.iter().filter_map(move |block| match block {
                    ContentBlock::Heading { level, text } if !text.trim().is_empty() => {
                        Some((*level, text.trim(), page.page_number))
                    }
                    _ => None,
                })
            })
            .take(Self::MAX_OUTLINE_ENTRIES)
            .collect();

        // Indent relative to the shallowest heading present
        let top_level = headings
            .iter()
            .map(|(level, _, _)| *level)
            .min()
            .unwrap_or(1);
        let outline = headings
            .iter()
            .map(|&(level, text, page)| {
                let indent = "  ".repeat(level.saturating_sub(top_level) as usize);
                if paginated {
                    format!("{}- {} (p. {})", indent, text, page)
                } else {
                    format!("{}- {}", indent, text)
                }
            })
            .collect();

        Self {
            title: document.title.clone().filter(|t| !t.trim().is_empty()),
            total_pages: document.pages.len(),
            paginated,
            outline,
        }
    }

    fn render(&self) -> String {
        let mut out = String::from("<document>\n");
        if let Some(title) = &self.title {
            out.push_str(&format!("Title: {}\n", title));
        }
        if self.paginated {
            out.push_str(&format!("Pages: {}\n", self.total_pages));
        } else {
            out.push_str("Pages: not paginated, so cite no page numbers\n");
        }
        if !self.outline.is_empty() {
            out.push_str("Outline:\n");
            for entry in &self.outline {
                out.push_str(entry);
                out.push('\n');
            }
        }
        out.push_str("</document>\n");
        out
    }
}

/// Text of a page's text and markdown blocks
fn page_text(page: &Page) -> String {
    page.content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) | ContentBlock::Markdown(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Comparison key for header and footer lines: the lowercase line, with digits masked
/// only for page-number lines ("Page 3 of 9" matches "Page 4 of 9"). Other lines must
/// repeat verbatim, so per-page data such as "Total: 4,500" is never treated as a footer.
fn edge_key(line: &str) -> String {
    static PAGE_NUMBER: OnceLock<Regex> = OnceLock::new();
    let page_number = PAGE_NUMBER.get_or_init(|| {
        Regex::new(
            r"^[\s\-–—|•·.,:()\[\]]*(?:(?:page|seite|página|pagina|pag\.?|pg\.?|p\.|s\.)\s*)?#+(?:\s*(?:of|von|/|de|sur|di)\s*#+)?[\s\-–—|•·.,:()\[\]]*$",
        )
        .expect("valid regex")
    });

    let lower = line.trim().to_lowercase();
    let masked: String = lower
        .chars()
        .map(|c| if c.is_ascii_digit() { '#' } else { c })
        .collect();
    if page_number.is_match(&masked) {
        masked
    } else {
        lower
    }
}

/// Indices of the first and of the last [`EDGE_LINES`] non-empty lines (top, bottom)
fn edge_line_indices(lines: &[&str]) -> (Vec<usize>, Vec<usize>) {
    let non_empty: Vec<usize> = (0..lines.len())
        .filter(|&i| !lines[i].trim().is_empty())
        .collect();
    let top = non_empty.iter().take(EDGE_LINES).copied().collect();
    let bottom = non_empty.iter().rev().take(EDGE_LINES).copied().collect();
    (top, bottom)
}

/// Running headers and footers: short lines at the top or bottom of at least half of the
/// pages (and of at least three). Returns their [`edge_key`]s.
fn repeated_edge_lines(document: &Document) -> HashSet<String> {
    if document.pages.len() < 4 {
        return HashSet::new();
    }

    let mut counts: HashMap<String, usize> = HashMap::new();
    for page in &document.pages {
        let text = page_text(page);
        let lines: Vec<&str> = text.lines().collect();
        let (top, bottom) = edge_line_indices(&lines);
        let keys: HashSet<String> = top
            .into_iter()
            .chain(bottom)
            .map(|i| lines[i].trim())
            .filter(|line| line.chars().count() <= 120)
            .map(edge_key)
            .collect();
        for key in keys {
            *counts.entry(key).or_default() += 1;
        }
    }

    let threshold = (document.pages.len() / 2).max(3);
    counts
        .into_iter()
        .filter(|(_, count)| *count >= threshold)
        .map(|(key, _)| key)
        .collect()
}

/// Remove header lines from the top (`start`) and footer lines from the bottom (`end`) of
/// a text block
fn strip_edge_lines(text: &str, boilerplate: &HashSet<String>, start: bool, end: bool) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let (top, bottom) = edge_line_indices(&lines);

    lines
        .iter()
        .enumerate()
        .filter(|&(i, line)| {
            let at_edge = (start && top.contains(&i)) || (end && bottom.contains(&i));
            !(at_edge && boilerplate.contains(&edge_key(line)))
        })
        .map(|(_, line)| *line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Prompt content of a page in reading order: the rendered page image first (models read
/// images best when they precede the related text), then text with embedded images inline.
fn page_items(
    page: &Page,
    config: &SummaryConfig,
    boilerplate: &HashSet<String>,
) -> Vec<PromptPart> {
    let mut items = Vec::new();
    let mut text = String::new();

    let rendered = page
        .rendered_image
        .as_ref()
        .filter(|img| !img.data.is_empty() && can_send_image(&img.mime_type));
    if let Some(render) = rendered {
        let mut part = image_part(render, config.page_image_max_edge);
        if let PromptPart::Image { high_detail, .. } = &mut part {
            *high_detail = true;
        }
        items.push(part);
    }

    let text_blocks: Vec<usize> = page
        .content
        .iter()
        .enumerate()
        .filter(|(_, block)| matches!(block, ContentBlock::Text(_) | ContentBlock::Markdown(_)))
        .map(|(i, _)| i)
        .collect();
    let first_text = text_blocks.first().copied();
    let last_text = text_blocks.last().copied();

    for (i, block) in page.content.iter().enumerate() {
        match block {
            ContentBlock::Text(content) | ContentBlock::Markdown(content)
                if !boilerplate.is_empty() =>
            {
                let stripped = strip_edge_lines(
                    content,
                    boilerplate,
                    Some(i) == first_text,
                    Some(i) == last_text,
                );
                text.push_str(&stripped);
                text.push('\n');
            }
            ContentBlock::Image(img) => {
                if let Some(description) = &img.description {
                    text.push_str(&format!("[Image: {}]\n", description.trim()));
                } else if config.include_images && is_informative_image(img, config) {
                    if let Some(alt) = &img.alt_text {
                        text.push_str(&format!("[Image: {}]\n", alt.trim()));
                    }
                    flush_text(&mut text, &mut items);
                    items.push(image_part(img, config.page_image_max_edge));
                } else if let Some(alt) = &img.alt_text {
                    text.push_str(&format!("[Image: {}]\n", alt.trim()));
                }
            }
            other => text.push_str(&other.to_markdown()),
        }
    }
    flush_text(&mut text, &mut items);

    // Text next to a page render is the page's extracted text layer
    if rendered.is_some() && items.len() > 1 {
        items.insert(1, PromptPart::text("[Extracted text of the page above]"));
    }

    items
}

fn flush_text(text: &mut String, items: &mut Vec<PromptPart>) {
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        items.push(PromptPart::text(trimmed));
    }
    text.clear();
}

/// Whether an embedded image is worth sending: a format the LLM accepts and not a tiny
/// icon. Images of unknown size are kept.
fn is_informative_image(img: &ExtractedImage, config: &SummaryConfig) -> bool {
    if img.data.is_empty() || !can_send_image(&img.mime_type) {
        return false;
    }
    match (img.width, img.height) {
        (Some(w), Some(h)) => w.max(h) >= config.min_image_size,
        _ => true,
    }
}

/// Build an image part, downscaling images that are larger than needed: providers resize
/// big images anyway, so sending them only adds bytes and can exceed request size limits.
fn image_part(img: &ExtractedImage, max_edge: u32) -> PromptPart {
    let dimensions = match (img.width, img.height) {
        (Some(w), Some(h)) => Some((w, h)),
        _ => image::ImageReader::new(Cursor::new(&img.data))
            .with_guessed_format()
            .ok()
            .and_then(|reader| reader.into_dimensions().ok()),
    };
    let oversized = dimensions.is_some_and(|(w, h)| w.max(h) > max_edge);

    if oversized || img.data.len() > MAX_IMAGE_BYTES {
        if let Some((data, mime_type)) = downscale_image(&img.data, &img.mime_type, max_edge) {
            return PromptPart::Image {
                data: data.into(),
                mime_type,
                high_detail: false,
            };
        }
    }

    PromptPart::image(img.data.clone(), img.mime_type.clone())
}

/// Resize an image to fit `max_edge`, as JPEG for photos and PNG otherwise
fn downscale_image(data: &[u8], mime_type: &str, max_edge: u32) -> Option<(Vec<u8>, String)> {
    let image = image::load_from_memory(data).ok()?;
    let resized = if image.width().max(image.height()) > max_edge {
        image.resize(max_edge, max_edge, image::imageops::FilterType::Triangle)
    } else {
        image
    };

    let mut out = Cursor::new(Vec::new());
    let is_photo = matches!(
        mime_type.to_lowercase().as_str(),
        "image/jpeg" | "image/jpg" | "image/webp"
    );
    if is_photo {
        image::DynamicImage::ImageRgb8(resized.to_rgb8())
            .write_to(&mut out, image::ImageFormat::Jpeg)
            .ok()?;
        Some((out.into_inner(), "image/jpeg".to_string()))
    } else {
        resized.write_to(&mut out, image::ImageFormat::Png).ok()?;
        Some((out.into_inner(), "image/png".to_string()))
    }
}

/// Approximate token count of text (about four bytes per token)
fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

fn item_tokens(item: &PromptPart) -> usize {
    match item {
        PromptPart::Text(text) => estimate_tokens(text),
        PromptPart::Image { .. } => IMAGE_TOKEN_ESTIMATE,
    }
}

/// Split a page into segments that each fit one request. Most pages stay whole; large
/// unpaginated documents (DOCX, HTML and Markdown are one "page") and image-heavy pages
/// are cut at headings, paragraphs and between images.
fn split_page(page_number: u32, items: Vec<PromptPart>, config: &SummaryConfig) -> Vec<Segment> {
    if items.is_empty() {
        return Vec::new();
    }

    let token_budget = config.max_window_tokens.max(1_000);
    let image_budget = config.max_images_per_window.max(1);

    let mut parts: Vec<Vec<PromptPart>> = Vec::new();
    let mut current: Vec<PromptPart> = Vec::new();
    let mut current_tokens = 0;
    let mut current_images = 0;

    for item in items {
        let pieces = match item {
            PromptPart::Text(text) => chunk_text(&text, token_budget)
                .into_iter()
                .map(PromptPart::Text)
                .collect(),
            image => vec![image],
        };

        for piece in pieces {
            let is_image = matches!(piece, PromptPart::Image { .. });
            let tokens = item_tokens(&piece);
            let over_budget = current_tokens + tokens > token_budget
                || (is_image && current_images + 1 > image_budget);
            if over_budget && !current.is_empty() {
                parts.push(std::mem::take(&mut current));
                current_tokens = 0;
                current_images = 0;
            }
            current_tokens += tokens;
            current_images += usize::from(is_image);
            current.push(piece);
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }

    let total = parts.len();
    parts
        .into_iter()
        .enumerate()
        .map(|(i, items)| Segment {
            page_number,
            part: (total > 1).then_some((i + 1, total)),
            text_tokens: items
                .iter()
                .filter(|item| matches!(item, PromptPart::Text(_)))
                .map(item_tokens)
                .sum(),
            images: items
                .iter()
                .filter(|item| matches!(item, PromptPart::Image { .. }))
                .count(),
            items,
        })
        .collect()
}

/// Cut text into chunks of at most `max_tokens`, preferring heading, paragraph, line,
/// sentence and word boundaries
fn chunk_text(text: &str, max_tokens: usize) -> Vec<String> {
    let max_bytes = (max_tokens * 4).max(400);
    let mut chunks = Vec::new();
    let mut rest = text.trim();

    while rest.len() > max_bytes {
        let mut limit = max_bytes;
        while !rest.is_char_boundary(limit) {
            limit -= 1;
        }
        let window = &rest[..limit];
        // Only accept a boundary in the second half of the window so chunks stay large
        let min_cut = limit / 2;
        let cut = ["\n#", "\n\n", "\n", ". ", " "]
            .iter()
            .find_map(|sep| {
                window
                    .rfind(sep)
                    // Cut before a heading, after any other separator
                    .map(|pos| {
                        if *sep == "\n#" {
                            pos + 1
                        } else {
                            pos + sep.len()
                        }
                    })
                    .filter(|&pos| pos > min_cut)
            })
            .unwrap_or(limit);

        chunks.push(rest[..cut].trim().to_string());
        rest = rest[cut..].trim_start();
    }
    if !rest.is_empty() {
        chunks.push(rest.to_string());
    }

    chunks.retain(|chunk| !chunk.is_empty());
    chunks
}

/// Pack segments into windows in document order
fn pack_windows(segments: Vec<Segment>, config: &SummaryConfig) -> Vec<Window> {
    let mut windows = Vec::new();
    let mut current = Window::default();

    for segment in segments {
        let full = current.segments.len() >= config.max_pages_per_window.max(1)
            || current.tokens() + segment.tokens() > config.max_window_tokens
            || current.images() + segment.images > config.max_images_per_window.max(1);
        if full && !current.segments.is_empty() {
            windows.push(std::mem::take(&mut current));
        }
        current.segments.push(segment);
    }
    if !current.segments.is_empty() {
        windows.push(current);
    }

    windows
}

/// Note length per page: `note_words_per_page`, reduced for long documents so that all
/// notes fit `max_notes_tokens` (but never below [`MIN_NOTE_WORDS`])
fn note_words_per_page(config: &SummaryConfig, page_equivalents: usize) -> usize {
    let budget_words = config.max_notes_tokens as f64 * 0.8 / TOKENS_PER_WORD;
    let fair_share = (budget_words / page_equivalents.max(1) as f64) as usize;
    fair_share.clamp(
        MIN_NOTE_WORDS.min(config.note_words_per_page),
        config.note_words_per_page,
    )
}

// ============================================================================
// Step 1: reading windows into notes
// ============================================================================

/// Notes read from one window
struct WindowRead {
    /// `(page number, notes)` in reading order
    notes: Vec<(u32, String)>,
    /// Updated running summary (sequential strategy)
    context: Option<String>,
}

/// Result of reading a window plus the tokens it used (also when it failed)
type ReadOutcome = (Result<WindowRead, MarkitdownError>, TokenUsage);

/// Read one window. `context` is `Some` for the sequential strategy: the running summary
/// of the preceding pages (empty for the first window).
async fn read_window(
    llm: &dyn LlmClient,
    config: &SummaryConfig,
    info: &DocumentInfo,
    window: &Window,
    context: Option<&str>,
    words_per_page: usize,
) -> ReadOutcome {
    let pages = window.page_numbers();
    let mut parts = pages_prompt(info, window, context, words_per_page);

    let mut task = format!(
        "\nBased on the pages above, write reading notes for {}.{}",
        pages_label(&pages),
        notes_format(config)
    );
    if context.is_some() {
        let last_page = pages.last().copied().unwrap_or_default();
        task.push_str(&format!(
            "\nAfter the notes, write <context_so_far>...</context_so_far>: an updated summary of \
             the document from its beginning through {} in at most {} words, merging the \
             previous context with these pages, for a reader of the following pages.",
            if info.paginated {
                format!("page {}", last_page)
            } else {
                "this part".to_string()
            },
            context_words(config)
        ));
    }
    parts.push(PromptPart::Text(task));

    let (response, usage) = call_llm(llm, &config.notes_prompt, &parts, config).await;
    let result = response.map(|text| {
        let mut notes = parse_notes(&text, &pages);
        let updated_context = context
            .is_some()
            .then(|| extract_tag(&text, "context_so_far"))
            .flatten();

        if notes.is_empty() {
            // The model ignored the format: keep its notes under the window's first page
            let fallback = strip_tags(&remove_tag(&text, "context_so_far"));
            if !fallback.is_empty() {
                let text = if pages.len() > 1 {
                    format!("(Pages {})\n{}", page_list(&pages), fallback)
                } else {
                    fallback
                };
                notes.push((pages[0], text));
            }
        }

        WindowRead {
            notes,
            context: updated_context,
        }
    });

    (result, usage)
}

/// Read a single-window document and summarize it in the same request. The summary is
/// `None` when the response has notes but no summary (e.g. it was cut off); the caller
/// then writes it from the notes. A response that ignored the format entirely is used
/// as the summary.
async fn read_and_summarize(
    llm: &dyn LlmClient,
    config: &SummaryConfig,
    info: &DocumentInfo,
    window: &Window,
    words_per_page: usize,
) -> (
    Result<(Vec<(u32, String)>, Option<FinalSummary>), MarkitdownError>,
    TokenUsage,
) {
    let pages = window.page_numbers();
    let mut parts = pages_prompt(info, window, None, words_per_page);
    parts.push(PromptPart::Text(format!(
        "\nBased on the pages above, first write reading notes for {}.{}\
         \nThen write the final summary of the whole document:{}",
        pages_label(&pages),
        notes_format(config),
        summary_task(config, info, &[])
    )));

    let (response, usage) = call_llm(llm, &config.direct_prompt, &parts, config).await;
    let result = response.and_then(|text| {
        let notes = parse_notes(&text, &pages);
        if notes.is_empty() || extract_tag(&text, "summary").is_some() {
            let summary = finish_summary(config, &text);
            if summary.summary.is_empty() && notes.is_empty() {
                return Err(MarkitdownError::LlmError(
                    "The LLM returned neither notes nor a summary".to_string(),
                ));
            }
            Ok((notes, Some(summary)))
        } else {
            Ok((notes, None))
        }
    });
    (result, usage)
}

/// Document context and the window's pages, each tagged with its number and note budget
fn pages_prompt(
    info: &DocumentInfo,
    window: &Window,
    context: Option<&str>,
    words_per_page: usize,
) -> Vec<PromptPart> {
    let mut parts = Vec::new();
    let mut header = info.render();
    if let Some(context) = context.filter(|c| !c.trim().is_empty()) {
        header.push_str(&format!(
            "<context_so_far>\n{}\n</context_so_far>\n",
            context.trim()
        ));
    }
    header.push_str("<pages>");
    parts.push(PromptPart::Text(header));

    for segment in &window.segments {
        let part = segment
            .part
            .map(|(i, n)| format!(" part=\"{} of {}\"", i, n))
            .unwrap_or_default();
        let (bullets, words) = segment.note_budget(words_per_page);
        parts.push(PromptPart::Text(format!(
            "<page number=\"{}\"{} note_budget=\"at most {} bullets, ~{} words\">",
            segment.page_number, part, bullets, words
        )));
        parts.extend(segment.items.iter().cloned());
        parts.push(PromptPart::text("</page>"));
    }
    parts.push(PromptPart::text("</pages>"));

    parts
}

fn pages_label(pages: &[u32]) -> String {
    format!(
        "{} {}",
        if pages.len() == 1 { "page" } else { "pages" },
        page_list(pages)
    )
}

/// Format and focus instructions for page notes
fn notes_format(config: &SummaryConfig) -> String {
    let mut format = String::from(
        "\nWrite one <notes page=\"N\">...</notes> block per page, in page order: bullet points, \
         most important first, within the page's note_budget.",
    );
    format.push_str(&language_instruction(config));
    if let Some(focus) = &config.focus {
        format.push_str(&format!(
            "\nThe reader is especially interested in: {}. Give related content more detail \
             and keep other notes brief.",
            focus.trim()
        ));
    }
    format
}

/// Length of the running summary passed between windows in the sequential strategy
fn context_words(config: &SummaryConfig) -> usize {
    config.summary_words.clamp(150, 400)
}

fn language_instruction(config: &SummaryConfig) -> String {
    match &config.language {
        Some(language) => format!("\nWrite in {}.", language.trim()),
        None => "\nWrite in the language of the document.".to_string(),
    }
}

/// Parse `<notes page="N">...</notes>` blocks, keeping pages that belong to the window.
/// Tolerates a missing closing tag (e.g. a response cut off by the token limit).
fn parse_notes(text: &str, pages: &[u32]) -> Vec<(u32, String)> {
    static OPEN: OnceLock<Regex> = OnceLock::new();
    let open = OPEN.get_or_init(|| {
        Regex::new(r#"<notes\b[^>]*?\bpage\s*=\s*["']?(\d+)[^>]*>"#).expect("valid regex")
    });

    let openings: Vec<(usize, usize, u32)> = open
        .captures_iter(text)
        .filter_map(|caps| {
            let whole = caps.get(0)?;
            let page = caps.get(1)?.as_str().parse().ok()?;
            Some((whole.start(), whole.end(), page))
        })
        .collect();

    let mut notes = Vec::new();
    for (i, &(_, body_start, page)) in openings.iter().enumerate() {
        let next_open = openings.get(i + 1).map(|&(start, _, _)| start);
        let body = &text[body_start..];
        let body_end = [
            body.find("</notes>"),
            next_open.map(|start| start - body_start),
            body.find("<context_so_far>"),
            body.find("<title>"),
            body.find("<summary>"),
            body.find("<key_points>"),
        ]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(body.len());

        let content = clean_block(&body[..body_end]);
        if !content.is_empty() && pages.contains(&page) {
            notes.push((page, content));
        }
    }
    notes
}

// ============================================================================
// Step 2: reducing notes to the final budget
// ============================================================================

/// Notes for one page, or a condensed section covering several pages
#[derive(Debug, Clone)]
struct NoteEntry {
    first_page: u32,
    last_page: u32,
    text: String,
    condensed: bool,
}

impl NoteEntry {
    fn page(page: u32, text: String) -> Self {
        Self {
            first_page: page,
            last_page: page,
            text,
            condensed: false,
        }
    }

    fn tokens(&self) -> usize {
        estimate_tokens(&self.text) + 10
    }

    fn render(&self) -> String {
        if self.condensed {
            format!(
                "<section pages=\"{}\">\n{}\n</section>\n",
                page_range(self.first_page, self.last_page),
                self.text.trim()
            )
        } else {
            format!(
                "<notes page=\"{}\">\n{}\n</notes>\n",
                self.first_page,
                self.text.trim()
            )
        }
    }
}

fn total_tokens(entries: &[NoteEntry]) -> usize {
    entries.iter().map(NoteEntry::tokens).sum()
}

/// Fit the notes into the final request: prune the least important points first, then
/// condense with the LLM if still needed
async fn reduce(
    llm: &dyn LlmClient,
    config: &SummaryConfig,
    info: &DocumentInfo,
    mut entries: Vec<NoteEntry>,
    concurrency: usize,
    usage: &mut TokenUsage,
) -> Result<Vec<NoteEntry>, MarkitdownError> {
    if total_tokens(&entries) > config.max_notes_tokens {
        prune_entries(&mut entries, config.max_notes_tokens);
    }

    for _ in 0..MAX_MERGE_LEVELS {
        if total_tokens(&entries) <= config.max_notes_tokens || entries.len() <= 1 {
            break;
        }

        let groups = group_entries(&entries, config.max_notes_tokens);
        let target_words = section_words(config, groups.len());
        let merges: Vec<_> = groups
            .iter()
            .map(|range| merge_group(llm, config, info, &entries[range.clone()], target_words))
            .collect();
        let outcomes: Vec<(Result<String, MarkitdownError>, TokenUsage)> =
            stream::iter(merges).buffered(concurrency).collect().await;

        let mut next = Vec::with_capacity(groups.len());
        for (range, (result, merge_usage)) in groups.into_iter().zip(outcomes) {
            *usage += merge_usage;
            next.push(NoteEntry {
                first_page: entries[range.start].first_page,
                last_page: entries[range.end - 1].last_page,
                text: result?,
                condensed: true,
            });
        }
        entries = next;
    }
    Ok(entries)
}

/// Drop the last bullets of every page (notes are written most important first) until the
/// notes fit `budget` tokens. Every page keeps at least its first bullet.
fn prune_entries(entries: &mut [NoteEntry], budget: usize) {
    let bullets: Vec<Vec<String>> = entries.iter().map(|e| bullet_items(&e.text)).collect();
    let mut ratio = budget as f64 / total_tokens(entries).max(1) as f64;

    loop {
        let pruned: Vec<String> = bullets
            .iter()
            .map(|items| {
                let keep =
                    ((items.len() as f64 * ratio).ceil() as usize).clamp(1, items.len().max(1));
                items
                    .iter()
                    .take(keep)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect();
        let tokens: usize = pruned.iter().map(|t| estimate_tokens(t) + 10).sum();

        if tokens <= budget || ratio < 0.02 {
            for (entry, text) in entries.iter_mut().zip(pruned) {
                entry.text = text;
            }
            return;
        }
        ratio *= 0.85;
    }
}

/// Group consecutive entries so each group fits one merge request
fn group_entries(entries: &[NoteEntry], max_tokens: usize) -> Vec<std::ops::Range<usize>> {
    let mut groups = Vec::new();
    let mut start = 0;
    let mut tokens = 0;

    for (i, entry) in entries.iter().enumerate() {
        if i > start && tokens + entry.tokens() > max_tokens {
            groups.push(start..i);
            start = i;
            tokens = 0;
        }
        tokens += entry.tokens();
    }
    groups.push(start..entries.len());
    groups
}

/// Words per condensed section so that all sections together fit comfortably in the
/// final request
fn section_words(config: &SummaryConfig, sections: usize) -> usize {
    let budget_words = config.max_notes_tokens as f64 * 0.6 / TOKENS_PER_WORD;
    ((budget_words / sections.max(1) as f64) as usize).clamp(120, 600)
}

async fn merge_group(
    llm: &dyn LlmClient,
    config: &SummaryConfig,
    info: &DocumentInfo,
    group: &[NoteEntry],
    target_words: usize,
) -> (Result<String, MarkitdownError>, TokenUsage) {
    let first = group.first().map(|e| e.first_page).unwrap_or_default();
    let last = group.last().map(|e| e.last_page).unwrap_or_default();

    let mut prompt = info.render();
    prompt.push_str("<notes_to_condense>\n");
    for entry in group {
        prompt.push_str(&entry.render());
    }
    prompt.push_str("</notes_to_condense>\n");
    prompt.push_str(&format!(
        "\nBased on the notes above, condense {} into one section summary: at most {} bullet \
         points, about {} words.",
        if info.paginated {
            format!("pages {}", page_range(first, last))
        } else {
            "this part of the document".to_string()
        },
        target_words.div_ceil(WORDS_PER_BULLET),
        target_words
    ));
    prompt.push_str(&language_instruction(config));
    if let Some(focus) = &config.focus {
        prompt.push_str(&format!("\nPrioritize content about: {}.", focus.trim()));
    }

    let (result, usage) = call_llm(
        llm,
        &config.merge_prompt,
        &[PromptPart::Text(prompt)],
        config,
    )
    .await;
    (result.map(|text| strip_tags(&text)), usage)
}

// ============================================================================
// Step 3: final summary
// ============================================================================

#[derive(Debug, Default)]
struct FinalSummary {
    title: Option<String>,
    summary: String,
    key_points: Vec<String>,
}

/// Output instructions for the summary, shared by the final and the single-request path
fn summary_task(config: &SummaryConfig, info: &DocumentInfo, failed_pages: &[u32]) -> String {
    let mut task = String::new();
    if info.title.is_none() {
        task.push_str("\n<title>...</title>: a short, specific title for the document.");
    }
    task.push_str(&format!(
        "\n<summary>...</summary>: about {} words (roughly {} sentences) of prose covering the \
         whole document in proportion to its content, including the middle, not only the \
         beginning and the end.",
        config.summary_words,
        (config.summary_words / WORDS_PER_SENTENCE).max(2)
    ));
    if config.max_key_points > 0 {
        task.push_str(&format!(
            "\n<key_points>...</key_points>: at most {} bullet points, most important first{}.",
            config.max_key_points,
            if info.paginated {
                ", each ending with its page citation such as [p. 4] or [pp. 4-6]"
            } else {
                ""
            }
        ));
    }
    task.push_str(&language_instruction(config));
    if let Some(focus) = &config.focus {
        task.push_str(&format!(
            "\nFocus the summary and key points on: {}. Say so if the document does not cover it.",
            focus.trim()
        ));
    }
    if !failed_pages.is_empty() {
        task.push_str("\nContent of the unread pages is missing from the notes; do not guess it.");
    }
    task
}

async fn write_summary(
    llm: &dyn LlmClient,
    config: &SummaryConfig,
    info: &DocumentInfo,
    entries: &[NoteEntry],
    context: Option<&str>,
    failed_pages: &[u32],
    usage: &mut TokenUsage,
) -> Result<FinalSummary, MarkitdownError> {
    let mut prompt = info.render();
    if let Some(context) = context.filter(|c| !c.trim().is_empty()) {
        prompt.push_str(&format!(
            "<running_summary>\n{}\n</running_summary>\n",
            context.trim()
        ));
    }
    prompt.push_str("<notes>\n");
    for entry in entries {
        prompt.push_str(&entry.render());
    }
    prompt.push_str("</notes>\n");
    if !failed_pages.is_empty() {
        prompt.push_str(&format!(
            "<unread_pages>{}</unread_pages>\n",
            page_list(failed_pages)
        ));
    }
    prompt.push_str("\nBased on the notes above, write the final summary of the whole document:");
    prompt.push_str(&summary_task(config, info, failed_pages));

    let (result, call_usage) = call_llm(
        llm,
        &config.summary_prompt,
        &[PromptPart::Text(prompt)],
        config,
    )
    .await;
    *usage += call_usage;

    Ok(finish_summary(config, &result?))
}

/// Parse the summary response and enforce the key point limit
fn finish_summary(config: &SummaryConfig, text: &str) -> FinalSummary {
    let mut summary = parse_final(text);
    summary.key_points.truncate(config.max_key_points);
    summary
}

/// Parse `<title>`, `<summary>` and `<key_points>`, falling back to the whole text (without
/// other tagged blocks) as the summary when the tags are missing
fn parse_final(text: &str) -> FinalSummary {
    let title = extract_tag(text, "title")
        .map(|t| {
            strip_tags(&t)
                .trim_matches(['"', '*', '#', ' '])
                .to_string()
        })
        .filter(|t| !t.is_empty());
    let key_points = extract_tag(text, "key_points")
        .map(|block| parse_bullets(&block))
        .unwrap_or_default();
    let summary = extract_tag(text, "summary").unwrap_or_else(|| {
        let without = remove_tag(&remove_tag(text, "key_points"), "title");
        strip_tags(&without)
    });

    FinalSummary {
        title,
        summary: clean_block(&summary),
        key_points,
    }
}

/// Bullet (or numbered) items with their continuation lines; text without bullets yields
/// one item per line
fn bullet_items(block: &str) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    for line in block.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match items.last_mut() {
            Some(last) if !is_bullet(line) => {
                last.push('\n');
                last.push_str(line);
            }
            _ => items.push(line.to_string()),
        }
    }
    items
}

fn bullet_marker() -> &'static Regex {
    static MARKER: OnceLock<Regex> = OnceLock::new();
    MARKER.get_or_init(|| Regex::new(r"^\s*(?:[-*•]|\d+[.)])\s+").expect("valid regex"))
}

fn is_bullet(line: &str) -> bool {
    bullet_marker().is_match(line)
}

/// List items without their markers, wrapped lines joined
fn parse_bullets(block: &str) -> Vec<String> {
    bullet_items(block)
        .into_iter()
        .map(|item| {
            let joined = item.lines().map(str::trim).collect::<Vec<_>>().join(" ");
            bullet_marker().replace(&joined, "").trim().to_string()
        })
        .filter(|item| !item.is_empty())
        .collect()
}

// ============================================================================
// Helpers
// ============================================================================

/// Call the LLM, retrying failed or empty responses with exponential backoff.
/// Returns the tokens used by all attempts.
async fn call_llm(
    llm: &dyn LlmClient,
    system_prompt: &str,
    parts: &[PromptPart],
    config: &SummaryConfig,
) -> (Result<String, MarkitdownError>, TokenUsage) {
    let mut usage = TokenUsage::default();
    let mut attempt = 0;

    loop {
        let error = match llm
            .complete_multimodal(system_prompt, parts, config.max_output_tokens)
            .await
        {
            Ok(response) => {
                usage += response.usage;
                if !response.text.trim().is_empty() {
                    return (Ok(response.text), usage);
                }
                MarkitdownError::LlmError("LLM returned an empty response".to_string())
            }
            Err(e) => e,
        };

        if attempt >= config.max_retries {
            return (Err(error), usage);
        }
        attempt += 1;
        tokio::time::sleep(Duration::from_millis(500 << attempt.min(5))).await;
    }
}

/// Content of the first `<tag>...</tag>` (an unclosed tag runs to the end of the text)
fn extract_tag(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = text.find(&open)? + open.len();
    let end = text[start..]
        .find(&close)
        .map(|p| start + p)
        .unwrap_or(text.len());
    let content = text[start..end].trim();
    (!content.is_empty()).then(|| content.to_string())
}

/// Remove a `<tag>...</tag>` block (an unclosed tag runs to the end of the text)
fn remove_tag(text: &str, tag: &str) -> String {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    match text.find(&open) {
        Some(start) => {
            let end = text[start..]
                .find(&close)
                .map(|p| start + p + close.len())
                .unwrap_or(text.len());
            format!("{}{}", &text[..start], &text[end..])
        }
        None => text.to_string(),
    }
}

/// Remove the XML-style tags of the prompt format (keeping their content)
fn strip_tags(text: &str) -> String {
    static TAG: OnceLock<Regex> = OnceLock::new();
    let tag = TAG.get_or_init(|| {
        Regex::new(
            r"</?(?:notes|section|summary|key_points|title|context_so_far|running_summary)\b[^>]*>",
        )
        .expect("valid regex")
    });
    clean_block(&tag.replace_all(text, ""))
}

/// Trim a block and drop a wrapping markdown code fence
fn clean_block(text: &str) -> String {
    let trimmed = text.trim();
    let unfenced = trimmed
        .strip_prefix("```markdown")
        .or_else(|| trimmed.strip_prefix("```md"))
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|rest| rest.trim_end().strip_suffix("```"))
        .unwrap_or(trimmed);
    unfenced.trim().to_string()
}

fn page_range(first: u32, last: u32) -> String {
    if first == last {
        first.to_string()
    } else {
        format!("{}-{}", first, last)
    }
}

/// Compact page list such as "1-3, 5, 8-9"
fn page_list(pages: &[u32]) -> String {
    let mut sorted = pages.to_vec();
    sorted.sort_unstable();
    sorted.dedup();

    let mut ranges: Vec<String> = Vec::new();
    let mut i = 0;
    while i < sorted.len() {
        let start = sorted[i];
        let mut end = start;
        while i + 1 < sorted.len() && sorted[i + 1] == end + 1 {
            i += 1;
            end = sorted[i];
        }
        ranges.push(page_range(start, end));
        i += 1;
    }
    ranges.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::MockLlmClient;

    fn text_page(number: u32, text: &str) -> Page {
        let mut page = Page::new(number);
        page.add_content(ContentBlock::Text(text.to_string()));
        page
    }

    fn texts(items: &[PromptPart]) -> Vec<&str> {
        items
            .iter()
            .filter_map(|item| match item {
                PromptPart::Text(text) => Some(text.as_str()),
                PromptPart::Image { .. } => None,
            })
            .collect()
    }

    #[test]
    fn summarize_future_is_send() {
        fn assert_send<T: Send>(_: &T) {}
        let document = Document::new();
        let llm = MockLlmClient::new();
        let config = SummaryConfig::default();
        let future = summarize_document(&document, &llm, &config);
        assert_send(&future);
    }

    #[test]
    fn parse_notes_reads_blocks_for_window_pages() {
        let text = "<notes page=\"3\">\n- Revenue rose 12% to $4.1M\n</notes>\n\
                    <notes page='4'>- Hiring freeze</notes>\n\
                    <notes page=\"9\">- Not in this window</notes>";
        let notes = parse_notes(text, &[3, 4]);
        assert_eq!(
            notes,
            vec![
                (3, "- Revenue rose 12% to $4.1M".to_string()),
                (4, "- Hiring freeze".to_string())
            ]
        );
    }

    #[test]
    fn parse_notes_tolerates_missing_closing_tags() {
        let text = "<notes page=\"1\">\n- First\n<notes page=\"2\">\n- Second, cut off";
        let notes = parse_notes(text, &[1, 2]);
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0], (1, "- First".to_string()));
        assert_eq!(notes[1], (2, "- Second, cut off".to_string()));

        let with_context =
            "<notes page=\"5\">- Point\n<context_so_far>Story so far</context_so_far>";
        assert_eq!(
            parse_notes(with_context, &[5]),
            vec![(5, "- Point".to_string())]
        );
    }

    #[test]
    fn parse_final_reads_tags_and_bullets() {
        let text = "<title>\"Q3 Report\"</title>\n<summary>\nThe report covers Q3.\n</summary>\n\
                    <key_points>\n- Revenue up [p. 2]\n2. Costs\n   down [p. 3]\n</key_points>";
        let summary = parse_final(text);
        assert_eq!(summary.title.as_deref(), Some("Q3 Report"));
        assert_eq!(summary.summary, "The report covers Q3.");
        assert_eq!(
            summary.key_points,
            vec![
                "Revenue up [p. 2]".to_string(),
                "Costs down [p. 3]".to_string()
            ]
        );
    }

    #[test]
    fn parse_final_falls_back_to_plain_text() {
        let summary = parse_final("```\nJust a plain summary.\n```");
        assert_eq!(summary.summary, "Just a plain summary.");
        assert!(summary.key_points.is_empty());
        assert!(summary.title.is_none());
    }

    #[test]
    fn chunk_text_respects_budget_and_char_boundaries() {
        let paragraph = "Prüfung der Größe läuft. ".repeat(40);
        let text = vec![paragraph.as_str(); 20].join("\n\n");
        let chunks = chunk_text(&text, 1_000);
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|c| c.len() <= 4_000));
        assert_eq!(
            chunks.join(" ").split_whitespace().count(),
            text.split_whitespace().count()
        );
    }

    #[test]
    fn chunk_text_prefers_heading_boundaries() {
        let body = "word ".repeat(300);
        let text = format!("# One\n{body}\n# Two\n{body}\n# Three\n{body}");
        let chunks = chunk_text(&text, 800);
        assert!(chunks.len() >= 2);
        assert!(
            chunks[1].starts_with("# "),
            "chunk starts with: {:?}",
            &chunks[1][..10]
        );
    }

    #[test]
    fn large_page_is_split_into_parts_and_windows() {
        let config = SummaryConfig::default().with_max_window_tokens(2_000);
        let text = "Lorem ipsum dolor sit amet. ".repeat(1_000);
        let segments = split_page(1, vec![PromptPart::text(text)], &config);
        assert!(segments.len() > 1);
        assert_eq!(segments[0].part, Some((1, segments.len())));
        assert!(segments.iter().all(|s| s.tokens() <= 2_000));

        let windows = pack_windows(segments, &config);
        assert!(windows.len() > 1);
        assert!(windows.iter().all(|w| w.page_numbers() == vec![1]));
    }

    #[test]
    fn windows_respect_page_and_image_limits() {
        let config = SummaryConfig::default()
            .with_max_pages_per_window(3)
            .with_max_images_per_window(2);
        let image = || PromptPart::image(bytes::Bytes::from_static(b"png"), "image/png");
        let segments: Vec<Segment> = (1..=7)
            .flat_map(|n| split_page(n, vec![image(), PromptPart::text("Some text")], &config))
            .collect();
        let windows = pack_windows(segments, &config);
        assert!(windows
            .iter()
            .all(|w| w.segments.len() <= 3 && w.images() <= 2));
        let pages: Vec<u32> = windows.iter().flat_map(|w| w.page_numbers()).collect();
        assert_eq!(pages, (1..=7).collect::<Vec<_>>());
    }

    #[test]
    fn running_headers_and_footers_are_removed() {
        let mut document = Document::new();
        for n in 1..=6 {
            document.add_page(text_page(
                n,
                &format!(
                    "ACME Corp Annual Report\nSection {n} body text with details.\n\
                     More findings for section {n}.\nPage {n} of 6"
                ),
            ));
        }
        let boilerplate = repeated_edge_lines(&document);
        assert!(boilerplate.contains(&edge_key("ACME Corp Annual Report")));
        assert!(boilerplate.contains(&edge_key("Page 3 of 6")));
        assert_eq!(
            boilerplate.len(),
            2,
            "templated body lines are not boilerplate"
        );

        let config = SummaryConfig::default();
        let items = page_items(&document.pages[2], &config, &boilerplate);
        let text = texts(&items).join("\n");
        assert!(!text.contains("ACME Corp"));
        assert!(!text.contains("Page 3 of 6"));
        assert!(text.contains("Section 3 body text"));
        assert!(text.contains("More findings for section 3."));
    }

    #[test]
    fn edge_lines_separated_by_blank_lines_are_removed() {
        let mut document = Document::new();
        for n in 1..=5 {
            document.add_page(text_page(
                n,
                &format!("ACME Report\n\n\nBody of page {n}.\n\n\n- {n} -"),
            ));
        }
        let boilerplate = repeated_edge_lines(&document);
        let stripped = strip_edge_lines(&page_text(&document.pages[1]), &boilerplate, true, true);
        assert_eq!(stripped.trim(), "Body of page 2.");

        // Only the requested edge is stripped
        let top_only = strip_edge_lines(&page_text(&document.pages[1]), &boilerplate, true, false);
        assert!(!top_only.contains("ACME Report"));
        assert!(top_only.contains("- 2 -"));
    }

    #[test]
    fn only_page_number_lines_match_across_numbers() {
        assert_eq!(edge_key("Page 3 of 9"), edge_key("Page 4 of 9"));
        assert_eq!(edge_key("- 12 -"), edge_key("- 13 -"));
        assert_eq!(edge_key("Seite 2 von 5"), edge_key("Seite 3 von 5"));
        assert_ne!(edge_key("Total: 4,500"), edge_key("Total: 3,900"));
        assert_ne!(edge_key("Section 3 results"), edge_key("Section 4 results"));
    }

    #[test]
    fn short_documents_keep_their_edge_lines() {
        let mut document = Document::new();
        for n in 1..=3 {
            document.add_page(text_page(n, "Same header\nBody"));
        }
        assert!(repeated_edge_lines(&document).is_empty());
    }

    #[test]
    fn page_items_put_render_first_and_skip_tiny_images() {
        let mut page = Page::new(1);
        let mut render =
            ExtractedImage::new("page_1", bytes::Bytes::from_static(b"png"), "image/png");
        render.width = Some(800);
        render.height = Some(1_000);
        page.rendered_image = Some(render);
        page.add_content(ContentBlock::Text("Extracted text".to_string()));

        let mut icon = ExtractedImage::new("icon", bytes::Bytes::from_static(b"png"), "image/png");
        icon.width = Some(32);
        icon.height = Some(32);
        icon.alt_text = Some("Logo".to_string());
        page.add_content(ContentBlock::Image(icon));

        let emf = ExtractedImage::new("emf", bytes::Bytes::from_static(b"emf"), "image/emf");
        page.add_content(ContentBlock::Image(emf));

        let items = page_items(&page, &SummaryConfig::default(), &HashSet::new());
        assert!(matches!(
            items[0],
            PromptPart::Image {
                high_detail: true,
                ..
            }
        ));
        let images = items
            .iter()
            .filter(|i| matches!(i, PromptPart::Image { .. }))
            .count();
        assert_eq!(images, 1, "only the page render is sent");
        let text = texts(&items).join("\n");
        assert!(text.contains("[Extracted text of the page above]"));
        assert!(text.contains("[Image: Logo]"));
    }

    #[test]
    fn oversized_images_are_downscaled() {
        let image = image::DynamicImage::new_rgb8(3_000, 1_500);
        let mut png = Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let img = ExtractedImage::new("big", bytes::Bytes::from(png.into_inner()), "image/png");

        match image_part(&img, 1_000) {
            PromptPart::Image {
                data, mime_type, ..
            } => {
                assert_eq!(mime_type, "image/png");
                let (w, h) = image::ImageReader::new(Cursor::new(&data))
                    .with_guessed_format()
                    .unwrap()
                    .into_dimensions()
                    .unwrap();
                assert_eq!((w, h), (1_000, 500));
            }
            PromptPart::Text(_) => panic!("expected an image part"),
        }
    }

    #[test]
    fn prune_keeps_most_important_bullets() {
        let mut entries: Vec<NoteEntry> = (1..=20)
            .map(|page| {
                NoteEntry::page(
                    page,
                    (1..=6)
                        .map(|i| format!("- Point {i} on page {page} with some extra detail"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
            })
            .collect();
        let budget = total_tokens(&entries) / 3;
        prune_entries(&mut entries, budget);

        assert!(total_tokens(&entries) <= budget);
        for (i, entry) in entries.iter().enumerate() {
            assert!(entry
                .text
                .starts_with(&format!("- Point 1 on page {}", i + 1)));
            assert!(!entry.text.contains("Point 6"));
        }
    }

    #[test]
    fn note_budget_shrinks_for_long_documents() {
        let config = SummaryConfig::default();
        assert_eq!(note_words_per_page(&config, 10), config.note_words_per_page);
        let long = note_words_per_page(&config, 300);
        assert!(long < config.note_words_per_page && long >= MIN_NOTE_WORDS);
        assert_eq!(note_words_per_page(&config, 100_000), MIN_NOTE_WORDS);
    }

    #[test]
    fn page_list_compacts_ranges() {
        assert_eq!(page_list(&[5, 1, 2, 3, 9, 8]), "1-3, 5, 8-9");
        assert_eq!(page_list(&[4]), "4");
    }

    #[test]
    fn summary_markdown_mentions_unread_pages() {
        let summary = DocumentSummary {
            summary: "A short summary.".to_string(),
            key_points: vec!["First [p. 1]".to_string()],
            failed_pages: vec![2, 3],
            ..DocumentSummary::default()
        };
        let md = summary.to_markdown();
        assert!(md.starts_with("## Summary\n\nA short summary.\n"));
        assert!(md.contains("## Key Points\n\n- First [p. 1]\n"));
        assert!(
            md.contains("pages 2-3 could not be read completely; the summary may not cover them.")
        );
    }
}
