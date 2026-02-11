// Ensure at least one TLS backend is selected at compile time.
#[cfg(not(any(feature = "native-tls", feature = "rustls-tls")))]
compile_error!(
    "At least one TLS backend must be enabled. Use feature \"native-tls\" or \"rustls-tls\"."
);

pub mod archive;
pub mod bibtex;
pub mod calendar;
pub mod csv;
pub mod data;
pub mod docbook;
pub mod docx;
pub mod email;
pub mod epub;
pub mod error;
pub mod excel;
pub mod fictionbook;
pub mod html;
pub mod image;
pub mod iwork;
pub mod jupyter;
pub mod latex;
pub mod legacy_office;
pub mod llm;
pub mod log;
pub mod markdown;
pub mod model;
pub mod opendocument;
pub mod opml;
pub mod orgmode;
pub mod pdf;
pub mod pptx;
pub mod prompts;
pub mod rss;
pub mod rst;
pub mod rtf;
pub mod sqlite;
pub mod table_merge;
pub mod typst;
pub mod vcard;

use archive::ArchiveConverter;
use bibtex::BibtexConverter;
use bytes::Bytes;
use calendar::ICalendarConverter;
use csv::CsvConverter;
use data::{CodeConverter, JsonConverter, TextConverter, TomlConverter, YamlConverter};
use docbook::DocBookConverter;
use docx::DocxConverter;
use email::EmailConverter;
use epub::EpubConverter;
use error::MarkitdownError;
use excel::ExcelConverter;
use fictionbook::FictionBookConverter;
use html::HtmlConverter;
use image::ImageConverter;
use iwork::{KeynoteConverter, NumbersConverter, PagesConverter};
use jupyter::JupyterConverter;
use latex::LatexConverter;
use legacy_office::{
    DocConverter, DotxConverter, PotxConverter, PptConverter, XlsConverter, XltxConverter,
};
use log::LogConverter;
use markdown::MarkdownConverter;
use mime_guess::MimeGuess;
use model::{DocumentConverter, DocumentConverterResult};
use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;
use object_store::ObjectStore;
use opendocument::{OdpConverter, OdsConverter, OdtConverter};
use opml::OpmlConverter;
use orgmode::OrgModeConverter;
use pdf::PdfConverter;
use pptx::PptxConverter;
use rss::RssConverter;
use rst::RstConverter;
use rtf::RtfConverter;
use sqlite::SqliteConverter;
use std::io::Cursor;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::{collections::HashMap, fs};
use typst::TypstConverter;
use vcard::VCardConverter;
use zip::ZipArchive;

// Re-export key types
pub use llm::{
    create_llm_client, create_llm_client_with_config, LlmClient, LlmConfig, LlmWrapper,
    MockLlmClient, SharedLlmClient,
};
pub use model::{ContentBlock, ConversionOptions, Document, ExtractedImage, Page};
pub use prompts::{
    DEFAULT_BATCH_IMAGE_PROMPT, DEFAULT_IMAGE_DESCRIPTION_PROMPT, DEFAULT_PAGE_CONVERSION_PROMPT,
};

/// Main interface for converting documents to markdown
pub struct MarkItDown {
    converters: Vec<Box<dyn DocumentConverter>>,
    store: Arc<dyn ObjectStore>,
}

impl MarkItDown {
    /// Create a new MarkItDown instance with local filesystem storage
    pub fn new() -> Self {
        Self::with_store(Arc::new(LocalFileSystem::new()))
    }

