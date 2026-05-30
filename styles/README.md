# RustyCalc CSS System

## Architecture

Each UI component gets its own CSS file with a short prefix. Files are grouped
by role into subfolders. All files are imported via `styles/index.css`.

```
styles/
  index.css            entry point (@import all)
  layout.css           #app, .workbook
  chrome/              persistent app frame
    file-bar.css       fl-
    toolbar.css        tb-
    formula-bar.css    fb-
    sheet-tabs.css     tab-
    left-drawer.css    ld-
    status-bar.css     status-bar / status-bar-error
  worksheet/           the grid surface
    worksheet.css      ws-
    cell-editor.css    ce-
    formula-overlay.css  fe-
  ui/                  reusable primitives
    color-picker.css   cp-
    context-menu.css   ctx-
    modal.css          md-
  panels/              feature surfaces
    perf-panel.css     pp-
    playback-panel.css pb-
    named-ranges.css   nrm-
    share-popover.css  sp-
```

## Naming Convention

**Prefix** — 2-3 character component identifier. The root element uses the bare
prefix as its class (`.tb`, `.cp`, `.ld`).

**Children** — `{prefix}-{element}`:  `.tb-btn`, `.cp-swatch`, `.ld-entry`

**States** — appended as a plain class: `.tb-btn.active`, `.ld-entry.active`,
`.tab.selected`

**Scoping** — all child rules nest under the root to prevent collision:
```css
.tb .tb-btn { ... }      /* only matches inside .tb */
```

## Prefix Table

| Prefix | Component       | Root class    | File                       |
|--------|-----------------|---------------|----------------------------|
| —      | Layout          | `#app`        | layout.css                 |
| `fl-`  | File Bar        | `.fl`         | chrome/file-bar.css        |
| `tb-`  | Toolbar         | `.tb`         | chrome/toolbar.css         |
| `fb-`  | Formula Bar     | `.fb`         | chrome/formula-bar.css     |
| `tab-` | Sheet Tabs      | `.tab-bar`    | chrome/sheet-tabs.css      |
| `ld-`  | Left Drawer     | `.ld`         | chrome/left-drawer.css     |
| —      | Status Bar      | `.status-bar` | chrome/status-bar.css      |
| `ws-`  | Worksheet       | `.ws`         | worksheet/worksheet.css    |
| `ce-`  | Cell Editor     | `.ce`         | worksheet/cell-editor.css  |
| `fe-`  | Formula Overlay | `.fe-host`    | worksheet/formula-overlay.css |
| `cp-`  | Color Picker    | `.cp`         | ui/color-picker.css        |
| `ctx-` | Context Menu    | `.ctx`        | ui/context-menu.css        |
| `md-`  | Modal Dialog    | `.md-box`     | ui/modal.css               |
| `pp-`  | Perf Panel      | `.pp`         | panels/perf-panel.css      |
| `pb-`  | Playback Panel  | `.pb`         | panels/playback-panel.css  |
| `nrm-` | Named Ranges    | `.nrm`        | panels/named-ranges.css    |
| `sp-`  | Share Popover   | `.sp-popover` | panels/share-popover.css   |

## Cross-Component Usage

Some elements compose classes from multiple components:

- Color picker trigger in toolbar: `class="tb-btn cp-trigger"`
- Color picker trigger in context menu: `class="ctx-item cp-trigger"`
- Context menu used by sheet tabs and header right-click (shared `ctx-` classes)

## CSS Variables

Theme variables are defined in `index.html` on `:root` and `[data-theme="dark"]`:

```
--bg-primary, --bg-secondary     backgrounds
--bg-hover, --bg-hover-strong, --bg-active   interactive states
--border-color, --border-inner   borders
--text-primary, --text-dim, --text-strong, --text-secondary, --text-placeholder   text
--accent, --accent-hover         interactive highlight
--btn-bg                         button backgrounds
--cell-editor-bg                 editor overlay
--font-mono                      monospace stack (formulas, ranges)
--danger / --error               error red (--error aliases --danger)
--danger-text, --danger-bg, --danger-border   error banners
--warning                        caution amber (unsaved / dirty cues)
```

Prefer a variable over a raw color. When a variable might be absent, keep an
inline fallback — `var(--danger, #cf222e)` — so the rule degrades gracefully.

## Adding a New Component

1. Pick a 2-3 char prefix (check table above for conflicts)
2. Create the file under the right role folder: `styles/{chrome|worksheet|ui|panels}/{component}.css`
3. Add `@import "{folder}/{component}.css";` to `index.css` (in that folder's group)
4. Use `.{prefix}` as root class, `.{prefix}-{element}` for children
5. Scope all rules under the root: `.xx .xx-child { ... }`

## Build Options

See `build_options.md` for production bundling/minification strategies.
