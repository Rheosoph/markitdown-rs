//! Figures on PDF pages, for `extract_images`.
//!
//! A PDF has two kinds of figures, and both count:
//! - **Embedded images** (photos, screenshots, scanned charts) are taken from the file at their
//!   own resolution. A JPEG that needs no conversion is passed through unchanged; other images
//!   become PNG, or JPEG when they look like a photo.
//! - **Vector drawings** (charts exported from Office, diagrams) have no image to take. The page
//!   is rendered and the region of the drawing, with the labels around it, is cut out.
//!
//! Left out: anything covering most of the page (a scan or a background; the page text or a
//! transcription covers it), small images and drawings (logos, icons, bullets), images and
//! shapes that repeat at the same place on many pages (letterheads, slide templates) and
//! drawings made only of rules and grey or pastel fills (tables, underlines, boxes).
//!
//! Two passes: the first reads every page without decoding anything and finds what repeats,
//! the second decodes and cuts out only on pages that still have a figure.
//!
//! Coordinates are page points with the origin at the top left, as hayro renders at scale 1.

use std::collections::{HashMap, HashSet};

use bytes::Bytes;
use hayro::hayro_interpret::font::GlyphRun;
use hayro::hayro_interpret::{
    interpret_page, BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageData,
    ImageDrawProps, InterpreterCache, InterpreterSettings, Paint, RasterImage, SoftMask,
    TransformExt,
};
use hayro::hayro_syntax::object::dict::keys::{
    COLORSPACE, DECODE, DEVICE_GRAY, DEVICE_RGB, ICC_BASED, IMAGE_MASK, MASK, N, SMASK,
};
use hayro::hayro_syntax::object::{Array, Name, Stream};
use hayro::hayro_syntax::page::Page as PdfPage;
use hayro::hayro_syntax::{Filter, Pdf};
use hayro::kurbo::{Affine, BezPath, PathEl, Rect, Shape};
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::vello_cpu::peniko::ImageAlphaType;
use hayro::{render, PixmapSettings, RenderCache, RenderSettings};

use crate::model::ExtractedImage;

/// A figure is at least this big on both sides (points; 72 points = 1 inch)
const MIN_SIDE: f64 = 36.0;
/// … and covers at least this share of the page
const MIN_PAGE_SHARE: f64 = 0.01;
/// Anything covering more of the page than this is a scan or a background
const MAX_PAGE_SHARE: f64 = 0.85;
/// An embedded image needs at least this many pixels on each side
const MIN_PIXELS: u32 = 32;
/// The same embedded image on this many pages is decoration (a letterhead or logo)
const REPEATED_ON_PAGES: usize = 3;
/// A vector drawing needs this much graphic weight: each curved or slanted segment counts one
/// (up to [`MAX_SHAPE_WEIGHT`] per shape), each fill in a chart colour one. A line chart's polyline
/// alone reaches it, a single arrow does not.
const MIN_GRAPHIC_WEIGHT: usize = 3;
/// Most weight one shape adds
const MAX_SHAPE_WEIGHT: usize = 5;
/// A filled shape thinner than this is a rule, whatever its colour
const MIN_FILL_THICKNESS: f64 = 1.5;
/// A drawing of rectangles only, with more of its area covered by text than this, is a table
const MAX_TABLE_TEXT_SHARE: f64 = 0.12;
/// A shape whose area is covered by text inside it more than this is a text box (a call-out,
/// a highlight, a coloured table cell): it is no reason for a figure
const MIN_CONTAINER_TEXT_SHARE: f64 = 0.2;
/// Shapes this close together belong to the same drawing
const SHAPE_GAP: f64 = 8.0;
/// Text this close to a drawing (axis labels, captions) is part of it: at least this far …
const LABEL_REACH: f64 = 10.0;
/// … or this many times the height of the text, whichever is more …
const LABEL_REACH_LINES: f64 = 1.2;
/// … but never further than this (a big headline above a chart is not its label)
const MAX_LABEL_REACH: f64 = 24.0;
/// Small coloured shapes (legend swatches) this far from a drawing, as a share of its size,
/// are part of it
const LEGEND_REACH_SHARE: f64 = 0.1;
/// Space added around a cut-out drawing
const CROP_MARGIN: f64 = 4.0;
/// Pages with drawings are rendered at this scale (2 = 144 dpi) …
const RENDER_SCALE: f64 = 2.0;
/// … but never wider or higher than this many pixels
const MAX_RENDER_EDGE: f64 = 4096.0;
/// Longest edge of an embedded image after decoding; larger ones are scaled down
const MAX_IMAGE_EDGE: u32 = 2048;
/// A colour counts as a chart colour when its channels differ by at least this much
const MIN_SATURATION: u8 = 40;

