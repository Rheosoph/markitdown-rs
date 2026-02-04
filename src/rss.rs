use crate::error::MarkitdownError;
use crate::model::{ContentBlock, ConversionOptions, Document, DocumentConverter, Page};
use async_trait::async_trait;
use bytes::Bytes;
use feed_rs::parser;
use object_store::ObjectStore;
use std::io::BufReader;
use std::sync::Arc;

pub struct RssConverter;

impl RssConverter {
    /// Convert bytes to Document
    fn bytes_to_document(&self, bytes: &[u8]) -> Result<Document, MarkitdownError> {
        let feed = parser::parse(BufReader::new(bytes))
            .map_err(|e| MarkitdownError::ParseError(format!("Failed to parse feed: {}", e)))?;

        let mut document = Document::new();

        // Set the document title from the feed
        if let Some(title) = &feed.title {
            document.title = Some(title.content.clone());
        }

        // Create a single page for the feed content
        let mut page = Page::new(1);

        // Add feed title as heading
        if let Some(title) = &feed.title {
            page.add_content(ContentBlock::Heading {
                level: 1,
                text: title.content.clone(),
            });
        }

        // Add feed description if available
        if let Some(description) = &feed.description {
            page.add_content(ContentBlock::Text(description.content.clone()));
        }

        // Process based on feed type
        if feed.feed_type == feed_rs::model::FeedType::Atom {
            Self::parse_atom_entries(&feed, &mut page);
        } else {
            Self::parse_rss_entries(&feed, &mut page);
        }

        document.add_page(page);
        Ok(document)
    }

    /// Parse Atom feed entries
    fn parse_atom_entries(feed: &feed_rs::model::Feed, page: &mut Page) {
        for entry in &feed.entries {
            // Entry title
            if let Some(title) = &entry.title {
                page.add_content(ContentBlock::Heading {
                    level: 2,
                    text: title.content.clone(),
                });
            }

            // Updated date
            if let Some(updated) = &entry.updated {
                page.add_content(ContentBlock::Text(format!("Updated on: {}", updated)));
            }

            // Entry content
            if let Some(content) = &entry.content {
                if let Some(body) = &content.body {
                    let markdown = htmd::convert(body).unwrap_or_default();
                    page.add_content(ContentBlock::Markdown(markdown));
                }
            }
        }
    }

    /// Parse RSS feed entries
    fn parse_rss_entries(feed: &feed_rs::model::Feed, page: &mut Page) {
        for entry in &feed.entries {
            // Entry title
            if let Some(title) = &entry.title {
                page.add_content(ContentBlock::Heading {
                    level: 2,
                    text: title.content.clone(),
                });
            }

            // Published date
            if let Some(published) = &entry.published {
                page.add_content(ContentBlock::Text(format!("Published on: {}", published)));
            }

            // Entry summary
            if let Some(summary) = &entry.summary {
                let markdown = htmd::convert(&summary.content).unwrap_or_default();
                page.add_content(ContentBlock::Markdown(markdown));
            }
        }
    }
}

#[async_trait]
impl DocumentConverter for RssConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        _options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let result = store.get(path).await.map_err(|e| {
            MarkitdownError::ObjectStoreError(format!("Failed to get object: {}", e))
        })?;

        let bytes = result.bytes().await.map_err(|e| {
            MarkitdownError::ObjectStoreError(format!("Failed to read bytes: {}", e))
        })?;

        self.bytes_to_document(&bytes)
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        _options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        self.bytes_to_document(&bytes)
    }

    fn supported_extensions(&self) -> &[&str] {
        &["rss", "xml", "atom"]
    }
}
