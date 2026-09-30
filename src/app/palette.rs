//! Keyboard-first command palette: the kit's `Command`, hosted in a dialog by the owner.
//!
//! Filtering is ours ([`crate::fuzzy`]); `Command` only renders the pre-filtered, sectioned
//! list (`filterable(false)`) and reports the typed query and the confirmed row.
use gpui_kit::component::ActiveTheme as _;

use std::rc::Rc;

use crate::app::actions::{self, Category, CommandSpec, PALETTE_CONTEXT, ToggleCommandPalette};
use crate::app::ui::shortcut_chips;
use crate::fuzzy;
use crate::search::Query;
use gpui_kit::{
    base::IndexPath,
    component::command::{Command, CommandGroup, CommandItem, CommandState},
    *,
};

pub struct CommandPalette {
    state: Entity<CommandState>,
    query: String,
    commands: Vec<CommandSpec>,
}

pub enum PaletteEvent {
    Run(Box<dyn Action>),
    Dismiss,
    /// Enter on a search query (`/…` or containing `from:` etc.); carries the raw text.
    Search(String),
}

/// What a confirmed row does.
#[derive(Clone)]
enum Target {
    Run(fn() -> Box<dyn Action>),
    Search(String),
}

impl CommandPalette {
    /// A palette showing `query` (empty for the full list).
    pub fn new(query: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = cx.new(|cx| CommandState::new(window, cx));
        if !query.is_empty() {
            state.update(cx, |state, cx| state.set_query(query, window, cx));
        }
        Self { state, query: query.to_owned(), commands: actions::commands() }
    }

    /// Focus the search field.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.state.clone().update(cx, |state, cx| state.focus(window, cx));
    }

    /// The typed text.
    pub fn query(&self) -> String {
        self.query.clone()
    }

    /// Names of the commands currently listed (empty while a search query is shown).
    pub fn rows(&self) -> Vec<String> {
        if Query::is_search(&self.query) {
            return Vec::new();
        }
        sections(&self.query, &self.commands)
            .into_iter()
            .flat_map(|(_, rows)| rows)
            .map(|i| self.commands[i].name.to_owned())
            .collect()
    }
}

impl EventEmitter<PaletteEvent> for CommandPalette {}

impl Render for CommandPalette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.weak_entity();
        let mut targets: Vec<Vec<Target>> = Vec::new();
        let mut command = Command::new(&self.state)
            .bordered(false)
            .filterable(false)
            .max_h(px(400.))
            .placeholder("Type a command…")
            .empty(|_, _, cx| {
                div()
                    .p_4()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("No matching commands")
            });

        if Query::is_search(&self.query) {
            let label = format!("Search: {}", Query::parse(&self.query).describe());
            command = command.item(CommandItem::new().label(label));
            targets.push(vec![Target::Search(self.query.clone())]);
        } else {
            for (category, rows) in sections(&self.query, &self.commands) {
                let mut group = CommandGroup::new().label(category.label());
                let mut group_targets = Vec::new();
                for i in rows {
                    let spec = &self.commands[i];
                    group = group.item(command_item(spec, window));
                    group_targets.push(Target::Run(spec.action));
                }
                command = command.group(group);
                targets.push(group_targets);
            }
        }

        let targets = Rc::new(targets);
        let on_query = weak.clone();
        let command = command
            .on_query(move |q, _, cx| {
                on_query
                    .update(cx, |palette, cx| {
                        if palette.query != q {
                            palette.query = q.to_owned();
                            cx.notify();
                        }
                    })
                    .ok();
            })
            .on_confirm(move |ix: IndexPath, _, cx| {
                let Some(target) = targets.get(ix.section).and_then(|g| g.get(ix.row)) else {
                    return;
                };
                let event = match target {
                    Target::Run(action) => PaletteEvent::Run(action()),
                    Target::Search(q) => PaletteEvent::Search(q.clone()),
                };
                weak.update(cx, |_, cx| cx.emit(event)).ok();
            });

        div()
            .key_context(PALETTE_CONTEXT)
            .on_action(cx.listener(|_, _: &ToggleCommandPalette, _, cx| cx.emit(PaletteEvent::Dismiss)))
            .child(command)
    }
}