/// An embedded image on a page
struct RasterHit {
    bbox: Rect,
    key: u128,
    /// Encoded image and its MIME type, decoded only when the image is big enough
    encoded: Option<(Vec<u8>, &'static str, u32, u32)>,
}

/// Where a shape sits, to the nearest 2 points: the same shape at the same place on many pages
/// is part of the template
type Fingerprint = (i32, i32, i32, i32);

/// A vector shape on a page
#[derive(Clone, Copy)]
struct ShapeHit {
    bbox: Rect,
    /// Curves, slanted lines or a saturated fill: what charts and diagrams are made of.
    /// 0 for rules, frames and plain fills.
    weight: usize,
    /// Part of the weight from curves and slanted lines (not from rectangles)
    curvy: usize,
    /// A rectangle or rounded rectangle: straight sides, at most rounded corners, no slants
    boxy: bool,
}

/// A figure found on a page, before it becomes an [`ExtractedImage`]
pub(crate) struct Figure {
    pub bbox: Rect,
    pub data: Vec<u8>,
    pub mime_type: &'static str,
    pub width: u32,
    pub height: u32,
}

/// Collects what a page draws: images, shapes and the extent of text
struct Collector<'k> {
    page: Rect,
    rasters: Vec<RasterHit>,
    shapes: Vec<ShapeHit>,
    text: Vec<Rect>,
    /// Embedded images to decode (second pass); `None` decodes nothing
    decode: Option<&'k HashSet<u128>>,
}

impl<'k> Collector<'k> {
    fn new(page: Rect, decode: Option<&'k HashSet<u128>>) -> Self {
        Self {
            page,
            rasters: Vec::new(),
            shapes: Vec::new(),
            text: Vec::new(),
            decode,
        }
    }

    fn page_area(&self) -> f64 {
        self.page.area().max(1.0)
    }

    /// Big enough to be a figure, small enough not to be the whole page
    fn figure_sized(&self, bbox: Rect) -> bool {
        let share = bbox.area() / self.page_area();
        bbox.width() >= MIN_SIDE
            && bbox.height() >= MIN_SIDE
            && (MIN_PAGE_SHARE..=MAX_PAGE_SHARE).contains(&share)
    }
}

fn saturated(paint: &Paint) -> bool {
    match paint {
        Paint::Color(color) => {
            let [r, g, b, a] = color.to_rgba().to_rgba8();
            let max = r.max(g).max(b);
            let min = r.min(g).min(b);
            a > 0 && max - min >= MIN_SATURATION
        }
        // Gradients and patterns are drawn on purpose
        Paint::Pattern(_) => true,
    }
}

/// The segments of a shape: curves, slanted lines and straight (horizontal or vertical) lines
#[derive(Default)]
struct Segments {
    curves: usize,
    slanted: usize,
    straight: usize,
}

impl Segments {
    fn of(path: &BezPath, transform: Affine) -> Self {
        let mut segments = Segments::default();
        let mut last = None;
        for element in path.elements() {
            match *element {
                PathEl::MoveTo(p) => last = Some(transform * p),
                PathEl::LineTo(p) => {
                    let p = transform * p;
                    if let Some(from) = last {
                        let (dx, dy) = ((p.x - from.x).abs(), (p.y - from.y).abs());
                        if dx > 0.5 && dy > 0.5 {
                            segments.slanted += 1;
                        } else {
                            segments.straight += 1;
                        }
                    }
                    last = Some(p);
                }
                PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => {
                    segments.curves += 1;
                    last = Some(transform * p);
                }
                PathEl::ClosePath => {}
            }
            // Enough to tell: a chart line or a free-hand stroke
            if segments.curves + segments.slanted > 64 {
                break;
            }
        }
        segments
    }

    /// Curved and slanted segments (pies, line charts, arrows, icons), up to [`MAX_SHAPE_WEIGHT`]
    fn weight(&self) -> usize {
        (self.curves + self.slanted).min(MAX_SHAPE_WEIGHT)
    }

