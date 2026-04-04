//! LaTeX to Markdown converter.
//!
//! Converts LaTeX files to markdown using basic parsing.
//! Math environments are preserved as-is for rendering.

use async_trait::async_trait;
use bytes::Bytes;
use object_store::ObjectStore;
use regex::Regex;
use std::sync::Arc;

use crate::error::MarkitdownError;
use crate::model::{ContentBlock, ConversionOptions, Document, DocumentConverter, Page};

/// LaTeX to Markdown converter
pub struct LatexConverter;

impl LatexConverter {
    fn convert_latex(bytes: &[u8]) -> Result<Document, MarkitdownError> {
        let content = String::from_utf8_lossy(bytes);
        let mut document = Document::new();
        let mut page = Page::new(1);

        let markdown = Self::latex_to_markdown(&content);
        page.add_content(ContentBlock::Markdown(markdown));
        document.add_page(page);

        Ok(document)
    }

    fn latex_to_markdown(content: &str) -> String {
        let mut result = String::new();
        let mut in_document = false;
        let mut in_math_env = false;
        let mut in_verbatim = false;
        let mut in_enumerate = false;
        let mut enum_counter = 0;

        for line in content.lines() {
            let trimmed = line.trim();

            // Handle document environment
            if trimmed.starts_with("\\begin{document}") {
                in_document = true;
                continue;
            }
            if trimmed.starts_with("\\end{document}") {
                in_document = false;
                continue;
            }

            // Skip preamble content (before \begin{document})
            if !in_document
                && (trimmed.starts_with("\\documentclass")
                    || trimmed.starts_with("\\usepackage")
                    || trimmed.starts_with("\\newcommand")
                    || trimmed.starts_with("\\renewcommand")
                    || trimmed.starts_with("\\def")
                    || trimmed.starts_with("\\set")
                    || trimmed.starts_with('%'))
            {
                continue;
            }

            // Handle verbatim/lstlisting environments
            if trimmed.starts_with("\\begin{verbatim}")
                || trimmed.starts_with("\\begin{lstlisting}")
            {
                in_verbatim = true;
                result.push_str("```\n");
                continue;
            }
            if trimmed.starts_with("\\end{verbatim}") || trimmed.starts_with("\\end{lstlisting}") {
                in_verbatim = false;
                result.push_str("```\n");
                continue;
            }
            if in_verbatim {
                result.push_str(line);
                result.push('\n');
                continue;
            }

            // Handle math environments
            if trimmed.starts_with("\\begin{equation")
                || trimmed.starts_with("\\begin{align")
                || trimmed.starts_with("\\begin{gather")
                || trimmed.starts_with("\\begin{math")
            {
                in_math_env = true;
                result.push_str("$$\n");
                continue;
            }
            if trimmed.starts_with("\\end{equation")
                || trimmed.starts_with("\\end{align")
                || trimmed.starts_with("\\end{gather")
                || trimmed.starts_with("\\end{math")
            {
                in_math_env = false;
                result.push_str("$$\n");
                continue;
            }
            if in_math_env {
                result.push_str(line);
                result.push('\n');
                continue;
            }

            // Handle lists
            if trimmed.starts_with("\\begin{itemize}") {
                continue;
            }
            if trimmed.starts_with("\\end{itemize}") {
                result.push('\n');
                continue;
            }
            if trimmed.starts_with("\\begin{enumerate}") {
                in_enumerate = true;
                enum_counter = 0;
                continue;
            }
            if trimmed.starts_with("\\end{enumerate}") {
                in_enumerate = false;
                result.push('\n');
                continue;
            }

            // Handle \item
            if trimmed.starts_with("\\item") {
                let item_text = trimmed.strip_prefix("\\item").unwrap_or("").trim();
                if in_enumerate {
                    enum_counter += 1;
                    result.push_str(&format!("{}. {}\n", enum_counter, item_text));
                } else {
                    result.push_str(&format!("- {}\n", item_text));
                }
                continue;
            }

            // Handle sectioning commands
            if let Some(section_text) = Self::extract_section(trimmed, "\\chapter", 1) {
                result.push_str(&section_text);
                continue;
            }
            if let Some(section_text) = Self::extract_section(trimmed, "\\section", 2) {
                result.push_str(&section_text);
                continue;
            }
            if let Some(section_text) = Self::extract_section(trimmed, "\\subsection", 3) {
                result.push_str(&section_text);
                continue;
            }
            if let Some(section_text) = Self::extract_section(trimmed, "\\subsubsection", 4) {
                result.push_str(&section_text);
                continue;
            }
            if let Some(section_text) = Self::extract_section(trimmed, "\\paragraph", 5) {
                result.push_str(&section_text);
                continue;
            }

            // Handle title/author/date (if before document)
            if let Some(title) = Self::extract_command(trimmed, "\\title") {
                result.push_str(&format!("# {}\n\n", title));
                continue;
            }
            if let Some(author) = Self::extract_command(trimmed, "\\author") {
                result.push_str(&format!("**Author:** {}\n\n", author));
                continue;
            }
            if let Some(date) = Self::extract_command(trimmed, "\\date") {
                result.push_str(&format!("**Date:** {}\n\n", date));
                continue;
            }

            // Skip \maketitle
            if trimmed.starts_with("\\maketitle") {
                continue;
            }

            // Skip comments
            if trimmed.starts_with('%') {
                continue;
            }

            // Convert inline formatting
            let line = Self::convert_inline_formatting(line);

            result.push_str(&line);
            result.push('\n');
        }

        result
    }

