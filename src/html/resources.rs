//! Bounded remote raster-image loading for the offline HTML renderer.
//!
//! The renderer never uses Blitz's default network stack. Every sub-resource the
//! document asks for is routed through [`RemoteImages`], which applies a fixed
//! policy:
//!
//! * `data:` raster images (PNG/JPEG/GIF/WebP) are decoded in-process under byte
//!   and pixel budgets.
//! * `http(s)` raster images are fetched only when remote images are unblocked,
//!   and only against public hosts (SSRF guard), with bounded redirects, body
//!   size, decode dimensions and total wait.
//! * Everything else (`file:`, fonts, stylesheets, `data:` non-raster, SVG, …)
//!   is denied without any network access.
//!
//! Fetches run on a small bounded worker pool so a single render can resolve
//! several images concurrently without unbounded thread spawning. Fetched bytes
//! are validated (raster format + dimensions + pixel budget) before being handed
//! back to Blitz; a failed image yields empty bytes, so one unavailable remote
//! image never erases the whole message.

use std::collections::{HashMap, VecDeque};
use std::io::Cursor;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::Duration;

use blitz_traits::net::{Bytes, Method, NetHandler, NetProvider, Request};
use data_url::DataUrl;
use image::{ImageFormat, ImageReader};
use ureq::Error as UreqError;
use ureq::config::Config as UreqConfig;
use ureq::http::Uri;
use ureq::unversioned::resolver::{ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

use super::RemoteImageFetcher;
use super::{MAX_RENDER_HEIGHT, MAX_RENDER_WIDTH};

/// Largest encoded `data:` image payload accepted, in bytes.
const MAX_IMAGE_DATA_URL_BYTES: usize = 6 * 1024 * 1024;
/// Largest encoded remote image payload accepted, in bytes.
const MAX_REMOTE_IMAGE_BYTES: usize = 8 * 1024 * 1024;
/// Decoded pixel ceiling for a single image.
const MAX_IMAGE_PIXELS: usize = 16 * 1024 * 1024;
/// Decoded pixel ceiling across every image in one document.
const MAX_TOTAL_IMAGE_PIXELS: usize = 16 * 1024 * 1024;
/// Number of remote fetches allowed across one render.
const MAX_REMOTE_REQUESTS: usize = 128;
/// Concurrent fetch workers created (lazily) for one render.
const REMOTE_FETCH_WORKERS: usize = 4;
/// Queue depth between the render thread and the fetch workers.
///
/// Equal to the per-render request cap, so a request that passes the cap check can
/// always be enqueued without ever blocking the render thread.
const REMOTE_FETCH_QUEUE: usize = MAX_REMOTE_REQUESTS;
/// Per-request deadline enforced by the HTTP agent.
const REMOTE_REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
/// Follow up to this many redirects (each hop is re-checked against the SSRF guard).
const REMOTE_MAX_REDIRECTS: u32 = 3;
/// Total time a render waits for pending remote images before proceeding.
pub(crate) const REMOTE_RENDER_DEADLINE: Duration = Duration::from_secs(15);
/// Shared encoded-image cache eviction bounds.
const CACHE_MAX_ENTRIES: usize = 128;
const CACHE_MAX_BYTES: usize = 32 * 1024 * 1024;

/// File extensions that never name a raster image. Anything else (including a
/// missing extension, since many CDNs serve images from opaque paths) is treated
/// as a possible image and, when unblocked, gated on the response content type.
const NON_IMAGE_EXTENSIONS: &[&str] = &[
    "css", "js", "mjs", "cjs", "html", "htm", "xhtml", "xml", "json", "txt", "csv", "svg", "svgz",
    "pdf", "zip", "gz", "tar", "woff", "woff2", "ttf", "otf", "eot", "ico",
];

/// Per-render observable state shared with the fetch workers.
#[derive(Default)]
struct FetchState {
    total_pixels: AtomicUsize,
    embedded_rejected: AtomicBool,
    in_flight: AtomicUsize,
    issued: AtomicUsize,
    delivered: AtomicUsize,
    blocked_remote: AtomicUsize,
    loaded_remote: AtomicUsize,
    failed_remote: AtomicUsize,
    /// Set when the renderer/document is dropped; workers stop fetching queued jobs.
    cancelled: AtomicBool,
}

/// A validated encoded image held in the shared cache.
///
/// The payload is stored as [`Bytes`] so cloning it for the cache and the Blitz
/// handler is a cheap refcount bump, never a multi-megabyte copy.
#[derive(Clone)]
struct CachedImage {
    bytes: Bytes,
    pixels: usize,
}

#[derive(Default)]
struct CacheState {
    map: HashMap<String, CachedImage>,
    order: VecDeque<String>,
    bytes: usize,
}

#[derive(Default)]
struct ImageCache {
    state: Mutex<CacheState>,
}

impl ImageCache {
    fn get(&self, url: &str) -> Option<CachedImage> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.map.get(url).cloned()
    }

    fn put(&self, url: String, image: CachedImage) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if image.bytes.len() > CACHE_MAX_BYTES {
            return;
        }
        if let Some(previous) = state.map.remove(&url) {
            state.bytes = state.bytes.saturating_sub(previous.bytes.len());
            state.order.retain(|key| key != &url);
        }
        state.bytes = state.bytes.saturating_add(image.bytes.len());
        state.map.insert(url.clone(), image);
        state.order.push_back(url);
        while (state.bytes > CACHE_MAX_BYTES || state.map.len() > CACHE_MAX_ENTRIES)
            && state.order.len() > 1
        {
            let Some(oldest) = state.order.pop_front() else {
                break;
            };
            if let Some(evicted) = state.map.remove(&oldest) {
                state.bytes = state.bytes.saturating_sub(evicted.bytes.len());
            }
        }
    }
}

