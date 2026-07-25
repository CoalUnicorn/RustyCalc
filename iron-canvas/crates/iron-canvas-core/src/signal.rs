//! Typed dirty signals for `PaintGate`. Each raised bit carries intent as a
//! payload so `Orchestrator::decide` can dispatch on *what* changed rather
//! than re-deriving intent from `is_still_valid` + `screen_for_blit`. Bit layout
//! mirrors `chrome::pane_region::PaneRegionMask`.

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    #[must_use = "GridSignals encode dirty-bit decisions; dropping the value without acting on it is a paint-skip bug"]
    pub struct GridSignals: u8 {
        /// Reserved bit; no setter raises it today. `screen_for_blit` detects
        /// viewport shifts geometrically.
        const VIEWPORT   = 0b0001;
        const CONTENT    = 0b0010;
        const STRUCTURAL = 0b0100;
        const OVERLAY    = 0b1000;

        const GRID_ANY   = 0b0111;
        const ALL        = 0b1111;
    }
}

impl GridSignals {
    /// `bitflags` v2 spells the zero value `empty()`. Alias kept so call
    /// sites (`Cell::new(GridSignals::EMPTY)`) stay declarative.
    pub const EMPTY: Self = Self::empty();

    pub fn grid_dirty(self) -> bool {
        self.intersects(Self::GRID_ANY)
    }
    pub fn overlay_dirty(self) -> bool {
        self.intersects(Self::OVERLAY)
    }
}

/// Row-band damage accumulated alongside the CONTENT bit — the third
/// repaint input, disjoint from both blit (viewport shift) and the
/// pane-level `pending_content` mask. Rows, not cell rectangles, are the
/// repaint unit for two distinct reasons: a row's top/bottom border may be
/// owned by (painted from) the row *above*/*below* it, so a rectangle
/// clipped to just the changed cell could leave a stale shared-edge stroke
/// behind or fail to draw a new one (`fingerprint::plan_pane_repaint`'s
/// border-safety check exists for exactly this); and cell text may overflow
/// horizontally into row neighbours (`render_pane` paints text last,
/// unclipped), which a future spill feature would only make more common —
/// a full-width band is the smallest repaint unit that cannot erase or
/// orphan overflow pixels either way.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CellDamage {
    /// No damage recorded since the last paint.
    #[default]
    Clean,
    /// Every CONTENT raise since the last paint named its rows on one
    /// sheet; repaint may clip to these bands. Spans are sorted, disjoint,
    /// and merged.
    Rows { sheet: u32, spans: Vec<RowSpan> },
    /// Damage info is incomplete — an un-rowed CONTENT raise, a second
    /// sheet, or more than `MAX_DAMAGE_SPANS` disjoint bands. Repaint
    /// falls back to the pane-mask path.
    Exceeded,
}

/// Inclusive row band in `RCRange` row coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowSpan {
    pub r1: i32,
    pub r2: i32,
}

/// Above this many disjoint bands a clipped repaint stops paying for
/// itself against one whole-pane walk.
// ponytail: fixed cap; tune only with profiler evidence.
const MAX_DAMAGE_SPANS: usize = 8;

impl CellDamage {
    pub fn add_rows(&mut self, sheet: u32, span: RowSpan) {
        // Callers may hand spans in either order; a reversed span would
        // survive merging and then silently intersect to nothing at paint
        // time — stale pixels with CONTENT already drained.
        let span = RowSpan {
            r1: span.r1.min(span.r2),
            r2: span.r1.max(span.r2),
        };
        match self {
            CellDamage::Clean => {
                *self = CellDamage::Rows {
                    sheet,
                    spans: vec![span],
                };
            }
            CellDamage::Rows { sheet: s, spans } if *s == sheet => {
                spans.push(span);
                spans.sort_by_key(|s| s.r1);
                let mut merged: Vec<RowSpan> = Vec::with_capacity(spans.len());
                for sp in spans.drain(..) {
                    match merged.last_mut() {
                        // +1: adjacent bands repaint as one strip.
                        Some(last) if sp.r1 <= last.r2 + 1 => {
                            last.r2 = last.r2.max(sp.r2);
                        }
                        _ => merged.push(sp),
                    }
                }
                if merged.len() > MAX_DAMAGE_SPANS {
                    *self = CellDamage::Exceeded;
                } else {
                    *spans = merged;
                }
            }
            CellDamage::Rows { .. } => *self = CellDamage::Exceeded,
            CellDamage::Exceeded => {}
        }
    }

    /// An un-rowed CONTENT raise: whatever rows were (or will be) recorded
    /// no longer cover everything that changed.
    pub fn poison(&mut self) {
        *self = CellDamage::Exceeded;
    }
}
