//! Pane -> row -> cell content fingerprint tree for the paint-skip
//! optimization.
//!
//! `build_pane_fingerprint` walks the bulk-fetched buffers (`pane_styles`,
//! `pane_values`, `pane_cell_types`, `pane_decorations`) once and produces a
//! fresh `PaneFingerprint`: a whole-pane digest, with a `RowFingerprint`
//! per row and a `CellFingerprint` per cell nested beneath it.
//! `rebuild_pane_fingerprint_in_place` computes the identical tree but
//! writes it into an existing `PaneFingerprint`, reusing its outer `rows`
//! Vec and every row's `cells` Vec rather than allocating fresh ones each
//! call — the hot path (`PaneCache`'s per-pane `PaneFingerprintState`,
//! not `Chrome`, via its persistent `scratch` slot) uses this every frame so
//! the common no-op "skip" case never reallocates the tree. `plan_pane_repaint`
//! is what dispatches on the finished tree: it compares the pane-level
//! `digest` first — equal, and the 5-pass walk is skipped entirely — then
//! falls to comparing row digests to decide between a row-band repaint and a
//! whole-pane repaint. `render_pane` calls it via
//! `PaneFingerprintState::with_trees` on every slots-reuse frame.
//!
//! Hash domain — the set of inputs that determine painted pixels.
//! Anything that affects paint MUST be included; anything that doesn't
//! affect paint must NOT be. Two cells whose digests match must paint
//! identical pixels, otherwise the skip leaks visual staleness.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::orchestrator::PaneVerdict;
use crate::pending_work::{ContentWork, PendingWork, RowSpan};
use crate::renderer::cf_types::parse_hex_color;
use crate::style::{BorderItem, CellDecoration, CellKind, CellStyle};

use crate::types::coord::RCRange;
use crate::types::fetched::Fetched;

/// One cell's fingerprint leaf: just the `u64` digest over every
/// paint-relevant input at that address (style, formatted value, cell kind,
/// and CF decoration semantics) — including the cell's row/col, which fold
/// into the digest itself (see `cell_digest`) so two cells with identical
/// content at different addresses still digest distinctly. Coordinates are
/// NOT stored as fields: a leaf's `(row, col)` is always derivable from
/// `PaneFingerprint.range` plus its position in the nested
/// `rows[_].cells[_]` vectors, so storing them again here would be a
/// redundant, driftable copy.
///
/// Production repaint planning never reads this level — `plan_pane_repaint`
/// only compares row and pane digests. Its only reader is
/// `diff_changed_cells`, used today by this crate's own integration tests to
/// assert exact changed-cell coordinates rather than the full set of cells
/// in a changed row.
///
/// Measured cost for a 50x20 (1,000-cell) pane, the representative visible
/// viewport size: roughly 16 KiB — two retained `u64` leaves per cell
/// (painted + scratch trees), excluding row-vector overhead and spare
/// capacity — and about 1.37 ms/call (~1370 us) to build one tree from
/// scratch (debug build; see `bench_build_pane_fingerprint_for_a_realistic_pane_size`,
/// which prints the current number with `--nocapture`). That per-call cost
/// includes one `DefaultHasher` per cell, on top of the row and pane
/// hashers a pane->row-only tree would still need.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CellFingerprint(pub u64);

/// One row's fingerprint: the folded digest of every cell leaf in the row
/// (which itself folds in the row index — see `build_pane_fingerprint`),
/// plus the single boolean a later row-band repaint plan needs —
/// `has_any_explicit_border`: whether ANY cell in the row carries an explicit
/// border rule on ANY of its four edges.
///
/// Direction-agnostic on purpose. `paint_border` extends every stroke by
/// `width_px / 2` along the stroke's OWN axis so perpendicular borders close
/// their corner gap. A LEFT/RIGHT border is a vertical line, so a
/// `Medium`/`Thick` one reaches 1+ px ABOVE and BELOW its own cell — bleeding
/// into the neighbouring row's territory. A row-band repaint that clears the
/// changed span would erase that bleed without repainting the row that owns
/// it, so the safety check must treat a vertical border on an adjacent row as
/// just as dangerous as a horizontal one. Tracking only top/bottom borders
/// would miss the LEFT/RIGHT bleed entirely; one flag over all four edges
/// closes that gap conservatively.
///
/// No stored row index: a row's model row is always derivable from
/// `PaneFingerprint.range.r1` plus its position in `PaneFingerprint.rows`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RowFingerprint {
    pub digest: u64,
    pub has_any_explicit_border: bool,
    pub cells: Vec<CellFingerprint>,
}

/// The whole-pane fingerprint tree: the address-space `range` it was built
/// for, the folded whole-pane `digest` (the drop-in replacement for the old
/// `compute_pane_fingerprint` scalar), and every row in between.
///
/// Range is folded into `digest` so two panes with structurally-identical
/// data at different addresses don't collide.
///
/// `Default` (zero-value `RCRange`, digest `0`, empty `rows`) is the warm,
/// zero-allocation seed for `PaneFingerprintState`'s `painted`/`scratch`
/// slots — [`rebuild_pane_fingerprint_in_place`] grows a `Default` tree up
/// to its first real range on the first call, same as any subsequent
/// resize.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct PaneFingerprint {
    pub range: RCRange,
    pub digest: u64,
    pub rows: Vec<RowFingerprint>,
}

/// Build the pane -> row -> cell fingerprint tree for one pane in a single
/// pass over `range`: each cell's digest folds into its row's hasher, and
/// each row's finished digest folds into the pane's hasher. Same range +
/// same buffers -> same tree (modulo `DefaultHasher` collision).
///
/// The four buffers must be dense, row-major over `range` — the same layout
/// `get_cell_styles_in` / `get_cell_decorations_in` / etc. produce — so
/// `idx = (row - r1) * cols + (col - c1)` addresses every cell exactly once,
/// including hidden or zero-size rows/columns (this tree has no notion of
/// visibility; it only sees the dense buffers it's handed).
///
/// `render_pane`'s hot path rebuilds in place (`rebuild_pane_fingerprint_in_place`,
/// which also handles growing a brand-new tree from `PaneFingerprint::default()`'s
/// empty `rows`, so this function isn't needed there) — the only callers today
/// are this module's own unit tests, which want a plain "build me an
/// independent comparison tree" helper rather than a mutate-in-place target.
#[allow(dead_code)] // Only called by this module's own unit tests today.
pub(crate) fn build_pane_fingerprint(
    styles: &[Fetched<CellStyle>],
    values: &[Fetched<String>],
    cell_types: &[Fetched<CellKind>],
    decorations: &[Fetched<CellDecoration>],
    range: RCRange,
) -> PaneFingerprint {
    let cols = (range.c2 - range.c1 + 1).max(0) as usize;

    let mut pane_hasher = DefaultHasher::new();
    pane_hasher.write_i32(range.r1);
    pane_hasher.write_i32(range.c1);
    pane_hasher.write_i32(range.r2);
    pane_hasher.write_i32(range.c2);

    let row_count = (range.r2 - range.r1 + 1).max(0) as usize;
    let mut rows = Vec::with_capacity(row_count);

    for (row_offset, row) in range.rows().enumerate() {
        let mut row_hasher = DefaultHasher::new();
        row_hasher.write_i32(row);

        let mut has_any_explicit_border = false;
        let mut cells = Vec::with_capacity(cols);

        for (col_offset, col) in range.columns().enumerate() {
            let idx = row_offset * cols + col_offset;
            let style = &styles[idx];
            let value = &values[idx];
            let cell_type = &cell_types[idx];
            let decoration = &decorations[idx];

            // Record the row's explicit border state while we're already
            // walking every cell's style — no second pass needed. Any edge
            // counts: a vertical (left/right) border bleeds into the adjacent
            // row via the stroke's corner extension just as a horizontal one
            // does (see `RowFingerprint`'s doc).
            if let Fetched::Value(s) = style {
                has_any_explicit_border |= s.border.left.is_some()
                    || s.border.right.is_some()
                    || s.border.top.is_some()
                    || s.border.bottom.is_some();
            }

            let digest = cell_digest(row, col, style, value, cell_type, decoration);
            digest.hash(&mut row_hasher);
            cells.push(CellFingerprint(digest));
        }

        let row_digest = row_hasher.finish();
        row_digest.hash(&mut pane_hasher);
        rows.push(RowFingerprint {
            digest: row_digest,
            has_any_explicit_border,
            cells,
        });
    }

    PaneFingerprint {
        range,
        digest: pane_hasher.finish(),
        rows,
    }
}

