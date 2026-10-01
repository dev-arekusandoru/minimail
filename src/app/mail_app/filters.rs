//! The list's filter state: one [`Query`] over all mail, the pills that show it, and the
//! picker behind `+ Filter` and a pill's value.
//!
//! Location is just an `in:` value, so the header, the sidebar and the sync scopes all read
//! the same query. The pills *are* the state: what they show is what the list applies, and
//! nothing is filtered behind their back.
use gpui_kit::assets::IconName;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;

use super::*;
use crate::app::filter_popover::{FilterPopover, FilterPopoverEvent, Step};
use crate::filters::{Choice, QuickFilter, ValueInput};
use crate::search::{Combinator, Field, Pill, Query};

/// Which popover the list header currently shows.
#[derive(PartialEq, Eq)]
pub(super) enum OpenFilterPopover {
    /// `+ Filter`: the field list, then a value.
    Add,
    /// One pill's own editor, prefilled with its value.
    Edit(Pill),
}

/// The open popover and the picker view inside it.
pub(super) struct FilterPopoverState {
    pub(super) target: OpenFilterPopover,
    pub(super) entity: Entity<FilterPopover>,
    _sub: Subscription,
}

/// One pill of the header: a field value, or a free-text term shown as itself.
#[derive(Clone)]
struct Shown {
    field: Field,
    value: String,
    combinator: Option<Combinator>,
    /// A free-text term rather than a `field:value` pair.
    text: bool,
}

impl Shown {
    fn key(&self) -> &str {
        if self.text { "text" } else { self.field.key() }
    }

    /// `<field>-<value>`: the pill is `pill-…`, its value `pill-value-…`, its × `pill-remove-…`.
    fn id(&self) -> String {
        format!("{}-{}", self.key(), self.value)
    }
}

impl MailApp {
    /// The one query the list is: its location is the `in:` value, everything else is a pill.
    pub fn query(&self) -> &Query {
        &self.triage.query
    }

    /// The location the list shows: the browsed folder while the query still holds its
    /// `in:` and `account:` values, else whatever place the query names on its own (a typed
    /// `in:archived`), else `None` for a global search.
    pub fn location(&self) -> Option<Location> {
        let q = &self.triage.query;
        let own = self.implicit_query();
        let keeps_folder = q.values(Field::In) == own.values(Field::In)
            && own.values(Field::Account).iter().all(|a| q.values(Field::Account).contains(a));
        if keeps_folder {
            Some(self.folder.clone())
        } else {
            self.mailbox.query_location(q)
        }
    }

    /// The query the browsed folder implies on its own: `in:<place>` plus `account:`.
    /// Its pills stay invisible while the list's query is exactly this.
    pub(super) fn implicit_query(&self) -> Query {
        self.mailbox.location_query(&self.folder)
    }

    /// Whether anything beyond the location narrows the list.
    pub fn is_filtered(&self) -> bool {
        self.triage.query != self.implicit_query()
    }

    /// Every pill the query holds, free-text terms included.
    fn shown_pills(&self) -> Vec<Shown> {
        self.triage
            .query
            .pills()
            .into_iter()
            .map(|p| Shown {
                field: p.field,
                value: p.value,
                combinator: p.combinator,
                text: false,
            })
            .chain(self.triage.query.text().iter().map(|t| Shown {
                field: Field::Body,
                value: t.clone(),
                combinator: None,
                text: true,
            }))
            .collect()
    }

    /// The pills the header draws: everything but the `in:` / `account:` values that only
    /// restate the folder being browsed.
    fn visible_pills(&self) -> Vec<Shown> {
        if self.is_filtered() {
            self.shown_pills()
        } else {
            Vec::new()
        }
    }

    /// `"Inbox · 100"`: the folder name, then how much mail is behind it.
    pub fn header_title(&self, count: usize) -> String {
        format!("{}{}", self.location_label(), self.header_count(count))
    }