    /// A rectangle or a rounded rectangle (rounded corners need up to two curves each)
    fn boxy(&self) -> bool {
        self.slanted == 0 && self.curves <= 8 && self.straight >= 2
    }
}

fn fingerprint(bbox: Rect) -> Fingerprint {
    let round = |v: f64| (v / 2.0).round() as i32;
    (
        round(bbox.x0),
        round(bbox.y0),
        round(bbox.x1),
        round(bbox.y1),
    )
}

impl<'a> Device<'a> for Collector<'_> {
    fn draw_path(&mut self, path: &BezPath, props: DrawProps<'a>, draw_mode: &DrawMode) {
        let bbox = props.transform.transform_rect_bbox(path.bounding_box());
        let bbox = bbox.intersect(self.page);
        if bbox.area() <= 0.0 && bbox.width().max(bbox.height()) <= 0.0 {
            return;
        }
        // Page frames and backgrounds say nothing about figures
        if bbox.area() / self.page_area() > MAX_PAGE_SHARE {
            return;
        }
        let filled = matches!(draw_mode, DrawMode::Fill(_));
        // A thin filled rectangle is a rule (table borders are often drawn that way)
        let rule = bbox.width().min(bbox.height()) < MIN_FILL_THICKNESS;
        let segments = Segments::of(path, props.transform);
        let curvy = segments.weight();
        let weight =
            (curvy + usize::from(filled && !rule && saturated(&props.paint))).min(MAX_SHAPE_WEIGHT);
        self.shapes.push(ShapeHit {
            bbox,
            weight,
            curvy,
            boxy: segments.boxy(),
        });
    }

    fn push_clip_path(&mut self, _: &ClipPath) {}

    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
        // Glyph outlines use 1000 units per em; this box roughly covers one glyph
        let em = Rect::new(0.0, -200.0, 600.0, 800.0);
        let mut bbox: Option<Rect> = None;
        for glyph in run.glyphs() {
            let glyph_box = (props.transform * glyph.transform()).transform_rect_bbox(em);
            bbox = Some(bbox.map_or(glyph_box, |b| b.union(glyph_box)));
        }
        if let Some(bbox) = bbox {
            self.text.push(bbox.intersect(self.page));
        }
    }

    fn draw_image(&mut self, image: Image<'a, '_>, props: ImageDrawProps<'a>) {
        // Stencil masks are icons or scanned text in one colour, not figures
        let Image::Raster(raster) = image else {
            return;
        };
        let (width, height) = (raster.width(), raster.height());
        let bbox = props
            .transform
            .transform_rect_bbox(Rect::new(0.0, 0.0, width as f64, height as f64))
            .intersect(self.page);
        // By content, not by object: some files store the same picture once per page
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            (width, height).hash(&mut hasher);
            raster.stream().raw_data().hash(&mut hasher);
            hasher.finish() as u128
        };
        let wanted = self.decode.is_some_and(|keys| keys.contains(&key))
            && self.figure_sized(bbox)
            && width >= MIN_PIXELS
            && height >= MIN_PIXELS;
        let encoded = wanted.then(|| encode_raster(&raster)).flatten();
        self.rasters.push(RasterHit { bbox, key, encoded });
    }

    fn pop_clip(&mut self) {}

    fn pop_transparency_group(&mut self) {}
}

/// The original JPEG when it can be shown as it is: one DCT filter, RGB or grey (plain or with
/// an ICC profile), no mask, no decode array (CMYK and inverted JPEGs are converted instead)
fn passthrough_jpeg(raster: &RasterImage) -> Option<Vec<u8>> {
    let stream = raster.stream();
    let filters = stream.filters();
    if filters.len() != 1 || filters[0] != Filter::DctDecode {
        return None;
    }
    let dict = stream.dict();
    if dict.contains_key(SMASK)
        || dict.contains_key(MASK)
        || dict.contains_key(DECODE)
        || dict.get::<bool>(IMAGE_MASK).unwrap_or(false)
    {
        return None;
    }
    let rgb_or_grey = match dict.get::<Name>(COLORSPACE) {
        Some(name) => &*name == DEVICE_RGB || &*name == DEVICE_GRAY,
        None => dict.get::<Array>(COLORSPACE).is_some_and(|array| {
            let mut items = array.flex_iter();
            items.next::<Name>().is_some_and(|name| &*name == ICC_BASED)
                && items
                    .next::<Stream>()
                    .and_then(|profile| profile.dict().get::<i32>(N))
                    .is_some_and(|channels| channels == 1 || channels == 3)
        }),
    };
    if !rgb_or_grey {
        return None;
    }
    let data = stream.raw_data();
    data.starts_with(&[0xFF, 0xD8, 0xFF]).then(|| data.to_vec())
}

