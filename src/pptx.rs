//! PowerPoint (.pptx) converter.
//!
//! Slides come in presentation order, read from `ppt/presentation.xml`, not in the order of
//! their file names: some tools name the parts `slide71.xml`, `slide122.xml` …, and PowerPoint's
//! own names stop matching the order as soon as a slide is moved. Each slide resolves its own
//! relationships (`ppt/slides/_rels/<slide file>.rels`).
//!
//! A slide becomes its content in the order of the slide XML: text boxes (one line per
//! paragraph), tables, pictures (with the alt text set in PowerPoint) and native charts, whose
//! cached data becomes a table. Background pictures, the fallback copies inside
//! `mc:AlternateContent` and the footer, date and slide number placeholders are left out.

use crate::error::MarkitdownError;
use crate::model::{
    ContentBlock, ConversionOptions, Document, DocumentConverter, ExtractedImage, Page,
};
use async_trait::async_trait;
use bytes::Bytes;
use object_store::{ObjectStore, ObjectStoreExt};
use quick_xml::events::{BytesRef, BytesStart, Event};
use quick_xml::reader::Reader;
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::Arc;
use zip::ZipArchive;

pub struct PptxConverter;

type Archive<'a> = ZipArchive<Cursor<&'a [u8]>>;

/// Most rows a chart table gets; a chart with more points is cut there
const MAX_CHART_ROWS: usize = 500;

/// Something on a slide, in the order of the slide XML
#[derive(Debug, Clone, PartialEq)]
enum SlideItem {
    Text(String),
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Picture {
        rel_id: String,
        alt: Option<String>,
    },
    Chart {
        rel_id: String,
    },
}

fn xml_error(e: quick_xml::Error) -> MarkitdownError {
    MarkitdownError::ParseError(format!("Failed to read XML event: {}", e))
}

fn read_part(archive: &mut Archive, name: &str) -> Option<Vec<u8>> {
    let mut file = archive.by_name(name).ok()?;
    let mut data = Vec::new();
    file.read_to_end(&mut data).ok()?;
    Some(data)
}

fn read_xml(archive: &mut Archive, name: &str) -> Option<String> {
    read_part(archive, name).map(|data| String::from_utf8_lossy(&data).into_owned())
}

use crate::resolve_part_target as resolve_target;

/// Value of a namespaced attribute such as `r:embed` or `r:id`, matched by local name
fn rel_attr(element: &BytesStart, local: &[u8]) -> Option<String> {
    element.attributes().flatten().find_map(|attr| {
        (attr.key.prefix().is_some() && attr.key.local_name().as_ref() == local)
            .then(|| String::from_utf8_lossy(&attr.value).into_owned())
    })
}

/// Alt text of a shape (`descr` on `p:cNvPr`), if set
fn alt_text(element: &BytesStart) -> Option<String> {
    element.attributes().flatten().find_map(|attr| {
        if attr.key.as_ref() != b"descr" {
            return None;
        }
        let text = attr.unescape_value().ok()?.trim().to_string();
        (!text.is_empty()).then_some(text)
    })
}

/// Placeholder for the footer, date or slide number (`<p:ph type="ftr"/>` …)
fn is_furniture(element: &BytesStart) -> bool {
    element.attributes().flatten().any(|attr| {
        attr.key.as_ref() == b"type" && matches!(attr.value.as_ref(), b"ftr" | b"dt" | b"sldNum")
    })
}

/// `&amp;`, `&#8364;` … in text: quick-xml reports entity references as their own events
fn push_entity(out: &mut String, reference: &BytesRef) {
    if let Ok(Some(ch)) = reference.resolve_char_ref() {
        out.push(ch);
        return;
    }
    match reference.decode().as_deref() {
        Ok("amp") => out.push('&'),
        Ok("lt") => out.push('<'),
        Ok("gt") => out.push('>'),
        Ok("quot") => out.push('"'),
        Ok("apos") => out.push('\''),
        _ => {}
    }
}