    /// The count part of [`Self::header_title`]: `" · 100"`, plus `" · 40 threads"` grouped.
    fn header_count(&self, count: usize) -> String {
        if !self.grouped() {
            return format!(" · {count}");
        }
        let headers = self.rows().iter().filter(|row| !matches!(row, Row::Child { .. })).count();
        format!(" · {count} · {headers} threads")
    }

    /// `Some("search: …")` while anything beyond the location is applied.
    pub fn search_header(&self) -> Option<String> {
        self.is_filtered().then(|| format!("search: {}", self.triage.query.describe()))
    }

    /// The pill row as text, left to right: each pill, with its group's `and`/`or` before
    /// every value after the first.
    pub fn pill_texts(&self) -> Vec<String> {
        let mut out = Vec::new();
        for pill in self.visible_pills() {
            if pill.combinator.is_some() {
                out.push(combinator_word(pill.combinator).to_owned());
            }
            out.push(self.pill_label(&pill));
        }
        out
    }

    /// What a pill reads: accounts by name, other values as the query stores them.
    fn pill_label(&self, pill: &Shown) -> String {
        match pill.field {
            _ if pill.text => pill.value.clone(),
            Field::Account => format!(
                "account:{}",
                crate::filters::value_label(pill.field, &pill.value, &self.mailbox)
            ),
            field => format!("{}:{}", field.key(), pill.value),
        }
    }

    // ------------------------------------------------------------------ query edits

    /// Point the list at `query`: the cursor goes to the top and the reader stays.
    pub(super) fn apply_query(&mut self, query: Query, cx: &mut Context<Self>) {
        self.end_session();
        self.triage.set_query(query);
        self.row_cursor = 0;
        self.row_anchor = None;
        self.scroll_to_cursor();
        self.close_filter_popover();
        cx.notify();
    }

    /// Show `location` the way the sidebar does: a fresh `in:` and no other filters.
    pub(super) fn show_location(&mut self, location: Location, cx: &mut Context<Self>) {
        let query = self.mailbox.location_query(&location);
        self.folder = location;
        self.apply_query(query, cx);
    }

    /// Add `value` to `field`, or swap it for `replacing` when editing a pill.
    fn add_pill(
        &mut self,
        field: Field,
        value: String,
        replacing: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let mut query = self.triage.query.clone();
        let changed = match replacing {
            Some(old) => query.replace(field, &old, &value),
            None => query.add(field, &value),
        };
        if changed {
            self.apply_query(query, cx);
        }
    }

    /// Remove one free-text term.
    pub(super) fn remove_text_pill(&mut self, term: &str, cx: &mut Context<Self>) {
        let mut query = self.triage.query.clone();
        if query.remove_text(term) {
            self.apply_query(query, cx);
        }
    }

    /// Flip a field's group between `and` and `or`.
    pub(super) fn toggle_combinator(&mut self, field: Field, cx: &mut Context<Self>) {
        let Some(current) = self.triage.query.combinator(field) else {
            return;
        };
        let mut query = self.triage.query.clone();
        query.set_combinator(field, current.toggled());
        self.apply_query(query, cx);
    }

    /// What number key `index + 1` does: drop every filter, or toggle one tag pill.
    pub(super) fn quick_filter(&mut self, index: usize, cx: &mut Context<Self>) {
        match crate::filters::quick_filter(index) {
            Some(QuickFilter::Clear) => self.clear_filters(cx),
            Some(QuickFilter::Toggle(field, value)) => {
                let mut query = self.triage.query.clone();
                query.toggle(field, value);
                self.apply_query(query, cx);
            }
            None => {}
        }
    }

    /// Drop every pill but the folder itself.
    pub(super) fn clear_filters(&mut self, cx: &mut Context<Self>) {
        let query = self.implicit_query();
        self.apply_query(query, cx);
    }

    // ------------------------------------------------------------------ the picker

