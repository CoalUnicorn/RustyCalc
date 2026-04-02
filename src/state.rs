use gloo_storage::Storage as GlooStorage;
use ironcalc_base::UserModel;
use leptos::prelude::*;
use std::rc::Rc;

// NOTE: <Meta name="color-scheme" content="dark"/>
use crate::events::*;
use crate::model::CellAddress;
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
    // pub(crate) sheet: u32,
    // pub(crate) row: i32,
    // pub(crate) col: i32,
    pub(crate) address: CellAddress,
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

/// Position and target for the active context menu.
#[derive(Clone, Copy)]
pub struct ContextMenuState {
    /// Viewport-relative x (from `ev.client_x()`).
    pub(crate) x: i32,
    /// Viewport-relative y (from `ev.client_y()`).
    pub(crate) y: i32,
    /// Which header triggered the menu.
    pub(crate) target: HeaderContextMenu,
}

/// All transient UI state — never persisted, never in the model.
///
/// Uses split signals (ReadSignal/WriteSignal) for fine-grained reactivity.
/// Components that only read can subscribe to ReadSignal without being affected
/// by writes from other components. This reduces unnecessary re-renders.
///
/// The model itself is NOT stored here — it lives in a `StoredValue::new_local`
/// in `App` and is accessed via `use_context::<StoredValue<UserModel<'static>, LocalStorage>>()`.
/// A split signal redraw counter (also in context) triggers canvas re-draws.
///
/// Note: row/col/sheet are NOT stored here — they are always derived from
/// `UserModel::get_selected_view()` inside a reactive closure that reads the redraw signal
/// to stay in sync with the model's navigation state.
#[derive(Clone, Copy)]
pub struct WorkbookState {
    /// None = not editing; Some = live edit buffer
    /// Split signals for better granularity: readers don't get notified of writes
    pub(crate) editing_cell: (
        ReadSignal<Option<EditingCell>>,
        WriteSignal<Option<EditingCell>>,
    ),
    /// Active mouse-drag interaction (selection, resize, autofill, point-mode).
    pub(crate) drag: (ReadSignal<DragState>, WriteSignal<DragState>),
    /// Typed event stream for fine-grained reactivity.
    pub(crate) events: (
        ReadSignal<Vec<SpreadsheetEvent>>,
        WriteSignal<Vec<SpreadsheetEvent>>,
    ),
    /// UUID of the workbook currently loaded in the model.
    /// Used by `storage::save` to write back to the correct localStorage key.
    /// `None` during the brief window before a workbook is loaded.
    pub(crate) current_uuid: (ReadSignal<Option<String>>, WriteSignal<Option<String>>),
    /// Active color theme; Enhanced with leptos-use auto-detection behind the scenes
    pub(crate) theme: (ReadSignal<Theme>, WriteSignal<Theme>),
    /// Whether the left workbook-list drawer is open.
    // pub(crate) is_drawer_open: (ReadSignal<bool>, WriteSignal<bool>),
    /// Controls the Upload xlsx file dialog.
    // pub(crate) show_upload_dialog: (ReadSignal<bool>, WriteSignal<bool>),
    /// Controls the Share dialog; holds the URL returned by the server.
    // pub(crate) share_url: (ReadSignal<Option<String>>, WriteSignal<Option<String>>),
    /// BCP-47 language tag for formula language, persisted in localStorage.
    // pub(crate) current_lang: (ReadSignal<String>, WriteSignal<String>),
    /// Whether the Regional Settings right panel is open.
    // pub(crate) show_regional_settings: (ReadSignal<bool>, WriteSignal<bool>),
    /// Whether the Named Ranges right panel is open.
    // pub(crate) show_named_ranges: (ReadSignal<bool>, WriteSignal<bool>),
    /// Whether the Performance panel is visible.
    pub(crate) show_perf_panel: (ReadSignal<bool>, WriteSignal<bool>),
    /// Active right-click context menu; None when no menu is showing.
    pub(crate) context_menu: (
        ReadSignal<Option<ContextMenuState>>,
        WriteSignal<Option<ContextMenuState>>,
    ),
    /// Range being pointed at during formula entry (`[r1, c1, r2, c2]`, 1-based).
    /// `None` when not in point mode.
    pub(crate) point_range: (ReadSignal<Option<[i32; 4]>>, WriteSignal<Option<[i32; 4]>>),
    /// Byte span `(start, end)` within `editing_cell.text` that holds the
    /// current point-mode reference text, so it can be replaced in-place
    /// when the user presses arrow keys or clicks another cell.
    pub(crate) point_ref_span: (
        ReadSignal<Option<(usize, usize)>>,
        WriteSignal<Option<(usize, usize)>>,
    ),
    /// NodeRef to the formula bar <input> — used by FunctionBrowserModal
    /// to read/write cursor position when inserting a function name.
    pub(crate) formula_input_ref: NodeRef<leptos::html::Input>,
    /// Performance timings for the commit→render pipeline.
    pub perf: PerfTimings,
    /// Recent/custom colors used in the document (hex strings)
    /// Limited to 16 colors, most recent first
    pub(crate) recent_colors: (ReadSignal<Vec<String>>, WriteSignal<Vec<String>>),
    /// Per-category derived memos.
    ///
    /// Filtering runs once inside the Memo when the events Vec changes, not
    /// once per subscriber per tick.  Downstream components only re-run when
    /// their specific category actually has new events.
    pub(crate) format_events_memo: Memo<Vec<FormatEvent>>,
    pub(crate) theme_events_memo: Memo<Vec<ThemeEvent>>,
    pub(crate) content_events_memo: Memo<Vec<ContentEvent>>,
    pub(crate) navigation_events_memo: Memo<Vec<NavigationEvent>>,
    pub(crate) structure_events_memo: Memo<Vec<StructureEvent>>,
    pub(crate) mode_events_memo: Memo<Vec<ModeEvent>>,
    pub(crate) visual_events_memo: Memo<Vec<SpreadsheetEvent>>,
}

