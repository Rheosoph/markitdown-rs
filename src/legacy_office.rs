//! Legacy Office format converters (.doc, .xls, .ppt).
//!
//! These formats use OLE Compound File Binary format.
//! We extract text using proper binary parsing of the internal structures.
//! Images are extracted from the Pictures stream (PPT) or Data stream (DOC).

use async_trait::async_trait;
use bytes::Bytes;
use cfb::CompoundFile;
use object_store::{ObjectStore, ObjectStoreExt};
use std::io::{Cursor, Read};
use std::sync::Arc;

use crate::error::MarkitdownError;
use crate::model::{
    ContentBlock, ConversionOptions, Document, DocumentConverter, ExtractedImage, Page,
};

/// Represents an extracted image from a legacy Office file
#[derive(Debug, Clone)]
struct LegacyImage {
    /// Image data
    data: Vec<u8>,
    /// Detected image format
    format: ImageFormat,
    /// Index of the image in the document
    index: usize,
}

/// Supported image formats in legacy Office files
#[derive(Debug, Clone, Copy, PartialEq)]
enum ImageFormat {
    Jpeg,
    Png,
    Wmf,
    Emf,
    Dib,
    Pict,
    Unknown,
}

impl ImageFormat {
    fn mime_type(&self) -> &'static str {
        match self {
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Png => "image/png",
            ImageFormat::Wmf => "image/wmf",
            ImageFormat::Emf => "image/emf",
            ImageFormat::Dib => "image/bmp",
            ImageFormat::Pict => "image/pict",
            ImageFormat::Unknown => "application/octet-stream",
        }
    }

    fn extension(&self) -> &'static str {
        match self {
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Png => "png",
            ImageFormat::Wmf => "wmf",
            ImageFormat::Emf => "emf",
            ImageFormat::Dib => "bmp",
            ImageFormat::Pict => "pict",
            ImageFormat::Unknown => "bin",
        }
    }

    /// Detect image format from byte signature
    fn detect(data: &[u8]) -> Self {
        if data.len() < 8 {
            return ImageFormat::Unknown;
        }

        // JPEG: FF D8 FF
        if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return ImageFormat::Jpeg;
        }

        // PNG: 89 50 4E 47 0D 0A 1A 0A
        if data.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
            return ImageFormat::Png;
        }

        // WMF Placeable: D7 CD C6 9A
        if data.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) {
            return ImageFormat::Wmf;
        }

        // WMF Standard: 01 00 or 02 00 (type field)
        if data.len() >= 18 && (data[0] == 0x01 || data[0] == 0x02) && data[1] == 0x00 {
            // Check for valid WMF header version
            let version = u16::from_le_bytes([data[4], data[5]]);
            if version == 0x0100 || version == 0x0300 {
                return ImageFormat::Wmf;
            }
        }

        // EMF: 01 00 00 00 ... 20 45 4D 46 at offset 40
        if data.len() >= 44
            && data[0..4] == [0x01, 0x00, 0x00, 0x00]
            && data[40..44] == [0x20, 0x45, 0x4D, 0x46]
        {
            return ImageFormat::Emf;
        }

        // DIB/BMP: BM
        if data.starts_with(b"BM") {
            return ImageFormat::Dib;
        }

        // PICT (Mac): starts with size then 0x00 0x11
        if data.len() >= 4 && data[2] == 0x00 && data[3] == 0x11 {
            return ImageFormat::Pict;
        }

        ImageFormat::Unknown
    }
}

/// Legacy Word document (.doc) converter
pub struct DocConverter;

impl DocConverter {
    fn convert_doc(
        bytes: &[u8],
        options: Option<&ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let cursor = Cursor::new(bytes);
        let mut cfb = CompoundFile::open(cursor)
            .map_err(|e| MarkitdownError::ParseError(format!("DOC parse error: {}", e)))?;

        let mut document = Document::new();
        let mut page = Page::new(1);
        let mut markdown = String::new();

        markdown.push_str("# Word Document (.doc)\n\n");

        // Try to read the WordDocument stream which contains the main text
        let text = if let Ok(mut stream) = cfb.open_stream("/WordDocument") {
            let mut data = Vec::new();
            stream.read_to_end(&mut data).ok();
            Self::extract_text_from_word_document(&data)
        } else {
            None
        };

        if let Some(text) = text {
            if !text.trim().is_empty() {
                markdown.push_str(&text);
            } else {
                markdown.push_str("*Document appears to be empty or contains only formatting.*\n");
            }
        } else {
            markdown.push_str("*Unable to extract text from legacy Word document.*\n\n");
            markdown.push_str("This .doc file uses a binary format. Consider converting to .docx for better extraction.\n");
        }

        page.add_content(ContentBlock::Markdown(markdown));

        // Extract images if enabled
        let extract_images = options.map(|o| o.extract_images).unwrap_or(true);
        if extract_images {
            // Re-open CFB to extract images
            let cursor = Cursor::new(bytes);
            if let Ok(mut cfb) = CompoundFile::open(cursor) {
                let images = Self::extract_images_from_doc(&mut cfb);
                for image in images {
                    let mut extracted = ExtractedImage::new(
                        format!("doc_image_{}", image.index),
                        Bytes::from(image.data),
                        image.format.mime_type(),
                    );
                    extracted.alt_text =
                        Some(format!("image{}.{}", image.index, image.format.extension()));
                    extracted.page_number = Some(1);
                    page.add_content(ContentBlock::Image(extracted));
                }
            }
        }

        document.add_page(page);
        Ok(document)
    }

