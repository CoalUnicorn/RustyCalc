//! Transient UI state and reactive signal primitives.
//!
//! [`WorkbookState`] holds all ephemeral UI state — nothing persisted, nothing
//! in the model. Every field is a [`Split<T>`] signal pair. Components read via
//! `.get()` (reactive) or `.get_untracked()` (in event handlers) and write via
//! `.set()` / `.update()`.
//!
//! The model itself lives in a [`ModelStore`] context value, not here.
//!
//! See `docs/state-and-events.md` for usage patterns and a fields reference.

use gloo_storage::Storage as GlooStorage;
use ironcalc_base::UserModel;
use leptos::prelude::*;

// NOTE: <Meta name="color-scheme" content="dark"/>
use crate::canvas::SheetRect;
use crate::events::*;
use crate::model::{CellAddress, CssColor};
use crate::perf::PerfTimings;
use crate::theme::Theme;

/// Shorthand for the context-provided `UserModel` storage handle.
///
/// `StoredValue<UserModel<'static>, LocalStorage>` is the Leptos arena-stored
/// wrapper used throughout the app.  This alias eliminates the repetition in
/// every `use_context` call.
pub type ModelStore = StoredValue<UserModel<'static>, LocalStorage>;

/// Thin zero-cost wrapper around a Leptos split-signal pair.
///
/// Replaces `(ReadSignal<T>, WriteSignal<T>)` tuple fields so callers use
/// named methods (`.get()`, `.set()`, `.read()`) rather than `.0` / `.1`.
/// Reactivity is identical — same two signal nodes, same reactive graph.
pub struct Split<T: Clone + Send + Sync + 'static>(ReadSignal<T>, WriteSignal<T>);

// Manual impls: ReadSignal<T>/WriteSignal<T> are always Copy (arena IDs),
// so Split<T> is Copy for any T — even non-Copy types like String or Vec.
// #[derive(Copy)] would incorrectly add a T: Copy bound.
impl<T: Clone + Send + Sync + 'static> Clone for Split<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: Clone + Send + Sync + 'static> Copy for Split<T> {}
#[allow(dead_code)]
impl<T: Clone + Send + Sync + 'static> Split<T> {
    pub fn new(initial: T) -> Self {
        let (r, w) = signal(initial);
        Self(r, w)
    }

    /// Reactive read — registers a dependency on the current reactive owner.
    pub fn get(&self) -> T {
        self.0.get()
    }

    /// Non-reactive read — safe to call outside reactive closures.
    pub fn get_untracked(&self) -> T {
        self.0.get_untracked()
    }

    /// Borrow the current value without cloning (reactive).
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.0.with(f)
    }

    /// Borrow the current value without cloning (non-reactive).
    pub fn with_untracked<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.0.with_untracked(f)
    }

    /// Write a new value. Always notifies subscribers.
    pub fn set(&self, v: T) {
        self.1.set(v);
    }

    /// Update in place.
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        self.1.update(f);
    }

    /// The read half — pass to child components that should only subscribe.
    pub fn read(&self) -> ReadSignal<T> {
        self.0
    }

    /// The write half — pass to child components that should only mutate.
    pub fn write(&self) -> WriteSignal<T> {
        self.1
    }
}

