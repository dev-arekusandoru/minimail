//! The macOS half of the email web view: building a locked-down `WKWebView`, keeping its frame
//! in step with GPUI, and the few JavaScript / snapshot calls the app makes on it.
//!
//! Page content runs no JavaScript (`allowsContentJavaScript = false`); the app's own
//! `evaluateJavaScript` calls are unaffected by that switch. Every view sits in a [`ClipView`] so
//! the part of the page outside GPUI's scroll viewport is clipped natively.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use block2::RcBlock;
use gpui_kit::*;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObject, ProtocolObject};
use objc2::{ClassType, DeclaredClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSEvent, NSImage, NSView, NSWorkspace};
use objc2_foundation::{NSError, NSNumber, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL};
#[allow(deprecated)]
use objc2_web_kit::WKProcessPool;
use objc2_web_kit::{
    WKContentRuleList, WKContentRuleListStore, WKContentWorld, WKScriptMessage,
    WKScriptMessageHandler, WKSnapshotConfiguration, WKUserContentController, WKUserScript,
    WKUserScriptInjectionTime, WKWebView, WKWebViewConfiguration, WKWebpagePreferences,
    WKWebsiteDataStore,
};
use wry::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use wry::{
    NewWindowResponse, PageLoadEvent, WebViewBuilder, WebViewBuilderExtDarwin as _,
    WebViewBuilderExtMacos as _, WebViewExtMacOS as _, WryWebView,
};

/// Compiled rule lists are cached on disk under this identifier; change it with the rules.
const RULES_ID: &str = "mail-classifier.block-remote.v1";
/// Block every remote subresource a message could load: images, CSS, fonts, media, fetches.
const RULES: &str = r#"[{"trigger":{"url-filter":"^https?://","resource-type":["image","style-sheet","font","media","raw","svg-document"]},"action":{"type":"block"}}]"#;

define_class!(
    /// A flipped container: its children are positioned from the top left, like GPUI's.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "MailEmailClipView"]
    struct ClipView;

    impl ClipView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> Bool {
            Bool::YES
        }
    }

    unsafe impl NSObjectProtocol for ClipView {}
);

define_class!(
    /// The email's `WKWebView`: wry's view class, except that it never becomes first responder,
    /// so every key stays with GPUI. Mouse scrolling, links and selection are unaffected.
    #[unsafe(super(WryWebView))]
    #[name = "MailEmailWebView"]
    struct KeylessWebView;

    impl KeylessWebView {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> Bool {
            Bool::NO
        }

        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder(&self) -> Bool {
            Bool::NO
        }

        /// The page never scrolls on its own: wheel events go up the responder chain to GPUI's
        /// scroll container.
        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            // SAFETY: forwarding the same event to the next responder.
            unsafe {
                if let Some(next) = self.nextResponder() {
                    let _: () = msg_send![&*next, scrollWheel: event];
                }
            }
        }

        /// A click in the page is a click in the reader: tell the app, then let WebKit select.
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            // SAFETY: plain super call with the same event.
            unsafe { let _: () = msg_send![super(self), mouseDown: event]; }
            if let Some(handler) = CLICK.with(|c| c.borrow().clone()) {
                handler();
            }
        }
    }

    unsafe impl NSObjectProtocol for KeylessWebView {}
);

thread_local! {
    /// What a mouse-down inside any email view does (the app's "click in the reader").
    static CLICK: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
}

/// Set what a click inside an email view does.
pub fn set_click_handler(handler: Rc<dyn Fn()>) {
    CLICK.with(|c| *c.borrow_mut() = Some(handler));
}

struct HeightIvars {
    on_height: Box<dyn Fn(f64)>,
}

define_class!(
    /// Receives the page's laid-out height from the measuring script.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = HeightIvars]
    #[name = "MailEmailHeightHandler"]
    struct HeightHandler;

    unsafe impl NSObjectProtocol for HeightHandler {}

    unsafe impl WKScriptMessageHandler for HeightHandler {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn did_receive(&self, _: &WKUserContentController, message: &WKScriptMessage) {
            // SAFETY: reading the body of the message being delivered.
            let body = unsafe { message.body() };
            if let Some(number) = body.downcast_ref::<NSNumber>() {
                (self.ivars().on_height)(number.doubleValue());
            }
        }
    }
);

