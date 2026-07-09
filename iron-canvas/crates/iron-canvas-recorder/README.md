# iron-canvas-recorder

A recording `Painter` backend — every draw call captured as data for assertion and replay.

## What it does

Records each painter operation as a `DrawOp` enum variant instead of rasterizing it. Tests assert against the resulting op stream ("did we stroke this border once, in this color?") without a browser or pixel buffer. `RecordingPainter<P>` forks ops to a recorder while still delegating to a real painter, so you can record a live render in flight.

## Crate role

The test and dev-tooling backend. `RecorderPainter` and `MemSurface` implement `iron-canvas-core`'s `Painter`/`Surface` traits in memory, giving integration tests a deterministic target. `RecordingPainter<P>` and `RecordingSurface<S>` decorate a live backend for capture. Used by `core` as a dev-dependency and optionally by `web` under its `dev-tools` feature.

## Key exports

- `DrawOp` — enum of ~20 recorded operations (fills, strokes, text, clips, layers)
- `RecorderPainter` — `Painter` impl that records instead of drawing
- `MemSurface` — in-memory `Surface` for integration tests
- `RecordingPainter<P>` — wraps another painter, forking ops to a recorder
- `RecordingSurface<S>` — the opt-in dev-tool decorator over a live `Surface`, serializing sessions to the `.icr` format (wired behind `iron-canvas-web`'s `dev-tools` feature)
- `RecordingFilter`, `LayerScope` — narrow recording to specific layers/ops
- `replay()` — re-drive a recorded op stream through any `BlitPainter`

## Dependencies

- `iron-canvas-core` — the traits it records against
- `serde`, `serde_json` — serialize draw operations and `.icr` recordings

## Relationship to sibling crates

Mirror image of `canvas2d`/`export`: where those turn paint calls into pixels or SVG/PDF, recorder turns them into inspectable data. `replay()` closes the loop — a recording can be re-emitted through `canvas2d` or `export` to verify equivalence across backends.