    /// Create a new MarkItDown instance with a custom object store
    pub fn with_store(store: Arc<dyn ObjectStore>) -> Self {
        let mut md = MarkItDown {
            converters: Vec::new(),
            store,
        };

        // Document formats
        md.register_converter(Box::new(CsvConverter));
        md.register_converter(Box::new(ExcelConverter));
        md.register_converter(Box::new(HtmlConverter));
        md.register_converter(Box::new(ImageConverter));
        md.register_converter(Box::new(RssConverter));
        md.register_converter(Box::new(PdfConverter));
        md.register_converter(Box::new(PptxConverter));
        md.register_converter(Box::new(DocxConverter));

        // Ebook and document formats
        md.register_converter(Box::new(RtfConverter));
        md.register_converter(Box::new(EpubConverter));
        md.register_converter(Box::new(EmailConverter));
        md.register_converter(Box::new(MarkdownConverter));

        // Calendar and contact formats
        md.register_converter(Box::new(ICalendarConverter));
        md.register_converter(Box::new(VCardConverter));

        // Database formats
        md.register_converter(Box::new(SqliteConverter));

        // Data formats
        md.register_converter(Box::new(JsonConverter));
        md.register_converter(Box::new(YamlConverter));
        md.register_converter(Box::new(TomlConverter));
        md.register_converter(Box::new(TextConverter));
        md.register_converter(Box::new(CodeConverter));

        // Archive formats
        md.register_converter(Box::new(ArchiveConverter::new()));

        // Bibliography and log formats
        md.register_converter(Box::new(BibtexConverter));
        md.register_converter(Box::new(LogConverter));

        // Legacy Office formats
        md.register_converter(Box::new(DocConverter));
        md.register_converter(Box::new(XlsConverter));
        md.register_converter(Box::new(PptConverter));
        md.register_converter(Box::new(DotxConverter));
        md.register_converter(Box::new(PotxConverter));
        md.register_converter(Box::new(XltxConverter));

        // OpenDocument formats
        md.register_converter(Box::new(OdtConverter));
        md.register_converter(Box::new(OdsConverter));
        md.register_converter(Box::new(OdpConverter));

        // Apple iWork formats
        md.register_converter(Box::new(PagesConverter));
        md.register_converter(Box::new(NumbersConverter));
        md.register_converter(Box::new(KeynoteConverter));

        // Markup language formats
        md.register_converter(Box::new(LatexConverter));
        md.register_converter(Box::new(OrgModeConverter));
        md.register_converter(Box::new(RstConverter));
        md.register_converter(Box::new(TypstConverter));

        // Structured document formats
        md.register_converter(Box::new(DocBookConverter));
        md.register_converter(Box::new(OpmlConverter));
        md.register_converter(Box::new(FictionBookConverter));

        // Notebook formats
        md.register_converter(Box::new(JupyterConverter));

        md
    }

    /// Create a new MarkItDown instance with in-memory storage (useful for bytes)
    pub fn in_memory() -> Self {
        Self::with_store(Arc::new(InMemory::new()))
    }

    pub fn register_converter(&mut self, converter: Box<dyn DocumentConverter>) {
        self.converters.insert(0, converter);
    }

    /// Get the object store
    pub fn store(&self) -> Arc<dyn ObjectStore> {
        self.store.clone()
    }

    fn get_file_type_map() -> HashMap<&'static str, Vec<&'static str>> {
        let mut map = HashMap::new();
        map.insert("application/pdf", vec![".pdf"]);
        map.insert("application/msword", vec![".doc"]);
        map.insert(
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            vec![".docx"],
        );
        map.insert("application/vnd.ms-excel", vec![".xls"]);
        map.insert(
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            vec![".xlsx"],
        );
        map.insert("text/html", vec![".html", ".htm"]);
        map.insert("image/jpeg", vec![".jpg", ".jpeg"]);
        map.insert("image/png", vec![".png"]);
        map.insert("image/gif", vec![".gif"]);
        map.insert("application/zip", vec![".zip"]);
        map.insert("audio/mpeg", vec![".mp3"]);
        map.insert("audio/wav", vec![".wav"]);
        map.insert("application/xml", vec![".xml", ".rss", ".atom"]);
        map
    }

