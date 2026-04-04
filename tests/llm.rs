use bytes::Bytes;
use markitdown::{create_llm_client, ConversionOptions, MarkItDown};
use rig::client::CompletionClient;
use rig::providers::{azure, azure::AzureOpenAIAuth, gemini, openai, openrouter};
use std::env;

/// Test image file - a real PNG image from a scientific paper
const TEST_IMAGE_PATH: &str = "tests/test_documents/images/2305_03393v1_pg9_img.png";
/// Test PDF with images
const TEST_PDF_PATH: &str = "tests/test_documents/pdf/with_images.pdf";

/// Safely truncate a string to a maximum number of characters (not bytes)
fn safe_truncate(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((byte_pos, _)) => &s[..byte_pos],
        None => s,
    }
}

/// Load test image from file
fn load_test_image() -> Vec<u8> {
    std::fs::read(TEST_IMAGE_PATH).expect("Failed to load test image")
}

fn azure_deployment_id() -> Result<String, env::VarError> {
    env::var("AZURE_DEPLOYMENT_ID").or_else(|_| env::var("AZURE_MODEL_ID"))
}

fn azure_build_client(api_key: String, endpoint: String, api_version: &str) -> azure::Client {
    azure::Client::builder()
        .api_key(AzureOpenAIAuth::ApiKey(api_key))
        .azure_endpoint(endpoint)
        .api_version(api_version)
        .build()
        .expect("Failed to build Azure client")
}

fn should_skip_missing_azure_deployment(
    test_name: &str,
    deployment_id: &str,
    error: &impl std::fmt::Display,
) -> bool {
    let message = error.to_string();
    if message.contains("DeploymentNotFound") {
        println!(
            "Skipping {}: Azure deployment '{}' was not found. Set AZURE_DEPLOYMENT_ID to a valid Azure deployment name.",
            test_name, deployment_id
        );
        true
    } else {
        false
    }
}

// ============================================================================
// OpenRouter Tests
// ============================================================================

#[tokio::test]
async fn test_openrouter_image_description() {
    let _ = dotenvy::dotenv();

    let api_key = env::var("OPENROUTER_API_KEY");
    let endpoint = env::var("OPENROUTER_ENDPOINT");
    // Use OPENROUTER_VISION_MODEL if set, otherwise fall back to OPENROUTER_MODEL
    let model = env::var("OPENROUTER_VISION_MODEL").or_else(|_| env::var("OPENROUTER_MODEL"));

    if api_key.is_err() || endpoint.is_err() || model.is_err() {
        println!("Skipping OpenRouter LLM test: Missing OPENROUTER_API_KEY, OPENROUTER_ENDPOINT, or OPENROUTER_MODEL");
        return;
    }

    let api_key = api_key.unwrap();
    let endpoint = endpoint.unwrap();
    let model_name = model.unwrap();

    // Some models don't support vision - skip with a helpful message
    if model_name.contains("@preset") || model_name.contains("prod-free") {
        println!(
            "Skipping OpenRouter image description test: Model '{}' likely doesn't support vision.",
            model_name
        );
        println!("Set OPENROUTER_VISION_MODEL to a vision-capable model like 'openai/gpt-4o-mini' for this test.");
        return;
    }

    // Normalize to OpenRouter's expected base URL
    let endpoint = endpoint.trim_end_matches('/').to_string();
    let endpoint = if endpoint.ends_with("/api/v1") {
        endpoint
    } else if endpoint.ends_with("/api") {
        format!("{}/v1", endpoint)
    } else if endpoint.ends_with("/v1") {
        endpoint
    } else {
        format!("{}/v1", endpoint)
    };

    println!(
        "Running OpenRouter image description test with model: {}",
        model_name
    );

    let client: openrouter::Client = openrouter::Client::builder()
        .api_key(api_key)
        .base_url(endpoint.as_str())
        .build()
        .expect("Failed to build OpenRouter client");

    let model = client.completion_model(&model_name);
    let llm_client = create_llm_client(model);

    let image_data = load_test_image();
    println!("Loaded test image: {} bytes", image_data.len());

    println!("Sending image to OpenRouter LLM...");
    let result = llm_client.describe_image(&image_data, "image/png").await;

    match result {
        Ok(description) => {
            println!(
                "✓ OpenRouter Image Description ({} chars):\n{}",
                description.len(),
                safe_truncate(&description, 500)
            );
            assert!(!description.is_empty(), "Description should not be empty");
        }
        Err(e) => panic!("OpenRouter image description test failed: {}", e),
    }
}

