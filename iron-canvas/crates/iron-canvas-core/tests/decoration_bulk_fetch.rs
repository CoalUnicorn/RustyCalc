//! Stage 5 bulk decoration fetch — CF decorations must flow through the
//! per-segment bulk buffer (`get_cell_decorations_in`) and reach the painter.
//! Decorations now resolve into `Painter` primitives at the renderer, so a
//! data bar paints as a `RectFill` (no CF-specific op). The bulk path also
//! has to survive the fingerprint-skip set-back: a second idempotent paint
//! must not corrupt the cached decorations buffer.

mod common;

use iron_canvas_core::chrome::{Chrome, FrameKindTag, FramePath};
use iron_canvas_core::renderer::RendererCore;
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::types::coord::RCRange;
use iron_canvas_core::{CellDecoration, DataBarSpec, Fetched, GridVerdict};
use iron_canvas_recorder::{DrawOp, RecorderPainter};

use common::{TestModel, canvas_default, test_inputs};

// A data bar paints as a `RectFill` in its own distinctive color; cell
// backgrounds always use the theme color, so matching on the bar color
// isolates the decoration from the per-cell bg fills.
fn data_bar_fill_count(painter: &RecorderPainter, bar_color: &str) -> usize {
    painter
        .ops()
        .iter()
        .filter(|op| matches!(op, DrawOp::RectFill { color, .. } if color == bar_color))
        .count()
}

// The default bulk method must lay decorations out dense, row-major, with
// the decorated cell at its `(row - r1) * cols + (col - c1)` index and
// `None` everywhere else — the layout the paint loop's `idx` reads.
#[test]
fn bulk_method_places_decoration_at_correct_index() {
    let model = TestModel::synthetic_grid();
    model.set_decoration(
        2,
        3,
        CellDecoration::DataBar(DataBarSpec {
            fraction: 0.5,
            color: "#0a0".to_string(),
        }),
    );

    let range = RCRange {
        r1: 1,
        c1: 1,
        r2: 3,
        c2: 4,
    };
    let cols = (range.c2 - range.c1 + 1) as usize;
    let mut out = Vec::new();
    CanvasModelExt::decorations(&model, range, &mut out);

    assert_eq!(out.len(), 12, "dense 3x4 range");
    let target = ((2 - range.r1) * cols as i32 + (3 - range.c1)) as usize;
    for (i, slot) in out.iter().enumerate() {
        if i == target {
            assert!(
                matches!(slot, Fetched::Value(_)),
                "decorated cell present at its index"
            );
        } else {
            assert!(
                matches!(slot, Fetched::Absent),
                "non-decorated slot {i} must be Absent"
            );
        }
    }
}

// Trait helper to call the bulk method without naming `CanvasModel` twice.
use iron_canvas_core::CanvasModel;
trait CanvasModelExt: CanvasModel {
    fn decorations(&self, range: RCRange, out: &mut Vec<Fetched<CellDecoration>>) {
        let sheet = self
            .get_selected_sheet()
            .expect("test model always has a selected sheet");
        self.get_cell_decorations_in(sheet, range, out);
    }
}
impl<T: CanvasModel> CanvasModelExt for T {}

// End-to-end: the bulk path must hand the decoration to the painter, and a
// second idempotent paint under SlotsReused must still skip cleanly (the
// fingerprint set-back having preserved the decorations buffer).
#[test]
fn decoration_reaches_painter_and_skip_is_stable() {
    let model = TestModel::synthetic_grid();
    model.set_decoration(
        2,
        2,
        CellDecoration::DataBar(DataBarSpec {
            fraction: 0.75,
            color: "#3366cc".to_string(),
        }),
    );

    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&model, canvas_default(), &theme);
    let mut frame = Chrome::next(None, &model, &inputs, FramePath::Fresh);

    // Painted-fingerprint state lives on `GridCache` (on `RendererCore`),
    // not `Chrome` — so the same `core` must paint both frames for the
    // second call's compare to see the first call's committed tree.
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));
    core.render_grid(&model, &frame);
    assert_eq!(
        data_bar_fill_count(core.painter(), "#3366cc"),
        1,
        "bulk fetch must deliver exactly one data-bar RectFill",
    );

    frame.kind = FrameKindTag::SlotsReused;

    let bars_before = data_bar_fill_count(core.painter(), "#3366cc");
    core.reset_trace();
    core.render_grid(&model, &frame);
    // Unchanged content -> fingerprint match -> the cell walk is skipped,
    // including the decoration pass; the grid shell may still paint chrome.
    assert_eq!(
        data_bar_fill_count(core.painter(), "#3366cc"),
        bars_before,
        "idempotent repaint must not repaint the decoration",
    );
    assert_eq!(core.trace().verdict, Some(GridVerdict::Skip));
}
