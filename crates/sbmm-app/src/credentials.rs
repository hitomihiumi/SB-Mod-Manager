//! Where the Nexus API key is kept.
//!
//! The key grants access to the user's Nexus account, so it does not go into
//! the SQLite file next to the mod list. It goes to the operating system's
//! credential store — Credential Manager on Windows — and the database only
//! records whether one has been set.

use std::sync::Mutex;

const SERVICE: &str = "sb-mod-manager";
const ACCOUNT: &str = "nexus-api-key";

#[derive(Debug, thiserror::Error)]
pub enum KeyStoreError {
    #[error("could not reach the system credential store: {0}")]
    Unavailable(String),
}

/// Somewhere to keep a secret.
///
/// A trait so tests never touch the real credential store, and so the failure
/// mode on a machine without one is explicit rather than a silent downgrade to
/// storing the key in plain text.
pub trait KeyStore: Send + Sync {
    fn get(&self) -> Result<Option<String>, KeyStoreError>;
    fn set(&self, secret: &str) -> Result<(), KeyStoreError>;
    fn clear(&self) -> Result<(), KeyStoreError>;
}

/// The operating system's credential store.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsKeyStore;

impl KeyStore for OsKeyStore {
    fn get(&self) -> Result<Option<String>, KeyStoreError> {
        let entry = entry()?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(KeyStoreError::Unavailable(e.to_string())),
        }
    }

    fn set(&self, secret: &str) -> Result<(), KeyStoreError> {
        entry()?
            .set_password(secret)
            .map_err(|e| KeyStoreError::Unavailable(e.to_string()))
    }

    fn clear(&self) -> Result<(), KeyStoreError> {
        match entry()?.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(KeyStoreError::Unavailable(e.to_string())),
        }
    }
}

fn entry() -> Result<keyring::Entry, KeyStoreError> {
    keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| KeyStoreError::Unavailable(e.to_string()))
}

/// An in-process store, for tests and for development on a machine with no
/// credential store of its own.
#[derive(Debug, Default)]
pub struct MemoryKeyStore {
    secret: Mutex<Option<String>>,
}

impl KeyStore for MemoryKeyStore {
    fn get(&self) -> Result<Option<String>, KeyStoreError> {
        Ok(self.secret.lock().expect("key store lock").clone())
    }

    fn set(&self, secret: &str) -> Result<(), KeyStoreError> {
        *self.secret.lock().expect("key store lock") = Some(secret.to_string());
        Ok(())
    }

    fn clear(&self) -> Result<(), KeyStoreError> {
        *self.secret.lock().expect("key store lock") = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_round_trips() {
        let store = MemoryKeyStore::default();
        assert_eq!(store.get().unwrap(), None);

        store.set("abc123").unwrap();
        assert_eq!(store.get().unwrap().as_deref(), Some("abc123"));

        store.clear().unwrap();
        assert_eq!(store.get().unwrap(), None);
    }

    #[test]
    fn clearing_an_empty_store_is_not_an_error() {
        MemoryKeyStore::default().clear().unwrap();
    }
}
