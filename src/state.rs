//! Transient UI state and reactive signal primitives.
//!
//! [`WorkbookState`] holds all ephemeral UI state as [`Split<T>`] signal pairs.
//! The model itself lives in a [`ModelStore`] context value, not here.

use gloo_storage::Storage as GlooStorage;
use ironcalc_base::UserModel;
use leptos::prelude::*;

use crate::coord::{CellAddress, CellArea};
use crate::events::*;
use crate::model::CssColor;
use crate::perf::PerfTimings;
use crate::theme::Theme;

pub type ModelStore = StoredValue<UserModel<'static>, LocalStorage>;

/// Zero-cost wrapper around a Leptos `(ReadSignal, WriteSignal)` pair.
pub struct Split<T: Clone + Send + Sync + 'static>(ReadSignal<T>, WriteSignal<T>);

// Manual impls: ReadSignal<T>/WriteSignal<T> are always Copy (arena IDs),
// so Split<T> is Copy for any T - even non-Copy types like String or Vec.
// #[derive(Copy)] would incorrectly add a T: Copy bound.
impl<T: Clone + Send + Sync + 'static> Clone for Split<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: Clone + Send + Sync + 'static> Copy for Split<T> {}

impl<T: Clone + Send + Sync + 'static> Split<T> {
    pub fn new(initial: T) -> Self {
        let (r, w) = signal(initial);
        Self(r, w)
    }

    pub fn get(&self) -> T {
        self.0.get()
    }

    pub fn get_untracked(&self) -> T {
        self.0.get_untracked()
    }

    #[allow(dead_code)]
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.0.with(f)
    }

    pub fn with_untracked<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.0.with_untracked(f)
    }

    pub fn set(&self, v: T) {
        self.1.set(v);
    }

    pub fn update(&self, f: impl FnOnce(&mut T)) {
        self.1.update(f);
    }

    #[allow(dead_code)]
    pub fn read(&self) -> ReadSignal<T> {
        self.0
    }

    #[allow(dead_code)]
    pub fn write(&self) -> WriteSignal<T> {
        self.1
    }
}

/// Single enum ensures at most one drag mode is active — illegal
/// combinations (e.g. selecting while resizing) are unrepresentable.
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
    /// Formula point-mode: highlighted range + byte span in formula text.
    Pointing {
        range: CellArea,
        ref_span: (usize, usize),
    },
}

/// Arrow key behavior during a cell edit.
#[derive(Clone, Debug, PartialEq)]
pub enum EditMode {
    /// Arrows commit and navigate. Default from printable keypress.
    Accept,
    /// Arrows move text cursor. Entered via F2 or double-click.
    Edit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EditFocus {
    Cell,
    FormulaBar,
}

/// In-progress cell edit not yet committed to the model.
#[derive(Clone, Debug, PartialEq)]
pub struct EditingCell {
    pub(crate) address: CellAddress,
    pub(crate) text: String,
    pub(crate) mode: EditMode,
    pub(crate) focus: EditFocus,
    /// Set on user input (typing, paste); cleared on arrow key consumption.
    /// In `Edit` mode, gates whether arrows enter point-mode — distinguishes
    /// "typed an operator" from "cursor moved through a reference position".
    pub(crate) text_dirty: bool,
}

/// 1-based index of the right-clicked header.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum HeaderContextMenu {
    Column(i32),
    Row(i32),
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct ContextMenuState {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) target: HeaderContextMenu,
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct WorkbookState {
    pub events: EventBus,
    pub(crate) current_uuid: Split<Option<String>>,
    pub(crate) theme: Split<Theme>,
    pub(crate) recent_colors: Split<Vec<CssColor>>,
    pub(crate) sidebar_open: Split<bool>,
    pub(crate) collapsed_groups: Split<Vec<String>>,
    pub(crate) editing_cell: Split<Option<EditingCell>>,
    pub(crate) formula_input_ref: NodeRef<leptos::html::Input>,
    pub(crate) drag: Split<DragState>,
    pub(crate) context_menu: Split<Option<ContextMenuState>>,
    pub perf: PerfTimings,
    pub(crate) show_perf_panel: Split<bool>,
}

impl WorkbookState {
    pub fn new() -> Self {
        // let lang: String = <gloo_storage::LocalStorage as GlooStorage>::get("ironcalc_lang")
        //    .unwrap_or_else(|_| "en".to_owned());

        // Load recent colors from localStorage (CssColor is serde-transparent, same JSON as String)
        let recent_colors: Vec<CssColor> =
            <gloo_storage::LocalStorage as GlooStorage>::get("ironcalc_recent_colors")
                .unwrap_or_default();

        Self {
            events: EventBus::new(),
            current_uuid: Split::new(None),
            theme: Split::new(Theme::from_storage()),
            recent_colors: Split::new(recent_colors),
            sidebar_open: Split::new(false),
            collapsed_groups: Split::new(vec![]),
            editing_cell: Split::new(None),
            formula_input_ref: NodeRef::new(),
            drag: Split::new(DragState::Idle),
            context_menu: Split::new(None),
            perf: PerfTimings::new(),
            show_perf_panel: Split::new(false),
        }
    }

