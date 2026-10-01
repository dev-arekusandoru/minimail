//! Bounded HTML rendering for email message bodies.
//!
//! The returned buffer is RGBA8, premultiplied-alpha data. Width and height are
//! physical pixel dimensions; link rectangles and `scale` are CSS/logical values.
//!
//! Rendering never uses Blitz's default network stack. Embedded `data:` raster
//! images are always decoded in-process. Remote `http(s)` raster images are
//! fetched only when `block_remote_images` is `false`, against public hosts
//! only, and every other sub-resource (stylesheets, fonts, files, SVG) is denied
//! without network access.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyrender::{PaintScene as _, render_to_buffer};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{DocumentConfig, StyleThreading, util::Color};
use blitz_html::HtmlDocument;
use blitz_paint::paint_scene;
use blitz_traits::shell::{ColorScheme, Viewport};
use peniko::Fill;
use peniko::kurbo::Rect;

mod resources;

use resources::{REMOTE_RENDER_DEADLINE, RemoteImages};

use blitz_dom::local_name;
const MAX_HTML_BYTES: usize = 8 * 1024 * 1024;
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
    /// Remote HTTP(S) images refused because remote images are blocked.
    ///
    /// Always `0` when `block_remote_images` is `false`.
    pub blocked_remote_images: usize,
}

/// A transport that fetches a remote image's encoded bytes.
///
/// Return `Err` to report an unavailable image; the renderer treats it as a
/// missing image rather than failing the whole document.
pub type RemoteImageFetcher = Arc<dyn Fn(&str) -> Result<Vec<u8>, String> + Send + Sync + 'static>;

/// Parse, style, lay out, and CPU-rasterize HTML.
///
/// Embedded `data:` raster images are always decoded in-process. Remote `http(s)`
/// raster images are fetched only when `block_remote_images` is `false`, and only
/// against public hosts. `width` is the CSS viewport width; overflow is preserved
/// in the returned full-document raster when it fits safety limits.
/// `color_scheme` supplies the CSS `prefers-color-scheme` preference; authored
/// colors are preserved rather than inverted.
pub fn render(
    html: &str,
    width: u32,
    scale: f32,
    block_remote_images: bool,
    color_scheme: ColorScheme,
) -> Result<RenderedHtml, String> {
    render_with_fetcher(
        html,
        width,
        scale,
        block_remote_images,
        color_scheme,
        resources::default_fetcher(),
    )
}

/// [`render`] with an injected remote-image transport.
///
/// Production callers use [`render`]; this exists so tests can exercise the
/// policy (including the allowed path) without any network access.
pub fn render_with_fetcher(
    html: &str,
    width: u32,
    scale: f32,
    block_remote_images: bool,
    color_scheme: ColorScheme,
    fetch: RemoteImageFetcher,
) -> Result<RenderedHtml, String> {
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

    let resources = Arc::new(RemoteImages::new(block_remote_images, fetch));
    let mut doc = HtmlDocument::from_html(
        html,
        DocumentConfig {
            viewport: Some(Viewport::new(
                physical_width as u32,
                physical_height as u32,
                scale,
                color_scheme,
            )),
            base_url: Some("https://mail.invalid/".to_owned()),
            net_provider: Some(resources.clone()),
            style_threading: StyleThreading::Sequential,
            ..Default::default()
        },
    )
    .into_inner();

    resolve_resources(&mut doc, &resources);

    if resources.embedded_rejected() {
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
        blocked_remote_images: resources.blocked_remote(),
    })
}

