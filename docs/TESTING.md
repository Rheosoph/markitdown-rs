# Testing Guide

Complete documentation on testing markitdown-rs converters and the overall system.

## Running Tests

### Run All Tests
```bash
cargo test
```

### Run Tests for Specific Format
```bash
cargo test csv
cargo test docx
```

### Run a Single Test
```bash
cargo test test_csv_basic
```

### Run with Output
```bash
cargo test -- --nocapture
```

### Run with Backtrace on Failure
```bash
RUST_BACKTRACE=1 cargo test
```

### Run Tests Sequentially (not in parallel)
```bash
cargo test -- --test-threads=1
```

## AI/LLM Testing

The library includes integration tests for AI capabilities (image description, etc.) using OpenRouter (or any OpenAI-compatible provider).

### Prerequisites

To run the LLM tests, you need to set the following environment variables, either in your shell or in a `.env` file in the project root:

```bash
OPENROUTER_API_KEY="your_api_key"
OPENROUTER_ENDPOINT="https://openrouter.ai/api/v1"
OPENROUTER_MODEL="google/gemini-2.0-flash-exp:free"
```

### Running LLM Tests

```bash
cargo test --test llm
```

If the environment variables are missing, the tests will be automatically skipped.

### Summarization Mode Tests

`tests/summarize.rs` covers summarization mode without network access. A scripted `LlmClient` records every request and answers in the tag format the pipeline asks for. The tests check routing (text, page images, embedded images, never transcription), the single-request path for short documents, the concurrent and sequential strategies, failure reporting, and pruning and merging of long notes:

```bash
cargo test --test summarize
```

The live counterparts (`test_openrouter_pdf_summarization`, `test_azure_openai_pdf_summarization`) live in `tests/llm.rs` and are skipped without credentials.

## Test Structure

Tests are organized by format in the `tests/` directory:

```
tests/
├── archive.rs        # ZIP, TAR, GZIP, etc.
├── bibtex_log.rs     # BibTeX and Log files
├── csv.rs            # Comma-separated values
├── docbook.rs        # DocBook XML
├── docx.rs           # Microsoft Word
├── email.rs          # Email (EML, MSG)
├── epub.rs           # EPUB ebooks
├── excel.rs          # Excel spreadsheets
├── fictionbook.rs    # FB2 ebooks
├── html.rs           # HTML web pages
├── image.rs          # Raster images
├── json.rs           # JSON data
├── jupyter.rs        # Jupyter notebooks
├── latex.rs          # LaTeX documents
├── legacy_office.rs  # Old Office formats
├── markdown.rs       # Markdown passthrough
├── opendocument.rs   # ODF documents
├── opml.rs           # OPML outlines
├── orgmode.rs        # Org-mode
├── pdf.rs            # PDF documents
├── pptx.rs           # PowerPoint
├── rst.rs            # reStructuredText
├── rtf.rs            # Rich Text Format
├── sqlite.rs         # SQLite databases
├── table_merge.rs    # Table merging utility
├── text.rs           # Plain text
├── typst.rs          # Typst documents
├── vcard.rs          # vCard contacts
├── xml.rs            # RSS/Atom feeds
└── yaml.rs           # YAML data

test_documents/       # Test fixtures organized by format
├── archive/
├── csv/
├── docbook/
├── docx/
├── email/
├── ... (one per format)
```

## Test Statistics

