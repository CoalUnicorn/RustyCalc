//! Stage-0 behaviour golden for the Track-A blit refactor.
//!
//! Captures the full post-scroll `Vec<DrawOp>` for a row scroll and a column
//! scroll via the direct `RendererCore` + `RecorderPainter` flow, then freezes
//! it as a blessed snapshot. Stages 1-2 of the refactor are behaviour-
//! preserving, so these snapshots must stay byte-for-byte identical. A diff is
//! a real regression — do NOT regenerate the snapshot to make the test pass.

mod common;

use std::path::PathBuf;

use iron_canvas_core::CanvasModel;
use iron_canvas_core::FrameDelta;
use iron_canvas_core::chrome::{
    ActiveCellSnapshot, BlitOutcome, Chrome, FramePath, PaneRegionMask,
};
use iron_canvas_core::painter::BlitPainter;
use iron_canvas_core::renderer::RendererCore;
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_recorder::{DrawOp, RecorderPainter};

use common::{TestModel, canvas_default as canvas, test_inputs};

fn snap(m: &TestModel) -> ActiveCellSnapshot {
    let view = m.get_selected_view().expect("scroll model has view");
    ActiveCellSnapshot::capture(m, view.sheet, view.row, view.column)
}

fn issue_blits<P: BlitPainter>(painter: &P, plan: &iron_canvas_core::chrome::BlitPlan) {
    for s in &plan.shifts {
        painter.blit(s.src, s.dst);
    }
}

/// Capture the draw ops emitted by a single-axis scroll-blit on a fresh model.
fn capture_scroll_ops(apply_scroll: impl FnOnce(&TestModel)) -> Vec<DrawOp> {
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let canvas = canvas();

    let inputs0 = test_inputs(&m, canvas, &theme);
    let frame0 = Chrome::next(None, &m, &inputs0, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));
    core.render_grid(&m, &frame0, PaneRegionMask::ALL);
    let baseline_ops = core.painter().ops().len();

    apply_scroll(&m);

    let inputs1 = test_inputs(&m, canvas, &theme);
    let FrameDelta::Scroll(plan) = Chrome::classify(Some(&frame0), &m, &inputs1, Some(&snap(&m)))
    else {
        panic!("single-axis scroll must qualify for blit");
    };

    let BlitOutcome::Blitted(frame1) = Chrome::next_blit(Some(frame0), &m, &inputs1, &plan) else {
        panic!("single-axis scroll must blit in place");
    };
    issue_blits(core.painter(), &plan);
    core.render_grid_blit(&m, &frame1, &plan);

    core.painter()
        .ops()
        .iter()
        .skip(baseline_ops)
        .cloned()
        .collect()
}

/// Blessed-snapshot assertion. When the snapshot file is absent it is written
/// (first run, freezing the behaviour); on every subsequent run it is read and
/// asserted byte-equal. Once written, the file is immutable for this task.
fn assert_blessed(name: &str, body: &str) {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("snapshots");
    path.push(format!("{name}.txt"));

    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create snapshots dir");
        }
        std::fs::write(&path, body).expect("write blessed snapshot");
        return;
    }

    let blessed = std::fs::read_to_string(&path).expect("read blessed snapshot");
    assert_eq!(
        blessed, body,
        "blessed snapshot `{name}` changed — this is a behaviour regression; \
         do NOT regenerate the snapshot, fix the code",
    );
}

#[test]
fn blit_scroll_pixels_unchanged() {
    let row_ops = capture_scroll_ops(|m| m.set_top_row(2));
    assert_blessed("blit_row_scroll", &format!("{row_ops:#?}"));

    let col_ops = capture_scroll_ops(|m| m.set_left_column(2));
    assert_blessed("blit_col_scroll", &format!("{col_ops:#?}"));
}