    /// Extract images from a DOC file
    /// Images in Word 97-2003 are stored in the "Data" stream as BLIP records
    /// or as embedded OLE objects in ObjectPool
    fn extract_images_from_doc<R: Read + std::io::Seek>(
        cfb: &mut CompoundFile<R>,
    ) -> Vec<LegacyImage> {
        let mut images = Vec::new();

        // Try to read the Data stream which contains embedded pictures
        if let Ok(mut stream) = cfb.open_stream("/Data") {
            let mut data = Vec::new();
            if stream.read_to_end(&mut data).is_ok() {
                let found = Self::extract_blips_from_data(&data);
                images.extend(found);
            }
        }

        // Also check for images in Pictures stream (some DOC files use this)
        if let Ok(mut stream) = cfb.open_stream("/Pictures") {
            let mut data = Vec::new();
            if stream.read_to_end(&mut data).is_ok() && !data.is_empty() {
                let found = Self::extract_blips_from_data(&data);
                images.extend(found);
            }
        }

        // Scan for embedded image signatures in WordDocument stream as fallback
        if images.is_empty() {
            if let Ok(mut stream) = cfb.open_stream("/WordDocument") {
                let mut data = Vec::new();
                if stream.read_to_end(&mut data).is_ok() {
                    let found = Self::scan_for_images(&data);
                    images.extend(found);
                }
            }
        }

        // Re-index images
        for (idx, img) in images.iter_mut().enumerate() {
            img.index = idx + 1;
        }

        images
    }

    /// Extract BLIP (Binary Large Image/Picture) records from data
    /// BLIP records start with a type indicator and contain raw image data
    fn extract_blips_from_data(data: &[u8]) -> Vec<LegacyImage> {
        let mut images = Vec::new();
        let mut pos = 0;

        while pos + 8 < data.len() {
            // Look for image signatures
            if let Some(img) = Self::try_extract_image_at(data, pos) {
                let img_len = img.0.len().max(1);
                images.push(LegacyImage {
                    data: img.0,
                    format: img.1,
                    index: images.len() + 1,
                });
                pos += img_len;
            } else {
                pos += 1;
            }
        }

        images
    }

    /// Try to extract an image starting at a given position
    fn try_extract_image_at(data: &[u8], pos: usize) -> Option<(Vec<u8>, ImageFormat)> {
        if pos + 8 > data.len() {
            return None;
        }

        let slice = &data[pos..];

        // JPEG: FF D8 FF
        if slice.starts_with(&[0xFF, 0xD8, 0xFF]) {
            if let Some(end) = Self::find_jpeg_end(slice) {
                return Some((slice[..end].to_vec(), ImageFormat::Jpeg));
            }
        }

        // PNG: 89 50 4E 47 0D 0A 1A 0A
        if slice.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
            if let Some(end) = Self::find_png_end(slice) {
                return Some((slice[..end].to_vec(), ImageFormat::Png));
            }
        }

        // EMF
        if slice.len() >= 44
            && slice[0..4] == [0x01, 0x00, 0x00, 0x00]
            && slice[40..44] == [0x20, 0x45, 0x4D, 0x46]
        {
            // EMF header contains size at bytes 4-8
            let size = u32::from_le_bytes([slice[4], slice[5], slice[6], slice[7]]) as usize;
            if size > 44 && size <= slice.len() {
                return Some((slice[..size].to_vec(), ImageFormat::Emf));
            }
        }

