use gloo_storage::Storage as GlooStorage;
use ironcalc_base::UserModel;
use leptos::prelude::*;

// NOTE: <Meta name="color-scheme" content="dark"/>
use crate::perf::PerfTimings;
use crate::theme::Theme;

/// Shorthand for the context-provided `UserModel` storage handle.
///
/// `StoredValue<UserModel<'static>, LocalStorage>` is the Leptos arena-stored
/// wrapper used throughout the app.  This alias eliminates the repetition in
/// every `use_context` call.
pub type ModelStore = StoredValue<UserModel<'static>, LocalStorage>;

/// In-progress cell edit not yet committed to the model.
/// Mirrors the TypeScript `EditingCell` interface in workbookState.ts.
#[derive(Clone, Debug, PartialEq)]
pub struct EditingCell {
    pub(crate) sheet: u32,
    pub(crate) row: i32,
    pub(crate) col: i32,
    /// Text the user has typed; NOT yet written to UserModel
    pub(crate) text: String,
    /// "accept" = arrow keys navigate; "edit" = arrow keys move cursor within text
    pub(crate) mode: EditMode,
    /// Which widget currently has keyboard focus
    pub(crate) focus: EditFocus,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EditMode {
    /// Arrow keys commit the edit and navigate to adjacent cells
    Accept,
    /// Arrow keys move the text cursor; entered via F2 or double-click
    Edit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EditFocus {
    Cell,
    FormulaBar,
}

/// Which header was right-clicked to open the context menu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ContextMenuTarget {
    /// Column index (1-based).
    Column(i32),
    /// Row index (1-based).
    Row(i32),
}

/// Active mouse-drag interaction.
///
/// At most one drag mode can be active at a time.  Using a single enum
/// instead of parallel `bool` / `Option` signals makes illegal combinations
/// (e.g. selecting while resizing) unrepresentable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DragState {
    /// No drag in progress.
    Idle,
    /// Mouse button held for a range-drag selection.
    Selecting,
    /// Autofill handle drag: the cell the user is dragging toward.
    Extending { to_row: i32, to_col: i32 },
    /// Column header resize: `(col_1based, current_mouse_x)`.
    ResizingCol { col: i32, x: f64 },
    /// Row header resize: `(row_1based, current_mouse_y)`.
    ResizingRow { row: i32, y: f64 },
    /// Dragging to extend the point-mode range during formula entry.
    Pointing,
}

/// Position and target for the active context menu.
#[derive(Clone, Copy, Debug)]
pub struct ContextMenuState {
    /// Viewport-relative x (from `ev.client_x()`).
    pub(crate) x: i32,
    /// Viewport-relative y (from `ev.client_y()`).
    pub(crate) y: i32,
    /// Which header triggered the menu.
    pub(crate) target: ContextMenuTarget,
}

/// All transient UI state — never persisted, never in the model.
///
/// All primitive fields use `RwSignal` (Send+Sync+Clone) so Leptos re-runs
/// only the closures that read a specific field.
///
/// The model itself is NOT stored here — it lives in a `StoredValue::new_local`
/// in `App` and is accessed via `use_context::<StoredValue<UserModel<'static>, LocalStorage>>()`.
/// A `RwSignal<u32>` redraw counter (also in context) triggers canvas re-draws.
///
/// Note: row/col/sheet are NOT stored here — they are always derived from
/// `UserModel::get_selected_view()` inside a `{ let _ = state.redraw.get(); … }` closure
/// to stay in sync with the model's navigation state.
#[derive(Clone, Copy)]
pub struct WorkbookState {
    /// None = not editing; Some = live edit buffer
    pub(crate) editing_cell: RwSignal<Option<EditingCell>>,
    /// Active mouse-drag interaction (selection, resize, autofill, point-mode).
    pub(crate) drag: RwSignal<DragState>,
    /// Increment after any model mutation to force canvas re-renders.
    /// Components that draw the canvas subscribe to this signal.
    pub(crate) redraw: RwSignal<u32>,
    /// UUID of the workbook currently loaded in the model.
    /// Used by `storage::save` to write back to the correct localStorage key.
    /// `None` during the brief window before a workbook is loaded.
    pub(crate) current_uuid: RwSignal<Option<String>>,
    /// Active color theme; initialized from localStorage on startup.
    pub(crate) theme: RwSignal<Theme>,
    /// Whether the left workbook-list drawer is open.
    pub(crate) is_drawer_open: RwSignal<bool>,
    /// Controls the Upload xlsx file dialog.
    pub(crate) show_upload_dialog: RwSignal<bool>,
    /// Controls the Share dialog; holds the URL returned by the server.
    pub(crate) share_url: RwSignal<Option<String>>,
    /// BCP-47 language tag for formula language, persisted in localStorage.
    pub(crate) current_lang: RwSignal<String>,
    /// Whether the Regional Settings right panel is open.
    pub(crate) show_regional_settings: RwSignal<bool>,
    /// Whether the Named Ranges right panel is open.
    pub(crate) show_named_ranges: RwSignal<bool>,
    /// Whether the Performance panel is visible.
    pub(crate) show_perf_panel: RwSignal<bool>,
    /// Active right-click context menu; None when no menu is showing.
    pub(crate) context_menu: RwSignal<Option<ContextMenuState>>,
    /// Range being pointed at during formula entry (`[r1, c1, r2, c2]`, 1-based).
    /// `None` when not in point mode.
    pub(crate) point_range: RwSignal<Option<[i32; 4]>>,
    /// Byte span `(start, end)` within `editing_cell.text` that holds the
    /// current point-mode reference text, so it can be replaced in-place
    /// when the user presses arrow keys or clicks another cell.
    pub(crate) point_ref_span: RwSignal<Option<(usize, usize)>>,
    /// NodeRef to the formula bar <input> — used by FunctionBrowserModal
    /// to read/write cursor position when inserting a function name.
    pub(crate) formula_input_ref: NodeRef<leptos::html::Input>,
    /// Performance timings for the commit→render pipeline.
    pub perf: PerfTimings,
    /// Recent/custom colors used in the document (hex strings)
    /// Limited to 16 colors, most recent first
    pub(crate) recent_colors: RwSignal<Vec<String>>,
}

