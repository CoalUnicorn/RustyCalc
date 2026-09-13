# FSM-A7 review

The review covers named blit and replay outcomes, autofit errors, export
errors, and RustyCalc consumers. It follows FSM-A7 in the consistency index.

## Findings and fixes

- P1: Autofit called `.value()` on formatted values and styles. This erased
  BridgeFailed. A failed read could produce Ok(None), a default-font fit,
  or a partial maximum. Both axes now return CellValue or CellStyle errors.
  A failed later style read discards the earlier measurement.
- P2: `capped_range` left inverted spans empty, but `span.cells()` normalized
  them again. This scanned cells outside the intended range and could bypass
  the scan cap. Both axes now iterate the capped inclusive interval directly.
- P2: `exit_playback` replaced CanvasMode before checking its variant.
  Calling it during capture dropped the recording. Check for Playback first.
  A browser regression verifies that capture can still be stopped normally.
- Documentation: fixed new unresolved links and links to private outcome
  types. Clarified that ValidatedRecording protects recording sessions;
  the low-level replay API still accepts raw operations.

## Contract checks

- PreparedBlitOutcome preserves by-value ownership for both Ready and
  FreshFallback. The prepare, commit, rollback, and held-retry paths retain
  their existing transaction order. No new allocation was added.
- ReplayResult maps both ReplayOutcome variants exhaustively. A browser
  test seeks back into a held prefix and checks NoCommittedFrame, the current
  frame index, and unchanged grid and overlay bytes.
- ExportError separates invalid metrics and failed paint attempts from a
  document. Tests cover invalid input before model reads, scalar and bulk
  failures, and success after recovery. Browser tests cover facade errors
  for missing models and failed sheet/value/style reads.
- Autofit callers apply only successful measurements. Camera autosize,
  automatic row fitting, and double-click handlers preserve stored extents
  after an error. The JS facade distinguishes errors from an empty range.
- FrameInputs was sealed in A6. This commit removes the unused px_demo
  file. No source file references that module.

## Validation

The final isolated staged snapshot passed:

- 631 iron-canvas native tests with all features;
- 155 RustyCalc native tests with all features;
- 56 Chrome tests across the spreadsheet and datagrid facades;
- strict all-target Clippy with all features for iron-canvas;
- the iron-canvas default-feature workspace check.

Five browser timing probes were ignored. The browser tests include actual
Canvas2D pixel comparisons. No manual RustyCalc UI smoke test was run.
Root strict Clippy and the final app Wasm check passed with all features.
Formatting and diff checks passed. Documentation generation succeeded. Seven
existing private-link and redundant-link warnings remain in core documentation.

The snapshot uses the existing IronCalc checkout. Its tracked source is
unchanged. The prior fractional-DPR header repaint issue remains recorded in
SESSION.md. No remote commit or push was requested.

## Related reviews

- [FSM-A5](2026-09-12-fsm-a5-review.md)
- [FSM-A6](2026-09-13-fsm-a6-review.md)