#[tokio::test]
async fn test_openrouter_text_completion() {
    let _ = dotenvy::dotenv();

    let api_key = env::var("OPENROUTER_API_KEY");
    let endpoint = env::var("OPENROUTER_ENDPOINT");
    let model = env::var("OPENROUTER_MODEL");

    if api_key.is_err() || endpoint.is_err() || model.is_err() {
        println!("Skipping OpenRouter text completion test");
        return;
    }

    let api_key = api_key.unwrap();
    let endpoint = endpoint.unwrap();
    let model_name = model.unwrap();

    let endpoint = endpoint.trim_end_matches('/').to_string();
    let endpoint = if endpoint.ends_with("/api/v1") {
        endpoint
    } else if endpoint.ends_with("/api") {
        format!("{}/v1", endpoint)
    } else {
        format!("{}/v1", endpoint)
    };

    println!(
        "Running OpenRouter text completion test with model: {}",
        model_name
    );

    let client: openrouter::Client = openrouter::Client::builder()
        .api_key(api_key)
        .base_url(endpoint.as_str())
        .build()
        .expect("Failed to build OpenRouter client");

    let model = client.completion_model(&model_name);
    let llm_client = create_llm_client(model);

    let result = llm_client
        .complete("Say 'Hello, World!' and nothing else.")
        .await;

    match result {
        Ok(response) => {
            println!("✓ OpenRouter Text Completion: {}", response);
            assert!(!response.is_empty(), "Response should not be empty");
        }
        Err(e) => panic!("OpenRouter text completion test failed: {}", e),
    }
}

// ============================================================================
// Azure OpenAI Tests
// ============================================================================

#[tokio::test]
async fn test_azure_openai_text_completion() {
    let _ = dotenvy::dotenv();

    let api_key = env::var("AZURE_API_KEY");
    let endpoint = env::var("AZURE_ENDPOINT");
    let api_version = env::var("AZURE_API_VERSION").or_else(|_| env::var("AZURE_VERSION"));
    let deployment_id = azure_deployment_id();

    if api_key.is_err() || endpoint.is_err() || api_version.is_err() || deployment_id.is_err() {
        println!(
            "Skipping Azure OpenAI text completion test: Missing environment variables.\n\
             Required: AZURE_API_KEY, AZURE_ENDPOINT, AZURE_API_VERSION (or AZURE_VERSION), AZURE_DEPLOYMENT_ID or AZURE_MODEL_ID"
        );
        return;
    }

    let api_key = api_key.unwrap();
    let endpoint = endpoint.unwrap();
    let api_version = api_version.unwrap();
    let deployment_id = deployment_id.unwrap();

    println!(
        "Running Azure OpenAI text completion test:\n  endpoint: {}\n  deployment: {}\n  api_version: {}",
        endpoint, deployment_id, api_version
    );

    // Build Azure client
    let client = azure_build_client(api_key, endpoint, &api_version);

    let model = client.completion_model(&deployment_id);
    let llm_client = create_llm_client(model);

    println!("Sending text completion request to Azure OpenAI...");
    let result = llm_client
        .complete("Say 'Hello, World!' and nothing else.")
        .await;

    match result {
        Ok(response) => {
            println!("✓ Azure OpenAI Text Completion: {}", response);
            assert!(!response.is_empty(), "Response should not be empty");
        }
        Err(e) => {
            if should_skip_missing_azure_deployment(
                "Azure OpenAI text completion test",
                &deployment_id,
                &e,
            ) {
                return;
            }
            eprintln!("Azure OpenAI text completion test failed: {}", e);
            panic!("Azure OpenAI text completion test failed: {}", e);
        }
    }
}