    /// Active point-mode range, or 1x1 at the current cell if not pointing yet.
    pub(crate) fn effective_point_range(&self, model: ModelStore) -> CellArea {
        if let DragState::Pointing { range, .. } = self.drag.get_untracked() {
            range
        } else {
            model.with_value(|m| {
                let v = m.get_selected_view();
                CellArea::from_cell(v.row, v.column)
            })
        }
    }

    //  Event System

    pub fn emit_event(&self, event: SpreadsheetEvent) {
        self.emit_events(vec![event]);
    }

    pub fn emit_events(&self, new_events: impl IntoIterator<Item = SpreadsheetEvent>) {
        let mut content = vec![];
        let mut format = vec![];
        let mut navigation = vec![];
        let mut structure = vec![];
        let mut theme = vec![];

        for event in new_events {
            #[cfg(debug_assertions)]
            {
                use std::cell::Cell;
                thread_local! { static LAST: Cell<f64> = const { Cell::new(0.0) }; }
                let now = crate::perf::now();
                LAST.with(|t| {
                    // let delta = now - t.get();
                    t.set(now);
                    // leptos::logging::log!(
                    //     "[EventBus] +{delta:>8.2}ms  {}",
                    //     event.dbg_description()
                    // );
                });
            }
            match event {
                SpreadsheetEvent::Content(e) => content.push(e),
                SpreadsheetEvent::Format(e) => format.push(e),
                SpreadsheetEvent::Navigation(e) => navigation.push(e),
                SpreadsheetEvent::Structure(e) => structure.push(e),
                SpreadsheetEvent::Theme(e) => theme.push(e),
            }
        }

        // Replace all 5 signals so no stale events from the previous action remain.
        // Use update() not set(): set() uses PartialEq and suppresses notification when
        // the same event fires twice on the same range (e.g. toggle bold twice without
        // navigating). update() always notifies subscribers regardless of value equality.
        self.events.content.update(|v| *v = content);
        self.events.format.update(|v| *v = format);
        self.events.navigation.update(|v| *v = navigation);
        self.events.structure.update(|v| *v = structure);
        self.events.theme.update(|v| *v = theme);
    }

    /// Get the resolved theme (reactive) - Auto resolves to Light/Dark based on system preference
    pub fn get_theme(&self) -> Theme {
        self.theme.get().resolve_with_system()
    }

    /// Get the resolved theme (non-reactive) - Auto resolves to Light/Dark based on system preference
    #[allow(dead_code)]
    pub fn get_theme_untracked(&self) -> Theme {
        self.theme.get_untracked().resolve_with_system()
    }

    /// Set the theme preference.
    /// Persistence and DOM update are handled by the `use_rusty_calc_theme`
    /// sync Effect in `App` - no manual save needed here.
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
    #[allow(dead_code)]
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

        // Convert to Vec<CssColor> for storage and event (same JSON representation)
        let string_colors: Vec<CssColor> = self
            .recent_colors
            .with_untracked(|colors| colors.iter().map(|c| c.to_owned()).collect());
        <gloo_storage::LocalStorage as GlooStorage>::set("rustycalc_recent_colors", &string_colors)
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
    #[allow(dead_code)]
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