/// In-place twin of [`build_pane_fingerprint`]: identical hashing (every
/// digest `target` ends up with is byte-for-byte what `build_pane_fingerprint`
/// would produce for the same inputs — see the equivalence test below), but
/// writes into `target` instead of returning a fresh tree, reusing its
/// outer `rows` Vec and each row's `cells` Vec rather than allocating new
/// ones. This is the "keep both trees' vector allocations warm" mechanism:
/// called every frame against the persistent `scratch` slot, a same-size
/// pane never triggers a single `Vec` allocation after its first paint.
///
/// Row-count changes resize `target.rows` in place rather than replacing
/// the Vec: `truncate` drops any excess rows (and their `cells` Vecs) when
/// the pane shrank; bare zero-valued `RowFingerprint`s are pushed when it
/// grew, then filled by the same per-row loop as every other row this call
/// (so a grown pane allocates only the newly-needed capacity, not a full
/// fresh tree). Each surviving row's `cells` Vec is `clear()`-ed (not
/// replaced) before being refilled, so its capacity survives a column-count
/// change the same way.
pub(crate) fn rebuild_pane_fingerprint_in_place(
    target: &mut PaneFingerprint,
    styles: &[Fetched<CellStyle>],
    values: &[Fetched<String>],
    cell_types: &[Fetched<CellKind>],
    decorations: &[Fetched<CellDecoration>],
    range: RCRange,
) {
    let cols = (range.c2 - range.c1 + 1).max(0) as usize;

    let mut pane_hasher = DefaultHasher::new();
    pane_hasher.write_i32(range.r1);
    pane_hasher.write_i32(range.c1);
    pane_hasher.write_i32(range.r2);
    pane_hasher.write_i32(range.c2);

    let row_count = (range.r2 - range.r1 + 1).max(0) as usize;
    if target.rows.len() > row_count {
        target.rows.truncate(row_count);
    } else {
        while target.rows.len() < row_count {
            target.rows.push(RowFingerprint {
                digest: 0,
                has_any_explicit_border: false,
                cells: Vec::new(),
            });
        }
    }

    for (row_offset, row) in range.rows().enumerate() {
        let mut row_hasher = DefaultHasher::new();
        row_hasher.write_i32(row);

        let mut has_any_explicit_border = false;

        let row_entry = &mut target.rows[row_offset];
        // Reuse the row's warm `cells` capacity — `clear()` drops the old
        // leaves without shrinking the backing allocation; the pushes below
        // refill it, only growing if this row's column count increased.
        row_entry.cells.clear();

        for (col_offset, col) in range.columns().enumerate() {
            let idx = row_offset * cols + col_offset;
            let style = &styles[idx];
            let value = &values[idx];
            let cell_type = &cell_types[idx];
            let decoration = &decorations[idx];

            if let Fetched::Value(s) = style {
                has_any_explicit_border |= s.border.left.is_some()
                    || s.border.right.is_some()
                    || s.border.top.is_some()
                    || s.border.bottom.is_some();
            }

            let digest = cell_digest(row, col, style, value, cell_type, decoration);
            digest.hash(&mut row_hasher);
            row_entry.cells.push(CellFingerprint(digest));
        }

        let row_digest = row_hasher.finish();
        row_digest.hash(&mut pane_hasher);
        row_entry.digest = row_digest;
        row_entry.has_any_explicit_border = has_any_explicit_border;
    }

    target.range = range;
    target.digest = pane_hasher.finish();
}

/// Fold one cell's row + col + style + formatted value + cell kind +
/// decoration into a single `u64`. The address is folded in here (rather
/// than stored on `CellFingerprint`) so two cells with identical content at
/// different addresses still produce distinct digests — without a
/// redundant, driftable `(row, col)` copy sitting next to the tree's own
/// nested-position addressing. `Absent` and `BridgeFailed` all hash as the
/// empty tag `0` per input — they paint identically *within a single
/// frame's walk* (nothing drawn), so the digest cannot tell them apart and
/// the skip stays behaviour-preserving. The hold-on-`BridgeFailed` decision
/// is made by the preflight *before* the fingerprint is committed, never here.
fn cell_digest(
    row: i32,
    col: i32,
    style: &Fetched<CellStyle>,
    value: &Fetched<String>,
    cell_type: &Fetched<CellKind>,
    decoration: &Fetched<CellDecoration>,
) -> u64 {
    let mut h = DefaultHasher::new();
    h.write_i32(row);
    h.write_i32(col);
    match style {
        Fetched::Absent | Fetched::BridgeFailed => h.write_u8(0),
        Fetched::Value(style) => {
            h.write_u8(1);
            StyleDigest(style).hash(&mut h);
        }
    }
    match value {
        Fetched::Absent | Fetched::BridgeFailed => h.write_u8(0),
        Fetched::Value(text) => {
            h.write_u8(1);
            h.write_usize(text.len());
            h.write(text.as_bytes());
        }
    }
    match cell_type {
        Fetched::Absent | Fetched::BridgeFailed => h.write_u8(0),
        Fetched::Value(ct) => {
            h.write_u8(1);
            std::mem::discriminant(ct).hash(&mut h);
        }
    }
    hash_decoration(decoration, &mut h);
    h.finish()
}

/// Hash a decoration's paint-relevant fields straight from the source
/// `CellDecoration` — parsed rgb, clamped fraction, star/filled counts —
/// without constructing the intermediate `CfDecorationPaint` value just to
/// throw it away: its `Icon` variant clones the icon name for no reason,
/// since every icon digests as the same constant tag regardless of name.
/// Two decorations that resolve to identical painted pixels must digest
/// identically, and vice versa.
///
/// `Icon` paints nothing yet (`CfDecorationPaint::paint`'s `Icon` arm is a
/// no-op — no glyph system exists), so every icon digests as tag `0`: the
/// same tag as `Absent`/`BridgeFailed`. Two different icon names must hash
/// identically to each other and to "no decoration", but still distinctly
/// from a data bar or rating (tags `1`/`2`), which DO paint pixels.
fn hash_decoration<H: Hasher>(decoration: &Fetched<CellDecoration>, h: &mut H) {
    match decoration {
        Fetched::Absent | Fetched::BridgeFailed => h.write_u8(0),
        Fetched::Value(CellDecoration::Icon(_)) => h.write_u8(0),
        Fetched::Value(CellDecoration::DataBar(spec)) => {
            h.write_u8(1);
            parse_hex_color(&spec.color).unwrap_or([0, 0, 0]).hash(h);
            // f64 is not Hash — hash the bit pattern instead. `fraction` is
            // clamped to [0.0, 1.0], matching what `CfDecorationPaint`
            // actually paints.
            spec.fraction.clamp(0.0, 1.0).to_bits().hash(h);
        }
        Fetched::Value(CellDecoration::Rating(spec)) => {
            h.write_u8(2);
            // RatingSpec fields are u32; cast to u8 mirrors
            // `CfDecorationPaint::from_cell_decoration`'s narrowing.
            (spec.stars as u8).hash(h);
            (spec.filled as u8).hash(h);
        }
    }
}

