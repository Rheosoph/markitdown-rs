//! Org-mode to Markdown converter.
//!
//! Converts Org-mode files to markdown using basic regex-based parsing.
//! Note: This is a simplified converter that handles common Org-mode elements.

use async_trait::async_trait;
use bytes::Bytes;
use object_store::ObjectStore;
use regex::Regex;
use std::sync::Arc;

use crate::error::MarkitdownError;
use crate::model::{ContentBlock, ConversionOptions, Document, DocumentConverter, Page};

/// Org-mode to Markdown converter
pub struct OrgModeConverter;

impl OrgModeConverter {
    fn convert_org(bytes: &[u8]) -> Result<Document, MarkitdownError> {
        let content = String::from_utf8_lossy(bytes);
        let mut document = Document::new();
        let mut page = Page::new(1);

        let markdown = Self::org_to_markdown(&content);
        page.add_content(ContentBlock::Markdown(markdown));
        document.add_page(page);

        Ok(document)
    }

    fn org_to_markdown(content: &str) -> String {
        let mut result = String::new();

        // Process line by line
        let lines: Vec<&str> = content.lines().collect();
        let mut in_code_block = false;

        for line in lines {
            // Handle code blocks
            if line.trim().to_lowercase().starts_with("#+begin_src") {
                in_code_block = true;
                // Extract language if present
                let parts: Vec<&str> = line.split_whitespace().collect();
                let code_lang = parts.get(1).copied().unwrap_or("");
                result.push_str(&format!("```{}\n", code_lang));
                continue;
            }

            if line.trim().to_lowercase().starts_with("#+end_src") {
                in_code_block = false;
                result.push_str("```\n");
                continue;
            }

            if in_code_block {
                result.push_str(line);
                result.push('\n');
                continue;
            }

            // Handle quote blocks
            if line.trim().to_lowercase().starts_with("#+begin_quote") {
                result.push_str("> ");
                continue;
            }
            if line.trim().to_lowercase().starts_with("#+end_quote") {
                result.push('\n');
                continue;
            }

            // Handle headings (* -> #)
            if line.starts_with('*') {
                let level = line.chars().take_while(|&c| c == '*').count();
                let text = line.trim_start_matches('*').trim();
                // Strip TODO/DONE keywords
                let text = Self::strip_org_keywords(text);
                let hashes = "#".repeat(level);
                result.push_str(&format!("{} {}\n", hashes, text));
                continue;
            }

            // Handle bold (*text* -> **text**)
            let line = Self::convert_formatting(line);

            // Handle links [[url][text]] -> [text](url)
            let line = Self::convert_links(&line);

            // Skip org-mode directives
            if line.starts_with("#+") {
                continue;
            }

            // Handle list items (- remains the same)
            result.push_str(&line);
            result.push('\n');
        }

        result
    }

    fn strip_org_keywords(text: &str) -> &str {
        let keywords = ["TODO", "DONE", "WAITING", "CANCELLED", "NEXT"];
        let mut result = text;
        for kw in keywords {
            if result.starts_with(kw) {
                result = result[kw.len()..].trim_start();
                break;
            }
        }
        result
    }

    fn convert_formatting(line: &str) -> String {
        // Convert *bold* to **bold** (but not headings)
        // Convert /italic/ to *italic*
        // Convert =code= to `code`
        // Convert ~verbatim~ to `verbatim`

        let mut result = line.to_string();

        // Bold: *text* -> **text**
        let bold_re = Regex::new(r"\*([^*\n]+)\*").unwrap();
        result = bold_re.replace_all(&result, "**$1**").to_string();

        // Italic: /text/ -> *text*
        let italic_re = Regex::new(r"/([^/\n]+)/").unwrap();
        result = italic_re.replace_all(&result, "*$1*").to_string();

        // Code: =text= -> `text`
        let code_re = Regex::new(r"=([^=\n]+)=").unwrap();
        result = code_re.replace_all(&result, "`$1`").to_string();

        // Verbatim: ~text~ -> `text`
        let verb_re = Regex::new(r"~([^~\n]+)~").unwrap();
        result = verb_re.replace_all(&result, "`$1`").to_string();

        result
    }

    fn convert_links(line: &str) -> String {
        // [[url][text]] -> [text](url)
        // [[url]] -> <url>
        let link_with_text = Regex::new(r"\[\[([^\]]+)\]\[([^\]]+)\]\]").unwrap();
        let result = link_with_text.replace_all(line, "[$2]($1)").to_string();

        let link_bare = Regex::new(r"\[\[([^\]]+)\]\]").unwrap();
        link_bare.replace_all(&result, "<$1>").to_string()
    }
}

#[async_trait]
impl DocumentConverter for OrgModeConverter {
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
        Self::convert_org(&bytes)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".org"]
    }
}