/// Decode an embedded image and encode it for display. `None` if it cannot be decoded.
fn encode_raster(raster: &RasterImage) -> Option<(Vec<u8>, &'static str, u32, u32)> {
    if let Some(jpeg) = passthrough_jpeg(raster) {
        let (width, height) = image::ImageReader::with_format(
            std::io::Cursor::new(&jpeg[..]),
            image::ImageFormat::Jpeg,
        )
        .into_dimensions()
        .ok()?;
        return Some((jpeg, "image/jpeg", width, height));
    }

    // Ask the decoder for no more than needed: JPEGs can decode at a reduced size directly
    let (full_width, full_height) = (raster.width(), raster.height());
    let longest = full_width.max(full_height).max(1);
    let hint = (longest > MAX_IMAGE_EDGE).then(|| {
        let scale = MAX_IMAGE_EDGE as f64 / longest as f64;
        (
            ((full_width as f64 * scale).ceil() as u32).max(1),
            ((full_height as f64 * scale).ceil() as u32).max(1),
        )
    });

    // Decode into the image's own format: RGB stays RGB, grey stays grey, an alpha channel only
    // when the image has one (each full-size copy of a large photo costs tens of megabytes)
    let mut decoded: Option<image::DynamicImage> = None;
    raster.with_rgba(
        |data, alpha| {
            let (width, height) = (data.width(), data.height());
            let alpha = alpha.filter(|a| {
                a.width == width && a.height == height && a.data.iter().any(|v| *v != 255)
            });
            decoded = match (data, alpha) {
                (ImageData::Rgb(rgb), None) => image::RgbImage::from_raw(width, height, rgb.data)
                    .map(image::DynamicImage::ImageRgb8),
                (ImageData::Luma(luma), None) => {
                    image::GrayImage::from_raw(width, height, luma.data)
                        .map(image::DynamicImage::ImageLuma8)
                }
                (ImageData::Rgb(rgb), Some(alpha)) => {
                    let rgba = rgb
                        .data
                        .chunks_exact(3)
                        .zip(alpha.data.iter())
                        .flat_map(|(p, a)| [p[0], p[1], p[2], *a])
                        .collect();
                    image::RgbaImage::from_raw(width, height, rgba)
                        .map(image::DynamicImage::ImageRgba8)
                }
                (ImageData::Luma(luma), Some(alpha)) => {
                    let la = luma
                        .data
                        .iter()
                        .zip(alpha.data.iter())
                        .flat_map(|(v, a)| [*v, *a])
                        .collect();
                    image::GrayAlphaImage::from_raw(width, height, la)
                        .map(image::DynamicImage::ImageLumaA8)
                }
            };
        },
        hint,
    );
    let mut picture = decoded?;

    if picture.width().max(picture.height()) > MAX_IMAGE_EDGE {
        picture = picture.resize(
            MAX_IMAGE_EDGE,
            MAX_IMAGE_EDGE,
            image::imageops::FilterType::Triangle,
        );
    }
    let (width, height) = (picture.width(), picture.height());

    let mut out = Vec::new();
    let cursor = &mut std::io::Cursor::new(&mut out);
    if !picture.color().has_alpha() && looks_like_photo(&picture) {
        image::codecs::jpeg::JpegEncoder::new_with_quality(cursor, 85)
            .encode_image(&picture)
            .ok()?;
        Some((out, "image/jpeg", width, height))
    } else {
        picture.write_to(cursor, image::ImageFormat::Png).ok()?;
        Some((out, "image/png", width, height))
    }
}

/// Many distinct colours: a photo (JPEG is small); few: a chart or screenshot (PNG stays sharp).
/// Sampled, at most about 20 000 pixels.
fn looks_like_photo(picture: &image::DynamicImage) -> bool {
    let pixels = (picture.width() as usize * picture.height() as usize).max(1);
    let step = (pixels / 20_000).max(1);
    let mut colors = std::collections::HashSet::new();
    match picture {
        image::DynamicImage::ImageRgb8(rgb) => {
            for pixel in rgb.pixels().step_by(step) {
                colors.insert([pixel[0] >> 2, pixel[1] >> 2, pixel[2] >> 2]);
                if colors.len() > 1_000 {
                    return true;
                }
            }
            false
        }
        // Grey photos have smooth shades: most of the 64 levels
        image::DynamicImage::ImageLuma8(grey) => {
            for pixel in grey.pixels().step_by(step) {
                colors.insert([pixel[0] >> 2, 0, 0]);
            }
            colors.len() > 40
        }
        _ => false,
    }
}