#[tokio::test]
async fn test_azure_openai_image_description() {
    let _ = dotenvy::dotenv();

    let api_key = env::var("AZURE_API_KEY");
    let endpoint = env::var("AZURE_ENDPOINT");
    let api_version = env::var("AZURE_API_VERSION").or_else(|_| env::var("AZURE_VERSION"));
    let deployment_id = azure_deployment_id();

    if api_key.is_err() || endpoint.is_err() || api_version.is_err() || deployment_id.is_err() {
        println!("Skipping Azure OpenAI image description test: Missing environment variables");
        return;
    }

    let api_key = api_key.unwrap();
    let endpoint = endpoint.unwrap();
    let api_version = api_version.unwrap();
    let deployment_id = deployment_id.unwrap();

    println!(
        "Running Azure OpenAI image description test:\n  endpoint: {}\n  deployment: {}\n  api_version: {}",
        endpoint, deployment_id, api_version
    );

    let client = azure_build_client(api_key, endpoint, &api_version);

    let model = client.completion_model(&deployment_id);
    let llm_client = create_llm_client(model);

    let image_data = load_test_image();
    println!("Loaded test image: {} bytes", image_data.len());

    println!("Sending image to Azure OpenAI LLM...");
    let result = llm_client.describe_image(&image_data, "image/png").await;

    match result {
        Ok(description) => {
            println!(
                "✓ Azure OpenAI Image Description ({} chars):\n{}",
                description.len(),
                safe_truncate(&description, 500)
            );
            assert!(!description.is_empty(), "Description should not be empty");
        }
        Err(e) => {
            if should_skip_missing_azure_deployment(
                "Azure OpenAI image description test",
                &deployment_id,
                &e,
            ) {
                return;
            }
            eprintln!("Azure OpenAI image description test failed: {}", e);
            eprintln!("This may be due to:");
            eprintln!(
                "  1. The Azure deployment '{}' doesn't support vision/image inputs",
                deployment_id
            );
            eprintln!("  2. API quota/limits exceeded");
            eprintln!("  3. Invalid API key or endpoint");
            panic!("Azure OpenAI image description test failed: {}", e);
        }
    }
}

#[tokio::test]
async fn test_azure_openai_pdf_conversion() {
    let _ = dotenvy::dotenv();

    let api_key = env::var("AZURE_API_KEY");
    let endpoint = env::var("AZURE_ENDPOINT");
    let api_version = env::var("AZURE_API_VERSION").or_else(|_| env::var("AZURE_VERSION"));
    let deployment_id = azure_deployment_id();

    if api_key.is_err() || endpoint.is_err() || api_version.is_err() || deployment_id.is_err() {
        println!("Skipping Azure OpenAI PDF conversion test: Missing environment variables");
        return;
    }

    let api_key = api_key.unwrap();
    let endpoint = endpoint.unwrap();
    let api_version = api_version.unwrap();
    let deployment_id = deployment_id.unwrap();

    println!(
        "Running Azure OpenAI PDF conversion test:\n  endpoint: {}\n  deployment: {}\n  api_version: {}",
        endpoint, deployment_id, api_version
    );

    let client = azure_build_client(api_key, endpoint, &api_version);

    let model = client.completion_model(&deployment_id);
    let llm_client = create_llm_client(model);

    // Load PDF and convert with LLM support
    let pdf_data = std::fs::read(TEST_PDF_PATH).expect("Failed to load test PDF");
    println!("Loaded test PDF: {} bytes", pdf_data.len());

    let converter = MarkItDown::new();
    let options = ConversionOptions::new()
        .with_extension(".pdf")
        .with_llm(llm_client)
        .with_force_llm_ocr(true);

    println!("Converting PDF with LLM support...");
    let result = converter
        .convert_bytes(Bytes::from(pdf_data), Some(options))
        .await;

    match result {
        Ok(doc) => {
            let markdown = doc.to_markdown();
            println!(
                "✓ Azure OpenAI PDF Conversion ({} chars):\n{}",
                markdown.len(),
                safe_truncate(&markdown, 1000)
            );
            assert!(!markdown.is_empty(), "Markdown should not be empty");
        }
        Err(e) => {
            if should_skip_missing_azure_deployment(
                "Azure OpenAI PDF conversion test",
                &deployment_id,
                &e,
            ) {
                return;
            }
            eprintln!("Azure OpenAI PDF conversion test failed: {}", e);
            panic!("Azure OpenAI PDF conversion test failed: {}", e);
        }
    }
}

// ============================================================================
// Google Gemini Tests
// ============================================================================

#[tokio::test]
async fn test_gemini_text_completion() {
    let _ = dotenvy::dotenv();

    let api_key = env::var("GEMINI_API_KEY");
    let model_name = env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-1.5-flash".to_string());

    if api_key.is_err() {
        println!("Skipping Gemini text completion test: Missing GEMINI_API_KEY");
        return;
    }

    let api_key = api_key.unwrap();

    println!(
        "Running Gemini text completion test with model: {}",
        model_name
    );

    let client: gemini::Client =
        gemini::Client::new(&api_key).expect("Failed to create Gemini client");
    let model = client.completion_model(&model_name);
    let llm_client = create_llm_client(model);

    println!("Sending text completion request to Gemini...");
    let result = llm_client
        .complete("Say 'Hello, World!' and nothing else.")
        .await;

    match result {
        Ok(response) => {
            println!("✓ Gemini Text Completion: {}", response);
            assert!(!response.is_empty(), "Response should not be empty");
        }
        Err(e) => panic!("Gemini text completion test failed: {}", e),
    }
}