| Test Suite | Count | Status |
|-----------|-------|--------|
| Library unit tests | 4 | ✅ Pass |
| Archive | 7 | ✅ Pass |
| BibTeX/Log | 5 | ✅ Pass |
| CSV | 3 | ✅ Pass |
| DocBook | 6 | ✅ Pass |
| DOCX | 12 | ✅ Pass |
| Email | 5 | ✅ Pass |
| EPUB | 6 | ✅ Pass |
| Excel | 6 | ✅ Pass |
| FictionBook | 10 | ✅ Pass |
| HTML | 11 | ✅ Pass |
| Image | 15 | ✅ Pass |
| JSON | 6 | ✅ Pass |
| Jupyter | 5 | ✅ Pass |
| LaTeX | 9 | ✅ Pass |
| Legacy Office | 12 | ✅ Pass |
| Markdown | 3 | ✅ Pass |
| OpenDocument | 10 | ✅ Pass |
| OPML | 6 | ✅ Pass |
| Org-mode | 6 | ✅ Pass |
| PDF | 10 | ✅ Pass (1 ignored) |
| PowerPoint | 7 | ✅ Pass |
| RST | 2 | ✅ Pass |
| RTF | 2 | ✅ Pass |
| SQLite | 4 | ✅ Pass |
| Table Merge | 4 | ✅ Pass |
| Text | 4 | ✅ Pass |
| Typst | 7 | ✅ Pass |
| vCard | 4 | ✅ Pass |
| XML/RSS | 2 | ✅ Pass |
| YAML | 4 | ✅ Pass |
| **TOTAL** | **198** | ✅ **Pass** |
| Ignored | 3 | ⚠️ Skipped |

## Writing Tests for a New Format

### Test File Template

Create `tests/myformat.rs`:

```rust
//! MyFormat conversion tests

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

const TEST_DIR: &str = "tests/test_documents/myformat";

fn test_file(name: &str) -> String {
    format!("{}/{}", TEST_DIR, name)
}

// Basic conversion test
#[tokio::test]
async fn test_myformat_basic() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("basic.myfmt"), Some(default_options(".myfmt")))
        .await;

    assert!(
        result.is_ok(),
        "MyFormat conversion failed: {:?}",
        result.err()
    );
    let doc = result.unwrap();
    let content = doc.to_markdown();
    assert!(!content.is_empty(), "Content should not be empty");
}

// Bytes conversion test
#[tokio::test]
async fn test_myformat_bytes_conversion() {
    let md = MarkItDown::new();
    let bytes = fs::read(test_file("basic.myfmt")).expect("Failed to read file");
    let result = md
        .convert_bytes(Bytes::from(bytes), Some(default_options(".myfmt")))
        .await;

    assert!(
        result.is_ok(),
        "MyFormat bytes conversion failed: {:?}",
        result.err()
    );
}

// Feature-specific test
#[tokio::test]
async fn test_myformat_with_tables() {
    let md = MarkItDown::new();
    let result = md
        .convert(&test_file("tables.myfmt"), Some(default_options(".myfmt")))
        .await;

    assert!(result.is_ok());
    let doc = result.unwrap();
    let content = doc.to_markdown();
    // Verify table structure is preserved
    assert!(content.contains("|"), "Should contain table markup");
}
```

### Test Fixtures

Create test files in `tests/test_documents/myformat/`:

```
tests/test_documents/myformat/
├── basic.myfmt              # Simple document
├── tables.myfmt             # Document with tables
├── with-images.myfmt        # Document with embedded images
├── complex.myfmt            # Complex/real-world example
└── README.md                # Notes on fixtures
```

### Best Practices

1. **Test Multiple Scenarios**:
   - Basic/minimal documents
   - Documents with special features (tables, images, etc.)
   - Bytes vs. file-based input
   - Error conditions

2. **Use Descriptive Names**:
   ```rust
   #[tokio::test]
   async fn test_myformat_preserves_heading_hierarchy() {
       // Good: describes what's being tested
   }
   
   #[tokio::test]
   async fn test_myformat_1() {
       // Bad: not descriptive
   }
   ```

3. **Assert Specific Content**:
   ```rust
   // Good: verifies specific feature
   assert!(content.contains("# Heading"), "Should preserve markdown headings");
   
   // Bad: too vague
   assert!(!content.is_empty());
   ```

4. **Test Error Cases**:
   ```rust
   #[tokio::test]
   async fn test_myformat_empty_file() {
       // Should handle gracefully
       let md = MarkItDown::new();
       let result = md
           .convert_bytes(Bytes::from(vec![]), Some(default_options(".myfmt")))
           .await;
       // Either Ok with empty doc or appropriate error
       assert!(result.is_ok() || result.is_err());
   }
   ```