        // WMF Placeable
        if slice.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) {
            // WMF placeable header is 22 bytes, followed by standard header
            if slice.len() >= 22 {
                // Try to find a reasonable end (usually next image or large gap)
                let end = Self::find_metafile_end(slice);
                if end > 22 {
                    return Some((slice[..end].to_vec(), ImageFormat::Wmf));
                }
            }
        }

        None
    }

    /// Find the end of a JPEG image
    fn find_jpeg_end(data: &[u8]) -> Option<usize> {
        // JPEG ends with FF D9
        for i in 2..data.len() - 1 {
            if data[i] == 0xFF && data[i + 1] == 0xD9 {
                return Some(i + 2);
            }
        }
        None
    }

    /// Find the end of a PNG image
    fn find_png_end(data: &[u8]) -> Option<usize> {
        // PNG ends with IEND chunk: length(4) + "IEND" + crc(4)
        // The IEND marker is: 00 00 00 00 49 45 4E 44 AE 42 60 82
        let iend_sig = [0x49, 0x45, 0x4E, 0x44]; // "IEND"
        for i in 8..data.len() - 8 {
            if data[i..i + 4] == iend_sig {
                // Found IEND, add 8 bytes (4 for length, 4 for CRC)
                return Some(i + 8);
            }
        }
        None
    }

    /// Find the end of a metafile (WMF/EMF)
    fn find_metafile_end(data: &[u8]) -> usize {
        // Look for the next image signature or a large gap of zeros
        let mut end = data.len().min(1024 * 1024); // Max 1MB for safety

        // Look for next image signature
        for i in 22..data.len() - 8 {
            if data[i..].starts_with(&[0xFF, 0xD8, 0xFF])
                || data[i..].starts_with(&[0x89, 0x50, 0x4E, 0x47])
                || data[i..].starts_with(&[0xD7, 0xCD, 0xC6, 0x9A])
            {
                end = i;
                break;
            }
        }

        end
    }

    /// Scan raw data for embedded image signatures
    fn scan_for_images(data: &[u8]) -> Vec<LegacyImage> {
        let mut images = Vec::new();
        let mut pos = 0;

        while pos + 16 < data.len() {
            if let Some((img_data, format)) = Self::try_extract_image_at(data, pos) {
                let len = img_data.len();
                images.push(LegacyImage {
                    data: img_data,
                    format,
                    index: images.len() + 1,
                });
                pos += len;
            } else {
                pos += 1;
            }
        }

        images
    }

    /// Extract text from the WordDocument stream using the FIB structure
    fn extract_text_from_word_document(data: &[u8]) -> Option<String> {
        if data.len() < 68 {
            return None;
        }

        // Read FIB (File Information Block) header
        // The first 32 bytes contain magic and version info
        // Offset 0x18 (24): fcClx - file offset to complex part
        // Offset 0x1C (28): lcbClx - size of complex part

        // For older Word documents, we need to check the FIB version
        let w_ident = u16::from_le_bytes([data[0], data[1]]);

        // 0xA5EC = Word 97-2003 magic
        // 0xA5DC = Word 6.0/95 magic
        if w_ident != 0xA5EC && w_ident != 0xA5DC {
            // Not a valid Word document header, try text extraction
            return Self::extract_unicode_and_ansi_text(data);
        }

        // Try to extract using the character position table
        // For complex documents, text may be in the clx structure
        // For simpler approach, we'll look for the text directly

        // Word stores text in a specific section, we need to find ccpText
        // Offset 0x4C: ccpText (character count in main document)
        if data.len() > 0x50 {
            let ccp_text =
                u32::from_le_bytes([data[0x4C], data[0x4D], data[0x4E], data[0x4F]]) as usize;

            // In Word 97-2003, text starts after the FIB header
            // The text can be either ANSI (1 byte per char) or Unicode (2 bytes per char)
            // fcMin at offset 0x18 gives the start position

            if data.len() > 0x1C {
                let fc_min =
                    u32::from_le_bytes([data[0x18], data[0x19], data[0x1A], data[0x1B]]) as usize;

                // Check if using Unicode (bit 10 of flags at offset 0xA)
                let flags = u16::from_le_bytes([data[0xA], data[0xB]]);
                let is_unicode = (flags & 0x0400) != 0;

                if fc_min > 0 && fc_min < data.len() {
                    let text_end = if is_unicode {
                        fc_min.saturating_add(ccp_text * 2)
                    } else {
                        fc_min.saturating_add(ccp_text)
                    };

                    if text_end <= data.len() && ccp_text > 0 && ccp_text < 10_000_000 {
                        let text_data = &data[fc_min..text_end.min(data.len())];

                        if is_unicode {
                            return Self::decode_utf16_text(text_data);
                        } else {
                            return Self::decode_cp1252_text(text_data);
                        }
                    }
                }
            }
        }

        // Fallback: scan for text patterns
        Self::extract_unicode_and_ansi_text(data)
    }

    /// Decode UTF-16LE text to String
    fn decode_utf16_text(data: &[u8]) -> Option<String> {
        if data.len() < 2 {
            return None;
        }

        let u16_iter = data
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]));

        let text: String = char::decode_utf16(u16_iter)
            .filter_map(|r| r.ok())
            .filter(|&c| c >= ' ' || c == '\n' || c == '\r' || c == '\t')
            .map(|c| if c == '\r' { '\n' } else { c })
            .collect();

        let cleaned = Self::clean_extracted_text(&text);
        if cleaned.len() > 10 {
            Some(cleaned)
        } else {
            None
        }
    }

    /// Decode CP1252 (Windows-1252) text to String
    fn decode_cp1252_text(data: &[u8]) -> Option<String> {
        // CP1252 to Unicode mapping for 0x80-0x9F range
        const CP1252_MAP: [char; 32] = [
            '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž',
            '\u{8F}', '\u{90}', '\u{2018}', '\u{2019}', '"', '"', '•', '–', '—', '˜', '™', 'š',
            '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
        ];

        let text: String = data
            .iter()
            .map(|&b| {
                if b < 0x80 {
                    b as char
                } else if b < 0xA0 {
                    CP1252_MAP[(b - 0x80) as usize]
                } else {
                    // 0xA0-0xFF maps directly to Unicode
                    char::from_u32(b as u32).unwrap_or('?')
                }
            })
            .filter(|&c| c >= ' ' || c == '\n' || c == '\r' || c == '\t')
            .map(|c| if c == '\r' { '\n' } else { c })
            .collect();

        let cleaned = Self::clean_extracted_text(&text);
        if cleaned.len() > 10 {
            Some(cleaned)
        } else {
            None
        }
    }

    /// Extract both Unicode (UTF-16LE) and ANSI text from binary data
    fn extract_unicode_and_ansi_text(data: &[u8]) -> Option<String> {
        let mut all_text = String::new();

        // Try to find UTF-16LE text runs (common in Word documents)
        let utf16_text = Self::find_utf16_text_runs(data);
        if !utf16_text.is_empty() {
            all_text.push_str(&utf16_text);
        }

        // Also try ANSI text extraction as fallback
        if all_text.len() < 50 {
            if let Some(ansi) = Self::find_ansi_text_runs(data) {
                if ansi.len() > all_text.len() {
                    all_text = ansi;
                }
            }
        }

        let cleaned = Self::clean_extracted_text(&all_text);
        if cleaned.len() > 10 {
            Some(cleaned)
        } else {
            None
        }
    }

    /// Find UTF-16LE text runs in binary data
    fn find_utf16_text_runs(data: &[u8]) -> String {
        let mut result = String::new();
        let mut i = 0;

        while i + 1 < data.len() {
            let mut run = String::new();

            while i + 1 < data.len() {
                let code_unit = u16::from_le_bytes([data[i], data[i + 1]]);

                // Check if it's a valid printable character or whitespace
                if let Some(c) = char::from_u32(code_unit as u32) {
                    if c.is_alphanumeric() || c.is_whitespace() || ".,;:!?()-\"'".contains(c) {
                        run.push(c);
                        i += 2;
                    } else if code_unit == 0x000D || code_unit == 0x000A {
                        run.push('\n');
                        i += 2;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }

            // Only keep runs with at least 5 characters (likely real text)
            if run.chars().filter(|c| c.is_alphabetic()).count() >= 3 {
                if !result.is_empty() && !result.ends_with('\n') {
                    result.push(' ');
                }
                result.push_str(&run);
            }

            i += 2;
        }

        result
    }

    /// Find ANSI text runs in binary data
    fn find_ansi_text_runs(data: &[u8]) -> Option<String> {
        let mut result = String::new();
        let mut run = String::new();

        for &byte in data {
            let c = byte as char;

            if c.is_alphanumeric()
                || c.is_whitespace()
                || ".,;:!?()-\"'äöüÄÖÜßéèêëàâáíìîïóòôõúùûñçÑ".contains(c)
            {
                run.push(c);
            } else if byte == 0x0D || byte == 0x0A {
                run.push('\n');
            } else {
                // End of run - keep if substantial
                if run.chars().filter(|c| c.is_alphabetic()).count() >= 4 {
                    if !result.is_empty() && !result.ends_with('\n') {
                        result.push(' ');
                    }
                    result.push_str(&run);
                }
                run.clear();
            }
        }

        // Don't forget the last run
        if run.chars().filter(|c| c.is_alphabetic()).count() >= 4 {
            if !result.is_empty() {
                result.push(' ');
            }
            result.push_str(&run);
        }

        if result.len() > 20 {
            Some(result)
        } else {
            None
        }
    }

    /// Clean up extracted text
    fn clean_extracted_text(text: &str) -> String {
        // Remove control characters and normalize whitespace
        let text: String = text
            .chars()
            .filter(|&c| c >= ' ' || c == '\n' || c == '\t')
            .collect();

        // Collapse multiple spaces/newlines
        let mut result = String::new();
        let mut prev_was_space = false;
        let mut prev_was_newline = false;

        for c in text.chars() {
            if c == '\n' {
                if !prev_was_newline {
                    result.push('\n');
                    prev_was_newline = true;
                }
                prev_was_space = false;
            } else if c.is_whitespace() {
                if !prev_was_space && !prev_was_newline {
                    result.push(' ');
                    prev_was_space = true;
                }
            } else {
                result.push(c);
                prev_was_space = false;
                prev_was_newline = false;
            }
        }

        result.trim().to_string()
    }
}

#[async_trait]
impl DocumentConverter for DocConverter {
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
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let mut document = Self::convert_doc(&bytes, options.as_ref())?;

        // If LLM client is provided, get descriptions for all images
        if let Some(ref opts) = options {
            if let Some(llm_client) = opts.image_description_llm() {
                if let Some(path) = opts.image_context_path.as_deref() {
                    document.apply_image_context_path(path);
                }
                document = document
                    .with_image_descriptions(llm_client.as_ref())
                    .await?;
            }
        }

        Ok(document)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".doc"]
    }
}

