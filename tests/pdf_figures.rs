//! PDF figures with `extract_images`: embedded images and cut-outs of vector drawings.
//!
//! The test document is written by hand below, so every figure on it is known:
//! - page 1: a photo, a vector bar chart with axis labels, a table drawn with rules, a small
//!   logo and a letterhead image
//! - page 2: the letterhead, a JPEG (passed through unchanged), a chart image with few
//!   colours (PNG) and a line chart drawn as one polyline
//! - page 3: the letterhead and text only
//! - page 4: a full-page background image and text
use bytes::Bytes;
use markitdown::{model::ConversionOptions, ContentBlock, Document, MarkItDown, MockLlmClient};
use std::sync::Arc;

/// Minimal PDF writer: numbered objects, cross-reference table from the byte offsets
struct PdfWriter {
    objects: Vec<Vec<u8>>,
}

impl PdfWriter {
    fn new() -> Self {
        Self {
            objects: Vec::new(),
        }
    }

    /// Reserve an object number, fill it later with `set`
    fn reserve(&mut self) -> usize {
        self.objects.push(Vec::new());
        self.objects.len()
    }

    fn set(&mut self, number: usize, body: impl Into<Vec<u8>>) {
        self.objects[number - 1] = body.into();
    }

    fn add(&mut self, body: impl Into<Vec<u8>>) -> usize {
        let number = self.reserve();
        self.set(number, body);
        number
    }

    fn stream(&mut self, dict: &str, data: &[u8]) -> usize {
        let mut body = format!("<< {} /Length {} >>\nstream\n", dict, data.len()).into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(b"\nendstream");
        self.add(body)
    }

    fn finish(self, root: usize) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in self.objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", self.objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            out.extend_from_slice(format!("{:010} 00000 n \n", offset).as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root {} 0 R >>\nstartxref\n{}\n%%EOF\n",
                self.objects.len() + 1,
                root,
                xref
            )
            .as_bytes(),
        );
        out
    }
}

/// Raw 8-bit RGB pixels as an image XObject
fn rgb_image(
    pdf: &mut PdfWriter,
    width: u32,
    height: u32,
    pixel: impl Fn(u32, u32) -> [u8; 3],
) -> usize {
    let mut data = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            data.extend_from_slice(&pixel(x, y));
        }
    }
    pdf.stream(
        &format!(
            "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8",
            width, height
        ),
        &data,
    )
}

