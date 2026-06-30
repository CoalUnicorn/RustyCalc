//! Data-driven header text: a model-supplied column label replaces the default
//! A/B/C... spreadsheet label; columns left unset fall back to the default.

mod common;

use common::TestModel;
use iron_canvas_core::Orchestrator;
use iron_canvas_core::geometry::CanvasSize as OrchCanvasSize;
use iron_canvas_recorder::{DrawOp, MemSurface};
use std::rc::Rc;

/// Paint the grid layer and collect every painted text string.
fn grid_text(model: Rc<TestModel>) -> Vec<String> {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.resize(OrchCanvasSize { w: 600.0, h: 400.0 }, 1);
    orch.set_model(model);
    orch.paint_if_dirty();
    orch.grid_surface()
        .recorder()
        .ops()
        .iter()
        .filter_map(|op| match op {
            DrawOp::FillText { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn model_column_header_overrides_default_and_others_fall_back() {
    let texts = grid_text(Rc::new(
        TestModel::synthetic_grid().with_column_header(1, "Name"),
    ));
    assert!(
        texts.iter().any(|t| t == "Name"),
        "custom column-1 header must paint as \"Name\"; got {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t == "B"),
        "column-2 header must fall back to the default \"B\"; got {texts:?}"
    );
}
