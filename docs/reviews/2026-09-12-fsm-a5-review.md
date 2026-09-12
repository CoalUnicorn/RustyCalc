# FSM-A5 review

The review covers the tracked RowSpan, DenseRange, range projection, and
GridShape changes. The implementation follows FSM-A5 in the iron-canvas
consistency index. RCRange remains permissive. RowSpanWire retains r1/r2.

## Findings and fixes

- P2: `ContentWork::normalize_rows` used `last.end + 1`. Merging a span
  ending at `i32::MAX` could panic or break idempotence. Use saturating
  addition. A regression merges a full-address span with itself.
- P2: DenseRange cast a count of 2^32 to usize. This becomes zero on
  Wasm32. Use checked conversion with saturation. A regression covers
  both full-address axes and the saturated area.

## Checks

- Private RowSpan fields preserve endpoint order at every caller.
- Dense fetching passes normalized corners to all four model channels.
  A regression checks the channel lengths and row-major values.
- Projection covers frozen-only, gap-starting, and gap-only ranges.
  Added column-axis and reversed-corner coverage.
- GridShape derives frozen counts from dense slot lengths. GridLayout
  uses the canonical PaneRegion index.
- No model reads or persistent cache writes were added to paint execution.

## Validation

The staged snapshot passed 612 native workspace tests with dev-tools,
strict all-target Clippy, and a Wasm workspace check with dev-tools.
The snapshot uses the existing IronCalc checkout. Its tracked files are
unchanged. The Chrome render suite passed 45 tests; five timing probes
were ignored. The browser runner required permission to start its local
server outside the sandbox.

The prior fractional-DPR header repaint issue remains in SESSION.md.
This review does not claim to fix that issue.
