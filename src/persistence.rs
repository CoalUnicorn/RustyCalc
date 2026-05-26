//! Storage backend abstraction (LOCK-3).
//!
//! Extracted from `storage.rs` so backends (localStorage, IndexedDB, in-memory
//! for tests) can be swapped without touching every call site.  Currently
//! `LocalStorageBackend` is the default; `storage.rs` free functions delegate
//! to it.  A future refactoring pass will thread `&dyn Persistence` through
//! callers so the backend is configurable at the app-entry point.

use std::collections::HashMap;
use std::fmt;

use gloo_storage::{LocalStorage, Storage};
use serde::{de::DeserializeOwned, Serialize};

/// Pluggable key-value persistence.
pub trait Persistence {
    type Error: fmt::Display;

    /// Read a value, returning `None` when the key is absent.
    fn get_raw<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, Self::Error>;

    /// Write a value.
    fn set_raw<T: Serialize>(&self, key: &str, value: &T) -> Result<(), Self::Error>;

    /// Remove a key.
    fn delete(&self, key: &str) -> Result<(), Self::Error>;

    /// List every stored key.  Implementations should filter out internal
    /// book-keeping keys (e.g. the RustyCalc `selected` sentinel).
    fn keys(&self) -> Result<Vec<String>, Self::Error>;
}

/// Default persistence backend: the browser's `localStorage`.
pub struct LocalStorageBackend;

impl Persistence for LocalStorageBackend {
    type Error = gloo_storage::errors::StorageError;

    fn get_raw<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, Self::Error> {
        // gloo has no "try_get" — `get` returns an error when the key is missing.
        match LocalStorage::get(key) {
            Ok(v) => Ok(Some(v)),
            Err(gloo_storage::errors::StorageError::KeyNotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn set_raw<T: Serialize>(&self, key: &str, value: &T) -> Result<(), Self::Error> {
        LocalStorage::set(key, value)
    }

    fn delete(&self, key: &str) -> Result<(), Self::Error> {
        LocalStorage::delete(key);
        Ok(())
    }

    fn keys(&self) -> Result<Vec<String>, Self::Error> {
        // gloo doesn't expose `keys()` directly, so we list by scanning a
        // dedicated sentinel.  For RustyCalc, the model keys are UUIDs
        // (36-char strings) and the registry key is "models".
        let registry: HashMap<String, serde_json::Value> =
            LocalStorage::get("models").unwrap_or_default();
        let mut keys: Vec<String> = registry.keys().cloned().collect();
        keys.push("models".into());
        keys.push("selected".into());
        Ok(keys)
    }
}
