//! RCAL workbook snapshots stored as binary IndexedDB records.

use ironcalc_base::UserModel;

use super::registry::{get_selected_uuid, load_registry, sanitize_name, set_selected_uuid};
use super::{LOCALE, SELECTED_KEY, WorkbookId, database};
use gloo_storage::{LocalStorage, Storage};

const STORAGE_MAGIC: &[u8; 4] = b"RCAL";
const STORAGE_VERSION: u8 = 3;
/// Limit both serialization and parsing within the application's Wasm memory budget.
pub const MAX_STORED_BYTES: usize = 64_000_000;

pub async fn initialize() -> Result<(), String> {
    database::initialize()
        .await
        .map_err(|error| format!("Cannot open workbook storage: {}", database::error(error)))?;
    let _: std::collections::HashMap<WorkbookId, super::WorkbookMeta> =
        serde_json::from_str(&database::registry()).map_err(|error| {
            format!("Cannot read workbook registry: {error}. Saved data has been kept.")
        })?;
    Ok(())
}

/// A snapshot owns its UUID and bytes across the asynchronous write.
pub struct WorkbookSnapshot {
    uuid: WorkbookId,
    name: String,
    bytes: Vec<u8>,
}

pub fn snapshot(uuid: WorkbookId, model: &UserModel) -> Result<WorkbookSnapshot, String> {
    let model_bytes = model.to_bytes();
    if model_bytes.len() > MAX_STORED_BYTES - 5 {
        return Err(
            "Cannot save workbook: it exceeds the 64 MB limit. Export it before closing.".into(),
        );
    }
    let mut bytes = Vec::with_capacity(5 + model_bytes.len());
    bytes.extend_from_slice(STORAGE_MAGIC);
    bytes.push(STORAGE_VERSION);
    bytes.extend_from_slice(&model_bytes);
    Ok(WorkbookSnapshot {
        uuid,
        name: sanitize_name(&model.get_name()),
        bytes,
    })
}

impl WorkbookSnapshot {
    pub async fn save(self) -> Result<(), String> {
        self.write("existing").await
    }

    pub async fn rename(self) -> Result<(), String> {
        self.write("rename").await
    }

    async fn write(self, origin: &str) -> Result<(), String> {
        database::save(&self.uuid.to_string(), &self.name, &self.bytes, origin)
            .await
            .map(|_| ())
            .map_err(|error| format!("Cannot save workbook: {}", database::error(error)))
    }
}

pub(super) fn decode(bytes: &[u8]) -> Result<UserModel<'static>, String> {
    if bytes.len() > MAX_STORED_BYTES {
        return Err("Workbook exceeds the 64 MB limit. Its saved data has been kept.".into());
    }
    if bytes.len() < 5 || &bytes[..4] != STORAGE_MAGIC {
        return Err("Invalid workbook header. Its saved data has been kept.".into());
    }
    let from_workbook =
        |workbook| ironcalc_base::Model::from_workbook(workbook, LOCALE).map(UserModel::from_model);
    let mut model = match bytes[4] {
        1 => super::legacy::decode_v1(&bytes[5..]).and_then(from_workbook),
        2 => {
            // The engine update briefly wrote the current layout with a v2 header.
            match super::legacy::decode_v2(&bytes[5..]) {
                Ok(workbook) => from_workbook(workbook),
                Err(_) => UserModel::from_bytes(&bytes[5..], LOCALE),
            }
        }
        STORAGE_VERSION => UserModel::from_bytes(&bytes[5..], LOCALE),
        _ => return Err("Unsupported workbook version. Its saved data has been kept.".into()),
    }
    .map_err(|error| format!("Cannot read workbook: {error}. Its saved data has been kept."))?;
    model.set_name(&sanitize_name(&model.get_name()));
    Ok(model)
}

pub async fn load(uuid: &WorkbookId) -> Result<UserModel<'static>, String> {
    let value = database::load(&uuid.to_string())
        .await
        .map_err(|error| format!("Cannot load workbook: {}", database::error(error)))?;
    if value.is_undefined() || value.is_null() {
        return Err("Workbook bytes are missing. Its registry entry has been kept.".into());
    }
    let bytes = js_sys::Uint8Array::new(&value);
    if bytes.length() as usize > MAX_STORED_BYTES {
        return Err("Workbook exceeds the 64 MB limit. Its saved data has been kept.".into());
    }
    let mut model = decode(&bytes.to_vec())?;
    if let Some(meta) = load_registry().get(uuid) {
        model.set_name(&meta.name);
    }
    Ok(model)
}

pub async fn load_selected() -> Result<Option<(WorkbookId, UserModel<'static>)>, String> {
    let registry = load_registry();
    let selected = get_selected_uuid()
        .filter(|id| registry.contains_key(id))
        .or_else(|| registry.keys().min().copied());
    match selected {
        Some(uuid) => {
            let model = load(&uuid).await?;
            set_selected_uuid(&uuid);
            Ok(Some((uuid, model)))
        }
        None => Ok(None),
    }
}

