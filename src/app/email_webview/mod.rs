//! Email bodies in the reader, drawn by a native `WKWebView` (via `gpui-wry`).
//!
//! GPUI's own HTML renderers overflow the small stacks of its worker threads on deeply nested
//! mail, and WebKit is what mail is written for. The cost is that a native view cannot be
//! painted over by GPUI, so this module owns four things:
//!
//! * **Lifetime.** A message renders into its view once and keeps it: every view belongs to a
//!   `(message, slot)` key, is only hidden when its tab is not showing, and is rebuilt only when
//!   the document, the remote-image policy or the theme changes. The oldest views beyond
//!   [`MAX_LIVE`] are dropped (and rebuilt on their next show); closing a tab drops its views.
//! * **Geometry.** The view is as tall as its document (measured after load and after a width
//!   change), so GPUI's reader scroll container does the scrolling for lone messages and threads
//!   alike. Each frame the view is placed at the element's bounds, clipped natively to the
//!   scroll viewport, with every edge rounded and unchanged placements skipped.
//! * **Layering.** While a GPUI overlay is open or a drag is active, the view is swapped for a
//!   snapshot of the same pixels and hidden. Snapshots are epoch-guarded and cached per view.
//! * **Safety.** No page JavaScript, no in-view navigation (links open in the browser), no
//!   downloads or windows, and remote content blocked by a content rule when asked.
//!
//! The test platform has no `NSWindow`; there the views degrade to an empty element and the
//! documents that would have been loaded are recorded ([`EmailWebViews::loads`]).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::panic::Location;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::*;
use objc2::Message as _;
use objc2_web_kit::WKWebView;

use super::mail_app::MailApp;
use crate::model::MessageId;

mod native;

/// Live native views kept at once; the least recently shown beyond this are dropped.
pub const MAX_LIVE: usize = 8;
/// Height of a document until its first measurement.
const DEFAULT_HEIGHT: f32 = 48.;
/// Extra page above and below the viewport that a snapshot covers.
const SNAP_MARGIN: f32 = 16.;
/// Re-measure the document this long after it loads, for images that arrive late.
const REMEASURE: [Duration; 2] = [Duration::from_millis(300), Duration::from_millis(1500)];
/// The document's laid-out height.
const MEASURE_JS: &str = "Math.ceil(document.documentElement.getBoundingClientRect().height)";

/// One view per message per reader slot (the user's tab, or the Triage tab).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ViewKey {
    pub msg: MessageId,
    pub triage: bool,
}

/// Everything besides the HTML that changes what a document looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocStyle {
    /// Block remote content (the "Block remote images" setting).
    pub block: bool,
    pub dark: bool,
    /// Default text and background colors (`0xRRGGBB`) for mail that sets none.
    pub fg: u32,
    pub bg: u32,
}

/// A document the headless platform would have loaded.
#[derive(Clone, Debug)]
pub struct LoadedDoc {
    pub key: ViewKey,
    pub style: DocStyle,
    pub document: String,
}

/// Identity of the loaded document: a hash of the HTML's bytes plus the style. The hash is taken
/// when a message's HTML is first seen or changes, never per frame (see [`Fingerprint`]).
#[derive(Clone, Copy, PartialEq, Eq)]
struct DocSig {
    hash: u64,
    style: DocStyle,
}

/// Cheap per-frame check of whether a message's HTML is still the one that was hashed: its
/// address, length and 128 sampled bytes. Only when that changes is the full hash recomputed.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Fingerprint {
    ptr: usize,
    len: usize,
    ends: u64,
}

impl Fingerprint {
    fn of(html: &str) -> Self {
        let bytes = html.as_bytes();
        let mut hasher = DefaultHasher::new();
        bytes[..bytes.len().min(64)].hash(&mut hasher);
        bytes[bytes.len().saturating_sub(64)..].hash(&mut hasher);
        Self { ptr: bytes.as_ptr() as usize, len: bytes.len(), ends: hasher.finish() }
    }
}