/// A group of shapes: where it is, its graphic weight and the part of it from curves
#[derive(Clone, Copy)]
struct Group {
    bbox: Rect,
    weight: usize,
    curvy: usize,
}

/// Groups of shapes that touch or nearly touch. Graphic shapes seed the groups; rules and plain
/// fills only join a group they touch.
fn drawings(shapes: &[ShapeHit]) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for shape in shapes.iter().filter(|shape| shape.weight > 0) {
        let reach = shape.bbox.inflate(SHAPE_GAP, SHAPE_GAP);
        let mut merged = Group {
            bbox: shape.bbox,
            weight: shape.weight,
            curvy: shape.curvy,
        };
        groups.retain(|group| {
            if group.bbox.overlaps(reach) || group.bbox.contains_rect(shape.bbox) {
                merged = Group {
                    bbox: merged.bbox.union(group.bbox),
                    weight: merged.weight + group.weight,
                    curvy: merged.curvy + group.curvy,
                };
                false
            } else {
                true
            }
        });
        groups.push(merged);
    }

    // Axes, gridlines and frames next to a drawing are part of it (a few rounds suffice)
    for _ in 0..4 {
        let mut grew = false;
        for group in &mut groups {
            let reach = group.bbox.inflate(SHAPE_GAP, SHAPE_GAP);
            for shape in shapes.iter().filter(|shape| shape.weight == 0) {
                if reach.overlaps(shape.bbox) && !group.bbox.contains_rect(shape.bbox) {
                    group.bbox = group.bbox.union(shape.bbox);
                    grew = true;
                }
            }
        }
        if !grew {
            break;
        }
    }
    merge_overlapping(groups)
}

fn merge_overlapping(mut groups: Vec<Group>) -> Vec<Group> {
    let mut changed = true;
    while changed {
        changed = false;
        'outer: for i in 0..groups.len() {
            for j in (i + 1)..groups.len() {
                if groups[i].bbox.overlaps(groups[j].bbox) {
                    let other = groups.remove(j);
                    groups[i] = Group {
                        bbox: groups[i].bbox.union(other.bbox),
                        weight: groups[i].weight + other.weight,
                        curvy: groups[i].curvy + other.curvy,
                    };
                    changed = true;
                    break 'outer;
                }
            }
        }
    }
    groups
}

/// Share of a region covered by text
fn text_share(region: Rect, text: &[Rect]) -> f64 {
    let covered: f64 = text.iter().map(|t| t.intersect(region).area()).sum();
    covered / region.area().max(1.0)
}

/// Share of `inner` that lies inside `outer`
fn covered(inner: Rect, outer: Rect) -> f64 {
    let area = inner.area();
    if area <= 0.0 {
        return 0.0;
    }
    inner.intersect(outer).area() / area
}

/// Cut regions out of a rendering of the page, as PNG
fn crop_drawings(page: &PdfPage, regions: &[Rect]) -> Vec<Option<(Vec<u8>, u32, u32)>> {
    let (width, height) = page.render_dimensions();
    let longest = (width.max(height) as f64).max(1.0);
    let scale = RENDER_SCALE.min(MAX_RENDER_EDGE / longest) as f32;
    let pixmap = render(
        page,
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &PixmapSettings {
            x_scale: scale,
            y_scale: scale,
            bg_color: WHITE,
        },
    );
    let (pixmap_width, pixmap_height) = (pixmap.width() as u32, pixmap.height() as u32);
    let Some(rendered) = image::RgbaImage::from_raw(
        pixmap_width,
        pixmap_height,
        pixmap.take_rgba8(ImageAlphaType::Alpha),
    ) else {
        return regions.iter().map(|_| None).collect();
    };

    regions
        .iter()
        .map(|region| {
            let s = scale as f64;
            let x0 = (region.x0 * s).floor().max(0.0) as u32;
            let y0 = (region.y0 * s).floor().max(0.0) as u32;
            let x1 = ((region.x1 * s).ceil() as u32).min(pixmap_width);
            let y1 = ((region.y1 * s).ceil() as u32).min(pixmap_height);
            if x1 <= x0 || y1 <= y0 {
                return None;
            }
            let crop = image::imageops::crop_imm(&rendered, x0, y0, x1 - x0, y1 - y0).to_image();
            let rgb = image::DynamicImage::ImageRgba8(crop).to_rgb8();
            let mut png = Vec::new();
            rgb.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .ok()?;
            Some((png, x1 - x0, y1 - y0))
        })
        .collect()
}