impl WorkbookState {
    pub fn new() -> Self {
        let lang: String = <gloo_storage::LocalStorage as GlooStorage>::get("ironcalc_lang")
            .unwrap_or_else(|_| "en".to_owned());

        // Load recent colors from localStorage
        let recent_colors: Vec<String> =
            <gloo_storage::LocalStorage as GlooStorage>::get("ironcalc_recent_colors")
                .unwrap_or_else(|_| Vec::new());

        // Create the events signal up-front so the read half can be captured
        // by each per-category Memo below.
        let events: (
            ReadSignal<Vec<SpreadsheetEvent>>,
            WriteSignal<Vec<SpreadsheetEvent>>,
        ) = signal(Vec::new());
        let ev = events.0;

        let format_events_memo = Memo::new(move |_| {
            ev.get()
                .into_iter()
                .filter_map(|e| match e {
                    SpreadsheetEvent::Format(f) => Some(f),
                    _ => None,
                })
                .collect::<Vec<_>>()
        });
        let theme_events_memo = Memo::new(move |_| {
            ev.get()
                .into_iter()
                .filter_map(|e| match e {
                    SpreadsheetEvent::Theme(t) => Some(t),
                    _ => None,
                })
                .collect::<Vec<_>>()
        });
        let content_events_memo = Memo::new(move |_| {
            ev.get()
                .into_iter()
                .filter_map(|e| match e {
                    SpreadsheetEvent::Content(c) => Some(c),
                    _ => None,
                })
                .collect::<Vec<_>>()
        });
        let navigation_events_memo = Memo::new(move |_| {
            ev.get()
                .into_iter()
                .filter_map(|e| match e {
                    SpreadsheetEvent::Navigation(n) => Some(n),
                    _ => None,
                })
                .collect::<Vec<_>>()
        });
        let structure_events_memo = Memo::new(move |_| {
            ev.get()
                .into_iter()
                .filter_map(|e| match e {
                    SpreadsheetEvent::Structure(s) => Some(s),
                    _ => None,
                })
                .collect::<Vec<_>>()
        });
        let mode_events_memo = Memo::new(move |_| {
            ev.get()
                .into_iter()
                .filter_map(|e| match e {
                    SpreadsheetEvent::Mode(m) => Some(m),
                    _ => None,
                })
                .collect::<Vec<_>>()
        });
        let visual_events_memo = Memo::new(move |_| {
            ev.get()
                .into_iter()
                .filter(|e| {
                    matches!(
                        e,
                        SpreadsheetEvent::Content(_)
                            | SpreadsheetEvent::Format(_)
                            | SpreadsheetEvent::Navigation(_)
                            | SpreadsheetEvent::Structure(_)
                    )
                })
                .collect::<Vec<_>>()
        });

        Self {
            editing_cell: signal(None),
            drag: signal(DragState::Idle),
            events,
            current_uuid: signal(None),
            theme: signal(Theme::from_storage()),
            // is_drawer_open: signal(false),
            // show_upload_dialog: signal(false),
            // share_url: signal(None),
            // current_lang: signal(lang),
            // show_regional_settings: signal(false),
            // show_named_ranges: signal(false),
            show_perf_panel: signal(false),
            context_menu: signal(None),
            point_range: signal(None),
            point_ref_span: signal(None),
            formula_input_ref: NodeRef::new(),
            perf: PerfTimings::new(),
            recent_colors: signal(recent_colors),
            format_events_memo,
            theme_events_memo,
            content_events_memo,
            navigation_events_memo,
            structure_events_memo,
            mode_events_memo,
            visual_events_memo,
        }
    }