#[tokio::test]
async fn test_gemini_image_description() {
    let _ = dotenvy::dotenv();

    let api_key = env::var("GEMINI_API_KEY");
    let model_name = env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-1.5-flash".to_string());

    if api_key.is_err() {
        println!("Skipping Gemini image description test: Missing GEMINI_API_KEY");
        return;
    }

    let api_key = api_key.unwrap();

    println!(
        "Running Gemini image description test with model: {}",
        model_name
    );

    let client: gemini::Client =
        gemini::Client::new(&api_key).expect("Failed to create Gemini client");
    let model = client.completion_model(&model_name);
    let llm_client = create_llm_client(model);

    let image_data = load_test_image();
    println!("Loaded test image: {} bytes", image_data.len());

    println!("Sending image to Gemini LLM...");
    let result = llm_client.describe_image(&image_data, "image/png").await;

    match result {
        Ok(description) => {
            println!(
                "✓ Gemini Image Description ({} chars):\n{}",
                description.len(),
                safe_truncate(&description, 500)
            );
            assert!(!description.is_empty(), "Description should not be empty");
        }
        Err(e) => {
            eprintln!("Gemini image description test failed: {}", e);
            eprintln!("This may be due to:");
            eprintln!(
                "  1. The model '{}' doesn't support vision/image inputs",
                model_name
            );
            eprintln!("  2. API quota/limits exceeded");
            eprintln!("  3. Invalid API key");
            panic!("Gemini image description test failed: {}", e);
        }
    }
}

#[tokio::test]
async fn test_gemini_pdf_conversion() {
    let _ = dotenvy::dotenv();

    let api_key = env::var("GEMINI_API_KEY");
    let model_name = env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-1.5-flash".to_string());

    if api_key.is_err() {
        println!("Skipping Gemini PDF conversion test: Missing GEMINI_API_KEY");
        return;
    }

    let api_key = api_key.unwrap();

    println!(
        "Running Gemini PDF conversion test with model: {}",
        model_name
    );

    let client = gemini::Client::new(&api_key).expect("Failed to create Gemini client");
    let model = client.completion_model(&model_name);
    let llm_client = create_llm_client(model);

    let pdf_data = std::fs::read(TEST_PDF_PATH).expect("Failed to load test PDF");
    println!("Loaded test PDF: {} bytes", pdf_data.len());

    let converter = MarkItDown::new();
    let options = ConversionOptions::new()
        .with_extension(".pdf")
        .with_llm(llm_client)
        .with_force_llm_ocr(true);

    println!("Converting PDF with LLM support...");
    let result = converter
        .convert_bytes(Bytes::from(pdf_data), Some(options))
        .await;

    match result {
        Ok(doc) => {
            let markdown = doc.to_markdown();
            println!(
                "✓ Gemini PDF Conversion ({} chars):\n{}",
                markdown.len(),
                safe_truncate(&markdown, 1000)
            );
            assert!(!markdown.is_empty(), "Markdown should not be empty");
        }
        Err(e) => {
            eprintln!("Gemini PDF conversion test failed: {}", e);
            panic!("Gemini PDF conversion test failed: {}", e);
        }
    }
}

// ============================================================================
// OpenAI (ChatGPT) Tests
// ============================================================================

#[tokio::test]
async fn test_openai_text_completion() {
    let _ = dotenvy::dotenv();

    let api_key = env::var("OPENAI_API_KEY");
    let model_name = env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());

    if api_key.is_err() {
        println!("Skipping OpenAI text completion test: Missing OPENAI_API_KEY");
        return;
    }

    let api_key = api_key.unwrap();

    println!(
        "Running OpenAI text completion test with model: {}",
        model_name
    );

    let client: openai::Client =
        openai::Client::new(&api_key).expect("Failed to create OpenAI client");
    let model = client.completion_model(&model_name);
    let llm_client = create_llm_client(model);

    println!("Sending text completion request to OpenAI...");
    let result = llm_client
        .complete("Say 'Hello, World!' and nothing else.")
        .await;

    match result {
        Ok(response) => {
            println!("✓ OpenAI Text Completion: {}", response);
            assert!(!response.is_empty(), "Response should not be empty");
        }
        Err(e) => panic!("OpenAI text completion test failed: {}", e),
    }
}

