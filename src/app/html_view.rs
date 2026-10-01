//! Native GPUI host for the asynchronous Blitz-rendered message document.
use blitz_traits::shell::ColorScheme;
use gpui_kit::component::ActiveTheme as _;
use std::sync::Arc;

use gpui_kit::*;

/// An HTML document rasterized off the UI thread and displayed as a native image.
pub(crate) struct HtmlView {
    id: ElementId,
    html: String,
    block_remote_images: bool,
}

impl HtmlView {
    pub(crate) fn new(
        id: impl Into<ElementId>,
        html: impl Into<String>,
        block_remote_images: bool,
    ) -> Self {
        Self {
            id: id.into(),
            html: html.into(),
            block_remote_images,
        }
    }
}

impl IntoElement for HtmlView {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for HtmlView {
    type RequestLayoutState = (Entity<HtmlRasterView>, AnyElement);
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let color_scheme = if cx.theme().is_dark() {
            ColorScheme::Dark
        } else {
            ColorScheme::Light
        };
        let (layout, child_state) = window.with_element_state::<HtmlElementState, _>(
            id.expect("HtmlView requires its stable element id"),
            |state, window| {
                let state = state.unwrap_or_else(|| HtmlElementState {
                    view: cx.new(|_| {
                        HtmlRasterView::new(
                            self.html.clone(),
                            self.block_remote_images,
                            color_scheme,
                        )
                    }),
                });
                let view = state.view.clone();
                view.update(cx, |view, cx| {
                    view.set_document(&self.html, self.block_remote_images, color_scheme, cx)
                });
                let mut child = view.clone().into_any_element();
                let layout = child.request_layout(window, cx);
                ((layout, (view, child)), state)
            },
        );
        (layout, child_state)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        state.1.prepaint(window, cx);
        state.0.update(cx, |view, cx| {
            view.set_width(
                f32::from(bounds.size.width).max(1.) as u32,
                window.scale_factor(),
                cx,
            )
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        state.1.paint(window, cx);
    }
}

#[derive(Clone)]
struct HtmlElementState {
    view: Entity<HtmlRasterView>,
}

pub(crate) struct HtmlRasterView {
    html: String,
    block_remote_images: bool,
    color_scheme: ColorScheme,
    width: u32,
    scale: f32,
    generation: u64,
    needs_render: bool,
    in_flight: bool,
    rendered: Option<(u64, RasterizedDocument)>,
    error: Option<(u64, String)>,
}

struct RasterizedDocument {
    image: Arc<RenderImage>,
    width: u32,
    height: u32,
    links: Vec<crate::html::HtmlLink>,
}

impl HtmlRasterView {
    fn new(html: String, block_remote_images: bool, color_scheme: ColorScheme) -> Self {
        Self {
            html,
            block_remote_images,
            color_scheme,
            width: 0,
            scale: 1.,
            generation: 0,
            needs_render: true,
            in_flight: false,
            rendered: None,
            error: None,
        }
    }

    fn set_document(
        &mut self,
        html: &str,
        block_remote_images: bool,
        color_scheme: ColorScheme,
        _: &mut Context<Self>,
    ) {
        if self.html != html
            || self.block_remote_images != block_remote_images
            || self.color_scheme != color_scheme
        {
            if self.html != html {
                self.html = html.to_owned();
            }
            self.block_remote_images = block_remote_images;
            self.color_scheme = color_scheme;
            self.generation = self.generation.wrapping_add(1);
            self.needs_render = true;
            self.rendered = None;
            self.error = None;
        }
    }

    fn set_width(&mut self, width: u32, scale: f32, cx: &mut Context<Self>) {
        let width = width.clamp(1, 4096);
        let scale = scale.max(0.5);
        let changed = self.width != width || (self.scale - scale).abs() >= f32::EPSILON;
        if !changed && !self.needs_render {
            return;
        }
        if changed && !self.needs_render {
            self.generation = self.generation.wrapping_add(1);
            self.rendered = None;
            self.error = None;
        }
        self.width = width;
        self.scale = scale;
        self.needs_render = false;
        self.start_render(cx);
    }

