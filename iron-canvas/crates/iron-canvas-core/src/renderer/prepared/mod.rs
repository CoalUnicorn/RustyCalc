//! Prepared grid work: fallible model reads are completed before painter or
//! committed cache state is touched.

use crate::CellContentQuery;
use crate::address::{DenseRange, RCRange};
use crate::chrome::{GridLayout, GridSegment, PaneRegion};
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Axis;
use crate::painter::Painter;
use crate::renderer::RendererCore;
use crate::renderer::cache::fingerprint::GridFingerprint;
use crate::style::{CellDecoration, CellKind, CellStyle};
use crate::types::fetched::Fetched;

mod execute;
mod paint;
mod prepare;
mod repaint;

pub(crate) use repaint::{PreparedFingerprintUpdate, PreparedRepaint, PreparedRepaintPlan};

use self::paint::shift_channel;

#[derive(Default, Clone)]
pub(crate) struct FetchedCells {
    styles: Vec<Fetched<CellStyle>>,
    values: Vec<Fetched<String>>,
    cell_types: Vec<Fetched<CellKind>>,
    decorations: Vec<Fetched<CellDecoration>>,
}

impl FetchedCells {
    pub(crate) const CHANNEL_COUNT: usize = 4;

    /// Addressed cells of `range`, counted against the dense invariant.
    ///
    /// `range` is any permissive address value convertible into a
    /// [`DenseRange`] (today always an `RCRange`); the conversion normalizes
    /// the corners once, so a dual-direction range can no longer be counted as
    /// zero cells. Reads of the returned bundle always cover `range`'s rows by
    /// `range`'s columns, never a smaller normalized subset.
    pub(crate) fn addressed_cells(range: impl Into<DenseRange>) -> usize {
        range.into().addressed_cells()
    }

    pub(crate) fn logical_channel_slots(range: impl Into<DenseRange>) -> usize {
        Self::addressed_cells(range).saturating_mul(Self::CHANNEL_COUNT)
    }

    pub(crate) fn is_dense_for(&self, range: impl Into<DenseRange>) -> bool {
        let expected = Self::addressed_cells(range);
        self.styles.len() == expected
            && self.values.len() == expected
            && self.cell_types.len() == expected
            && self.decorations.len() == expected
    }

    #[cfg(any(test, feature = "surface-introspection"))]
    pub(super) fn capacities(&self) -> (usize, usize, usize, usize) {
        (
            self.styles.capacity(),
            self.values.capacity(),
            self.cell_types.capacity(),
            self.decorations.capacity(),
        )
    }

    #[cfg(test)]
    pub(crate) fn from_parts(
        styles: Vec<Fetched<CellStyle>>,
        values: Vec<Fetched<String>>,
        cell_types: Vec<Fetched<CellKind>>,
        decorations: Vec<Fetched<CellDecoration>>,
    ) -> Self {
        Self {
            styles,
            values,
            cell_types,
            decorations,
        }
    }

    pub(crate) fn styles(&self) -> &[Fetched<CellStyle>] {
        &self.styles
    }

    pub(crate) fn values(&self) -> &[Fetched<String>] {
        &self.values
    }

    pub(crate) fn cell_types(&self) -> &[Fetched<CellKind>] {
        &self.cell_types
    }

    pub(crate) fn decorations(&self) -> &[Fetched<CellDecoration>] {
        &self.decorations
    }

    /// Fetch all four channels for `range` into `reuse`'s buffers.
    ///
    /// `range` is parsed into a [`DenseRange`] first: the model's bulk
    /// accessors keep their permissive `RCRange` signatures, so without this
    /// parse a reversed range would reach them and fetch nothing while the
    /// caller still believed a full bundle was addressed. Every consumer of
    /// the returned bundle may therefore index it by `height * width`
    /// directly.
    pub(crate) fn fetch_into(
        model: &dyn CellContentQuery,
        sheet: u32,
        range: impl Into<DenseRange>,
        reuse: Self,
    ) -> Self {
        let range = range.into().as_rc();
        let Self {
            mut styles,
            mut values,
            mut cell_types,
            mut decorations,
        } = reuse;
        model.get_cell_styles_in(sheet, range, &mut styles);
        model.get_formatted_cell_values_in(sheet, range, &mut values);
        model.get_cell_types_in(sheet, range, &mut cell_types);
        model.get_cell_decorations_in(sheet, range, &mut decorations);
        Self {
            styles,
            values,
            cell_types,
            decorations,
        }
    }