impl PptxConverter {
    /// Detect MIME type from bytes
    fn detect_mime_type(bytes: &[u8]) -> String {
        if bytes.len() >= 8 {
            if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
                return "image/png".to_string();
            }
            if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
                return "image/jpeg".to_string();
            }
            if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
                return "image/gif".to_string();
            }
            // WebP: RIFF....WEBP
            if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
                return "image/webp".to_string();
            }
            // BMP: BM
            if bytes.starts_with(b"BM") {
                return "image/bmp".to_string();
            }
            // EMF (Enhanced Metafile)
            if bytes.starts_with(&[0x01, 0x00, 0x00, 0x00]) {
                return "image/emf".to_string();
            }
            // WMF (Windows Metafile)
            if bytes.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) {
                return "image/wmf".to_string();
            }
        }
        "application/octet-stream".to_string()
    }

    /// Internal relationships of a part: id → resolved part name. External targets are left out.
    fn relationships(archive: &mut Archive, part: &str) -> HashMap<String, String> {
        let (dir, file) = part.rsplit_once('/').unwrap_or(("", part));
        let rels_path = if dir.is_empty() {
            format!("_rels/{}.rels", file)
        } else {
            format!("{}/_rels/{}.rels", dir, file)
        };
        let mut relationships = HashMap::new();
        let Some(xml) = read_xml(archive, &rels_path) else {
            return relationships;
        };

        let mut reader = Reader::from_str(&xml);
        loop {
            match reader.read_event() {
                Ok(Event::Start(element)) | Ok(Event::Empty(element))
                    if element.local_name().as_ref() == b"Relationship" =>
                {
                    let mut id = None;
                    let mut target = None;
                    let mut external = false;
                    for attr in element.attributes().flatten() {
                        let value = String::from_utf8_lossy(&attr.value).into_owned();
                        match attr.key.as_ref() {
                            b"Id" => id = Some(value),
                            b"Target" => target = Some(value),
                            b"TargetMode" => external = value.eq_ignore_ascii_case("External"),
                            _ => {}
                        }
                    }
                    if let (Some(id), Some(target), false) = (id, target, external) {
                        relationships.insert(id, resolve_target(dir, &target));
                    }
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
        relationships
    }

    /// Slide parts in presentation order (`p:sldIdLst`). Without a usable presentation part,
    /// all `ppt/slides/slideN.xml` sorted by N.
    fn slide_paths(archive: &mut Archive) -> Vec<String> {
        let rels = Self::relationships(archive, "ppt/presentation.xml");
        let mut ordered: Vec<String> = Vec::new();
        if let Some(xml) = read_xml(archive, "ppt/presentation.xml") {
            let mut reader = Reader::from_str(&xml);
            loop {
                match reader.read_event() {
                    Ok(Event::Start(element)) | Ok(Event::Empty(element))
                        if element.local_name().as_ref() == b"sldId" =>
                    {
                        if let Some(path) = rel_attr(&element, b"id").and_then(|id| rels.get(&id)) {
                            if !ordered.contains(path) {
                                ordered.push(path.clone());
                            }
                        }
                    }
                    Ok(Event::Eof) | Err(_) => break,
                    _ => {}
                }
            }
        }
        ordered.retain(|path| archive.by_name(path).is_ok());
        if !ordered.is_empty() {
            return ordered;
        }

        let mut slides: Vec<(u64, String)> = archive
            .file_names()
            .filter_map(|name| {
                let name = name.ok()?;
                let number = name
                    .strip_prefix("ppt/slides/slide")?
                    .strip_suffix(".xml")?
                    .parse::<u64>()
                    .ok()?;
                Some((number, name.to_string()))
            })
            .collect();
        slides.sort();
        slides.into_iter().map(|(_, name)| name).collect()
    }

    /// Everything on a slide, in document order
    fn parse_slide(xml: &str) -> Result<Vec<SlideItem>, MarkitdownError> {
        let mut reader = Reader::from_str(xml);
        let mut items = Vec::new();
        // Alt text of the shape being read (p:cNvPr comes before the picture's blip)
        let mut alt: Option<String> = None;
        // The shape is a footer, date or slide number placeholder: its text is noise
        let mut furniture = false;

        loop {
            match reader.read_event().map_err(xml_error)? {
                Event::Start(element) => match element.local_name().as_ref() {
                    // Background and fallback copies: skip with everything inside
                    b"bg" | b"Fallback" => {
                        reader.read_to_end(element.name()).map_err(xml_error)?;
                    }
                    b"sp" | b"pic" | b"graphicFrame" | b"cxnSp" => {
                        alt = None;
                        furniture = false;
                    }
                    b"cNvPr" => alt = alt_text(&element),
                    b"ph" => furniture = is_furniture(&element),
                    b"txBody" if furniture => {
                        reader.read_to_end(element.name()).map_err(xml_error)?;
                    }
                    b"txBody" => {
                        if let Some(text) = Self::read_text_body(&mut reader)? {
                            items.push(SlideItem::Text(text));
                        }
                    }
                    b"tbl" => {
                        if let Some(table) = Self::read_table(&mut reader)? {
                            items.push(table);
                        }
                    }
                    b"blip" => {
                        if let Some(rel_id) = rel_attr(&element, b"embed") {
                            items.push(SlideItem::Picture {
                                rel_id,
                                alt: alt.clone(),
                            });
                        }
                    }
                    b"chart" => {
                        if let Some(rel_id) = rel_attr(&element, b"id") {
                            items.push(SlideItem::Chart { rel_id });
                        }
                    }
                    _ => {}
                },
                Event::Empty(element) => match element.local_name().as_ref() {
                    b"cNvPr" => alt = alt_text(&element),
                    b"ph" => furniture = is_furniture(&element),
                    b"blip" => {
                        if let Some(rel_id) = rel_attr(&element, b"embed") {
                            items.push(SlideItem::Picture {
                                rel_id,
                                alt: alt.clone(),
                            });
                        }
                    }
                    b"chart" => {
                        if let Some(rel_id) = rel_attr(&element, b"id") {
                            items.push(SlideItem::Chart { rel_id });
                        }
                    }
                    _ => {}
                },
                Event::Eof => break,
                _ => {}
            }
        }
        Ok(items)
    }

    /// Text of a text body: one line per paragraph, `a:br` as a line break inside it.
    /// Text is taken from `a:t` exactly as written, spaces between runs included.
    fn read_text_body(reader: &mut Reader<&[u8]>) -> Result<Option<String>, MarkitdownError> {
        let mut lines: Vec<String> = Vec::new();
        let mut line = String::new();
        let mut in_text = false;
        loop {
            match reader.read_event().map_err(xml_error)? {
                Event::Start(element) if element.local_name().as_ref() == b"t" => in_text = true,
                Event::End(element) => match element.local_name().as_ref() {
                    b"t" => in_text = false,
                    b"p" => {
                        let text = line.trim();
                        if !text.is_empty() {
                            lines.push(text.to_string());
                        }
                        line.clear();
                    }
                    b"txBody" => break,
                    _ => {}
                },
                Event::Empty(element) if element.local_name().as_ref() == b"br" => line.push('\n'),
                Event::Text(text) if in_text => line.push_str(&text.decode().map_err(|e| {
                    MarkitdownError::ParseError(format!("Failed to decode text: {}", e))
                })?),
                Event::GeneralRef(reference) if in_text => push_entity(&mut line, &reference),
                Event::Eof => break,
                _ => {}
            }
        }
        let text = line.trim();
        if !text.is_empty() {
            lines.push(text.to_string());
        }
        Ok((!lines.is_empty()).then(|| lines.join("\n")))
    }

    /// A table: first row as headers. Paragraphs inside a cell are joined with a space.
    fn read_table(reader: &mut Reader<&[u8]>) -> Result<Option<SlideItem>, MarkitdownError> {
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut cell: Option<String> = None;
        let mut paragraph = String::new();
        let mut in_text = false;
        loop {
            match reader.read_event().map_err(xml_error)? {
                Event::Start(element) => match element.local_name().as_ref() {
                    b"tr" => rows.push(Vec::new()),
                    b"tc" => cell = Some(String::new()),
                    b"t" => in_text = true,
                    _ => {}
                },
                Event::Empty(element) => match element.local_name().as_ref() {
                    b"tr" => rows.push(Vec::new()),
                    b"tc" => {
                        if let Some(row) = rows.last_mut() {
                            row.push(String::new());
                        }
                    }
                    _ => {}
                },
                Event::End(element) => match element.local_name().as_ref() {
                    b"t" => in_text = false,
                    b"p" => {
                        if let Some(cell) = cell.as_mut() {
                            let text = paragraph.trim();
                            if !text.is_empty() {
                                if !cell.is_empty() {
                                    cell.push(' ');
                                }
                                cell.push_str(text);
                            }
                        }
                        paragraph.clear();
                    }
                    b"tc" => {
                        if let (Some(row), Some(text)) = (rows.last_mut(), cell.take()) {
                            row.push(text);
                        }
                    }
                    b"tbl" => break,
                    _ => {}
                },
                Event::Text(text) if in_text => {
                    paragraph.push_str(&text.decode().map_err(|e| {
                        MarkitdownError::ParseError(format!("Failed to decode text: {}", e))
                    })?)
                }
                Event::GeneralRef(reference) if in_text => push_entity(&mut paragraph, &reference),
                Event::Eof => break,
                _ => {}
            }
        }

        let mut rows = rows.into_iter();
        Ok(rows.next().map(|headers| SlideItem::Table {
            headers,
            rows: rows.collect(),
        }))
    }

    /// A native chart as a caption line and a table of its cached data: one row per category,
    /// one column per series. Empty when the chart holds no data and no title.
    fn parse_chart(xml: &str) -> Vec<ContentBlock> {
        #[derive(Default)]
        struct Series {
            name: String,
            categories: Vec<(usize, String)>,
            values: Vec<(usize, String)>,
        }

        let mut reader = Reader::from_str(xml);
        let mut path: Vec<Vec<u8>> = Vec::new();
        let mut title = String::new();
        let mut kind: Option<String> = None;
        let mut series: Vec<Series> = Vec::new();
        let mut point = 0usize;

        let mut take_text = |path: &[Vec<u8>],
                             text: &str,
                             point: usize,
                             series: &mut Vec<Series>| {
            let names: Vec<&[u8]> = path.iter().map(Vec::as_slice).collect();
            let last = names.last().copied().unwrap_or_default();
            // Chart title only, not axis titles (those sit under plotArea)
            if names.len() > 2 && names[1] == b"chart" && names[2] == b"title" {
                if last == b"t" || last == b"v" {
                    title.push_str(text);
                }
                return;
            }
            let Some(at) = names.iter().position(|name| *name == b"ser") else {
                return;
            };
            if last != b"v" {
                return;
            }
            let Some(current) = series.last_mut() else {
                return;
            };
            match names.get(at + 1).copied() {
                Some(b"tx") => current.name.push_str(text),
                Some(b"cat") | Some(b"xVal") => current.categories.push((point, text.to_string())),
                Some(b"val") | Some(b"yVal") => current.values.push((point, text.to_string())),
                _ => {}
            }
        };

        loop {
            match reader.read_event() {
                Ok(Event::Start(element)) => {
                    let name = element.local_name().as_ref().to_vec();
                    match name.as_slice() {
                        b"ser" => series.push(Series::default()),
                        b"pt" => {
                            point = element
                                .attributes()
                                .flatten()
                                .find(|attr| attr.key.as_ref() == b"idx")
                                .and_then(|attr| {
                                    String::from_utf8_lossy(&attr.value).parse::<usize>().ok()
                                })
                                .unwrap_or(0)
                        }
                        _ => {}
                    }
                    if kind.is_none()
                        && path.last().map(Vec::as_slice) == Some(b"plotArea".as_slice())
                        && name.ends_with(b"Chart")
                    {
                        let name = String::from_utf8_lossy(&name[..name.len() - 5]).into_owned();
                        kind = Some(name.trim_end_matches("3D").to_string());
                    }
                    path.push(name);
                }
                Ok(Event::End(_)) => {
                    path.pop();
                }
                Ok(Event::Text(text)) => {
                    if let Ok(text) = text.decode() {
                        take_text(&path, &text, point, &mut series);
                    }
                }
                Ok(Event::GeneralRef(reference)) => {
                    let mut text = String::new();
                    push_entity(&mut text, &reference);
                    take_text(&path, &text, point, &mut series);
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }

        let title = title.trim().to_string();
        let caption = match (kind.as_deref(), title.is_empty()) {
            (Some(kind), false) => format!("Chart ({}): {}", kind, title),
            (Some(kind), true) => format!("Chart ({})", kind),
            (None, false) => format!("Chart: {}", title),
            (None, true) => "Chart".to_string(),
        };

        let points = series
            .iter()
            .flat_map(|s| {
                s.categories
                    .iter()
                    .chain(s.values.iter())
                    .map(|(i, _)| i + 1)
            })
            .max()
            .unwrap_or(0)
            .min(MAX_CHART_ROWS);
        if points == 0 {
            return if title.is_empty() {
                Vec::new()
            } else {
                vec![ContentBlock::Text(caption)]
            };
        }

        let label = |values: &[(usize, String)], index: usize| {
            values
                .iter()
                .find(|(i, _)| *i == index)
                .map(|(_, value)| value.clone())
        };
        let mut headers = vec![String::new()];
        for (index, s) in series.iter().enumerate() {
            let name = s.name.trim();
            headers.push(if !name.is_empty() {
                name.to_string()
            } else if series.len() == 1 {
                "Value".to_string()
            } else {
                format!("Series {}", index + 1)
            });
        }
        let rows = (0..points)
            .map(|index| {
                let category = series
                    .iter()
                    .find_map(|s| label(&s.categories, index))
                    .unwrap_or_else(|| (index + 1).to_string());
                std::iter::once(category)
                    .chain(
                        series
                            .iter()
                            .map(|s| label(&s.values, index).unwrap_or_default()),
                    )
                    .collect()
            })
            .collect();

        vec![
            ContentBlock::Text(caption),
            ContentBlock::Table { headers, rows },
        ]
    }

    /// Convert bytes to Document
    fn bytes_to_document(
        &self,
        bytes: &[u8],
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let extract_images = options.as_ref().map(|o| o.extract_images).unwrap_or(true);
        let mut archive = crate::open_zip(Cursor::new(bytes))?;
        let mut document = Document::new();
        // The same picture is often on many slides (a logo): read it once
        let mut media: HashMap<String, Bytes> = HashMap::new();

        for (index, slide_path) in Self::slide_paths(&mut archive).iter().enumerate() {
            let slide_num = index as u32 + 1;
            let Some(xml) = read_xml(&mut archive, slide_path) else {
                continue;
            };
            let items = Self::parse_slide(&xml)?;
            let rels = Self::relationships(&mut archive, slide_path);

            let mut page = Page::new(slide_num);
            page.add_content(ContentBlock::Markdown(format!(
                "<!-- Slide {} -->\n",
                slide_num
            )));

            let mut picture = 0;
            for item in items {
                match item {
                    SlideItem::Text(text) => page.add_content(ContentBlock::Text(text)),
                    SlideItem::Table { headers, rows } => {
                        page.add_content(ContentBlock::Table { headers, rows })
                    }
                    SlideItem::Picture { rel_id, alt } if extract_images => {
                        let Some(target) = rels.get(&rel_id) else {
                            continue;
                        };
                        let data = media
                            .entry(target.clone())
                            .or_insert_with(|| {
                                read_part(&mut archive, target)
                                    .map(Bytes::from)
                                    .unwrap_or_default()
                            })
                            .clone();
                        if data.is_empty() {
                            continue;
                        }
                        picture += 1;
                        let mime_type = Self::detect_mime_type(&data);
                        let mut image = ExtractedImage::new(
                            format!("slide{}_image{}", slide_num, picture),
                            data,
                            mime_type,
                        );
                        image.alt_text = alt;
                        image.page_number = Some(slide_num);
                        page.add_content(ContentBlock::Image(image));
                    }
                    SlideItem::Picture { .. } => {}
                    SlideItem::Chart { rel_id } => {
                        if let Some(xml) = rels
                            .get(&rel_id)
                            .and_then(|target| read_xml(&mut archive, target))
                        {
                            for block in Self::parse_chart(&xml) {
                                page.add_content(block);
                            }
                        }
                    }
                }
            }

            document.add_page(page);
        }

        Ok(document)
    }
}

#[async_trait]
impl DocumentConverter for PptxConverter {
    async fn convert(
        &self,
        store: Arc<dyn ObjectStore>,
        path: &object_store::path::Path,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let result = store.get(path).await.map_err(|e| {
            MarkitdownError::ObjectStoreError(format!("Failed to get object: {}", e))
        })?;

        let bytes = result.bytes().await.map_err(|e| {
            MarkitdownError::ObjectStoreError(format!("Failed to read bytes: {}", e))
        })?;

        self.convert_bytes(bytes, options).await
    }

    async fn convert_bytes(
        &self,
        bytes: Bytes,
        options: Option<ConversionOptions>,
    ) -> Result<Document, MarkitdownError> {
        let mut document = self.bytes_to_document(&bytes, options.clone())?;

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
        &["pptx"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_resolve_against_the_part_folder() {
        assert_eq!(
            resolve_target("ppt/slides", "../media/image1.png"),
            "ppt/media/image1.png"
        );
        assert_eq!(
            resolve_target("ppt", "slides/slide3.xml"),
            "ppt/slides/slide3.xml"
        );
        assert_eq!(
            resolve_target("ppt/slides", "/ppt/media/a.png"),
            "ppt/media/a.png"
        );
        assert_eq!(
            resolve_target("ppt/slides", "./../charts/chart2.xml"),
            "ppt/charts/chart2.xml"
        );
    }

    #[test]
    fn slide_items_keep_order_spaces_entities_and_alt_text() {
        let xml = r#"<p:sld xmlns:p="p" xmlns:a="a" xmlns:r="r" xmlns:mc="mc">
            <p:cSld>
              <p:bg><p:bgPr><a:blipFill><a:blip r:embed="rIdBg"/></a:blipFill></p:bgPr></p:bg>
              <p:spTree>
                <p:sp><p:nvSpPr><p:cNvPr id="2" name="Title"/></p:nvSpPr>
                  <p:txBody><a:p><a:r><a:t>Tom </a:t></a:r><a:r><a:t>&amp; Jerry</a:t></a:r></a:p>
                  <a:p><a:r><a:t>Second</a:t></a:r><a:br/><a:r><a:t>line</a:t></a:r></a:p></p:txBody></p:sp>
                <p:sp><p:nvSpPr><p:cNvPr id="9" name="Footer"/><p:cNvSpPr/><p:nvPr><p:ph type="ftr" idx="11"/></p:nvPr></p:nvSpPr>
                  <p:txBody><a:p><a:r><a:t>Presentation title</a:t></a:r></a:p></p:txBody></p:sp>
                <p:sp><p:nvSpPr><p:cNvPr id="10" name="Nr"/><p:nvPr><p:ph type="sldNum" idx="12"/></p:nvPr></p:nvSpPr>
                  <p:txBody><a:p><a:fld type="slidenum"><a:t>2</a:t></a:fld></a:p></p:txBody></p:sp>
                <p:pic><p:nvPicPr><p:cNvPr id="3" name="Bild 2" descr="Umsatz je Quartal"/></p:nvPicPr>
                  <p:blipFill><a:blip r:embed="rId2"/></p:blipFill></p:pic>
                <mc:AlternateContent><mc:Choice Requires="a14"><p:pic><p:nvPicPr><p:cNvPr id="4" name="x"/></p:nvPicPr>
                  <p:blipFill><a:blip r:embed="rId3"/></p:blipFill></p:pic></mc:Choice>
                  <mc:Fallback><p:pic><p:nvPicPr><p:cNvPr id="4" name="x"/></p:nvPicPr>
                  <p:blipFill><a:blip r:embed="rId4"/></p:blipFill></p:pic></mc:Fallback></mc:AlternateContent>
              </p:spTree></p:cSld></p:sld>"#;
        let items = PptxConverter::parse_slide(xml).unwrap();
        assert_eq!(
            items,
            vec![
                SlideItem::Text("Tom & Jerry\nSecond\nline".to_string()),
                SlideItem::Picture {
                    rel_id: "rId2".to_string(),
                    alt: Some("Umsatz je Quartal".to_string()),
                },
                SlideItem::Picture {
                    rel_id: "rId3".to_string(),
                    alt: None,
                },
            ]
        );
    }

    #[test]
    fn chart_data_becomes_a_table() {
        let xml = r#"<c:chartSpace xmlns:c="c" xmlns:a="a"><c:chart>
            <c:title><c:tx><c:rich><a:p><a:r><a:t>Umsatz 2026</a:t></a:r></a:p></c:rich></c:tx></c:title>
            <c:plotArea><c:barChart>
              <c:ser><c:tx><c:strRef><c:strCache><c:pt idx="0"><c:v>Nord</c:v></c:pt></c:strCache></c:strRef></c:tx>
                <c:cat><c:strRef><c:strCache><c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="1"><c:v>Q2</c:v></c:pt></c:strCache></c:strRef></c:cat>
                <c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>7</c:v></c:pt><c:pt idx="1"><c:v>11</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>
              <c:ser><c:tx><c:v>Süd</c:v></c:tx>
                <c:val><c:numRef><c:numCache><c:pt idx="1"><c:v>4</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>
            </c:barChart><c:valAx><c:title><c:tx><c:rich><a:p><a:r><a:t>Mio €</a:t></a:r></a:p></c:rich></c:tx></c:title></c:valAx>
            </c:plotArea></c:chart></c:chartSpace>"#;
        let blocks = PptxConverter::parse_chart(xml);
        assert_eq!(blocks.len(), 2);
        assert!(matches!(&blocks[0], ContentBlock::Text(t) if t == "Chart (bar): Umsatz 2026"));
        match &blocks[1] {
            ContentBlock::Table { headers, rows } => {
                assert_eq!(
                    headers,
                    &vec!["".to_string(), "Nord".to_string(), "Süd".to_string()]
                );
                assert_eq!(
                    rows,
                    &vec![
                        vec!["Q1".to_string(), "7".to_string(), "".to_string()],
                        vec!["Q2".to_string(), "11".to_string(), "4".to_string()],
                    ]
                );
            }
            other => panic!("expected a table, got {:?}", other),
        }
    }
}
