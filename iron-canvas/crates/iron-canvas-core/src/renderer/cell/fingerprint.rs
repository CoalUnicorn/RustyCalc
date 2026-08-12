//! Pane -> row content fingerprint tree for the paint-skip optimization.
//!
//! `build_pane_fingerprint` walks the bulk-fetched buffers (`pane_styles`,
//! `pane_values`, `pane_cell_types`, `pane_decorations`) once and produces a
//! fresh `PaneFingerprint`: a whole-pane digest with a `RowFingerprint` per
//! row. Every cell still gets its own `cell_digest`, folded into its row's
//! hasher as it is computed — the tree just doesn't *retain* the per-cell
//! value afterwards, because nothing ever read it back (Stage 6, Gate B).
//! `rebuild_pane_fingerprint_in_place` computes the identical tree but
//! writes it into an existing `PaneFingerprint`, reusing its `rows` Vec
//! rather than allocating a fresh one each call — the hot path (`PaneCache`'s
//! per-pane `PaneFingerprintState`, not `Chrome`, via its persistent
//! `scratch` slot) uses this every frame so the common no-op "skip" case
//! never reallocates the tree. `plan_pane_repaint` is what dispatches on the
//! finished tree: it compares the pane-level `digest` first — equal, and the
//! 5-pass walk is skipped entirely — then falls to comparing row digests to
//! decide between a row-band repaint and a whole-pane repaint. `render_pane`
//! calls it via `PaneFingerprintState::with_trees` on every slots-reuse frame.
//! `rotate_pane_fingerprint_in_place` is the third builder: it derives the
//! same complete tree for a row-scrolled pane out of a truthful prior tree
//! plus the blit's revealed strip, without a full-pane fetch (Stage 6,
//! Gate C).
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
use crate::renderer::prepared::FetchedCells;
use crate::style::{BorderItem, CellDecoration, CellKind, CellStyle};

use crate::types::coord::RCRange;
use crate::types::fetched::Fetched;

/// One row's fingerprint: the folded digest of every cell digest in the row
/// (which itself folds in the row index — see `build_pane_fingerprint`),
/// plus the single boolean a later row-band repaint plan needs —
/// `has_any_explicit_border`: whether ANY cell in the row carries an explicit
/// border rule on ANY of its four edges.
///
/// This is the finest granularity the tree retains. Per-cell digests are
/// computed and folded in, never stored: Stage 6's Gate B measured that no
/// production reader existed for them and that dropping the retained leaves
/// saves 16-29 bytes per visible cell across the two warm trees at no build
/// cost (`docs/performance/2026-08-02-stage-6-render-costs.md`).
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

/// Build the pane -> row fingerprint tree for one pane in a single pass over
/// `range`: each cell's digest folds into its row's hasher, and each row's
/// finished digest folds into the pane's hasher. Same range + same buffers ->
/// same tree (modulo `DefaultHasher` collision).
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
    let row_count = (range.r2 - range.r1 + 1).max(0) as usize;
    let mut rows = Vec::with_capacity(row_count);
    for row in range.rows() {
        rows.push(fingerprint_dense_row_from_channels(
            row,
            styles,
            values,
            cell_types,
            decorations,
            range,
        ));
    }

    PaneFingerprint {
        digest: fold_pane_digest(range, &rows),
        range,
        rows,
    }
}

/// Fingerprint the single model `row` out of the coherent fetched bundle's
/// dense, row-major channels laid out over `range` — the one place a row
/// digest and its border flag are ever computed. Both whole-pane builders and
/// row-shift rotation share it, so "the candidate a rotation produces equals
/// a full rebuild" holds by construction rather than by two loops being kept
/// in sync by hand.
///
/// `row` must lie inside `range`, and the four buffers must be dense over
/// `range` (`idx = (row - r1) * cols + (col - c1)`). The caller decides which
/// buffers those are: a full-pane fetch addresses the whole pane, a blit's
/// revealed strip addresses only the strip — a row's digest depends on its
/// own model address and content, never on which buffer it was read from.
fn fingerprint_dense_row(row: i32, cells: &FetchedCells, range: RCRange) -> RowFingerprint {
    fingerprint_dense_row_from_channels(
        row,
        cells.styles(),
        cells.values(),
        cells.cell_types(),
        cells.decorations(),
        range,
    )
}

fn fingerprint_dense_row_from_channels(
    row: i32,
    styles: &[Fetched<CellStyle>],
    values: &[Fetched<String>],
    cell_types: &[Fetched<CellKind>],
    decorations: &[Fetched<CellDecoration>],
    range: RCRange,
) -> RowFingerprint {
    let cols = (range.c2 - range.c1 + 1).max(0) as usize;
    let base = (row - range.r1).max(0) as usize * cols;

    let mut row_hasher = DefaultHasher::new();
    row_hasher.write_i32(row);

    let mut has_any_explicit_border = false;

    for (col_offset, col) in range.columns().enumerate() {
        let idx = base + col_offset;
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

        cell_digest(row, col, style, value, cell_type, decoration).hash(&mut row_hasher);
    }

    RowFingerprint {
        digest: row_hasher.finish(),
        has_any_explicit_border,
    }
}

/// Fold a pane's address-space `range` and its finished row digests into the
/// whole-pane digest. Range first, then every row digest in row order — the
/// order is part of the hash, so this is the single definition of it.
fn fold_pane_digest(range: RCRange, rows: &[RowFingerprint]) -> u64 {
    let mut pane_hasher = DefaultHasher::new();
    pane_hasher.write_i32(range.r1);
    pane_hasher.write_i32(range.c1);
    pane_hasher.write_i32(range.r2);
    pane_hasher.write_i32(range.c2);
    for row in rows {
        row.digest.hash(&mut pane_hasher);
    }
    pane_hasher.finish()
}