/// Read a page: what it draws, decoding only the given embedded images
fn collect<'a, 'k>(
    page: &'a PdfPage<'a>,
    cache: &InterpreterCache<'a>,
    decode: Option<&'k HashSet<u128>>,
) -> Collector<'k> {
    let (width, height) = page.render_dimensions();
    let page_rect = Rect::new(0.0, 0.0, width as f64, height as f64);
    let mut collector = Collector::new(page_rect, decode);
    let mut context = Context::new(
        page.initial_transform(true).to_kurbo(),
        page_rect,
        cache,
        page.xref(),
        InterpreterSettings::default(),
    );
    interpret_page(page, &mut context, &mut collector);
    collector
}

/// A drawing with what belongs around it: legend swatches (small coloured groups nearby), then
/// labels, legend text and captions (text within [`LABEL_REACH_LINES`] of its own height, in
/// two rounds so a legend's text follows its swatches)
fn with_surroundings(group: &Group, minor: &[Group], text: &[Rect]) -> Rect {
    let reach = (group.bbox.width().max(group.bbox.height()) * LEGEND_REACH_SHARE).max(SHAPE_GAP);
    let near = group.bbox.inflate(reach, reach);
    let mut bbox = minor
        .iter()
        .filter(|small| near.overlaps(small.bbox))
        .fold(group.bbox, |bbox, small| bbox.union(small.bbox));
    for _ in 0..2 {
        bbox = text
            .iter()
            .filter(|t| {
                let reach = (t.height() * LABEL_REACH_LINES).clamp(LABEL_REACH, MAX_LABEL_REACH);
                bbox.inflate(reach, reach).overlaps(**t)
            })
            .fold(bbox, |bbox, t| bbox.union(*t));
    }
    bbox
}

/// A frame or background for text: text covers a good part of the shape, or the shape is a
/// box (rectangle, rounded rectangle) with text inside
fn holds_text(shape: &ShapeHit, text: &[Rect]) -> bool {
    let area = shape.bbox.area();
    if area <= 0.0 {
        return false;
    }
    if shape.boxy && text.iter().any(|t| shape.bbox.contains_rect(*t)) {
        return true;
    }
    let inside: f64 = text
        .iter()
        .map(|t| t.intersect(shape.bbox).area())
        .filter(|&a| a > 0.0)
        .sum();
    inside / area > MIN_CONTAINER_TEXT_SHARE
}

/// Regions of vector drawings on a page, leaving out template shapes, text boxes and tables
fn drawing_regions(collector: &Collector, template: &HashSet<Fingerprint>) -> Vec<Rect> {
    let shapes: Vec<ShapeHit> = collector
        .shapes
        .iter()
        .filter(|shape| !template.contains(&fingerprint(shape.bbox)))
        .map(|shape| {
            let mut shape = *shape;
            // Frames and backgrounds of text still belong to a drawing, but do not make one
            if shape.weight > 0 && holds_text(&shape, &collector.text) {
                shape.weight = 0;
                shape.curvy = 0;
            }
            shape
        })
        .collect();
    let (major, minor): (Vec<Group>, Vec<Group>) = drawings(&shapes)
        .into_iter()
        .partition(|group| group.weight >= MIN_GRAPHIC_WEIGHT);
    let regions: Vec<Group> = major
        .into_iter()
        .filter_map(|group| {
            let bbox = with_surroundings(&group, &minor, &collector.text);
            let text = text_share(bbox, &collector.text);
            let table = group.curvy == 0 && text > MAX_TABLE_TEXT_SHARE;
            (!table && collector.figure_sized(bbox)).then(|| Group {
                bbox: bbox
                    .inflate(CROP_MARGIN, CROP_MARGIN)
                    .intersect(collector.page),
                ..group
            })
        })
        .collect();
    merge_overlapping(regions)
        .into_iter()
        .map(|group| group.bbox)
        .collect()
}

