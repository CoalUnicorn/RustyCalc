# Excel Feature Development Discussion (May 25, 2026)

## Context

Discussed with Claude how to structure our approach for building
full Excel-like features (data validation, conditional formatting,
charts, sparklines, autofilter, hyperlinks, comments, form controls).

## Key insights

### Five-layer pipeline
Parse → Store → React → Paint → Interact. A feature is only "done"
when all five layers work. Missing one breaks the feature.

### Engine vs Sidecar
IronCalc owns the formula/value substrate. Everything that sits on
top (charts, comments, drawings) is a RustyCalc sidecar — separate
data structure that lives alongside UserModel, survives round-trips,
and diffs alongside engine diffs.

Query: "does this feature affect what formulas compute?"
- YES → engine (upstream IronCalc PR)
- NO → sidecar (this repo)

### OOXML resources
- ECMA-376 Part 1 §18 (SpreadsheetML) — free download
- Open XML SDK docs (Microsoft) — schema diagrams
- Existing ironcalc xlsx/src/import/ — shows gaps
- mc:AlternateContent for preserving unsupported features

### Skill structure
One umbrella `excel-features` skill with per-feature references.
Don't split per-feature — shared pipeline means duplication risk.

### DOM-over-canvas
iron-canvas for grid cells (60fps paint). DOM overlays positioned
via cellRect() for interactive controls (dropdowns, comment bubbles,
chart canvases). Accessibility requires real DOM.

## What we built
- `docs/guides/developing-excel-features.md` — full guide: pipeline,
  decision tree, OOXML survival kit (minimal XML per feature,
  part paths, rels wiring), DOM-over-canvas pattern, round-trip
  checklist, worked example (data validation dropdown)
- `.claude/skills/excel-features/SKILL.md` — Claude skill scaffold
- Updated `rustycalc-knowledge` Hermes skill with 4 new references
  from ironcalc-patterns (types, mutate, diffs, formula-refs)