const HANDLER_NAME: &str = "mcHeight";
const WORLD_NAME: &str = "mail-classifier.measure";
/// Strips the email's own outer background so it blends with the reader card instead of painting
/// a differently-coloured box inside it. Runs in the same isolated world, before the measuring
/// script, at document end (the view is still hidden waiting for its height, so there is no
/// flash). An email may put a colour on `body`, on the outermost wrapper (typically a full-width
/// `<table width="100%">`/`<center>`/`div`), or both; inner cards are narrower than the body and
/// keep their background. The `html` colour set by our own stylesheet (the card colour) stays.
const NEUTRALIZE_SCRIPT: &str = r#"(function () {
  function clear(el) {
    if (!el || el.nodeType !== 1) return;
    if (el.style) {
      el.style.setProperty('background-color', 'transparent', 'important');
      el.style.setProperty('background-image', 'none', 'important');
      el.style.setProperty('background', 'transparent', 'important');
    }
    el.removeAttribute('bgcolor');
    el.removeAttribute('background');
  }
  function tag(el) { return el.tagName ? el.tagName.toUpperCase() : ''; }
  // The single rendered element child, or null when there is none or more than one. Head
  // furniture (style/script/link/meta) does not count as content.
  function soleChild(el) {
    var found = null, n = 0;
    for (var c = el.firstElementChild; c; c = c.nextElementSibling) {
      var t = tag(c);
      if (t === 'SCRIPT' || t === 'STYLE' || t === 'LINK' || t === 'META' || t === 'TITLE' ||
          t === 'BASE' || t === 'HEAD') continue;
      if (++n > 1) return null;
      found = c;
    }
    return n === 1 ? found : null;
  }
  var html = document.documentElement, body = document.body;
  if (!html || !body) return;
  // Our stylesheet owns <html>; drop any inline colour an email put on it.
  if (html.style) {
    html.style.removeProperty('background');
    html.style.removeProperty('background-color');
    html.style.removeProperty('background-image');
  }
  html.removeAttribute('bgcolor');
  html.removeAttribute('background');
  clear(body);
  // Walk the single full-width wrapper chain, clearing each element's background, and stop at
  // the first node with siblings or one narrower than the width available to it (the inner
  // card). The tolerance absorbs table border-spacing and rounding.
  var node = body;
  for (var guard = 0; node && guard < 32; guard++) {
    var child = soleChild(node);
    if (!child) return;
    var cs = window.getComputedStyle(node);
    var pad = (parseFloat(cs.paddingLeft) || 0) + (parseFloat(cs.paddingRight) || 0);
    var avail = node.getBoundingClientRect().width - pad;
    if (!(avail > 0)) return;
    if (!(child.getBoundingClientRect().width >= avail - Math.max(8, avail * 0.02))) return;
    clear(child);
    node = child;
  }
})();"#;
/// Reports the document height whenever layout changes it (resize, images, fonts), from an
/// isolated content world, so page scripts (which are off) are never involved.
const MEASURE_SCRIPT: &str = r#"(function () {
  var last = -1;
  function send() {
    var h = Math.ceil(document.documentElement.getBoundingClientRect().height);
    if (h !== last) { last = h; window.webkit.messageHandlers.mcHeight.postMessage(h); }
  }
  function later() { setTimeout(send, 0); }
  new ResizeObserver(send).observe(document.documentElement);
  if (document.body) new ResizeObserver(send).observe(document.body);
  document.addEventListener('load', later, true);
  window.addEventListener('load', later);
  if (document.fonts && document.fonts.ready) document.fonts.ready.then(later);
  send();
})();"#;

/// What every email view shares: one web process, one non-persistent data store, and the
/// remote-content rule list once it has compiled.
#[allow(deprecated)] // The pool is a no-op on current macOS (one shared web process); kept explicit.
pub struct Shared {
    mtm: MainThreadMarker,
    pool: Retained<WKProcessPool>,
    store: Retained<WKWebsiteDataStore>,
    rules: Rc<RefCell<Rules>>,
}

pub enum Rules {
    Idle,
    Loading,
    Ready(Retained<WKContentRuleList>),
    Failed,
}