pub async fn create_new() -> Result<(WorkbookId, UserModel<'static>), String> {
    let max_n = load_registry()
        .values()
        .filter_map(|meta| {
            meta.name
                .strip_prefix("Workbook ")
                .and_then(|s| s.parse::<usize>().ok())
        })
        .max()
        .unwrap_or(0);
    let name = format!("Workbook {}", max_n.saturating_add(1));
    let mut model = UserModel::new_empty("Workbook", LOCALE, "UTC", LOCALE)
        .map_err(|error| format!("Cannot create workbook: {error}"))?;
    model.set_name(&name);
    create_new_from(model, WorkbookOrigin::FileImport).await
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WorkbookOrigin {
    ShareLink,
    FileImport,
}

pub async fn create_new_from(
    model: UserModel<'static>,
    origin: WorkbookOrigin,
) -> Result<(WorkbookId, UserModel<'static>), String> {
    let uuid = WorkbookId::new();
    let origin = match origin {
        WorkbookOrigin::ShareLink => "shared",
        WorkbookOrigin::FileImport => "new",
    };
    snapshot(uuid, &model)?.write(origin).await?;
    set_selected_uuid(&uuid);
    Ok((uuid, model))
}

pub async fn delete(uuid: &WorkbookId) -> Result<(), String> {
    database::delete(&uuid.to_string())
        .await
        .map_err(|error| format!("Cannot delete workbook: {}", database::error(error)))?;
    if get_selected_uuid() == Some(*uuid) {
        LocalStorage::delete(SELECTED_KEY);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_workbook_saved_by_previous_engine() -> Result<(), String> {
        let model = decode(include_bytes!("fixtures/legacy-93f3407.rcal"))?;
        assert_eq!(model.get_name(), "Legacy workbook");
        assert_eq!(model.get_formatted_cell_value(0, 1, 1)?, "42");
        assert_eq!(model.get_formatted_cell_value(0, 1, 2)?, "84");
        assert_eq!(model.get_formatted_cell_value(0, 2, 1)?, "#DIV/0!");
        assert_eq!(model.get_formatted_cell_value(0, 3, 1)?, "legacy text");
        Ok(())
    }

    #[test]
    fn reads_v2_workbook_saved_before_views_were_removed() -> Result<(), String> {
        let model = decode(include_bytes!("fixtures/legacy-7e6ae21c.rcal"))?;
        assert_eq!(model.get_name(), "Legacy workbook");
        assert_eq!(model.get_formatted_cell_value(0, 1, 1)?, "42");
        assert_eq!(model.get_cell_content(0, 1, 2)?, "=A1*2");
        assert_eq!(model.get_formatted_cell_value(0, 1, 2)?, "84");
        assert_eq!(model.get_formatted_cell_value(0, 2, 1)?, "#DIV/0!");
        assert_eq!(model.get_formatted_cell_value(0, 3, 1)?, "legacy text");
        assert_eq!(model.get_formatted_cell_value(0, 1000, 26)?, "sparse cell");
        assert_eq!(model.get_worksheets_properties().len(), 2);
        assert_eq!(model.get_formatted_cell_value(1, 1, 1)?, "43");
        assert!(model.get_cell_style(0, 1, 1)?.font.b);
        assert_eq!(model.get_frozen_rows_count(0)?, 1);
        assert!(!model.get_show_grid_lines(0)?);
        assert_eq!(model.get_defined_name_list()[0].0, "answer");
        assert_eq!(
            model.get_cell_link(0, 3, 1)?,
            Some(ironcalc_base::types::Link::External {
                target: "https://example.com".into(),
                tooltip: Some("Legacy link".into()),
            })
        );
        assert_eq!(model.get_selected_view().sheet, 0);
        assert_eq!(model.get_selected_view().row, 1);
        Ok(())
    }

    #[test]
    fn rejects_invalid_envelopes() {
        assert!(decode(b"RCAL").is_err());
        assert!(decode(b"NOPE\x01").is_err());
        assert!(decode(b"RCAL\x04").is_err());
        assert!(decode(b"RCAL\x01bad bitcode").is_err());
        assert!(decode(b"RCAL\x02bad bitcode").is_err());
        assert!(decode(b"RCAL\x03bad bitcode").is_err());
    }

    #[test]
    fn legacy_workbook_can_be_saved_in_the_new_format() -> Result<(), String> {
        for bytes in [
            include_bytes!("fixtures/legacy-93f3407.rcal").as_slice(),
            include_bytes!("fixtures/legacy-7e6ae21c.rcal").as_slice(),
        ] {
            let mut model = decode(bytes)?;
            model.set_user_input(0, 1, 1, "100")?;
            let uuid = "550e8400-e29b-41d4-a716-446655440000"
                .parse()
                .map_err(|error| format!("Invalid fixture UUID: {error}"))?;
            let saved = snapshot(uuid, &model)?;
            assert_eq!(saved.bytes[4], 3);
            let restored = decode(&saved.bytes)?;
            assert_eq!(restored.get_formatted_cell_value(0, 1, 2)?, "200");
            assert_eq!(restored.get_formatted_cell_value(0, 2, 1)?, "#DIV/0!");
            assert_eq!(restored.get_model().workbook, model.get_model().workbook);
        }
        Ok(())
    }

    #[test]
    fn reads_current_engine_workbook_with_temporary_v2_header() -> Result<(), String> {
        let model = decode(include_bytes!("fixtures/legacy-7e6ae21c.rcal"))?;
        let mut bytes = b"RCAL\x02".to_vec();
        bytes.extend(model.to_bytes());
        let restored = decode(&bytes)?;
        assert_eq!(restored.get_model().workbook, model.get_model().workbook);
        Ok(())
    }

    #[test]
    fn rejects_truncated_v2_records_and_old_layout_with_v3_header() {
        let bytes = include_bytes!("fixtures/legacy-7e6ae21c.rcal");
        for end in 5..bytes.len() {
            assert!(decode(&bytes[..end]).is_err(), "accepted {end} bytes");
        }
        let mut bytes = bytes.to_vec();
        bytes[4] = 3;
        assert!(decode(&bytes).is_err());
    }
}