/// A JPEG made with the image crate, the bytes the PDF embeds
fn jpeg_bytes() -> Vec<u8> {
    let picture = image::RgbImage::from_fn(64, 48, |x, y| {
        image::Rgb([(x * 4) as u8, (y * 5) as u8, 128])
    });
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(picture)
        .write_to(
            &mut std::io::Cursor::new(&mut out),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    out
}

fn text(x: u32, y: u32, size: u32, content: &str) -> String {
    format!("BT /F1 {} Tf {} {} Td ({}) Tj ET\n", size, x, y, content)
}

fn test_pdf() -> Vec<u8> {
    let mut pdf = PdfWriter::new();
    let catalog = pdf.reserve();
    let pages = pdf.reserve();
    let font = pdf.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    // Photo: grain in every pixel, thousands of colours
    let photo = rgb_image(&mut pdf, 160, 120, |x, y| {
        let grain =
            (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663)).wrapping_mul(2_654_435_761);
        [(grain >> 24) as u8, (grain >> 16) as u8, (grain >> 8) as u8]
    });
    let letterhead = rgb_image(&mut pdf, 100, 30, |x, _| {
        if x < 50 {
            [20, 60, 140]
        } else {
            [240, 240, 240]
        }
    });
    let logo = rgb_image(&mut pdf, 40, 20, |_, _| [200, 30, 30]);
    let jpeg = jpeg_bytes();
    let jpeg_image = pdf.stream(
        "/Type /XObject /Subtype /Image /Width 64 /Height 48 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode",
        &jpeg,
    );
    // A chart saved as an image: four flat colours
    let chart_image = rgb_image(&mut pdf, 120, 90, |x, y| match (x / 30, y > 45) {
        (_, false) => [255, 255, 255],
        (0, true) => [233, 100, 63],
        (1, true) => [47, 111, 222],
        _ => [21, 128, 61],
    });
    let background = rgb_image(&mut pdf, 60, 80, |x, y| [230, 230, (100 + x + y) as u8]);

    let resources = format!(
        "/Resources << /Font << /F1 {} 0 R >> /XObject << /Ph {} 0 R /Lh {} 0 R /Lg {} 0 R /Jp {} 0 R /Ci {} 0 R /Bg {} 0 R >> >>",
        font, photo, letterhead, logo, jpeg_image, chart_image, background
    );

    let letterhead_draw = "q 200 0 0 60 340 720 cm /Lh Do Q\n";
    let page1 = [
        text(72, 740, 18, "Quarterly report"),
        letterhead_draw.to_string(),
        // Photo, top left
        "q 240 0 0 180 72 480 cm /Ph Do Q\n".to_string(),
        // Bar chart: axes, four bars in chart colours, labels under the axis
        "0 G 1 w 340 480 m 340 660 l S 340 480 m 540 480 l S\n".to_string(),
        "0.91 0.39 0.25 rg 360 480 30 60 re f 410 480 30 110 re f 460 480 30 150 re f\n"
            .to_string(),
        "0.18 0.44 0.87 rg 505 480 25 90 re f\n".to_string(),
        text(368, 468, 9, "Q1"),
        text(418, 468, 9, "Q2"),
        text(468, 468, 9, "Q3"),
        text(510, 468, 9, "Q4"),
        // Table: grey header row and black rules only
        "0.9 g 72 300 468 20 re f 0 G 0.5 w\n".to_string(),
        "72 320 m 540 320 l S 72 300 m 540 300 l S 72 280 m 540 280 l S 72 260 m 540 260 l S\n"
            .to_string(),
        "72 260 m 72 320 l S 228 260 m 228 320 l S 384 260 m 384 320 l S 540 260 m 540 320 l S\n"
            .to_string(),
        "0 g\n".to_string(),
        text(80, 306, 10, "Region"),
        text(236, 306, 10, "Revenue"),
        text(80, 286, 10, "North"),
        text(236, 286, 10, "12"),
        // Small logo at the bottom
        "q 30 0 0 15 72 40 cm /Lg Do Q\n".to_string(),
    ]
    .concat();
    let page2 = [
        text(72, 740, 14, "Details"),
        letterhead_draw.to_string(),
        "q 200 0 0 150 72 450 cm /Jp Do Q\n".to_string(),
        "q 160 0 0 120 340 480 cm /Ci Do Q\n".to_string(),
        // Line chart: two axes and one polyline
        "0 G 1 w 72 120 m 72 300 l S 72 120 m 400 120 l S\n".to_string(),
        "0.18 0.44 0.87 RG 2 w 80 150 m 140 200 l 200 170 l 260 260 l 320 230 l 390 290 l S\n"
            .to_string(),
    ]
    .concat();
    let page3 = [
        text(72, 740, 14, "Notes"),
        letterhead_draw.to_string(),
        text(72, 600, 11, "Only text here."),
    ]
    .concat();
    let page4 = [
        "q 612 0 0 792 0 0 cm /Bg Do Q\n".to_string(),
        text(72, 400, 14, "Text on a background"),
    ]
    .concat();

    let mut kids = Vec::new();
    for content in [page1, page2, page3, page4] {
        let stream = pdf.stream("", content.as_bytes());
        kids.push(pdf.add(format!(
            "<< /Type /Page /Parent {} 0 R /MediaBox [0 0 612 792] {} /Contents {} 0 R >>",
            pages, resources, stream
        )));
    }
    pdf.set(
        catalog,
        format!("<< /Type /Catalog /Pages {} 0 R >>", pages),
    );
    pdf.set(
        pages,
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.iter()
                .map(|k| format!("{} 0 R", k))
                .collect::<Vec<_>>()
                .join(" "),
            kids.len()
        ),
    );
    pdf.finish(catalog)
}

async fn convert(options: ConversionOptions) -> Document {
    MarkItDown::new()
        .convert_bytes(
            Bytes::from(test_pdf()),
            Some(options.with_extension(".pdf")),
        )
        .await
        .expect("conversion failed")
}