    fn start_render(&mut self, cx: &mut Context<Self>) {
        if self.width == 0 || self.in_flight {
            return;
        }
        let key = self.generation;
        let html = self.html.clone();
        let width = self.width;
        let scale = self.scale;
        let block_remote_images = self.block_remote_images;
        let color_scheme = self.color_scheme;
        self.in_flight = true;
        let task = cx.background_spawn(async move {
            crate::html::render(&html, width, scale, block_remote_images, color_scheme)
                .and_then(prepare_rendered_image)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |view, cx| view.finish(key, result, cx))
                .ok();
        })
        .detach();
    }

    fn finish(
        &mut self,
        key: u64,
        result: Result<RasterizedDocument, String>,
        cx: &mut Context<Self>,
    ) {
        self.in_flight = false;
        if key != self.generation {
            self.start_render(cx);
            return;
        }
        match result {
            Ok(rendered) => {
                self.rendered = Some((key, rendered));
                self.error = None;
            }
            Err(error) => self.error = Some((key, error)),
        }
        cx.notify();
    }
}

impl Render for HtmlRasterView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let key = self.generation;
        if let Some((rendered_key, rendered)) = &self.rendered {
            if *rendered_key == key {
                let logical_w = rendered.width as f32 / self.scale;
                let logical_h = rendered.height as f32 / self.scale;
                let mut document = div()
                    .relative()
                    .w(px(logical_w))
                    .h(px(logical_h))
                    .bg(rgb(0xffffff))
                    .child(
                        img(rendered.image.clone())
                            .w(px(logical_w))
                            .h(px(logical_h)),
                    );
                for (index, link) in rendered.links.iter().enumerate() {
                    let Some(url) = checked_link(&link.href) else {
                        continue;
                    };
                    document = document.child(
                        div()
                            .id(("html-link", index))
                            .absolute()
                            .left(px(link.x))
                            .top(px(link.y))
                            .w(px(link.width))
                            .h(px(link.height))
                            .cursor_pointer()
                            .on_click(move |_, _, cx| {
                                cx.open_url(&url);
                            }),
                    );
                }
                div()
                    .id("html-scroll")
                    .w_full()
                    .overflow_x_scroll()
                    .child(document)
                    .into_any_element()
            } else {
                self.pending_or_error(key).into_any_element()
            }
        } else {
            self.pending_or_error(key).into_any_element()
        }
    }
}

impl HtmlRasterView {
    fn pending_or_error(&self, key: u64) -> Div {
        if let Some((error_key, error)) = &self.error
            && *error_key == key
        {
            return div()
                .w_full()
                .min_h(px(36.))
                .p_2()
                .bg(rgb(0xfff0f0))
                .text_color(rgb(0x9b1c1c))
                .text_size(px(12.))
                .child(format!("HTML rendering failed: {error}"));
        }
        div()
            .w_full()
            .min_h(px(36.))
            .bg(rgb(0xffffff))
            .child("Rendering message…")
    }
}

fn prepare_rendered_image(
    rendered: crate::html::RenderedHtml,
) -> Result<RasterizedDocument, String> {
    let mut pixels = rendered.pixels;
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let buffer =
        image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(rendered.width, rendered.height, pixels)
            .ok_or_else(|| "Blitz returned an invalid RGBA bitmap".to_owned())?;
    let frame = image::Frame::new(buffer);
    let image = Arc::new(RenderImage::new([frame]));
    Ok(RasterizedDocument {
        image,
        width: rendered.width,
        height: rendered.height,
        links: rendered.links,
    })
}

fn checked_link(href: &str) -> Option<String> {
    let url = url::Url::parse(href).ok()?;
    match url.scheme() {
        "http" | "https" | "mailto" => Some(url.to_string()),
        _ => None,
    }
}
