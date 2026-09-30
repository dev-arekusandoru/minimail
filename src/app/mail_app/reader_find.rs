//! Find in the active reader tab: the query state per tab (`crate::find`), landing on a match
//! (expanding and revealing what hides it), and getting it into view. View state only: no undo
//! steps, dropped with the tab.

use super::reader::FindReveal;
use super::*;
use crate::find::{self, Find, Match, Options, Segment};
use gpui_kit::component::input::{InputEvent, InputState};
use std::ops::Range;

/// One tab's find bar.
pub(super) struct FindTab {
    pub find: Find,
    pub input: Entity<InputState>,
    /// Matches of the current query over the tab's thread; refreshed every frame.
    pub matches: Vec<Match>,
    /// The query is a regex that does not compile.
    pub invalid: bool,
    _sub: Subscription,
}

/// The painted text holding the current match, and the frame (`find_gen`) it was built in.
pub(super) struct FindLayout {
    key: (MessageId, Segment),
    layout: TextLayout,
    frame: u64,
}

impl MailApp {
    /// The find bar is open in the active tab.
    pub fn find_open(&self) -> bool {
        self.find_thread().is_some()
    }

    /// `(current, total)` of the active tab's find, `current` 1-based and 0 without matches.
    pub fn find_count(&self) -> Option<(usize, usize)> {
        let ft = self.finds.get(&self.find_thread()?)?;
        let total = ft.matches.len();
        Some((if total == 0 { 0 } else { ft.find.current(total) + 1 }, total))
    }

    /// The active tab's find query is an invalid regex.
    pub fn find_invalid(&self) -> bool {
        self.find_thread().and_then(|t| self.finds.get(&t)).is_some_and(|ft| ft.invalid)
    }

    /// The active tab's find options.
    pub fn find_options(&self) -> Option<Options> {
        self.finds.get(&self.find_thread()?).map(|ft| ft.find.options)
    }

    /// Thread of the active tab when its find bar is open.
    fn find_thread(&self) -> Option<u32> {
        let thread = self.tabs.active()?.thread;
        self.finds.contains_key(&thread).then_some(thread)
    }

    /// The current match of `thread`'s find.
    pub(super) fn find_current(&self, thread: u32) -> Option<&Match> {
        let ft = self.finds.get(&thread)?;
        ft.matches.get(ft.find.current(ft.matches.len()))
    }

