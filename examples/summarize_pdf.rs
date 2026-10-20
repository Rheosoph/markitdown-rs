//! Example: summarize a document with an LLM instead of transcribing it.
//!
//! Run with:
//!   source .env && cargo run --example summarize_pdf -- path/to/document.pdf
//!
//! Summarization mode sends pages to the LLM as input (text, page images, embedded
//! images) and only generates page notes plus a summary, so it needs far fewer output
//! tokens than extraction mode, which transcribes every page.
//!
//! Required environment variables:
//! - OPENROUTER_API_KEY: Your OpenRouter API key
//! - OPENROUTER_VISION_MODEL or OPENROUTER_MODEL: (optional) Model to use; scanned pages
//!   and images need a vision-capable model. Defaults to @preset/prod-free
//! - SUMMARY_FOCUS: (optional) What the summary should concentrate on
//! - SUMMARY_STRATEGY: (optional) "sequential" to read pages in order with a running
//!   summary instead of concurrently

use markitdown::{
    create_llm_client_with_config, ConversionOptions, LlmConfig, MarkItDown, SummaryConfig,
    SummaryStrategy,
};
use rig::client::{CompletionClient, ProviderClient};
use rig::providers::openrouter;
use std::time::Instant;

const DEFAULT_DOCUMENT: &str = "tests/test_documents/pdfs/multi_page.pdf";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_DOCUMENT.to_string());

    if std::env::var("OPENROUTER_API_KEY").is_err() {
        eprintln!("Error: OPENROUTER_API_KEY environment variable not set");
        eprintln!("Please run: source .env");
        return Ok(());
    }

    let model_id = std::env::var("OPENROUTER_VISION_MODEL")
        .or_else(|_| std::env::var("OPENROUTER_MODEL"))
        .unwrap_or_else(|_| "@preset/prod-free".to_string());
    let model = openrouter::Client::from_env()?.completion_model(&model_id);
    let llm = create_llm_client_with_config(model, LlmConfig::default().with_temperature(0.2));

    let mut config = SummaryConfig::default();
    if let Ok(focus) = std::env::var("SUMMARY_FOCUS") {
        config = config.with_focus(focus);
    }
    if std::env::var("SUMMARY_STRATEGY").is_ok_and(|s| s.eq_ignore_ascii_case("sequential")) {
        config = config.with_strategy(SummaryStrategy::Sequential);
    }

    let options = ConversionOptions::default()
        .with_llm(llm)
        .with_summarization(config);

    eprintln!("Summarizing {} with {}...\n", path, model_id);
    let start = Instant::now();
    let document = MarkItDown::new().convert(&path, Some(options)).await?;

    // Summary, key points and the notes for every page
    println!("{}", document.to_markdown());

    if let Some(summary) = &document.summary {
        eprintln!(
            "\n{} requests, {} input tokens, {} output tokens in {:.1?}",
            summary.usage.requests,
            summary.usage.input_tokens,
            summary.usage.output_tokens,
            start.elapsed()
        );
        if !summary.failed_pages.is_empty() {
            eprintln!("Pages that could not be read: {:?}", summary.failed_pages);
        }
    }

    Ok(())
}
