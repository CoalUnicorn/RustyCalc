use crate::CanvasModel;
use crate::geometry::constants::{DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT};

/// A single-axis slot extent resolved from a model read.
///
/// The `Fetched` outcome from `CanvasModel` is resolved here, at the
/// geometry boundary: `Value` becomes its validated pixel extent, `Absent`
/// selects the axis's documented default (a row/column the model has no
/// override for), `BridgeFailed` is a transient read failure, and `Invalid`
/// is a host value that cannot become geometry (non-finite, negative, or
/// beyond `i32::MAX` px). `BridgeFailed` and `Invalid` both abort the walk —
/// the caller must hold the attempt, never substitute a default or fabricate
/// geometry from a broken number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtentFetch {
    /// Validated pixel extent: finite, non-negative, `0..=i32::MAX` px.
    Px(i32),
    /// The host returned a value that cannot become a slot extent
    /// (non-finite, negative, or overflowing `i32` px). Never round/cast it
    /// into geometry: a `NaN` cast would fabricate a hidden row, a negative
    /// value would break monotonic slot math.
    Invalid,
    /// The model read failed transiently; retry the whole attempt later.
    BridgeFailed,
}

impl ExtentFetch {
    /// Validated pixel parse for one host extent value.
    ///
    /// Accepts a finite value in `0..=i32::MAX` px after rounding. Zero stays
    /// a valid extent — the hidden-row/hidden-column value. Everything else
    /// (NaN, infinities, negative values, overflow) is `ExtentFetch::Invalid`,
    /// never clamped or cast into a fabricated slot.
    fn from_host_px(v: f64) -> ExtentFetch {
        if !v.is_finite() || v < 0.0 {
            return ExtentFetch::Invalid;
        }
        let px = v.round();
        if px > i32::MAX as f64 {
            return ExtentFetch::Invalid;
        }
        ExtentFetch::Px(px as i32)
    }

    /// Collapse to a pixel extent for an abortable walk: `Some(px)` for a
    /// resolved extent (validated concrete value or documented default),
    /// `None` for `BridgeFailed` (transient) or `Invalid` (host value that
    /// cannot become geometry), so the walk stops without committing
    /// fabricated geometry.
    pub fn extent(self) -> Option<i32> {
        match self {
            ExtentFetch::Px(px) => Some(px),
            ExtentFetch::Invalid | ExtentFetch::BridgeFailed => None,
        }
    }
}

/// Resolve one row-height read to a pixel extent.
///
/// `sheet` is the caller's already-captured/committed sheet — this never
/// re-reads `CanvasModel::get_selected_sheet()` itself, so a slot walk over
/// many rows costs one sheet read total, not one per row (see
/// `PaneSet::fill_rows`, `Chrome`'s blit rebuild, and `Orchestrator::scroll_to_show`,
/// every one of which now supplies it explicitly).
pub fn row_height(model: &dyn CanvasModel, sheet: u32, row: i32) -> ExtentFetch {
    match model.get_row_height(sheet, row) {
        crate::model::fetched::Fetched::Value(h) => ExtentFetch::from_host_px(h),
        crate::model::fetched::Fetched::Absent => {
            ExtentFetch::Px(DEFAULT_ROW_HEIGHT.round() as i32)
        }
        crate::model::fetched::Fetched::BridgeFailed => ExtentFetch::BridgeFailed,
    }
}

/// Column mirror of [`row_height`]; same explicit-`sheet` rationale.
pub fn col_width(model: &dyn CanvasModel, sheet: u32, col: i32) -> ExtentFetch {
    match model.get_column_width(sheet, col) {
        crate::model::fetched::Fetched::Value(w) => ExtentFetch::from_host_px(w),
        crate::model::fetched::Fetched::Absent => ExtentFetch::Px(DEFAULT_COL_WIDTH.round() as i32),
        crate::model::fetched::Fetched::BridgeFailed => ExtentFetch::BridgeFailed,
    }
}