/// One palette row. When the action's real binding is the very key `CommandSpec` documents,
/// the row hands its action to `Command`, which draws the `Kbd` from that binding. Otherwise
/// (multi-stroke `g i`, an action bound to several keys, a context-only binding, or the
/// palette's own toggle, which must not fire from inside the dialog) the row draws the
/// documented key itself with the shared [`shortcut_chips`].
fn command_item(spec: &CommandSpec, window: &Window) -> CommandItem {
    let item = CommandItem::new().label(spec.name);
    let action = (spec.action)();
    let documented = Keystroke::parse(spec.key).ok().filter(|_| !spec.key.contains(' '));
    let bound = window.highest_precedence_binding_for_action(action.as_ref());
    let faithful = match (&documented, &bound) {
        (Some(doc), Some(bound)) => match bound.keystrokes() {
            [stroke] => {
                stroke.as_keystroke().key == doc.key
                    && stroke.as_keystroke().modifiers == doc.modifiers
            }
            _ => false,
        },
        (None, None) => spec.key.is_empty(),
        _ => false,
    };
    if faithful && !action.as_any().is::<ToggleCommandPalette>() {
        return item.action(action);
    }
    if spec.key.is_empty() {
        return item;
    }
    let (name, key) = (spec.name, spec.key);
    item.child(move |_, cx| {
        div().flex().w_full().items_center().justify_between().child(name).child(shortcut_chips(key, cx))
    })
}

/// Indexes into `commands` matching `query`, grouped by category. With a query, rows are
/// ordered best match first within a section and sections by their best row (ties keep the
/// category order), so the highlighted first row is the best match. Empty sections are dropped,
/// so a section's position is its `IndexPath::section`.
fn sections(query: &str, commands: &[CommandSpec]) -> Vec<(Category, Vec<usize>)> {
    let mut found: Vec<(fuzzy::Rank, Category, Vec<usize>)> = Category::ALL
        .into_iter()
        .filter_map(|category| {
            let mut rows: Vec<(fuzzy::Rank, usize)> = commands
                .iter()
                .enumerate()
                .filter(|(_, c)| c.category == category)
                .filter_map(|(i, c)| fuzzy::rank(c.name, c.key, query).map(|r| (r, i)))
                .collect();
            rows.sort_by_key(|&(rank, _)| rank);
            let best = rows.first()?.0;
            Some((best, category, rows.into_iter().map(|(_, i)| i).collect()))
        })
        .collect();
    found.sort_by_key(|&(best, ..)| best);
    found.into_iter().map(|(_, category, rows)| (category, rows)).collect()
}

#[cfg(test)]
mod tests {
    use super::{actions, sections};

    #[test]
    fn empty_query_lists_every_command_once_in_category_order() {
        let cmds = actions::commands();
        let found = sections("", &cmds);
        let total: usize = found.iter().map(|(_, rows)| rows.len()).sum();
        assert_eq!(total, cmds.len());
        let cats: Vec<_> = found.iter().map(|(c, _)| *c).collect();
        let expected: Vec<_> = actions::Category::ALL.to_vec();
        assert_eq!(cats, expected);
    }

    #[test]
    fn best_match_leads_even_from_a_later_category() {
        let cmds = actions::commands();
        let found = sections("archive", &cmds);
        assert!(found.iter().all(|(_, rows)| !rows.is_empty()));
        let (_, first_rows) = &found[0];
        assert_eq!(cmds[first_rows[0]].name, "Archive", "prefix beats the `Go to archive` substring");
    }

    #[test]
    fn key_string_finds_its_command_first() {
        let cmds = actions::commands();
        let found = sections("g s", &cmds);
        assert_eq!(cmds[found[0].1[0]].name, "Go to snoozed");
    }
}
