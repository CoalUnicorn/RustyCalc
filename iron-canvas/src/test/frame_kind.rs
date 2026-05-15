//! Spec for `FrameKindTag` — every Chrome constructor sets the tag, and
//! Stage 5's regime dispatch reads it. These two specs lock down the
//! `Fresh` / `SlotsReused` constructor contracts; `Blitted` is covered
//! by the scroll-blit suite which already exercises `next_frame_with_blit`.

use ironcalc_base::types::{CellType, Style};

use crate::chrome::{Chrome, FrameKindTag};
use crate::theme::CanvasTheme;
use crate::{CanvasModel, CanvasSize, CanvasView, RCRange};

#[derive(Default)]
struct FixtureModel;

impl CanvasModel for FixtureModel {
    fn get_selected_sheet(&self) -> u32 {
        0
    }
    fn get_selected_view(&self) -> Option<CanvasView> {
        Some(CanvasView {
            sheet: 0,
            row: 1,
            column: 1,
            selection: RCRange::from([1, 1, 1, 1]),
            top_row: 1,
            left_column: 1,
        })
    }
    fn get_frozen_rows_count(&self, _: u32) -> Option<i32> {
        Some(0)
    }
    fn get_frozen_columns_count(&self, _: u32) -> Option<i32> {
        Some(0)
    }
    fn get_row_height(&self, _: u32, _: i32) -> Option<f64> {
        Some(20.0)
    }
    fn get_column_width(&self, _: u32, _: i32) -> Option<f64> {
        Some(80.0)
    }
    fn get_show_grid_lines(&self, _: u32) -> Option<bool> {
        Some(true)
    }
    fn get_cell_style(&self, _: u32, _: i32, _: i32) -> Option<Style> {
        Some(Style::default())
    }
    fn get_cell_type(&self, _: u32, _: i32, _: i32) -> Option<CellType> {
        Some(CellType::Text)
    }
    fn get_formatted_cell_value(&self, _: u32, _: i32, _: i32) -> Option<String> {
        Some(String::new())
    }
}

fn canvas() -> CanvasSize {
    CanvasSize { w: 600.0, h: 400.0 }
}

#[test]
fn next_frame_emits_fresh_when_no_prev() {
    let model = FixtureModel;
    let theme = CanvasTheme::light();
    let frame = Chrome::next_frame(None, &model, canvas(), &theme);
    assert_eq!(frame.kind, FrameKindTag::Fresh);
    assert!(
        !frame.kind.reuses_slots(),
        "Fresh is the one kind that does not reuse slot vecs",
    );
}

#[test]
fn from_slots_reuse_emits_slots_reused() {
    let model = FixtureModel;
    let theme = CanvasTheme::light();
    let fresh = Chrome::next_frame(None, &model, canvas(), &theme);
    let reused = Chrome::from_slots_reuse(fresh, theme.clone());
    assert_eq!(reused.kind, FrameKindTag::SlotsReused);
    assert!(
        reused.kind.reuses_slots(),
        "SlotsReused must report reuses_slots() so render_pane fingerprint-skips engage",
    );
}

/// Documents the Stage 5 invariant: adding a `FrameKindTag` variant must
/// break every regime arm in `orchestrator::paint_*`. The non-exhaustive
/// `match` blocks in `paint_viewport` / `paint_content` / `paint_rebuild`
/// (no `_ =>` arm) enforce this at compile time.
///
/// To verify locally:
/// 1. Add a fourth variant `Speculative` to `FrameKindTag` in `chrome/kind.rs`.
/// 2. `cargo check -p iron-canvas` — expect `error[E0004]` at three call
///    sites in `orchestrator.rs`.
/// 3. Revert.
///
/// This test does NOT do the experiment automatically; it pins the variant
/// list so a future reader can run the experiment in <1 minute.
#[test]
fn frame_kind_variants_documented() {
    let variants = [
        FrameKindTag::Fresh,
        FrameKindTag::SlotsReused,
        FrameKindTag::Blitted,
    ];
    assert_eq!(variants.len(), 3);
    // The compile-time presence check: if any variant is renamed or
    // removed, this array literal stops compiling before the doc rot.
}
