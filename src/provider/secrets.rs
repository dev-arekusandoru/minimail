use std::cell::RefCell;
use std::collections::HashMap;

pub trait SecretStore {
    fn get(&self, account: &str) -> Option<String>;
    fn set(&self, account: &str, secret: &str) -> Result<(), String>;
    /// Forget the secret; a missing entry is not an error.
    fn delete(&self, account: &str) -> Result<(), String>;
}

/// OS keychain via `keyring`.
pub struct KeyringStore;

const SERVICE: &str = "mail-classifier";

impl SecretStore for KeyringStore {
    fn get(&self, account: &str) -> Option<String> {
        keyring::Entry::new(SERVICE, account)
            .ok()?
            .get_password()
            .ok()
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        keyring::Entry::new(SERVICE, account)
            .and_then(|e| e.set_password(secret))
            .map_err(|e| e.to_string())
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        match keyring::Entry::new(SERVICE, account).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
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