/// Legacy Excel spreadsheet (.xls) converter
pub struct XlsConverter;

impl XlsConverter {
    fn convert_xls(bytes: &[u8]) -> Result<Document, MarkitdownError> {
        // Use calamine which already supports .xls
        use calamine::{open_workbook_auto_from_rs, Reader};

        let cursor = Cursor::new(bytes.to_vec());
        let mut workbook = open_workbook_auto_from_rs(cursor)
            .map_err(|e| MarkitdownError::ParseError(format!("XLS parse error: {}", e)))?;

        let mut document = Document::new();
        let mut markdown = String::new();

        markdown.push_str("# Excel Spreadsheet (.xls)\n\n");

        let sheet_names = workbook.sheet_names().to_vec();

        for sheet_name in &sheet_names {
            if let Ok(range) = workbook.worksheet_range(sheet_name) {
                markdown.push_str(&format!("## Sheet: {}\n\n", sheet_name));

                let rows: Vec<_> = range.rows().collect();
                if rows.is_empty() {
                    markdown.push_str("*Empty sheet*\n\n");
                    continue;
                }

                // Create table
                let num_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
                if num_cols == 0 {
                    continue;
                }

                // Header row
                if let Some(first_row) = rows.first() {
                    let headers: Vec<String> = first_row
                        .iter()
                        .map(|c| format!("{}", c).replace('|', "\\|"))
                        .collect();
                    markdown.push_str(&format!("| {} |\n", headers.join(" | ")));
                    markdown.push_str(&format!("|{}|\n", vec!["---"; headers.len()].join("|")));
                }

                // Data rows (skip header)
                for row in rows.iter().skip(1).take(100) {
                    // Limit rows
                    let cells: Vec<String> = row
                        .iter()
                        .map(|c| format!("{}", c).replace('|', "\\|"))
                        .collect();
                    markdown.push_str(&format!("| {} |\n", cells.join(" | ")));
                }

                if rows.len() > 101 {
                    markdown.push_str(&format!("\n*... and {} more rows*\n", rows.len() - 101));
                }
                markdown.push('\n');
            }
        }

        let mut page = Page::new(1);
        page.add_content(ContentBlock::Markdown(markdown));
        document.add_page(page);
        Ok(document)
    }
}