    fn extract_section(line: &str, command: &str, level: usize) -> Option<String> {
        if line.starts_with(command) {
            if let Some(title) = Self::extract_braces(line, command.len()) {
                let hashes = "#".repeat(level);
                return Some(format!("{} {}\n\n", hashes, title));
            }
        }
        None
    }

    fn extract_command(line: &str, command: &str) -> Option<String> {
        if line.starts_with(command) {
            Self::extract_braces(line, command.len())
        } else {
            None
        }
    }

    fn extract_braces(line: &str, start: usize) -> Option<String> {
        let rest = &line[start..];
        if rest.starts_with('{') {
            let mut depth = 0;
            let mut end = 0;
            for (i, c) in rest.chars().enumerate() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if end > 1 {
                return Some(rest[1..end].to_string());
            }
        }
        None
    }

    fn convert_inline_formatting(line: &str) -> String {
        let mut result = line.to_string();

        // \textbf{text} -> **text**
        let bold_re = Regex::new(r"\\textbf\{([^}]+)\}").unwrap();
        result = bold_re.replace_all(&result, "**$1**").to_string();

        // \textit{text} -> *text*
        let italic_re = Regex::new(r"\\textit\{([^}]+)\}").unwrap();
        result = italic_re.replace_all(&result, "*$1*").to_string();

        // \emph{text} -> *text*
        let emph_re = Regex::new(r"\\emph\{([^}]+)\}").unwrap();
        result = emph_re.replace_all(&result, "*$1*").to_string();

        // \texttt{text} -> `text`
        let tt_re = Regex::new(r"\\texttt\{([^}]+)\}").unwrap();
        result = tt_re.replace_all(&result, "`$1`").to_string();

        // \verb|text| -> `text`
        let verb_re = Regex::new(r"\\verb\|([^|]+)\|").unwrap();
        result = verb_re.replace_all(&result, "`$1`").to_string();

        // $math$ stays as $math$ (inline math)
        // $$math$$ stays as $$math$$ (display math)

        // \href{url}{text} -> [text](url)
        let href_re = Regex::new(r"\\href\{([^}]+)\}\{([^}]+)\}").unwrap();
        result = href_re.replace_all(&result, "[$2]($1)").to_string();

        // \url{url} -> <url>
        let url_re = Regex::new(r"\\url\{([^}]+)\}").unwrap();
        result = url_re.replace_all(&result, "<$1>").to_string();

        result
    }
}

#[async_trait]
impl DocumentConverter for LatexConverter {
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
        Self::convert_latex(&bytes)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".tex", ".latex"]
    }
}