    /// Trigger a canvas re-render by emitting a generic content change event.
    pub fn request_redraw(&self) {
        self.emit_event(SpreadsheetEvent::Content(ContentEvent::GenericChange));
    }

    // ===== Event System =====

    /// Emit a typed event - replaces `request_redraw()` for specific changes.
    ///
    /// This method:
    /// 1. Adds the event to the event stream
    /// 2. Bumps the version counter for legacy subscribers
    /// 3. Auto-updates related signals based on event type
    pub fn emit_event(&self, event: SpreadsheetEvent) {
        // Add to event stream (keep last 100 events for subscribers)
        self.events.1.update(|events| {
            events.push(event.clone());
            if events.len() > 100 {
                events.drain(0..events.len() - 100);
            }
        });

        // Auto-update related signals based on event type
        match event {
            SpreadsheetEvent::Theme(ThemeEvent::ThemeToggled { new_theme }) => {
                self.theme.1.set(new_theme);
            }
            SpreadsheetEvent::Format(FormatEvent::RecentColorsUpdated { colors }) => {
                self.recent_colors.1.set(colors);
            }
            SpreadsheetEvent::Mode(ModeEvent::DragModeChanged { to_mode, .. }) => {
                self.drag.1.set(to_mode);
            }
            _ => {} // Other events don't auto-update signals
        }
    }

    /// Subscribe to all events (returns the current event list)
    pub fn get_events(&self) -> Vec<SpreadsheetEvent> {
        self.events.0.get()
    }