#[async_trait]
impl DocumentConverter for XlsConverter {
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
        Self::convert_xls(&bytes)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".xls"]
    }
}

/// Legacy PowerPoint presentation (.ppt) converter
pub struct PptConverter;

impl PptConverter {
    fn convert_ppt(
        bytes: &[u8],
        options: Option<&ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let cursor = Cursor::new(bytes);
        let mut cfb = CompoundFile::open(cursor)
            .map_err(|e| MarkitdownError::ParseError(format!("PPT parse error: {}", e)))?;

        let mut document = Document::new();
        let mut page = Page::new(1);
        let mut markdown = String::new();

        markdown.push_str("# PowerPoint Presentation (.ppt)\n\n");

        // Try to extract text from PowerPoint Document stream
        let text = if let Ok(mut stream) = cfb.open_stream("/PowerPoint Document") {
            let mut data = Vec::new();
            stream.read_to_end(&mut data).ok();
            Self::extract_text_from_ppt_stream(&data)
        } else {
            None
        };

        if let Some(text) = text {
            if !text.trim().is_empty() {
                markdown.push_str(&text);
            } else {
                markdown.push_str("*Presentation appears to be empty or contains only images.*\n");
            }
        } else {
            markdown.push_str("*Unable to extract text from legacy PowerPoint document.*\n\n");
            markdown.push_str("This .ppt file uses a binary format. Consider converting to .pptx for better extraction.\n");
        }

        page.add_content(ContentBlock::Markdown(markdown));

        // Extract images if enabled
        let extract_images = options.map(|o| o.extract_images).unwrap_or(true);
        if extract_images {
            // Re-open CFB to extract images from Pictures stream
            let cursor = Cursor::new(bytes);
            if let Ok(mut cfb) = CompoundFile::open(cursor) {
                let images = Self::extract_images_from_ppt(&mut cfb);
                for image in images {
                    let mut extracted = ExtractedImage::new(
                        format!("ppt_image_{}", image.index),
                        Bytes::from(image.data),
                        image.format.mime_type(),
                    );
                    extracted.alt_text = Some(format!(
                        "slide_image{}.{}",
                        image.index,
                        image.format.extension()
                    ));
                    extracted.page_number = Some(1);
                    page.add_content(ContentBlock::Image(extracted));
                }
            }
        }

        document.add_page(page);
        Ok(document)
    }