## Unit Tests

Internal unit tests for library components live in `src/`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_table_merge_basic() {
        let input = vec![/* table data */];
        let result = merge_tables(input);
        assert_eq!(result.len(), 1);
    }
}
```

Run unit tests:
```bash
cargo test --lib
```

## Integration Tests

Integration tests verify end-to-end format conversion:

```bash
# All integration tests
cargo test --test '*'

# Specific format
cargo test --test docx
```

## Property-Based Testing

For complex formats, consider property-based testing with `proptest`:

```rust
#[cfg(test)]
mod prop_tests {
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn test_csv_always_produces_markdown(csv in ".*") {
            let result = CsvConverter::parse(csv.as_bytes());
            // Property: conversion should never panic
            let _ = result;
        }
    }
}
```

## Performance Tests

Use Criterion for benchmarking:

```bash
cargo bench
```

See `benches/conversion.rs` for benchmark definitions.

## Continuous Integration

Tests run automatically on:
- Every commit
- Every pull request
- Before release

## Debugging Failed Tests

### 1. Run with Backtrace
```bash
RUST_BACKTRACE=1 cargo test test_name
```

### 2. Run Single Test with Output
```bash
cargo test test_name -- --nocapture
```

### 3. Add Debug Logging
```rust
#[tokio::test]
async fn test_myformat_debug() {
    env_logger::builder()
        .is_test(true)
        .try_init()
        .ok();

    log::debug!("Starting test");
    // ... test code
}
```

Run with logging:
```bash
RUST_LOG=debug cargo test -- --nocapture
```

### 4. Examine Test Fixtures
```bash
ls tests/test_documents/myformat/
file tests/test_documents/myformat/basic.myfmt
```

## Common Issues

### Tests Fail Intermittently
- Check for hardcoded paths
- Verify test isolation (independent from other tests)
- Look for file locking issues

### Slow Tests
- PDF and image tests take longer
- Run specific test suites during development:
  ```bash
  cargo test csv  # Fast
  cargo test pdf  # Slower
  ```

### Missing Test Fixtures
- Ensure files exist in `tests/test_documents/<format>/`
- Use skip attribute for optional tests:
  ```rust
  #[tokio::test]
  #[ignore = "requires large fixture file"]
  async fn test_large_file() { }
  ```

## Adding Test Fixtures

### From Real Documents

1. Collect sample documents
2. Store in `tests/test_documents/<format>/`
3. Name clearly: `basic.ext`, `with-images.ext`, `complex.ext`
4. Document source and purpose in README.md

### Minimal Test Files

Create minimal test files to verify basic functionality:

```bash
# Create minimal CSV
echo "Name,Age
Alice,30
Bob,25" > tests/test_documents/csv/minimal.csv

# Create minimal JSON
echo '{"key": "value"}' > tests/test_documents/json/minimal.json
```

## Skipping Tests

Skip problematic tests temporarily:

```rust
#[tokio::test]
#[ignore = "PDF library issue #123"]
async fn test_pdf_complex_layout() {
    // Test code
}
```

Run ignored tests:
```bash
cargo test -- --ignored
```

## Test Coverage

Generate coverage reports:

```bash
# Using tarpaulin
cargo tarpaulin --out Html

# View in browser
open tarpaulin-report.html
```

Current coverage target: **80%+** for converters

## Contributing Tests

When submitting a new format:

1. ✅ Add converter implementation
2. ✅ Create test file `tests/<format>.rs`
3. ✅ Add test fixtures to `tests/test_documents/<format>/`
4. ✅ Ensure all tests pass: `cargo test`
5. ✅ Add format to supported list in README

## Further Reading

- [Rust Testing Documentation](https://doc.rust-lang.org/book/ch11-00-testing.html)
- [Tokio Testing Guide](https://tokio.rs/tokio/tutorial/select#useful-testing-utilities)
- [Test Organization Best Practices](https://doc.rust-lang.org/book/ch11-03-test-organization.html)
