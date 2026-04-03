//! **Enhanced Storage System**
//!
//! Practical improvements to the existing storage.rs with better error handling,
//! user feedback, and storage optimization. Works with existing codebase.

use base64::{engine::general_purpose::STANDARD, Engine};
use gloo_storage::{LocalStorage, Storage};
use ironcalc_base::UserModel;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Enhanced storage statistics for monitoring and optimization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageStats {
    pub total_workbooks: usize,
    pub total_size_bytes: usize,
    pub last_save_timestamp: u64,
    pub save_count: usize,
    pub load_count: usize,
}

impl Default for StorageStats {
    fn default() -> Self {
        Self {
            total_workbooks: 0,
            total_size_bytes: 0,
            last_save_timestamp: js_sys::Date::now() as u64,
            save_count: 0,
            load_count: 0,
        }
    }
}

/// Enhanced workbook metadata with timestamps and size tracking
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct EnhancedWorkbookMeta {
    pub name: String,
    pub created_at: u64,    // JS timestamp
    pub last_modified: u64, // JS timestamp
    pub size_bytes: usize,
    pub version: u32, // For future migration support
}

#[allow(dead_code)]
impl EnhancedWorkbookMeta {
    pub fn new(name: String, size_bytes: usize) -> Self {
        let now = js_sys::Date::now() as u64;
        Self {
            name,
            created_at: now,
            last_modified: now,
            size_bytes,
            version: 1,
        }
    }

    pub fn update_modified(&mut self, new_size: usize) {
        self.last_modified = js_sys::Date::now() as u64;
        self.size_bytes = new_size;
    }

    /// Format timestamp for display
    pub fn format_last_modified(&self) -> String {
        let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(self.last_modified as f64));
        date.to_locale_string("en-US", &js_sys::Object::new())
            .as_string()
            .unwrap_or_else(|| "Unknown".to_string())
    }

    /// Calculate age in minutes
    pub fn age_minutes(&self) -> u64 {
        let now = js_sys::Date::now() as u64;
        (now - self.last_modified) / (1000 * 60) // Convert ms to minutes
    }
}

// Enhanced constants
const STATS_KEY: &str = "storage_stats";
const ENHANCED_MODELS_KEY: &str = "enhanced_models";
const MAX_STORAGE_SIZE: usize = 10 * 1024 * 1024; // 10MB limit (typical localStorage limit is 5-10MB)

/// Enhanced error handling with user-friendly messages
#[allow(dead_code)]
pub enum StorageError {
    QuotaExceeded {
        current_size: usize,
        attempted_size: usize,
    },
    CorruptedData {
        uuid: String,
        error: String,
    },
    NetworkError(String),
}

impl StorageError {
    pub fn user_message(&self) -> String {
        match self {
            StorageError::QuotaExceeded {
                current_size,
                attempted_size,
            } => {
                format!(
                    "Storage full! Using {:.1}MB of ~10MB. This workbook needs {:.1}MB more.",
                    *current_size as f64 / 1024.0 / 1024.0,
                    *attempted_size as f64 / 1024.0 / 1024.0
                )
            }
            StorageError::CorruptedData { uuid, error: _ } => {
                format!("Workbook {} is corrupted and cannot be opened.", uuid)
            }
            StorageError::NetworkError(msg) => {
                format!("Connection issue: {}", msg)
            }
        }
    }
}

/// Get current storage statistics
pub fn get_storage_stats() -> StorageStats {
    LocalStorage::get(STATS_KEY).unwrap_or_default()
}

/// Update storage statistics
fn update_stats<F>(updater: F)
where
    F: FnOnce(&mut StorageStats),
{
    let mut stats = get_storage_stats();
    updater(&mut stats);
    let _ = LocalStorage::set(STATS_KEY, &stats);
}

/// Get enhanced workbook registry
pub fn get_enhanced_registry() -> HashMap<String, EnhancedWorkbookMeta> {
    LocalStorage::get(ENHANCED_MODELS_KEY).unwrap_or_default()
}

/// Save enhanced registry
fn save_enhanced_registry(registry: &HashMap<String, EnhancedWorkbookMeta>) {
    let _ = LocalStorage::set(ENHANCED_MODELS_KEY, registry);
}

/// **Enhanced save with size monitoring and quotas**
pub fn save_enhanced(uuid: &str, model: &UserModel) -> Result<(), StorageError> {
    let bytes = model.to_bytes();
    let encoded = STANDARD.encode(&bytes);
    let size = encoded.len();

    // Check storage quota
    let current_stats = get_storage_stats();
    if current_stats.total_size_bytes + size > MAX_STORAGE_SIZE {
        return Err(StorageError::QuotaExceeded {
            current_size: current_stats.total_size_bytes,
            attempted_size: size,
        });
    }

    // Save the data
    LocalStorage::set(uuid, encoded)
        .map_err(|e| StorageError::NetworkError(format!("Save failed: {}", e)))?;

    // Update enhanced registry
    let mut registry = get_enhanced_registry();
    registry
        .entry(uuid.to_string())
        .and_modify(|m| m.update_modified(size))
        .or_insert_with(|| EnhancedWorkbookMeta::new(model.get_name(), size));
    save_enhanced_registry(&registry);

    // Update statistics
    update_stats(|stats| {
        stats.save_count += 1;
        stats.last_save_timestamp = js_sys::Date::now() as u64;
        stats.total_workbooks = registry.len();
        stats.total_size_bytes = registry.values().map(|m| m.size_bytes).sum();
    });

    Ok(())
}