#[allow(deprecated)]
impl Shared {
    pub fn new() -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        // SAFETY: main thread; plain constructors.
        let (pool, store) = unsafe {
            (WKProcessPool::new(mtm), WKWebsiteDataStore::nonPersistentDataStore(mtm))
        };
        Some(Self { mtm, pool, store, rules: Rc::new(RefCell::new(Rules::Idle)) })
    }

    /// The compiled rules, starting the (cached) compile on first use. `done` runs once the
    /// compile has finished, successfully or not.
    pub fn rules(&self, done: impl Fn() + 'static) -> Option<Retained<WKContentRuleList>> {
        match &*self.rules.borrow() {
            Rules::Ready(list) => return Some(list.clone()),
            Rules::Failed | Rules::Loading => return None,
            Rules::Idle => {}
        }
        *self.rules.borrow_mut() = Rules::Loading;
        let finish = {
            let rules = self.rules.clone();
            Rc::new(move |list: *mut WKContentRuleList| {
                // SAFETY: WebKit hands over a valid list or null.
                *rules.borrow_mut() = match unsafe { Retained::retain(list) } {
                    Some(list) => Rules::Ready(list),
                    None => Rules::Failed,
                };
                done();
            })
        };
        // SAFETY: main thread; the blocks only touch `Rc` state on the main thread.
        unsafe {
            let Some(store) = WKContentRuleListStore::defaultStore(self.mtm) else {
                finish(std::ptr::null_mut());
                return None;
            };
            let id = NSString::from_str(RULES_ID);
            let compile_store = store.clone();
            let (compile_id, compile_finish) = (id.clone(), finish.clone());
            let lookup = RcBlock::new(move |list: *mut WKContentRuleList, _: *mut NSError| {
                if !list.is_null() {
                    return finish(list);
                }
                let finish = compile_finish.clone();
                let compiled =
                    RcBlock::new(move |list: *mut WKContentRuleList, _: *mut NSError| finish(list));
                compile_store
                    .compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
                        Some(&compile_id),
                        Some(&NSString::from_str(RULES)),
                        Some(&compiled),
                    );
            });
            store.lookUpContentRuleListForIdentifier_completionHandler(Some(&id), Some(&lookup));
        }
        None
    }
}

/// One email's native view: a `WKWebView` inside a clip container, both children of GPUI's
/// content view.
pub struct Native {
    /// Owns the wry view; dropping it hides and (with the last handle) destroys the web view.
    _view: gpui_wry::WebView,
    web: Retained<WryWebView>,
    clip: Retained<ClipView>,
    parent: Retained<NSView>,
    content: Retained<WKUserContentController>,
    world: Retained<WKContentWorld>,
    parent_flipped: bool,
    shown: Cell<bool>,
    /// Last applied `[clip x, y, w, h, view x, y, w, h, parent height]`.
    frame: Cell<[f32; 9]>,
}

/// Why a native view could not be built.
pub enum Unavailable {
    /// No platform window (the headless test platform).
    Headless,
    Failed,
}

pub struct Config<'a> {
    pub html: &'a str,
    pub rules: Option<Retained<WKContentRuleList>>,
    /// A navigation started (the old document's height no longer counts).
    pub on_started: Box<dyn Fn()>,
    /// The page has finished loading.
    pub on_finished: Box<dyn Fn()>,
    /// The laid-out document height changed.
    pub on_height: Box<dyn Fn(f64)>,
}

/// Whether `window` is backed by a real `NSWindow` (the headless test platform is not).
pub fn available(window: &Window) -> bool {
    HasWindowHandle::window_handle(window).is_ok()
}

/// Open `url` in the system browser, if it is a link we hand over.
pub fn open_external(url: &str) {
    let allowed = url.split_once(':').is_some_and(|(scheme, _)| {
        ["http", "https", "mailto"].iter().any(|s| scheme.eq_ignore_ascii_case(s))
    });
    if !allowed {
        return;
    }
    if let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) {
        // The navigation delegate runs on the main thread, which `sharedWorkspace` needs.
        NSWorkspace::sharedWorkspace().openURL(&url);
    }
}

impl Native {
    pub fn new(
        window: &mut Window,
        cx: &mut App,
        shared: &Shared,
        config: Config<'_>,
    ) -> Result<Self, Unavailable> {
        let handle = HasWindowHandle::window_handle(&*window).map_err(|_| Unavailable::Headless)?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return Err(Unavailable::Headless);
        };
        // SAFETY: GPUI's content view outlives this call; we retain it.
        let parent: Retained<NSView> = unsafe { Retained::retain(handle.ns_view.as_ptr().cast()) }
            .ok_or(Unavailable::Failed)?;
        let mtm = shared.mtm;

