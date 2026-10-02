//! The macOS HTML pasteboard flavor, which GPUI's string clipboard omits.

use objc2_app_kit::{NSPasteboard, NSPasteboardTypeHTML, NSPasteboardTypeString};

/// Read the HTML corresponding to an already captured plain-text paste.
/// A clipboard change between the two reads must not substitute a different paste.
pub fn html_for_text(expected_text: &str) -> Option<String> {
    let pasteboard = NSPasteboard::generalPasteboard();
    // These are AppKit's immutable, process-lifetime pasteboard type constants.
    let (plain_type, html_type) = unsafe { (NSPasteboardTypeString, NSPasteboardTypeHTML) };
    let plain = pasteboard.stringForType(plain_type)?;
    if plain.to_string() != expected_text {
        return None;
    }
    pasteboard
        .stringForType(html_type)
        .map(|html| html.to_string())
}