fn image_cache() -> &'static ImageCache {
    static CACHE: LazyLock<ImageCache> = LazyLock::new(ImageCache::default);
    &CACHE
}

/// A queued remote fetch.
struct FetchJob {
    url: String,
    handler: Box<dyn NetHandler>,
}

struct PoolHandle {
    sender: SyncSender<FetchJob>,
}

/// The complete offline resource provider for a single render.
pub(crate) struct RemoteImages {
    block_remote_images: bool,
    transport: RemoteImageFetcher,
    state: Arc<FetchState>,
    pool: Mutex<Option<PoolHandle>>,
}

impl RemoteImages {
    pub(crate) fn new(block_remote_images: bool, transport: RemoteImageFetcher) -> Self {
        Self {
            block_remote_images,
            transport,
            state: Arc::new(FetchState::default()),
            pool: Mutex::new(None),
        }
    }

    /// Number of remote fetches started but not yet delivered.
    pub(crate) fn in_flight(&self) -> usize {
        self.state.in_flight.load(Ordering::Relaxed)
    }

    /// Number of sub-resource responses delivered to the document so far.
    pub(crate) fn delivered(&self) -> usize {
        self.state.delivered.load(Ordering::Relaxed)
    }

    /// Remote HTTP(S) images denied because blocking is enabled.
    pub(crate) fn blocked_remote(&self) -> usize {
        self.state.blocked_remote.load(Ordering::Relaxed)
    }

    /// Whether an embedded raster image was malformed or over budget.
    pub(crate) fn embedded_rejected(&self) -> bool {
        self.state.embedded_rejected.load(Ordering::Relaxed)
    }

    fn deliver(&self, url: String, handler: Box<dyn NetHandler>, bytes: Bytes) {
        self.state.delivered.fetch_add(1, Ordering::Relaxed);
        handler.bytes(url, bytes);
    }

    fn sender(&self) -> SyncSender<FetchJob> {
        let mut guard = self.pool.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(pool) = guard.as_ref() {
            return pool.sender.clone();
        }
        let (sender, receiver) = sync_channel::<FetchJob>(REMOTE_FETCH_QUEUE);
        let receiver = Arc::new(Mutex::new(receiver));
        for _ in 0..REMOTE_FETCH_WORKERS {
            let receiver = Arc::clone(&receiver);
            let state = Arc::clone(&self.state);
            let transport = Arc::clone(&self.transport);
            let _ = thread::Builder::new()
                .name("mail-image-fetch".to_owned())
                .spawn(move || worker_loop(receiver, state, transport));
        }
        *guard = Some(PoolHandle {
            sender: sender.clone(),
        });
        sender
    }

