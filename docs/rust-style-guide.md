# RustyCalc Rust Style Guide

This document describes the Rust design patterns and conventions used throughout RustyCalc. These patterns keep the codebase consistent and use Rust's type system to prevent bugs at compile time.

## Core Principles

### 1. Model the Domain in Types

Use newtypes for domain values instead of bare strings or primitives:

```rust
// Better: Domain meaning encoded in type
pub struct CssColor(String);
impl CssColor {
    pub fn new(s: impl Into<String>) -> Self { /* validate */ }
}

// Problematic: Bare strings lose domain knowledge
fn set_color(color: String) { /* can't distinguish from other strings */ }
```

Use enums for closed sets of values. Here's how RustyCalc handles font families:

```rust
/// Font families the browser can reliably render.
/// Unknown font names from Excel files map to `SystemUi`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SafeFontFamily {
    Arial,
    /// Renders as `"Calibri, system-ui"`. On Linux/Android, `system-ui` activates.
    CalibriLike,
    CourierNew,
    Georgia,
    TimesNewRoman,
    Verdana,
    /// Fallback for any unrecognised font name.
    SystemUi,
}

// Graceful handling of unknown inputs
impl From<Option<&str>> for SafeFontFamily {
    fn from(name: Option<&str>) -> Self {
        match name {
            Some("Arial") => Self::Arial,
            Some("Calibri") => Self::CalibriLike,
            Some("Courier New") => Self::CourierNew,
            Some("Georgia") => Self::Georgia,
            Some("Times New Roman") => Self::TimesNewRoman,
            Some("Verdana") => Self::Verdana,
            _ => Self::SystemUi,  // Unknown fonts safely fallback
        }
    }
}
```

This approach gives you:
- Only valid fonts can be passed around (compile-time safety)
- Adding a new font breaks the build until handled everywhere (exhaustive matching)
- Unknown fonts get mapped to a safe fallback (graceful degradation) 
- Same enum works for CSS, model storage, and UI display (multiple contexts)
- Zero-cost abstractions that optimize to integers (performance)

Compare with a string-based approach where typos like "Ariell" compile but render wrong, and arbitrary fonts like "Comic Sans" pass validation.

### 2. Use Enums for State Machines

Replace correlated booleans with state enums to make illegal states impossible:

```rust
// Better: Single enum prevents impossible combinations
pub enum DragState {
    Idle,
    Selecting,
    Extending { to_row: i32, to_col: i32 },
    /// Formula point-mode: range the user is actively selecting, plus the
    /// text span it occupies in the formula bar so it can be spliced in place.
    Pointing { range: SheetRect, ref_span: (usize, usize) },
    ResizingCol { col: i32, x: f64 },
}

// Problematic: Multiple booleans allow impossible states
struct BadState {
    selecting: bool,
    extending: bool, 
    resizing: bool,  // all three could be true simultaneously!
}
```

### 3. Parse Don't Validate

Parse input once at boundaries, then use domain types internally:

```rust
// Better: Validation at construction ensures invariants
impl CssColor {
    pub fn new(s: impl Into<String>) -> Self {
        let s = s.into();
        if s.is_empty() { 
            Self("#000000".to_owned()) 
        } else { 
            Self(s) 
        }
    }
}

// Problematic: Validation functions that discard proof
fn validate_color(s: &str) -> Result<(), Error> { /* caller forgets result */ }
```

### 4. Exhaustive Matching

List every variant for enums you control:

```rust
// Better: Adding variants breaks the build
match edit_mode {
    EditMode::Accept => { /* handle */ }
    EditMode::Edit => { /* handle */ }
}

// Problematic: Wildcards hide missing cases
match edit_mode {
    EditMode::Accept => { /* handle */ }
    _ => {} // new variants slip through silently
}
```

### 5. Error Handling

Input modules use per-domain errors derived with `thiserror`. Each module has
one error type; `execute()` maps them all to `String` at the dispatch point so
there's a single log line per user action.

```rust
// error.rs - one type per input module
#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("format: {0}")]
    Engine(String),
}

// In execute():
let result: Result<(), String> = match action {
    Format(a) => execute_format(a, model, state).map_err(|e| e.to_string()),
    Nav(a)    => execute_nav(a, model, state).map_err(|e| e.to_string()),
    // ...
};
if let Err(msg) = result {
    web_sys::console::warn_1(&format!("[RustyCalc] {msg}").into());
}
```

For browser DOM calls that should never fail in a real browser context (window,
document body, media queries), use an inner `Result`-returning function with a
`thiserror` error type and fall back gracefully at the public boundary:

```rust
#[derive(Debug, thiserror::Error)]
enum DarkModeQueryError {
    #[error("window not available")]
    NoWindow,
}

fn query_prefers_dark() -> Result<bool, DarkModeQueryError> {
    let window = web_sys::window().ok_or(DarkModeQueryError::NoWindow)?;
    // ...
}

pub fn system_prefers_dark() -> bool {
    query_prefers_dark().unwrap_or(false)
}
```

