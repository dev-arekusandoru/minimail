//! Folder picker: type to filter an account's folders, `enter` files into the
//! selected one, and a name with no folder of its own offers to create it.
//!
//! The kit's `Command`, hosted in a dialog by the owner. Like [`crate::app::dialog`], it holds
//! no business logic: the choice leaves through [`FolderPickerEvent`] and the owner creates
//! folders and files mail. Filtering is ours; `Command` renders the pre-filtered rows.
use gpui_kit::component::ActiveTheme as _;

use std::rc::Rc;

use crate::model::FolderId;
use gpui_kit::{
    base::IndexPath,
    component::command::{Command, CommandItem, CommandState},
    *,
};

pub enum FolderPickerEvent {
    /// File into an existing folder.
    File(FolderId),
    /// Create a folder with this name, then file into it.
    Create(String),
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
    state: Entity<CommandState>,
    title: SharedString,
    folders: Vec<FolderOption>,
    /// Trimmed filter text.
    query: String,
}

impl FolderPicker {
    pub fn new(
        title: impl Into<SharedString>,
        folders: Vec<FolderOption>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            state: cx.new(|cx| CommandState::new(window, cx)),
            title: title.into(),
            folders,
            query: String::new(),
        }
    }

    pub fn title(&self) -> String {
        self.title.to_string()
    }

    /// Current filter text.
    pub fn query(&self) -> String {
        self.query.clone()
    }

    /// Focus the filter field.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.state.clone().update(cx, |state, cx| state.focus(window, cx));
    }

    /// The visible rows, top to bottom, as labels.
    pub fn rows(&self) -> Vec<String> {
        self.row_labels().0
    }

    fn row_labels(&self) -> (Vec<String>, Vec<Row>) {
        let q = self.query.as_str();
        let needle = q.to_lowercase();
        let mut rows: Vec<Row> = self
            .folders
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                q.is_empty()
                    || f.name.to_lowercase().contains(&needle)
                    || f.label.to_lowercase().contains(&needle)
            })
            .map(|(i, _)| Row::Folder(i))
            .collect();
        let exact = self.folders.iter().any(|f| f.name.eq_ignore_ascii_case(q));
        if !q.is_empty() && !exact {
            rows.push(Row::Create(q.to_owned()));
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
}

impl EventEmitter<FolderPickerEvent> for FolderPicker {}

impl Render for FolderPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.weak_entity();
        let (labels, rows) = self.row_labels();
        let events: Rc<Vec<FolderPickerEvent>> = Rc::new(
            rows.iter()
                .map(|row| match row {
                    Row::Folder(i) => FolderPickerEvent::File(self.folders[*i].id),
                    Row::Create(name) => FolderPickerEvent::Create(name.clone()),
                })
                .collect(),
        );
        let title = self.title.clone();
        let on_query = weak.clone();
        Command::new(&self.state)
            .bordered(false)
            .filterable(false)
            .max_h(px(360.))
            .placeholder("Filter or new folder name")
            .header(move |_, _, cx| {
                div()
                    .px_3()
                    .pt_3()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().foreground)
                    .child(title.clone())
            })
            .empty(|_, _, cx| {
                div()
                    .p_4()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("No folders in this account yet.")
            })
            .items(labels.into_iter().map(|label| CommandItem::new().label(label)))
            .on_query(move |q, _, cx| {
                on_query
                    .update(cx, |picker, cx| {
                        let q = q.trim();
                        if picker.query != q {
                            picker.query = q.to_owned();
                            cx.notify();
                        }
                    })
                    .ok();
            })
            .on_confirm(move |ix: IndexPath, _, cx| {
                let Some(event) = events.get(ix.row) else { return };
                let event = match event {
                    FolderPickerEvent::File(id) => FolderPickerEvent::File(*id),
                    FolderPickerEvent::Create(name) => FolderPickerEvent::Create(name.clone()),
                };
                weak.update(cx, |_, cx| cx.emit(event)).ok();
            })
    }
}
