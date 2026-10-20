//! Summarization mode tests, using a scripted LLM client that records every request and
//! answers in the tag format the pipeline asks for.

use async_trait::async_trait;
use markitdown::{
    error::MarkitdownError, ContentBlock, ConversionOptions, Document, LlmClient, LlmConfig,
    LlmResponse, MarkItDown, MockLlmClient, Page, PromptPart, SummaryConfig, SummaryStrategy,
    TokenUsage, DEFAULT_SUMMARY_DIRECT_PROMPT, DEFAULT_SUMMARY_MERGE_PROMPT,
    DEFAULT_SUMMARY_PROMPT,
};
use regex::Regex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const MULTI_PAGE_PDF: &str = "tests/test_documents/pdfs/multi_page.pdf";
const SCANNED_PDF: &str = "tests/test_documents/pdfs/image_only_german_pdf.pdf";

/// One recorded `complete_multimodal` request
#[derive(Debug, Clone)]
struct Call {
    system: String,
    text: String,
    images: usize,
}

#[derive(Default)]
struct ScriptedLlm {
    calls: Mutex<Vec<Call>>,
    /// Pages transcribed through `convert_page_image(s)` (extraction mode)
    transcriptions: AtomicUsize,
    /// Images described through `describe_*` (extraction mode)
    descriptions: AtomicUsize,
    /// Reading requests containing one of these pages fail
    fail_pages: Vec<u32>,
    /// Write several long bullets per page
    long_notes: bool,
    config: LlmConfig,
}

impl ScriptedLlm {
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn calls_with(&self, system: &str) -> Vec<Call> {
        self.calls()
            .into_iter()
            .filter(|call| call.system == system)
            .collect()
    }

    fn notes_for(&self, page: u32) -> String {
        if self.long_notes {
            (1..=6)
                .map(|i| format!("- Note {} for page {}: {}", i, page, "detail ".repeat(40)))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            format!("- Note for page {}", page)
        }
    }
}