#[tokio::test]
async fn test_openai_image_description() {
    let _ = dotenvy::dotenv();

    let api_key = env::var("OPENAI_API_KEY");
    let model_name = env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());

    if api_key.is_err() {
        println!("Skipping OpenAI image description test: Missing OPENAI_API_KEY");
        return;
    }

    let api_key = api_key.unwrap();

    println!(
        "Running OpenAI image description test with model: {}",
        model_name
    );

    let client: openai::Client =
        openai::Client::new(&api_key).expect("Failed to create OpenAI client");
    let model = client.completion_model(&model_name);
    let llm_client = create_llm_client(model);

    let image_data = load_test_image();
    println!("Loaded test image: {} bytes", image_data.len());

    println!("Sending image to OpenAI LLM...");
    let result = llm_client.describe_image(&image_data, "image/png").await;

    match result {
        Ok(description) => {
            println!(
                "✓ OpenAI Image Description ({} chars):\n{}",
                description.len(),
                safe_truncate(&description, 500)
            );
            assert!(!description.is_empty(), "Description should not be empty");
        }
        Err(e) => {
            eprintln!("OpenAI image description test failed: {}", e);
            eprintln!("This may be due to:");
            eprintln!(
                "  1. The model '{}' doesn't support vision/image inputs",
                model_name
            );
            eprintln!("  2. API quota/limits exceeded");
            eprintln!("  3. Invalid API key");
            panic!("OpenAI image description test failed: {}", e);
        }
    }
}

#[tokio::test]
async fn test_openai_pdf_conversion() {
    let _ = dotenvy::dotenv();

    let api_key = env::var("OPENAI_API_KEY");
    let model_name = env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());

    if api_key.is_err() {
        println!("Skipping OpenAI PDF conversion test: Missing OPENAI_API_KEY");
        return;
    }

    let api_key = api_key.unwrap();

    println!(
        "Running OpenAI PDF conversion test with model: {}",
        model_name
    );

    let client: openai::Client =
        openai::Client::new(&api_key).expect("Failed to create OpenAI client");
    let model = client.completion_model(&model_name);
    let llm_client = create_llm_client(model);

    let pdf_data = std::fs::read(TEST_PDF_PATH).expect("Failed to load test PDF");
    println!("Loaded test PDF: {} bytes", pdf_data.len());

    let converter = MarkItDown::new();
    let options = ConversionOptions::new()
        .with_extension(".pdf")
        .with_llm(llm_client)
        .with_force_llm_ocr(true);

    println!("Converting PDF with LLM support...");
    let result = converter
        .convert_bytes(Bytes::from(pdf_data), Some(options))
        .await;

    match result {
        Ok(doc) => {
            let markdown = doc.to_markdown();
            println!(
                "✓ OpenAI PDF Conversion ({} chars):\n{}",
                markdown.len(),
                safe_truncate(&markdown, 1000)
            );
            assert!(!markdown.is_empty(), "Markdown should not be empty");
        }
        Err(e) => {
            eprintln!("OpenAI PDF conversion test failed: {}", e);
            panic!("OpenAI PDF conversion test failed: {}", e);
        }
    }
}

// ============================================================================
// Legacy test name compatibility
// ============================================================================

#[tokio::test]
async fn test_llm_integration() {
    // This is kept for backward compatibility - runs the OpenRouter test
    let _ = dotenvy::dotenv();

    let api_key = env::var("OPENROUTER_API_KEY");
    let endpoint = env::var("OPENROUTER_ENDPOINT");
    let model = env::var("OPENROUTER_MODEL");

    if api_key.is_err() || endpoint.is_err() || model.is_err() {
        println!("Skipping legacy LLM integration test: Missing OpenRouter credentials");
        return;
    }

    // Just run the text completion test as a quick sanity check
    let api_key = api_key.unwrap();
    let endpoint = endpoint.unwrap();
    let model_name = model.unwrap();

    let endpoint = endpoint.trim_end_matches('/').to_string();
    let endpoint = if endpoint.ends_with("/api/v1") {
        endpoint
    } else if endpoint.ends_with("/api") {
        format!("{}/v1", endpoint)
    } else {
        format!("{}/v1", endpoint)
    };

    let client: openrouter::Client = openrouter::Client::builder()
        .api_key(api_key)
        .base_url(endpoint.as_str())
        .build()
        .expect("Failed to build OpenRouter client");

    let model = client.completion_model(&model_name);
    let llm_client = create_llm_client(model);

    let result = llm_client.complete("Say 'test passed'").await;
    assert!(
        result.is_ok(),
        "Legacy LLM integration test failed: {:?}",
        result.err()
    );
    println!("✓ Legacy LLM integration test passed");
}