    fn enqueue(&self, url: String, handler: Box<dyn NetHandler>) {
        let reserved =
            self.state
                .issued
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |issued| {
                    (issued < MAX_REMOTE_REQUESTS).then_some(issued + 1)
                });
        if reserved.is_err() {
            self.deliver(url, handler, Bytes::new());
            return;
        }
        self.state.in_flight.fetch_add(1, Ordering::Relaxed);
        let job = FetchJob { url, handler };
        // The queue's capacity equals the request cap, so this never blocks.
        match self.sender().send(job) {
            Ok(()) => {}
            Err(error) => {
                self.state.in_flight.fetch_sub(1, Ordering::Relaxed);
                let job = error.0;
                self.deliver(job.url, job.handler, Bytes::new());
            }
        }
    }
}

impl Drop for RemoteImages {
    fn drop(&mut self) {
        // The document/render is gone: workers must abandon queued fetches instead
        // of continuing to contact senders (each queued job would otherwise still
        // start an up-to-8s request).
        self.state.cancelled.store(true, Ordering::Relaxed);
    }
}

impl NetProvider for RemoteImages {
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url.as_str().to_owned();

        if request.method != Method::GET {
            self.deliver(url, handler, Bytes::new());
            return;
        }

        // Embedded images are decoded in-process; every other `data:` payload is
        // refused (no sender-authored stylesheets) without touching the network.
        if url.starts_with("data:") {
            let bytes = if url.starts_with("data:image/") && url.len() <= MAX_IMAGE_DATA_URL_BYTES {
                match decode_embedded_image(&url, &self.state.total_pixels) {
                    Ok(bytes) => Bytes::from(bytes),
                    Err(_) => {
                        self.state.embedded_rejected.store(true, Ordering::Relaxed);
                        Bytes::new()
                    }
                }
            } else {
                Bytes::new()
            };
            self.deliver(url, handler, bytes);
            return;
        }

        let scheme = request.url.scheme();
        if (scheme != "http" && scheme != "https") || !is_remote_image_url(&request.url) {
            self.deliver(url, handler, Bytes::new());
            return;
        }

        if self.block_remote_images {
            self.state.blocked_remote.fetch_add(1, Ordering::Relaxed);
            self.deliver(url, handler, Bytes::new());
            return;
        }

        if let Some(cached) = image_cache().get(&url) {
            let bytes = if reserve_pixels(cached.pixels, &self.state.total_pixels) {
                self.state.loaded_remote.fetch_add(1, Ordering::Relaxed);
                cached.bytes
            } else {
                Bytes::new()
            };
            self.deliver(url, handler, bytes);
            return;
        }

        self.enqueue(url, handler);
    }
}

fn worker_loop(
    receiver: Arc<Mutex<Receiver<FetchJob>>>,
    state: Arc<FetchState>,
    transport: RemoteImageFetcher,
) {
    loop {
        let job = {
            let guard = receiver.lock().unwrap_or_else(|error| error.into_inner());
            guard.recv()
        };
        let Ok(job) = job else {
            return;
        };
        // The render finished (or was dropped); never start queued work the
        // document can no longer consume.
        if state.cancelled.load(Ordering::Relaxed) {
            return;
        }
        let payload = match transport(&job.url) {
            Ok(bytes) => match validate_remote_image(&bytes) {
                Ok(pixels) => {
                    let payload = Bytes::from(bytes);
                    image_cache().put(
                        job.url.clone(),
                        CachedImage {
                            bytes: payload.clone(),
                            pixels,
                        },
                    );
                    if reserve_pixels(pixels, &state.total_pixels) {
                        state.loaded_remote.fetch_add(1, Ordering::Relaxed);
                        payload
                    } else {
                        state.failed_remote.fetch_add(1, Ordering::Relaxed);
                        Bytes::new()
                    }
                }
                Err(_) => {
                    state.failed_remote.fetch_add(1, Ordering::Relaxed);
                    Bytes::new()
                }
            },
            Err(_) => {
                state.failed_remote.fetch_add(1, Ordering::Relaxed);
                Bytes::new()
            }
        };
        // Deliver before clearing in-flight: an observer that sees zero in-flight
        // work is guaranteed the response is already queued in the document channel.
        state.delivered.fetch_add(1, Ordering::Relaxed);
        job.handler.bytes(job.url, payload);
        state.in_flight.fetch_sub(1, Ordering::Relaxed);
    }
}

