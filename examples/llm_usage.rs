//! Example demonstrating how to use the LLM abstraction with any rig-core provider.
//!
//! Run with: cargo run --example llm_usage
//!
//! Note: This example requires API keys to be set in the environment:
//! - OPENAI_API_KEY for OpenAI
//! - ANTHROPIC_API_KEY for Anthropic
//! - GEMINI_API_KEY for Google Gemini

use markitdown::{create_llm_client, create_llm_client_with_config, LlmConfig, MarkItDown};
use rig::client::{CompletionClient, ProviderClient};
use rig::providers::{anthropic, gemini, openai};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Example 1: Using OpenAI
    println!("=== Example 1: Creating an LLM client with OpenAI ===\n");

    if let Ok(_) = std::env::var("OPENAI_API_KEY") {
        let openai_client = openai::Client::from_env()?;
        let openai_model = openai_client.completion_model("gpt-4o");
        let llm = create_llm_client(openai_model);

        println!("OpenAI LLM client created successfully!");
        println!("Type: Arc<dyn LlmClient>\n");

        // Use with MarkItDown
        let _markitdown = MarkItDown::new();
        // options.with_llm(llm.clone());
        drop(llm);
    } else {
        println!("OPENAI_API_KEY not set, skipping OpenAI example\n");
    }

    // Example 2: Using Anthropic
    println!("=== Example 2: Creating an LLM client with Anthropic ===\n");

    if let Ok(_) = std::env::var("ANTHROPIC_API_KEY") {
        let anthropic_client = anthropic::Client::from_env()?;
        let anthropic_model = anthropic_client.completion_model("claude-sonnet-4-20250514");
        let llm = create_llm_client(anthropic_model);

        println!("Anthropic LLM client created successfully!");
        drop(llm);
    } else {
        println!("ANTHROPIC_API_KEY not set, skipping Anthropic example\n");
    }

    // Example 3: Using Google Gemini
    println!("=== Example 3: Creating an LLM client with Google Gemini ===\n");

    if let Ok(_) = std::env::var("GEMINI_API_KEY") {
        let gemini_client = gemini::Client::from_env()?;
        let gemini_model = gemini_client.completion_model("gemini-2.0-flash");
        let llm = create_llm_client(gemini_model);

        println!("Gemini LLM client created successfully!");
        drop(llm);
    } else {
        println!("GEMINI_API_KEY not set, skipping Gemini example\n");
    }

    // Example 4: Custom configuration
    println!("=== Example 4: Custom LLM configuration ===\n");

    if let Ok(_) = std::env::var("OPENAI_API_KEY") {
        let openai_client = openai::Client::from_env()?;
        let openai_model = openai_client.completion_model("gpt-4o-mini");

        let config = LlmConfig::default()
            .with_temperature(0.1) // More deterministic
            .with_images_per_message(5) // Batch 5 images per request
            .with_image_prompt(
                "You are an expert at describing images for accessibility purposes.",
            );

        let llm = create_llm_client_with_config(openai_model, config);

        println!("Custom configured LLM client created!");
        println!("- Temperature: 0.1");
        println!("- Images per message: 5");
        println!("- Custom image prompt set\n");
        drop(llm);
    } else {
        println!("OPENAI_API_KEY not set, skipping custom config example\n");
    }

    // Example 5: Using the SharedLlmClient type alias
    println!("=== Example 5: Type flexibility ===\n");

    println!("The LLM abstraction provides:");
    println!("- LlmWrapper<M>: Generic wrapper for any CompletionModel");
    println!("- SharedLlmClient: Arc<dyn LlmClient> for sharing across threads");
    println!("- create_llm_client(): Helper to create SharedLlmClient");
    println!("- MockLlmClient: For testing without API calls\n");

    // Example 6: Mock client for testing
    println!("=== Example 6: Mock client for testing ===\n");

    use markitdown::MockLlmClient;

    let mock = MockLlmClient::new()
        .with_image_response("A beautiful landscape with mountains")
        .with_text_response("# Document Title\n\nThis is the content.");

    let mock_llm: Arc<dyn markitdown::LlmClient> = Arc::new(mock);
    println!("Mock LLM client created for testing!");
    println!("- Image responses: 'A beautiful landscape with mountains'");
    println!("- Text responses: '# Document Title\\n\\nThis is the content.'\n");
    drop(mock_llm);

    println!("=== All examples completed! ===");

    Ok(())
}