// ============================================================================
// Legacy Office Format Tests (PPT, DOC)
// Test text extraction from legacy binary Office formats
// ============================================================================

/// Test legacy PPT conversion - extracts text from binary PowerPoint format
#[tokio::test]
async fn test_legacy_ppt_conversion() {
    let converter = MarkItDown::new();
    let ppt_path = "tests/test_documents/legacy_office/simple.ppt";

    let ppt_data = std::fs::read(ppt_path).expect("Failed to load test PPT");
    println!("Loaded test PPT: {} bytes", ppt_data.len());

    let options = ConversionOptions::new().with_extension(".ppt");
    let result = converter
        .convert_bytes(Bytes::from(ppt_data), Some(options))
        .await;

    match result {
        Ok(doc) => {
            let markdown = doc.to_markdown();
            let images = doc.images();
            println!(
                "=== Legacy PPT Conversion Output ({} chars, {} images) ===",
                markdown.len(),
                images.len()
            );
            println!("{}", safe_truncate(&markdown, 2000));
            if !images.is_empty() {
                println!("\n--- Extracted Images ---");
                for img in &images {
                    println!(
                        "  - {}: {} bytes, type: {}",
                        img.id,
                        img.data.len(),
                        img.mime_type
                    );
                }
            }
            println!("=== End of PPT Output ===\n");

            // Check if output contains garbled binary data (common in legacy formats)
            let has_garbled = markdown.contains("[Content_Types]")
                || markdown.contains("\\x")
                || markdown.contains("PK")
                || markdown
                    .chars()
                    .filter(|c| !c.is_ascii_graphic() && !c.is_whitespace())
                    .count()
                    > 50;

            if has_garbled {
                println!("⚠ WARNING: PPT output contains garbled/binary data - legacy format parsing issue detected!");
            }

            assert!(
                !markdown.is_empty(),
                "PPT conversion should produce some output"
            );
        }
        Err(e) => {
            eprintln!("PPT conversion failed: {}", e);
            panic!("PPT conversion failed: {}", e);
        }
    }
}

/// Test legacy DOC conversion - extracts text from binary Word format
#[tokio::test]
async fn test_legacy_doc_conversion() {
    let converter = MarkItDown::new();
    let doc_path = "tests/test_documents/legacy_office/unit_test_lists.doc";

    let doc_data = std::fs::read(doc_path).expect("Failed to load test DOC");
    println!("Loaded test DOC: {} bytes", doc_data.len());

    let options = ConversionOptions::new().with_extension(".doc");
    let result = converter
        .convert_bytes(Bytes::from(doc_data), Some(options))
        .await;

    match result {
        Ok(doc) => {
            let markdown = doc.to_markdown();
            let images = doc.images();
            println!(
                "=== Legacy DOC Conversion Output ({} chars, {} images) ===",
                markdown.len(),
                images.len()
            );
            println!("{}", safe_truncate(&markdown, 2000));
            if !images.is_empty() {
                println!("\n--- Extracted Images ---");
                for img in &images {
                    println!(
                        "  - {}: {} bytes, type: {}",
                        img.id,
                        img.data.len(),
                        img.mime_type
                    );
                }
            }
            println!("=== End of DOC Output ===\n");

            // Check if we got meaningful text or just binary garbage
            let printable_chars: usize = markdown
                .chars()
                .filter(|c| c.is_alphanumeric() || c.is_whitespace())
                .count();
            let total_chars = markdown.len();
            let quality_ratio = printable_chars as f64 / total_chars as f64;

            println!(
                "Text quality ratio: {:.2}% printable characters",
                quality_ratio * 100.0
            );

            if quality_ratio < 0.5 {
                println!("⚠ WARNING: DOC output has low text quality - legacy format parsing issue detected!");
            }

            assert!(
                !markdown.is_empty(),
                "DOC conversion should produce some output"
            );
        }
        Err(e) => {
            eprintln!("DOC conversion failed: {}", e);
            panic!("DOC conversion failed: {}", e);
        }
    }
}

// ============================================================================
// PPTX Image Description Tests (with LLM)
// ============================================================================

