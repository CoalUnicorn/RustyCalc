# Conditional Formatting Research (May 25, 2026)

## IronCalc status

IronCalc has a complete conditional formatting subsystem:
- `cf_types.rs` (493 lines): CfRule (16 variants), CfRuleInput, operators, thresholds, icons, data bars, ExtendedStyle
- `conditional_formatting.rs` (1475 lines): evaluation engine, cf_cache, color interpolation, stop_if_true priority
- UserModel API: get_list, get_dxf, get_extended_style_for_cell, add/delete/update (with Diff for undo)
- xlsx import/export with round-trip test
- WASM bridge exposed to JS

ExtendedStyle correctly splits base style (dxf-applied font/fill/border) from decorations (icons, data bars, ratings).

## Other apps research (with Claude)

**LibreOffice Calc**: ScConditionalFormatList + cell→rule index for O(1) lookup. ScPatternAttr is shared format flyweight; CF results overlay it, never mutate it.

**Handsontable**: CF as plugin with beforeRenderer hook. Decoration overlay handled by separate renderer in chain — not merged into base style.

**OnlyOffice**: Eagerly merged into one per-cell style cache. Avoid — collapses dxf priority and forces full rebuild on any rule edit.

**Apache POI**: OOXML mirror, dxf indirection identical to IronCalc.

**Steal**: LibreOffice's cell→rule index (IronCalc already has cf_cache), Handsontable's decoration overlay pattern.
**Avoid**: OnlyOffice's eager merge.

## Architecture decisions

1. Don't invent parallel RustyCalc CF layer — IronCalc owns evaluation
2. Don't bake CF into base Style — data_bar/icon/rating have no home there
3. Follow decoration overlay pattern: paint_cf_decoration after paint_cell
4. Use per-cell resolved ExtendedStyle (one call, pre-resolved)
5. Bump cf_version signal for reactivity (whole-viewport rebuild, slots reused)

## Plan written

docs/plans/2026-05-25-conditional-formatting-integration.md — 7-step implementation order: CanvasModel extension → CfDecorationPaint type → CellPaint extension → Painter trait → cf_version signal → render integration → UI panel.
