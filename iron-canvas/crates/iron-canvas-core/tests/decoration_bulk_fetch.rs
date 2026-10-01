//! Stage 5 bulk decoration fetch — CF decorations must flow through the
//! per-segment bulk buffer (`get_cell_decorations_in`) and reach the painter.
//! Decorations now resolve into `Painter` primitives at the renderer, so a
//! data bar paints as a `RectFill` (no CF-specific op). The bulk path also
//! has to survive the fingerprint-skip set-back: a second idempotent paint
//! must not corrupt the cached decorations buffer.

mod common;

use iron_canvas_core::address::RCRange;
use iron_canvas_core::chrome::{Chrome, FrameKindTag, FramePath};
use iron_canvas_core::renderer::RendererCore;
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::{CellDecoration, DataBarSpec, Fetched, GridVerdict, IconGlyph, RatingSpec};
use iron_canvas_recorder::{DrawOp, RecorderPainter};

use common::{TestModel, canvas_default, test_inputs};

fn data_bar(color: &str, value: f64) -> CellDecoration {
    bar(color, None, value, 0.0, false)
}

fn gradient_bar(color: &str, value: f64, axis: f64) -> CellDecoration {
    bar(color, None, value, axis, true)
}

fn bar(
    positive: &str,
    negative: Option<&str>,
    value: f64,
    axis_position: f64,
    is_gradient: bool,
) -> CellDecoration {
    CellDecoration {
        data_bar: Some(DataBarSpec {
            positive_color: positive.to_string(),
            negative_color: negative.map(str::to_string),
            is_gradient,
            value,
            axis_position,
            show_value: true,
        }),
        ..CellDecoration::default()
    }
}

fn rating(count: u32, max: u32) -> CellDecoration {
    CellDecoration {
        rating: Some(RatingSpec {
            glyph: IconGlyph::Star,
            color: None,
            count,
            max,
            show_value: true,
        }),
        ..CellDecoration::default()
    }
}

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
    model.set_decoration(2, 3, data_bar("#0a0", 0.5));

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
    model.set_decoration(2, 2, data_bar("#3366cc", 0.75));

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

#[test]
fn malformed_data_bar_color_paints_black_and_matches_black_fingerprint() {
    for color in ["#aé000", "#00aé0", "#0000é", "#中文", "#+10000"] {
        let model = TestModel::synthetic_grid();
        model.set_decoration(2, 2, data_bar(color, 0.75));
        let theme = std::rc::Rc::new(CanvasTheme::light());
        let inputs = test_inputs(&model, canvas_default(), &theme);
        let mut frame = Chrome::next(None, &model, &inputs, FramePath::Fresh);
        let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

        core.render_grid(&model, &frame);
        assert_eq!(data_bar_fill_count(core.painter(), "#000000"), 1, "{color}");

        model.set_decoration(2, 2, data_bar("#000000", 0.75));
        frame.kind = FrameKindTag::SlotsReused;
        core.reset_trace();
        core.render_grid(&model, &frame);
        assert_eq!(core.trace().verdict, Some(GridVerdict::Skip), "{color}");
        assert_eq!(data_bar_fill_count(core.painter(), "#000000"), 1, "{color}");
    }
}

/// The rects of every `RectFill` in the recorded op stream that use `color`.
fn rect_fills(painter: &RecorderPainter, color: &str) -> Vec<iron_canvas_core::PixelRect> {
    painter
        .ops()
        .iter()
        .filter_map(|op| match op {
            DrawOp::RectFill { rect, color: c } if c == color => Some(*rect),
            _ => None,
        })
        .collect()
}

fn gradients(painter: &RecorderPainter) -> Vec<(String, String)> {
    painter
        .ops()
        .iter()
        .filter_map(|op| match op {
            DrawOp::RectFillHGradient { from, to, .. } => Some((from.clone(), to.clone())),
            _ => None,
        })
        .collect()
}