    pub(super) fn as_mut(&mut self) -> FetchedCellsMut<'_> {
        debug_assert!(
            self.styles.len() == self.values.len()
                && self.styles.len() == self.cell_types.len()
                && self.styles.len() == self.decorations.len()
        );
        FetchedCellsMut {
            styles: &mut self.styles,
            values: &mut self.values,
            cell_types: &mut self.cell_types,
            decorations: &mut self.decorations,
        }
    }

    pub(super) fn splice_strip_from(
        &mut self,
        strip: &mut Self,
        segment_range: RCRange,
        strip_range: RCRange,
    ) {
        super::cell::splice_strip_into(
            &mut self.styles,
            &mut strip.styles,
            segment_range,
            strip_range,
        );
        super::cell::splice_strip_into(
            &mut self.values,
            &mut strip.values,
            segment_range,
            strip_range,
        );
        super::cell::splice_strip_into(
            &mut self.cell_types,
            &mut strip.cell_types,
            segment_range,
            strip_range,
        );
        super::cell::splice_strip_into(
            &mut self.decorations,
            &mut strip.decorations,
            segment_range,
            strip_range,
        );
    }

    pub(crate) fn has_bridge_failure(&self) -> bool {
        super::cell::has_bridge_failure(&self.styles)
            || super::cell::has_bridge_failure(&self.values)
            || super::cell::has_bridge_failure(&self.cell_types)
            || super::cell::has_bridge_failure(&self.decorations)
    }

    fn shift(&mut self, previous: RCRange, candidate: RCRange, axis: Axis) {
        shift_channel(&mut self.styles, previous, candidate, axis, Fetched::Absent);
        shift_channel(&mut self.values, previous, candidate, axis, Fetched::Absent);
        shift_channel(
            &mut self.cell_types,
            previous,
            candidate,
            axis,
            Fetched::Absent,
        );
        shift_channel(
            &mut self.decorations,
            previous,
            candidate,
            axis,
            Fetched::Absent,
        );
    }
}

pub(super) struct FetchedCellsMut<'a> {
    pub(super) styles: &'a mut [Fetched<CellStyle>],
    pub(super) values: &'a mut [Fetched<String>],
    pub(super) cell_types: &'a mut [Fetched<CellKind>],
    pub(super) decorations: &'a mut [Fetched<CellDecoration>],
}

pub(crate) struct SegmentData {
    pub(crate) segment: GridSegment,
    pub(crate) fetched: FetchedCells,
}

pub(crate) struct PreparedStrip {
    pub(crate) region: PaneRegion,
    pub(crate) range: RCRange,
    pub(crate) fetched: FetchedCells,
}

// Full preparation uses fixed segment storage by design. Boxing the large arm
// would trade this predictable stack value for a heap allocation per frame.
#[allow(clippy::large_enum_variant)]
pub(crate) enum PreparedGrid {
    Empty,
    Full {
        layout: GridLayout,
        segments: [Option<SegmentData>; 4],
        repaint: PreparedRepaint,
    },
    Damage {
        layout: GridLayout,
        strips: Vec<PreparedStrip>,
    },
    Blit {
        previous: GridLayout,
        layout: GridLayout,
        axis: Axis,
        address_strips: [Option<PreparedStrip>; 2],
        pixel_clip: PixelRect,
        fingerprint: PreparedFingerprintUpdate,
    },
}

pub(crate) enum GridCacheCommit {
    Replace {
        layout: GridLayout,
        segments: [Option<FetchedCells>; 4],
        fingerprint: GridFingerprint,
    },
    Shift {
        previous: GridLayout,
        layout: GridLayout,
        axis: Axis,
        address_strips: [Option<PreparedStrip>; 2],
        fingerprint: PreparedFingerprintUpdate,
    },
    Splice {
        layout: GridLayout,
        strips: Vec<PreparedStrip>,
        fingerprint: PreparedFingerprintUpdate,
    },
    Reset,
}

impl<P: Painter> RendererCore<P> {
    fn take_strip_scratch(&self) -> FetchedCells {
        self.frame_cache
            .strip_scratch
            .borrow_mut()
            .pop()
            .unwrap_or_default()
    }

    fn park_strip_scratch(&self, cells: FetchedCells) {
        self.frame_cache.strip_scratch.borrow_mut().push(cells);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reversed_fetch_preserves_dense_channels_and_row_major_values() {
        struct Model;
        impl CellContentQuery for Model {
            fn get_cell_style(&self, _: u32, _: i32, _: i32) -> Fetched<CellStyle> {
                Fetched::Absent
            }
            fn get_cell_type(&self, _: u32, _: i32, _: i32) -> Fetched<CellKind> {
                Fetched::Absent
            }
            fn get_formatted_cell_value(&self, sheet: u32, row: i32, col: i32) -> Fetched<String> {
                Fetched::Value(format!("{sheet}:{row}:{col}"))
            }
        }
        let range = RCRange::from([3, 2, 2, 1]);
        let cells = FetchedCells::fetch_into(&Model, 7, range, FetchedCells::default());
        assert!(cells.is_dense_for(range));
        assert_eq!(FetchedCells::logical_channel_slots(range), 16);
        assert_eq!(
            cells.values,
            ["7:2:1", "7:2:2", "7:3:1", "7:3:2"].map(|value| Fetched::Value(value.into()))
        );
    }

    #[test]
    fn bridge_failure_is_detected_in_each_dense_channel() {
        fn clean<T>() -> Vec<Fetched<T>> {
            vec![Fetched::Absent]
        }
        fn failed<T>() -> Vec<Fetched<T>> {
            vec![Fetched::BridgeFailed]
        }

        let range = RCRange::from_cell(1, 1);
        for cells in [
            FetchedCells::from_parts(failed(), clean(), clean(), clean()),
            FetchedCells::from_parts(clean(), failed(), clean(), clean()),
            FetchedCells::from_parts(clean(), clean(), failed(), clean()),
            FetchedCells::from_parts(clean(), clean(), clean(), failed()),
        ] {
            assert!(cells.is_dense_for(range));
            assert!(cells.has_bridge_failure());
        }
    }
}
