# FSM-A6 review

The review covers validated canvas metrics, sealed FrameInputs, recording
validation, playback loading, and their host adapters.

## Findings and fixes

- P2: Loading another recording saved the first recording's dimensions as
  the live metrics. Preserve the original session's live metrics. Exiting
  either recording now restores the original logical size and DPR.
- P2: Recording validation and replay selected anchors from the requested
  strategy. A ScrollBlit fallback can execute FullRebuild. Both paths now
  call Frame::is_replay_anchor. It requires a painted result, a painted
  outcome, an effective FullRebuild, and a committed sequence.
- P2: Direct TryFrom<Recording> skipped the schema-version check.
  Validation and deserialization now use the same schema check.

## Checks

CanvasMetrics validates finite, non-negative logical extents, positive finite
DPR, and u32 backing dimensions. Resize parses before it mutates the runtime.
Surfaces, FrameInputs, and Chrome receive the parsed value. No unchecked public
constructor was added. Recording validation precedes resize, CSS writes, and
session replacement. Tests cover schema construction, fallback anchors, held
anchors, invalid numeric values, brackets, timestamps, and dense channels.

The commit includes export error propagation and its consumers because export
surfaces must adopt the validated metrics contract. The remaining named blit,
autofit, and replay-result changes belong to the FSM-A7 commit.

## Validation

The isolated staged snapshot passed 624 native tests with all features,
strict all-target Clippy with all features, and the RustyCalc Wasm app check
with dev-tools and export. The three Chrome playback tests passed. These
include raw pixel comparisons after rejected input and restoration after
recording replacement. The Chrome render suite passed 45 tests; five timing
probes were ignored. Formatting and staged diff checks passed.

The snapshot uses the existing IronCalc checkout. Its tracked files are
unchanged. No manual RustyCalc UI smoke test was run for this slice.