/// In-place twin of [`build_pane_fingerprint`]: identical hashing (every
/// digest `target` ends up with is byte-for-byte what `build_pane_fingerprint`
/// would produce for the same inputs — see the equivalence test below), but
/// writes into `target` instead of returning a fresh tree, reusing its
/// `rows` Vec rather than allocating a new one. This is the "keep both
/// trees' vector allocations warm" mechanism: called every frame against the
/// persistent `scratch` slot, a same-size pane never triggers a single `Vec`
/// allocation after its first paint.
///
/// Row-count changes resize `target.rows` in place rather than replacing
/// the Vec: `truncate` drops any excess rows when the pane shrank; bare
/// zero-valued `RowFingerprint`s are pushed when it grew, then filled by the
/// same per-row loop as every other row this call (so a grown pane allocates
/// only the newly-needed capacity, not a full fresh tree). A column-count
/// change costs nothing at all now that rows are fixed-size: only the folded
/// digest they carry differs.
#[allow(dead_code)] // The parallel-slice form remains a focused test helper.
pub(crate) fn rebuild_pane_fingerprint_in_place(
    target: &mut PaneFingerprint,
    styles: &[Fetched<CellStyle>],
    values: &[Fetched<String>],
    cell_types: &[Fetched<CellKind>],
    decorations: &[Fetched<CellDecoration>],
    range: RCRange,
) {
    resize_rows(target, (range.r2 - range.r1 + 1).max(0) as usize);

    for (row_offset, row) in range.rows().enumerate() {
        target.rows[row_offset] = fingerprint_dense_row_from_channels(
            row,
            styles,
            values,
            cell_types,
            decorations,
            range,
        );
    }

    target.range = range;
    target.digest = fold_pane_digest(range, &target.rows);
}

/// In-place bundle-owned twin used by the renderer hot path. The legacy
/// channel-parameter function above remains for focused fingerprint tests;
/// all production callers pass the coherent fetched bundle through here.
pub(crate) fn rebuild_pane_fingerprint_in_place_from_cells(
    target: &mut PaneFingerprint,
    cells: &FetchedCells,
    range: RCRange,
) {
    resize_rows(target, (range.r2 - range.r1 + 1).max(0) as usize);

    for (row_offset, row) in range.rows().enumerate() {
        target.rows[row_offset] = fingerprint_dense_row_from_cells(cells, row, range);
    }

    target.range = range;
    target.digest = fold_pane_digest(range, &target.rows);
}

fn fingerprint_dense_row_from_cells(
    cells: &FetchedCells,
    row: i32,
    range: RCRange,
) -> RowFingerprint {
    fingerprint_dense_row(row, cells, range)
}

/// Resize `target.rows` to `row_count` in place, reusing the Vec: `truncate`
/// drops any excess rows when the pane shrank, bare zero-valued rows are
/// pushed when it grew. Every entry is overwritten by the caller's own
/// per-row loop before it is read, so the pushed placeholders never reach a
/// digest.
fn resize_rows(target: &mut PaneFingerprint, row_count: usize) {
    if target.rows.len() > row_count {
        target.rows.truncate(row_count);
    } else {
        while target.rows.len() < row_count {
            target.rows.push(RowFingerprint {
                digest: 0,
                has_any_explicit_border: false,
            });
        }
    }
}

// ==============================================================================
// Row-axis rotation — deriving a complete candidate without a full-pane fetch
// ==============================================================================
//
// A row scroll moves whole rows of pixels; the model rows that survived the
// shift kept their address, their content and therefore their digest. So a
// pane whose retained tree is *provably* truthful for `prev_range` can carry
// those overlapping rows across into `new_range` and only fingerprint the
// rows the blit actually revealed, from the strip the blit already fetched.
//
// Two separate facts have to hold before that is sound, and only one of them
// is geometry:
//
// 1. the history must be exact — a `Splice`-kind commit (Damage, or a strip
//    that changed pixels) leaves the painted tree's range untouched, so range
//    equality alone proves nothing. That check is the caller's
//    (`PaneFingerprintState::build_row_shift_candidate` gates on
//    `FingerprintTruth::Exact`);
// 2. the shape must rotate — same columns, same row extent, non-empty
//    overlap, and a strip that names every row the overlap cannot supply.
//    That is this module's half, below.
//
// Column-axis rotation is deliberately absent (Stage 6 requirement 8): a
// horizontal shift changes which columns every row contains, so no row digest
// survives it.

/// Why a row-shift candidate could not be derived. Every rejection is named
/// rather than collapsed into `None`, so the caller (and Stage 6's tests) can
/// tell a column-axis request apart from stale history apart from a strip
/// that didn't cover what it had to.
///
/// Every variant means the same thing operationally — no candidate, fall back
/// — but a single unnamed "no" would make an incomplete strip look like a
/// deliberate policy decision in a trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowShiftIneligible {
    /// The retained tree is not known to describe the pixels it claims to
    /// (`FingerprintTruth::Stale`) — including the never-painted default.
    StaleHistory,
    /// The retained tree does not describe `prev_range`, so its rows cannot
    /// be addressed against the shift the caller is performing.
    PriorRangeMismatch,
    /// The two ranges disagree on the orthogonal (column) axis.
    ColumnBounds,
    /// The two ranges disagree on the scroll axis' inclusive extent.
    RowExtent,
    /// The two ranges share no model row, so nothing survives to rotate.
    EmptyOverlap,
    /// The strip does not span the full pane width, its channels are not
    /// dense over `strip_range`, or some new row is named by neither the
    /// strip nor the overlap.
    IncompleteStrip,
    /// A column-axis shift was requested. Rotation is row-axis only.
    ColumnAxis,
}

/// Outcome of deriving a row-shift candidate: either a complete tree for the
/// new range, owned by the caller, or a named reason there isn't one.
///
/// `Rotated` is a *complete* candidate — every row of `new_range`, the
/// recomputed pane digest, the new range — indistinguishable from what a
/// full-pane rebuild over the post-shift buffers would produce. It is not a
/// patch a later step has to finish.
#[derive(Debug, PartialEq)]
pub(crate) enum RowShiftFingerprint {
    Rotated(PaneFingerprint),
    Ineligible(RowShiftIneligible),
}