fn reserve_pixels(pixels: usize, total: &AtomicUsize) -> bool {
    total
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
            used.checked_add(pixels)
                .filter(|next| *next <= MAX_TOTAL_IMAGE_PIXELS)
        })
        .is_ok()
}

fn is_raster_format(format: ImageFormat) -> bool {
    matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP
    )
}

/// Pull the lower-cased extension of the URL's final path segment.
fn url_extension(url: &url::Url) -> Option<String> {
    let last = url.path().rsplit('/').next().unwrap_or_default();
    let (_, extension) = last.rsplit_once('.')?;
    if extension.is_empty() || extension.len() > 8 {
        return None;
    }
    Some(extension.to_ascii_lowercase())
}

/// Whether a URL could name a raster image. Used only to skip requests we know
/// cannot be images (stylesheets, fonts, documents); the response content type
/// is still verified before any bytes are used.
fn is_remote_image_url(url: &url::Url) -> bool {
    match url_extension(url) {
        Some(extension) => !NON_IMAGE_EXTENSIONS.contains(&extension.as_str()),
        None => true,
    }
}

/// Validate fetched bytes as a bounded raster image and return its pixel count.
fn validate_remote_image(bytes: &[u8]) -> Result<usize, String> {
    if bytes.is_empty() {
        return Err("empty image response".to_owned());
    }
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    let format = reader
        .format()
        .ok_or_else(|| "unrecognized image format".to_owned())?;
    if !is_raster_format(format) {
        return Err("unsupported image format".to_owned());
    }
    let (width, height) = reader
        .into_dimensions()
        .map_err(|error| error.to_string())?;
    if width > MAX_RENDER_WIDTH || height > MAX_RENDER_HEIGHT {
        return Err("image dimensions exceed limit".to_owned());
    }
    (width as usize)
        .checked_mul(height as usize)
        .filter(|pixels| *pixels > 0 && *pixels <= MAX_IMAGE_PIXELS)
        .ok_or_else(|| "image dimensions exceed limit".to_owned())
}

fn decode_embedded_image(url: &str, total_pixels: &AtomicUsize) -> Result<Vec<u8>, String> {
    let data_url = DataUrl::process(url).map_err(|error| error.to_string())?;
    let mime = data_url.mime_type();
    let format = match (mime.type_.as_str(), mime.subtype.as_str()) {
        ("image", "png") => ImageFormat::Png,
        ("image", "jpeg") => ImageFormat::Jpeg,
        ("image", "gif") => ImageFormat::Gif,
        ("image", "webp") => ImageFormat::WebP,
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
    if !reserve_pixels(pixels, total_pixels) {
        return Err("total embedded image pixel budget exceeded".to_owned());
    }

    Ok(encoded)
}

/// The production transport: bounded `ureq` fetch of a single raster image.
pub(crate) fn default_fetcher() -> RemoteImageFetcher {
    Arc::new(|url: &str| fetch_remote_image(url))
}

fn agent() -> &'static ureq::Agent {
    static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(REMOTE_REQUEST_TIMEOUT))
            .max_redirects(REMOTE_MAX_REDIRECTS)
            .user_agent("mail-classifier/0.1")
            .accept("image/png,image/jpeg,image/gif,image/webp")
            .build();
        ureq::Agent::with_parts(config, DefaultConnector::default(), PublicHostResolver)
    });
    &AGENT
}

fn fetch_remote_image(url: &str) -> Result<Vec<u8>, String> {
    let mut response = agent().get(url).call().map_err(|error| error.to_string())?;

    // A sender-controlled content type cannot make us read a stylesheet, font or
    // document body: only image responses (or opaque binary types, which are
    // still magic-byte validated) have their body read.
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if content_type.starts_with("text/")
        || content_type.starts_with("font/")
        || content_type.contains("javascript")
        || content_type.contains("svg")
        || content_type.contains("json")
        || content_type.contains("xml")
    {
        return Err(format!("refusing non-image content type {content_type}"));
    }

    let limit = (MAX_REMOTE_IMAGE_BYTES + 1) as u64;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_REMOTE_IMAGE_BYTES {
        return Err("remote image exceeds byte limit".to_owned());
    }
    Ok(bytes)
}

