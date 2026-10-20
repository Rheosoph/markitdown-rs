# markitdown-rs

A high-performance Rust library that converts **40+ document formats** to clean, readable Markdown. Perfect for preparing documents for LLM consumption, documentation generation, knowledge bases, or archival.

🚀 **Rust implementation** of the original [markitdown](https://github.com/microsoft/markitdown) Python library with extensive format support and async-first design.

[![Rust](https://img.shields.io/badge/Rust-1.70+-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Tests](https://img.shields.io/badge/tests-198%20passing-brightgreen.svg)](#testing)

## ✨ Features

- **40+ Format Support**: Word, Excel, PowerPoint (modern & legacy), PDF, EPUB, HTML, Markdown, LaTeX, and more
- **Async-First Design**: Non-blocking I/O with Tokio runtime
- **Archive Extraction**: Automatically extract and convert ZIP, TAR, GZIP, and more
- **Image Extraction**: Optional intelligent image extraction with LLM-powered descriptions
- **LLM Integration**: Works with OpenAI, Gemini, Claude, Cohere, and custom providers
- **Summarization Mode**: Page notes and a cited summary instead of a full transcription, at a fraction of the output tokens
- **Streaming Support**: Process large files efficiently
- **Rich Output Structure**: Preserves pagination, images, tables, and metadata
- **Production-Ready**: Comprehensive test suite with 198+ passing tests

## 📋 Supported Formats

### Document Formats

**Microsoft Office (Modern)**
- [x] Word (.docx, .dotx, .dotm)
- [x] Excel (.xlsx, .xltx, .xltm)
- [x] PowerPoint (.pptx, .potx, .potm)

**Microsoft Office (Legacy)**
- [x] Word 97-2003 (.doc)
- [x] Excel 97-2003 (.xls)
- [x] PowerPoint 97-2003 (.ppt)
- [x] Rich Text Format (.rtf)

**OpenDocument Format**
- [x] Text (.odt, .ott)
- [x] Spreadsheet (.ods, .ots)
- [x] Presentation (.odp, .otp)

**Apple iWork**
- [x] Pages (.pages)
- [x] Numbers (.numbers)
- [x] Keynote (.key)

**Other Document Formats**
- [x] PDF (.pdf)
  - **Intelligent fallback mechanism**: Automatically detects scanned PDFs, complex pages with diagrams, or pages with limited text and images
  - Uses text extraction by default for efficiency
  - Falls back to LLM-powered page rendering when:
    - Page has < 10 words (likely scanned)
    - Low alphanumeric ratio < 0.5 (OCR artifacts/garbage)
    - Unstructured content < 50 characters
    - Page contains images + < 350 words (provides full context to LLM)
  - Renders entire page as PNG for LLM processing when needed
- [x] EPUB (.epub)
- [x] Markdown (.md)

### Data Formats

- [x] CSV (.csv)
- [x] Excel spreadsheets (.xlsx, .xls)
- [x] SQLite databases (.sqlite, .db)

### Structured Data

- [x] XML (.xml)
- [x] RSS feeds (.rss, .atom)
- [x] HTML (.html, .htm)
- [x] Email (.eml, .msg)
- [x] vCard (.vcf)
- [x] iCalendar (.ics)
- [x] BibTeX (.bib)

### Archive Formats

- [x] ZIP (.zip)
- [x] TAR (.tar, .tar.gz, .tar.bz2, .tar.xz)
- [x] GZIP (.gz)
- [x] BZIP2 (.bz2)
- [x] XZ (.xz)
- [x] ZSTD (.zst)
- [x] 7-Zip (.7z)

### Media

- [x] Images (.jpg, .png, .gif, .bmp, .tiff, .webp)
  - With LLM integration for intelligent image descriptions
- [ ] Audio (planned)

### Other

- [x] Plain text (.txt)
- [x] Log files (.log)

> **Note:** All formats support both file path and in-memory bytes conversion.

## Usage

### Command-Line

#### Installation

```
cargo install markitdown
```

#### Convert a File

```
markitdown path-to-file.pdf
```

Or use -o to specify the output file:

```
markitdown path-to-file.pdf -o document.md
```

Supported formats include Office documents (.docx, .xlsx, .pptx), legacy Office (.doc, .xls, .ppt), OpenDocument (.odt, .ods), Apple iWork (.pages, .numbers, .key), PDFs, EPUB, images, archives, and more. See the full list above.

### Rust API

#### Installation

Add the following to your `Cargo.toml`:

```toml
[dependencies]
markitdown = "0.1.10"
```

#### TLS Backend Selection

By default, `markitdown` uses the platform's **native TLS** implementation (Security.framework on macOS, SChannel on Windows, OpenSSL on Linux). You can switch to a pure-Rust TLS backend (**rustls**) which removes the system OpenSSL dependency — useful for cross-compilation and reproducible builds.

```toml
# Default: native TLS (same as omitting features)
markitdown = { version = "0.1.10", features = ["native-tls"] }

# Pure-Rust TLS via rustls (no system OpenSSL needed)
markitdown = { version = "0.1.10", default-features = false, features = ["rustls-tls"] }

# Force a vendored static liblzma build for XZ/LZMA archive support
markitdown = { version = "0.1.10", features = ["native-tls", "static-liblzma"] }
```

| Feature | TLS Backend | System dependency |
|---|---|---|
| `native-tls` *(default)* | OS-native (Security.framework / SChannel / OpenSSL) | OpenSSL on Linux |
| `rustls-tls` | rustls (pure Rust) | None |

`static-liblzma` is an optional build feature for archive support. It forces `liblzma` to build from bundled sources instead of using a system `liblzma`, which is useful for reproducible CI builds and mobile or cross-compiled targets.

#### Initialize MarkItDown

```rust
use markitdown::MarkItDown;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let md = MarkItDown::new();
    Ok(())
}
```

#### Convert a File

```rust
use markitdown::{ConversionOptions, MarkItDown};
use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let md = MarkItDown::new();
    
    // Create a local file system object store
    let store = Arc::new(LocalFileSystem::new());
    
    // Convert file path string to ObjectStore Path
    let path = ObjectPath::from("path/to/file.xlsx");

    // Basic conversion - file type is auto-detected from extension
    let result = md.convert_with_store(store.clone(), &path, None).await?;
    println!("Converted Text: {}", result.to_markdown());

    // Convert legacy Office formats
    let doc_path = ObjectPath::from("document.doc");
    let result = md.convert_with_store(store.clone(), &doc_path, None).await?;

    // Convert archives (extracts and converts contents)
    let zip_path = ObjectPath::from("archive.zip");
    let result = md.convert_with_store(store.clone(), &zip_path, None).await?;

    // Or explicitly specify options
    let options = ConversionOptions::default()
        .with_extension(".xlsx")
        .with_extract_images(true);

    let result = md.convert_with_store(store, &path, Some(options)).await?;
    
    Ok(())
}
```

> **Important:** The library uses `object_store` for file operations, not plain file paths. You must:
> 1. Create an `ObjectStore` implementation (like `LocalFileSystem` for local files)
> 2. Convert file path strings to `object_store::path::Path` using `Path::from()`
> 3. Use `convert_with_store()` method with the store and path
>
> For convenience, there's also a `convert()` method that accepts string paths and uses `LocalFileSystem` internally.

#### Convert with LLM for Image Descriptions

```rust
use markitdown::{ConversionOptions, MarkItDown, create_llm_client};
use rig::providers::openai;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let md = MarkItDown::new();
    
    // Create an LLM client using any rig-core compatible provider
    // OpenAI example:
    let openai_client = openai::Client::from_env()?;
    let model = openai_client.completion_model("gpt-4o");
    let llm = create_llm_client(model);
    
    // Google Gemini example:
    // let gemini_client = gemini::Client::from_env()?;
    // let model = gemini_client.completion_model("gemini-2.0-flash");
    // let llm = create_llm_client(model);
    
    // Anthropic Claude example:
    // let anthropic_client = anthropic::Client::from_env()?;
    // let model = anthropic_client.completion_model("claude-sonnet-4-20250514");
    // let llm = create_llm_client(model);
    
    // Cohere example with custom endpoint:
    // let api_key = std::env::var("COHERE_API_KEY")?;
    // let mut builder = rig::providers::cohere::Client::builder(&api_key);
    // if let Some(endpoint) = custom_endpoint {
    //     builder = builder.base_url(endpoint);
    // }
    // let client = builder.build();
    // let model = client.completion_model("command-r-plus");
    // let llm = create_llm_client(model);

    let options = ConversionOptions::default()
        .with_extension(".jpg")
        .with_llm(llm);

    let result = md.convert("path/to/image.jpg", Some(options)).await?;
    println!("Image description: {}", result.to_markdown());
    
    Ok(())
}
```

**Environment Variables for LLM Tests (OpenRouter):**

The integration test in `tests/llm.rs` expects these variables (via `.env` or your shell):

```bash
export OPENROUTER_API_KEY="your_api_key"
export OPENROUTER_ENDPOINT="https://openrouter.ai/api/v1"
export OPENROUTER_MODEL="@preset/prod-free"
```

If any of them are missing, the LLM test is skipped.

**Supported LLM Providers** (via rig-core):
- OpenAI (GPT-4, GPT-4o, etc.)
- Google Gemini (gemini-2.0-flash, gemini-pro, etc.)
- Anthropic Claude (claude-sonnet, claude-opus, etc.)
- Cohere (command-r-plus, etc.)
- Any custom provider implementing `CompletionModel`

**Temperature:** requests use a low temperature (0.1) by default for faithful extraction. Some models accept only their default temperature, reasoning models for example. If a request is rejected because of its temperature, it is retried without one, and later requests leave it out. To always use the provider's default, use `LlmConfig::default().without_temperature()`. Google recommends this for Gemini 3, where temperatures below 1.0 can cause looping.

#### Summarize Documents with an LLM

By default, conversion runs in **extraction mode**: every page that needs vision is transcribed by the LLM and every embedded image gets a detailed description, so the LLM *generates* about as many tokens as the document contains. **Summarization mode** turns this around. Pages go to the LLM as *input* (extracted text, rendered page images, embedded images), and the LLM writes only short notes per page plus a summary. Output tokens cost several times more than input tokens and dominate latency, so this is much cheaper and faster when you need to understand a document rather than reproduce it.

```rust
use markitdown::{ConversionOptions, MarkItDown, SummaryConfig};

let options = ConversionOptions::default()
    .with_llm(llm) // any client from create_llm_client
    .with_summarization(
        SummaryConfig::default()
            .with_focus("financial risks") // optional
            .with_language("English"),     // default: the document's language
    );

let doc = MarkItDown::new().convert("annual_report.pdf", Some(options)).await?;

let summary = doc.summary.as_ref().unwrap();
println!("{}", summary.summary);
for point in &summary.key_points {
    println!("- {}", point); // e.g. "Revenue grew 12% to €4.1M [p. 7]"
}

// Each page now holds the LLM's notes for that page instead of its full content
for page in &doc.pages {
    println!("Page {}:\n{}", page.page_number, page.to_markdown());
}

println!(
    "{} requests, {} input / {} output tokens",
    summary.usage.requests, summary.usage.input_tokens, summary.usage.output_tokens
);

// Or everything as markdown: summary, key points, then the notes per page
let markdown = doc.to_markdown();
```

How it works:

1. **Read.** Pages are packed into windows of about 10 pages or 16K tokens. Pages with good text are sent as text, scanned pages as images, and pages with figures as both. Embedded images from DOCX, PPTX and similar formats are sent with their page, so they are never described one by one. For each window, the LLM writes concise notes per page, most important first. Notes get shorter per page for long documents so that they all fit the final step.
2. **Reduce.** If the notes still exceed the final step's budget, the least important points are dropped first. Only if that is not enough are notes condensed hierarchically, at most two levels.
3. **Summarize.** One final request writes the summary and key points, with page citations.

A document that fits in one window is read and summarized in a single request. Running headers and footers are removed before reading. With `with_force_llm_ocr(true)`, every PDF page is also sent as an image.

| Strategy | Behavior |
|---|---|
| `SummaryStrategy::MapReduce` *(default)* | Windows are read concurrently. Fastest. |
| `SummaryStrategy::Sequential` | Windows are read in order, and each receives a running summary of the pages before it (Chain of Agents). Slower, but better for narratives and documents that build on themselves. |

The design follows current long-document summarization research: per-page gists (ReadAgent), parallel map-reduce vs. sequential agents (Chain of Agents), pruning before hierarchical merging to avoid compounding hallucinations, and sending both text and page image where layout matters. `SummaryConfig` exposes the budgets (`summary_words`, `max_key_points`, `note_words_per_page`, window sizes, `max_notes_tokens`), concurrency, retries and all four system prompts. Summarization mode requires an LLM client; scanned pages and images need a vision-capable model. See `examples/summarize_pdf.rs`.

#### Convert from Bytes

```rust
use markitdown::{ConversionOptions, MarkItDown};
use bytes::Bytes;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let md = MarkItDown::new();
    
    let file_bytes = std::fs::read("path/to/file.pdf")?;

    // Auto-detect file type from bytes
    let result = md.convert_bytes(Bytes::from(file_bytes.clone()), None).await?;
    println!("Converted: {}", result.to_markdown());

    // Or specify options explicitly
    let options = ConversionOptions::default()
        .with_extension(".pdf");

    let result = md.convert_bytes(Bytes::from(file_bytes), Some(options)).await?;
    
    Ok(())
}
```

#### Working with the Output Structure

The conversion returns a `Document` struct that preserves the page/slide structure of the original file:

```rust
use markitdown::{MarkItDown, Document, Page, ContentBlock, ExtractedImage};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let md = MarkItDown::new();
    let result: Document = md.convert("presentation.pptx", None).await?;
    
    // Access document metadata
    if let Some(title) = &result.title {
        println!("Document: {}", title);
    }
    
    // Iterate through pages/slides
    for page in &result.pages {
        println!("Page {}", page.page_number);
        
        // Get page content as markdown
        let markdown = page.to_markdown();
        
        // Or access individual content blocks
        for block in &page.content {
            match block {
                ContentBlock::Text(text) => println!("Text: {}", text),
                ContentBlock::Heading { level, text } => println!("H{}: {}", level, text),
                ContentBlock::Image(img) => {
                    println!("Image: {} ({} bytes)", img.id, img.data.len());
                    if let Some(desc) = &img.description {
                        println!("  Description: {}", desc);
                    }
                }
                ContentBlock::Table { headers, rows } => {
                    println!("Table: {} cols, {} rows", headers.len(), rows.len());
                }
                ContentBlock::List { ordered, items } => {
                    println!("List ({} items)", items.len());
                }
                ContentBlock::Code { language, code } => {
                    println!("Code block: {:?}", language);
                }
                ContentBlock::Quote(text) => println!("Quote: {}", text),
                ContentBlock::Markdown(md) => println!("Markdown: {}", md),
            }
        }
        
        // Get all images from this page
        let images: Vec<&ExtractedImage> = page.images();
        
        // Access rendered page image (for scanned PDFs, complex pages)
        if let Some(rendered) = &page.rendered_image {
            println!("Page rendered as image: {} bytes", rendered.data.len());
        }
    }
    
    // Convert entire document to markdown (with page separators)
    let full_markdown = result.to_markdown();
    
    // Get all images from the entire document
    let all_images = result.images();
    
    Ok(())
}
```

**Output Structure:**
- `Document` - Complete document with optional title, pages, and metadata
  - `Page` - Single page/slide with page number and content blocks
    - `ContentBlock` - Individual content element (Text, Heading, Image, Table, List, Code, Quote, Markdown)
    - `rendered_image` - Optional full-page render (for scanned PDFs, slides with complex layouts)
  - `ExtractedImage` - Image data with id, bytes, MIME type, dimensions, alt text, and LLM description

This structure is ideal for:
- **Pagination-aware processing** - Handle each page separately
- **Image extraction** - Access embedded images with their metadata
- **Structured content** - Work with tables, lists, headings programmatically
- **LLM pipelines** - Pass individual pages or content blocks to AI models

## Recent Improvements

### Format Expansion
- **40+ new formats** including legacy Office (.doc, .xls, .ppt), OpenDocument (.odt, .ods, .odp), Apple iWork (.pages, .numbers, .key)
- **Archive support** for ZIP, TAR, GZIP, BZIP2, XZ, ZSTD, and 7-Zip with automatic content extraction
- **Additional formats**: EPUB, vCard, iCalendar, BibTeX, log files, SQLite databases, email files

### Performance & Reliability
- **Improved archive portability** by keeping compression support compatible with shared native `lzma` dependencies
- **Improved file detection** - prioritizes file extension over magic byte detection for legacy formats
- **Template support** for Office formats (.dotx, .potx, .xltx)
- **LLM flexibility** - works with any rig-core compatible model (OpenAI, Gemini, Claude, Cohere, custom providers)

### Testing
- **Comprehensive test suite** using real-world files from [Kreuzberg](https://github.com/kreuzberg-dev/kreuzberg)
- Tests for all supported formats with both file and bytes conversion
- In-memory test generation for compression formats

#### Register a Custom Converter

You can extend MarkItDown by implementing the `DocumentConverter` trait for your custom converters and registering them:

```rust
use markitdown::{DocumentConverter, Document, ConversionOptions, MarkItDown};
use markitdown::error::MarkitdownError;
use async_trait::async_trait;
use bytes::Bytes;
use std::sync::Arc;
use object_store::ObjectStore;

struct MyCustomConverter;

#[async_trait]
impl DocumentConverter for MyCustomConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        // Implement file conversion logic
        todo!()
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        // Implement bytes conversion logic
        todo!()
    }
    
    fn supported_extensions(&self) -> &[&str] {
        &[".custom"]
    }
}

let mut md = MarkItDown::new();
md.register_converter(Box::new(MyCustomConverter));
```

## Contributing

Contributions are welcome! Please feel free to submit issues or pull requests.

## 📚 Documentation

For detailed information, see:

- **[FORMATS.md](docs/FORMATS.md)** – Complete reference of all 40+ supported formats with capabilities and limitations
- **[ARCHITECTURE.md](docs/ARCHITECTURE.md)** – Internal design, converter pattern, and how to implement new formats
- **[TESTING.md](docs/TESTING.md)** – Comprehensive testing guide with 198+ test examples
- **[FORMAT_COVERAGE.md](docs/FORMAT_COVERAGE.md)** – Converter matrix with extensions and test locations

### Quick Links

- [API Documentation](#rust-api) – Usage examples and API reference
- [CLI Usage](#command-line) – Command-line tool guide
- [Adding Formats](#register-a-custom-converter) – Extend with custom converters
- [LLM Integration](#convert-with-llm-for-image-descriptions) – Use AI for image descriptions
- [Summarization Mode](#summarize-documents-with-an-llm) – Notes and a summary instead of a full transcription

## Acknowledgments

- Original Python implementation: [microsoft/markitdown](https://github.com/microsoft/markitdown)
- Test files from: [Kreuzberg](https://github.com/kreuzberg-dev/kreuzberg)

## License

MarkItDown is licensed under the MIT License. See `LICENSE` for more details.