/// Derive `prior`'s tree rotated onto `new_range` into `target`: overlapping
/// model rows are carried across from `prior` unchanged (same address, same
/// content, same digest), every row the widened `strip_range` names is
/// fingerprinted fresh from the already-fetched `strip`, and the whole-pane
/// digest is recomputed for `new_range`.
///
/// The widened strip wins over the overlap wherever the two meet. A blit
/// widens its revealed strip to the pixel clip, so the strip can reach one
/// row back into the kept band; those rows are repainted from the strip's
/// values, so the strip — not the older history — is what the pixels will
/// show.
///
/// `prior` and `target` must be different trees. Nothing is written to
/// `target` unless the whole shape validates first, so a rejected rotation
/// leaves the caller's scratch slot exactly as warm (and exactly as
/// meaningless) as it found it.
///
/// This function knows nothing about truth: it is pure geometry over two
/// trees and a strip. `PaneFingerprintState::build_row_shift_candidate` is
/// what refuses to call it on history that isn't `Exact`.
pub(crate) fn rotate_pane_fingerprint_in_place(
    target: &mut PaneFingerprint,
    prior: &PaneFingerprint,
    new_range: RCRange,
    strip: &FetchedCells,
    strip_range: RCRange,
) -> Result<(), RowShiftIneligible> {
    let prev_range = prior.range;

    // Same compatibility discipline `shift_is_safe` applies to the buffers
    // themselves: identical orthogonal bounds, equal inclusive extent on the
    // scroll axis. A row-shift candidate must not be derivable for a shape
    // the blit itself would refuse to rotate.
    if prev_range.c1 != new_range.c1 || prev_range.c2 != new_range.c2 {
        return Err(RowShiftIneligible::ColumnBounds);
    }
    if (new_range.r2 - new_range.r1) != (prev_range.r2 - prev_range.r1) {
        return Err(RowShiftIneligible::RowExtent);
    }

    let prev_row_count = (prev_range.r2 - prev_range.r1 + 1).max(0) as usize;
    if prior.rows.len() != prev_row_count {
        return Err(RowShiftIneligible::PriorRangeMismatch);
    }

    let overlap_r1 = prev_range.r1.max(new_range.r1);
    let overlap_r2 = prev_range.r2.min(new_range.r2);
    if overlap_r1 > overlap_r2 {
        return Err(RowShiftIneligible::EmptyOverlap);
    }

    // The strip has to be usable as a full-pane-width dense buffer: same
    // columns as the pane, and all four channels dense over `strip_range`.
    if strip_range.c1 != new_range.c1 || strip_range.c2 != new_range.c2 {
        return Err(RowShiftIneligible::IncompleteStrip);
    }
    let strip_rows = (strip_range.r2 - strip_range.r1 + 1).max(0) as usize;
    if !strip.is_dense_for(strip_range) {
        return Err(RowShiftIneligible::IncompleteStrip);
    }

    let from_strip = |row: i32| strip_rows > 0 && row >= strip_range.r1 && row <= strip_range.r2;
    let from_overlap = |row: i32| row >= overlap_r1 && row <= overlap_r2;
    if !new_range
        .rows()
        .all(|row| from_strip(row) || from_overlap(row))
    {
        return Err(RowShiftIneligible::IncompleteStrip);
    }

    resize_rows(target, (new_range.r2 - new_range.r1 + 1).max(0) as usize);
    for (row_offset, row) in new_range.rows().enumerate() {
        target.rows[row_offset] = if from_strip(row) {
            fingerprint_dense_row_from_cells(strip, row, strip_range)
        } else {
            prior.rows[(row - prev_range.r1) as usize].clone()
        };
    }

    target.range = new_range;
    target.digest = fold_pane_digest(new_range, &target.rows);
    Ok(())
}