    /// Extract images from a PPT file's Pictures stream
    /// The Pictures stream contains BLIP (Binary Large Image/Picture) records
    /// Each BLIP has a header followed by raw image data
    fn extract_images_from_ppt<R: Read + std::io::Seek>(
        cfb: &mut CompoundFile<R>,
    ) -> Vec<LegacyImage> {
        let mut images = Vec::new();

        // Read the Pictures stream
        if let Ok(mut stream) = cfb.open_stream("/Pictures") {
            let mut data = Vec::new();
            if stream.read_to_end(&mut data).is_ok() && !data.is_empty() {
                images = Self::parse_pictures_stream(&data);
            }
        }

        // Also check for embedded objects in ObjectPool
        // PowerPoint sometimes stores images as OLE objects
        if images.is_empty() {
            if let Ok(mut stream) = cfb.open_stream("/PowerPoint Document") {
                let mut data = Vec::new();
                if stream.read_to_end(&mut data).is_ok() {
                    let found = Self::scan_ppt_for_images(&data);
                    images.extend(found);
                }
            }
        }

        images
    }

    /// Parse the Pictures stream for BLIP records
    /// BLIP format in PPT:
    /// - 8 bytes: record header (ver/inst, type, length)
    /// - 16 or 32 bytes: UID (MD4 hash)
    /// - Optional: 1 byte tag
    /// - Image data
    fn parse_pictures_stream(data: &[u8]) -> Vec<LegacyImage> {
        let mut images = Vec::new();
        let mut pos = 0;

        // BLIP type constants from MS-ODRAW
        const BLIP_EMF: u16 = 0xF01A;
        const BLIP_WMF: u16 = 0xF01B;
        const BLIP_PICT: u16 = 0xF01C;
        const BLIP_JPEG: u16 = 0xF01D;
        const BLIP_PNG: u16 = 0xF01E;
        const BLIP_DIB: u16 = 0xF01F;
        const BLIP_JPEG2: u16 = 0xF02A; // JPEG with secondary UID

        while pos + 8 <= data.len() {
            // Read record header
            let rec_ver_inst = u16::from_le_bytes([data[pos], data[pos + 1]]);
            let rec_type = u16::from_le_bytes([data[pos + 2], data[pos + 3]]);
            let rec_len =
                u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]])
                    as usize;

            pos += 8;

            if rec_len == 0 || rec_len > data.len() - pos {
                pos += 1;
                continue;
            }

            // Check if this is a BLIP record
            let (format, header_size) = match rec_type {
                BLIP_EMF => (ImageFormat::Emf, 50), // 16 (UID) + 34 (metafile header)
                BLIP_WMF => (ImageFormat::Wmf, 50),
                BLIP_PICT => (ImageFormat::Pict, 50),
                BLIP_JPEG | BLIP_JPEG2 => {
                    // JPEG has 1 byte tag after 16/32 byte UID
                    let has_secondary = (rec_ver_inst >> 4) == 0x46B; // instance for secondary UID
                    (ImageFormat::Jpeg, if has_secondary { 33 } else { 17 })
                }
                BLIP_PNG => {
                    let has_secondary = (rec_ver_inst >> 4) == 0x6E1;
                    (ImageFormat::Png, if has_secondary { 33 } else { 17 })
                }
                BLIP_DIB => (ImageFormat::Dib, 17),
                _ => {
                    // Unknown record, skip
                    pos += rec_len;
                    continue;
                }
            };

            let record_data = &data[pos..pos + rec_len];

            // Skip the header to get to raw image data
            if record_data.len() > header_size {
                let img_data = &record_data[header_size..];

                // Verify the image data looks valid
                let detected_format = ImageFormat::detect(img_data);
                let final_format = if detected_format != ImageFormat::Unknown {
                    detected_format
                } else {
                    format
                };

                if img_data.len() > 16 {
                    images.push(LegacyImage {
                        data: img_data.to_vec(),
                        format: final_format,
                        index: images.len() + 1,
                    });
                }
            }

            pos += rec_len;
        }

        images
    }

    /// Scan PPT document stream for embedded images
    fn scan_ppt_for_images(data: &[u8]) -> Vec<LegacyImage> {
        let mut images = Vec::new();
        let mut pos = 0;

        while pos + 16 < data.len() {
            // Look for image signatures
            let slice = &data[pos..];

            // JPEG: FF D8 FF
            if slice.starts_with(&[0xFF, 0xD8, 0xFF]) {
                if let Some(end) = DocConverter::find_jpeg_end(slice) {
                    images.push(LegacyImage {
                        data: slice[..end].to_vec(),
                        format: ImageFormat::Jpeg,
                        index: images.len() + 1,
                    });
                    pos += end;
                    continue;
                }
            }

            // PNG: 89 50 4E 47 0D 0A 1A 0A
            if slice.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
                if let Some(end) = DocConverter::find_png_end(slice) {
                    images.push(LegacyImage {
                        data: slice[..end].to_vec(),
                        format: ImageFormat::Png,
                        index: images.len() + 1,
                    });
                    pos += end;
                    continue;
                }
            }

            pos += 1;
        }

        images
    }

    /// Extract text from the PowerPoint Document stream
    /// PPT uses a record-based format where each record has:
    /// - 2 bytes: record version and instance
    /// - 2 bytes: record type
    /// - 4 bytes: record length
    fn extract_text_from_ppt_stream(data: &[u8]) -> Option<String> {
        let mut texts: Vec<String> = Vec::new();
        let mut slide_texts: Vec<Vec<String>> = Vec::new();
        let mut current_slide_texts: Vec<String> = Vec::new();

        // PowerPoint record types for text
        const RT_TEXT_CHARS_ATOM: u16 = 0x0FA0; // Unicode text
        const RT_TEXT_BYTES_ATOM: u16 = 0x0FA8; // ANSI text
        const RT_CSTRING: u16 = 0x0FBA; // Unicode C-string
        const RT_SLIDE: u16 = 0x03EE; // Slide container
        const RT_SLIDE_PERSIST_ATOM: u16 = 0x03F3; // Slide persist

        let mut pos = 0;

        while pos + 8 <= data.len() {
            // Read record header
            let rec_ver_instance = u16::from_le_bytes([data[pos], data[pos + 1]]);
            let rec_type = u16::from_le_bytes([data[pos + 2], data[pos + 3]]);
            let rec_len =
                u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]])
                    as usize;

            let rec_ver = rec_ver_instance & 0x0F;

            pos += 8;

            if rec_len > data.len().saturating_sub(pos) {
                // Invalid record length, try to continue
                pos += 1;
                continue;
            }

            let rec_data = &data[pos..pos + rec_len.min(data.len() - pos)];

            match rec_type {
                RT_SLIDE | RT_SLIDE_PERSIST_ATOM => {
                    // New slide started - save previous slide's texts
                    if !current_slide_texts.is_empty() {
                        slide_texts.push(current_slide_texts.clone());
                        current_slide_texts.clear();
                    }
                }
                RT_TEXT_CHARS_ATOM | RT_CSTRING => {
                    // Unicode text (UTF-16LE)
                    if let Some(text) = Self::decode_utf16_text(rec_data) {
                        let cleaned = text.trim().to_string();
                        if !cleaned.is_empty() && Self::is_meaningful_text(&cleaned) {
                            current_slide_texts.push(cleaned.clone());
                            texts.push(cleaned);
                        }
                    }
                }
                RT_TEXT_BYTES_ATOM => {
                    // ANSI text
                    if let Some(text) = Self::decode_ansi_text(rec_data) {
                        let cleaned = text.trim().to_string();
                        if !cleaned.is_empty() && Self::is_meaningful_text(&cleaned) {
                            current_slide_texts.push(cleaned.clone());
                            texts.push(cleaned);
                        }
                    }
                }
                _ => {
                    // For container records (version 0xF), don't skip the content
                    // as it may contain nested text records
                    if rec_ver == 0x0F {
                        // This is a container, text records are inside
                        // We'll process them in the next iteration
                        continue;
                    }
                }
            }

            pos += rec_len;
        }

        // Don't forget the last slide
        if !current_slide_texts.is_empty() {
            slide_texts.push(current_slide_texts);
        }

        // If we found slides with text, format nicely
        if !slide_texts.is_empty() {
            let mut result = String::new();
            for (i, slide) in slide_texts.iter().enumerate() {
                if !slide.is_empty() {
                    result.push_str(&format!("## Slide {}\n\n", i + 1));
                    for text in slide {
                        result.push_str(text);
                        result.push_str("\n\n");
                    }
                }
            }
            if !result.is_empty() {
                return Some(result);
            }
        }

        // Fallback: return all texts found
        if !texts.is_empty() {
            // Deduplicate while preserving order
            let mut seen = std::collections::HashSet::new();
            let unique: Vec<_> = texts
                .into_iter()
                .filter(|t| seen.insert(t.clone()))
                .collect();
            return Some(unique.join("\n\n"));
        }

        // Last resort: try to find any text patterns
        Self::find_text_patterns(data)
    }

    /// Decode UTF-16LE text
    fn decode_utf16_text(data: &[u8]) -> Option<String> {
        if data.len() < 2 {
            return None;
        }

        let u16_iter = data
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]));

        let text: String = char::decode_utf16(u16_iter)
            .filter_map(|r| r.ok())
            .filter(|&c| c >= ' ' || c == '\n' || c == '\r' || c == '\t')
            .map(|c| if c == '\r' { '\n' } else { c })
            .collect();

        let trimmed = text.trim();
        if trimmed.len() >= 2 {
            Some(trimmed.to_string())
        } else {
            None
        }
    }

    /// Decode ANSI text
    fn decode_ansi_text(data: &[u8]) -> Option<String> {
        let text: String = data
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| {
                if (0x20..0x7F).contains(&b) {
                    b as char
                } else if b == 0x0D || b == 0x0A {
                    '\n'
                } else if b == 0x09 {
                    ' '
                } else if b >= 0xA0 {
                    // Extended ASCII - try to map to Unicode
                    char::from_u32(b as u32).unwrap_or(' ')
                } else {
                    ' '
                }
            })
            .collect();

        let trimmed = text.trim();
        if trimmed.len() >= 2 {
            Some(trimmed.to_string())
        } else {
            None
        }
    }

    /// Check if text is meaningful (not just garbage)
    fn is_meaningful_text(text: &str) -> bool {
        // Must have at least some alphabetic characters
        let alpha_count = text.chars().filter(|c| c.is_alphabetic()).count();
        let total_count = text.chars().count();

        if total_count == 0 {
            return false;
        }

        // At least 30% should be alphabetic
        let ratio = alpha_count as f64 / total_count as f64;

        // Reject if it looks like binary garbage
        let has_garbage = text.contains("\x00")
            || text.contains("[Content_Types]")
            || text
                .chars()
                .filter(|c| !c.is_ascii_graphic() && !c.is_whitespace())
                .count()
                > total_count / 4;

        ratio > 0.3 && !has_garbage && alpha_count >= 2
    }

    /// Find text patterns in binary data as last resort
    fn find_text_patterns(data: &[u8]) -> Option<String> {
        let mut texts: Vec<String> = Vec::new();

        // Look for UTF-16LE text runs
        let mut i = 0;
        while i + 1 < data.len() {
            let mut run = String::new();
            let start = i;

            while i + 1 < data.len() {
                let code_unit = u16::from_le_bytes([data[i], data[i + 1]]);

                if let Some(c) = char::from_u32(code_unit as u32) {
                    if c.is_alphanumeric() || c.is_whitespace() || ".,;:!?()-\"'äöüÄÖÜß".contains(c)
                    {
                        run.push(c);
                        i += 2;
                    } else if code_unit == 0x000D || code_unit == 0x000A {
                        run.push('\n');
                        i += 2;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }

            let trimmed = run.trim();
            if trimmed.len() >= 4 && Self::is_meaningful_text(trimmed) {
                texts.push(trimmed.to_string());
            }

            i = start + 2;
            if i <= start {
                i = start + 1;
            }
        }

        if texts.is_empty() {
            return None;
        }

        // Deduplicate
        let mut seen = std::collections::HashSet::new();
        let unique: Vec<_> = texts
            .into_iter()
            .filter(|t| t.len() >= 3 && seen.insert(t.clone()))
            .collect();

        if unique.is_empty() {
            None
        } else {
            Some(unique.join("\n\n"))
        }
    }
}

#[async_trait]
impl DocumentConverter for PptConverter {
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
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let mut document = Self::convert_ppt(&bytes, options.as_ref())?;

        // If LLM client is provided, get descriptions for all images
        if let Some(ref opts) = options {
            if let Some(llm_client) = opts.image_description_llm() {
                if let Some(path) = opts.image_context_path.as_deref() {
                    document.apply_image_context_path(path);
                }
                document = document
                    .with_image_descriptions(llm_client.as_ref())
                    .await?;
            }
        }

        Ok(document)
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".ppt"]
    }
}