/// Hashable view over the subset of `Style` fields that affect painted
/// pixels. The field selection is load-bearing: a paint-read field the
/// digest misses -> stale pixels on skip; a paint-irrelevant field the
/// digest includes -> unnecessary repaint when only that field changed.
pub struct StyleDigest<'a>(pub &'a CellStyle);

impl<'a> Hash for StyleDigest<'a> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let s = self.0;

        s.fill_color.hash(state);

        s.font.strike.hash(state);
        s.font.underline.hash(state);
        s.font.bold.hash(state);
        s.font.italic.hash(state);
        // f64 is not Hash — hash the bit pattern instead. Font size is always
        // a finite positive number here, so to_bits() produces a stable value.
        s.font.size.to_bits().hash(state);
        s.font.color.hash(state);
        s.font.name.hash(state);

        match &s.alignment {
            None => state.write_u8(0),
            Some(a) => {
                state.write_u8(1);
                std::mem::discriminant(&a.horizontal).hash(state);
                std::mem::discriminant(&a.vertical).hash(state);
                a.wrap_text.hash(state);
            }
        }

        hash_border_item(&s.border.left, state);
        hash_border_item(&s.border.right, state);
        hash_border_item(&s.border.top, state);
        hash_border_item(&s.border.bottom, state);
        s.border.diagonal_up.hash(state);
        s.border.diagonal_down.hash(state);
    }
}

fn hash_border_item<H: Hasher>(b: &Option<BorderItem>, state: &mut H) {
    match b {
        None => state.write_u8(0),
        Some(bi) => {
            state.write_u8(1);
            std::mem::discriminant(&bi.style).hash(state);
            bi.color.hash(state);
        }
    }
}

// Pane-local row damage planning
//
// `plan_pane_repaint` is deliberately a pure function of two already-built
// trees, with no `Chrome`/`RendererCore` dependency: it decides Skip / Rows /
// Full. `render_pane` calls it via `PaneFingerprintState::with_trees` on
// every slots-reuse frame to narrow the repaint.

/// One pane's repaint decision, derived from comparing its `painted` and
/// `scratch` fingerprint trees. Exhaustive by construction — `bool` +
/// `Option<Vec<RowSpan>>` would let a caller's `match` silently treat
/// "invalid"/"unset" as one more meaningful state; this type has exactly the
/// three the planner ever produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RepaintPlan {
    /// The two trees are content-identical; no pixels need repainting.
    Skip,
    /// Repaint exactly these merged, disjoint row bands (full pane width —
    /// see `ContentWork`'s doc for why bands, not cell rectangles).
    Rows(Vec<RowSpan>),
    /// Repaint the whole pane — either the merged spans exceeded the cap,
    /// the two trees don't share a range, or a span's internal boundary
    /// carries shared-border risk.
    Full,
}

/// Trace projection: the planner's verdict as reported to `FrameTrace`. Lives
/// here so a new `RepaintPlan` variant breaks this match, not the trace.
impl From<&RepaintPlan> for PaneVerdict {
    fn from(plan: &RepaintPlan) -> Self {
        match plan {
            RepaintPlan::Skip => Self::Skip,
            RepaintPlan::Rows(spans) => Self::Rows {
                spans: spans.len().min(u8::MAX as usize) as u8,
                rows: spans
                    .iter()
                    .map(|s| (s.r2 - s.r1 + 1).max(0) as u32)
                    .sum::<u32>()
                    .min(u16::MAX as u32) as u16,
            },
            RepaintPlan::Full => Self::Full,
        }
    }
}

/// The `PendingWork` instance this planner feeds is scoped to a single
/// pane's single-frame comparison and immediately discarded once the
/// merged spans are read back out — its `sheet` tag never escapes this
/// function, so any constant works.
const PANE_LOCAL_SHEET: u32 = 0;

/// Decide how to repaint one pane given its previously-painted tree and this
/// frame's freshly-rebuilt scratch tree.
///
/// Skips the row/cell walk entirely on an equal whole-pane digest (the common
/// no-op case `render_pane` already optimizes for). Otherwise walks both
/// trees' rows in lockstep, feeds every differing row index into a
/// pane-local `PendingWork` (reusing `ContentWork`'s adjacent-merge and
/// `MAX_DAMAGE_SPANS` cap — not reimplemented here), then rejects the merged
/// spans in favour of `Full` if any span's internal top/bottom boundary
/// carries old or new explicit-border risk (see `span_has_unsafe_border`).
pub(crate) fn plan_pane_repaint(
    painted: &PaneFingerprint,
    scratch: &PaneFingerprint,
) -> RepaintPlan {
    if painted.digest == scratch.digest {
        return RepaintPlan::Skip;
    }

    // A row-for-row walk only means something when both trees address the
    // same range: a resize (or a first paint against a still-`Default`
    // painted tree) changes what "row i" even corresponds to. Full is
    // always a safe fallback here.
    if painted.range != scratch.range {
        return RepaintPlan::Full;
    }

    let range = scratch.range;
    let mut damage = PendingWork::default();
    for (row_offset, (painted_row, scratch_row)) in
        painted.rows.iter().zip(scratch.rows.iter()).enumerate()
    {
        if painted_row.digest != scratch_row.digest {
            let model_row = range.r1 + row_offset as i32;
            damage.mark_rows(
                PANE_LOCAL_SHEET,
                RowSpan {
                    r1: model_row,
                    r2: model_row,
                },
            );
        }
    }

    let spans = match damage.content() {
        // Unreachable in practice: the whole-pane digest already proved a
        // difference above, so at least one row must differ too (barring a
        // `DefaultHasher` collision). Skip is the safe reading of "no rows
        // to repaint" regardless.
        ContentWork::Clean => return RepaintPlan::Skip,
        // Only reachable via the span-count cap here — a single sheet tag
        // and row-only marks can degrade no other way.
        ContentWork::Panes(_) => return RepaintPlan::Full,
        ContentWork::Rows { spans, .. } => spans,
    };

    if spans
        .iter()
        .any(|span| span_has_unsafe_border(painted, scratch, range, *span))
    {
        return RepaintPlan::Full;
    }

    RepaintPlan::Rows(spans.clone())
}

/// True when `span`'s internal top or bottom boundary (i.e. NOT the pane's
/// own outer edge — see the doc on `plan_pane_repaint`'s caller) carries
/// explicit-border risk in either tree, on the span row itself OR on the
/// untouched neighbour across the boundary.
///
/// The risk is direction-agnostic, so the check reads the single
/// `has_any_explicit_border` flag rather than an edge-specific one. Two
/// separate bleed mechanisms share this boundary: a HORIZONTAL border on the
/// shared pixel edge may be owned by (painted from) either adjacent row, and
/// a VERTICAL (left/right) border on the neighbour row extends `width_px / 2`
/// past its own top/bottom edge (`paint_border`'s corner extension) into the
/// span's territory. A clipped row-band repaint that clears only the span
/// would erase a neighbour-owned stroke of either kind without repainting the
/// row that owns it. Checking both `painted` and `scratch` covers add and
/// remove: a border the old frame drew that the new frame removed still needs
/// erasing correctly, and a border the new frame adds still needs drawing.
fn span_has_unsafe_border(
    painted: &PaneFingerprint,
    scratch: &PaneFingerprint,
    range: RCRange,
    span: RowSpan,
) -> bool {
    fn row(tree: &PaneFingerprint, range: RCRange, model_row: i32) -> &RowFingerprint {
        &tree.rows[(model_row - range.r1) as usize]
    }

    // Top boundary: only meaningful when the row above the span is still
    // inside this pane — the pane's own first row has no neighbour above it
    // for a clipped repaint to collide with.
    if span.r1 > range.r1 {
        let above = span.r1 - 1;
        let unsafe_top = row(painted, range, above).has_any_explicit_border
            || row(scratch, range, above).has_any_explicit_border
            || row(painted, range, span.r1).has_any_explicit_border
            || row(scratch, range, span.r1).has_any_explicit_border;
        if unsafe_top {
            return true;
        }
    }

    // Bottom boundary: symmetric, only meaningful when the row below the
    // span is still inside this pane.
    if span.r2 < range.r2 {
        let below = span.r2 + 1;
        let unsafe_bottom = row(painted, range, span.r2).has_any_explicit_border
            || row(scratch, range, span.r2).has_any_explicit_border
            || row(painted, range, below).has_any_explicit_border
            || row(scratch, range, below).has_any_explicit_border;
        if unsafe_bottom {
            return true;
        }
    }

    false
}

