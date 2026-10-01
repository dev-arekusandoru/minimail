//! Bounded, offline HTML rendering for email message bodies.
//!
//! The returned buffer is RGBA8, premultiplied-alpha data. Width and height are
//! physical pixel dimensions; link rectangles and `scale` are CSS/logical values.

use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use anyrender::{PaintScene as _, render_to_buffer};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{DocumentConfig, StyleThreading, util::Color};
use blitz_html::HtmlDocument;
use blitz_paint::paint_scene;
use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};
use blitz_traits::shell::{ColorScheme, Viewport};
use data_url::DataUrl;
use image::ImageReader;
use peniko::Fill;
use peniko::kurbo::Rect;

use blitz_dom::local_name;
const MAX_HTML_BYTES: usize = 8 * 1024 * 1024;
const MAX_IMAGE_DATA_URL_BYTES: usize = 6 * 1024 * 1024;
const MAX_IMAGE_PIXELS: usize = 16 * 1024 * 1024;
const MAX_TOTAL_IMAGE_PIXELS: usize = 16 * 1024 * 1024;
const MAX_RENDER_WIDTH: u32 = 16_000;
const MAX_RENDER_HEIGHT: u32 = 16_000;
const MAX_RENDER_PIXELS: usize = 32 * 1024 * 1024;
const VIEWPORT_HEIGHT: u32 = 800;

/// A link's clickable fragment in document CSS coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct HtmlLink {
    pub href: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A full-document raster and the link fragments laid out in that document.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedHtml {
    /// Physical pixel width of `pixels`.
    pub width: u32,
    /// Physical pixel height of `pixels`.
    pub height: u32,
    /// Physical pixels per logical CSS pixel.
    pub scale: f32,
    /// Premultiplied RGBA8 pixel bytes, row-major.
    pub pixels: Vec<u8>,
    pub links: Vec<HtmlLink>,
}

/// Parse, style, lay out, and CPU-rasterize HTML without fetching network
/// resources or sender-referenced files. `width` is the CSS viewport width; overflow is
/// preserved in the returned full-document raster when it fits safety limits.
pub fn render(html: &str, width: u32, scale: f32) -> Result<RenderedHtml, String> {
    if html.len() > MAX_HTML_BYTES {
        return Err(format!("HTML document exceeds {MAX_HTML_BYTES} byte limit"));
    }
    if width == 0 || width > MAX_RENDER_WIDTH || !scale.is_finite() || !(0.5..=4.0).contains(&scale)
    {
        return Err("invalid viewport width or scale".to_owned());
    }

    let physical_width = (width as f64 * f64::from(scale)).ceil();
    let physical_height = (VIEWPORT_HEIGHT as f64 * f64::from(scale)).ceil();
    if physical_width > f64::from(u16::MAX) || physical_height > f64::from(u16::MAX) {
        return Err("viewport dimensions exceed the rasterizer limit".to_owned());
    }

    let resources = Arc::new(EmbeddedImages::default());
    let mut doc = HtmlDocument::from_html(
        html,
        DocumentConfig {
            viewport: Some(Viewport::new(
                physical_width as u32,
                physical_height as u32,
                scale,
                ColorScheme::Light,
            )),
            base_url: Some("https://mail.invalid/".to_owned()),
            net_provider: Some(resources.clone()),
            style_threading: StyleThreading::Sequential,
            ..Default::default()
        },
    )
    .into_inner();

    doc.resolve(0.0);
    if resources.rejected_image.load(Ordering::Relaxed) {
        return Err("an embedded image is invalid or exceeds the resource budget".to_owned());
    }
    let (logical_width, logical_height, links) = document_geometry(&doc)?;
    let width = pixel_extent(logical_width, scale, MAX_RENDER_WIDTH, "document width")?;
    let height = pixel_extent(logical_height, scale, MAX_RENDER_HEIGHT, "document height")?;
    let pixel_count = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| "document raster dimensions overflow".to_owned())?;
    if pixel_count > MAX_RENDER_PIXELS {
        return Err(format!(
            "document raster exceeds {MAX_RENDER_PIXELS} pixel limit"
        ));
    }

    let pixels = render_to_buffer::<VelloCpuImageRenderer, _>(
        |scene| {
            scene.fill(
                Fill::NonZero,
                Default::default(),
                Color::WHITE,
                Default::default(),
                &Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
            );
            paint_scene(scene, &mut doc, f64::from(scale), width, height, 0, 0);
        },
        width,
        height,
    );

    Ok(RenderedHtml {
        width,
        height,
        scale,
        pixels,
        links,
    })
}

fn pixel_extent(logical: f32, scale: f32, max: u32, name: &str) -> Result<u32, String> {
    if !logical.is_finite() || logical < 0.0 {
        return Err(format!("invalid {name}"));
    }
    let px = (f64::from(logical) * f64::from(scale)).ceil();
    if px > f64::from(max) || px > f64::from(u16::MAX) {
        return Err(format!("{name} exceeds rasterizer limit"));
    }
    Ok((px as u32).max(1))
}

