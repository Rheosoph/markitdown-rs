# Architecture & Development Guide

This guide explains how markitdown-rs works internally and how to extend it.

## Core Architecture

### High-Level Flow

```
Input File/Bytes
       ↓
Extension Detection
       ↓
Converter Selection
       ↓
Parse → Convert → Markdown
       ↓
Document Model
       ↓
Output Markdown
```

### Data Model

The library uses a simple hierarchical model:

```rust
Document
├── Page (n)
│   └── ContentBlock (markdown or image)
└── Metadata (title, author, etc.)
```

**Key Types:**
- `Document`: Top-level container for pages and metadata
- `Page`: Represents a logical page with content blocks
- `ContentBlock`: Either Markdown text or an extracted image

### Converter Pattern

Every format converter implements the `DocumentConverter` trait:

```rust
#[async_trait]
pub trait DocumentConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError>;

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError>;

    fn supported_extensions(&self) -> &[&str];
}
```

**Key Points:**
- Converters are async (for I/O operations)
- Support both file-path and bytes-based input
- Declare their supported file extensions
- Return a structured `Document` with pages and content

## Converter Implementation Guide

### Minimal Example: Text Converter

```rust
use async_trait::async_trait;
use bytes::Bytes;
use crate::model::{ContentBlock, Document, DocumentConverter, Page};

pub struct TextConverter;

#[async_trait]
impl DocumentConverter for TextConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let result = store.get(path).await?;
        let bytes = result.bytes().await?;
        self.convert_bytes(bytes, options).await
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        _options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let text = String::from_utf8(bytes.to_vec())?;
        let mut document = Document::new();
        let mut page = Page::new(1);
        
        page.add_content(ContentBlock::Markdown(text));
        document.add_page(page);
        
        Ok(document)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".txt"]
    }
}
```

### Complex Example: Parsing with External Library

For formats requiring parsing (XML, JSON, binary formats), follow this pattern:

```rust
pub struct MyConverter;

impl MyConverter {
    fn parse_content(bytes: &[u8]) -> Result<String, MarkitdownError> {
        // 1. Parse the format
        let parsed = my_parser::parse(bytes)?;
        
        // 2. Extract meaningful content
        let mut markdown = String::new();
        for section in parsed.sections {
            markdown.push_str(&format!("# {}\n\n", section.title));
            markdown.push_str(&section.text);
            markdown.push('\n');
        }
        
        // 3. Return markdown
        Ok(markdown)
    }
}

#[async_trait]
impl DocumentConverter for MyConverter {
    async fn convert_bytes(
        &self,
        bytes: Bytes,
        _options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let markdown = Self::parse_content(&bytes)?;
        let mut document = Document::new();
        let mut page = Page::new(1);
        page.add_content(ContentBlock::Markdown(markdown));
        document.add_page(page);
        Ok(document)
    }

    // ... implement other trait methods
}
```

### Multi-Page Format Example: Presentations

For formats with multiple logical pages:

```rust
#[async_trait]
impl DocumentConverter for PresentationConverter {
    async fn convert_bytes(
        &self,
        bytes: Bytes,
        _options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let parsed = parse_presentation(&bytes)?;
        let mut document = Document::new();
        
        // One page per slide
        for (page_num, slide) in parsed.slides.iter().enumerate() {
            let mut page = Page::new((page_num + 1) as u32);
            
            // Add title
            if let Some(title) = &slide.title {
                page.add_content(ContentBlock::Markdown(format!("# {}\n", title)));
            }
            
            // Add content
            page.add_content(ContentBlock::Markdown(slide.text.clone()));
            
            // Add images
            for image in &slide.images {
                page.add_content(ContentBlock::Image(image.clone()));
            }
            
            document.add_page(page);
        }
        
        Ok(document)
    }

    // ... other methods
}
```

## Working with Images

Converters can extract images and include them in the document:

