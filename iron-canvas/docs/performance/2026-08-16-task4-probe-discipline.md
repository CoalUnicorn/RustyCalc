# Task 4 Probe Discipline

Every Task 4 cost probe records one row with these fields, in this order:

1. **Edit:** exact address (e.g. `B3`) and old/new value class (empty ->
   text, text -> text, formula recalc, style-only).
2. **View state:** top row, left column, frozen rows/cols, DPR, canvas
   CSS size.
3. **One-line trace:** `frameTrace()` of the timed frame.
4. **Snapshot:** `frameDiagnostics()` from a representative run —
   probe attribution, segments, fetch batches/addressed cells/logical
   slots, verdict + reason, painted rows/cells, cache resolution, blit
   detail when applicable.
5. **Host wall time:** `Draw` (Perf panel) or the probe's own
   `performance.now()` bracket around `paintIfDirty`.
6. **Capture flag:** whether `setFrameDiagnosticsEnabled(true)` was active
   during the timed samples. Timing samples must run with capture OFF;
   enable it only for the representative attribution run.

Rules:

- A `grid:skip` run is never reported as the cost of painting a quadrant.
  Only a snapshot proving the intended visible, paint-relevant change
  reached that segment (probe inside exactly that segment, verdict
  `rows`/`FULL` with `changedRows` or a named promotion reason) may be
  costed as a quadrant repaint.
- Never describe a `FULL` promotion without its reason
  (`spanCapExceeded`, `borderSafety`, `layoutMismatch`,
  `rowAddressMismatch`); a rebuild `FULL` has no fingerprint reason —
  quote its `rebuildReason` instead.
- Retained-pixel scenarios still gate on raw Canvas2D `ImageData`
  equality against independent forced-Fresh output
  (`stage6_assert_matches_forced_fresh`); the snapshot explains, the
  raster comparison proves.

Example row (B3 freeze toggle, illustrative numbers):

| Edit | View | Trace | Segments | Fetch | Verdict | Wall | Capture |
| --- | --- | --- | --- | --- | --- | --- | --- |
| freeze on @ B3 | 1,1 2x1 2.0 1600x900 | `Fresh[GEOMETRY\|OVERLAY] grid:FULL fetched=1856` | 4 segs, 464 cells | 4 batches / 464 / 1856 | FULL, reason null, `rebuildReason: freeze` | 3.1 ms | off |
