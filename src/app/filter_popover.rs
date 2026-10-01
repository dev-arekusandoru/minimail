//! The popover behind `+ Filter` and behind a pill's value: pick a field, then a value.
//!
//! Built from the kit: a `Command` for the field list and for fuzzy value lists (tag, is, kind,
//! account, in), an `Input` for free text (people, subject, body) and a `Calendar` plus an
//! `Input` for dates (so `7d` stays typeable). Like the other pickers it holds no business
//! logic: the choice leaves through [`FilterPopoverEvent`] and the owner edits the query.
//! Clicking a pill's value opens it straight on the value step, prefilled.
use chrono::{Datelike, NaiveDate};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::calendar::{Calendar, CalendarEvent, CalendarState};

use crate::app::ui::button;
use crate::filters::{self, Choice, FieldChoice, ValueInput};
use crate::search::Field;
use gpui_kit::{
    base::IndexPath,
    component::{
        command::{Command, CommandItem, CommandState},
        input::{Input, InputEvent, InputState},
    },
    prelude::FluentBuilder as _,
    *,
};

/// Key context of the picker. The kit popover around it binds `enter` and `space` to close
/// itself; `bind_keys` disables those here so they reach the picker's list and inputs.
pub const FILTER_PICKER_CONTEXT: &str = "FilterPicker";

pub enum FilterPopoverEvent {
    /// Add `value` to `field`, or swap it in for `replacing` when editing a pill.
    Apply { field: Field, value: String, replacing: Option<String> },
    Dismiss,
}

/// Where the popover is: choosing a field, or entering that field's value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    Field,
    Value(Field),
}

pub struct FilterPopover {
    step: Step,
    /// The value being edited, when opened from a pill.
    replacing: Option<String>,
    /// Values of every fuzzy-listed field, fixed when the popover opens.
    choices: Vec<(Field, Vec<Choice>)>,
    command: Entity<CommandState>,
    /// Trimmed text typed into the command's filter.
    query: String,
    input: Entity<InputState>,
    calendar: Entity<CalendarState>,
    invalid: bool,
}

impl FilterPopover {
    /// The field list (`edit` is `None`), or the value step of `edit` = `(field, value)`.
    pub fn new(
        choices: Vec<(Field, Vec<Choice>)>,
        edit: Option<(Field, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let command = cx.new(|cx| CommandState::new(window, cx));
        let input = cx.new(|cx| InputState::new(window, cx));
        let calendar = cx.new(|cx| CalendarState::new(window, cx));
        cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::PressEnter { .. } => this.submit_text(window, cx),
            InputEvent::Change => {
                this.invalid = false;
                cx.notify();
            }
            _ => {}
        })
        .detach();
        cx.subscribe(&calendar, |this, _, event: &CalendarEvent, cx| {
            let CalendarEvent::Selected(date) = event;
            if let (Step::Value(field), Some(day)) = (this.step, date.start()) {
                this.apply(field, filters::format_ymd(day.year(), day.month(), day.day()), cx);
            }
        })
        .detach();
        let mut popover = Self {
            step: Step::Field,
            replacing: None,
            choices,
            command,
            query: String::new(),
            input,
            calendar,
            invalid: false,
        };
        if let Some((field, value)) = edit {
            popover.replacing = Some(value.clone());
            popover.enter_value_step(field, Some(&value), window, cx);
        }
        popover
    }

    pub fn step(&self) -> Step {
        self.step
    }

    /// Whether this popover edits an existing pill rather than adding one.
    pub fn is_editing(&self) -> bool {
        self.replacing.is_some()
    }

    /// Labels of the listed rows (field or value step); empty on a text or date step.
    pub fn rows(&self) -> Vec<String> {
        match self.step {
            Step::Field => self.field_rows().into_iter().map(|c| c.label.to_owned()).collect(),
            Step::Value(field) => self.value_rows(field).into_iter().map(|c| c.label).collect(),
        }
    }

    /// Put the cursor where typing belongs: the filter field, or the text/date input.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        match self.step {
            Step::Value(field) if filters::value_input(field) != ValueInput::Pick => {
                window.focus(&self.input.focus_handle(cx), cx);
            }
            _ => self.command.clone().update(cx, |state, cx| state.focus(window, cx)),
        }
    }

    /// The handle typing belongs to: the text/date input, else the command's filter field.
    pub fn focus_handle_for_focus(&self, cx: &App) -> FocusHandle {
        match self.step {
            Step::Value(field) if filters::value_input(field) != ValueInput::Pick => {
                self.input.focus_handle(cx)
            }
            _ => self.command.read(cx).focus_handle(cx),
        }
    }

    fn field_rows(&self) -> Vec<FieldChoice> {
        filters::rank_fields(&self.query)
    }

    fn value_rows(&self, field: Field) -> Vec<Choice> {
        let all = self
            .choices
            .iter()
            .find(|(f, _)| *f == field)
            .map(|(_, c)| c.clone())
            .unwrap_or_default();
        filters::rank_choices(all, &self.query)
    }

    fn enter_value_step(&mut self, field: Field, prefill: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        self.step = Step::Value(field);
        self.invalid = false;
        self.query.clear();
        self.command.clone().update(cx, |state, cx| state.set_query("", window, cx));
        let text = prefill.unwrap_or_default();
        self.input.update(cx, |input, cx| input.set_value(text, window, cx));
        if filters::value_input(field) == ValueInput::Date
            && let Some((y, m, d)) = filters::parse_ymd(text)
            && let Some(day) = NaiveDate::from_ymd_opt(y, m, d)
        {
            self.calendar.update(cx, |calendar, cx| calendar.set_date(day, window, cx));
        }
        self.focus(window, cx);
        cx.notify();
    }

    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.step = Step::Field;
        self.query.clear();
        self.command.clone().update(cx, |state, cx| state.set_query("", window, cx));
        self.focus(window, cx);
        cx.notify();
    }

    fn apply(&mut self, field: Field, value: String, cx: &mut Context<Self>) {
        cx.emit(FilterPopoverEvent::Apply { field, value, replacing: self.replacing.clone() });
    }

    /// Enter in the text/date input: the typed value, or flag it invalid.
    fn submit_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Step::Value(field) = self.step else { return };
        if filters::value_input(field) == ValueInput::Pick {
            return;
        }
        let text = self.input.read(cx).value().to_string();
        match field.normalize(&text) {
            Some(value) => self.apply(field, value, cx),
            None => {
                self.invalid = true;
                window.focus(&self.input.focus_handle(cx), cx);
                cx.notify();
            }
        }
    }

    /// Enter on row `row` of the command list.
    fn confirm(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        match self.step {
            Step::Field => {
                if let Some(choice) = self.field_rows().get(row).copied() {
                    self.enter_value_step(choice.field, None, window, cx);
                }
            }
            Step::Value(field) => {
                if let Some(choice) = self.value_rows(field).get(row) {
                    self.apply(field, choice.value.clone(), cx);
                }
            }
        }
    }

    fn field_label(field: Field) -> &'static str {
        filters::FIELD_CHOICES.iter().find(|c| c.field == field).map_or("", |c| c.label)
    }
}