        // SAFETY: main thread; configuration objects are ours until handed to wry.
        let (configuration, content) = unsafe {
            let configuration = WKWebViewConfiguration::new(mtm);
            #[allow(deprecated)]
            configuration.setProcessPool(&shared.pool);
            configuration.setWebsiteDataStore(&shared.store);
            let preferences = WKWebpagePreferences::new(mtm);
            preferences.setAllowsContentJavaScript(false);
            configuration.setDefaultWebpagePreferences(Some(&preferences));
            let content = configuration.userContentController();
            if let Some(rules) = &config.rules {
                content.addContentRuleList(rules);
            }
            (configuration, content)
        };
        // The background neutraliser, the measuring script and its handler live in their own
        // content world, installed before the first navigation.
        let (world, handler) = {
            let world = unsafe { WKContentWorld::worldWithName(&NSString::from_str(WORLD_NAME), mtm) };
            let handler: Retained<HeightHandler> = unsafe {
                let this = HeightHandler::alloc(mtm).set_ivars(HeightIvars { on_height: config.on_height });
                msg_send![super(this), init]
            };
            // SAFETY: main thread; the controller retains both until we remove them in `Drop`.
            unsafe {
                for source in [NEUTRALIZE_SCRIPT, MEASURE_SCRIPT] {
                    let script = WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
                        WKUserScript::alloc(mtm),
                        &NSString::from_str(source),
                        WKUserScriptInjectionTime::AtDocumentEnd,
                        true,
                        &world,
                    );
                    content.addUserScript(&script);
                }
                content.addScriptMessageHandler_contentWorld_name(
                    ProtocolObject::from_ref(&*handler),
                    &world,
                    &NSString::from_str(HANDLER_NAME),
                );
            }
            (world, handler)
        };
        drop(handler);

        let (on_started, on_finished) = (config.on_started, config.on_finished);
        let page_load = move |event: PageLoadEvent, _: String| match event {
            PageLoadEvent::Started => on_started(),
            PageLoadEvent::Finished => on_finished(),
        };
        let view = WebViewBuilder::new()
            .with_webview_configuration(configuration)
            .with_html(config.html)
            .with_focused(false)
            .with_accept_first_mouse(true)
            .with_allow_link_preview(false)
            // The document itself is `about:blank`; any other navigation is a link click, which
            // belongs to the system browser.
            .with_navigation_handler(|url| {
                if url.starts_with("about:blank") {
                    return true;
                }
                open_external(&url);
                false
            })
            .with_new_window_req_handler(|url, _| {
                open_external(&url);
                NewWindowResponse::Deny
            })
            .with_download_started_handler(|_, _| false)
            .with_on_page_load_handler(page_load)
            .build_as_child(window)
            .map_err(|_| Unavailable::Failed)?;

        let web = view.webview();
        // Out of the window first: WebKit and AppKit key-value-observe the view while it is in
        // one, and an isa swap under a live observer breaks that bookkeeping.
        web.removeFromSuperview();
        // SAFETY: `KeylessWebView` adds no ivars to its superclass, so swapping the class of a
        // live `WryWebView` is layout-compatible.
        unsafe {
            objc2::ffi::object_setClass(
                Retained::as_ptr(&web).cast_mut().cast::<AnyObject>(),
                KeylessWebView::class(),
            );
        }

        // SAFETY: main thread; `addSubview` re-parents wry's view into the clip container.
        let (clip, parent_flipped) = unsafe {
            let clip: Retained<ClipView> = msg_send![ClipView::alloc(mtm), init];
            clip.setWantsLayer(true);
            let layer: Option<Retained<AnyObject>> = msg_send![&*clip, layer];
            if let Some(layer) = layer {
                let _: () = msg_send![&*layer, setMasksToBounds: true];
            }
            clip.setHidden(true);
            clip.addSubview(&web);
            parent.addSubview(&clip);
            (clip, parent.isFlipped())
        };
        let view = gpui_wry::WebView::new(view, window, cx);
        Ok(Self {
            _view: view,
            web,
            clip,
            parent,
            content,
            world,
            parent_flipped,
            shown: Cell::new(false),
            frame: Cell::new([f32::NAN; 9]),
        })
    }

    pub fn web(&self) -> &WKWebView {
        &self.web
    }

    /// Swap the document, applying the remote-content policy first.
    pub fn load(&self, html: &str, rules: Option<&Retained<WKContentRuleList>>) {
        // SAFETY: main thread.
        unsafe {
            self.content.removeAllContentRuleLists();
            if let Some(rules) = rules {
                self.content.addContentRuleList(rules);
            }
        }
        let _ = self._view.raw().load_html(html);
    }

    /// Place the clip container at `clip` and the page inside it at `view` (`[x, y, w, h]`,
    /// logical pixels, window coordinates for the clip and clip coordinates for the view). Does
    /// nothing when nothing moved.
    pub fn place(&self, clip: [f32; 4], view: [f32; 4]) {
        let parent_h = if self.parent_flipped {
            0.
        } else {
            self.parent.frame().size.height as f32
        };
        let key = [clip[0], clip[1], clip[2], clip[3], view[0], view[1], view[2], view[3], parent_h];
        if key == self.frame.get() {
            return;
        }
        self.frame.set(key);
        let top = if self.parent_flipped { clip[1] } else { parent_h - clip[1] - clip[3] };
        self.clip.setFrame(rect(clip[0], top, clip[2], clip[3]));
        self.web.setFrame(rect(view[0], view[1], view[2], view[3]));
    }

    /// Show or hide the whole view. A hidden view never keeps first responder.
    pub fn set_visible(&self, visible: bool) {
        if self.shown.replace(visible) == visible {
            return;
        }
        self.clip.setHidden(!visible);
        if !visible {
            self.release_first_responder();
        }
    }

    pub fn is_visible(&self) -> bool {
        self.shown.get()
    }

    fn release_first_responder(&self) {
        let Some(window) = self.web.window() else { return };
        if let Some(responder) = window.firstResponder() {
            let ours = Retained::as_ptr(&self.web).cast::<AnyObject>();
            let held = std::ptr::eq(Retained::as_ptr(&responder).cast::<AnyObject>(), ours);
            if held {
                window.makeFirstResponder(Some(&self.parent));
            }
        }
    }

    /// Backing scale of the window the view is in.
    pub fn scale(&self) -> f64 {
        self.web.window().map_or(2., |w| w.backingScaleFactor())
    }
}