fn content_hash(html: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    html.hash(&mut hasher);
    hasher.finish()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Nothing loaded (new, or its native view was dropped); the next render queues a document.
    Empty,
    /// A document is queued for the next prepaint to load.
    Pending,
    Live,
    /// No native window: the document was recorded instead.
    Headless,
    /// The native view could not be built; stays blank until the document changes.
    Failed,
}

struct Pending {
    document: String,
    block: bool,
}

/// A snapshot of `rect` of the page (`[x, y, w, h]`, view coordinates) taken when the view was
/// `size` big.
struct Snap {
    rect: [f32; 4],
    size: [f32; 2],
    data: Arc<RenderImage>,
}

/// State shared with the callbacks WebKit runs later.
struct ViewState {
    height: Cell<f32>,
    /// Width the document was last laid out at.
    width: Cell<f32>,
    loaded: Cell<bool>,
    /// Bumped by anything that makes an in-flight snapshot stale; completions must match it.
    epoch: Cell<u64>,
    inflight: Cell<bool>,
    snap: RefCell<Option<Snap>>,
    /// Visible part of the view in view coordinates, and the view's size, as of the last frame.
    visible: Cell<[f32; 4]>,
    size: Cell<[f32; 2]>,
}

impl ViewState {
    fn new() -> Rc<Self> {
        Rc::new(Self {
            height: Cell::new(DEFAULT_HEIGHT),
            width: Cell::new(f32::NAN),
            loaded: Cell::new(false),
            epoch: Cell::new(0),
            inflight: Cell::new(false),
            snap: RefCell::new(None),
            visible: Cell::new([0.; 4]),
            size: Cell::new([0.; 2]),
        })
    }

    /// Invalidate any snapshot request in flight.
    fn bump(&self) -> u64 {
        self.inflight.set(false);
        self.epoch.set(self.epoch.get() + 1);
        self.epoch.get()
    }

    /// A new document: forget everything measured or captured of the old one.
    fn reset(&self) {
        self.bump();
        self.loaded.set(false);
        self.width.set(f32::NAN);
        *self.snap.borrow_mut() = None;
    }

    /// The cached snapshot, if it covers the `visible` part of a view still `size` big.
    fn snapshot_for(&self, visible: [f32; 4]) -> Option<(Arc<RenderImage>, [f32; 4])> {
        let snap = self.snap.borrow();
        let snap = snap.as_ref()?;
        let same_size = (snap.size[0] - self.size.get()[0]).abs() < 0.5
            && (snap.size[1] - self.size.get()[1]).abs() < 0.5;
        let [x, y, w, h] = snap.rect;
        let covers = visible[0] >= x - 0.5
            && visible[1] >= y - 0.5
            && visible[0] + visible[2] <= x + w + 0.5
            && visible[1] + visible[3] <= y + h + 0.5;
        (same_size && covers).then(|| (snap.data.clone(), snap.rect))
    }
}

struct Entry {
    sig: DocSig,
    fingerprint: Fingerprint,
    blocked: usize,
    phase: Phase,
    pending: Option<Pending>,
    state: Rc<ViewState>,
    native: Option<native::Native>,
    /// Logical clock of the last frame this view was laid out in.
    used: u64,
}

impl Entry {
    /// Hide the native view and free its snapshot: a view nobody sees keeps no pixels.
    fn hide(&self) {
        if let Some(native) = &self.native {
            native.set_visible(false);
            if self.state.snap.borrow_mut().take().is_some() {
                self.state.bump();
            }
        }
    }
}

/// Wakes `MailApp` (and so the reader) from WebKit callbacks and timers.
#[derive(Clone)]
struct Notifier {
    app: AsyncApp,
    target: WeakEntity<MailApp>,
}

impl Notifier {
    fn notify(&self) {
        let mut app = self.app.clone();
        let _ = self.target.update(&mut app, |_, cx| cx.notify());
    }