    /// Detect file type from path
    /// Priority: 1) File extension (if valid), 2) infer magic bytes, 3) MIME guess
    pub fn detect_file_type(&self, file_path: &str) -> Option<String> {
        // First, check file extension - this is most reliable for known formats
        // especially for OLE compound files (.doc, .xls, .ppt) which are all detected as .msi by infer
        if let Some(ext) = Path::new(file_path).extension() {
            if let Some(ext_str) = ext.to_str() {
                let ext_lower = ext_str.to_lowercase();
                // Check if we have a converter for this extension
                let ext_with_dot = format!(".{}", ext_lower);
                if self.find_converter(&ext_with_dot).is_some() {
                    return Some(ext_with_dot);
                }
            }
        }

        // Fall back to infer for unknown extensions
        if let Some(kind) = infer::get_from_path(file_path).ok().flatten() {
            return Some(format!(".{}", kind.extension()));
        }

        // Finally try MIME guess
        if let Ok(_content) = fs::read(file_path) {
            if let Some(mime) = MimeGuess::from_path(file_path).first() {
                let mime_str = mime.to_string();
                if let Some(extensions) = Self::get_file_type_map().get(mime_str.as_str()) {
                    return extensions.first().map(|&ext| ext.to_string());
                }
            }
        }

        // Last resort: return the extension anyway
        if let Some(ext) = Path::new(file_path).extension() {
            if let Some(ext_str) = ext.to_str() {
                return Some(format!(".{}", ext_str.to_lowercase()));
            }
        }

        None
    }

    /// Detect file type from bytes
    pub fn detect_bytes(&self, bytes: &[u8]) -> Option<String> {
        if let Some(kind) = infer::get(bytes) {
            return Some(format!(".{}", kind.extension()));
        }
        None
    }

    /// Find the appropriate converter for an extension
    fn find_converter(&self, extension: &str) -> Option<&dyn DocumentConverter> {
        let ext = extension.trim_start_matches('.');
        for converter in &self.converters {
            if converter.can_handle(ext) {
                return Some(converter.as_ref());
            }
        }
        None
    }

    /// Convert a file from the object store to a Document
    pub async fn convert(
        &self,
        path: &str,
        mut options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        // Detect extension if not provided
        let extension = if let Some(ref opts) = options {
            opts.file_extension.clone()
        } else {
            None
        }
        .or_else(|| self.detect_file_type(path));

        if let Some(ref mut opts) = options {
            if opts.file_extension.is_none() {
                opts.file_extension = extension.clone();
            }
            if opts.image_context_path.is_none() {
                opts.image_context_path = Some(path.to_string());
            }
        } else {
            options = Some(
                ConversionOptions::default()
                    .with_extension(extension.clone().unwrap_or_default())
                    .with_image_context_path(path.to_string()),
            );
        }

        let ext = extension.as_deref().unwrap_or("");

        // Handle ZIP files specially
        if ext == ".zip" || ext == "zip" {
            return self.convert_zip_file(path, options).await;
        }

        // Find converter
        if let Some(converter) = self.find_converter(ext) {
            // Check if the path is a local file - if so, read it and use convert_bytes
            // This makes local file handling work seamlessly with any ObjectStore
            let local_path = Path::new(path);
            if local_path.exists() {
                let bytes = fs::read(path)?;
                return converter.convert_bytes(Bytes::from(bytes), options).await;
            }

            // Otherwise, use the object store
            let obj_path = object_store::path::Path::from(path);
            return converter
                .convert(self.store.clone(), &obj_path, options)
                .await;
        }

        Err(MarkitdownError::UnsupportedFormat(format!(
            "No converter found for extension: {}",
            ext
        )))
    }

    /// Convert bytes directly to a Document
    pub async fn convert_bytes(
        &self,
        bytes: Bytes,
        mut options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        // Detect extension if not provided
        let extension = if let Some(ref opts) = options {
            opts.file_extension.clone()
        } else {
            None
        }
        .or_else(|| self.detect_bytes(&bytes));

        if let Some(ref mut opts) = options {
            if opts.file_extension.is_none() {
                opts.file_extension = extension.clone();
            }
        } else {
            options = Some(
                ConversionOptions::default().with_extension(extension.clone().unwrap_or_default()),
            );
        }

        let ext = extension.as_deref().unwrap_or("");

        // Handle ZIP files specially
        if ext == ".zip" || ext == "zip" {
            return self.convert_zip_bytes(&bytes, options).await;
        }

        // Find converter
        if let Some(converter) = self.find_converter(ext) {
            return converter.convert_bytes(bytes, options).await;
        }

        Err(MarkitdownError::UnsupportedFormat(format!(
            "No converter found for extension: {}",
            ext
        )))
    }