/// Test/diagnostic-only: the exact `(row, col)` coordinates whose cell leaf
/// differs between `painted` and `scratch`. NOT used to shape
/// `plan_pane_repaint`'s decision — row bands remain the only repaint unit
/// it ever returns; this exists purely so a test can assert "A1 and B2
/// changed" without asserting the full Cartesian product of every touched
/// row's cells. Returns empty when the two trees don't share a range (there
/// is no meaningful per-cell correspondence to report).
#[allow(dead_code)] // Unused outside this module's own unit tests today.
pub(crate) fn diff_changed_cells(
    painted: &PaneFingerprint,
    scratch: &PaneFingerprint,
) -> Vec<(i32, i32)> {
    if painted.range != scratch.range {
        return Vec::new();
    }
    let range = scratch.range;
    let mut changed = Vec::new();
    for (row_offset, (painted_row, scratch_row)) in
        painted.rows.iter().zip(scratch.rows.iter()).enumerate()
    {
        let model_row = range.r1 + row_offset as i32;
        for (col_offset, (painted_cell, scratch_cell)) in painted_row
            .cells
            .iter()
            .zip(scratch_row.cells.iter())
            .enumerate()
        {
            if painted_cell.0 != scratch_cell.0 {
                changed.push((model_row, range.c1 + col_offset as i32));
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{
        Alignment, Border, BorderStyle, DataBarSpec, FontStyle, HAlign, RatingSpec,
    };

    fn range_2x2() -> RCRange {
        RCRange {
            r1: 1,
            c1: 1,
            r2: 2,
            c2: 2,
        }
    }

    type DenseBuffers = (
        Vec<Fetched<CellStyle>>,
        Vec<Fetched<String>>,
        Vec<Fetched<CellKind>>,
        Vec<Fetched<CellDecoration>>,
    );

    /// Four `Absent`/`Value` cells laid out row-major over a 2x2 range,
    /// with `Absent` everywhere except the given `(row, col)` value cell.
    fn dense_buffers_with_value_at(target: (i32, i32), text: &str) -> DenseBuffers {
        let range = range_2x2();
        let cols = (range.c2 - range.c1 + 1) as usize;
        let mut styles = vec![Fetched::Absent; 4];
        let mut values = vec![Fetched::Absent; 4];
        let cell_types = vec![Fetched::Value(CellKind::Text); 4];
        let decorations = vec![Fetched::Absent; 4];

        let idx = ((target.0 - range.r1) * cols as i32 + (target.1 - range.c1)) as usize;
        styles[idx] = Fetched::Value(CellStyle::default());
        values[idx] = Fetched::Value(text.to_string());

        (styles, values, cell_types, decorations)
    }

    // Acceptance 1: equal dense inputs -> equal complete trees.
    #[test]
    fn equal_dense_inputs_produce_equal_complete_trees() {
        let (styles, values, cell_types, decorations) =
            dense_buffers_with_value_at((1, 1), "hello");
        let range = range_2x2();

        let a = build_pane_fingerprint(&styles, &values, &cell_types, &decorations, range);
        let b = build_pane_fingerprint(&styles, &values, &cell_types, &decorations, range);

        assert_eq!(a, b, "identical buffers must build identical trees");
    }

    // Acceptance 2: changing one cell's value changes exactly that cell
    // leaf, its row digest, and the pane digest — every other cell leaf and
    // the untouched row's digest stay put.
    #[test]
    fn changing_one_value_changes_exactly_one_cell_leaf_its_row_and_pane_digest() {
        let range = range_2x2();
        let (styles, values, cell_types, decorations) =
            dense_buffers_with_value_at((1, 1), "before");
        let before = build_pane_fingerprint(&styles, &values, &cell_types, &decorations, range);

        let (styles2, values2, cell_types2, decorations2) =
            dense_buffers_with_value_at((1, 1), "after");
        let after = build_pane_fingerprint(&styles2, &values2, &cell_types2, &decorations2, range);

        assert_ne!(before.digest, after.digest, "pane digest must change");

        let mut changed_cells = 0;
        for (row_idx, (row_before, row_after)) in
            before.rows.iter().zip(after.rows.iter()).enumerate()
        {
            // Model row is derived from position, not stored — `range.r1 +
            // row_idx` is the coordinate a caller would report for this row.
            let model_row = range.r1 + row_idx as i32;
            let row_changed = row_before.digest != row_after.digest;
            for (cell_before, cell_after) in row_before.cells.iter().zip(row_after.cells.iter()) {
                if cell_before.0 != cell_after.0 {
                    changed_cells += 1;
                    assert!(
                        row_changed,
                        "a cell leaf changed but its row digest did not: row {model_row}"
                    );
                } else if !row_changed {
                    // Untouched row: every cell leaf in it must also match.
                    assert_eq!(cell_before.0, cell_after.0);
                }
            }
        }
        assert_eq!(changed_cells, 1, "exactly one cell leaf must change");
    }

    // Changing style, kind, or decoration (independently) must each change
    // exactly the touched cell leaf too — value isn't the only signal that
    // must participate.
    #[test]
    fn changing_style_changes_the_cell_leaf() {
        let range = range_2x2();
        let (styles, values, cell_types, decorations) = dense_buffers_with_value_at((1, 1), "same");
        let before = build_pane_fingerprint(&styles, &values, &cell_types, &decorations, range);

        let mut styles2 = styles.clone();
        styles2[0] = Fetched::Value(CellStyle {
            font: FontStyle {
                bold: true,
                ..FontStyle::default()
            },
            ..CellStyle::default()
        });
        let after = build_pane_fingerprint(&styles2, &values, &cell_types, &decorations, range);

        assert_ne!(before.digest, after.digest);
        assert_eq!(before.rows[0].cells[1].0, after.rows[0].cells[1].0);
        assert_ne!(before.rows[0].cells[0].0, after.rows[0].cells[0].0);
    }

    #[test]
    fn changing_cell_kind_changes_the_cell_leaf() {
        let range = range_2x2();
        let (styles, values, cell_types, decorations) = dense_buffers_with_value_at((1, 1), "same");
        let before = build_pane_fingerprint(&styles, &values, &cell_types, &decorations, range);

        let mut cell_types2 = cell_types.clone();
        cell_types2[0] = Fetched::Value(CellKind::Number);
        let after = build_pane_fingerprint(&styles, &values, &cell_types2, &decorations, range);

        assert_ne!(before.digest, after.digest);
        assert_ne!(before.rows[0].cells[0].0, after.rows[0].cells[0].0);
    }

    #[test]
    fn changing_painted_decoration_changes_the_cell_leaf() {
        let range = range_2x2();
        let (styles, values, cell_types, decorations) = dense_buffers_with_value_at((1, 1), "same");
        let before = build_pane_fingerprint(&styles, &values, &cell_types, &decorations, range);

        let mut decorations2 = decorations.clone();
        decorations2[0] = Fetched::Value(CellDecoration::DataBar(DataBarSpec {
            fraction: 0.5,
            color: "#00ff00".to_string(),
        }));
        let after = build_pane_fingerprint(&styles, &values, &cell_types, &decorations2, range);

        assert_ne!(before.digest, after.digest);
        assert_ne!(before.rows[0].cells[0].0, after.rows[0].cells[0].0);
    }

    // Acceptance 3: A1 + B2 changes (two different rows) report exactly
    // two cell leaves, one per row.
    #[test]
    fn two_cells_in_different_rows_report_exactly_two_cell_leaves() {
        let range = range_2x2();
        let styles = vec![Fetched::Value(CellStyle::default()); 4];
        let cell_types = vec![Fetched::Value(CellKind::Text); 4];
        let decorations = vec![Fetched::Absent; 4];
        let values_before = vec![
            Fetched::Value("a1".to_string()),
            Fetched::Value("b1".to_string()),
            Fetched::Value("a2".to_string()),
            Fetched::Value("b2".to_string()),
        ];
        let before =
            build_pane_fingerprint(&styles, &values_before, &cell_types, &decorations, range);

        // Change A1 (row 1, col 1 -> idx 0) and B2 (row 2, col 2 -> idx 3).
        let mut values_after = values_before.clone();
        values_after[0] = Fetched::Value("a1-changed".to_string());
        values_after[3] = Fetched::Value("b2-changed".to_string());
        let after =
            build_pane_fingerprint(&styles, &values_after, &cell_types, &decorations, range);

        let mut changed_cells = 0;
        for (row_before, row_after) in before.rows.iter().zip(after.rows.iter()) {
            for (cell_before, cell_after) in row_before.cells.iter().zip(row_after.cells.iter()) {
                if cell_before.0 != cell_after.0 {
                    changed_cells += 1;
                }
            }
        }
        assert_eq!(
            changed_cells, 2,
            "A1 + B2 must report exactly two cell leaves"
        );
        assert_ne!(before.rows[0].digest, after.rows[0].digest, "row 1 changed");
        assert_ne!(before.rows[1].digest, after.rows[1].digest, "row 2 changed");
    }

    // Acceptance 4: hidden/zero-size addresses retain their dense indices —
    // the tree has no visibility concept, so every row/col in `range` gets
    // a slot regardless of the model's hidden/zero-height state (modeled
    // here simply as `Absent` slots, since hidden rows fetch nothing).
    #[test]
    fn dense_indices_are_retained_regardless_of_absent_slots() {
        let range = RCRange {
            r1: 1,
            c1: 1,
            r2: 3,
            c2: 2,
        };
        let styles = vec![Fetched::Absent; 6];
        let values = vec![Fetched::Absent; 6];
        let cell_types = vec![Fetched::Absent; 6];
        let decorations = vec![Fetched::Absent; 6];

        let tree = build_pane_fingerprint(&styles, &values, &cell_types, &decorations, range);

        // Coordinates aren't stored on the leaves (derived from `range` +
        // position instead), so "dense indices retained" means the nested
        // vector lengths match the range dimensions exactly, for every row —
        // hidden/zero-size or not, nothing is skipped or coalesced.
        assert_eq!(
            tree.rows.len(),
            3,
            "one row entry per model row, hidden or not"
        );
        for row in &tree.rows {
            assert_eq!(row.cells.len(), 2, "one cell entry per model column");
        }
    }

    // Icons don't paint pixels yet: two different icon names must digest
    // identically to each other and to "no decoration".
    #[test]
    fn icon_decoration_digests_same_as_absence_regardless_of_name() {
        let range = range_2x2();
        let (styles, values, cell_types, absent_decorations) =
            dense_buffers_with_value_at((1, 1), "same");

        let mut icon_a = absent_decorations.clone();
        icon_a[0] = Fetched::Value(CellDecoration::Icon("ArrowUp".to_string()));
        let mut icon_b = absent_decorations.clone();
        icon_b[0] = Fetched::Value(CellDecoration::Icon("TrafficLight".to_string()));

        let no_deco =
            build_pane_fingerprint(&styles, &values, &cell_types, &absent_decorations, range);
        let with_icon_a = build_pane_fingerprint(&styles, &values, &cell_types, &icon_a, range);
        let with_icon_b = build_pane_fingerprint(&styles, &values, &cell_types, &icon_b, range);

        assert_eq!(
            no_deco.digest, with_icon_a.digest,
            "icon vs absent must match"
        );
        assert_eq!(
            with_icon_a.digest, with_icon_b.digest,
            "two different icon names must digest identically"
        );
    }

    // Data bar / rating decorations DO paint pixels, so they must digest
    // distinctly from absence — using actual parsed rgb/fraction and actual
    // star/filled counts, not the raw model spec.
    #[test]
    fn data_bar_and_rating_digest_distinctly_from_absence_and_each_other() {
        let range = range_2x2();
        let (styles, values, cell_types, absent_decorations) =
            dense_buffers_with_value_at((1, 1), "same");

        let mut data_bar = absent_decorations.clone();
        data_bar[0] = Fetched::Value(CellDecoration::DataBar(DataBarSpec {
            fraction: 0.5,
            color: "#3366cc".to_string(),
        }));
        let mut rating = absent_decorations.clone();
        rating[0] = Fetched::Value(CellDecoration::Rating(RatingSpec {
            stars: 5,
            filled: 3,
        }));

        let no_deco =
            build_pane_fingerprint(&styles, &values, &cell_types, &absent_decorations, range);
        let with_bar = build_pane_fingerprint(&styles, &values, &cell_types, &data_bar, range);
        let with_rating = build_pane_fingerprint(&styles, &values, &cell_types, &rating, range);

        assert_ne!(no_deco.digest, with_bar.digest);
        assert_ne!(no_deco.digest, with_rating.digest);
        assert_ne!(with_bar.digest, with_rating.digest);
    }

    // Data bars with different fraction/color must digest distinctly — the
    // digest must hash the parsed rgb + clamped fraction, not a constant tag.
    #[test]
    fn data_bar_digest_varies_with_fraction_and_color() {
        let range = range_2x2();
        let (styles, values, cell_types, absent_decorations) =
            dense_buffers_with_value_at((1, 1), "same");

        let mut bar_half = absent_decorations.clone();
        bar_half[0] = Fetched::Value(CellDecoration::DataBar(DataBarSpec {
            fraction: 0.5,
            color: "#3366cc".to_string(),
        }));
        let mut bar_full = absent_decorations.clone();
        bar_full[0] = Fetched::Value(CellDecoration::DataBar(DataBarSpec {
            fraction: 1.0,
            color: "#3366cc".to_string(),
        }));
        let mut bar_other_color = absent_decorations.clone();
        bar_other_color[0] = Fetched::Value(CellDecoration::DataBar(DataBarSpec {
            fraction: 0.5,
            color: "#ff0000".to_string(),
        }));

        let half = build_pane_fingerprint(&styles, &values, &cell_types, &bar_half, range);
        let full = build_pane_fingerprint(&styles, &values, &cell_types, &bar_full, range);
        let other_color =
            build_pane_fingerprint(&styles, &values, &cell_types, &bar_other_color, range);

        assert_ne!(half.digest, full.digest, "fraction must participate");
        assert_ne!(half.digest, other_color.digest, "color must participate");
    }

    // Rating digest must use the actual star/filled counts, not a constant
    // tag — two different ratings must digest distinctly.
    #[test]
    fn rating_digest_varies_with_star_and_filled_counts() {
        let range = range_2x2();
        let (styles, values, cell_types, absent_decorations) =
            dense_buffers_with_value_at((1, 1), "same");

        let mut rating_3 = absent_decorations.clone();
        rating_3[0] = Fetched::Value(CellDecoration::Rating(RatingSpec {
            stars: 5,
            filled: 3,
        }));
        let mut rating_4 = absent_decorations.clone();
        rating_4[0] = Fetched::Value(CellDecoration::Rating(RatingSpec {
            stars: 5,
            filled: 4,
        }));

        let three = build_pane_fingerprint(&styles, &values, &cell_types, &rating_3, range);
        let four = build_pane_fingerprint(&styles, &values, &cell_types, &rating_4, range);

        assert_ne!(three.digest, four.digest);
    }

    // The single any-edge border flag must be recorded per row while walking
    // styles: ANY cell in the row carrying an explicit border on ANY edge
    // sets it, including vertical (left/right) borders — a fill color alone
    // does not.
    #[test]
    fn row_records_any_explicit_border_from_any_cell() {
        let range = RCRange {
            r1: 1,
            c1: 1,
            r2: 3,
            c2: 2,
        };
        let cell_types = vec![Fetched::Value(CellKind::Text); 6];
        let values = vec![Fetched::Value(String::new()); 6];
        let decorations = vec![Fetched::Absent; 6];

        let mut styles = vec![Fetched::Value(CellStyle::default()); 6];
        // Row 1, col 2 (idx 1) carries an explicit LEFT border.
        styles[1] = Fetched::Value(CellStyle {
            border: Border {
                left: Some(BorderItem {
                    style: BorderStyle::Thin,
                    color: None,
                }),
                ..Border::default()
            },
            ..CellStyle::default()
        });
        // Row 2, col 1 (idx 2) carries an explicit RIGHT border.
        styles[2] = Fetched::Value(CellStyle {
            border: Border {
                right: Some(BorderItem {
                    style: BorderStyle::Thin,
                    color: None,
                }),
                ..Border::default()
            },
            ..CellStyle::default()
        });
        // Row 3, col 1 (idx 4) carries only a fill color — no border at all.
        styles[4] = Fetched::Value(CellStyle {
            fill_color: Some("#ffcc00".to_string()),
            ..CellStyle::default()
        });

        let tree = build_pane_fingerprint(&styles, &values, &cell_types, &decorations, range);

        assert!(
            tree.rows[0].has_any_explicit_border,
            "a LEFT border sets the flag"
        );
        assert!(
            tree.rows[1].has_any_explicit_border,
            "a RIGHT border sets the flag"
        );
        assert!(
            !tree.rows[2].has_any_explicit_border,
            "a fill color with no border must NOT set the flag"
        );
    }

    // The vertical-border regression: a LEFT/RIGHT border on an untouched
    // neighbour row bleeds vertically across a span boundary via
    // `paint_border`'s corner extension, so `span_has_unsafe_border` must
    // veto a clipped row-band repaint even when the border lives on the
    // neighbour, not the changed span — and even when only one tree carries
    // it (an added or removed border).
    #[test]
    fn span_unsafe_when_untouched_neighbor_row_has_vertical_border() {
        let range = RCRange {
            r1: 1,
            c1: 1,
            r2: 3,
            c2: 1,
        };
        let cell_types = vec![Fetched::Value(CellKind::Text); 3];
        let values = vec![Fetched::Value(String::new()); 3];
        let decorations = vec![Fetched::Absent; 3];

        // `painted`: every row plain.
        let plain = vec![Fetched::Value(CellStyle::default()); 3];
        let painted = build_pane_fingerprint(&plain, &values, &cell_types, &decorations, range);

        // `scratch`: row 1 — the untouched neighbour ABOVE the span — gains a
        // medium LEFT border (a vertical stroke that extends past its own
        // top/bottom edge into the span boundary).
        let mut styles = plain.clone();
        styles[0] = Fetched::Value(CellStyle {
            border: Border {
                left: Some(BorderItem {
                    style: BorderStyle::Medium,
                    color: None,
                }),
                ..Border::default()
            },
            ..CellStyle::default()
        });
        let scratch = build_pane_fingerprint(&styles, &values, &cell_types, &decorations, range);

        // The changed span is row 2 alone; its top boundary touches row 1.
        let span = RowSpan { r1: 2, r2: 2 };
        assert!(
            span_has_unsafe_border(&painted, &scratch, range, span),
            "a vertical border on the untouched neighbour row must trip the boundary check"
        );
        // Control: with no border in either tree, the same interior span is
        // safe for a scoped row-band repaint.
        assert!(
            !span_has_unsafe_border(&painted, &painted, range, span),
            "a plain interior span must not be vetoed"
        );
    }

    // Alignment participates in the style digest (via `StyleDigest`, reused
    // unmodified) — sanity check that the reused digest is actually wired
    // into the per-cell hash, not silently dropped.
    #[test]
    fn alignment_change_participates_via_reused_style_digest() {
        let range = range_2x2();
        let (styles, values, cell_types, decorations) = dense_buffers_with_value_at((1, 1), "same");
        let before = build_pane_fingerprint(&styles, &values, &cell_types, &decorations, range);

        let mut styles2 = styles.clone();
        styles2[0] = Fetched::Value(CellStyle {
            alignment: Some(Alignment {
                horizontal: HAlign::Center,
                ..Alignment::default()
            }),
            ..CellStyle::default()
        });
        let after = build_pane_fingerprint(&styles2, &values, &cell_types, &decorations, range);

        assert_ne!(before.digest, after.digest);
    }

    // `rebuild_pane_fingerprint_in_place` must be a drop-in replacement for
    // `build_pane_fingerprint`'s hashing — same inputs, same tree, whether
    // written fresh or rebuilt into a `Default::default()` target.
    #[test]
    fn rebuild_in_place_matches_build_from_scratch() {
        let range = range_2x2();
        let (styles, values, cell_types, decorations) =
            dense_buffers_with_value_at((1, 1), "hello");
        let expected = build_pane_fingerprint(&styles, &values, &cell_types, &decorations, range);

        let mut tree = PaneFingerprint::default();
        rebuild_pane_fingerprint_in_place(
            &mut tree,
            &styles,
            &values,
            &cell_types,
            &decorations,
            range,
        );

        assert_eq!(
            tree, expected,
            "in-place rebuild must produce the identical tree build_pane_fingerprint would"
        );
    }

    // The property this function exists for: a same-size rebuild must not
    // reallocate the outer `rows` Vec or any row's `cells` Vec. Capacity
    // staying put is a concrete, checkable proxy for "no allocation
    // happened" without an allocation-counting harness.
    #[test]
    fn rebuild_in_place_keeps_row_and_cell_vec_capacities_warm() {
        let range = range_2x2();
        let (styles, values, cell_types, decorations) =
            dense_buffers_with_value_at((1, 1), "before");

        let mut tree = PaneFingerprint::default();
        rebuild_pane_fingerprint_in_place(
            &mut tree,
            &styles,
            &values,
            &cell_types,
            &decorations,
            range,
        );
        let rows_capacity_after_first = tree.rows.capacity();
        let cell_capacities_after_first: Vec<usize> =
            tree.rows.iter().map(|r| r.cells.capacity()).collect();

        let (styles2, values2, cell_types2, decorations2) =
            dense_buffers_with_value_at((1, 1), "after");
        rebuild_pane_fingerprint_in_place(
            &mut tree,
            &styles2,
            &values2,
            &cell_types2,
            &decorations2,
            range,
        );

        assert_eq!(
            tree.rows.capacity(),
            rows_capacity_after_first,
            "outer rows Vec must not reallocate on a same-size rebuild"
        );
        for (row, expected_cap) in tree.rows.iter().zip(cell_capacities_after_first.iter()) {
            assert_eq!(
                row.cells.capacity(),
                *expected_cap,
                "row cells Vec must not reallocate on a same-size rebuild"
            );
        }
        // Capacities staying warm must not come at the cost of correctness.
        let expected =
            build_pane_fingerprint(&styles2, &values2, &cell_types2, &decorations2, range);
        assert_eq!(tree, expected, "rebuilt content must still be correct");
    }

    // A shrinking row count must truncate the stale tail rows (not leave
    // them behind with leftover content from a previous, larger range); a
    // subsequent grow must restore the correct row count and content.
    #[test]
    fn rebuild_in_place_shrinks_and_grows_row_count_without_stale_rows() {
        let big_range = RCRange {
            r1: 1,
            c1: 1,
            r2: 4,
            c2: 1,
        };
        let small_range = RCRange {
            r1: 1,
            c1: 1,
            r2: 2,
            c2: 1,
        };

        let styles_big = vec![Fetched::Value(CellStyle::default()); 4];
        let values_big = vec![Fetched::Value(String::new()); 4];
        let cell_types_big = vec![Fetched::Value(CellKind::Text); 4];
        let decorations_big = vec![Fetched::Absent; 4];

        let mut tree = PaneFingerprint::default();
        rebuild_pane_fingerprint_in_place(
            &mut tree,
            &styles_big,
            &values_big,
            &cell_types_big,
            &decorations_big,
            big_range,
        );
        assert_eq!(tree.rows.len(), 4, "must grow from the Default empty tree");

        let styles_small = vec![Fetched::Value(CellStyle::default()); 2];
        let values_small = vec![Fetched::Value(String::new()); 2];
        let cell_types_small = vec![Fetched::Value(CellKind::Text); 2];
        let decorations_small = vec![Fetched::Absent; 2];
        rebuild_pane_fingerprint_in_place(
            &mut tree,
            &styles_small,
            &values_small,
            &cell_types_small,
            &decorations_small,
            small_range,
        );

        assert_eq!(
            tree.rows.len(),
            2,
            "shrinking range must truncate stale rows, not leave them behind"
        );
        let expected = build_pane_fingerprint(
            &styles_small,
            &values_small,
            &cell_types_small,
            &decorations_small,
            small_range,
        );
        assert_eq!(tree, expected);
    }

    // ==========================================================================
    // `plan_pane_repaint` / `diff_changed_cells` planner tests. Pure functions
    // of two already-built trees, with no `Chrome`/`RendererCore` involvement.
    // ==========================================================================

    fn single_col_range(r1: i32, r2: i32) -> RCRange {
        RCRange {
            r1,
            c1: 1,
            r2,
            c2: 1,
        }
    }

    /// Dense, uniform buffers over `range`: every cell gets a plain default
    /// style (no borders) and a value text encoding its own coordinate.
    fn plain_buffers(range: RCRange) -> DenseBuffers {
        let cols = (range.c2 - range.c1 + 1) as usize;
        let rows = (range.r2 - range.r1 + 1) as usize;
        let mut styles = Vec::with_capacity(rows * cols);
        let mut values = Vec::with_capacity(rows * cols);
        let mut cell_types = Vec::with_capacity(rows * cols);
        let mut decorations = Vec::with_capacity(rows * cols);
        for row in range.rows() {
            for col in range.columns() {
                styles.push(Fetched::Value(CellStyle::default()));
                values.push(Fetched::Value(format!("r{row}c{col}")));
                cell_types.push(Fetched::Value(CellKind::Text));
                decorations.push(Fetched::Absent);
            }
        }
        (styles, values, cell_types, decorations)
    }

    fn idx(range: RCRange, row: i32, col: i32) -> usize {
        let cols = (range.c2 - range.c1 + 1) as usize;
        ((row - range.r1) as usize) * cols + (col - range.c1) as usize
    }

    fn set_value(values: &mut [Fetched<String>], range: RCRange, row: i32, col: i32, text: &str) {
        values[idx(range, row, col)] = Fetched::Value(text.to_string());
    }

    fn set_top_border(
        styles: &mut [Fetched<CellStyle>],
        range: RCRange,
        row: i32,
        col: i32,
        present: bool,
    ) {
        let border_top = if present {
            Some(BorderItem {
                style: BorderStyle::Thin,
                color: None,
            })
        } else {
            None
        };
        styles[idx(range, row, col)] = Fetched::Value(CellStyle {
            border: Border {
                top: border_top,
                ..Border::default()
            },
            ..CellStyle::default()
        });
    }

    fn set_bottom_border(
        styles: &mut [Fetched<CellStyle>],
        range: RCRange,
        row: i32,
        col: i32,
        present: bool,
    ) {
        let border_bottom = if present {
            Some(BorderItem {
                style: BorderStyle::Thin,
                color: None,
            })
        } else {
            None
        };
        styles[idx(range, row, col)] = Fetched::Value(CellStyle {
            border: Border {
                bottom: border_bottom,
                ..Border::default()
            },
            ..CellStyle::default()
        });
    }

    fn build(buffers: &DenseBuffers, range: RCRange) -> PaneFingerprint {
        let (styles, values, cell_types, decorations) = buffers;
        build_pane_fingerprint(styles, values, cell_types, decorations, range)
    }

    // Acceptance criterion 1: adjacent changed rows merge into one span.
    #[test]
    fn planning_merges_adjacent_changed_rows_into_one_span() {
        let range = single_col_range(1, 5);
        let painted_buf = plain_buffers(range);
        let painted = build(&painted_buf, range);

        let mut scratch_buf = plain_buffers(range);
        set_value(&mut scratch_buf.1, range, 2, 1, "changed-2");
        set_value(&mut scratch_buf.1, range, 3, 1, "changed-3");
        let scratch = build(&scratch_buf, range);

        let plan = plan_pane_repaint(&painted, &scratch);
        assert_eq!(
            plan,
            RepaintPlan::Rows(vec![RowSpan { r1: 2, r2: 3 }]),
            "two adjacent changed rows must merge into a single span"
        );
    }

    // Acceptance criterion 2: nine disjoint spans select full-pane repaint.
    #[test]
    fn planning_nine_disjoint_spans_select_full_pane_repaint() {
        let range = single_col_range(1, 40);
        let painted_buf = plain_buffers(range);
        let painted = build(&painted_buf, range);

        let mut scratch_buf = plain_buffers(range);
        // Nine isolated rows, each separated by a gap of 2 so `add_rows`
        // never merges them into fewer than nine spans.
        let changed_rows = [2, 5, 8, 11, 14, 17, 20, 23, 26];
        for row in changed_rows {
            set_value(&mut scratch_buf.1, range, row, 1, "changed");
        }
        let scratch = build(&scratch_buf, range);

        let plan = plan_pane_repaint(&painted, &scratch);
        assert_eq!(
            plan,
            RepaintPlan::Full,
            "nine disjoint spans exceed the merge cap and must fall back to Full"
        );
    }

    // Acceptance criterion 3: a border-free changed row selects a row repaint.
    #[test]
    fn planning_border_free_changed_row_selects_row_repaint() {
        let range = single_col_range(1, 5);
        let painted_buf = plain_buffers(range);
        let painted = build(&painted_buf, range);

        let mut scratch_buf = plain_buffers(range);
        set_value(&mut scratch_buf.1, range, 3, 1, "changed-3");
        let scratch = build(&scratch_buf, range);

        let plan = plan_pane_repaint(&painted, &scratch);
        assert_eq!(
            plan,
            RepaintPlan::Rows(vec![RowSpan { r1: 3, r2: 3 }]),
            "a single border-free changed row must select a row repaint"
        );
    }

    // Acceptance criterion 4a: an explicit border that existed in the
    // *painted* tree (and was removed this frame) at an internal top
    // boundary forces Full — the old stroke on the shared edge must still
    // be erased correctly.
    #[test]
    fn planning_old_top_border_at_internal_boundary_selects_full_repaint() {
        let range = single_col_range(1, 5);
        let mut painted_buf = plain_buffers(range);
        set_top_border(&mut painted_buf.0, range, 3, 1, true);
        let painted = build(&painted_buf, range);

        let mut scratch_buf = plain_buffers(range);
        set_top_border(&mut scratch_buf.0, range, 3, 1, false);
        set_value(&mut scratch_buf.1, range, 3, 1, "changed-3");
        let scratch = build(&scratch_buf, range);

        let plan = plan_pane_repaint(&painted, &scratch);
        assert_eq!(
            plan,
            RepaintPlan::Full,
            "an old explicit top border at an internal span boundary must force Full"
        );
    }

    // Acceptance criterion 4b: an explicit border newly added this frame
    // (in *scratch*, absent in painted) at an internal bottom boundary
    // forces Full.
    #[test]
    fn planning_new_bottom_border_at_internal_boundary_selects_full_repaint() {
        let range = single_col_range(1, 5);
        let painted_buf = plain_buffers(range);
        let painted = build(&painted_buf, range);

        let mut scratch_buf = plain_buffers(range);
        set_value(&mut scratch_buf.1, range, 3, 1, "changed-3");
        set_bottom_border(&mut scratch_buf.0, range, 3, 1, true);
        let scratch = build(&scratch_buf, range);

        let plan = plan_pane_repaint(&painted, &scratch);
        assert_eq!(
            plan,
            RepaintPlan::Full,
            "a new explicit bottom border at an internal span boundary must force Full"
        );
    }

    // The pane's outer TOP edge still needs no boundary check: a changed row
    // at the pane's very first row has no "row above" inside the pane, so
    // the top boundary is skipped. But the border-safety flag is a single
    // direction-agnostic `has_any_explicit_border`, so the SAME border
    // conservatively trips row 1's shared BOTTOM boundary with row 2 (the
    // flag can't tell a top border from a bottom one). That conservative
    // over-approximation is the accepted size of the fix — finer per-edge
    // tracking was explicitly deferred — so this forces Full via the bottom
    // boundary.
    #[test]
    fn planning_outer_top_edge_border_conservatively_forces_full() {
        let range = single_col_range(1, 5);
        let mut painted_buf = plain_buffers(range);
        set_top_border(&mut painted_buf.0, range, 1, 1, true);
        let painted = build(&painted_buf, range);

        let mut scratch_buf = plain_buffers(range);
        set_top_border(&mut scratch_buf.0, range, 1, 1, true);
        set_value(&mut scratch_buf.1, range, 1, 1, "changed-1");
        let scratch = build(&scratch_buf, range);

        let plan = plan_pane_repaint(&painted, &scratch);
        assert_eq!(
            plan,
            RepaintPlan::Full,
            "the top boundary is still skipped at the pane's first row, but the single \
             any-edge border flag conservatively trips the shared bottom boundary with row 2"
        );
    }

    // Differing ranges (e.g. a resize straddling the two trees) can't be
    // walked row-for-row meaningfully — the safe fallback is Full.
    #[test]
    fn planning_differing_ranges_fall_back_to_full() {
        let painted_range = single_col_range(1, 5);
        let scratch_range = single_col_range(1, 6);

        let painted_buf = plain_buffers(painted_range);
        let painted = build(&painted_buf, painted_range);

        let scratch_buf = plain_buffers(scratch_range);
        let scratch = build(&scratch_buf, scratch_range);

        let plan = plan_pane_repaint(&painted, &scratch);
        assert_eq!(
            plan,
            RepaintPlan::Full,
            "mismatched ranges must fall back to Full rather than misalign row indices"
        );
    }

    // Acceptance criterion 5: two independent panes (different address-space
    // ranges, e.g. a frozen top pane and the scrolling body pane) plan
    // damage independently — each call reports only its own changed rows.
    #[test]
    fn planning_frozen_panes_plan_damage_independently() {
        let top_pane_range = single_col_range(1, 5);
        let top_painted = build(&plain_buffers(top_pane_range), top_pane_range);
        let mut top_scratch_buf = plain_buffers(top_pane_range);
        set_value(&mut top_scratch_buf.1, top_pane_range, 2, 1, "top-changed");
        let top_scratch = build(&top_scratch_buf, top_pane_range);

        let body_pane_range = single_col_range(10, 15);
        let body_painted = build(&plain_buffers(body_pane_range), body_pane_range);
        let mut body_scratch_buf = plain_buffers(body_pane_range);
        set_value(
            &mut body_scratch_buf.1,
            body_pane_range,
            12,
            1,
            "body-changed",
        );
        let body_scratch = build(&body_scratch_buf, body_pane_range);

        let top_plan = plan_pane_repaint(&top_painted, &top_scratch);
        let body_plan = plan_pane_repaint(&body_painted, &body_scratch);

        assert_eq!(top_plan, RepaintPlan::Rows(vec![RowSpan { r1: 2, r2: 2 }]));
        assert_eq!(
            body_plan,
            RepaintPlan::Rows(vec![RowSpan { r1: 12, r2: 12 }])
        );
    }

    // Equal whole-pane digests must return Skip without needing any
    // row/cell data to differ (the trivial no-op case).
    #[test]
    fn planning_equal_trees_select_skip() {
        let range = single_col_range(1, 5);
        let buffers = plain_buffers(range);
        let painted = build(&buffers, range);
        let scratch = build(&buffers, range);

        assert_eq!(plan_pane_repaint(&painted, &scratch), RepaintPlan::Skip);
    }

    // Diagnostic-only cell-level compare: reports the exact (row, col)
    // coordinates that changed, without asserting the Cartesian product of
    // the two touched rows' cells.
    #[test]
    fn planning_diff_changed_cells_reports_exact_coordinates() {
        let range = range_2x2();
        let painted_buf = plain_buffers(range);
        let painted = build(&painted_buf, range);

        let mut scratch_buf = plain_buffers(range);
        set_value(&mut scratch_buf.1, range, 1, 1, "a1-changed");
        set_value(&mut scratch_buf.1, range, 2, 2, "b2-changed");
        let scratch = build(&scratch_buf, range);

        let mut changed = diff_changed_cells(&painted, &scratch);
        changed.sort();
        assert_eq!(
            changed,
            vec![(1, 1), (2, 2)],
            "must report exactly A1 and B2, not their cross product"
        );
    }

    // ==========================================================================
    // Fix G: cell-level fingerprint cost measurement — see the doc on
    // `CellFingerprint` for what this number is weighed against. Smoke
    // measurement, not a perf gate: no hard timing assertion, since that
    // would make CI flaky on a slower runner. Run with `--nocapture` to see
    // the printed per-call average.
    // ==========================================================================
    #[test]
    fn bench_build_pane_fingerprint_for_a_realistic_pane_size() {
        const ROWS: i32 = 50;
        const COLS: i32 = 20;
        const REPS: u32 = 500;

        let range = RCRange {
            r1: 1,
            c1: 1,
            r2: ROWS,
            c2: COLS,
        };
        let (styles, values, cell_types, decorations) = plain_buffers(range);

        let start = std::time::Instant::now();
        for _ in 0..REPS {
            let tree = build_pane_fingerprint(&styles, &values, &cell_types, &decorations, range);
            std::hint::black_box(&tree);
        }
        let elapsed = start.elapsed();

        let per_call_us = elapsed.as_secs_f64() * 1_000_000.0 / f64::from(REPS);
        println!(
            "build_pane_fingerprint: {per_call_us:.2} us/call over {REPS} reps \
             ({ROWS}x{COLS} = {} cells)",
            ROWS * COLS
        );
    }
}