    fn after(&self, delay: Duration, then: impl FnOnce() + 'static) {
        self.app
            .spawn(async move |cx| {
                cx.background_executor().timer(delay).await;
                then();
            })
            .detach();
    }
}

struct Inner {
    entries: RefCell<HashMap<ViewKey, Entry>>,
    /// The keys the last render pass of each slot (`[tab, triage]`) drew.
    passes: [RefCell<Vec<ViewKey>>; 2],
    clock: Cell<u64>,
    overlay: Cell<bool>,
    was_overlay: Cell<bool>,
    slot_visible: [Cell<bool>; 2],
    shared: RefCell<Option<native::Shared>>,
    notifier: Notifier,
    loads: RefCell<Vec<LoadedDoc>>,
}

/// The reader's email views. Cheap to clone; lives in `MailApp`.
#[derive(Clone)]
pub struct EmailWebViews(Rc<Inner>);

impl EmailWebViews {
    pub fn new(cx: &mut Context<MailApp>) -> Self {
        Self(Rc::new(Inner {
            entries: RefCell::default(),
            passes: Default::default(),
            clock: Cell::new(0),
            overlay: Cell::new(false),
            was_overlay: Cell::new(false),
            slot_visible: [Cell::new(true), Cell::new(true)],
            shared: RefCell::new(None),
            notifier: Notifier { app: cx.to_async(), target: cx.weak_entity() },
            loads: RefCell::default(),
        }))
    }

    /// The element for `html`, and how many remote images the policy blocks in it. Queues a new
    /// document only when the HTML, policy or theme differ from what the view has (or it has
    /// none); an unchanged message costs a comparison and clones nothing.
    pub fn body(&self, key: ViewKey, html: &str, style: DocStyle) -> (EmailView, usize) {
        let inner = &*self.0;
        inner.passes[usize::from(key.triage)].borrow_mut().push(key);
        let fingerprint = Fingerprint::of(html);
        let mut entries = inner.entries.borrow_mut();
        let e = entries.entry(key).or_insert_with(|| Entry {
            sig: DocSig { hash: 0, style },
            fingerprint: Fingerprint { ptr: 0, len: 0, ends: 0 },
            blocked: 0,
            phase: Phase::Empty,
            pending: None,
            state: ViewState::new(),
            native: None,
            used: 0,
        });
        if e.fingerprint != fingerprint {
            e.fingerprint = fingerprint;
            e.sig.hash = content_hash(html);
        }
        let sig = DocSig { hash: e.sig.hash, style };
        if e.sig != sig || e.phase == Phase::Empty {
            e.sig = sig;
            e.blocked = if style.block { crate::reading::blocked_images(html) } else { 0 };
            e.pending = Some(Pending { document: document(html, style), block: style.block });
            e.phase = Phase::Pending;
            e.state.reset();
        }
        (EmailView { views: self.clone(), key, state: e.state.clone() }, e.blocked)
    }

    /// A reader slot is about to draw its message views.
    pub fn begin_pass(&self, triage: bool) {
        self.0.passes[usize::from(triage)].borrow_mut().clear();
    }

    /// The slot has drawn: hide its views that were not part of the pass (another tab's, or none
    /// at all when the reader shows something else).
    pub fn end_pass(&self, triage: bool) {
        let pass = self.0.passes[usize::from(triage)].borrow();
        for (key, e) in self.0.entries.borrow().iter() {
            if key.triage == triage && !pass.contains(key) {
                e.hide();
            }
        }
    }

    /// Whether a slot's reader is on screen at all (not zoomed away). Hides its views when not.
    pub fn set_slot_visible(&self, triage: bool, visible: bool) {
        if self.0.slot_visible[usize::from(triage)].replace(visible) && !visible {
            for (key, e) in self.0.entries.borrow().iter() {
                if key.triage == triage {
                    e.hide();
                }
            }
        }
    }

    /// Whether a GPUI overlay (modal, menu, popover, toast) covers the window; drags are
    /// detected per frame.
    pub fn set_overlay(&self, open: bool) {
        self.0.overlay.set(open);
    }

