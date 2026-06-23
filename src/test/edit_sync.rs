//! `sync_edit` upholds the cursor byte-offset invariant.
//!
//! The DOM reports `selectionEnd` in UTF-16 code units, but `EditingCell.cursor`
//! is a UTF-8 byte offset (state/editing_cell.rs) — every reader
//! (`is_in_reference_mode`, `splice_ref`, `refs_at_cursor`) indexes the formula
//! by byte. `sync_edit` is the single boundary that converts. This guards a
//! regression where the raw UTF-16 offset was stored, desyncing (and panicking)
//! the byte-indexed consumers once a multibyte char precedes the caret.

use crate::Owner;
use crate::coord::CellAddress;
use crate::input::formula::{FormulaAnalysis, is_in_reference_mode, sync_edit};
use crate::state::{EditFocus, EditMode, EditingCell, Split};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn cell_edit() -> EditingCell {
    EditingCell {
        address: CellAddress {
            sheet: 1,
            row: 1,
            column: 1,
        },
        text: String::new(),
        mode: EditMode::Edit,
        focus: EditFocus::Cell,
        text_dirty: false,
        formula_analysis: FormulaAnalysis::default(),
        cursor: 0,
    }
}

// "=é": '=' is 1 byte / 1 UTF-16 unit; 'é' (U+00E9) is 2 bytes / 1 UTF-16 unit.
// A caret at the end is UTF-16 offset 2 but byte offset 3. Storing the raw 2
// makes every byte-indexed reader slice mid-'é' — `&text[..2]` panics.
#[wasm_bindgen_test]
fn sync_edit_converts_utf16_cursor_to_byte_offset() {
    let owner = Owner::new();
    owner.with(|| {
        let editing = Split::new(Some(cell_edit()));
        sync_edit(editing, "=é".to_string(), 2, &[], &[]);

        let Some(edit) = editing.get_untracked() else {
            panic!("edit session must persist through sync_edit");
        };
        assert_eq!(edit.cursor, 3, "UTF-16 offset 2 must map to byte offset 3");

        // The stored cursor must be safe to feed the byte-indexed readers:
        // with the raw UTF-16 offset this line panicked on the char boundary.
        let _ = is_in_reference_mode("=é", edit.cursor);
    });
}