fn render_cell(decoration: CellDecoration) -> (RendererCore<RecorderPainter>, Chrome) {
    render_cell_inner(Some(decoration), "")
}

fn render_text_cell(
    decoration: Option<CellDecoration>,
    text: &str,
) -> (RendererCore<RecorderPainter>, Chrome) {
    render_cell_inner(decoration, text)
}

fn render_cell_inner(
    decoration: Option<CellDecoration>,
    text: &str,
) -> (RendererCore<RecorderPainter>, Chrome) {
    let model = TestModel::synthetic_grid();
    model.set_col_width(2, 120.0);
    model.set_row_height(2, 24.0);
    if let Some(decoration) = decoration {
        model.set_decoration(2, 2, decoration);
    }
    if !text.is_empty() {
        model.set_cell(2, 2, text);
    }
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&model, canvas_default(), &theme);
    let frame = Chrome::next(None, &model, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));
    core.render_grid(&model, &frame);
    (core, frame)
}

/// Clear every present category's `show_value`, so the cell value is hidden.
fn hide_value(mut decoration: CellDecoration) -> CellDecoration {
    if let Some(icon) = decoration.icon.as_mut() {
        icon.show_value = false;
    }
    if let Some(bar) = decoration.data_bar.as_mut() {
        bar.show_value = false;
    }
    if let Some(rating) = decoration.rating.as_mut() {
        rating.show_value = false;
    }
    decoration
}

fn icon_decoration() -> CellDecoration {
    CellDecoration {
        icon: Some(iron_canvas_core::IconSpec {
            glyph: IconGlyph::ArrowUp,
            color: Some("#84cb1f".to_string()),
            show_value: true,
        }),
        ..CellDecoration::default()
    }
}

/// The anchor x of the first text op inside `rect` — the target cell's text,
/// not another populated cell's.
fn fill_text_x(painter: &RecorderPainter, rect: iron_canvas_core::PixelRect) -> Option<f64> {
    let ops = painter.ops();
    ops.iter().find_map(|op| match op {
        DrawOp::FillText { x, y, .. }
            if *x >= f64::from(rect.left())
                && *x <= f64::from(rect.right())
                && *y >= f64::from(rect.top())
                && *y <= f64::from(rect.bottom()) =>
        {
            Some(*x)
        }
        _ => None,
    })
}

/// A positive bar runs from the zero axis to the value endpoint — not from
/// the cell's left edge.
#[test]
fn positive_data_bar_spans_axis_to_value() {
    let (core, frame) = render_cell(bar("#3366cc", None, 1.0, 0.5, false));
    let rect = frame.cell_rect(2, 2).expect("the cell is visible");
    let inner_left = rect.left() + 2; // CF_INSET
    let axis = inner_left + (0.5 * f64::from(rect.width - 4)).round() as i32;

    let fills = rect_fills(core.painter(), "#3366cc");
    assert_eq!(fills.len(), 1, "exactly one positive bar");
    assert_eq!(fills[0].left(), axis, "bar starts at the zero axis");
    assert_eq!(fills[0].right(), inner_left + rect.width - 4);
}

/// A negative value paints the negative color between the value endpoint and
/// the axis.
#[test]
fn negative_data_bar_spans_value_to_axis_in_the_negative_color() {
    let (core, frame) = render_cell(bar("#3366cc", Some("#ff0000"), 0.0, 0.5, false));
    let rect = frame.cell_rect(2, 2).expect("the cell is visible");
    let inner_left = rect.left() + 2;
    let axis = inner_left + (0.5 * f64::from(rect.width - 4)).round() as i32;

    assert!(rect_fills(core.painter(), "#3366cc").is_empty());
    let fills = rect_fills(core.painter(), "#ff0000");
    assert_eq!(fills.len(), 1, "exactly one negative bar");
    assert_eq!(fills[0].left(), inner_left);
    assert_eq!(fills[0].right(), axis, "negative bar ends at the axis");
}