/// In-progress cell edit not yet committed to the model.
/// Mirrors the TypeScript `EditingCell` interface in workbookState.ts.
#[derive(Clone, Debug, PartialEq)]
pub struct EditingCell {
    /// Cell being edited.
    pub(crate) address: CellAddress,
    /// Text the user has typed; not yet written to `UserModel`.
    pub(crate) text: String,
    /// How arrow keys behave during the edit.
    pub(crate) mode: EditMode,
    /// Which widget currently holds keyboard focus.
    pub(crate) focus: EditFocus,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EditMode {
    /// Arrow keys commit the edit and navigate to the adjacent cell.
    ///
    /// The default mode when editing starts from a printable keypress.
    Accept,
    /// Arrow keys move the text cursor within the formula.
    ///
    /// Entered via F2 or double-click.
    Edit,
}

/// Which widget holds keyboard focus during a cell edit.
#[derive(Clone, Debug, PartialEq)]
pub enum EditFocus {
    /// The in-cell `<textarea>` overlay positioned over the active cell.
    Cell,
    /// The formula bar `<input>` at the top of the workbook.
    FormulaBar,
}

/// Position and target for the active context menu.
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct ContextMenuState {
    /// Viewport-relative x (from `ev.client_x()`).
    pub(crate) x: i32,
    /// Viewport-relative y (from `ev.client_y()`).
    pub(crate) y: i32,
    /// Which header triggered the menu.
    pub(crate) target: HeaderContextMenu,
}

/// Per-category event signals.
///
/// Replaces the single `Vec<SpreadsheetEvent>` + 7 `Memo` filters.
/// Each signal holds the events from the most recent `emit_event(s)` call.
/// Contents are REPLACED (not appended) on each emit — never accumulate
/// cross-action history here.
#[derive(Clone, Copy)]
pub struct EventBus {
    pub content: RwSignal<Vec<ContentEvent>>,
    pub format: RwSignal<Vec<FormatEvent>>,
    pub navigation: RwSignal<Vec<NavigationEvent>>,
    pub structure: RwSignal<Vec<StructureEvent>>,
    pub mode: RwSignal<Vec<ModeEvent>>,
    pub theme: RwSignal<Vec<ThemeEvent>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            content: RwSignal::new(vec![]),
            format: RwSignal::new(vec![]),
            navigation: RwSignal::new(vec![]),
            structure: RwSignal::new(vec![]),
            mode: RwSignal::new(vec![]),
            theme: RwSignal::new(vec![]),
        }
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// All transient UI state - never persisted, never in the model.
///
/// Uses split signals (ReadSignal/WriteSignal) for fine-grained reactivity.
/// Components that only read can subscribe to ReadSignal without being affected
/// by writes from other components. This reduces unnecessary re-renders.
///
/// The model itself is NOT stored here - it lives in a `StoredValue::new_local`
/// in `App` and is accessed via `use_context::<StoredValue<UserModel<'static>, LocalStorage>>()`.
/// A split signal redraw counter (also in context) triggers canvas re-draws.
///
/// Note: row/col/sheet are NOT stored here - they are always derived from
/// `UserModel::get_selected_view()` inside a reactive closure that reads the redraw signal
/// to stay in sync with the model's navigation state.
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct WorkbookState {
    /// None = not editing; Some = live edit buffer
    pub(crate) editing_cell: Split<Option<EditingCell>>,
    /// Active mouse-drag interaction (selection, resize, autofill, point-mode).
    pub(crate) drag: Split<DragState>,
    /// Typed per-category event bus.
    pub events: EventBus,
    /// UUID of the workbook currently loaded in the model.
    pub(crate) current_uuid: Split<Option<String>>,
    /// Active color theme preference (may be Auto).
    pub(crate) theme: Split<Theme>,
    /// Whether the Performance panel is visible.
    pub(crate) show_perf_panel: Split<bool>,
    /// Active right-click context menu; None when no menu is showing.
    pub(crate) context_menu: Split<Option<ContextMenuState>>,
    /// NodeRef to the formula bar <input>.
    pub(crate) formula_input_ref: NodeRef<leptos::html::Input>,
    /// Performance timings for the commit→render pipeline.
    pub perf: PerfTimings,
    /// Recent/custom colors. Limited to 16, most recent first.
    pub(crate) recent_colors: Split<Vec<CssColor>>,
}

#[allow(dead_code)]
impl WorkbookState {
    /// Creates a `WorkbookState` with default signal values.
    ///
    /// Loads recent colors from `localStorage`. All other fields start at their
    /// zero values: no active edit, [`DragState::Idle`], theme loaded from storage.
    pub fn new() -> Self {
        // let lang: String = <gloo_storage::LocalStorage as GlooStorage>::get("ironcalc_lang")
        //    .unwrap_or_else(|_| "en".to_owned());

        // Load recent colors from localStorage (CssColor is serde-transparent, same JSON as String)
        let recent_colors: Vec<CssColor> =
            <gloo_storage::LocalStorage as GlooStorage>::get("ironcalc_recent_colors")
                .unwrap_or_default();

        Self {
            editing_cell: Split::new(None),
            drag: Split::new(DragState::Idle),
            events: EventBus::new(),
            current_uuid: Split::new(None),
            theme: Split::new(Theme::from_storage()),
            show_perf_panel: Split::new(false),
            context_menu: Split::new(None),
            formula_input_ref: NodeRef::new(),
            perf: PerfTimings::new(),
            recent_colors: Split::new(recent_colors),
        }
    }

    /// Trigger a canvas re-render by emitting a generic content change event.
    pub fn request_redraw(&self) {
        self.emit_event(SpreadsheetEvent::Content(ContentEvent::GenericChange));
    }

    /// Returns the active point-mode range, or a 1×1 rect at the model's
    /// current cell when point-mode has not started yet.
    ///
    /// Use this in event handlers that need a point-mode anchor regardless of
    /// whether point-mode is already active (e.g. arrow-key extension in
    /// `Accept` edit mode).
    pub(crate) fn effective_point_range(&self, model: ModelStore) -> SheetRect {
        if let DragState::Pointing { range, .. } = self.drag.get_untracked() {
            range
        } else {
            model.with_value(|m| {
                let v = m.get_selected_view();
                SheetRect::from_cell(v.row, v.column)
            })
        }
    }

    //  Event System