/// DNS resolver that refuses any host resolving to a non-public address.
///
/// It is plugged into the HTTP agent so the addresses actually connected to are
/// the ones that were checked, closing the DNS-rebinding window. If *any*
/// resolved address is private, loopback, link-local or otherwise reserved, the
/// whole name is rejected.
#[derive(Debug)]
struct PublicHostResolver;

impl Resolver for PublicHostResolver {
    fn resolve(
        &self,
        uri: &Uri,
        _config: &UreqConfig,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, UreqError> {
        let scheme = uri.scheme_str().unwrap_or_default();
        let default_port = match scheme {
            "http" => 80,
            "https" => 443,
            other => return Err(UreqError::BadUri(format!("unsupported scheme {other}"))),
        };
        let host = uri.host().ok_or(UreqError::HostNotFound)?;
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let port = uri.port_u16().unwrap_or(default_port);

        let addresses = resolve_addresses(host, port, &timeout)?;
        let mut allowed = self.empty();
        for address in addresses {
            if !is_public_ip(address.ip()) {
                return Err(UreqError::HostNotFound);
            }
            let _ = allowed.try_push(address);
        }
        if allowed.is_empty() {
            return Err(UreqError::HostNotFound);
        }
        Ok(allowed)
    }
}

fn resolve_addresses(
    host: &str,
    port: u16,
    timeout: &NextTimeout,
) -> Result<Vec<SocketAddr>, UreqError> {
    if timeout.after.is_not_happening() {
        return (host, port)
            .to_socket_addrs()
            .map(|addresses| addresses.collect())
            .map_err(|_| UreqError::HostNotFound);
    }
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let owned = host.to_owned();
    let _ = thread::Builder::new()
        .name("mail-image-dns".to_owned())
        .spawn(move || {
            let _ = sender.send(
                (owned.as_str(), port)
                    .to_socket_addrs()
                    .map(|a| a.collect()),
            );
        });
    match receiver.recv_timeout(*timeout.after) {
        Ok(Ok(addresses)) => Ok(addresses),
        Ok(Err(_)) => Err(UreqError::HostNotFound),
        Err(RecvTimeoutError::Timeout) => Err(UreqError::Timeout(timeout.reason)),
        Err(RecvTimeoutError::Disconnected) => Err(UreqError::HostNotFound),
    }
}

/// Whether an IP is a globally routable public address, never an internal one.
fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
        || octets[0] == 0
        || octets[0] >= 240
        // Carrier-grade NAT (100.64.0.0/10).
        || (octets[0] == 100 && (octets[1] & 0xc0) == 64)
        // IETF protocol assignments (192.0.0.0/24).
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        // Benchmarking (198.18.0.0/15).
        || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19)))
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return is_public_v4(mapped);
    }
    let segments = ip.segments();
    // Only native global-unicast space: this also excludes IPv4-compatible
    // addresses and NAT64 translation prefixes that could reach private IPv4.
    (segments[0] & 0xe000) == 0x2000
        // IETF special-purpose space (including Teredo and benchmarking).
        && !(segments[0] == 0x2001 && segments[1] < 0x0200)
        // Documentation prefixes and 6to4 tunnels are not image CDN addresses.
        && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
        && segments[0] != 0x2002
        && !(segments[0] == 0x3fff && (segments[1] & 0xf000) == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use image::ImageEncoder;
    use parking_lot::Mutex as ParkingMutex;

    fn png_bytes(size: u32) -> Vec<u8> {
        let mut png = Vec::new();
        let pixels = vec![0u8; (size * size * 4) as usize];
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&pixels, size, size, image::ExtendedColorType::Rgba8)
            .unwrap();
        png
    }

    #[test]
    fn classifies_image_and_non_image_urls() {
        let image = url::Url::parse("https://cdn.example.test/a/b.png?v=2").unwrap();
        let opaque = url::Url::parse("https://track.example.test/o/abc123").unwrap();
        let css = url::Url::parse("https://cdn.example.test/site.css").unwrap();
        let font = url::Url::parse("https://cdn.example.test/f.woff2").unwrap();
        let svg = url::Url::parse("https://cdn.example.test/logo.svg").unwrap();
        assert!(is_remote_image_url(&image));
        assert!(is_remote_image_url(&opaque));
        assert!(!is_remote_image_url(&css));
        assert!(!is_remote_image_url(&font));
        assert!(!is_remote_image_url(&svg));
    }

    #[test]
    fn rejects_private_and_reserved_addresses() {
        for ip in [
            "127.0.0.1",
            "10.0.0.5",
            "192.168.1.1",
            "172.16.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "192.0.0.1",
            "198.18.0.1",
            "::1",
            "fe80::1",
            "fc00::1",
            "::ffff:127.0.0.1",
            "2001:db8::1",
            "::10.0.0.1",
            "64:ff9b::a00:1",
            "2001:2::1",
            "3fff::1",
        ] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip} should be denied");
        }
        for ip in ["8.8.8.8", "1.1.1.1", "23.20.239.12", "2606:4700::1111"] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip} should be allowed");
        }
    }

    #[test]
    fn resolver_rejects_private_hosts_and_accepts_public_literals() {
        use ureq::unversioned::transport::time::Duration as UreqDuration;
        let resolver = PublicHostResolver;
        let timeout = NextTimeout {
            after: UreqDuration::NotHappening,
            reason: ureq::Timeout::Resolve,
        };
        let config = ureq::Agent::config_builder().build();
        for denied in ["http://127.0.0.1/x.png", "http://[::1]/x.png"] {
            let uri: Uri = denied.parse().unwrap();
            assert!(
                resolver.resolve(&uri, &config, timeout).is_err(),
                "{denied} should be denied"
            );
        }
        let uri: Uri = "http://8.8.8.8/x.png".parse().unwrap();
        assert!(resolver.resolve(&uri, &config, timeout).is_ok());
    }

    #[test]
    fn validates_raster_bounds() {
        assert!(validate_remote_image(&png_bytes(1)).is_ok());
        assert!(validate_remote_image(b"<html>not an image</html>").is_err());
        assert!(validate_remote_image(b"").is_err());
    }

    #[test]
    fn injected_transport_loads_and_blocking_denies() {
        // A fake transport proves the path end-to-end without touching the network.
        let png = png_bytes(2);
        let requests = Arc::new(ParkingMutex::new(Vec::<String>::new()));
        let seen = Arc::clone(&requests);
        let transport: RemoteImageFetcher = Arc::new(move |url: &str| {
            seen.lock().push(url.to_owned());
            Ok(png.clone())
        });

        #[derive(Clone)]
        struct Capture(Arc<ParkingMutex<Option<Vec<u8>>>>);
        impl NetHandler for Capture {
            fn bytes(self: Box<Self>, _resolved_url: String, bytes: Bytes) {
                *self.0.lock() = Some(bytes.to_vec());
            }
        }

        let deliver = |provider: &RemoteImages, url: &str| {
            let captured = Arc::new(ParkingMutex::new(None));
            provider.fetch(
                1,
                Request::get(url::Url::parse(url).unwrap()),
                Box::new(Capture(captured.clone())),
            );
            captured
        };

        // Blocked: never consults the transport, counts the image.
        let blocked = RemoteImages::new(true, Arc::clone(&transport));
        let captured = deliver(&blocked, "https://cdn.example.test/injected-hero.png");
        assert_eq!(*captured.lock(), Some(Vec::new()));
        assert_eq!(blocked.blocked_remote(), 1);
        assert!(requests.lock().is_empty(), "blocked mode must not fetch");

        // Allowed: queues a bounded background fetch that reaches the transport.
        let allowed = RemoteImages::new(false, Arc::clone(&transport));
        deliver(&allowed, "https://cdn.example.test/injected-hero.png");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while allowed.in_flight() > 0 && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(allowed.in_flight(), 0, "fetch did not complete");
        assert_eq!(requests.lock().len(), 1);

        // A stylesheet URL is denied without consulting the transport.
        deliver(&allowed, "https://cdn.example.test/site.css");
        assert_eq!(requests.lock().len(), 1);
    }

    #[test]
    fn embedded_images_share_the_pixel_budget() {
        let png = png_bytes(4);
        let data_url = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&png)
        );
        let total = AtomicUsize::new(0);
        assert!(decode_embedded_image(&data_url, &total).is_ok());
        assert_eq!(total.load(Ordering::Relaxed), 16);
        assert!(decode_embedded_image("data:image/svg+xml,%3Csvg%3E%3C/svg%3E", &total).is_err());
    }
}