    /// Drop every view for which `gone` holds (closed tabs, ended sessions).
    pub fn forget(&self, gone: impl Fn(&ViewKey) -> bool) {
        self.0.entries.borrow_mut().retain(|key, _| !gone(key));
    }

    /// Fetch the selected text of every visible view and put the first non-empty one on the
    /// clipboard. Returns whether any view was asked.
    pub fn copy_selection(&self) -> bool {
        let mut asked = false;
        for e in self.0.entries.borrow().values() {
            let Some(native) = e.native.as_ref().filter(|n| n.is_visible()) else { continue };
            asked = true;
            let app = self.0.notifier.app.clone();
            native::eval(native.web(), "String(window.getSelection())", move |value| {
                if let Some(text) = native::as_string(value).filter(|t| !t.is_empty()) {
                    app.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(text)));
                }
            });
        }
        asked
    }

    /// Collapse the page selection of every visible view (a click elsewhere in the window).
    pub fn clear_selections(&self) {
        for e in self.0.entries.borrow().values() {
            if let Some(native) = e.native.as_ref().filter(|n| n.is_visible()) {
                native::eval(native.web(), "window.getSelection().removeAllRanges()", |_| {});
            }
        }
    }

    /// Documents the headless platform recorded instead of loading.
    pub fn loads(&self) -> Vec<LoadedDoc> {
        self.0.loads.borrow().clone()
    }

    fn prepaint(
        &self,
        key: ViewKey,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Swap> {
        let inner = &*self.0;
        let overlay = inner.overlay_now(cx);
        let mut entries = inner.entries.borrow_mut();
        if entries.get(&key).is_some_and(|e| e.phase == Phase::Pending && e.native.is_none()) {
            inner.make_room(&mut entries, key);
        }
        let e = entries.get_mut(&key)?;
        inner.clock.set(inner.clock.get() + 1);
        e.used = inner.clock.get();
        if e.phase == Phase::Pending {
            inner.realize(key, e, window, cx);
        }
        let native = e.native.as_ref()?;
        let state = &e.state;

        // Placement: the clip container is the visible part, the page sits inside it at its
        // full size. Every edge is rounded on its own so neighbours never gap or overlap.
        let vis = bounds.intersect(&window.content_mask().bounds);
        let edge = |p: Pixels| f32::from(p).round();
        let (cl, ct, cr, cb) = (edge(vis.left()), edge(vis.top()), edge(vis.right()), edge(vis.bottom()));
        let (vl, vt, vr, vb) =
            (edge(bounds.left()), edge(bounds.top()), edge(bounds.right()), edge(bounds.bottom()));
        native.place([cl, ct, cr - cl, cb - ct], [vl - cl, vt - ct, vr - vl, vb - vt]);
        let visible = [cl - vl, ct - vt, cr - cl, cb - ct];
        state.visible.set(visible);
        state.size.set([vr - vl, vb - vt]);
        if state.width.replace(vr - vl) != vr - vl && state.loaded.get() {
            measure(native.web(), state, &inner.notifier);
        }

        let showing = inner.slot_visible[usize::from(key.triage)].get() && visible[2] >= 1. && visible[3] >= 1.;
        if !showing {
            e.hide();
            return None;
        }
        if overlay {
            if let Some((data, rect)) = state.snapshot_for(visible) {
                native.set_visible(false);
                let at = |x: f32, y: f32, w: f32, h: f32| Bounds {
                    origin: point(px(vl + x), px(vt + y)),
                    size: size(px(w), px(h)),
                };
                return Some(Swap {
                    data,
                    visible: at(visible[0], visible[1], visible[2], visible[3]),
                    image: at(rect[0], rect[1], rect[2], rect[3]),
                });
            }
            // Not captured yet: stay native until the snapshot lands.
            native.set_visible(true);
            if state.loaded.get() && !state.inflight.get() {
                inner.snapshot(native, state, visible, cx);
            }
            return None;
        }
        native.set_visible(true);
        None
    }
}

