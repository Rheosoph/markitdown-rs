//! iCalendar (.ics) to Markdown converter.
//!
//! Supports conversion of iCalendar files (events, todos) to markdown.

use async_trait::async_trait;
use bytes::Bytes;
use icalendar::{Calendar, CalendarComponent, Component, EventLike};
use object_store::ObjectStore;
use std::sync::Arc;

use crate::error::MarkitdownError;
use crate::model::{ContentBlock, ConversionOptions, Document, DocumentConverter, Page};

/// iCalendar (.ics) converter
pub struct ICalendarConverter;

impl ICalendarConverter {
    fn convert_ics(bytes: &[u8]) -> Result<Document, MarkitdownError> {
        let content = String::from_utf8_lossy(bytes);
        let calendar: Calendar = content
            .parse()
            .map_err(|e| MarkitdownError::ParseError(format!("iCalendar parse error: {:?}", e)))?;

        let mut document = Document::new();
        let mut page = Page::new(1);
        let mut markdown = String::new();

        // Get calendar name if available
        if let Some(name) = calendar.get_name() {
            markdown.push_str(&format!("# {}\n\n", name));
        } else {
            markdown.push_str("# Calendar\n\n");
        }

        let mut event_count = 0;
        let mut todo_count = 0;

        for component in &calendar.components {
            match component {
                CalendarComponent::Event(event) => {
                    event_count += 1;
                    markdown.push_str("## 📅 Event\n\n");

                    if let Some(summary) = event.get_summary() {
                        markdown.push_str(&format!("**{}**\n\n", summary));
                    }

                    if let Some(start) = event.get_start() {
                        markdown.push_str(&format!("- **Start:** {:?}\n", start));
                    }
                    if let Some(end) = event.get_end() {
                        markdown.push_str(&format!("- **End:** {:?}\n", end));
                    }
                    if let Some(location) = event.get_location() {
                        markdown.push_str(&format!("- **Location:** {}\n", location));
                    }
                    if let Some(description) = event.get_description() {
                        markdown.push_str(&format!("\n{}\n", description));
                    }
                    markdown.push('\n');
                }
                CalendarComponent::Todo(todo) => {
                    todo_count += 1;
                    markdown.push_str("## ☑️ Task\n\n");

                    if let Some(summary) = todo.get_summary() {
                        markdown.push_str(&format!("**{}**\n\n", summary));
                    }

                    if let Some(due) = todo.get_due() {
                        markdown.push_str(&format!("- **Due:** {:?}\n", due));
                    }
                    if let Some(description) = todo.get_description() {
                        markdown.push_str(&format!("\n{}\n", description));
                    }
                    markdown.push('\n');
                }
                _ => {}
            }
        }

        if event_count == 0 && todo_count == 0 {
            markdown.push_str("*No events or tasks found in calendar.*\n");
        } else {
            markdown.push_str(&format!(
                "\n---\n*{} event(s), {} task(s)*\n",
                event_count, todo_count
            ));
        }

        page.add_content(ContentBlock::Markdown(markdown));
        document.add_page(page);
        Ok(document)
    }
}

#[async_trait]
impl DocumentConverter for ICalendarConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        if let Some(opts) = &options {
            if let Some(ext) = &opts.file_extension {
                if ext.to_lowercase() != ".ics" && ext.to_lowercase() != ".ical" {
                    return Err(MarkitdownError::InvalidFile(format!(
                        "Expected .ics file, got {}",
                        ext
                    )));
                }
            }
        }

        let result = store.get(path).await?;
        let bytes = result.bytes().await?;
        self.convert_bytes(bytes, options).await
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        if let Some(opts) = &options {
            if let Some(ext) = &opts.file_extension {
                if ext.to_lowercase() != ".ics" && ext.to_lowercase() != ".ical" {
                    return Err(MarkitdownError::InvalidFile(format!(
                        "Expected .ics file, got {}",
                        ext
                    )));
                }
            }
        }

        Self::convert_ics(&bytes)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".ics", ".ical"]
    }
}
