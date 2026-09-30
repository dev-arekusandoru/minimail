//! Pure footer-hint selection: which keys the bottom bar advertises for a context.
//!
//! Hints are ranked; when there are more than [`MAX_HINTS`], or they don't fit the
//! available width, the lowest-ranked go first. `?` and the escape hint never drop.
//! `u` (undo) is deliberately absent: it lives on the post-action toast.

/// Most hints ever shown at once.
pub const MAX_HINTS: usize = 5;

/// Which surface the bar sits under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintMode {
    List,
    /// Number of selected messages.
    Selection(usize),
    Reader,
    Compose,
    Palette,
    /// Triage session; `true` once the end card is showing.
    Session(bool),
    NewSenders,
    Settings,
    Rules,
    Snooze,
}

/// Everything the hint choice depends on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HintContext {
    pub mode: HintMode,
    /// The list shows no messages.
    pub empty: bool,
    /// The focused message's sender is not yet screened.
    pub new_sender: bool,
    /// The focused message has pending AI suggestions.
    pub suggestions: bool,
    /// The current location is not an inbox (archive, snoozed, trash, sent, folder).
    pub outside_inbox: bool,
    /// The focused message's thread has more than one message.
    pub multi_message: bool,
}

impl HintContext {
    pub fn new(mode: HintMode) -> Self {
        Self {
            mode,
            empty: false,
            new_sender: false,
            suggestions: false,
            outside_inbox: false,
            multi_message: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hint {
    pub key: &'static str,
    pub label: &'static str,
}

/// Drop rank: higher is dropped first; 0 is never dropped.
type Ranked = (Hint, u8);

const fn h(key: &'static str, label: &'static str, rank: u8) -> Ranked {
    (Hint { key, label }, rank)
}

const HELP: Ranked = h("?", "help", 0);
const ESC_CLOSE: Ranked = h("escape", "close", 0);
const ESC_CANCEL: Ranked = h("escape", "cancel", 0);

/// Triage verbs in display order, with their drop ranks (archive > delete > snooze > summarize > reply).
fn verbs(reply: bool) -> impl Iterator<Item = Ranked> {
    [h("e", "archive", 4), h("s", "snooze", 6), h("d", "delete", 5)]
        .into_iter()
        .chain(reply.then(|| h("r", "reply", 8)))
}

fn contextual(ctx: &HintContext, out: &mut Vec<Ranked>) {
    if ctx.suggestions {
        out.push(h("y", "accept", 1));
        out.push(h("n", "reject", 1));
    }
    if ctx.new_sender {
        out.push(h("a", "allow", 2));
        out.push(h("b", "block", 2));
    }
    if ctx.outside_inbox {
        out.push(h("i", "inbox", 3));
    }
}

fn ranked(ctx: &HintContext) -> Vec<Ranked> {
    let mut v = Vec::new();
    match ctx.mode {
        HintMode::List if ctx.empty => {
            v.push(h("g", "go to", 1));
            v.push(HELP);
        }
        HintMode::List => {
            contextual(ctx, &mut v);
            v.extend(verbs(true));
            if ctx.multi_message {
                v.push(h("z", "summarize", 7));
            }
            v.push(HELP);
        }
        HintMode::Selection(_) => {
            contextual(ctx, &mut v);
            v.extend(verbs(false));
            v.push(h("escape", "clear", 0));
        }
        HintMode::Reader => {
            contextual(ctx, &mut v);
            v.push(h("r", "reply", 8));
            v.extend(verbs(false));
            if ctx.multi_message {
                v.push(h("z", "summarize", 7));
            }
            v.push(HELP);
        }
        HintMode::Compose => {
            v.push(h("cmd-enter", "send", 0));
            v.push(ESC_CANCEL);
        }
        HintMode::Palette => {
            v.push(h("enter", "run", 0));
            v.push(ESC_CLOSE);
        }
        HintMode::Session(true) => v.push(ESC_CLOSE),
        HintMode::Session(false) => {
            v.extend(verbs(false));
            if ctx.outside_inbox {
                v.push(h("i", "inbox", 3));
            }
            v.push(h("escape", "end", 0));
        }
        HintMode::NewSenders if ctx.empty => v.push(HELP),
        HintMode::NewSenders => {
            v.push(h("a", "allow", 0));
            v.push(h("b", "block", 0));
            v.push(HELP);
        }
        HintMode::Settings => {
            v.push(h("space", "auto/review", 1));
            v.push(h("=", "threshold +", 2));
            v.push(h("-", "threshold -", 2));
            v.push(ESC_CLOSE);
        }
        HintMode::Rules => {
            v.push(h("backspace", "revoke", 0));
            v.push(ESC_CLOSE);
        }
        HintMode::Snooze => {
            v.push(h("1", "tonight", 1));
            v.push(h("2", "tomorrow", 2));
            v.push(h("3", "monday", 3));
            v.push(h("4", "custom", 4));
            v.push(ESC_CANCEL);
        }
    }
    v
}

/// Drop the highest-ranked (last among ties) hint that may be dropped. False if none can.
fn drop_one(v: &mut Vec<Ranked>) -> bool {
    let worst = v
        .iter()
        .enumerate()
        .filter(|(_, (_, r))| *r > 0)
        .max_by_key(|(i, (_, r))| (*r, *i))
        .map(|(i, _)| i);
    match worst {
        Some(i) => {
            v.remove(i);
            true
        }
        None => false,
    }
}

/// The ordered hints for `ctx`, at most [`MAX_HINTS`].
pub fn hints(ctx: &HintContext) -> Vec<Hint> {
    let mut v = ranked(ctx);
    while v.len() > MAX_HINTS && drop_one(&mut v) {}
    v.into_iter().map(|(hint, _)| hint).collect()
}

/// Estimated rendered width in px of one hint chip (keys, label, gap).
pub fn hint_width(hint: &Hint) -> f32 {
    const CHAR_W: f32 = 6.5;
    const KEY_PAD: f32 = 14.;
    const GAP: f32 = 16.;
    let key_chars: usize = hint.key.split_whitespace().map(|k| if k.len() > 1 && k != "?" { k.len().min(6) } else { 1 }).sum();
    key_chars as f32 * CHAR_W + KEY_PAD + hint.label.len() as f32 * CHAR_W + GAP
}

/// [`hints`], then the lowest-ranked dropped until the bar fits `budget` px
/// (`reserved` px are already taken, e.g. by the "N selected" label).
pub fn fit_hints(ctx: &HintContext, budget: f32, reserved: f32) -> Vec<Hint> {
    let mut v = ranked(ctx);
    let width = |v: &[Ranked]| reserved + v.iter().map(|(h, _)| hint_width(h)).sum::<f32>();
    while (v.len() > MAX_HINTS || width(&v) > budget) && drop_one(&mut v) {}
    v.into_iter().map(|(hint, _)| hint).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(ctx: &HintContext) -> Vec<&'static str> {
        hints(ctx).iter().map(|h| h.key).collect()
    }

    fn list() -> HintContext {
        HintContext::new(HintMode::List)
    }

    #[test]
    fn never_more_than_cap_and_never_undo() {
        let all = HintContext {
            new_sender: true,
            suggestions: true,
            outside_inbox: true,
            multi_message: true,
            ..list()
        };
        let modes = [
            HintMode::List,
            HintMode::Selection(3),
            HintMode::Reader,
            HintMode::Session(false),
            HintMode::Settings,
            HintMode::Snooze,
            HintMode::NewSenders,
        ];
        for mode in modes {
            let k = keys(&HintContext { mode, ..all });
            assert!(k.len() <= MAX_HINTS, "{mode:?}: {k:?}");
            assert!(!k.contains(&"u"), "{mode:?}: {k:?}");
        }
    }

    #[test]
    fn contextual_keys_only_when_they_apply() {
        let plain = keys(&list());
        for k in ["a", "b", "y", "n", "i", "z"] {
            assert!(!plain.contains(&k), "{k} in {plain:?}");
        }
        assert_eq!(plain.last(), Some(&"?"));

        let new = keys(&HintContext { new_sender: true, ..list() });
        assert_eq!(&new[..2], ["a", "b"]);
        let away = keys(&HintContext { outside_inbox: true, ..list() });
        assert_eq!(away[0], "i");
        let multi = keys(&HintContext { multi_message: true, ..list() });
        assert!(multi.contains(&"z"));
    }

    #[test]
    fn priority_truncation_keeps_contextual_and_help() {
        let k = keys(&HintContext { suggestions: true, ..HintContext::new(HintMode::Reader) });
        assert_eq!(k, ["y", "n", "e", "d", "?"]);
        let k = keys(&HintContext { new_sender: true, ..list() });
        assert_eq!(k, ["a", "b", "e", "d", "?"]);
    }

    #[test]
    fn empty_list_has_no_triage_keys() {
        let k = keys(&HintContext { empty: true, new_sender: true, ..list() });
        assert_eq!(k, ["g", "?"]);
    }

    #[test]
    fn narrow_width_drops_lowest_priority_but_keeps_help() {
        let ctx = list();
        let wide = fit_hints(&ctx, 2000., 0.);
        assert_eq!(wide.len(), 5);
        let narrow = fit_hints(&ctx, 200., 0.);
        assert!(narrow.len() < wide.len());
        assert_eq!(narrow.last().map(|h| h.key), Some("?"));
        assert!(narrow.iter().any(|h| h.key == "e"), "archive outranks reply");
        let tiny = fit_hints(&ctx, 1., 0.);
        assert_eq!(tiny.iter().map(|h| h.key).collect::<Vec<_>>(), ["?"]);
    }
}