For UI event handlers where the return type is `()`, use a typed inner function
and log on error rather than panicking:

```rust
fn extract_file_input(ev: &web_sys::Event) -> Result<..., FileChangeError> { ... }

let on_file_change = move |ev| {
    let (input, file) = match extract_file_input(&ev) {
        Ok(result) => result,
        Err(e) => { web_sys::console::warn_1(...); return; }
    };
    // ...
};
```

### 6. Ownership and Borrowing

Borrow by default, own only when you need to store or transform:

```rust
// Borrow when you only read
fn process_text(text: &str) -> String { /* ... */ }

// Own when storing or transforming
fn store_name(self, name: String) -> Self { 
    self.name = name; 
    self 
}
```

### 7. Collections and Structures

Group related data in a single struct instead of parallel collections:

```rust
// Better: Related data grouped together
type Registry = HashMap<String, WorkbookMeta>;

struct WorkbookMeta {
    name: String,
    // other metadata fields...
}

// Problematic: Parallel collections can drift out of sync  
struct BadRegistry {
    names: HashMap<String, String>,
    timestamps: HashMap<String, DateTime>, // same keys, different maps
}
```

### 8. Module Organization

Use modules for namespacing, not impl blocks on unit structs:

```rust
// Better: Module groups related functionality
pub mod geometry {
    pub fn cell_to_pixels(row: i32, col: i32) -> (f64, f64) { /* ... */ }
    pub fn pixels_to_cell(x: f64, y: f64) -> (i32, i32) { /* ... */ }
}

// Awkward: Unit structs as namespaces
struct Geometry;
impl Geometry {
    fn cell_to_pixels(row: i32, col: i32) -> (f64, f64) { /* ... */ }
}
```

### 9. Data-Driven Enum Implementations

For enums with multiple output formats, use a single match with structured data:

```rust
#[derive(Debug, Clone, Copy)]
struct FontNames {
    css: &'static str,
    model: &'static str,
    label: &'static str,
}

impl SafeFontFamily {
    fn names(&self) -> FontNames {
        match self {
            Self::Arial => FontNames {
                css: "Arial",
                model: "Arial",
                label: "Arial",
            },
            Self::CalibriLike => FontNames {
                css: "Calibri, system-ui",
                model: "Calibri",
                label: "Calibri",
            },
            // ... other variants in one place
        }
    }

    pub fn css_name(&self) -> &'static str { self.names().css }
    pub fn model_name(&self) -> &'static str { self.names().model }
    pub fn label(&self) -> &'static str { self.names().label }
}
```

This approach prevents the multiple-match problem where you have to update three separate functions every time you add a variant. Adding a new font family requires editing only one match block, and all output formats automatically stay in sync.

## Component Patterns

### Signal Organization

Group related signals in a state struct using `Split<T>` - a thin wrapper around
`(ReadSignal<T>, WriteSignal<T>)` that is `Copy` for any `T: Clone + Send + Sync`:

```rust
#[derive(Clone, Copy)]
pub struct WorkbookState {
    pub editing_cell: Split<Option<EditingCell>>,
    pub drag: Split<DragState>,
    // ... other related signals
}
```

Reading reactively uses `.get()`; writing uses `.set()` / `.update()`. See
`src/state.rs` and `docs/state-and-events.md` for the full API.

### Context Usage

Use typed aliases instead of raw generic storage:

```rust
// Typed alias clarifies intent
pub type ModelStore = StoredValue<UserModel<'static>, LocalStorage>;

// Usage
let model = expect_context::<ModelStore>();
```

### Error Handling in Components

Log errors but keep the UI functional, then emit the appropriate typed event:

```rust
let on_click = move |_| {
    model.update_value(|m| {
        warn_if_err(m.delete_sheet(sheet_id), "delete_sheet");
    });
    state.emit_event(SpreadsheetEvent::Structure(
        StructureEvent::WorksheetDeleted { sheet: sheet_id },
    ));
};
```

Use `state.request_redraw()` (emits `ContentEvent::GenericChange`) only when no
specific event applies - e.g. after a viewport resize or canvas-only repaint.
For model mutations, always prefer the typed event so subscribers can filter by
category.

## Examples from the Codebase

These patterns come from actual RustyCalc code:

- Domain types: `CssColor`, `SafeFontFamily`, `ActiveCell`, `FrozenPanes`
- State enums: `EditMode`, `EditFocus`, `DragState`, `ArrowKey`  
- Parse-don't-validate: `CssColor::new()`, `SafeFontFamily::from()`
- Data-driven enums: `SafeFontFamily::names()` with single match block
- Structured collections: `HashMap<String, WorkbookMeta>`
- Error utilities: `warn_if_err()`, storage error logging
- Module organization: `canvas::geometry`, `input::action`, `components::*`

Following these patterns helps catch bugs at compile time and makes the code easier to maintain.