impl WorkbookState {
    pub fn new() -> Self {
        let lang: String = <gloo_storage::LocalStorage as GlooStorage>::get("ironcalc_lang")
            .unwrap_or_else(|_| "en".to_owned());
        
        // Load recent colors from localStorage
        let recent_colors: Vec<String> = <gloo_storage::LocalStorage as GlooStorage>::get("ironcalc_recent_colors")
            .unwrap_or_else(|_| Vec::new());
        
        Self {
            editing_cell: RwSignal::new(None),
            drag: RwSignal::new(DragState::Idle),
            redraw: RwSignal::new(0),
            current_uuid: RwSignal::new(None),
            theme: RwSignal::new(Theme::from_storage()),
            is_drawer_open: RwSignal::new(false),
            show_upload_dialog: RwSignal::new(false),
            share_url: RwSignal::new(None),
            current_lang: RwSignal::new(lang),
            show_regional_settings: RwSignal::new(false),
            show_named_ranges: RwSignal::new(false),
            show_perf_panel: RwSignal::new(false),
            context_menu: RwSignal::new(None),
            point_range: RwSignal::new(None),
            point_ref_span: RwSignal::new(None),
            formula_input_ref: NodeRef::new(),
            perf: PerfTimings::new(),
            recent_colors: RwSignal::new(recent_colors),
        }
    }

    /// Call after any UserModel mutation to trigger canvas re-render.
    pub fn request_redraw(&self) {
        self.redraw.update(|n| *n += 1);
    }
    
    /// Add a color to the recent colors list
    /// 
    /// - Moves color to front if already exists
    /// - Limits list to 16 colors maximum  
    /// - Persists to localStorage
    /// - Ignores colors already in COLOR_PALETTE
    pub fn add_recent_color(&self, color: &str) {
        use crate::theme::COLOR_PALETTE;
        
        // Don't add colors that are already in the standard palette
        if COLOR_PALETTE.contains(&color) {
            return;
        }
        
        // Normalize color (ensure lowercase, with #)
        let normalized = if color.starts_with('#') {
            color.to_lowercase()
        } else {
            format!("#{}", color.to_lowercase())
        };
        
        self.recent_colors.update(|colors| {
            // Remove if already exists
            colors.retain(|c| c != &normalized);
            
            // Add to front
            colors.insert(0, normalized);
            
            // Limit to 16 colors
            colors.truncate(16);
        });
        
        // Persist to localStorage
        let colors = self.recent_colors.get();
        <gloo_storage::LocalStorage as GlooStorage>::set("ironcalc_recent_colors", &colors).ok();
    }
    
    /// Get colors from the current document that aren't in the standard palette
    /// 
    /// This scans all cells and extracts unique colors for the recent colors section
    pub fn extract_document_colors(&self, model: ModelStore) -> Vec<String> {
        use crate::theme::COLOR_PALETTE;
        use std::collections::HashSet;
        
        let mut document_colors = HashSet::new();
        
        model.with_value(|m| {
            // Get all worksheets
            let sheets = m.get_worksheets_properties();
            
            for sheet_props in &sheets {
                let sheet_idx = sheet_props.sheet_id;
                
                // Scan a reasonable range of cells (don't scan infinite sheets)
                for row in 1..=100 {
                    for col in 1..=50 {
                        // Get cell style (only check cells that might have formatting)
                        if let Ok(style) = m.get_cell_style(sheet_idx, row, col) {
                            // Only process if the style has non-default values
                            if style.font.color.is_some() || style.fill.fg_color.is_some() {
                                    // Collect text color
                                    if let Some(text_color) = style.font.color.as_ref() {
                                        if !text_color.is_empty() && text_color != "#000000" {
                                            let normalized = text_color.to_lowercase();
                                            if !COLOR_PALETTE.contains(&normalized.as_str()) {
                                                document_colors.insert(normalized);
                                            }
                                        }
                                    }
                                    
                                    // Collect background color
                                    if let Some(bg_color) = style.fill.fg_color.as_ref() {
                                        if !bg_color.is_empty() {
                                            let normalized = bg_color.to_lowercase();
                                            if !COLOR_PALETTE.contains(&normalized.as_str()) {
                                                document_colors.insert(normalized);
                                            }
                                        }
                                    }
                                }
                            }
                    }
                }
            }
        });
        
        document_colors.into_iter().collect()
    }
}