/// Test PPTX with images - extract and describe images using Azure LLM
#[tokio::test]
async fn test_azure_pptx_with_images() {
    let _ = dotenvy::dotenv();

    let api_key = env::var("AZURE_API_KEY");
    let endpoint = env::var("AZURE_ENDPOINT");
    let api_version = env::var("AZURE_API_VERSION").or_else(|_| env::var("AZURE_VERSION"));
    let deployment_id = azure_deployment_id();

    if api_key.is_err() || endpoint.is_err() || api_version.is_err() || deployment_id.is_err() {
        println!("Skipping Azure PPTX image test: Missing environment variables");
        return;
    }

    let api_key = api_key.unwrap();
    let endpoint = endpoint.unwrap();
    let api_version = api_version.unwrap();
    let deployment_id = deployment_id.unwrap();

    println!(
        "Running Azure PPTX with images test:\n  endpoint: {}\n  deployment: {}",
        endpoint, deployment_id
    );

    let client = azure_build_client(api_key, endpoint, &api_version);

    let model = client.completion_model(&deployment_id);
    let llm_client = create_llm_client(model);

    // Test powerpoint_with_image.pptx
    let pptx_path = "tests/test_documents/presentations/powerpoint_with_image.pptx";
    let pptx_data = std::fs::read(pptx_path).expect("Failed to load powerpoint_with_image.pptx");
    println!("Loaded PPTX: {} bytes", pptx_data.len());

    let converter = MarkItDown::new();
    let options = ConversionOptions::new()
        .with_extension(".pptx")
        .with_llm(llm_client)
        .with_images(true);

    println!("Converting PPTX with LLM image descriptions...");
    let result = converter
        .convert_bytes(Bytes::from(pptx_data), Some(options))
        .await;

    match result {
        Ok(doc) => {
            let markdown = doc.to_markdown();
            println!("✓ Azure PPTX Conversion ({} chars):", markdown.len());
            println!("{}", safe_truncate(&markdown, 3000));

            // Check for image placeholders or descriptions
            let has_images = markdown.contains("[Image")
                || markdown.contains("![")
                || markdown.contains("*[Image:");
            println!("\nImage references found: {}", has_images);

            assert!(
                !markdown.is_empty(),
                "PPTX conversion should produce output"
            );
        }
        Err(e) => {
            if should_skip_missing_azure_deployment(
                "Azure PPTX with images test",
                &deployment_id,
                &e,
            ) {
                return;
            }
            eprintln!("PPTX conversion failed: {}", e);
            panic!("PPTX conversion failed: {}", e);
        }
    }
}

/// Test pitch deck PPTX - extract and describe images using Azure LLM
#[tokio::test]
async fn test_azure_pitch_deck_pptx() {
    let _ = dotenvy::dotenv();

    let api_key = env::var("AZURE_API_KEY");
    let endpoint = env::var("AZURE_ENDPOINT");
    let api_version = env::var("AZURE_API_VERSION").or_else(|_| env::var("AZURE_VERSION"));
    let deployment_id = azure_deployment_id();

    if api_key.is_err() || endpoint.is_err() || api_version.is_err() || deployment_id.is_err() {
        println!("Skipping Azure pitch deck PPTX test: Missing environment variables");
        return;
    }

    let api_key = api_key.unwrap();
    let endpoint = endpoint.unwrap();
    let api_version = api_version.unwrap();
    let deployment_id = deployment_id.unwrap();

    println!(
        "Running Azure pitch deck PPTX test:\n  endpoint: {}\n  deployment: {}",
        endpoint, deployment_id
    );

    let client = azure_build_client(api_key, endpoint, &api_version);

    let model = client.completion_model(&deployment_id);
    let llm_client = create_llm_client(model);

    // Test pitch_deck_presentation.pptx
    let pptx_path = "tests/test_documents/presentations/pitch_deck_presentation.pptx";
    let pptx_data = std::fs::read(pptx_path).expect("Failed to load pitch_deck_presentation.pptx");
    println!("Loaded pitch deck PPTX: {} bytes", pptx_data.len());

    let converter = MarkItDown::new();
    let options = ConversionOptions::new()
        .with_extension(".pptx")
        .with_llm(llm_client)
        .with_images(true);

    println!("Converting pitch deck PPTX with LLM image descriptions...");
    let result = converter
        .convert_bytes(Bytes::from(pptx_data), Some(options))
        .await;

    match result {
        Ok(doc) => {
            let markdown = doc.to_markdown();
            println!(
                "✓ Azure Pitch Deck PPTX Conversion ({} chars):",
                markdown.len()
            );
            println!("{}", safe_truncate(&markdown, 3000));

            // Check for image placeholders or descriptions
            let has_images = markdown.contains("[Image")
                || markdown.contains("![")
                || markdown.contains("*[Image:");
            println!("\nImage references found: {}", has_images);

            // Count pages/slides
            let page_count = markdown.matches("Page:").count();
            println!("Pages/Slides found: {}", page_count);

            assert!(
                !markdown.is_empty(),
                "Pitch deck PPTX conversion should produce output"
            );
        }
        Err(e) => {
            if should_skip_missing_azure_deployment(
                "Azure pitch deck PPTX test",
                &deployment_id,
                &e,
            ) {
                return;
            }
            eprintln!("Pitch deck PPTX conversion failed: {}", e);
            panic!("Pitch deck PPTX conversion failed: {}", e);
        }
    }
}

