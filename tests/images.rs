//! Images across formats: every placeholder in the text has exactly one image with the same id
//! and the other way round (consumers match them by id), slides come in presentation order
//! with their own pictures, and images nothing can show are dropped with their placeholder.
use bytes::Bytes;
use markitdown::{model::ConversionOptions, ContentBlock, Document, MarkItDown};
use regex::Regex;
use std::collections::HashSet;
use std::io::{Cursor, Write};

/// Ids of the image placeholders `![id](id)` in a page's markdown
fn placeholders(markdown: &str) -> Vec<String> {
    let re = Regex::new(r"!\[([^\]]+)\]\(([^)]+)\)").unwrap();
    re.captures_iter(markdown)
        .filter(|c| c[1] == c[2])
        .map(|c| c[2].to_string())
        .collect()
}

/// Every placeholder has one image with its id on the same page, every image has one placeholder,
/// and ids are unique in the document. Returns the number of images.
fn assert_ids_match(document: &Document) -> usize {
    let mut seen = HashSet::new();
    for page in &document.pages {
        let ids: Vec<String> = page.images().iter().map(|i| i.id.clone()).collect();
        assert_eq!(
            placeholders(&page.to_markdown()),
            ids,
            "page {}: placeholders and images differ",
            page.page_number
        );
        for id in ids {
            assert!(seen.insert(id.clone()), "id {} used twice", id);
        }
    }
    seen.len()
}

async fn convert_file(path: &str, ext: &str) -> Document {
    MarkItDown::new()
        .convert(
            path,
            Some(
                ConversionOptions::default()
                    .with_extension(ext)
                    .with_images(true),
            ),
        )
        .await
        .unwrap_or_else(|e| panic!("{} failed: {:?}", path, e))
}

#[tokio::test]
async fn ids_in_the_text_match_the_images() {
    for (path, ext, at_least) in [
        (
            "tests/test_documents/presentations/pitch_deck_presentation.pptx",
            ".pptx",
            20,
        ),
        (
            "tests/test_documents/presentations/powerpoint_with_image.pptx",
            ".pptx",
            1,
        ),
        (
            "tests/test_documents/documents/word_image_anchors.docx",
            ".docx",
            1,
        ),
        (
            "tests/test_documents/documents/word_sample.docx",
            ".docx",
            1,
        ),
        (
            "tests/test_documents/pdfs/embedded_images_tables.pdf",
            ".pdf",
            2,
        ),
    ] {
        let document = convert_file(path, ext).await;
        let count = assert_ids_match(&document);
        assert!(
            count >= at_least,
            "{}: {} images, expected at least {}",
            path,
            count,
            at_least
        );
        for image in document.images() {
            assert!(
                matches!(
                    image.mime_type.as_str(),
                    "image/png" | "image/jpeg" | "image/gif" | "image/webp"
                ),
                "{}: {} has {}",
                path,
                image.id,
                image.mime_type
            );
            assert!(
                image.width.is_some() && image.height.is_some(),
                "{}: {} without size",
                path,
                image.id
            );
        }
    }
}

#[tokio::test]
async fn docx_pictures_stand_where_the_document_shows_them() {
    let document = convert_file(
        "tests/test_documents/documents/word_image_anchors.docx",
        ".docx",
    )
    .await;
    let blocks = &document.pages[0].content;
    let first_image = blocks
        .iter()
        .position(|b| matches!(b, ContentBlock::Image(_)))
        .expect("an image");
    assert!(
        first_image > 0,
        "images no longer come first, before all text"
    );
}

#[tokio::test]
async fn metafiles_disappear_with_their_placeholder() {
    let document = convert_file("tests/test_documents/documents/test_emf_docx.docx", ".docx").await;
    assert_ids_match(&document);
    assert!(
        document
            .images()
            .iter()
            .all(|i| !i.mime_type.contains("emf") && !i.mime_type.contains("wmf")),
        "no metafiles"
    );
    assert!(!document.to_markdown().contains("[Image skipped"));
}

// ------------------------------------------------------------------ PPTX order and relationships

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;