/// **Enhanced load with better error handling**
#[allow(dead_code)]
pub fn load_enhanced(uuid: &str) -> Result<UserModel<'static>, StorageError> {
    let encoded: String = LocalStorage::get(uuid)
        .map_err(|e| StorageError::NetworkError(format!("Load failed: {}", e)))?;

    let bytes = STANDARD
        .decode(encoded)
        .map_err(|e| StorageError::CorruptedData {
            uuid: uuid.to_string(),
            error: format!("Base64 decode: {}", e),
        })?;

    let model = UserModel::from_bytes(&bytes, "en").map_err(|e| StorageError::CorruptedData {
        uuid: uuid.to_string(),
        error: format!("Model parse: {}", e),
    })?;

    // Update load statistics
    update_stats(|stats| {
        stats.load_count += 1;
    });

    Ok(model)
}

/// **Storage cleanup utilities**
#[allow(dead_code)]
pub fn cleanup_old_workbooks(max_age_days: u64) -> usize {
    let registry = get_enhanced_registry();
    let now = js_sys::Date::now() as u64;
    let max_age_ms = max_age_days * 24 * 60 * 60 * 1000;

    let mut cleaned = 0;
    let mut updated_registry = registry.clone();

    for (uuid, meta) in &registry {
        if now - meta.last_modified > max_age_ms {
            LocalStorage::delete(uuid);
            updated_registry.remove(uuid);
            cleaned += 1;
        }
    }

    if cleaned > 0 {
        save_enhanced_registry(&updated_registry);
        update_stats(|stats| {
            stats.total_workbooks = updated_registry.len();
            stats.total_size_bytes = updated_registry.values().map(|m| m.size_bytes).sum();
        });
    }

    cleaned
}

/// **Storage analysis for optimization**
pub fn analyze_storage() -> String {
    let stats = get_storage_stats();
    let registry = get_enhanced_registry();

    let total_mb = stats.total_size_bytes as f64 / 1024.0 / 1024.0;
    let avg_size = if !registry.is_empty() {
        stats.total_size_bytes / registry.len()
    } else {
        0
    };

    let oldest_workbook = registry
        .values()
        .map(|meta| meta.age_minutes())
        .max()
        .unwrap_or(0);

    format!(
        "Storage Analysis:\n\
         • Total: {:.1}MB / 10.0MB ({:.1}% used)\n\
         • Workbooks: {} (avg {:.1}KB each)\n\
         • Operations: {} saves, {} loads\n\
         • Oldest: {} minutes ago",
        total_mb,
        (total_mb / 10.0) * 100.0,
        registry.len(),
        avg_size as f64 / 1024.0,
        stats.save_count,
        stats.load_count,
        oldest_workbook
    )
}

/// Get workbook metadata by UUID
#[allow(dead_code)]
pub fn get_workbook_metadata(uuid: &str) -> Result<EnhancedWorkbookMeta, StorageError> {
    let registry = crate::storage::load_registry();
    registry
        .get(uuid)
        .cloned()
        .map(|meta| EnhancedWorkbookMeta::new(meta.name, 0)) // Convert from basic to enhanced
        .ok_or_else(|| StorageError::CorruptedData {
            uuid: uuid.to_string(),
            error: "Workbook metadata not found in registry".to_string(),
        })
}

/// **Backward compatibility layer**
/// These functions wrap the enhanced versions for drop-in replacement
pub fn save_compatible(uuid: &str, model: &UserModel) {
    match save_enhanced(uuid, model) {
        Ok(()) => {
            web_sys::console::info_1(&format!("Saved {}", model.get_name()).into());
        }
        Err(e) => {
            web_sys::console::error_1(&format!("❌ Save failed: {}", e.user_message()).into());
            // Fallback to original save method
            crate::storage::save(uuid, model);
        }
    }
}

#[allow(dead_code)]
pub fn load_compatible(uuid: &str) -> Option<UserModel<'static>> {
    match load_enhanced(uuid) {
        Ok(model) => Some(model),
        Err(e) => {
            web_sys::console::warn_1(
                &format!(
                    "Enhanced load failed, trying fallback: {}",
                    e.user_message()
                )
                .into(),
            );
            // Fallback to original load method
            crate::storage::load(uuid)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enhanced_meta_formatting() {
        let meta = EnhancedWorkbookMeta::new("Test Book".to_string(), 1024);
        assert_eq!(meta.name, "Test Book");
        assert_eq!(meta.size_bytes, 1024);
        assert!(meta.age_minutes() == 0); // Just created
    }

    #[test]
    fn storage_error_messages() {
        let error = StorageError::QuotaExceeded {
            current_size: 9_000_000,
            attempted_size: 2_000_000,
        };
        let message = error.user_message();
        assert!(message.contains("Storage full"));
        assert!(message.contains("8.6MB")); // ~9MB in user message
    }
}