    /// Convert a local file to markdown (convenience method)
    pub async fn convert_file(&self, file_path: &str) -> Result<String, MarkitdownError> {
        // Read file and convert
        let bytes = fs::read(file_path)?;
        let extension = self.detect_file_type(file_path);
        let options = extension.map(|ext| ConversionOptions::default().with_extension(ext));

        let document = self.convert_bytes(Bytes::from(bytes), options).await?;
        Ok(document.to_markdown())
    }

    /// Convert bytes to a legacy DocumentConverterResult
    pub async fn convert_bytes_legacy(
        &self,
        bytes: &[u8],
        options: Option<ConversionOptions>,
    ) -> Result<Option<DocumentConverterResult>, MarkitdownError> {
        match self
            .convert_bytes(Bytes::copy_from_slice(bytes), options)
            .await
        {
            Ok(doc) => Ok(Some(DocumentConverterResult::from(doc))),
            Err(MarkitdownError::UnsupportedFormat(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Convert a file to a legacy DocumentConverterResult
    pub async fn convert_legacy(
        &self,
        source: &str,
        options: Option<ConversionOptions>,
    ) -> Result<Option<DocumentConverterResult>, MarkitdownError> {
        // Read file bytes and convert
        let bytes = fs::read(source)?;
        let mut opts = options.unwrap_or_default();
        if opts.file_extension.is_none() {
            opts.file_extension = self.detect_file_type(source);
        }

        match self.convert_bytes(Bytes::from(bytes), Some(opts)).await {
            Ok(doc) => Ok(Some(DocumentConverterResult::from(doc))),
            Err(MarkitdownError::UnsupportedFormat(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Convert a ZIP file by extracting and converting each file
    async fn convert_zip_file(
        &self,
        path: &str,
        _options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let data = fs::read(path)?;
        self.convert_zip_bytes(&data, _options).await
    }

    /// Convert ZIP bytes
    async fn convert_zip_bytes(
        &self,
        bytes: &[u8],
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let cursor = Cursor::new(bytes);
        let mut archive = ZipArchive::new(cursor)?;

        let mut document = Document::new();
        document.title = Some("ZIP Archive Contents".to_string());

        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            let file_name = file.name().to_string();

            // Skip directories
            if file.is_dir() {
                continue;
            }

            // Read file contents
            let mut file_contents = Vec::new();
            file.read_to_end(&mut file_contents)?;

            // Detect file type
            let ext = self.detect_bytes(&file_contents).or_else(|| {
                Path::new(&file_name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|s| format!(".{}", s.to_lowercase()))
            });

            if let Some(extension) = ext {
                let file_opts = options
                    .clone()
                    .map(|mut o| {
                        o.file_extension = Some(extension.clone());
                        if o.image_context_path.is_none() {
                            o.image_context_path = Some(file_name.clone());
                        }
                        o
                    })
                    .or_else(|| {
                        Some(ConversionOptions::default().with_extension(extension.clone()))
                    });

                if let Some(converter) = self.find_converter(&extension) {
                    match converter
                        .convert_bytes(Bytes::from(file_contents), file_opts)
                        .await
                    {
                        Ok(file_doc) => {
                            // Add file header as first page or merge with existing
                            let mut page = Page::new((document.pages.len() + 1) as u32);
                            page.add_content(ContentBlock::Heading {
                                level: 2,
                                text: format!("File: {}", file_name),
                            });

                            // Add content from converted document
                            for file_page in file_doc.pages {
                                for block in file_page.content {
                                    page.add_content(block);
                                }
                            }

                            document.add_page(page);
                        }
                        Err(_) => {
                            // Skip files we cant convert
                        }
                    }
                }
            }
        }

        Ok(document)
    }
}

impl Default for MarkItDown {
    fn default() -> Self {
        Self::new()
    }
}
