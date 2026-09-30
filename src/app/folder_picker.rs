//! Folder picker: type to filter an account's folders, `enter` files into the
//! selected one, and a name with no folder of its own offers to create it.
//!
//! Like [`crate::app::dialog`], it holds no business logic: the choice leaves
//! through [`FolderPickerEvent`] and the owner creates folders and files mail.

use crate::app::overlay::FitViewport as _;
use crate::model::FolderId;
use crate::theme::{self, Theme};
use gpui_kit::{
    component::input::{Input, InputEvent, InputState},
    prelude::FluentBuilder as _,
    *,
};

/// Key context of the picker. `enter` is bound under `FolderPicker && !Input`,
/// `escape` and `up`/`down` under the bare context.
pub const FOLDER_PICKER_CONTEXT: &str = "FolderPicker";

gpui_kit::actions!(
    folder_picker,
    [FolderPickerNext, FolderPickerPrev, FolderPickerConfirm, FolderPickerCancel]
);

pub enum FolderPickerEvent {
    /// File into an existing folder.
    File(FolderId),
    /// Create a folder with this name, then file into it.
    Create(String),
    Cancel,
}

/// One selectable folder, already in tree order and indented by depth.
pub struct FolderOption {
    pub id: FolderId,
    pub name: SharedString,
    pub label: SharedString,
}

impl FolderOption {
    pub fn new(id: FolderId, name: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id,
            name: name.into(),
            label: label.into(),
        }
    }
}

/// One visible line of the picker.
enum Row {
    Folder(usize),
    Create(String),
}

pub struct FolderPicker {
    focus: FocusHandle,
    input: Entity<InputState>,
    title: SharedString,
    folders: Vec<FolderOption>,
    selected: usize,
}

impl FolderPicker {
    pub fn new(
        title: impl Into<SharedString>,
        folders: Vec<FolderOption>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter or new folder name"));
        cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            match event {
                // A focused `Input` owns `enter`/`escape`; it emits this instead.
                InputEvent::PressEnter { .. } => this.confirm(cx),
                InputEvent::Change => {
                    this.selected = 0;
                    cx.notify();
                }
                _ => {}
            }
        })
        .detach();
        Self {
            focus: cx.focus_handle(),
            input,
            title: title.into(),
            folders,
            selected: 0,
        }
    }

    pub fn title(&self) -> String {
        self.title.to_string()
    }

    /// Current filter text.
    pub fn query(&self, cx: &App) -> String {
        self.input.read(cx).value().trim().to_owned()
    }

    /// Focus handle of the filter input (the picker focuses it on open).
    pub fn input_focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }

    /// The visible rows, top to bottom, as labels.
    pub fn rows(&self, cx: &App) -> Vec<String> {
        self.row_labels(cx).0
    }

    fn row_labels(&self, cx: &App) -> (Vec<String>, Vec<Row>) {
        let q = self.query(cx);
        let mut rows: Vec<Row> = self
            .folders
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                q.is_empty()
                    || f.name.to_lowercase().contains(&q.to_lowercase())
                    || f.label.to_lowercase().contains(&q.to_lowercase())
            })
            .map(|(i, _)| Row::Folder(i))
            .collect();
        let exact = self.folders.iter().any(|f| f.name.eq_ignore_ascii_case(&q));
        if !q.is_empty() && !exact {
            rows.push(Row::Create(q));
        }
        let labels = rows
            .iter()
            .map(|row| match row {
                Row::Folder(i) => self.folders[*i].label.to_string(),
                Row::Create(name) => format!("Create “{name}”"),
            })
            .collect();
        (labels, rows)
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.row_labels(cx).0.len();
        if len == 0 {
            return;
        }
        self.selected = (self.selected as isize + delta).clamp(0, len as isize - 1) as usize;
        cx.notify();
    }

    /// `enter`: file into the selected row.
    pub fn confirm(&mut self, cx: &mut Context<Self>) {
        let (_, rows) = self.row_labels(cx);
        if rows.is_empty() {
            return;
        }
        let ix = self.selected.min(rows.len() - 1);
        match &rows[ix] {
            Row::Folder(i) => cx.emit(FolderPickerEvent::File(self.folders[*i].id)),
            Row::Create(name) => cx.emit(FolderPickerEvent::Create(name.clone())),
        }
    }

    fn pick_folder(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(folder) = self.folders.get(ix) {
            cx.emit(FolderPickerEvent::File(folder.id));
        }
    }

    fn row(t: &Theme, label: String, selected: bool) -> Div {
        let hover = t.hover;
        let selected_bg = t.selection;
        div()
            .flex()
            .items_center()
            .px_2()
            .py_1()
            .rounded_sm()
            .text_sm()
            .text_color(t.text)
            .when(selected, |d| d.bg(selected_bg))
            .hover(move |el| el.bg(hover))
            .child(label)
    }
}

impl Focusable for FolderPicker {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EventEmitter<FolderPickerEvent> for FolderPicker {}

impl Render for FolderPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::active(cx);
        let (labels, rows) = self.row_labels(cx);
        let selected = self.selected.min(labels.len().saturating_sub(1));
        let empty = labels.is_empty();
        let footer = match rows.get(selected) {
            Some(Row::Create(name)) => format!("enter creates “{name}” and files"),
            Some(Row::Folder(_)) => "enter files into the highlighted folder".to_owned(),
            None => "no folder matches · type a new name".to_owned(),
        };
        let mut folder_rows = Vec::new();
        for (row_ix, row) in rows.iter().enumerate() {
            match row {
                Row::Folder(i) => {
                    let i = *i;
                    folder_rows.push(
                        Self::row(&t, labels[row_ix].clone(), row_ix == selected)
                            .id(("folder-row", row_ix))
                            .test_support()
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| this.pick_folder(i, cx))),
                    );
                }
                Row::Create(name) => {
                    let name = name.clone();
                    folder_rows.push(
                        Self::row(&t, labels[row_ix].clone(), row_ix == selected)
                            .id(("folder-row", row_ix))
                            .test_support()
                            .cursor_pointer()
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(FolderPickerEvent::Create(name.clone()))
                            })),
                    );
                }
            }
        }
        div()
            .key_context(FOLDER_PICKER_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &FolderPickerNext, _, cx| this.move_selection(1, cx)))
            .on_action(cx.listener(|this, _: &FolderPickerPrev, _, cx| this.move_selection(-1, cx)))
            .on_action(cx.listener(|this, _: &FolderPickerConfirm, _, cx| this.confirm(cx)))
            .on_action(
                cx.listener(|_, _: &FolderPickerCancel, _, cx| cx.emit(FolderPickerEvent::Cancel)),
            )
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .fit_viewport(window, 380.)
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .rounded_md()
            .id("folder-picker")
            .test_support()
            .overflow_y_scroll()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.title.clone()),
            )
            .child(div().w_full().child(Input::new(&self.input)))
            .when(empty, |d| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(t.text_muted)
                        .child("No folders in this account yet."),
                )
            })
            .children(folder_rows)
            .child(div().text_xs().text_color(t.text_muted).child(footer))
    }
}
