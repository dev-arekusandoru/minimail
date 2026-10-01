//! Typed, scoped user preferences over a pluggable key/value store.
//!
//! Only non-default values are stored: a row existing means "modified", and
//! resetting a setting deletes its row. Lookup for an account scope falls
//! back to [`Scope::Global`], then to the built-in default; missing or
//! malformed JSON is treated as absent.
//!
//! [`PrefStore`] is deliberately infallible: a preference that cannot be read
//! behaves like the default, and a failed write must not take the app down.
//! [`ContactStore`] reports such failures on stderr; call its fallible
//! `setting_*` methods directly when the error matters.

use std::cell::RefCell;
use std::collections::BTreeMap;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::contacts::ContactStore;

/// Where a value applies.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scope {
    Global,
    Account(String),
}

impl Scope {
    /// Stored form: `"global"` or `"account:<id>"`.
    pub fn as_key(&self) -> String {
        match self {
            Scope::Global => "global".to_string(),
            Scope::Account(id) => format!("account:{id}"),
        }
    }
}

/// Raw JSON key/value storage keyed by scope.
pub trait PrefStore {
    fn get_raw(&self, scope: &Scope, key: &str) -> Option<String>;
    fn set_raw(&self, scope: &Scope, key: &str, json: &str);
    fn delete(&self, scope: &Scope, key: &str);
    /// Remove every setting in every scope.
    fn clear(&self);
}

fn report(op: &str, result: crate::contacts::Result<()>) {
    if let Err(e) = result {
        eprintln!("settings: {op} failed: {e}");
    }
}

impl PrefStore for ContactStore {
    fn get_raw(&self, scope: &Scope, key: &str) -> Option<String> {
        match self.setting_get(&scope.as_key(), key) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("settings: read of `{key}` failed: {e}");
                None
            }
        }
    }

    fn set_raw(&self, scope: &Scope, key: &str, json: &str) {
        report("write", self.setting_set(&scope.as_key(), key, json));
    }

    fn delete(&self, scope: &Scope, key: &str) {
        report("delete", self.setting_delete(&scope.as_key(), key));
    }

    fn clear(&self) {
        report("clear", self.settings_clear());
    }
}

/// In-memory [`PrefStore`] for tests.
#[derive(Debug, Default)]
pub struct MemoryPrefs {
    rows: RefCell<BTreeMap<(String, String), String>>,
}

impl MemoryPrefs {
    pub fn new() -> Self {
        Self::default()
    }
}

impl PrefStore for MemoryPrefs {
    fn get_raw(&self, scope: &Scope, key: &str) -> Option<String> {
        self.rows.borrow().get(&(scope.as_key(), key.to_string())).cloned()
    }

    fn set_raw(&self, scope: &Scope, key: &str, json: &str) {
        self.rows
            .borrow_mut()
            .insert((scope.as_key(), key.to_string()), json.to_string());
    }

    fn delete(&self, scope: &Scope, key: &str) {
        self.rows.borrow_mut().remove(&(scope.as_key(), key.to_string()));
    }

    fn clear(&self) {
        self.rows.borrow_mut().clear();
    }
}

/// A typed setting: storage key plus built-in default. Usable in `const`s.
#[derive(Debug, Clone, Copy)]
pub struct Setting<T> {
    pub key: &'static str,
    pub default: T,
}

impl<T> Setting<T> {
    pub const fn new(key: &'static str, default: T) -> Self {
        Self { key, default }
    }
}

impl<T: Serialize + DeserializeOwned + PartialEq + Clone> Setting<T> {
    fn read(&self, store: &dyn PrefStore, scope: &Scope) -> Option<T> {
        let raw = store.get_raw(scope, self.key)?;
        serde_json::from_str(&raw).ok()
    }

    /// Effective value: the scope's own row, else (for accounts) the global
    /// row, else the default. Malformed rows are skipped.
    pub fn get(&self, store: &dyn PrefStore, scope: &Scope) -> T {
        if let Some(v) = self.read(store, scope) {
            return v;
        }
        if *scope != Scope::Global
            && let Some(v) = self.read(store, &Scope::Global)
        {
            return v;
        }
        self.default.clone()
    }

    /// Store `value` at exactly `scope`. A value equal to what the scope
    /// would inherit anyway (the default for global; the effective global
    /// value for an account) deletes the row instead of storing it.
    pub fn set(&self, store: &dyn PrefStore, scope: &Scope, value: T) {
        let inherited = match scope {
            Scope::Global => self.default.clone(),
            Scope::Account(_) => self.get(store, &Scope::Global),
        };
        if value == inherited {
            store.delete(scope, self.key);
            return;
        }
        match serde_json::to_string(&value) {
            Ok(json) => store.set_raw(scope, self.key, &json),
            Err(e) => eprintln!("settings: cannot encode `{}`: {e}", self.key),
        }
    }

    /// Whether a row exists at exactly `scope`.
    pub fn is_modified(&self, store: &dyn PrefStore, scope: &Scope) -> bool {
        store.get_raw(scope, self.key).is_some()
    }

    /// Delete the row at `scope`.
    pub fn reset(&self, store: &dyn PrefStore, scope: &Scope) {
        store.delete(scope, self.key);
    }
}