fn page_numbers(text: &str) -> Vec<u32> {
    let re = Regex::new(r#"<page number="(\d+)""#).unwrap();
    let mut pages: Vec<u32> = re
        .captures_iter(text)
        .map(|c| c[1].parse().unwrap())
        .collect();
    pages.dedup();
    pages
}

const SUMMARY_TAGS: &str = "<summary>Scripted summary.</summary>\n\
                            <key_points>\n- First key point [p. 1]\n- Second key point [p. 2]\n</key_points>";

#[async_trait]
impl LlmClient for ScriptedLlm {
    async fn describe_image(&self, _: &[u8], _: &str) -> Result<String, MarkitdownError> {
        self.descriptions.fetch_add(1, Ordering::SeqCst);
        Ok("described".to_string())
    }

    async fn describe_image_base64(&self, _: &str, _: &str) -> Result<String, MarkitdownError> {
        self.descriptions.fetch_add(1, Ordering::SeqCst);
        Ok("described".to_string())
    }

    async fn describe_images_batch(
        &self,
        images: &[(&[u8], &str)],
    ) -> Result<Vec<String>, MarkitdownError> {
        self.descriptions.fetch_add(images.len(), Ordering::SeqCst);
        Ok(vec!["described".to_string(); images.len()])
    }

    async fn convert_page_image(&self, _: &[u8], _: &str) -> Result<String, MarkitdownError> {
        self.transcriptions.fetch_add(1, Ordering::SeqCst);
        Ok("transcribed page".to_string())
    }

    async fn convert_page_images_batch(&self, pages: &[(&[u8], &str)]) -> Vec<Option<String>> {
        self.transcriptions.fetch_add(pages.len(), Ordering::SeqCst);
        vec![Some("transcribed page".to_string()); pages.len()]
    }

    async fn complete(&self, _: &str) -> Result<String, MarkitdownError> {
        Ok("ok".to_string())
    }

    async fn complete_multimodal(
        &self,
        system_prompt: &str,
        parts: &[PromptPart],
        _max_tokens: Option<u64>,
    ) -> Result<LlmResponse, MarkitdownError> {
        let text = parts
            .iter()
            .filter_map(|part| match part {
                PromptPart::Text(text) => Some(text.as_str()),
                PromptPart::Image { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let images = parts
            .iter()
            .filter(|part| matches!(part, PromptPart::Image { .. }))
            .count();
        self.calls.lock().unwrap().push(Call {
            system: system_prompt.to_string(),
            text: text.clone(),
            images,
        });

        let mut response = String::new();
        if system_prompt == DEFAULT_SUMMARY_MERGE_PROMPT {
            response.push_str("- Condensed section point");
        } else if system_prompt == DEFAULT_SUMMARY_PROMPT {
            if text.contains("<title>...</title>:") {
                response.push_str("<title>Scripted Title</title>\n");
            }
            response.push_str(SUMMARY_TAGS);
        } else {
            let pages = page_numbers(&text);
            if pages.iter().any(|page| self.fail_pages.contains(page)) {
                return Err(MarkitdownError::LlmError("scripted failure".to_string()));
            }
            for page in &pages {
                response.push_str(&format!(
                    "<notes page=\"{}\">\n{}\n</notes>\n",
                    page,
                    self.notes_for(*page)
                ));
            }
            if text.contains("write <context_so_far>") {
                response.push_str(&format!(
                    "<context_so_far>Context through page {}</context_so_far>\n",
                    pages.last().unwrap()
                ));
            }
            if system_prompt == DEFAULT_SUMMARY_DIRECT_PROMPT {
                if text.contains("<title>...</title>:") {
                    response.push_str("<title>Scripted Title</title>\n");
                }
                response.push_str(SUMMARY_TAGS);
            }
        }

        Ok(LlmResponse {
            usage: TokenUsage {
                requests: 1,
                input_tokens: (text.len() / 4) as u64,
                output_tokens: (response.len() / 4) as u64,
            },
            text: response,
        })
    }

    fn config(&self) -> &LlmConfig {
        &self.config
    }
}

fn summarize_options(llm: Arc<ScriptedLlm>, config: SummaryConfig) -> ConversionOptions {
    ConversionOptions::default()
        .with_llm(llm)
        .with_summarization(config)
}

fn is_reading_call(call: &Call) -> bool {
    call.system != DEFAULT_SUMMARY_PROMPT && call.system != DEFAULT_SUMMARY_MERGE_PROMPT
}

#[tokio::test]
async fn short_document_is_read_and_summarized_in_one_request() {
    let llm = Arc::new(ScriptedLlm::default());
    let doc = MarkItDown::new()
        .convert(
            "tests/test_documents/pdfs/fake_memo.pdf",
            Some(summarize_options(llm.clone(), SummaryConfig::default())),
        )
        .await
        .expect("summarization succeeds");

    let calls = llm.calls();
    assert_eq!(calls.len(), 1, "one request for a one-page document");
    assert_eq!(calls[0].system, DEFAULT_SUMMARY_DIRECT_PROMPT);

    let summary = doc.summary.as_ref().expect("document has a summary");
    assert_eq!(summary.summary, "Scripted summary.");
    assert_eq!(summary.key_points.len(), 2);
    assert_eq!(summary.usage.requests, 1);
    assert_eq!(doc.title.as_deref(), Some("Scripted Title"));
    assert_eq!(doc.pages.len(), 1);
    assert!(doc.pages[0].to_markdown().contains("Note for page 1"));

    let markdown = doc.to_markdown();
    assert!(markdown.starts_with("# Scripted Title\n\n## Summary\n\nScripted summary."));
    assert!(markdown.contains("## Key Points\n\n- First key point [p. 1]"));
    assert!(markdown.contains("## Notes\n\n- Note for page 1"));
}

#[tokio::test]
async fn multi_window_document_is_read_concurrently_then_summarized() {
    let llm = Arc::new(ScriptedLlm::default());
    let config = SummaryConfig::default().with_max_pages_per_window(2);
    let doc = MarkItDown::new()
        .convert(MULTI_PAGE_PDF, Some(summarize_options(llm.clone(), config)))
        .await
        .expect("summarization succeeds");

    let reads: Vec<Call> = llm.calls().into_iter().filter(is_reading_call).collect();
    assert_eq!(reads.len(), 3, "5 pages in windows of 2");
    assert!(reads
        .iter()
        .all(|call| !call.text.contains("<context_so_far>")));
    assert_eq!(llm.calls_with(DEFAULT_SUMMARY_PROMPT).len(), 1);
    assert_eq!(llm.transcriptions.load(Ordering::SeqCst), 0);

    // The final request reads the notes of every page, not the pages themselves
    let final_call = &llm.calls_with(DEFAULT_SUMMARY_PROMPT)[0];
    for page in 1..=5 {
        assert!(final_call
            .text
            .contains(&format!("<notes page=\"{page}\">\n- Note for page {page}")));
    }
    assert!(final_call.text.contains("Pages: 5"));

    let summary = doc.summary.as_ref().unwrap();
    assert_eq!(summary.strategy, SummaryStrategy::MapReduce);
    assert_eq!(summary.usage.requests, 4);
    assert!(summary.failed_pages.is_empty());
    let page_numbers: Vec<u32> = doc.pages.iter().map(|p| p.page_number).collect();
    assert_eq!(page_numbers, vec![1, 2, 3, 4, 5]);

    let markdown = doc.to_markdown();
    assert!(markdown.contains("### Page 3\n\n- Note for page 3"));
}

#[tokio::test]
async fn sequential_strategy_hands_running_context_forward() {
    let llm = Arc::new(ScriptedLlm::default());
    let config = SummaryConfig::default()
        .with_max_pages_per_window(2)
        .with_strategy(SummaryStrategy::Sequential);
    let doc = MarkItDown::new()
        .convert(MULTI_PAGE_PDF, Some(summarize_options(llm.clone(), config)))
        .await
        .expect("summarization succeeds");

    let reads: Vec<Call> = llm.calls().into_iter().filter(is_reading_call).collect();
    assert_eq!(reads.len(), 3);
    assert!(!reads[0].text.contains("<context_so_far>\n"));
    assert!(reads[1]
        .text
        .contains("<context_so_far>\nContext through page 2\n</context_so_far>"));
    assert!(reads[2]
        .text
        .contains("<context_so_far>\nContext through page 4\n</context_so_far>"));

    let final_call = &llm.calls_with(DEFAULT_SUMMARY_PROMPT)[0];
    assert!(final_call
        .text
        .contains("<running_summary>\nContext through page 5\n</running_summary>"));
    assert_eq!(
        doc.summary.as_ref().unwrap().strategy,
        SummaryStrategy::Sequential
    );
}

#[tokio::test]
async fn scanned_pages_are_sent_as_images_instead_of_transcribed() {
    let llm = Arc::new(ScriptedLlm::default());
    let doc = MarkItDown::new()
        .convert(
            SCANNED_PDF,
            Some(summarize_options(llm.clone(), SummaryConfig::default())),
        )
        .await
        .expect("summarization succeeds");

    assert_eq!(llm.transcriptions.load(Ordering::SeqCst), 0);
    let calls = llm.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].images, 3, "one rendered image per scanned page");
    assert_eq!(doc.pages.len(), 3);
}

#[tokio::test]
async fn extraction_mode_still_transcribes_scanned_pages() {
    let llm = Arc::new(ScriptedLlm::default());
    let doc = MarkItDown::new()
        .convert(
            SCANNED_PDF,
            Some(ConversionOptions::default().with_llm(llm.clone())),
        )
        .await
        .expect("extraction succeeds");

    assert_eq!(llm.transcriptions.load(Ordering::SeqCst), 3);
    assert!(llm.calls().is_empty());
    assert!(doc.summary.is_none());
    assert!(doc.to_markdown().contains("transcribed page"));
}

#[tokio::test]
async fn forced_visual_reading_renders_text_pages_too() {
    let llm = Arc::new(ScriptedLlm::default());
    let options = summarize_options(llm.clone(), SummaryConfig::default()).with_force_llm_ocr(true);
    MarkItDown::new()
        .convert(MULTI_PAGE_PDF, Some(options))
        .await
        .expect("summarization succeeds");

    let calls = llm.calls();
    let images: usize = calls.iter().map(|call| call.images).sum();
    assert_eq!(images, 5, "every page is rendered");
    // Good text is sent next to the page image
    assert!(calls[0].text.contains("[Extracted text of the page above]"));
    assert_eq!(llm.transcriptions.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn embedded_images_are_read_directly_instead_of_described() {
    for path in [
        "tests/test_documents/presentations/powerpoint_with_image.pptx",
        "tests/test_documents/documents/word_sample.docx",
    ] {
        let llm = Arc::new(ScriptedLlm::default());
        let doc = MarkItDown::new()
            .convert(
                path,
                Some(summarize_options(llm.clone(), SummaryConfig::default())),
            )
            .await
            .unwrap_or_else(|e| panic!("{path}: {e}"));

        assert_eq!(llm.descriptions.load(Ordering::SeqCst), 0, "{path}");
        let images: usize = llm.calls().iter().map(|call| call.images).sum();
        assert!(images >= 1, "{path}: embedded image is sent with the page");
        assert!(doc.summary.is_some());

        // Extraction mode describes the same images up front
        let llm = Arc::new(ScriptedLlm::default());
        MarkItDown::new()
            .convert(
                path,
                Some(ConversionOptions::default().with_llm(llm.clone())),
            )
            .await
            .unwrap();
        assert!(llm.descriptions.load(Ordering::SeqCst) >= 1, "{path}");
    }
}

#[tokio::test]
async fn unpaginated_document_cites_no_pages() {
    let llm = Arc::new(ScriptedLlm::default());
    MarkItDown::new()
        .convert(
            "tests/test_documents/documents/word_sample.docx",
            Some(summarize_options(llm.clone(), SummaryConfig::default())),
        )
        .await
        .expect("summarization succeeds");

    let call = &llm.calls()[0];
    assert!(call.text.contains("not paginated, so cite no page numbers"));
    assert!(!call.text.contains("page citation such as"));
}

#[tokio::test]
async fn failed_windows_are_reported_and_left_out() {
    let llm = Arc::new(ScriptedLlm {
        fail_pages: vec![2],
        ..ScriptedLlm::default()
    });
    let config = SummaryConfig::default()
        .with_max_pages_per_window(1)
        .with_max_retries(0);
    let doc = MarkItDown::new()
        .convert(MULTI_PAGE_PDF, Some(summarize_options(llm.clone(), config)))
        .await
        .expect("other pages are still summarized");

    let summary = doc.summary.as_ref().unwrap();
    assert_eq!(summary.failed_pages, vec![2]);
    let page_2 = doc.pages.iter().find(|p| p.page_number == 2).unwrap();
    assert!(page_2.to_markdown().contains("[Notes unavailable"));

    let final_call = &llm.calls_with(DEFAULT_SUMMARY_PROMPT)[0];
    assert!(final_call.text.contains("<unread_pages>2</unread_pages>"));
    assert!(!final_call.text.contains("<notes page=\"2\">"));
    assert!(doc
        .to_markdown()
        .contains("> Note: page 2 could not be read completely; the summary may not cover it."));
}

#[tokio::test]
async fn all_windows_failing_is_an_error() {
    let llm = ScriptedLlm {
        fail_pages: vec![1, 2, 3, 4, 5],
        ..ScriptedLlm::default()
    };
    let mut document = Document::new();
    for page in 1..=3 {
        let mut p = Page::new(page);
        p.add_content(ContentBlock::Text(format!("Content of page {page}")));
        document.add_page(p);
    }
    let config = SummaryConfig::default()
        .with_max_pages_per_window(1)
        .with_max_retries(0);

    let result = document.summarize(&llm, &config).await;
    assert!(matches!(result, Err(MarkitdownError::LlmError(_))));
}

#[tokio::test]
async fn long_notes_are_pruned_and_condensed_to_fit() {
    let llm = ScriptedLlm {
        long_notes: true,
        ..ScriptedLlm::default()
    };
    let mut document = Document::new();
    for page in 1..=60 {
        let mut p = Page::new(page);
        p.add_content(ContentBlock::Text(format!(
            "Findings for chapter {page}. {}",
            "Body text. ".repeat(50)
        )));
        document.add_page(p);
    }
    let config = SummaryConfig::default().with_max_notes_tokens(2_000);

    let summarized = document.summarize(&llm, &config).await.unwrap();

    let merges = llm.calls_with(DEFAULT_SUMMARY_MERGE_PROMPT);
    assert!(!merges.is_empty(), "notes over budget are condensed");
    // Pruning keeps only the most important bullet of each page before merging
    assert!(merges
        .iter()
        .all(|call| !call.text.contains("- Note 6 for page")));
    assert!(merges[0].text.contains("- Note 1 for page 1:"));

    let final_call = &llm.calls_with(DEFAULT_SUMMARY_PROMPT)[0];
    assert!(final_call.text.contains("<section pages=\"1-"));
    assert!(final_call.text.len() / 4 < 2_000 + 1_000);

    // The returned notes are complete: pruning only shapes the final request
    assert!(summarized.pages[0]
        .to_markdown()
        .contains("- Note 6 for page 1:"));
}

#[tokio::test]
async fn large_unpaginated_page_is_read_in_parts() {
    let llm = ScriptedLlm::default();
    let mut page = Page::new(1);
    for section in 1..=12 {
        page.add_content(ContentBlock::Heading {
            level: 2,
            text: format!("Section {section}"),
        });
        page.add_content(ContentBlock::Text("Long paragraph text. ".repeat(400)));
    }
    let document = Document::from_page(page);
    let config = SummaryConfig::default().with_max_window_tokens(8_000);

    let summarized = document.summarize(&llm, &config).await.unwrap();

    let reads: Vec<Call> = llm.calls().into_iter().filter(is_reading_call).collect();
    assert!(reads.len() > 1);
    assert!(reads[0].text.contains("part=\"1 of"));
    // Every window carries the document outline built from the headings
    assert!(reads
        .iter()
        .all(|call| call.text.contains("Outline:\n- Section 1")));
    // The notes of all parts end up on the one page
    assert_eq!(summarized.pages.len(), 1);
    assert_eq!(
        summarized.pages[0]
            .to_markdown()
            .matches("Note for page 1")
            .count(),
        reads.len()
    );
}

#[tokio::test]
async fn models_ignoring_the_format_still_produce_a_summary() {
    let llm = Arc::new(MockLlmClient::new().with_text_response("Plain prose without any tags."));
    for config in [
        SummaryConfig::default(),
        SummaryConfig::default().with_max_pages_per_window(2),
    ] {
        let options = ConversionOptions::default()
            .with_llm(llm.clone())
            .with_summarization(config);
        let doc = MarkItDown::new()
            .convert(MULTI_PAGE_PDF, Some(options))
            .await
            .expect("untagged output is used as the summary");

        let summary = doc.summary.as_ref().unwrap();
        assert_eq!(summary.summary, "Plain prose without any tags.");
        assert!(summary.key_points.is_empty());
    }
}

#[tokio::test]
async fn summarization_requires_an_llm_client() {
    let options = ConversionOptions::default().with_summarization(SummaryConfig::default());
    let result = MarkItDown::new()
        .convert(MULTI_PAGE_PDF, Some(options))
        .await;
    assert!(matches!(result, Err(MarkitdownError::LlmError(_))));
}

#[tokio::test]
async fn summarized_document_round_trips_through_json() {
    let llm = Arc::new(ScriptedLlm::default());
    let doc = MarkItDown::new()
        .convert(
            "tests/test_documents/pdfs/fake_memo.pdf",
            Some(summarize_options(llm, SummaryConfig::default())),
        )
        .await
        .unwrap();

    let json = serde_json::to_string(&doc).unwrap();
    let restored: Document = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.to_markdown(), doc.to_markdown());

    // Documents serialized before summarization mode existed still load
    let legacy: Document =
        serde_json::from_str(r#"{"title":null,"pages":[],"metadata":{}}"#).unwrap();
    assert!(legacy.summary.is_none());
}

#[test]
fn conversion_future_is_send() {
    fn assert_send<T: Send>(_: &T) {}
    let markitdown = MarkItDown::new();
    let options = ConversionOptions::default().with_summarization(SummaryConfig::default());
    let future = markitdown.convert("document.pdf", Some(options));
    assert_send(&future);
}