```rust
// Extract an image
let image = ExtractedImage {
    data: Bytes::from(image_bytes),
    media_type: "image/png".to_string(),
    alt_text: Some("Description".to_string()),
};

// Add to page
page.add_content(ContentBlock::Image(image));
```

When the document is serialized to Markdown with `.to_markdown()`, images are saved to disk and referenced as Markdown image links.

## LLM Modes: Extraction and Summarization

`ConversionOptions::mode` selects what an LLM is used for:

- **`ConversionMode::Extract`** (default): converters produce the full content. Pages that need vision are transcribed by the LLM, and embedded images get descriptions.
- **`ConversionMode::Summarize(SummaryConfig)`**: converters only *read* the document, and `MarkItDown::convert` then runs `summarize::summarize_document` on the result. Pages go to the LLM as input, and it writes page notes plus a `DocumentSummary`, so nothing is transcribed.

Converters follow two rules so that both modes work:

1. Describe embedded images only through `options.image_description_llm()`. It returns `None` in summarization mode, because the summarizer sends images to the LLM together with their page.
2. A page that can only be understood visually gets a `rendered_image` in summarization mode instead of an LLM transcription. `PdfConverter` does this for scanned pages, garbled text and figure-heavy pages, rendered on white at `SummaryConfig::page_image_max_edge`.

```rust
if let Some(llm) = opts.image_description_llm() {
    document = document.with_image_descriptions(llm.as_ref()).await?;
}
```

The summarizer (`src/summarize.rs`) works on any `Document`:

1. It turns pages into prompt parts (page render first, then text with embedded images inline), removing running headers and footers.
2. It packs them into windows and has the LLM write notes per page, either concurrently (`MapReduce`) or in order with a running summary (`Sequential`).
3. It prunes the notes, then condenses them only if needed.
4. One final request writes the summary.

A document that fits one window takes a single request. The LLM calls go through `LlmClient::complete_multimodal`, which takes ordered text and image parts and reports token usage.

## Error Handling

Use the `MarkitdownError` enum for all errors:

```rust
use crate::error::MarkitdownError;

// Parse errors
Err(MarkitdownError::ParseError("Invalid format".to_string()))

// Format not supported
Err(MarkitdownError::UnsupportedFormat("Format XYZ not recognized".to_string()))

// File I/O
Err(MarkitdownError::IoError(io_error))

// Encoding issues
Err(MarkitdownError::EncodingError("UTF-8 decode failed".to_string()))
```

## Testing Converters

### LLM Integration Tests

LLM-related tests live in `tests/llm.rs` and are treated as integration tests.

- The test always tries to load a local `.env` first (project root) so you can keep API credentials out of your shell.
- The test expects these variables for an OpenRouter (OpenAI-compatible) client:
    - `OPENROUTER_API_KEY`
    - `OPENROUTER_ENDPOINT`
    - `OPENROUTER_MODEL`
- If the `.env` file is missing or any of the variables are not set, the test is skipped.

Run it with:

```bash
cargo test --test llm -- --nocapture
```

Each converter should have a test file in `tests/<format>.rs`:

```rust
use bytes::Bytes;
use markitdown::{ConversionOptions, MarkItDown};
use std::fs;

fn default_options(ext: &str) -> ConversionOptions {
    ConversionOptions {
        file_extension: Some(ext.to_string()),
        url: None,
        llm_client: None,
        extract_images: true,
        force_llm_ocr: false,
        merge_multipage_tables: false,
        ..Default::default()
    }
}

#[tokio::test]
async fn test_basic_conversion() {
    let md = MarkItDown::new();
    let result = md
        .convert(
            "tests/test_documents/myformat/basic.xyz",
            Some(default_options(".xyz")),
        )
        .await;

    assert!(result.is_ok());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty());
    assert!(content.contains("expected text"));
}

#[tokio::test]
async fn test_bytes_conversion() {
    let md = MarkItDown::new();
    let bytes = fs::read("tests/test_documents/myformat/basic.xyz")
        .expect("Failed to read test file");
    
    let result = md
        .convert_bytes(Bytes::from(bytes), Some(default_options(".xyz")))
        .await;

    assert!(result.is_ok());
}
```