fn document_geometry(doc: &blitz_dom::BaseDocument) -> Result<(f32, f32, Vec<HtmlLink>), String> {
    let mut width = doc.root_element().final_layout().size.width.max(1.0);
    let mut height = doc.root_element().final_layout().size.height.max(1.0);
    let scroll = doc.viewport_scroll();
    let mut links = Vec::new();

    for (id, node) in doc.tree().iter() {
        if !node.flags.is_in_document() || !node.is_element() {
            continue;
        }
        if let Some(rect) = doc.get_client_bounding_rect(id) {
            let right = rect.x + scroll.x + rect.width;
            let bottom = rect.y + scroll.y + rect.height;
            if right.is_finite() && bottom.is_finite() {
                width = width.max(right.max(0.0) as f32);
                height = height.max(bottom.max(0.0) as f32);
            }
        }

        let Some(element) = node.element_data().filter(|element| element.is_link()) else {
            continue;
        };
        let Some(href) = element.attr(local_name!("href")) else {
            continue;
        };
        for rect in doc.node_client_rects(id) {
            if rect.width > 0.0 && rect.height > 0.0 {
                links.push(HtmlLink {
                    href: href.to_owned(),
                    x: (rect.x + scroll.x) as f32,
                    y: (rect.y + scroll.y) as f32,
                    width: rect.width as f32,
                    height: rect.height as f32,
                });
            }
        }
    }

    if !width.is_finite() || !height.is_finite() {
        return Err("document has invalid layout dimensions".to_owned());
    }
    Ok((width, height, links))
}

#[derive(Default)]
struct EmbeddedImages {
    total_pixels: AtomicUsize,
    rejected_image: AtomicBool,
}

impl NetProvider for EmbeddedImages {
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url.as_str();
        let is_embedded_image = url.starts_with("data:image/");
        let bytes = if request.method == blitz_traits::net::Method::GET
            && url.len() <= MAX_IMAGE_DATA_URL_BYTES
            && url.starts_with("data:")
        {
            match decode_embedded_image(url, &self.total_pixels) {
                Ok(bytes) => bytes,
                Err(_) => {
                    if is_embedded_image {
                        self.rejected_image.store(true, Ordering::Relaxed);
                    }
                    Vec::new()
                }
            }
        } else {
            if is_embedded_image {
                self.rejected_image.store(true, Ordering::Relaxed);
            }
            Vec::new()
        };
        handler.bytes(url.to_owned(), Bytes::from(bytes));
    }
}

fn decode_embedded_image(url: &str, total_pixels: &AtomicUsize) -> Result<Vec<u8>, String> {
    let data_url = DataUrl::process(url).map_err(|error| error.to_string())?;
    let mime = data_url.mime_type();
    let format = match (mime.type_.as_str(), mime.subtype.as_str()) {
        ("image", "png") => image::ImageFormat::Png,
        ("image", "jpeg") => image::ImageFormat::Jpeg,
        ("image", "gif") => image::ImageFormat::Gif,
        ("image", "webp") => image::ImageFormat::WebP,
        _ => return Err("unsupported embedded image MIME type".to_owned()),
    };

    // Bound the encoded input and decoded output before accepting the resource.
    let mut encoded = Vec::new();
    data_url
        .decode(|chunk| {
            if encoded.len().saturating_add(chunk.len()) > MAX_IMAGE_DATA_URL_BYTES {
                return Err("embedded image exceeds byte limit");
            }
            encoded.extend_from_slice(chunk);
            Ok(())
        })
        .map_err(|error| format!("invalid embedded image data: {error}"))?;

    let reader = ImageReader::with_format(Cursor::new(encoded.as_slice()), format);
    let (width, height) = reader
        .into_dimensions()
        .map_err(|error| format!("invalid embedded image: {error}"))?;
    if width > MAX_RENDER_WIDTH || height > MAX_RENDER_HEIGHT {
        return Err("embedded image dimensions exceed limit".to_owned());
    }
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .filter(|pixels| *pixels > 0 && *pixels <= MAX_IMAGE_PIXELS)
        .ok_or_else(|| "embedded image dimensions exceed limit".to_owned())?;
    let updated = total_pixels.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
        used.checked_add(pixels)
            .filter(|next| *next <= MAX_TOTAL_IMAGE_PIXELS)
    });
    if updated.is_err() {
        return Err("total embedded image pixel budget exceeded".to_owned());
    }

    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use image::ImageEncoder;
    use parking_lot::Mutex;

    #[test]
    fn renders_embedded_image_links_and_horizontal_overflow() {
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[255, 0, 0, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        let data_uri = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        );
        let html = format!(
            "<a href=\"mailto:reader@example.test\">message link</a>\
             <img width=\"16\" height=\"16\" src=\"{data_uri}\">\
             <div style=\"width:1400px;height:4px;background:#00ff00\"></div>"
        );

        let rendered = render(&html, 320, 1.0).unwrap();
        assert!(rendered.width >= 1400, "wide content was clipped");
        assert!(rendered.links.iter().any(|link| {
            link.href == "mailto:reader@example.test" && link.width > 0.0 && link.height > 0.0
        }));
        assert!(
            rendered
                .pixels
                .chunks_exact(4)
                .any(|pixel| pixel[0] > 200 && pixel[1] < 60 && pixel[2] < 60),
            "embedded red image was not rasterized"
        );
    }

    #[test]
    fn rejects_remote_file_and_non_image_data_resources() {
        #[derive(Clone)]
        struct Capture(Arc<Mutex<Option<Vec<u8>>>>);

        impl NetHandler for Capture {
            fn bytes(self: Box<Self>, _resolved_url: String, bytes: Bytes) {
                *self.0.lock() = Some(bytes.to_vec());
            }
        }

        let provider = EmbeddedImages::default();
        for url in [
            "https://example.test/tracker.png",
            "file:///etc/passwd",
            "data:text/css,body%7Bbackground:red%7D",
            "data:image/svg+xml,%3Csvg%3E%3C/svg%3E",
        ] {
            let captured = Arc::new(Mutex::new(None));
            provider.fetch(
                1,
                Request::get(blitz_traits::net::Url::parse(url).unwrap()),
                Box::new(Capture(captured.clone())),
            );
            assert_eq!(*captured.lock(), Some(Vec::new()), "{url}");
        }
    }
}
