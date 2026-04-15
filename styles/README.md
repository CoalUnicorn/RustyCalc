# RustyCalc CSS System

## Architecture

Each UI component gets its own CSS file with a short prefix. All files are imported
via `styles/index.css`.

```
styles/
  index.css          entry point (@import all)
  layout.css         #app, .workbook
  file-bar.css       fl-
  toolbar.css        tb-
  formula-bar.css    fb-
  color-picker.css   cp-
  worksheet.css      ws-
  cell-editor.css    ce-
  sheet-tabs.css     tab-
  context-menu.css   ctx-
  perf-panel.css     pp-
  left-drawer.css    ld-
  status_bar.css     status-bar / status-bar-error
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

| Prefix | Component     | Root class | File            |
|--------|---------------|------------|-----------------|
| —      | Layout        | `#app`     | layout.css      |
| `fl-`  | File Bar      | `.fl`      | file-bar.css    |
| `tb-`  | Toolbar       | `.tb`      | toolbar.css     |
| `fb-`  | Formula Bar   | `.fb`      | formula-bar.css |
| `cp-`  | Color Picker  | `.cp`      | color-picker.css|
| `ws-`  | Worksheet     | `.ws`      | worksheet.css   |
| `ce-`  | Cell Editor   | `.ce`      | cell-editor.css |
| `tab-` | Sheet Tabs    | `.tab-bar` | sheet-tabs.css  |
| `ctx-` | Context Menu  | `.ctx`     | context-menu.css|
| `pp-`  | Perf Panel    | `.pp`      | perf-panel.css  |
| `ld-`  | Left Drawer   | `.ld`      | left-drawer.css |
| —      | Status Bar    | `.status-bar` | status_bar.css |

## Cross-Component Usage

Some elements compose classes from multiple components:

- Color picker trigger in toolbar: `class="tb-btn cp-trigger"`
- Color picker trigger in context menu: `class="ctx-item cp-trigger"`
- Context menu used by sheet tabs and header right-click (shared `ctx-` classes)

## CSS Variables

Theme variables are defined in `index.html` on `:root` and `[data-theme="dark"]`:

```
--bg-primary, --bg-secondary     backgrounds
--border-color, --border-inner   borders
--text-primary, --text-dim, --text-strong   text
--accent                         interactive highlight
--btn-bg                         button backgrounds
--cell-editor-bg                 editor overlay
```

## Adding a New Component

1. Pick a 2-3 char prefix (check table above for conflicts)
2. Create `styles/{component}.css`
3. Add `@import "{component}.css";` to `index.css`
4. Use `.{prefix}` as root class, `.{prefix}-{element}` for children
5. Scope all rules under the root: `.xx .xx-child { ... }`

## Build Options

See `build_options.md` for production bundling/minification strategies.