/// A value equal to the axis has zero length: the cell paints no bar.
#[test]
fn zero_length_bar_paints_nothing() {
    let (core, _) = render_cell(bar("#3366cc", None, 0.25, 0.25, false));
    assert!(rect_fills(core.painter(), "#3366cc").is_empty());
    assert!(gradients(core.painter()).is_empty());
}

/// A gradient bar emits the gradient primitive, running from the lighter
/// shade at the axis to the full color at the tip.
#[test]
fn gradient_data_bar_emits_a_gradient_fill() {
    let (core, _) = render_cell(gradient_bar("#3366cc", 1.0, 0.0));
    assert!(
        rect_fills(core.painter(), "#3366cc").is_empty(),
        "a gradient bar must not also paint a solid rect"
    );
    let grads = gradients(core.painter());
    assert_eq!(grads.len(), 1);
    assert_eq!(grads[0].1, "#3366cc", "gradient ends at the full color");
    assert_ne!(grads[0].0, grads[0].1, "gradient starts at a lighter shade");
}

/// A rating paints `count` copies of its glyph in the resolved color — one
/// per rating point, advancing left to right — not a fixed gold-star set.
#[test]
fn rating_paints_count_glyphs_in_the_resolved_color() {
    let (core, _) = render_cell(rating(3, 5));
    let paths: Vec<(Vec<iron_canvas_core::geometry::prim::Point>, String)> = core
        .painter()
        .ops()
        .iter()
        .filter_map(|op| match op {
            DrawOp::FillPath { points, color } => Some((points.clone(), color.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(paths.len(), 3, "one glyph per filled rating point");
    assert!(
        paths.iter().all(|(_, color)| color == "#000000"),
        "rating color comes from the engine (unresolved -> black)"
    );
    let xs: Vec<i32> = paths.iter().map(|(points, _)| points[0].x).collect();
    assert!(
        xs[0] < xs[1] && xs[1] < xs[2],
        "glyphs advance left to right"
    );
}

/// An icon paints its glyph geometry as a filled polygon.
#[test]
fn icon_paints_its_glyph() {
    let (core, _) = render_cell(icon_decoration());
    let ops = core.painter().ops();
    let arrows = ops
        .iter()
        .filter(|op| matches!(op, DrawOp::FillPath { color, .. } if color == "#84cb1f"))
        .count();
    assert!(arrows > 0, "the icon must paint at least one polygon");
}

/// A decoration with `show_value = false` hides the painted cell value; the
/// decoration itself still paints. The model value is untouched (formula bar
/// and editing read the model, not this pass).
#[test]
fn hidden_value_paints_no_text_but_keeps_the_bar() {
    let (core, frame) = render_text_cell(Some(hide_value(data_bar("#3366cc", 1.0))), "42");
    let rect = frame.cell_rect(2, 2).expect("the cell is visible");
    assert!(
        fill_text_x(core.painter(), rect).is_none(),
        "a hidden value must paint no text"
    );
    assert_eq!(
        rect_fills(core.painter(), "#3366cc").len(),
        1,
        "the data bar still paints"
    );
}

/// A shown icon reserves a left band, so the value lays out to the right of
/// the icon rather than under it.
#[test]
fn shown_icon_shifts_the_value_right() {
    let (plain, plain_frame) = render_text_cell(None, "42");
    let rect = plain_frame.cell_rect(2, 2).expect("the cell is visible");
    let plain_x = fill_text_x(plain.painter(), rect).expect("the plain cell paints text");

    let (icon, _) = render_text_cell(Some(icon_decoration()), "42");
    let icon_x = fill_text_x(icon.painter(), rect).expect("the icon cell still paints its value");

    assert!(
        icon_x > plain_x,
        "the icon must reserve a left band: {plain_x} -> {icon_x}"
    );
}