// ============================================================================
// PPTX without LLM (baseline extraction test)
// ============================================================================

/// Test PPTX extraction without LLM (baseline)
#[tokio::test]
async fn test_pptx_extraction_without_llm() {
    let converter = MarkItDown::new();

    // Test powerpoint_with_image.pptx
    let pptx_path = "tests/test_documents/presentations/powerpoint_with_image.pptx";
    let pptx_data = std::fs::read(pptx_path).expect("Failed to load powerpoint_with_image.pptx");
    println!("Loaded PPTX: {} bytes", pptx_data.len());

    let options = ConversionOptions::new()
        .with_extension(".pptx")
        .with_images(true);

    println!("Converting PPTX without LLM (baseline extraction)...");
    let result = converter
        .convert_bytes(Bytes::from(pptx_data), Some(options))
        .await;

    match result {
        Ok(doc) => {
            let markdown = doc.to_markdown();
            println!(
                "=== PPTX Baseline Extraction ({} chars) ===",
                markdown.len()
            );
            println!("{}", markdown);
            println!("=== End of PPTX Output ===\n");

            // Check extraction quality
            let has_garbled = markdown.contains("[Content_Types]")
                || markdown
                    .chars()
                    .filter(|c| !c.is_ascii_graphic() && !c.is_whitespace())
                    .count()
                    > 50;

            if has_garbled {
                println!("⚠ WARNING: PPTX output contains garbled data");
            } else {
                println!("✓ PPTX extraction looks clean");
            }

            // Check for image placeholders
            let image_refs = markdown.matches("[Image").count() + markdown.matches("![").count();
            println!("Image references found: {}", image_refs);

            assert!(
                !markdown.is_empty(),
                "PPTX conversion should produce output"
            );
        }
        Err(e) => {
            eprintln!("PPTX conversion failed: {}", e);
            panic!("PPTX conversion failed: {}", e);
        }
    }
}

/// Test pitch deck PPTX extraction without LLM (baseline)
#[tokio::test]
async fn test_pitch_deck_pptx_extraction_without_llm() {
    let converter = MarkItDown::new();

    let pptx_path = "tests/test_documents/presentations/pitch_deck_presentation.pptx";
    let pptx_data = std::fs::read(pptx_path).expect("Failed to load pitch_deck_presentation.pptx");
    println!("Loaded pitch deck PPTX: {} bytes", pptx_data.len());

    let options = ConversionOptions::new()
        .with_extension(".pptx")
        .with_images(true);

    println!("Converting pitch deck PPTX without LLM (baseline extraction)...");
    let result = converter
        .convert_bytes(Bytes::from(pptx_data), Some(options))
        .await;

    match result {
        Ok(doc) => {
            let markdown = doc.to_markdown();
            println!(
                "=== Pitch Deck PPTX Baseline Extraction ({} chars) ===",
                markdown.len()
            );
            println!("{}", safe_truncate(&markdown, 5000));
            println!("=== End of Pitch Deck Output ===\n");

            // Check extraction quality
            let has_garbled = markdown.contains("[Content_Types]")
                || markdown
                    .chars()
                    .filter(|c| !c.is_ascii_graphic() && !c.is_whitespace())
                    .count()
                    > 50;

            if has_garbled {
                println!("⚠ WARNING: Pitch deck output contains garbled data");
            } else {
                println!("✓ Pitch deck extraction looks clean");
            }

            // Count slides
            let page_count = markdown.matches("Page:").count();
            println!("Pages/Slides found: {}", page_count);

            assert!(
                !markdown.is_empty(),
                "Pitch deck conversion should produce output"
            );
        }
        Err(e) => {
            eprintln!("Pitch deck conversion failed: {}", e);
            panic!("Pitch deck conversion failed: {}", e);
        }
    }
}