/// Fold one cell's row + col + style + formatted value + cell kind +
/// decoration into a single `u64`. The result is folded into the row hasher
/// and dropped — no caller retains it. The address is folded in here so two
/// cells with identical content at different addresses still produce
/// distinct digests, which is what makes a row's folded digest sensitive to
/// *which* column changed. `Absent` and `BridgeFailed` all hash as the
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

    /// Model rows whose digest differs between two trees over the same range.
    /// Rows are the finest unit the tree retains, so this is the finest
    /// granularity a test can assert on.
    fn changed_rows(before: &PaneFingerprint, after: &PaneFingerprint) -> Vec<i32> {
        assert_eq!(before.range, after.range, "comparable trees share a range");
        before
            .rows
            .iter()
            .zip(after.rows.iter())
            .enumerate()
            .filter(|(_, (b, a))| b.digest != a.digest)
            .map(|(row_offset, _)| before.range.r1 + row_offset as i32)
            .collect()
    }

    // Acceptance 2: changing one cell's value changes exactly that cell's row
    // digest and the pane digest — every other row's digest stays put.
    #[test]
    fn changing_one_value_changes_exactly_its_row_and_the_pane_digest() {
        let range = range_2x2();
        let (styles, values, cell_types, decorations) =
            dense_buffers_with_value_at((1, 1), "before");
        let before = build_pane_fingerprint(&styles, &values, &cell_types, &decorations, range);

        let (styles2, values2, cell_types2, decorations2) =
            dense_buffers_with_value_at((1, 1), "after");
        let after = build_pane_fingerprint(&styles2, &values2, &cell_types2, &decorations2, range);

        assert_ne!(before.digest, after.digest, "pane digest must change");
        assert_eq!(
            changed_rows(&before, &after),
            vec![1],
            "exactly the edited cell's row must change"
        );
    }

    // Changing style, kind, or decoration (independently) must each change
    // exactly the touched cell's row too — value isn't the only signal that
    // must participate in the hash domain.
    #[test]
    fn changing_style_changes_its_row_digest() {
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
        assert_eq!(changed_rows(&before, &after), vec![1]);
    }

    #[test]
    fn changing_cell_kind_changes_its_row_digest() {
        let range = range_2x2();
        let (styles, values, cell_types, decorations) = dense_buffers_with_value_at((1, 1), "same");
        let before = build_pane_fingerprint(&styles, &values, &cell_types, &decorations, range);

        let mut cell_types2 = cell_types.clone();
        cell_types2[0] = Fetched::Value(CellKind::Number);
        let after = build_pane_fingerprint(&styles, &values, &cell_types2, &decorations, range);

        assert_ne!(before.digest, after.digest);
        assert_eq!(changed_rows(&before, &after), vec![1]);
    }

    #[test]
    fn changing_painted_decoration_changes_its_row_digest() {
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
        assert_eq!(changed_rows(&before, &after), vec![1]);
    }

    // Acceptance 3: A1 + B2 changes (two different rows) change exactly those
    // two rows' digests.
    #[test]
    fn two_cells_in_different_rows_change_exactly_two_row_digests() {
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

        assert_eq!(
            changed_rows(&before, &after),
            vec![1, 2],
            "A1 + B2 must change exactly their own two rows"
        );
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

        assert_eq!(
            tree.rows.len(),
            3,
            "one row entry per model row, hidden or not"
        );

        // Columns are folded, not retained, so "dense indices retained" means
        // every dense column slot still reaches its row's hasher — nothing is
        // skipped or coalesced. Walk every slot in the buffer and confirm each
        // one moves exactly the row it belongs to.
        for (row_offset, row) in range.rows().enumerate() {
            for col_offset in 0..2 {
                let mut touched = values.clone();
                touched[row_offset * 2 + col_offset] = Fetched::Value("touched".to_string());
                let after =
                    build_pane_fingerprint(&styles, &touched, &cell_types, &decorations, range);
                assert_eq!(
                    changed_rows(&tree, &after),
                    vec![row],
                    "dense slot ({row_offset}, {col_offset}) must move exactly its own row"
                );
            }
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
    // reallocate the `rows` Vec. Capacity staying put is a concrete, checkable
    // proxy for "no allocation happened" without an allocation-counting
    // harness.
    #[test]
    fn rebuild_in_place_keeps_row_vec_capacity_warm() {
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
            "rows Vec must not reallocate on a same-size rebuild"
        );
        // Capacity staying warm must not come at the cost of correctness.
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
    // `plan_pane_repaint` planner tests. A pure function of two already-built
    // trees, with no `Chrome`/`RendererCore` involvement.
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

    // ==========================================================================
    // Fix G: fingerprint construction cost — the per-call number Stage 6's
    // report weighs the tree's shape against. Smoke measurement, not a perf
    // gate: no hard timing assertion, since that would make CI flaky on a
    // slower runner. Run with `--nocapture` to see the printed per-call
    // average.
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

    // ==========================================================================
    // Stage 6, Task 4: the retained-leaf reference shape.
    //
    // `LeafPane` is the fingerprint tree exactly as production carried it
    // *before* Stage 6 collapsed it to pane -> row: a `u64` leaf retained per
    // cell, folded into the row hasher, folded into the pane hasher. Production
    // no longer stores leaves; this reference does, so the collapse is
    // provable rather than asserted. The same equivalence test passes against
    // the leaf-retaining production tree before the change and against the
    // row-only tree after it, which is what "the hash domain did not move"
    // means operationally.
    //
    // It stays inside this private module's own test scope — no Cargo feature,
    // no `pub` test hook, nothing outside this file can see it.
    // ==========================================================================

    #[derive(Debug, Clone, PartialEq)]
    struct LeafRow {
        digest: u64,
        has_any_explicit_border: bool,
        cells: Vec<u64>,
    }

    #[derive(Debug, Clone, Default, PartialEq)]
    struct LeafPane {
        range: RCRange,
        digest: u64,
        rows: Vec<LeafRow>,
    }

    /// Faithful copy of the pre-collapse `rebuild_pane_fingerprint_in_place`,
    /// leaf `Vec` and all. In-place like its production twin so the Stage 6
    /// A/B stays an apples-to-apples comparison of two warm builders.
    fn rebuild_leaf_reference_in_place(
        target: &mut LeafPane,
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
                target.rows.push(LeafRow {
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
                row_entry.cells.push(digest);
            }

            let row_digest = row_hasher.finish();
            row_digest.hash(&mut pane_hasher);
            row_entry.digest = row_digest;
            row_entry.has_any_explicit_border = has_any_explicit_border;
        }

        target.range = range;
        target.digest = pane_hasher.finish();
    }

    fn leaf_retained_bytes(tree: &LeafPane) -> usize {
        let rows = tree.rows.capacity() * std::mem::size_of::<LeafRow>();
        let leaves: usize = tree
            .rows
            .iter()
            .map(|row| row.cells.capacity() * std::mem::size_of::<u64>())
            .sum();
        rows + leaves
    }

    /// Explicit borders every fifth row — the "bordered" corpus.
    fn bordered_buffers(range: RCRange) -> DenseBuffers {
        let (mut styles, values, cell_types, decorations) = plain_buffers(range);
        for (row_offset, row) in range.rows().enumerate() {
            for col_offset in 0..range.columns().count() {
                let i = row_offset * (range.c2 - range.c1 + 1).max(0) as usize + col_offset;
                if row % 5 == 0 {
                    styles[i] = Fetched::Value(CellStyle {
                        border: Border {
                            bottom: Some(BorderItem {
                                style: BorderStyle::Thin,
                                color: None,
                            }),
                            ..Border::default()
                        },
                        ..CellStyle::default()
                    });
                }
            }
        }
        (styles, values, cell_types, decorations)
    }

    /// CF data bars every fourth column — the "decorated" corpus.
    fn decorated_buffers(range: RCRange) -> DenseBuffers {
        let (styles, values, cell_types, mut decorations) = plain_buffers(range);
        for (row_offset, row) in range.rows().enumerate() {
            for (col_offset, col) in range.columns().enumerate() {
                let i = row_offset * (range.c2 - range.c1 + 1).max(0) as usize + col_offset;
                if col % 4 == 0 {
                    decorations[i] = Fetched::Value(CellDecoration::DataBar(DataBarSpec {
                        fraction: f64::from(row % 10) / 10.0,
                        color: "#3366cc".to_string(),
                    }));
                }
            }
        }
        (styles, values, cell_types, decorations)
    }

    /// Bordered and decorated at once — the styled half of Stage 6's workload
    /// matrix, so the measurement is not taken only over trivially cheap
    /// default styles.
    fn styled_buffers(range: RCRange) -> DenseBuffers {
        let (styles, values, cell_types, _) = bordered_buffers(range);
        let (_, _, _, decorations) = decorated_buffers(range);
        (styles, values, cell_types, decorations)
    }

    /// Both production builders must fold the given buffers to exactly the
    /// digests the retained-leaf reference produces: same pane digest, same
    /// per-row digests, same per-row border flags, same range.
    fn assert_matches_leaf_reference(name: &str, buffers: &DenseBuffers, range: RCRange) {
        let (styles, values, cell_types, decorations) = buffers;
        let cols = (range.c2 - range.c1 + 1).max(0) as usize;

        let mut reference = LeafPane::default();
        rebuild_leaf_reference_in_place(
            &mut reference,
            styles,
            values,
            cell_types,
            decorations,
            range,
        );
        // Guard against a degenerate reference silently agreeing with anything:
        // it must actually retain one leaf per model column.
        for row in &reference.rows {
            assert_eq!(
                row.cells.len(),
                cols,
                "{name}: the reference must retain one leaf per model column"
            );
        }

        let fresh = build_pane_fingerprint(styles, values, cell_types, decorations, range);
        let mut rebuilt = PaneFingerprint::default();
        rebuild_pane_fingerprint_in_place(
            &mut rebuilt,
            styles,
            values,
            cell_types,
            decorations,
            range,
        );

        for (builder, produced) in [("build", &fresh), ("rebuild_in_place", &rebuilt)] {
            assert_eq!(produced.range, reference.range, "{name}/{builder}: range");
            assert_eq!(
                produced.digest, reference.digest,
                "{name}/{builder}: pane digest must equal the retained-leaf shape's"
            );
            assert_eq!(
                produced.rows.len(),
                reference.rows.len(),
                "{name}/{builder}: one entry per model row"
            );
            for (i, (row, reference_row)) in
                produced.rows.iter().zip(reference.rows.iter()).enumerate()
            {
                assert_eq!(
                    row.digest, reference_row.digest,
                    "{name}/{builder}: row {i} digest must equal the retained-leaf shape's"
                );
                assert_eq!(
                    row.has_any_explicit_border, reference_row.has_any_explicit_border,
                    "{name}/{builder}: row {i} border flag must equal the retained-leaf shape's"
                );
            }
        }
    }

    // Acceptance criterion 1 of Stage 6 Task 4, and the reason the collapse is
    // safe: plain, bordered, decorated and both-at-once buffers all fold to the
    // same row and pane digests with or without retained cell leaves.
    #[test]
    fn row_and_pane_digests_match_the_retained_leaf_shape() {
        let range = RCRange {
            r1: 1,
            c1: 1,
            r2: 12,
            c2: 8,
        };
        assert_matches_leaf_reference("plain", &plain_buffers(range), range);
        assert_matches_leaf_reference("bordered", &bordered_buffers(range), range);
        assert_matches_leaf_reference("decorated", &decorated_buffers(range), range);
        assert_matches_leaf_reference("styled", &styled_buffers(range), range);
    }

    fn median(mut samples: Vec<f64>) -> f64 {
        samples.sort_by(|a, b| a.total_cmp(b));
        match samples.len() {
            0 => 0.0,
            n if n % 2 == 1 => samples[n / 2],
            n => f64::midpoint(samples[n / 2 - 1], samples[n / 2]),
        }
    }

    fn row_only_retained_bytes(tree: &PaneFingerprint) -> usize {
        tree.rows.capacity() * std::mem::size_of::<RowFingerprint>()
    }

    /// Stage 6 Gate B evidence, retained after Task 4 as the regression guard
    /// that the shape stayed collapsed: it re-derives, on demand, the
    /// retained-bytes and build-cost gap between the pre-collapse leaf shape
    /// (the local reference) and the production row-only tree. Ignored: it is a
    /// release-mode measurement whose numbers belong in
    /// `docs/performance/2026-08-02-stage-6-render-costs.md`, not a CI
    /// assertion — a timing gate here would fail on a slow runner. The
    /// digest-equivalence half of Gate B is *not* ignored; it lives in
    /// `row_and_pane_digests_match_the_retained_leaf_shape`.
    ///
    /// Protocol, per the plan: both targets are warmed to their final
    /// capacities before the clock starts, samples are batched (so
    /// `Instant::now` resolution is not the thing being measured), `black_box`
    /// defeats the optimizer, and the A/B order alternates per sample so
    /// thermal drift cannot systematically favour whichever ran first.
    #[test]
    #[ignore = "Stage 6 manual measurement probe: retained-leaf reference vs production row-only fingerprint build; run with --release --ignored --nocapture --test-threads=1"]
    fn stage6_compare_fingerprint_shapes() {
        const WARMUP_REPS: u32 = 200;
        const BATCH: u32 = 25;
        const SAMPLES: usize = 41;

        let workloads: [(&str, RCRange, DenseBuffers); 4] = [
            (
                "prod29x21-plain",
                RCRange {
                    r1: 1,
                    c1: 1,
                    r2: 29,
                    c2: 21,
                },
                plain_buffers(RCRange {
                    r1: 1,
                    c1: 1,
                    r2: 29,
                    c2: 21,
                }),
            ),
            (
                "prod29x21-styled",
                RCRange {
                    r1: 1,
                    c1: 1,
                    r2: 29,
                    c2: 21,
                },
                styled_buffers(RCRange {
                    r1: 1,
                    c1: 1,
                    r2: 29,
                    c2: 21,
                }),
            ),
            (
                "stress50x20-plain",
                RCRange {
                    r1: 1,
                    c1: 1,
                    r2: 50,
                    c2: 20,
                },
                plain_buffers(RCRange {
                    r1: 1,
                    c1: 1,
                    r2: 50,
                    c2: 20,
                }),
            ),
            (
                "stress50x20-styled",
                RCRange {
                    r1: 1,
                    c1: 1,
                    r2: 50,
                    c2: 20,
                },
                styled_buffers(RCRange {
                    r1: 1,
                    c1: 1,
                    r2: 50,
                    c2: 20,
                }),
            ),
        ];

        println!("# stage6-fingerprint-shapes v1");
        for (name, range, buffers) in &workloads {
            let (styles, values, cell_types, decorations) = buffers;
            let cells = (range.r2 - range.r1 + 1) as usize * (range.c2 - range.c1 + 1) as usize;

            let mut full = LeafPane::default();
            let mut row_only = PaneFingerprint::default();

            // Warm-up doubles as the equivalence check: an A/B between two
            // builders that disagree measures nothing.
            for _ in 0..WARMUP_REPS {
                rebuild_leaf_reference_in_place(
                    &mut full,
                    styles,
                    values,
                    cell_types,
                    decorations,
                    *range,
                );
                rebuild_pane_fingerprint_in_place(
                    &mut row_only,
                    styles,
                    values,
                    cell_types,
                    decorations,
                    *range,
                );
                std::hint::black_box((&full, &row_only));
            }
            assert_eq!(
                full.digest, row_only.digest,
                "{name}: the row-only twin must fold to the same pane digest"
            );
            assert_eq!(
                full.rows.len(),
                row_only.rows.len(),
                "{name}: both shapes must retain one entry per row"
            );
            for (i, (a, b)) in full.rows.iter().zip(row_only.rows.iter()).enumerate() {
                assert_eq!(a.digest, b.digest, "{name}: row {i} digest must match");
                assert_eq!(
                    a.has_any_explicit_border, b.has_any_explicit_border,
                    "{name}: row {i} border flag must match"
                );
            }

            let mut full_us = Vec::with_capacity(SAMPLES);
            let mut row_only_us = Vec::with_capacity(SAMPLES);
            for sample in 0..SAMPLES {
                let mut time_full = || {
                    let start = std::time::Instant::now();
                    for _ in 0..BATCH {
                        rebuild_leaf_reference_in_place(
                            &mut full,
                            styles,
                            values,
                            cell_types,
                            decorations,
                            *range,
                        );
                        std::hint::black_box(&full);
                    }
                    start.elapsed().as_secs_f64() * 1_000_000.0 / f64::from(BATCH)
                };
                let mut time_row_only = || {
                    let start = std::time::Instant::now();
                    for _ in 0..BATCH {
                        rebuild_pane_fingerprint_in_place(
                            &mut row_only,
                            styles,
                            values,
                            cell_types,
                            decorations,
                            *range,
                        );
                        std::hint::black_box(&row_only);
                    }
                    start.elapsed().as_secs_f64() * 1_000_000.0 / f64::from(BATCH)
                };
                // Alternate which shape runs first: thermal drift and cache
                // residency then bias both halves equally.
                if sample % 2 == 0 {
                    full_us.push(time_full());
                    row_only_us.push(time_row_only());
                } else {
                    row_only_us.push(time_row_only());
                    full_us.push(time_full());
                }
            }

            let full_median = median(full_us.clone());
            let row_only_median = median(row_only_us.clone());
            let delta_pct = if full_median > 0.0 {
                (row_only_median - full_median) / full_median * 100.0
            } else {
                0.0
            };

            let full_bytes = leaf_retained_bytes(&full);
            let row_only_bytes = row_only_retained_bytes(&row_only);
            let leaf_cap: usize = full.rows.iter().map(|row| row.cells.capacity()).sum();
            // Two warm trees per pane (`painted` + `scratch`), which is the unit
            // Gate B's "16 fewer bytes per visible cell" threshold is stated in.
            let saved_per_cell_two_trees =
                2.0 * (full_bytes as f64 - row_only_bytes as f64) / cells as f64;

            println!(
                "stage6-fingerprint {name} cells={cells} \
                 full_leaf_median_us={full_median:.3} row_only_median_us={row_only_median:.3} \
                 row_only_delta_pct={delta_pct:+.2} samples={SAMPLES} batch={BATCH}"
            );
            println!(
                "stage6-fingerprint {name} full_leaf_min_us={:.3} full_leaf_max_us={:.3} \
                 row_only_min_us={:.3} row_only_max_us={:.3}",
                full_us.iter().copied().fold(f64::INFINITY, f64::min),
                full_us.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                row_only_us.iter().copied().fold(f64::INFINITY, f64::min),
                row_only_us
                    .iter()
                    .copied()
                    .fold(f64::NEG_INFINITY, f64::max),
            );
            println!(
                "stage6-fingerprint {name} full_leaf_rows_cap={} full_leaf_cells_cap={leaf_cap} \
                 full_leaf_bytes={full_bytes} row_only_rows_cap={} row_only_bytes={row_only_bytes} \
                 saved_bytes_per_cell_two_trees={saved_per_cell_two_trees:.2}",
                full.rows.capacity(),
                row_only.rows.capacity(),
            );
        }
    }

    // ==========================================================================
    // Stage 6, Task 5: row-axis rotation.
    //
    // Nothing in production calls the rotation path yet — Task 6 wires it into
    // blit preparation. These tests are its only callers, and they hold it to
    // one standard throughout: a rotated candidate must be *indistinguishable*
    // from the tree a full-pane rebuild over the post-shift buffers would
    // produce. `PaneFingerprint`'s `PartialEq` covers exactly what that means
    // — range, whole-pane digest, every row digest, every row border flag — so
    // a single `assert_eq!` on the whole tree is the strongest available
    // assertion, not a loose one.
    // ==========================================================================

    use crate::geometry::prim::Axis;
    use crate::renderer::cache::PaneBuffers;

    fn rows_1_to(r1: i32, r2: i32) -> RCRange {
        RCRange {
            r1,
            c1: 1,
            r2,
            c2: 4,
        }
    }

    /// A pane whose painted tree is `Exact` for `range` — the state a healthy
    /// whole-pane commit leaves behind, and the only state rotation accepts.
    fn pane_with_exact_history(buffers: &DenseBuffers, range: RCRange) -> PaneBuffers {
        let pane = PaneBuffers::default();
        pane.fingerprint.install(build(buffers, range));
        pane
    }

    fn strip_from(buffers: DenseBuffers) -> FetchedCells {
        let (styles, values, cell_types, decorations) = buffers;
        FetchedCells::from_parts(styles, values, cell_types, decorations)
    }

    // Acceptance 1 (down): rows 1..=10 scrolled to 4..=13. The revealed strip
    // is what `compute_strip` produces for a downward row scroll — from the
    // old overflow row (`prev.r2`, whose pixels were off-canvas) to the new
    // last row.
    #[test]
    fn stage6_row_shift_down_candidate_matches_a_full_rebuild() {
        let prev_range = rows_1_to(1, 10);
        let new_range = rows_1_to(4, 13);
        let strip_range = rows_1_to(10, 13);

        let pane = pane_with_exact_history(&plain_buffers(prev_range), prev_range);
        let strip = strip_from(plain_buffers(strip_range));

        let candidate = pane.fingerprint.build_row_shift_candidate(
            prev_range,
            new_range,
            Axis::Row,
            &strip,
            strip_range,
        );

        assert_eq!(
            candidate,
            RowShiftFingerprint::Rotated(build(&plain_buffers(new_range), new_range)),
            "a downward rotation must be indistinguishable from a full rebuild"
        );
    }

    // Acceptance 1 (up): the mirror case. The strip is the band above the old
    // first row.
    #[test]
    fn stage6_row_shift_up_candidate_matches_a_full_rebuild() {
        let prev_range = rows_1_to(4, 13);
        let new_range = rows_1_to(1, 10);
        let strip_range = rows_1_to(1, 3);

        let pane = pane_with_exact_history(&plain_buffers(prev_range), prev_range);
        let strip = strip_from(plain_buffers(strip_range));

        let candidate = pane.fingerprint.build_row_shift_candidate(
            prev_range,
            new_range,
            Axis::Row,
            &strip,
            strip_range,
        );

        assert_eq!(
            candidate,
            RowShiftFingerprint::Rotated(build(&plain_buffers(new_range), new_range)),
            "an upward rotation must be indistinguishable from a full rebuild"
        );
    }

    // Acceptance 3: revealed rows are fingerprinted from the strip the blit
    // already fetched — the values a painter drain would later consume — not
    // from anything the prior tree knew. Both halves of a row's fingerprint
    // have to come from there: its digest (an edited value) and its border
    // flag (a border that exists only in the revealed band).
    #[test]
    fn stage6_revealed_rows_are_fingerprinted_from_the_prepared_strip() {
        let prev_range = rows_1_to(1, 10);
        let new_range = rows_1_to(4, 13);
        let strip_range = rows_1_to(10, 13);

        let pane = pane_with_exact_history(&plain_buffers(prev_range), prev_range);

        let mut strip_buffers = plain_buffers(strip_range);
        set_value(&mut strip_buffers.1, strip_range, 13, 2, "revealed-edit");
        set_bottom_border(&mut strip_buffers.0, strip_range, 12, 3, true);
        let strip = strip_from(strip_buffers);

        let mut expected_buffers = plain_buffers(new_range);
        set_value(&mut expected_buffers.1, new_range, 13, 2, "revealed-edit");
        set_bottom_border(&mut expected_buffers.0, new_range, 12, 3, true);
        let expected = build(&expected_buffers, new_range);

        let candidate = pane.fingerprint.build_row_shift_candidate(
            prev_range,
            new_range,
            Axis::Row,
            &strip,
            strip_range,
        );

        assert_eq!(
            candidate,
            RowShiftFingerprint::Rotated(expected),
            "revealed rows must carry the strip's values and border flags"
        );
    }

    // A blit widens its revealed strip to the pixel clip, so the strip can
    // reach back over a row that also survived the shift. Those pixels are
    // repainted from the strip, so the strip — not the older history — is what
    // the candidate must describe.
    #[test]
    fn stage6_widened_strip_rows_override_carried_history() {
        let prev_range = rows_1_to(1, 10);
        let new_range = rows_1_to(4, 13);
        // Row 9 lies in BOTH the surviving overlap (4..=10) and the widened
        // strip (9..=13).
        let strip_range = rows_1_to(9, 13);

        let pane = pane_with_exact_history(&plain_buffers(prev_range), prev_range);

        let mut strip_buffers = plain_buffers(strip_range);
        set_value(
            &mut strip_buffers.1,
            strip_range,
            9,
            2,
            "repainted-from-strip",
        );
        let strip = strip_from(strip_buffers);

        let mut expected_buffers = plain_buffers(new_range);
        set_value(
            &mut expected_buffers.1,
            new_range,
            9,
            2,
            "repainted-from-strip",
        );

        let candidate = pane.fingerprint.build_row_shift_candidate(
            prev_range,
            new_range,
            Axis::Row,
            &strip,
            strip_range,
        );

        assert_eq!(
            candidate,
            RowShiftFingerprint::Rotated(build(&expected_buffers, new_range)),
            "a widened strip row must win over the history it overlaps"
        );
        assert_ne!(
            candidate,
            RowShiftFingerprint::Rotated(build(&plain_buffers(new_range), new_range)),
            "the guard: carrying row 9's old history across would produce a different tree"
        );
    }

    // Acceptance 2: a Damage strip changes pixels without changing the painted
    // tree's range, so range equality proves nothing. Only the truth state
    // does — and it is checked before the range is even looked at.
    #[test]
    fn stage6_stale_history_is_rejected_even_when_its_range_matches() {
        let prev_range = rows_1_to(1, 10);
        let new_range = rows_1_to(4, 13);
        let strip_range = rows_1_to(10, 13);
        let strip = strip_from(plain_buffers(strip_range));

        let pane = pane_with_exact_history(&plain_buffers(prev_range), prev_range);
        // Exactly what a Damage/strip commit will do from Task 6 on: the tree
        // stays, its claim to describe the pixels does not.
        pane.fingerprint.mark_stale();

        assert_eq!(
            pane.fingerprint.build_row_shift_candidate(
                prev_range,
                new_range,
                Axis::Row,
                &strip,
                strip_range,
            ),
            RowShiftFingerprint::Ineligible(RowShiftIneligible::StaleHistory),
            "a same-range but unproven tree must not be rotated"
        );

        // The never-painted default is Stale for the same reason, and is
        // refused before its (default) range is consulted.
        let fresh = PaneBuffers::default();
        assert_eq!(
            fresh.fingerprint.build_row_shift_candidate(
                RCRange::default(),
                new_range,
                Axis::Row,
                &strip,
                strip_range,
            ),
            RowShiftFingerprint::Ineligible(RowShiftIneligible::StaleHistory),
            "an unpainted tree is never rotatable"
        );
    }

    // Acceptance 4, column axis: rotation is row-only by design. A horizontal
    // shift changes which columns each row spans, so no row digest survives.
    #[test]
    fn stage6_column_axis_request_returns_no_update() {
        let prev_range = RCRange {
            r1: 1,
            c1: 1,
            r2: 10,
            c2: 4,
        };
        let new_range = RCRange {
            r1: 1,
            c1: 3,
            r2: 10,
            c2: 6,
        };
        let strip_range = RCRange {
            r1: 1,
            c1: 5,
            r2: 10,
            c2: 6,
        };

        let pane = pane_with_exact_history(&plain_buffers(prev_range), prev_range);
        let strip = strip_from(plain_buffers(strip_range));

        assert_eq!(
            pane.fingerprint.build_row_shift_candidate(
                prev_range,
                new_range,
                Axis::Column,
                &strip,
                strip_range,
            ),
            RowShiftFingerprint::Ineligible(RowShiftIneligible::ColumnAxis),
            "a column-axis shift must return the explicit no-update result"
        );
    }

    // Acceptance 4, incompatible shapes. The column-bounds and row-extent
    // rejections deliberately mirror `shift_is_safe`'s own discipline: a
    // candidate must not be derivable for a shape the buffer rotation itself
    // would refuse.
    #[test]
    fn stage6_incompatible_row_shapes_return_a_named_no_update() {
        let prev_range = rows_1_to(1, 10);
        let pane = pane_with_exact_history(&plain_buffers(prev_range), prev_range);

        let reject = |prev: RCRange, new: RCRange, strip_range: RCRange| {
            let strip = strip_from(plain_buffers(strip_range));
            pane.fingerprint
                .build_row_shift_candidate(prev, new, Axis::Row, &strip, strip_range)
        };

        // The painted tree describes a different range than the shift claims.
        assert_eq!(
            reject(rows_1_to(2, 11), rows_1_to(5, 14), rows_1_to(11, 14)),
            RowShiftFingerprint::Ineligible(RowShiftIneligible::PriorRangeMismatch)
        );

        // Orthogonal axis moved.
        assert_eq!(
            reject(
                prev_range,
                RCRange {
                    r1: 4,
                    c1: 2,
                    r2: 13,
                    c2: 5,
                },
                RCRange {
                    r1: 10,
                    c1: 2,
                    r2: 13,
                    c2: 5,
                },
            ),
            RowShiftFingerprint::Ineligible(RowShiftIneligible::ColumnBounds)
        );

        // Scroll-axis extent changed by one — the partially-visible edge row
        // case `PaneShiftPrep::IncompatibleRange` already falls back on.
        assert_eq!(
            reject(prev_range, rows_1_to(4, 14), rows_1_to(10, 14)),
            RowShiftFingerprint::Ineligible(RowShiftIneligible::RowExtent)
        );

        // Equal extent, but the ranges share no model row: nothing to rotate.
        assert_eq!(
            reject(prev_range, rows_1_to(21, 30), rows_1_to(21, 30)),
            RowShiftFingerprint::Ineligible(RowShiftIneligible::EmptyOverlap)
        );
    }

    // Acceptance 4, incomplete strip: a strip that cannot supply every row the
    // overlap doesn't is not a rotation input, and neither is one that is not
    // dense over the range it claims.
    #[test]
    fn stage6_incomplete_strip_coverage_returns_no_update() {
        let prev_range = rows_1_to(1, 10);
        let new_range = rows_1_to(4, 13);
        let pane = pane_with_exact_history(&plain_buffers(prev_range), prev_range);

        let candidate = |strip: &FetchedCells, strip_range: RCRange| {
            pane.fingerprint.build_row_shift_candidate(
                prev_range,
                new_range,
                Axis::Row,
                strip,
                strip_range,
            )
        };

        // Rows 11..=13 are revealed; a strip starting at 12 leaves row 11
        // described by neither source.
        let short_range = rows_1_to(12, 13);
        assert_eq!(
            candidate(&strip_from(plain_buffers(short_range)), short_range),
            RowShiftFingerprint::Ineligible(RowShiftIneligible::IncompleteStrip),
            "a strip that skips a revealed row must be refused"
        );

        // A strip narrower than the pane cannot be read as a full-width dense
        // row buffer.
        let narrow_range = RCRange {
            r1: 10,
            c1: 1,
            r2: 13,
            c2: 3,
        };
        assert_eq!(
            candidate(&strip_from(plain_buffers(narrow_range)), narrow_range),
            RowShiftFingerprint::Ineligible(RowShiftIneligible::IncompleteStrip),
            "a strip narrower than the pane must be refused"
        );

        // A channel that is not dense over `strip_range` — a fetch that was
        // never completed — must be refused rather than indexed into.
        let strip_range = rows_1_to(10, 13);
        let (styles, mut values, cell_types, decorations) = plain_buffers(strip_range);
        values.truncate(values.len() - 1);
        assert_eq!(
            candidate(
                &FetchedCells::from_parts(styles, values, cell_types, decorations),
                strip_range,
            ),
            RowShiftFingerprint::Ineligible(RowShiftIneligible::IncompleteStrip),
            "a short channel must be refused, not indexed past"
        );
    }

    // Acceptance 5: building a candidate is side-effect-free with respect to
    // everything semantic. The painted tree still answers comparisons the same
    // way afterwards, and the truth state is still `Exact` — proven by the
    // fact that a second, identical request is still eligible.
    #[test]
    fn stage6_candidate_building_leaves_painted_and_truth_untouched() {
        let prev_range = rows_1_to(1, 10);
        let new_range = rows_1_to(4, 13);
        let strip_range = rows_1_to(10, 13);

        let prev_buffers = plain_buffers(prev_range);
        let pane = pane_with_exact_history(&prev_buffers, prev_range);
        let strip = strip_from(plain_buffers(strip_range));

        let first = pane.fingerprint.build_row_shift_candidate(
            prev_range,
            new_range,
            Axis::Row,
            &strip,
            strip_range,
        );

        assert_eq!(
            pane.fingerprint
                .compare_to_painted(&build(&prev_buffers, prev_range)),
            RepaintPlan::Skip,
            "the painted tree must still describe the pane it did before"
        );
        assert_eq!(
            pane.fingerprint.build_row_shift_candidate(
                prev_range,
                new_range,
                Axis::Row,
                &strip,
                strip_range,
            ),
            first,
            "truth must still be Exact, and the same inputs must still rotate"
        );

        // A rejected rotation must not disturb the scratch slot either: the
        // next valid request still produces the identical candidate.
        let column_shifted = RCRange {
            r1: 4,
            c1: 2,
            r2: 13,
            c2: 5,
        };
        let _ = pane.fingerprint.build_row_shift_candidate(
            prev_range,
            column_shifted,
            Axis::Row,
            &strip,
            strip_range,
        );
        assert_eq!(
            pane.fingerprint.build_row_shift_candidate(
                prev_range,
                new_range,
                Axis::Row,
                &strip,
                strip_range,
            ),
            first,
            "a rejected rotation must leave the scratch slot usable"
        );
    }
}
