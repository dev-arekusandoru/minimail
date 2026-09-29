//! Sender rules learned from repeated sender-wide triage actions.

use std::collections::{HashMap, HashSet};

use crate::model::TriageState;

/// "Always move mail from `sender` to `state`". `sender` is lowercase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub sender: String,
    pub state: TriageState,
}

#[derive(Default, Debug)]
pub struct RuleBook {
    rules: Vec<Rule>,
    /// Per sender: last sender-wide action and its consecutive streak.
    streaks: HashMap<String, (TriageState, u32)>,
    dismissed: HashSet<(String, TriageState)>,
}

impl RuleBook {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a sender-wide action. Returns a suggestion on the 2nd consecutive
    /// identical action for a sender, unless that exact rule was dismissed or
    /// the sender already has a rule. Sender matching is case-insensitive.
    pub fn record(&mut self, sender: &str, state: TriageState) -> Option<Rule> {
        let sender = sender.trim().to_lowercase();
        let streak = match self.streaks.get(&sender) {
            Some(&(s, n)) if s == state => n + 1,
            _ => 1,
        };
        self.streaks.insert(sender.clone(), (state, streak));
        if streak != 2
            || self.rules.iter().any(|r| r.sender == sender)
            || self.dismissed.contains(&(sender.clone(), state))
        {
            return None;
        }
        Some(Rule { sender, state })
    }

    /// Adopt a rule; replaces any existing rule for the same sender.
    pub fn accept(&mut self, rule: Rule) {
        let sender = rule.sender.to_lowercase();
        let rule = Rule { sender, ..rule };
        match self.rules.iter_mut().find(|r| r.sender == rule.sender) {
            Some(existing) => *existing = rule,
            None => self.rules.push(rule),
        }
    }

    /// Never suggest this rule again.
    pub fn dismiss(&mut self, rule: Rule) {
        self.dismissed
            .insert((rule.sender.to_lowercase(), rule.state));
    }

    /// Remove the rule at `index`; out of range returns `None`.
    pub fn revoke(&mut self, index: usize) -> Option<Rule> {
        (index < self.rules.len()).then(|| self.rules.remove(index))
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    pub fn rule_for(&self, sender: &str) -> Option<TriageState> {
        let sender = sender.trim().to_lowercase();
        self.rules
            .iter()
            .find(|r| r.sender == sender)
            .map(|r| r.state)
    }
}