/// Top to bottom in rows, left to right within a row: a figure whose top is above the middle of
/// the row's first figure sits next to it
fn reading_order(mut figures: Vec<Figure>) -> Vec<Figure> {
    let by = |a: f64, b: f64| a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal);
    figures.sort_by(|a, b| by(a.bbox.y0, b.bbox.y0));
    let mut ordered = Vec::with_capacity(figures.len());
    let mut row: Vec<Figure> = Vec::new();
    for figure in figures {
        if let Some(first) = row.first() {
            if figure.bbox.y0 >= first.bbox.center().y {
                row.sort_by(|a, b| by(a.bbox.x0, b.bbox.x0));
                ordered.append(&mut row);
            }
        }
        row.push(figure);
    }
    row.sort_by(|a, b| by(a.bbox.x0, b.bbox.x0));
    ordered.append(&mut row);
    ordered
}

/// Figures of every page as images, `page{n}_image{m}`, top to bottom on each page
pub(crate) fn document_figures(pdf: &Pdf) -> Vec<Vec<ExtractedImage>> {
    let cache = InterpreterCache::new();
    let pages = pdf.pages();

    // Pass 1: what each page draws, and what repeats across pages
    let first: Vec<Collector> = pages
        .iter()
        .map(|page| collect(page, &cache, None))
        .collect();
    let mut image_pages: HashMap<u128, usize> = HashMap::new();
    let mut shape_pages: HashMap<Fingerprint, usize> = HashMap::new();
    for collector in &first {
        let keys: HashSet<u128> = collector.rasters.iter().map(|hit| hit.key).collect();
        for key in keys {
            *image_pages.entry(key).or_default() += 1;
        }
        let prints: HashSet<Fingerprint> = collector
            .shapes
            .iter()
            .map(|shape| fingerprint(shape.bbox))
            .collect();
        for print in prints {
            *shape_pages.entry(print).or_default() += 1;
        }
    }
    let template: HashSet<Fingerprint> = shape_pages
        .into_iter()
        .filter(|&(_, count)| count >= REPEATED_ON_PAGES)
        .map(|(print, _)| print)
        .collect();

    // Pass 2: decode and cut out, only where a figure is left
    first
        .iter()
        .zip(pages.iter())
        .enumerate()
        .map(|(index, (collector, page))| {
            let page_number = index as u32 + 1;
            let regions = drawing_regions(collector, &template);
            let wanted: HashSet<u128> = collector
                .rasters
                .iter()
                .filter(|hit| {
                    image_pages[&hit.key] < REPEATED_ON_PAGES
                        && collector.figure_sized(hit.bbox)
                        && !regions
                            .iter()
                            .any(|region| covered(hit.bbox, *region) > 0.8)
                })
                .map(|hit| hit.key)
                .collect();
            if regions.is_empty() && wanted.is_empty() {
                return Vec::new();
            }

            let mut figures: Vec<Figure> = Vec::new();
            if !wanted.is_empty() {
                let mut seen = HashSet::new();
                // A cache of its own: decoded images are released after the page
                let page_cache = InterpreterCache::new();
                for hit in collect(page, &page_cache, Some(&wanted)).rasters {
                    // The same image twice on a page counts once
                    if let (Some((data, mime_type, width, height)), true) =
                        (hit.encoded, seen.insert(hit.key))
                    {
                        figures.push(Figure {
                            bbox: hit.bbox,
                            data,
                            mime_type,
                            width,
                            height,
                        });
                    }
                }
            }
            if !regions.is_empty() {
                for (bbox, crop) in regions.iter().zip(crop_drawings(page, &regions)) {
                    if let Some((data, width, height)) = crop {
                        figures.push(Figure {
                            bbox: *bbox,
                            data,
                            mime_type: "image/png",
                            width,
                            height,
                        });
                    }
                }
            }
            reading_order(figures)
                .into_iter()
                .enumerate()
                .map(|(i, figure)| {
                    let mut image = ExtractedImage::new(
                        format!("page{}_image{}", page_number, i + 1),
                        Bytes::from(figure.data),
                        figure.mime_type,
                    );
                    image.width = Some(figure.width);
                    image.height = Some(figure.height);
                    image.page_number = Some(page_number);
                    image
                })
                .collect()
        })
        .collect()
}