impl Drop for Native {
    fn drop(&mut self) {
        // SAFETY: main thread; breaks the controller's retain on the height handler.
        unsafe {
            self.content
                .removeScriptMessageHandlerForName_contentWorld(&NSString::from_str(HANDLER_NAME), &self.world);
        }
        self.release_first_responder();
        self.clip.removeFromSuperview();
    }
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> NSRect {
    NSRect::new(NSPoint::new(f64::from(x), f64::from(y)), NSSize::new(f64::from(w), f64::from(h)))
}

/// Run `js` in the page; `done` gets the raw result object (null on error).
pub fn eval(web: &WKWebView, js: &str, done: impl Fn(*mut AnyObject) + 'static) {
    let block = RcBlock::new(move |value: *mut AnyObject, _: *mut NSError| done(value));
    // SAFETY: main thread; the block is copied by WebKit.
    unsafe { web.evaluateJavaScript_completionHandler(&NSString::from_str(js), Some(&block)) };
}

/// The result of [`eval`] as a string.
pub fn as_string(value: *mut AnyObject) -> Option<String> {
    // SAFETY: WebKit passes a valid object or null.
    let object = unsafe { value.as_ref() }?;
    object.downcast_ref::<NSString>().map(ToString::to_string)
}

/// Snapshot `rect` (`[x, y, w, h]`, page coordinates) at `scale`× resolution as TIFF bytes.
pub fn snapshot(web: &WKWebView, rect_: [f32; 4], scale: f64, done: impl Fn(Option<Vec<u8>>) + 'static) {
    let Some(mtm) = MainThreadMarker::new() else { return done(None) };
    // SAFETY: main thread; the block is copied by WebKit.
    unsafe {
        let config = WKSnapshotConfiguration::new(mtm);
        config.setRect(rect(rect_[0], rect_[1], rect_[2], rect_[3]));
        config.setSnapshotWidth(Some(&NSNumber::new_f64(f64::from(rect_[2]) * scale)));
        let block = RcBlock::new(move |image: *mut NSImage, _: *mut NSError| {
            let bytes = image
                .as_ref()
                .and_then(NSImage::TIFFRepresentation)
                .map(|data| data.to_vec());
            done(bytes);
        });
        web.takeSnapshotWithConfiguration_completionHandler(Some(&config), &block);
    }
}