    /// Emit a typed event.
    ///
    /// This method:
    /// 1. Adds the event to the event stream
    /// 2. Bumps the version counter for legacy subscribers
    /// 3. Auto-updates related signals based on event type
    pub fn emit_event(&self, event: SpreadsheetEvent) {
        self.emit_events(vec![event]);
    }

    /// Emit multiple events in a single signal update.
    /// Use when an action produces several events (e.g. CommitAndNavigate).
    pub fn emit_events(&self, new_events: impl IntoIterator<Item = SpreadsheetEvent>) {
        let mut content = vec![];
        let mut format = vec![];
        let mut navigation = vec![];
        let mut structure = vec![];
        let mut mode_evs = vec![];
        let mut theme_evs = vec![];

        for event in new_events {
            #[cfg(debug_assertions)]
            {
                use std::cell::Cell;
                thread_local! { static LAST: Cell<f64> = const { Cell::new(0.0) }; }
                let now = crate::perf::now();
                LAST.with(|t| {
                    let delta = now - t.get();
                    t.set(now);
                    leptos::logging::log!(
                        "[EventBus] +{delta:>8.2}ms  {}",
                        event.dbg_description()
                    );
                });
            }
            match event {
                SpreadsheetEvent::Content(e) => content.push(e),
                SpreadsheetEvent::Format(e) => format.push(e),
                SpreadsheetEvent::Navigation(e) => navigation.push(e),
                SpreadsheetEvent::Structure(e) => structure.push(e),
                SpreadsheetEvent::Mode(e) => mode_evs.push(e),
                SpreadsheetEvent::Theme(e) => theme_evs.push(e),
            }
        }

        // Replace all 6 signals so no stale events from the previous action remain.
        // Use update() not set(): set() uses PartialEq and suppresses notification when
        // the same event fires twice on the same range (e.g. toggle bold twice without
        // navigating). update() always notifies subscribers regardless of value equality.
        self.events.content.update(|v| *v = content);
        self.events.format.update(|v| *v = format);
        self.events.navigation.update(|v| *v = navigation);
        self.events.structure.update(|v| *v = structure);
        self.events.mode.update(|v| *v = mode_evs);
        self.events.theme.update(|v| *v = theme_evs);
    }

    /// Get the resolved theme (reactive) - Auto resolves to Light/Dark based on system preference
    pub fn get_theme(&self) -> Theme {
        self.theme.get().resolve_with_system()
    }

    /// Get the resolved theme (non-reactive) - Auto resolves to Light/Dark based on system preference
    pub fn get_theme_untracked(&self) -> Theme {
        self.theme.get_untracked().resolve_with_system()
    }

    /// Set the theme preference.
    /// Persistence and DOM update are handled by the `use_rusty_calc_theme`
    /// sync Effect in `App` — no manual save needed here.
    pub fn set_theme(&self, theme: Theme) {
        self.theme.set(theme);
        self.emit_event(SpreadsheetEvent::Theme(ThemeEvent::ThemeToggled {
            new_theme: theme,
        }));
    }

    /// Toggle theme in cycle: Auto -> Light -> Dark -> Auto
    pub fn toggle_theme(&self) {
        let current = self.theme.get();
        let next = match current {
            Theme::Auto => Theme::Light,
            Theme::Light => Theme::Dark,
            Theme::Dark => Theme::Auto,
        };
        self.set_theme(next);
    }

    /// Toggle between Light and Dark only (preserving Auto if set)
    pub fn toggle_light_dark(&self) {
        let current = self.theme.get();
        match current {
            Theme::Auto => {} // Keep Auto unchanged
            Theme::Light => self.set_theme(Theme::Dark),
            Theme::Dark => self.set_theme(Theme::Light),
        }
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

        // Normalize color (ensure lowercase, with #) and wrap in the domain type.
        let normalized = CssColor::new(if color.starts_with('#') {
            color.to_lowercase()
        } else {
            format!("#{}", color.to_lowercase())
        });

        self.recent_colors.update(|colors| {
            // Remove if already exists
            colors.retain(|c| c != &normalized);

            // Add to front
            colors.insert(0, normalized);

            // Limit to 16 colors
            colors.truncate(16);
        });

        // Convert to Vec<String> for storage and event (same JSON representation)
        let string_colors: Vec<String> = self
            .recent_colors
            .with_untracked(|colors| colors.iter().map(|c| c.as_str().to_owned()).collect());
        <gloo_storage::LocalStorage as GlooStorage>::set("ironcalc_recent_colors", &string_colors)
            .ok();

        // Emit event for reactive subscribers
        self.emit_event(SpreadsheetEvent::Format(FormatEvent::RecentColorsUpdated {
            colors: string_colors,
        }));
    }

    /// Get colors from the current document that aren't in the standard palette
    ///
    /// NOTE: Check if this works need import support
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
