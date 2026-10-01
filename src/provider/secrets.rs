use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

pub trait SecretStore {
    fn get(&self, account: &str) -> Option<String>;
    fn set(&self, account: &str, secret: &str) -> Result<(), String>;
    /// Forget the secret; a missing entry is not an error.
    fn delete(&self, account: &str) -> Result<(), String>;
}

/// One raw keychain item per name. Kept apart from `SecretStore` so the
/// bundling logic can be tested without the real keychain.
trait RawKeychain {
    fn read(&self, name: &str) -> Result<Option<String>, String>;
    fn write(&self, name: &str, value: &str) -> Result<(), String>;
    /// A missing item is not an error.
    fn remove(&self, name: &str) -> Result<(), String>;
}

const SERVICE: &str = "mail-classifier";
/// The single item holding every account's secret as a JSON object. Each
/// separate keychain item costs its own access prompt on an unsigned binary.
const BUNDLE: &str = "accounts";

struct KeyringRaw;

impl RawKeychain for KeyringRaw {
    fn read(&self, name: &str) -> Result<Option<String>, String> {
        match keyring::Entry::new(SERVICE, name).and_then(|e| e.get_password()) {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn write(&self, name: &str, value: &str) -> Result<(), String> {
        keyring::Entry::new(SERVICE, name)
            .and_then(|e| e.set_password(value))
            .map_err(|e| e.to_string())
    }

    fn remove(&self, name: &str) -> Result<(), String> {
        match keyring::Entry::new(SERVICE, name).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// All secrets in one keychain item, read at most once per process (then
/// served from memory) and rewritten only when something changed. Secrets
/// saved by older versions as one item per account migrate on first lookup.
struct Bundle<R> {
    raw: R,
    cache: Mutex<Option<HashMap<String, String>>>,
}

impl<R: RawKeychain> Bundle<R> {
    fn new(raw: R) -> Self {
        Self { raw, cache: Mutex::new(None) }
    }

    /// Runs `f` on the loaded map. A failed read (e.g. access denied) is not cached.
    fn with<T>(&self, f: impl FnOnce(&R, &mut HashMap<String, String>) -> T) -> Result<T, String> {
        let mut guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            let map = match self.raw.read(BUNDLE)? {
                Some(json) => serde_json::from_str(&json).map_err(|e| format!("corrupt keychain item: {e}"))?,
                None => HashMap::new(),
            };
            *guard = Some(map);
        }
        Ok(f(&self.raw, guard.as_mut().expect("loaded above")))
    }

    fn save(raw: &R, map: &HashMap<String, String>) -> Result<(), String> {
        if map.is_empty() {
            return raw.remove(BUNDLE);
        }
        let json = serde_json::to_string(map).map_err(|e| e.to_string())?;
        raw.write(BUNDLE, &json)
    }
}

impl<R: RawKeychain> SecretStore for Bundle<R> {
    fn get(&self, account: &str) -> Option<String> {
        self.with(|raw, map| {
            if let Some(secret) = map.get(account) {
                return Some(secret.clone());
            }
            let legacy = raw.read(account).ok().flatten()?;
            map.insert(account.to_owned(), legacy.clone());
            // Drop the old item only once the bundle holds the secret.
            if Self::save(raw, map).is_ok() {
                let _ = raw.remove(account);
            } else {
                map.remove(account);
            }
            Some(legacy)
        })
        .ok()
        .flatten()
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        self.with(|raw, map| {
            if map.get(account).map(String::as_str) == Some(secret) {
                return Ok(());
            }
            let previous = map.insert(account.to_owned(), secret.to_owned());
            Self::save(raw, map).inspect_err(|_| match previous {
                Some(p) => drop(map.insert(account.to_owned(), p)),
                None => drop(map.remove(account)),
            })?;
            let _ = raw.remove(account); // stale pre-bundle item, if any
            Ok(())
        })?
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        self.with(|raw, map| {
            raw.remove(account)?;
            if map.remove(account).is_some() {
                Self::save(raw, map)?;
            }
            Ok(())
        })?
    }
}

/// OS keychain via `keyring`. All handles share one process-wide cache.
pub struct KeyringStore;

fn shared() -> &'static Bundle<KeyringRaw> {
    static SHARED: OnceLock<Bundle<KeyringRaw>> = OnceLock::new();
    SHARED.get_or_init(|| Bundle::new(KeyringRaw))
}

impl SecretStore for KeyringStore {
    fn get(&self, account: &str) -> Option<String> {
        shared().get(account)
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        shared().set(account, secret)
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        shared().delete(account)
    }
}

/// In-memory store for tests.
#[derive(Default)]
pub struct MemorySecrets(RefCell<HashMap<String, String>>);

impl SecretStore for MemorySecrets {
    fn get(&self, account: &str) -> Option<String> {
        self.0.borrow().get(account).cloned()
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        self.0.borrow_mut().insert(account.to_owned(), secret.to_owned());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        self.0.borrow_mut().remove(account);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    /// Fake keychain counting reads/writes; clones share state.
    #[derive(Clone, Default)]
    struct FakeRaw {
        items: Rc<RefCell<HashMap<String, String>>>,
        reads: Rc<RefCell<usize>>,
        writes: Rc<RefCell<usize>>,
    }

    impl RawKeychain for FakeRaw {
        fn read(&self, name: &str) -> Result<Option<String>, String> {
            *self.reads.borrow_mut() += 1;
            Ok(self.items.borrow().get(name).cloned())
        }
        fn write(&self, name: &str, value: &str) -> Result<(), String> {
            *self.writes.borrow_mut() += 1;
            self.items.borrow_mut().insert(name.to_owned(), value.to_owned());
            Ok(())
        }
        fn remove(&self, name: &str) -> Result<(), String> {
            self.items.borrow_mut().remove(name);
            Ok(())
        }
    }

    // `Bundle` holds a `Mutex`, which needs `Send` only for statics; the fake is single-threaded.
    #[test]
    fn many_lookups_read_the_keychain_bundle_once() {
        let raw = FakeRaw::default();
        let store = Bundle::new(raw.clone());
        store.set("a@x.com", "ta").unwrap();
        store.set("b@x.com", "tb").unwrap();
        let reads_after_sets = *raw.reads.borrow();
        for _ in 0..5 {
            assert_eq!(store.get("a@x.com").as_deref(), Some("ta"));
            assert_eq!(store.get("b@x.com").as_deref(), Some("tb"));
        }
        assert_eq!(*raw.reads.borrow(), reads_after_sets);
        assert_eq!(reads_after_sets, 1);
    }

    #[test]
    fn setting_an_unchanged_secret_does_not_rewrite_the_keychain() {
        let raw = FakeRaw::default();
        let store = Bundle::new(raw.clone());
        store.set("a@x.com", "ta").unwrap();
        store.set("a@x.com", "ta").unwrap();
        assert_eq!(*raw.writes.borrow(), 1);
    }

    #[test]
    fn legacy_per_account_items_migrate_into_the_bundle() {
        let raw = FakeRaw::default();
        raw.items.borrow_mut().insert("a@x.com".into(), "old".into());
        let store = Bundle::new(raw.clone());
        assert_eq!(store.get("a@x.com").as_deref(), Some("old"));
        assert!(!raw.items.borrow().contains_key("a@x.com"), "legacy item removed");
        let fresh = Bundle::new(raw.clone());
        assert_eq!(fresh.get("a@x.com").as_deref(), Some("old"));
    }

    #[test]
    fn delete_forgets_the_secret_and_drops_an_empty_bundle() {
        let raw = FakeRaw::default();
        let store = Bundle::new(raw.clone());
        store.set("a@x.com", "ta").unwrap();
        store.delete("a@x.com").unwrap();
        assert_eq!(store.get("a@x.com"), None);
        assert!(raw.items.borrow().is_empty());
        store.delete("a@x.com").unwrap();
    }

    #[test]
    fn a_missing_secret_is_none() {
        let store = Bundle::new(FakeRaw::default());
        assert_eq!(store.get("nobody@x.com"), None);
    }
}
