//! Example demonstrating PDF extraction with OpenRouter LLM.
//!
//! Run with:
//!   source .env && cargo run --example openrouter_pdf
//!
//! This example uses the OpenRouter API to extract content from PDFs,
//! including image descriptions via LLM OCR.
//!
//! Required environment variables:
//! - OPENROUTER_API_KEY: Your OpenRouter API key
//! - OPENROUTER_MODEL: (optional) Model to use, defaults to @preset/prod-free
//! - MERGE_TABLES: (optional) Set to "true" to merge multi-page tables
//!
//! Note: Free models may require enabling "Free model training" in your
//! OpenRouter privacy settings: https://openrouter.ai/settings/privacy
//! Otherwise you may see 404 errors when using multimodal (image) requests.

use markitdown::table_merge;
use markitdown::{create_llm_client_with_config, ConversionOptions, LlmConfig, MarkItDown};
use rig::client::{CompletionClient, ProviderClient};
use rig::providers::openrouter;

const TEST_PDF: &str = "tests/test_files/BMW.pdf";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== OpenRouter PDF Extraction Example ===\n");

    // Check for API key
    if std::env::var("OPENROUTER_API_KEY").is_err() {
        eprintln!("Error: OPENROUTER_API_KEY environment variable not set");
        eprintln!("Please run: source .env");
        return Ok(());
    }

    // Create OpenRouter client
    println!("Creating OpenRouter client...");
    let openrouter_client = openrouter::Client::from_env();

    // Use the model from OPENROUTER_MODEL env var, or default to @preset/prod-free
    let model_id =
        std::env::var("OPENROUTER_MODEL").unwrap_or_else(|_| "@preset/prod-free".to_string());
    let model = openrouter_client.completion_model(&model_id);

    // Configure for PDF processing - use low temperature for accurate extraction
    // Set max_tokens to prevent runaway generation from free models
    let config = LlmConfig::default()
        .with_temperature(0.0) // Zero temperature for deterministic output
        .with_images_per_message(1) // Process one image at a time for better quality
        .with_max_tokens(Some(4096)); // Limit output to prevent repetition loops

    let llm = create_llm_client_with_config(model, config);
    println!("OpenRouter LLM client created with model: {}", model_id);
    println!("Config: temperature=0.0, max_tokens=4096\n");

    // Create MarkItDown instance
    let markitdown = MarkItDown::new();

    // Set up conversion options with LLM
    // Use force_llm_ocr to always use LLM for image-heavy PDFs like BMW.pdf
    let merge_tables = std::env::var("MERGE_TABLES")
        .map(|v| v.to_lowercase() == "true")
        .unwrap_or(false);

    let options = ConversionOptions::default()
        .with_extension(".pdf")
        .with_llm(llm)
        .with_force_llm_ocr(true) // Force LLM OCR mode for image descriptions
        .with_merge_multipage_tables(merge_tables);

    if merge_tables {
        println!("Table merging enabled: multi-page tables will be combined\n");
    }

    // Convert the PDF
    println!("Converting PDF: {}", TEST_PDF);
    println!("This may take a while as each page is processed by the LLM...\n");

    // First, let's check actual page count with hayro
    let bytes = std::fs::read(TEST_PDF)?;
    let data: std::sync::Arc<dyn AsRef<[u8]> + Send + Sync> = std::sync::Arc::new(bytes.clone());
    if let Ok(pdf) = hayro_syntax::Pdf::new(data) {
        println!("PDF parsed with hayro: {} pages", pdf.pages().len());
    }

    // Also check pdf_extract page splits
    if let Ok(text) = pdf_extract::extract_text_from_mem(&bytes) {
        let page_count = text.split('\x0c').count();
        println!("pdf_extract form feed pages: {}", page_count);
    }

    match markitdown.convert(TEST_PDF, Some(options)).await {
        Ok(document) => {
            // Apply table merging if enabled
            let document = if merge_tables {
                println!("Applying table merging across pages...");
                let merged = document.with_merged_tables();

                // Report any merges that occurred
                let page_contents: Vec<(u32, String)> = document
                    .pages
                    .iter()
                    .map(|p| (p.page_number, p.to_markdown()))
                    .collect();
                let merge_results = table_merge::merge_tables_across_pages(&page_contents);
                let merged_count = merge_results.iter().filter(|r| r.merged_from_next).count();
                if merged_count > 0 {
                    println!(
                        "Merged {} table(s) spanning page boundaries\n",
                        merged_count
                    );
                }
                merged
            } else {
                document
            };

            println!("=== Conversion Result ===\n");
            println!("Number of pages: {}", document.pages.len());

            // Print summary of each page
            for (i, page) in document.pages.iter().enumerate() {
                let markdown = page.to_markdown();
                let word_count = markdown.split_whitespace().count();
                let image_count = page.images().len();
                println!(
                    "Page {}: {} words, {} images, {} chars",
                    i + 1,
                    word_count,
                    image_count,
                    markdown.len()
                );
            }

            // Print first page content (truncated)
            if let Some(first_page) = document.pages.first() {
                println!("\n--- First Page Preview ---");
                let markdown = first_page.to_markdown();
                if markdown.chars().count() > 3000 {
                    let truncated: String = markdown.chars().take(3000).collect();
                    println!("{}...\n[truncated]", truncated);
                } else {
                    println!("{}", markdown);
                }
            }

            // Save to file
            let output_path = "test.md";
            let full_markdown = document.to_markdown();
            std::fs::write(output_path, &full_markdown)?;
            println!(
                "\n=== Full output saved to {} ({} bytes) ===",
                output_path,
                full_markdown.len()
            );

            // Save individual pages to out/ folder
            std::fs::create_dir_all("out")?;
            let base_name = std::path::Path::new(TEST_PDF)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("page");

            for (i, page) in document.pages.iter().enumerate() {
                let page_path = format!("out/{}_{}.md", base_name, i + 1);
                let page_markdown = page.to_markdown();
                std::fs::write(&page_path, &page_markdown)?;
                println!("Saved: {} ({} bytes)", page_path, page_markdown.len());
            }
        }
        Err(e) => {
            eprintln!("Error converting PDF: {:?}", e);
        }
    }

    Ok(())
}