    /// Subscribe to events, with reactive updates
    pub fn subscribe_to_events(&self) -> impl Fn() -> Vec<SpreadsheetEvent> + Copy + use<'_> {
        move || self.events.0.get()
    }

    /// Subscribe to format-related events only
    pub fn subscribe_to_format_events(&self) -> impl Fn() -> Vec<FormatEvent> + Copy + use<'_> {
        let memo = self.format_events_memo;
        move || memo.get()
    }

    /// Subscribe to theme-related events only
    pub fn subscribe_to_theme_events(&self) -> impl Fn() -> Vec<ThemeEvent> + Copy + use<'_> {
        let memo = self.theme_events_memo;
        move || memo.get()
    }

    /// Subscribe to content-related events only
    pub fn subscribe_to_content_events(&self) -> impl Fn() -> Vec<ContentEvent> + Copy + use<'_> {
        let memo = self.content_events_memo;
        move || memo.get()
    }

    /// Subscribe to events affecting a specific sheet
    pub fn subscribe_to_sheet_events(
        &self,
        sheet: u32,
    ) -> impl Fn() -> Vec<SpreadsheetEvent> + Copy + use<'_> {
        move || {
            self.events
                .0
                .get()
                .into_iter()
                .filter(|e| e.affects_sheet(sheet))
                .collect()
        }
    }

    /// Subscribe to navigation-related events only (selection, sheet changes)
    pub fn subscribe_to_navigation_events(
        &self,
    ) -> impl Fn() -> Vec<NavigationEvent> + Copy + use<'_> {
        let memo = self.navigation_events_memo;
        move || memo.get()
    }

    /// Subscribe to structure-related events only (sheets, rows, columns)
    pub fn subscribe_to_structure_events(
        &self,
    ) -> impl Fn() -> Vec<StructureEvent> + Copy + use<'_> {
        let memo = self.structure_events_memo;
        move || memo.get()
    }

    /// Subscribe to mode-related events only (edit mode, drag mode)
    pub fn subscribe_to_mode_events(&self) -> impl Fn() -> Vec<ModeEvent> + Copy + use<'_> {
        let memo = self.mode_events_memo;
        move || memo.get()
    }

    /// Subscribe to visual-related events (content, format, navigation, structure)
    /// Optimized for components like worksheets that need to re-render on visual changes
    pub fn subscribe_to_visual_events(
        &self,
    ) -> impl Fn() -> Vec<SpreadsheetEvent> + Copy + use<'_> {
        let memo = self.visual_events_memo;
        move || memo.get()
    }

    // Convenience methods for commonly used signals
    // These reduce boilerplate and make the API more ergonomic

    /// Get the current editing cell (reactive)
    pub fn get_editing_cell(&self) -> Option<EditingCell> {
        self.editing_cell.0.get()
    }

    /// Get the current editing cell (non-reactive)
    pub fn get_editing_cell_untracked(&self) -> Option<EditingCell> {
        self.editing_cell.0.get_untracked()
    }

    /// Set the editing cell
    pub fn set_editing_cell(&self, cell: Option<EditingCell>) {
        self.editing_cell.1.set(cell);
    }

    /// Update the editing cell
    pub fn update_editing_cell(&self, f: impl FnOnce(&mut Option<EditingCell>)) {
        self.editing_cell.1.update(f);
    }

    /// Get the current drag state (reactive)
    pub fn get_drag(&self) -> DragState {
        self.drag.0.get()
    }

    /// Get the current drag state (non-reactive)
    pub fn get_drag_untracked(&self) -> DragState {
        self.drag.0.get_untracked()
    }

    /// Set the drag state
    pub fn set_drag(&self, drag: DragState) {
        self.drag.1.set(drag);
    }

    /// Get the current theme preference (reactive) - may return Auto
    pub fn get_theme_preference(&self) -> Theme {
        self.theme.0.get()
    }

    /// Get the current theme preference (non-reactive) - may return Auto
    pub fn get_theme_preference_untracked(&self) -> Theme {
        self.theme.0.get_untracked()
    }

    /// Get the resolved theme (reactive) - Auto resolves to Light/Dark based on system preference
    pub fn get_theme(&self) -> Theme {
        self.theme.0.get().resolve_with_system()
    }

    /// Get the resolved theme (non-reactive) - Auto resolves to Light/Dark based on system preference
    pub fn get_theme_untracked(&self) -> Theme {
        self.theme.0.get_untracked().resolve_with_system()
    }

    /// Set the theme preference and persist to storage
    pub fn set_theme(&self, theme: Theme) {
        self.theme.1.set(theme);
        theme.save(); // Keep manual persistence for now
        self.emit_event(SpreadsheetEvent::Theme(ThemeEvent::ThemeToggled {
            new_theme: theme,
        }));
    }

    /// Toggle theme in cycle: Auto -> Light -> Dark -> Auto
    pub fn toggle_theme(&self) {
        let current = self.get_theme_preference();
        let next = match current {
            Theme::Auto => Theme::Light,
            Theme::Light => Theme::Dark,
            Theme::Dark => Theme::Auto,
        };
        self.set_theme(next);
    }

    /// Toggle between Light and Dark only (preserving Auto if set)
    pub fn toggle_light_dark(&self) {
        let current = self.get_theme_preference();
        match current {
            Theme::Auto => {} // Keep Auto unchanged
            Theme::Light => self.set_theme(Theme::Dark),
            Theme::Dark => self.set_theme(Theme::Light),
        }
    }

    /// Get point range (reactive)
    pub fn get_point_range(&self) -> Option<[i32; 4]> {
        self.point_range.0.get()
    }

    /// Get point range (non-reactive)
    pub fn get_point_range_untracked(&self) -> Option<[i32; 4]> {
        self.point_range.0.get_untracked()
    }

    /// Set point range
    pub fn set_point_range(&self, range: Option<[i32; 4]>) {
        self.point_range.1.set(range);
    }

    /// Get point ref span (non-reactive)
    pub fn get_point_ref_span_untracked(&self) -> Option<(usize, usize)> {
        self.point_ref_span.0.get_untracked()
    }

    /// Set point ref span
    pub fn set_point_ref_span(&self, span: Option<(usize, usize)>) {
        self.point_ref_span.1.set(span);
    }

    /// Get context menu (reactive)
    pub fn get_context_menu(&self) -> Option<ContextMenuState> {
        self.context_menu.0.get()
    }

    /// Set context menu
    pub fn set_context_menu(&self, menu: Option<ContextMenuState>) {
        self.context_menu.1.set(menu);
    }

    /// Get current UUID (non-reactive)
    pub fn get_current_uuid_untracked(&self) -> Option<String> {
        self.current_uuid.0.get_untracked()
    }

    /// Set current UUID
    pub fn set_current_uuid(&self, uuid: Option<String>) {
        self.current_uuid.1.set(uuid);
    }

    /// Get show perf panel (reactive)
    pub fn get_show_perf_panel(&self) -> bool {
        self.show_perf_panel.0.get()
    }

    /// Set show perf panel
    pub fn set_show_perf_panel(&self, show: bool) {
        self.show_perf_panel.1.set(show);
    }

    /// Toggle show perf panel
    pub fn toggle_show_perf_panel(&self) {
        self.show_perf_panel.1.update(|v| *v = !*v);
    }

    /// Get recent colors (reactive)
    pub fn get_recent_colors(&self) -> Vec<String> {
        self.recent_colors.0.get()
    }

    /// Get recent colors (non-reactive)
    pub fn get_recent_colors_untracked(&self) -> Vec<String> {
        self.recent_colors.0.get_untracked()
    }

    /// Set recent colors
    pub fn set_recent_colors(&self, colors: Vec<String>) {
        self.recent_colors.1.set(colors);
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

        self.recent_colors.1.update(|colors| {
            // Remove if already exists
            colors.retain(|c| c != &normalized);

            // Add to front
            colors.insert(0, normalized);

            // Limit to 16 colors
            colors.truncate(16);
        });

        // Persist to localStorage (use with_untracked since this is called from callbacks)
        let colors = self.recent_colors.0.with_untracked(|colors| colors.clone());
        <gloo_storage::LocalStorage as GlooStorage>::set("ironcalc_recent_colors", &colors).ok();

        // Emit event for reactive subscribers
        self.emit_event(SpreadsheetEvent::Format(FormatEvent::RecentColorsUpdated {
            colors,
        }));
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

                // FIXME: Scan a reasonable range of cells (don't scan infinite sheets)
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