/// A one-colour PNG; each slide gets its own colour, so a picture shows which slide it came from
fn png(shade: u8) -> Vec<u8> {
    let picture = image::RgbImage::from_pixel(40, 30, image::Rgb([shade, 100, 200]));
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(picture)
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// Twelve slides stored as slide1.xml … slide12.xml, presented in reverse order (slide12 first).
/// Each slide has a title "Part N" (N = file number) and a picture media/imageN.png, and
/// slide 5 has a native chart.
fn reversed_deck() -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    let mut add = |name: &str, data: &[u8]| {
        zip.start_file(name, options).unwrap();
        zip.write_all(data).unwrap();
    };

    let ids: String = (1..=12)
        .rev()
        .map(|n| format!(r#"<p:sldId id="{}" r:id="rId{}"/>"#, 255 + n, n))
        .collect();
    add(
        "ppt/presentation.xml",
        format!(r#"<?xml version="1.0"?><p:presentation {NS}><p:sldIdLst>{ids}</p:sldIdLst></p:presentation>"#).as_bytes(),
    );
    let rels: String = (1..=12)
        .map(|n| format!(r#"<Relationship Id="rId{n}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide{n}.xml"/>"#))
        .collect();
    add(
        "ppt/_rels/presentation.xml.rels",
        format!(r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}</Relationships>"#).as_bytes(),
    );

    for n in 1..=12u8 {
        let chart = if n == 5 {
            r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="9" name="Chart"/></p:nvGraphicFramePr><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rId3"/></a:graphicData></a:graphic></p:graphicFrame>"#
        } else {
            ""
        };
        add(
            &format!("ppt/slides/slide{n}.xml"),
            format!(
                r#"<?xml version="1.0"?><p:sld {NS}><p:cSld><p:spTree>
                <p:sp><p:nvSpPr><p:cNvPr id="2" name="Title"/></p:nvSpPr><p:txBody><a:p><a:r><a:t>Part {n}</a:t></a:r></a:p></p:txBody></p:sp>
                <p:pic><p:nvPicPr><p:cNvPr id="3" name="Picture" descr="Picture of part {n}"/></p:nvPicPr><p:blipFill><a:blip r:embed="rId2"/></p:blipFill></p:pic>
                {chart}</p:spTree></p:cSld></p:sld>"#
            )
            .as_bytes(),
        );
        add(
            &format!("ppt/slides/_rels/slide{n}.xml.rels"),
            format!(
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
                <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image{n}.png"/>
                <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/></Relationships>"#
            )
            .as_bytes(),
        );
        add(&format!("ppt/media/image{n}.png"), &png(n * 20));
    }
    add(
        "ppt/charts/chart1.xml",
        br#"<?xml version="1.0"?><c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart>
        <c:title><c:tx><c:rich><a:p><a:r><a:t>Revenue</a:t></a:r></a:p></c:rich></c:tx></c:title>
        <c:plotArea><c:lineChart><c:ser><c:tx><c:v>2026</c:v></c:tx>
        <c:cat><c:strRef><c:strCache><c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="1"><c:v>Q2</c:v></c:pt></c:strCache></c:strRef></c:cat>
        <c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>7</c:v></c:pt><c:pt idx="1"><c:v>11</c:v></c:pt></c:numCache></c:numRef></c:val>
        </c:ser></c:lineChart></c:plotArea></c:chart></c:chartSpace>"#,
    );
    zip.finish().unwrap().into_inner()
}

#[tokio::test]
async fn slides_follow_the_presentation_order_with_their_own_pictures() {
    let document = MarkItDown::new()
        .convert_bytes(
            Bytes::from(reversed_deck()),
            Some(
                ConversionOptions::default()
                    .with_extension(".pptx")
                    .with_images(true),
            ),
        )
        .await
        .unwrap();
    assert_eq!(document.pages.len(), 12);
    assert_ids_match(&document);

    for (index, page) in document.pages.iter().enumerate() {
        let part = 12 - index; // presented in reverse
        assert_eq!(page.page_number as usize, index + 1);
        assert!(
            page.to_markdown().contains(&format!("Part {}\n", part)),
            "page {} should be part {}: {}",
            index + 1,
            part,
            page.to_markdown()
        );
        let images = page.images();
        assert_eq!(images.len(), 1, "page {}", index + 1);
        let image = images[0];
        assert_eq!(image.id, format!("slide{}_image1", index + 1));
        assert_eq!(
            image.alt_text.as_deref(),
            Some(format!("Picture of part {}", part).as_str())
        );
        let decoded = image::load_from_memory(&image.data).unwrap().to_rgb8();
        assert_eq!(
            decoded.get_pixel(0, 0)[0],
            (part * 20) as u8,
            "page {} shows the picture of part {}",
            index + 1,
            part
        );
    }

    // Part 5 is presented eighth: its chart is a table there
    let chart_page = &document.pages[7];
    let markdown = chart_page.to_markdown();
    assert!(markdown.contains("Chart (line): Revenue"), "{}", markdown);
    assert!(markdown.contains("| Q2 | 11 |"), "{}", markdown);
}

#[tokio::test]
async fn html_images_stand_where_the_page_shows_them() {
    use base64::prelude::*;
    let data = BASE64_STANDARD.encode(png(90));
    let html = format!(
        r#"<html><body><p>Before the chart.</p><img alt="Revenue chart" src="data:image/png;base64,{data}"><p>After the chart.</p><img src="https://example.org/logo.png" alt="Logo"></body></html>"#
    );
    let document = MarkItDown::new()
        .convert_bytes(
            Bytes::from(html),
            Some(
                ConversionOptions::default()
                    .with_extension(".html")
                    .with_images(true),
            ),
        )
        .await
        .unwrap();
    assert_ids_match(&document);
    let blocks = &document.pages[0].content;
    let kinds: Vec<&str> = blocks
        .iter()
        .map(|b| match b {
            ContentBlock::Image(_) => "image",
            _ => "text",
        })
        .collect();
    assert_eq!(
        kinds,
        vec!["text", "image", "text"],
        "text, the chart, text"
    );
    let markdown = document.to_markdown();
    assert!(!markdown.contains("base64"), "no base64 in the text");
    assert!(markdown.contains("Before the chart.") && markdown.contains("After the chart."));
    assert!(
        markdown.contains("https://example.org/logo.png"),
        "linked images stay links"
    );
    assert_eq!(
        document.images()[0].alt_text.as_deref(),
        Some("Revenue chart")
    );
}