## File Organization

```
src/
├── lib.rs                 # Main entry point, converter registration
├── model.rs               # Core data types (Document, Page, ContentBlock)
├── error.rs               # Error types
├── <format>.rs            # One module per format (csv.rs, html.rs, etc.)
└── ...

tests/
├── <format>.rs            # Tests for each converter
├── test_documents/        # Test fixtures
│   ├── csv/
│   ├── html/
│   └── ...
└── ...

docs/
├── FORMATS.md             # Supported formats reference
├── ARCHITECTURE.md        # This file
└── ...
```

## Adding a New Format

### Step 1: Create the Converter

Create `src/myformat.rs`:

```rust
//! MyFormat to Markdown converter.

use async_trait::async_trait;
use bytes::Bytes;
use crate::model::{ContentBlock, ConversionOptions, Document, DocumentConverter, Page};
use crate::error::MarkitdownError;

/// MyFormat converter
pub struct MyFormatConverter;

#[async_trait]
impl DocumentConverter for MyFormatConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let result = store.get(path).await?;
        let bytes = result.bytes().await?;
        self.convert_bytes(bytes, options).await
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        _options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        // Parse and convert to markdown
        let markdown = self.parse(&bytes)?;
        
        let mut document = Document::new();
        let mut page = Page::new(1);
        page.add_content(ContentBlock::Markdown(markdown));
        document.add_page(page);
        
        Ok(document)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".myformat", ".myfmt"]
    }
}

impl MyFormatConverter {
    fn parse(&self, bytes: &[u8]) -> Result<String, MarkitdownError> {
        // Implementation
        Ok(String::new())
    }
}
```

### Step 2: Register in lib.rs

```rust
mod myformat;
use myformat::MyFormatConverter;

impl MarkItDown {
    pub fn with_store(store: Arc<dyn ObjectStore>) -> Self {
        let mut md = MarkItDown { /* ... */ };
        
        // Add your converter
        md.register_converter(Box::new(MyFormatConverter));
        
        md
    }
}
```

### Step 3: Add Test Fixtures

Create `tests/test_documents/myformat/` with sample files.

### Step 4: Create Tests

Create `tests/myformat.rs` with test cases (see Testing section above).

### Step 5: Run Tests

```bash
cargo test myformat
```

## Performance Considerations

1. **Async I/O**: Use async/await for file operations
2. **Streaming**: For large files, parse incrementally rather than loading entirely into memory
3. **Caching**: The `ObjectStore` handles file caching
4. **Image Extraction**: Optional `extract_images` flag for expensive operations

## Dependencies

Key crates used:

- `bytes` – Efficient byte buffer handling
- `tokio` – Async runtime
- `async-trait` – Async trait methods
- `serde` – Serialization (JSON, YAML, TOML)
- `quick-xml` – Fast XML parsing
- `zip` – Archive handling
- `pdf` – PDF extraction

Add dependencies to `Cargo.toml` as needed for new format support.

## Debugging

Enable debug logging:

```rust
use log::debug;

debug!("Parsing format, found {} sections", sections.len());
```

Run tests with logs:

```bash
RUST_LOG=debug cargo test -- --nocapture
```

## Common Pitfalls

1. **Forgetting `#[async_trait]`**: Required for async trait methods
2. **Not handling UTF-8 errors**: Use `.from_utf8_lossy()` or handle errors
3. **Incomplete extension list**: Declare all supported extensions in `supported_extensions()`
4. **Memory usage**: Stream large files instead of loading entirely
5. **Not testing edge cases**: Test with corrupted, empty, and malformed files

## Reference

- [API Documentation](../README.md)
- [Supported Formats](./FORMATS.md)
- [Error Types](../src/error.rs)
- [Model Types](../src/model.rs)