    /// Values of every field the picker offers a list for, taken when it opens.
    fn value_choices(&self) -> Vec<(Field, Vec<Choice>)> {
        Field::ALL
            .into_iter()
            .filter(|f| crate::filters::value_input(*f) == ValueInput::Pick)
            .map(|f| (f, crate::filters::value_choices(f, &self.mailbox)))
            .filter(|(_, choices)| !choices.is_empty())
            .collect()
    }

    fn popover_is(&self, target: &OpenFilterPopover) -> bool {
        self.filter_popover.as_ref().is_some_and(|p| &p.target == target)
    }

    /// Open the `+ Filter` picker (the `l` key) with the cursor in it.
    pub(super) fn open_add_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.filter_popover.is_some() {
            return;
        }
        let entity = cx.new(|cx| FilterPopover::new(self.value_choices(), None, window, cx));
        self.attach_filter_popover(OpenFilterPopover::Add, entity, window, cx);
    }

    /// Open one pill's editor, prefilled with its value.
    fn open_pill_editor(&mut self, pill: Pill, window: &mut Window, cx: &mut Context<Self>) {
        let target = OpenFilterPopover::Edit(pill.clone());
        if self.popover_is(&target) {
            return;
        }
        let choices = self.value_choices();
        let edit = Some((pill.field, pill.value.clone()));
        let entity = cx.new(|cx| FilterPopover::new(choices, edit, window, cx));
        self.attach_filter_popover(target, entity, window, cx);
    }

    fn attach_filter_popover(
        &mut self,
        target: OpenFilterPopover,
        entity: Entity<FilterPopover>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sub = cx.subscribe_in(
            &entity,
            window,
            |this, _, event: &FilterPopoverEvent, window, cx| {
                if let FilterPopoverEvent::Apply { field, value, replacing } = event {
                    match replacing.clone() {
                        Some(old) => this.add_pill(*field, value.clone(), Some(old), cx),
                        None => this.add_pill(*field, value.clone(), None, cx),
                    }
                }
                this.close_filter_popover();
                window.focus(&this.focus_handle, cx);
                cx.notify();
            },
        );
        let focus = entity.read(cx).focus_handle_for_focus(cx);
        focus.focus(window, cx);
        self.filter_popover = Some(FilterPopoverState { target, entity, _sub: sub });
        cx.notify();
    }

    pub(super) fn close_filter_popover(&mut self) {
        self.filter_popover = None;
    }

    /// Whether the filter picker (or a pill's editor) is open.
    pub fn filter_popover_open(&self) -> bool {
        self.filter_popover.is_some()
    }

    /// Which step the open picker is on: the field list, or the field being filled.
    pub fn filter_popover_step(&self, cx: &App) -> Option<Step> {
        self.filter_popover.as_ref().map(|p| p.entity.read(cx).step())
    }

    /// Labels of the rows the open picker lists, top to bottom.
    pub fn filter_popover_rows(&self, cx: &App) -> Vec<String> {
        self.filter_popover
            .as_ref()
            .map(|p| p.entity.read(cx).rows())
            .unwrap_or_default()
    }

    // ------------------------------------------------------------------ header

    /// The pill row: the folder's result count, every visible pill with its `and`/`or`
    /// toggles, `+ Filter` and, while anything is applied, `Clear`.
    pub(super) fn render_pills(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let mut row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .text_size(px(12.))
                    .text_color(t.foreground)
                    .child(self.location_label())
                    .child(
                        div()
                            .text_color(t.muted_foreground)
                            .child(self.header_count(self.visible_ids().len())),
                    ),
            );
        let mut in_group = 0;
        let mut last_field = None;
        for pill in self.visible_pills() {
            in_group = if last_field == Some(pill.field) && !pill.text { in_group + 1 } else { 0 };
            last_field = (!pill.text).then_some(pill.field);
            if pill.combinator.is_some() {
                let field = pill.field;
                let word = combinator_word(pill.combinator);
                row = row.child(
                    button(
                        format!("group-op-{}-{in_group}", pill.key()),
                        word,
                        "Combine these values differently",
                        "",
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_combinator(field, cx))),
                );
            }
            row = row.child(self.render_pill(pill, cx));
        }
        row = row.child(self.render_add_filter(cx));
        if self.is_filtered() {
            row = row.child(
                button("btn-clear-filters", "Clear", "Remove every filter", "", cx)
                    .on_click(run(ClearFilters)),
            );
        }
        row.id("list-pills").test_support().into_any_element()
    }

    /// One pill: its value (click to edit, inside a popover) and the remove button.
    fn render_pill(&self, pill: Shown, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let id = pill.id();
        let label = self.pill_label(&pill);
        let value = button(format!("pill-value-{id}"), label, "Edit this filter", "", cx)
            .h(px(22.))
            .px_2()
            .border_color(t.border);
        let editing_pill = Pill {
            field: pill.field,
            value: pill.value.clone(),
            combinator: pill.combinator,
        };
        let open = self.popover_is(&OpenFilterPopover::Edit(editing_pill.clone()));
        let editing = if open {
            self.filter_popover.as_ref().map(|p| p.entity.clone())
        } else {
            None
        };
        let weak = cx.weak_entity();
        let value = Popover::new(format!("pill-popover-{id}"))
            .trigger(value)
            .open(open)
            .content(move |_, _, _| match &editing {
                Some(entity) => entity.clone().into_any_element(),
                None => div().into_any_element(),
            })
            .on_open_change(move |open, window, cx| {
                weak.update(cx, |this, cx| {
                    if *open {
                        this.open_pill_editor(editing_pill.clone(), window, cx);
                    } else {
                        this.close_filter_popover();
                        cx.notify();
                    }
                })
                .ok();
            });
        div()
            .id(format!("pill-{id}"))
            .test_support()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .h(px(24.))
            .pr_1()
            .pl_0()
            .rounded_full()
            .border_1()
            .border_color(t.border)
            .bg(t.secondary)
            .child(value)
            .child(
                icon_button(format!("pill-remove-{id}"), IconName::X, "Remove this filter", "", cx)
                    .h(px(18.))
                    .w(px(18.))
                    .on_click(cx.listener(move |this, _, _, cx| this.remove_pill_shown(&pill, cx))),
            )
            .into_any_element()
    }

    /// Remove a header pill: a field value, or a free-text term.
    fn remove_pill_shown(&mut self, pill: &Shown, cx: &mut Context<Self>) {
        if pill.text {
            self.remove_text_pill(&pill.value, cx);
        } else {
            let mut query = self.triage.query.clone();
            if query.remove(pill.field, &pill.value) {
                self.apply_query(query, cx);
            }
        }
    }

    /// The `+ Filter` popover, triggered by the header's add button.
    fn render_add_filter(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let open = self.popover_is(&OpenFilterPopover::Add);
        let editing = if open {
            self.filter_popover.as_ref().map(|p| p.entity.clone())
        } else {
            None
        };
        let trigger = button("btn-add-filter", "+ Filter", "Add a filter pill", "l", cx)
            .when(open, |b| b.bg(t.list_active).text_color(t.primary));
        let weak = cx.weak_entity();
        Popover::new("filter-popover-add")
            .trigger(trigger)
            .open(open)
            .content(move |_, _, _| match &editing {
                Some(entity) => entity.clone().into_any_element(),
                None => div().into_any_element(),
            })
            .on_open_change(move |open, window, cx| {
                weak.update(cx, |this, cx| {
                    if *open {
                        this.open_add_filter(window, cx);
                    } else {
                        this.close_filter_popover();
                        cx.notify();
                    }
                })
                .ok();
            })
            .into_any_element()
    }
}

/// Word on a group's toggle: how this value joins the ones before it.
fn combinator_word(combinator: Option<Combinator>) -> &'static str {
    match combinator {
        Some(Combinator::And) => "and",
        Some(Combinator::Or) => "or",
        None => "",
    }
}