    /// `cmd-f`: open the bar in the active tab (or focus it again).
    pub(super) fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs_locked() {
            return;
        }
        let Some(thread) = self.tabs.active().map(|t| t.thread) else { return };
        if let std::collections::hash_map::Entry::Vacant(slot) = self.finds.entry(thread) {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in thread"));
            let sub = cx.subscribe_in(&input, window, move |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = input.read(cx).value().to_string();
                    this.set_find_query(thread, &text, cx);
                }
            });
            slot.insert(FindTab { find: Find::default(), input, matches: Vec::new(), invalid: false, _sub: sub });
        }
        if let Some(ft) = self.finds.get(&thread) {
            window.focus(&ft.input.focus_handle(cx), cx);
        }
        cx.notify();
    }

    /// `esc`: close the bar, drop the query and its highlights, and give focus back.
    pub(super) fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(thread) = self.find_thread() {
            self.drop_find(thread);
        }
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// Forget a thread's find (its tab closed, or the bar was closed).
    pub(super) fn drop_find(&mut self, thread: u32) {
        self.finds.remove(&thread);
        if let Some(pane) = self.reader_panes.borrow_mut().get_mut(&thread) {
            pane.find_reveal = FindReveal::Idle;
        }
    }

    fn set_find_query(&mut self, thread: u32, text: &str, cx: &mut Context<Self>) {
        let Some(ft) = self.finds.get_mut(&thread) else { return };
        ft.find.set_query(text);
        self.recompute_find(thread);
        self.land_find(thread);
        cx.notify();
    }

    /// `enter` / `cmd-g` and their shift forms: the next or previous match, wrapping.
    pub(super) fn find_step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.tabs_locked() {
            return;
        }
        let Some(thread) = self.find_thread() else { return };
        self.recompute_find(thread);
        if let Some(ft) = self.finds.get_mut(&thread) {
            let total = ft.matches.len();
            ft.find.step(total, delta);
        }
        self.land_find(thread);
        cx.notify();
    }

    /// Flip one option (`alt-c`, `alt-w`, `alt-r`) and search again from the first match.
    pub(super) fn toggle_find_option(
        &mut self,
        flip: fn(&mut Options),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(thread) = self.find_thread() else { return };
        if let Some(ft) = self.finds.get_mut(&thread) {
            let mut options = ft.find.options;
            flip(&mut options);
            ft.find.set_options(options);
            window.focus(&ft.input.focus_handle(cx), cx);
        }
        self.recompute_find(thread);
        self.land_find(thread);
        cx.notify();
    }

    /// Search `thread` again: the thread's text or the opened message may have changed.
    fn recompute_find(&mut self, thread: u32) {
        let Some(opened) = self.tabs.tab_for(thread).map(|t| t.msg) else { return };
        let Some(ft) = self.finds.get_mut(&thread) else { return };
        match find::find_in_thread(self.mailbox.messages(), thread, opened, &ft.find.query, ft.find.options) {
            Ok(matches) => {
                ft.matches = matches;
                ft.invalid = false;
            }
            Err(_) => {
                ft.matches.clear();
                ft.invalid = true;
            }
        }
    }

    /// Keep the active tab's matches current; called every frame.
    pub(super) fn refresh_find(&mut self) {
        if let Some(thread) = self.find_thread() {
            self.recompute_find(thread);
        }
    }

    /// Make the current match visible: expand its collapsed message (which pins the tab) or
    /// reveal its folded quote, then have the reader scroll to it.
    fn land_find(&mut self, thread: u32) {
        let Some(m) = self.find_current(thread).cloned() else { return };
        let opened = self.tabs.tab_for(thread).map(|t| t.msg);
        if m.segment != Segment::Subject && Some(m.msg) != opened && !self.reader.is_expanded(thread, m.msg) {
            self.reader.expand(thread, m.msg);
            self.tabs.pin(thread);
        }
        if m.segment == Segment::Quoted {
            self.reader.show_quoted(thread, m.msg);
        }
        self.reader_panes.borrow_mut().entry(thread).or_default().find_reveal = FindReveal::Message;
    }

    /// Highlights of one displayed string: every match of `thread`'s find in (`msg`, `segment`),
    /// the current one in the accent colour.
    pub(super) fn find_highlights(
        &self,
        thread: u32,
        msg: MessageId,
        segment: Segment,
        cx: &App,
    ) -> Option<Vec<(Range<usize>, HighlightStyle)>> {
        if self.in_session() {
            return None;
        }
        let ft = self.finds.get(&thread)?;
        let current = ft.find.current(ft.matches.len());
        let spans = find::highlights(&ft.matches, current, msg, segment);
        if spans.is_empty() {
            return None;
        }
        let t = theme::active(cx);
        let others = HighlightStyle { background_color: Some(t.warning.opacity(0.35)), ..Default::default() };
        let this = HighlightStyle {
            background_color: Some(t.accent),
            color: Some(t.on_accent),
            ..Default::default()
        };
        Some(spans.into_iter().map(|(r, cur)| (r, if cur { this } else { others })).collect())
    }

    /// `text` as a reader element, with the tab's find matches highlighted. The layout of the
    /// string holding the current match is kept so the match can be scrolled to.
    pub(super) fn find_text(
        &self,
        thread: u32,
        msg: MessageId,
        segment: Segment,
        text: &str,
        cx: &App,
    ) -> AnyElement {
        let Some(highlights) = self.find_highlights(thread, msg, segment, cx) else {
            return text.to_owned().into_any_element();
        };
        let styled = StyledText::new(text.to_owned()).with_highlights(highlights);
        if self.find_current(thread).is_some_and(|m| m.msg == msg && m.segment == segment) {
            *self.find_layout.borrow_mut() = Some(FindLayout {
                key: (msg, segment),
                layout: styled.layout().clone(),
                frame: self.find_gen.get(),
            });
        }
        styled.into_any_element()
    }

    /// Bodies with matches show their text, not the HTML rendering, which cannot be highlighted.
    /// Only while the find is on; the message's own Reader mode setting is untouched.
    pub(super) fn find_forces_plain(&self, m: &Message) -> bool {
        !self.in_session()
            && self.finds.get(&m.thread_id).is_some_and(|ft| {
                ft.matches.iter().any(|x| x.msg == m.id && x.segment != Segment::Subject)
            })
    }

    /// Second step of scrolling to the current match, once the frame holding its text is
    /// painted: nudge the match's line into the viewport. Runs at the start of a render.
    pub(super) fn place_find_match(&mut self, window: &mut Window) {
        self.find_gen.set(self.find_gen.get() + 1);
        let Some(thread) = self.find_thread() else { return };
        let mut panes = self.reader_panes.borrow_mut();
        let Some(pane) = panes.get_mut(&thread) else { return };
        if pane.find_reveal != FindReveal::Line {
            return;
        }
        pane.find_reveal = FindReveal::Idle;
        let Some(m) = self.find_current(thread) else { return };
        let stash = self.find_layout.borrow();
        let Some(stash) = stash.as_ref() else { return };
        // Only a layout from the previous frame has been painted (and so measured).
        if stash.key != (m.msg, m.segment) || stash.frame + 1 != self.find_gen.get() {
            return;
        }
        let Some(at) = stash.layout.position_for_index(m.range.start) else { return };
        let view = pane.scroll.bounds();
        let (top, bottom) = (view.top() + px(32.), view.bottom() - px(56.));
        let dy = if at.y < top {
            top - at.y
        } else if at.y > bottom {
            bottom - at.y
        } else {
            return;
        };
        let offset = pane.scroll.offset();
        pane.scroll.set_offset(point(offset.x, offset.y + dy));
        window.refresh();
    }

    /// A frame is still owed to finish placing the match.
    pub(super) fn find_placement_pending(&self) -> bool {
        self.reader_panes.borrow().values().any(|p| p.find_reveal == FindReveal::Line)
    }
}