/// Drive `resolve` until every sub-resource response has been ingested.
///
/// Synchronous responses (embedded/denied/cached) are queued during `resolve`
/// itself and are only ingested by the *next* call, so we keep resolving until a
/// pass neither starts nor delivers any new work. Remote fetches run on the
/// worker pool; `in_flight` tracks them and `REMOTE_RENDER_DEADLINE` bounds the
/// total wait so a slow host can never stall a message indefinitely.
fn resolve_resources(doc: &mut blitz_dom::BaseDocument, resources: &RemoteImages) {
    let deadline = Instant::now() + REMOTE_RENDER_DEADLINE;
    let mut previous_delivered = resources.delivered();
    loop {
        doc.resolve(0.0);
        let delivered = resources.delivered();
        if resources.in_flight() == 0 && delivered == previous_delivered {
            return;
        }
        previous_delivered = delivered;
        if Instant::now() >= deadline {
            return;
        }
        thread::sleep(Duration::from_millis(2));
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use image::ImageEncoder;
    use parking_lot::Mutex;

    fn png_bytes(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..(width * height) {
            pixels.extend_from_slice(&rgba);
        }
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&pixels, width, height, image::ExtendedColorType::Rgba8)
            .unwrap();
        png
    }

    fn has_red(rendered: &RenderedHtml) -> bool {
        rendered
            .pixels
            .chunks_exact(4)
            .any(|pixel| pixel[0] > 200 && pixel[1] < 60 && pixel[2] < 60)
    }

    /// Records every URL the injected transport is asked for.
    fn recording_fetch(bytes: Vec<u8>) -> (RemoteImageFetcher, Arc<Mutex<Vec<String>>>) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        let fetch: RemoteImageFetcher = Arc::new(move |url: &str| {
            seen.lock().push(url.to_owned());
            Ok(bytes.clone())
        });
        (fetch, requests)
    }

    #[test]
    fn renders_embedded_image_links_and_horizontal_overflow() {
        let data_uri = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png_bytes(1, 1, [255, 0, 0, 255]))
        );
        let html = format!(
            "<a href=\"mailto:reader@example.test\">message link</a>\
             <img width=\"16\" height=\"16\" src=\"{data_uri}\">\
             <div style=\"width:1400px;height:4px;background:#00ff00\"></div>"
        );

        let rendered = render(&html, 320, 1.0, false, ColorScheme::Light).unwrap();
        assert!(rendered.width >= 1400, "wide content was clipped");
        assert!(rendered.links.iter().any(|link| {
            link.href == "mailto:reader@example.test" && link.width > 0.0 && link.height > 0.0
        }));
        assert!(has_red(&rendered), "embedded red image was not rasterized");
        assert_eq!(rendered.blocked_remote_images, 0);
    }

    #[test]
    fn remote_images_load_when_allowed_and_count_when_blocked() {
        let (fetch, requests) = recording_fetch(png_bytes(8, 8, [255, 0, 0, 255]));
        let html = "<p>tracked</p>\
                    <img width=\"16\" height=\"16\" src=\"https://cdn.example.test/hero.png\">";

        let blocked =
            render_with_fetcher(html, 320, 1.0, true, ColorScheme::Light, Arc::clone(&fetch))
                .unwrap();
        assert_eq!(blocked.blocked_remote_images, 1);
        assert!(!has_red(&blocked));
        assert!(
            requests.lock().is_empty(),
            "blocked mode must not consult the network transport"
        );

        let allowed =
            render_with_fetcher(html, 320, 1.0, false, ColorScheme::Light, fetch).unwrap();
        assert_eq!(allowed.blocked_remote_images, 0);
        assert_eq!(
            requests.lock().clone(),
            vec!["https://cdn.example.test/hero.png".to_owned()]
        );
        assert!(
            has_red(&allowed),
            "an allowed remote raster image must be rasterized"
        );
    }

    #[test]
    fn unavailable_remote_image_does_not_erase_the_message() {
        let fetch: RemoteImageFetcher = Arc::new(|_: &str| Err("offline".to_owned()));
        let html = "<p style=\"color:#0000ff\">still here</p>\
                    <img width=\"16\" height=\"16\" src=\"https://cdn.example.test/missing.png\">";

        let rendered =
            render_with_fetcher(html, 320, 1.0, false, ColorScheme::Light, fetch).unwrap();
        assert!(rendered.width >= 1 && rendered.height >= 1);
        assert!(!has_red(&rendered));
    }

    #[test]
    fn denies_files_stylesheets_and_non_raster_data_resources() {
        let (fetch, requests) = recording_fetch(png_bytes(1, 1, [255, 0, 0, 255]));
        let html = "<link rel=\"stylesheet\" href=\"file:///etc/passwd\">\
                    <img src=\"data:text/css,body%7B%7D\">\
                    <img width=\"16\" height=\"16\" src=\"https://cdn.example.test/site.css\">";

        let rendered =
            render_with_fetcher(html, 320, 1.0, false, ColorScheme::Light, fetch).unwrap();
        assert!(
            requests.lock().is_empty(),
            "non-image resources must be denied without a fetch: {:?}",
            requests.lock()
        );
        assert_eq!(rendered.blocked_remote_images, 0);
        assert!(!has_red(&rendered));
    }

    #[test]
    fn color_scheme_selects_email_css_without_inverting_authored_colors() {
        let html = "<style>\
            body { margin:0; background:#ffffff; }\
            #adaptive { width:20px; height:20px; background:#ff0000; }\
            @media (prefers-color-scheme:dark) { #adaptive { background:#0000ff; } }\
            </style><div id='adaptive'></div>\
            <div style='width:20px;height:20px;background:#00ff00'></div>";
        for (scheme, adaptive) in [
            (ColorScheme::Light, [255, 0, 0, 255]),
            (ColorScheme::Dark, [0, 0, 255, 255]),
        ] {
            let rendered = render(html, 100, 1.0, true, scheme).unwrap();
            let pixel = |x: usize, y: usize| {
                let offset = (y * rendered.width as usize + x) * 4;
                &rendered.pixels[offset..offset + 4]
            };
            assert_eq!(pixel(10, 10), adaptive);
            assert_eq!(pixel(10, 30), [0, 255, 0, 255]);
            assert_eq!(pixel(50, 10), [255, 255, 255, 255]);
        }
    }
}
