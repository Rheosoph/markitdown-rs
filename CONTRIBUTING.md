# Contributing to markitdown-rs

Thank you for your interest in contributing to markitdown-rs! This guide will help you get started.

## Getting Started

### Prerequisites

- Rust 1.70 or later
- Cargo
- Git

### Setting Up

1. Fork the repository on GitHub
2. Clone your fork:
   ```bash
   git clone https://github.com/YOUR_USERNAME/markitdown-rs.git
   cd markitdown-rs
   ```
3. Add upstream remote:
   ```bash
   git remote add upstream https://github.com/Rheosoph/markitdown-rs.git
   ```

### Building

```bash
cargo build
```

### Running Tests

```bash
# All tests
cargo test

# Specific format tests
cargo test csv
cargo test docx

# With output
cargo test -- --nocapture

# Single threaded (helpful for debugging)
cargo test -- --test-threads=1

# LLM Integration Tests
# Requires .env file with OPENROUTER_API_KEY, OPENROUTER_ENDPOINT, OPENROUTER_MODEL
cargo test --test llm
```

## How to Contribute

### Reporting Bugs

1. Check [existing issues](../../issues) first
2. Create a new issue with:
   - Clear title describing the bug
   - Steps to reproduce
   - Expected vs. actual behavior
   - Format(s) affected
   - Your environment (OS, Rust version)

### Requesting Features

1. Check [existing issues/discussions](../../discussions)
2. Create an issue or discussion with:
   - Clear description of the feature
   - Why it's useful
   - Example use cases
   - Links to similar projects if applicable

### Adding a New Format

Follow the step-by-step guide in [ARCHITECTURE.md](docs/ARCHITECTURE.md#adding-a-new-format).

**Checklist:**
- [ ] Converter implemented in `src/<format>.rs`
- [ ] Registered in `src/lib.rs`
- [ ] Test file created at `tests/<format>.rs`
- [ ] Test fixtures in `tests/test_documents/<format>/`
- [ ] All tests pass: `cargo test`
- [ ] Documentation updated in relevant docs
- [ ] No clippy warnings: `cargo clippy`

### Improving Existing Converters

1. Identify the issue
2. Create a test that reproduces the issue
3. Fix the converter
4. Verify all tests pass
5. Submit PR with clear description

### Improving Documentation

- Fix typos or unclear explanations
- Add examples
- Improve organization
- Add missing information

## Development Workflow

### Creating a Branch

```bash
# Update main branch
git checkout main
git pull upstream main

# Create feature branch
git checkout -b feature/add-xyz-format
```

### Making Changes

1. Make your changes
2. Run tests: `cargo test`
3. Check formatting: `cargo fmt`
4. Check for issues: `cargo clippy`

### Committing

Use clear, descriptive commit messages:

```bash
git commit -m "Add support for XYZ format

- Implement XyzConverter trait
- Add 5 test fixtures
- Support .xyz and .xyz2 extensions
- Handle edge cases (empty files, invalid UTF-8)

Fixes #123"
```

### Pushing and Creating PR

```bash
git push origin feature/add-xyz-format
```

Then create a Pull Request on GitHub with:
- Clear title
- Description of changes
- Related issues (fixes #123)
- Testing notes

## Code Style

### Formatting

```bash
cargo fmt
```

### Linting

```bash
cargo clippy -- -D warnings
```

### Documentation

Add doc comments to public items:

```rust
/// Converts XYZ documents to Markdown.
///
/// # Arguments
///
/// * `bytes` - The XYZ file as bytes
///
/// # Returns
///
/// A `Document` containing the converted Markdown
///
/// # Errors
///
/// Returns an error if the file is malformed
pub async fn convert_bytes(
    &self,
    bytes: Bytes,
    _options: Option<ConversionOptions>,
) -> Result<Document, MarkitdownError> {
    // implementation
}
```

## Testing Guidelines

### Test Coverage

- **Minimum**: Basic conversion test + bytes conversion test
- **Recommended**: Tests for key features (tables, images, etc.)
- **Examples**: See [TESTING.md](docs/TESTING.md)

### Test Naming

```rust
// Good: describes what's tested
#[tokio::test]
async fn test_docx_preserves_table_structure() { }

// Bad: non-descriptive
#[tokio::test]
async fn test_docx_1() { }
```

### Test Fixtures

- Store in `tests/test_documents/<format>/`
- Use clear names: `basic.ext`, `with-images.ext`, `complex.ext`
- Document origin and purpose in `README.md`
- Keep minimal to reduce repo size

## Common Issues

### Tests Failing Locally

1. Update your branch: `git pull upstream main`
2. Clean build: `cargo clean && cargo build`
3. Run tests again: `cargo test`
4. Check for environment-specific issues

### Clippy Warnings

```bash
cargo clippy -- -D warnings
```

Fix warnings before submitting PR. Use `#[allow(...)]` only when justified.

### Large Binary Dependencies

If adding a new library dependency, discuss in the issue first to ensure it's worth the compile time.

## Performance Considerations

- Prefer streaming over loading entire files into memory
- Use async/await for I/O operations
- Consider impact on build time for new dependencies
- Benchmark impact on common use cases

### Adding Dependencies

1. Justify in the issue/PR why it's needed
2. Check for alternatives
3. Verify it's actively maintained
4. Check for security issues
5. Consider optional features if it bloats the build

## Documentation Updates

When adding a new format:

1. Update [docs/FORMATS.md](docs/FORMATS.md) with format details
2. Update [docs/FORMAT_COVERAGE.md](docs/FORMAT_COVERAGE.md) with converter info
3. Add examples in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) if relevant
4. Update README.md supported formats list

## Security

- Don't commit secrets or credentials
- Test with malformed/invalid files (don't crash)
- Use safe parsing practices
- Report security issues privately to maintainers

## License

By contributing, you agree your work will be licensed under MIT License (same as the project).

## Questions?

- 💬 Open a [Discussion](../../discussions)
- 📧 Contact maintainers
- 📖 Check [Documentation](docs/)

## Recognition

Contributors will be recognized in:
- CHANGELOG
- GitHub Contributors page
- Release notes (for significant contributions)

---

Thank you for helping make markitdown-rs better! 🎉