fn images_of(document: &Document, page_number: u32) -> Vec<&markitdown::ExtractedImage> {
    document
        .pages
        .iter()
        .find(|p| p.page_number == page_number)
        .map(|p| p.images())
        .unwrap_or_default()
}

#[tokio::test]
async fn embedded_images_and_vector_charts_become_figures() {
    let document = convert(ConversionOptions::default().with_images(true)).await;

    // Page 1: the photo and the bar chart; not the table, the logo or the letterhead
    let page1 = images_of(&document, 1);
    let ids: Vec<&str> = page1.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, vec!["page1_image1", "page1_image2"], "page 1 figures");
    let (photo, chart) = (page1[0], page1[1]);
    assert_eq!(photo.mime_type, "image/jpeg", "a photo becomes JPEG");
    assert_eq!(
        (photo.width, photo.height),
        (Some(160), Some(120)),
        "at its own resolution"
    );
    assert_eq!(chart.mime_type, "image/png", "a drawing is cut out as PNG");
    // Chart from x 340 to 540 plus labels and margin, rendered at 2x: about 410 px wide
    let width = chart.width.unwrap();
    assert!((380..=460).contains(&width), "chart crop width {}", width);
    let height = chart.height.unwrap();
    assert!(
        (370..=440).contains(&height),
        "chart crop includes the axis labels, height {}",
        height
    );
    let decoded = image::load_from_memory(&chart.data).unwrap().to_rgb8();
    let orange = decoded
        .pixels()
        .filter(|p| p[0] > 200 && (80..=120).contains(&p[1]) && p[2] < 90)
        .count();
    assert!(
        orange > 1_000,
        "the crop shows the orange bars ({} pixels)",
        orange
    );

    // Page 2: the JPEG unchanged, the chart image as PNG, the line chart cut out
    let page2 = images_of(&document, 2);
    assert_eq!(
        page2.len(),
        3,
        "page 2 figures: {:?}",
        page2.iter().map(|i| &i.id).collect::<Vec<_>>()
    );
    let jpeg = page2
        .iter()
        .find(|i| i.mime_type == "image/jpeg")
        .expect("JPEG kept");
    assert_eq!(
        jpeg.data.as_ref(),
        jpeg_bytes().as_slice(),
        "JPEG passed through byte for byte"
    );
    let flat = page2
        .iter()
        .find(|i| i.width == Some(120))
        .expect("chart image");
    assert_eq!(flat.mime_type, "image/png", "few colours stay PNG");
    let line_chart = page2
        .iter()
        .find(|i| i.height.unwrap_or(0) > 300)
        .expect("line chart crop");
    assert!(line_chart.width.unwrap() > 600, "line chart spans the axes");

    // Page 3: only the letterhead, which repeats on three pages: nothing
    assert!(images_of(&document, 3).is_empty(), "letterhead left out");
    // Page 4: the background covers the page: nothing
    assert!(images_of(&document, 4).is_empty(), "background left out");

    // Figures follow the page text
    let page = document.pages.iter().find(|p| p.page_number == 1).unwrap();
    let blocks = page.content.len();
    assert!(blocks >= 2);
    assert!(matches!(page.content[blocks - 1], ContentBlock::Image(_)));
    assert!(matches!(page.content[blocks - 2], ContentBlock::Image(_)));
    assert!(
        page.content[..blocks - 2]
            .iter()
            .all(|block| !matches!(block, ContentBlock::Image(_))),
        "text first, then the figures"
    );
    let markdown = document.to_markdown();
    assert!(
        markdown.contains("![page1_image2](page1_image2)"),
        "{}",
        markdown
    );
}

#[tokio::test]
async fn without_extract_images_there_are_no_figures() {
    let document = convert(ConversionOptions::default().with_images(false)).await;
    assert!(document.images().is_empty());
}

#[tokio::test]
async fn figures_are_described_once_with_an_llm() {
    let llm = Arc::new(MockLlmClient::new()) as markitdown::SharedLlmClient;
    let document = convert(ConversionOptions::default().with_images(true).with_llm(llm)).await;
    let images = document.images();
    assert_eq!(images.len(), 5, "two on page 1, three on page 2");
    assert!(
        images.iter().all(|i| i.description.is_some()),
        "every figure has a description"
    );
}