impl EventEmitter<FilterPopoverEvent> for FilterPopover {}

impl Render for FilterPopover {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.weak_entity();
        let t = cx.theme();
        let (muted, danger) = (t.muted_foreground, t.danger);
        let heading = |text: String| div().px_2().pt_1().text_xs().text_color(muted).child(text);
        match self.step {
            Step::Value(field) if filters::value_input(field) != ValueInput::Pick => {
                let dated = filters::value_input(field) == ValueInput::Date;
                let invalid = self.invalid;
                let hint = if dated { "yyyy-mm-dd, or 7d, 2w, 3m, 1y" } else { "" };
                div()
                    .id("filter-popover")
                    .key_context(FILTER_PICKER_CONTEXT)
                    .w(px(if dated { 280. } else { 260. }))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(heading(Self::field_label(field).to_owned()))
                    .when(dated, |d| d.child(Calendar::new(&self.calendar)))
                    .child(Input::new(&self.input))
                    .when(dated, |d| d.child(div().px_2().text_xs().text_color(muted).child(hint)))
                    .when(invalid, |d| {
                        d.child(div().px_2().text_xs().text_color(danger).child("Not a valid value"))
                    })
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .when(!self.is_editing(), |d| {
                                d.child(button("filter-back", "Back", "Choose another field", "", cx).on_click(
                                    cx.listener(|this, _, window, cx| this.back(window, cx)),
                                ))
                            })
                            .child(div().flex_1())
                            .child(
                                button("filter-apply", "Apply", "Apply the value", "enter", cx)
                                    .on_click(cx.listener(|this, _, window, cx| this.submit_text(window, cx))),
                            ),
                    )
                    .into_any_element()
            }
            step => {
                let (items, title, placeholder): (Vec<String>, String, &str) = match step {
                    Step::Field => (self.rows(), "Filter by".to_owned(), "Choose a field…"),
                    Step::Value(field) => (self.rows(), Self::field_label(field).to_owned(), "Choose a value…"),
                };
                let on_query = weak.clone();
                div()
                    .id("filter-popover")
                    .key_context(FILTER_PICKER_CONTEXT)
                    .w(px(260.))
                    .child(heading(title))
                    .child(
                        Command::new(&self.command)
                            .bordered(false)
                            .filterable(false)
                            .max_h(px(280.))
                            .placeholder(placeholder)
                            .empty(move |_, _, _| div().p_3().text_xs().text_color(muted).child("Nothing matches"))
                            .items(items.into_iter().map(|label| CommandItem::new().label(label)))
                            .on_query(move |q, _, cx| {
                                on_query
                                    .update(cx, |this, cx| {
                                        let q = q.trim();
                                        if this.query != q {
                                            this.query = q.to_owned();
                                            cx.notify();
                                        }
                                    })
                                    .ok();
                            })
                            .on_confirm(move |ix: IndexPath, window, cx| {
                                weak.update(cx, |this, cx| this.confirm(ix.row, window, cx)).ok();
                            }),
                    )
                    .into_any_element()
            }
        }
    }
}