impl Inner {
    /// The overlay flag for this frame, discarding in-flight snapshots when an overlay closes.
    fn overlay_now(&self, cx: &App) -> bool {
        let now = self.overlay.get() || cx.has_active_drag();
        if self.was_overlay.replace(now) && !now {
            for e in self.entries.borrow().values() {
                e.state.bump();
            }
        }
        now
    }

    /// Drop the least recently shown native view if building `key`'s would exceed the cap. Views
    /// the latest render passes drew are never chosen.
    fn make_room(&self, entries: &mut HashMap<ViewKey, Entry>, key: ViewKey) {
        if entries.values().filter(|e| e.native.is_some()).count() < MAX_LIVE {
            return;
        }
        let drawn = |k: &ViewKey| self.passes.iter().any(|p| p.borrow().contains(k));
        let victim = lru_victim(
            entries
                .iter()
                .filter(|(k, e)| e.native.is_some() && **k != key && !drawn(k))
                .map(|(k, e)| (*k, e.used)),
        );
        if let Some(victim) = victim.and_then(|k| entries.get_mut(&k)) {
            victim.native = None;
            victim.pending = None;
            victim.phase = Phase::Empty;
            victim.state.reset();
        }
    }

    /// Load the queued document into the entry's view, building the view first if it has none.
    fn realize(&self, key: ViewKey, e: &mut Entry, window: &mut Window, cx: &mut App) {
        let Some(pending) = e.pending.take() else {
            e.phase = Phase::Empty;
            return;
        };
        if !native::available(window) {
            e.phase = Phase::Headless;
            self.loads.borrow_mut().push(LoadedDoc {
                key,
                style: e.sig.style,
                document: pending.document,
            });
            return;
        }
        let mut shared = self.shared.borrow_mut();
        let shared = shared.get_or_insert_with(|| native::Shared::new().expect("GPUI draws on the main thread"));
        let rules = if pending.block {
            let notifier = self.notifier.clone();
            match shared.rules(move || notifier.notify()) {
                Some(rules) => Some(rules),
                // Compiling (the render is notified when it is done) or failed: stay blank
                // rather than load remote content.
                None => {
                    e.pending = Some(pending);
                    return;
                }
            }
        } else {
            None
        };
        match &e.native {
            Some(native) => native.load(&pending.document, rules.as_ref()),
            None => {
                let (state, notifier) = (e.state.clone(), self.notifier.clone());
                let config = native::Config {
                    html: &pending.document,
                    rules,
                    on_finished: Box::new(move |web: &WKWebView| {
                        state.loaded.set(true);
                        remeasure(web, &state, &notifier);
                    }),
                };
                match native::Native::new(window, cx, shared, config) {
                    Ok(native) => e.native = Some(native),
                    Err(_) => {
                        e.phase = Phase::Failed;
                        return;
                    }
                }
            }
        }
        e.phase = Phase::Live;
    }

    /// Capture the visible part of the page plus a small margin when an overlay needs it, decode it
    /// off the UI thread and keep only the decoded image while the view stays on screen. A completion that finds the epoch moved on is discarded.
    fn snapshot(&self, native: &native::Native, state: &Rc<ViewState>, visible: [f32; 4], cx: &App) {
        let size = state.size.get();
        let top = (visible[1] - SNAP_MARGIN).max(0.);
        let bottom = (visible[1] + visible[3] + SNAP_MARGIN).min(size[1]);
        let rect = [visible[0], top, visible[2], (bottom - top).max(visible[3])];
        let epoch = state.bump();
        state.inflight.set(true);
        let (state, notifier, renderer) = (state.clone(), self.notifier.clone(), cx.svg_renderer());
        native::snapshot(native.web(), rect, native.scale(), move |bytes| {
            if state.epoch.get() != epoch {
                return;
            }
            let Some(bytes) = bytes else {
                state.inflight.set(false);
                return;
            };
            let image = Image::from_bytes(ImageFormat::Tiff, bytes);
            let (state, wake, renderer) = (state.clone(), notifier.clone(), renderer.clone());
            notifier
                .app
                .spawn(async move |cx| {
                    let decoded =
                        cx.background_executor().spawn(async move { image.to_image_data(renderer) }).await;
                    if state.epoch.get() != epoch {
                        return;
                    }
                    state.inflight.set(false);
                    if let Ok(data) = decoded {
                        *state.snap.borrow_mut() = Some(Snap { rect, size, data });
                        wake.notify();
                    }
                })
                .detach();
        });
    }
}