/// Word template (.dotx) converter - uses same format as .docx
pub struct DotxConverter;

#[async_trait]
impl DocumentConverter for DotxConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        // DOTX uses the same format as DOCX - override extension
        let mut opts = options.unwrap_or_default();
        opts.file_extension = Some(".docx".to_string());
        let docx = crate::docx::DocxConverter;
        docx.convert(store, path, Some(opts)).await
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        // DOTX uses the same format as DOCX - override extension
        let mut opts = options.unwrap_or_default();
        opts.file_extension = Some(".docx".to_string());
        let docx = crate::docx::DocxConverter;
        docx.convert_bytes(bytes, Some(opts)).await
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".dotx", ".dotm"]
    }
}

/// PowerPoint template (.potx) converter - uses same format as .pptx
pub struct PotxConverter;

#[async_trait]
impl DocumentConverter for PotxConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        // POTX uses the same format as PPTX - override extension
        let mut opts = options.unwrap_or_default();
        opts.file_extension = Some(".pptx".to_string());
        let pptx = crate::pptx::PptxConverter;
        pptx.convert(store, path, Some(opts)).await
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        // POTX uses the same format as PPTX - override extension
        let mut opts = options.unwrap_or_default();
        opts.file_extension = Some(".pptx".to_string());
        let pptx = crate::pptx::PptxConverter;
        pptx.convert_bytes(bytes, Some(opts)).await
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".potx", ".potm"]
    }
}

/// Excel template (.xltx) converter - uses same format as .xlsx
pub struct XltxConverter;

#[async_trait]
impl DocumentConverter for XltxConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        // XLTX uses the same format as XLSX - override extension
        let mut opts = options.unwrap_or_default();
        opts.file_extension = Some(".xlsx".to_string());
        let xlsx = crate::excel::ExcelConverter;
        xlsx.convert(store, path, Some(opts)).await
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        // XLTX uses the same format as XLSX - override extension
        let mut opts = options.unwrap_or_default();
        opts.file_extension = Some(".xlsx".to_string());
        let xlsx = crate::excel::ExcelConverter;
        xlsx.convert_bytes(bytes, Some(opts)).await
    }

    fn supported_extensions(&self) -> &[&str] {
        &[".xltx", ".xltm"]
    }
}
