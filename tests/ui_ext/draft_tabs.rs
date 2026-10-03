//! Draft tabs in the strip: they carry a draft glyph in the same leading slot a thread's
//! avatar uses, and the "sender avatar in tabs" setting hides that slot for both.

use crate::harness::{harness, harness_with_prefs};
use gpui_kit::AssetSource as _;
use gpui_kit::TestAppContext;
use mail_classifier::app_settings::TAB_AVATARS;
use mail_classifier::contacts::ContactStore;
use mail_classifier::model::Mailbox;
use mail_classifier::prefs::Scope;
use std::rc::Rc;

/// With the setting on (the default), a draft tab shows the draft glyph — and the real
/// render path can actually paint it: the glyph's SVG is embedded in the app's asset source,
/// which is what the running app loads (the headless element tree alone would pass even if
/// the bytes were missing).
#[gpui_kit::gpui::test]
pub fn a_draft_tab_shows_the_draft_glyph_when_avatars_are_on(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.new_draft();
    assert!(
        h.has(("reader-tab-draft", 0usize)),
        "the draft tab's leading slot holds the draft glyph"
    );
    assert!(
        matches!(
            mail_classifier::app::icons::AppAssets.load("icons/file-pen.svg"),
            Ok(Some(bytes)) if !bytes.is_empty()
        ),
        "the draft glyph's SVG resolves through the app's asset source"
    );
}

/// With the setting off, the leading slot is gone for a draft tab exactly as it is for a
/// thread tab, so the two kinds of tab still line up.
#[gpui_kit::gpui::test]
pub fn a_draft_tab_hides_the_glyph_when_avatars_are_off(cx: &mut TestAppContext) {
    let store = Rc::new(ContactStore::open_in_memory().expect("in-memory prefs"));
    TAB_AVATARS.set(store.as_ref(), &Scope::Global, false);
    let mut h = harness_with_prefs(cx, Mailbox::load_default(), store);
    h.new_draft();
    assert!(
        !h.has(("reader-tab-draft", 0usize)),
        "the leading slot is hidden while the setting is off"
    );
}