/// Read the document height into the view state, waking the reader when it changed.
fn measure(web: &WKWebView, state: &Rc<ViewState>, notifier: &Notifier) {
    let (state, notifier) = (state.clone(), notifier.clone());
    native::eval(web, MEASURE_JS, move |value| {
        let Some(height) = native::as_f64(value).map(|h| h as f32).filter(|h| *h > 0.) else {
            return;
        };
        if (state.height.get() - height).abs() >= 0.5 {
            state.height.set(height);
            state.bump();
            notifier.notify();
        }
    });
}

/// Measure now and again shortly after, for images that finish loading late.
fn remeasure(web: &WKWebView, state: &Rc<ViewState>, notifier: &Notifier) {
    measure(web, state, notifier);
    for delay in REMEASURE {
        let (web, state, again) = (web.retain(), state.clone(), notifier.clone());
        notifier.after(delay, move || measure(&web, &state, &again));
    }
}

/// The key to evict: the one shown least recently.
pub fn lru_victim<K>(candidates: impl Iterator<Item = (K, u64)>) -> Option<K> {
    candidates.min_by_key(|(_, used)| *used).map(|(key, _)| key)
}

/// The page we load: our default styling, then the message exactly as received. Written before
/// the message so anything it sets itself wins.
fn document(html: &str, style: DocStyle) -> String {
    let scheme = if style.dark { "dark" } else { "light" };
    let mut out = String::with_capacity(html.len() + 420);
    let _ = write!(
        out,
        "<!DOCTYPE html><meta charset=\"utf-8\"><style>\
         :root{{color-scheme:{scheme}}}\
         html{{background:#{:06x};color:#{:06x};font:13.5px/1.55 -apple-system,system-ui,sans-serif;\
         overflow-wrap:break-word}}\
         body{{margin:0}}img{{max-width:100%;height:auto}}</style>",
        style.bg, style.fg
    );
    out.push_str(html);
    out
}

/// A color as `0xRRGGBB`.
pub fn rgb(color: Hsla) -> u32 {
    let c: Rgba = color.into();
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u32;
    byte(c.r) << 16 | byte(c.g) << 8 | byte(c.b)
}

/// Relative luminance of `0xRRGGBB`, 0 (black) to 1 (white).
pub fn luminance(rgb: u32) -> f32 {
    let channel = |shift: u32| ((rgb >> shift) & 0xff) as f32 / 255.;
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

/// A snapshot to paint instead of the native view while an overlay is up.
pub struct Swap {
    data: Arc<RenderImage>,
    visible: Bounds<Pixels>,
    image: Bounds<Pixels>,
}

/// The element for one email view: as tall as its document, full width.
pub struct EmailView {
    views: EmailWebViews,
    key: ViewKey,
    state: Rc<ViewState>,
}

impl IntoElement for EmailView {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for EmailView {
    type RequestLayoutState = ();
    type PrepaintState = Option<Swap>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = px(self.state.height.get()).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Swap> {
        let swap = self.views.prepaint(self.key, bounds, window, cx);
        // The native view takes the mouse; keep GPUI's hover and click targets off it too.
        window.insert_hitbox(bounds.intersect(&window.content_mask().bounds), HitboxBehavior::Normal);
        swap
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        swap: &mut Option<Swap>,
        window: &mut Window,
        _: &mut App,
    ) {
        if let Some(swap) = swap.take() {
            let _ = window.paint_image(swap.visible, swap.image, Corners::default(), swap.data, 0, false);
        }
    }
}
