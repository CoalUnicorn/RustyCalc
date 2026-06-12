//! Filled SVG icons, grouped by toolbar function. Each group is its own enum
//! implementing [`Glyph`]; the [`Icon`] component is generic over `Glyph`, so a
//! call site names the group it belongs to (`<Icon icon=TextIcon::Bold/>`) and
//! the path lookup is statically dispatched.
//!
//! Every glyph is one inline `<svg fill="currentColor">` so hover/active/disabled
//! recolor for free via the existing `.tb-btn` rules.

use leptos::prelude::*;

/// Inner SVG markup for a `0 0 24 24` viewBox. May contain multiple elements
/// (e.g. a color icon could carry a colored underline bar).
pub trait Glyph {
    fn path(&self) -> &'static str;
}

// ==============================================================================
// Group enums
// ==============================================================================
// One enum per toolbar function. Variants drop the group prefix the flat enum
// carried (`AlignLeft` -> `AlignIcon::Left`) since the type now names the group.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditIcon {
    Undo,
    Redo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextIcon {
    Bold,
    Italic,
    Underline,
    Strikethrough,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignIcon {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetIcon {
    Freeze,
    NamedRange,
    Camera,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileIcon {
    Import,
    Download,
    Share,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChromeIcon {
    Menu,
    GitHub,
    Sun,
    Moon,
}

/// Border-preset glyphs, one per [`BorderSide`](crate::model::style_types::BorderSide).
///
/// Each glyph is a single-color SVG: a faint **dashed** cell-outline reference
/// (drawn as a `fill="none"` stroke at group `opacity`, so overlaps never
/// darken) plus the active edge(s) as solid filled bars. Encoding meaning in
/// shape rather than color lets the whole set recolor for hover/active/disabled
/// and dark mode for free. Design source: `docs/designs/2026-06-01-border-icons-preview.html`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderIcon {
    All,
    Inner,
    Outer,
    Top,
    Right,
    Bottom,
    Left,
    CenterH,
    CenterV,
    None,
}

// ==============================================================================
// Glyph paths
// ==============================================================================

impl Glyph for EditIcon {
    fn path(&self) -> &'static str {
        match self {
            EditIcon::Undo => {
                r#"<path d="M12.5 8c-2.65 0-5.05.99-6.9 2.6L2 7v9h9l-3.62-3.62c1.39-1.16 3.16-1.88 5.12-1.88 3.54 0 6.55 2.31 7.6 5.5l2.37-.78C21.08 11.03 17.15 8 12.5 8z"/>"#
            }
            EditIcon::Redo => {
                r#"<path d="M18.4 10.6C16.55 8.99 14.15 8 11.5 8c-4.65 0-8.57 3.03-9.95 7.22L3.9 16c1.05-3.19 4.05-5.5 7.6-5.5 1.95 0 3.73.72 5.12 1.88L13 16h9V7l-3.6 3.6z"/>"#
            }
        }
    }
}

impl Glyph for TextIcon {
    fn path(&self) -> &'static str {
        match self {
            TextIcon::Bold => {
                r#"<path d="M15.6 10.8c.97-.66 1.65-1.76 1.65-2.8 0-2.26-1.75-4-4-4H7v14h7.04c2.09 0 3.71-1.7 3.71-3.79 0-1.52-.86-2.82-2.15-3.41zM10 6.5h3c.83 0 1.5.67 1.5 1.5S13.83 9.5 13 9.5h-3v-3zm3.5 9H10v-3h3.5c.83 0 1.5.67 1.5 1.5s-.67 1.5-1.5 1.5z"/>"#
            }
            TextIcon::Italic => r#"<path d="M10 4v3h2.21l-3.42 8H6v3h8v-3h-2.21l3.42-8H18V4z"/>"#,
            TextIcon::Underline => {
                r#"<path d="M12 17c3.31 0 6-2.69 6-6V3h-2.5v8c0 1.93-1.57 3.5-3.5 3.5S8.5 12.93 8.5 11V3H6v8c0 3.31 2.69 6 6 6zm-7 2v2h14v-2H5z"/>"#
            }
            TextIcon::Strikethrough => {
                r#"<path d="M10 19h4v-3h-4v3zM5 4v3h5v3h4V7h5V4H5zM3 14h18v-2H3v2z"/>"#
            }
        }
    }
}

impl Glyph for AlignIcon {
    fn path(&self) -> &'static str {
        match self {
            AlignIcon::Left => r#"<path d="M2 4h20v2H2zm0 4h12v2H2zm0 4h20v2H2zm0 4h12v2H2z"/>"#,
            AlignIcon::Center => r#"<path d="M2 4h20v2H2zm4 4h12v2H6zm-4 4h20v2H2zm4 4h12v2H6z"/>"#,
            AlignIcon::Right => {
                r#"<path d="M2 4h20v2H2zm8 4h12v2H10zm-8 4h20v2H2zm8 4h12v2H10z"/>"#
            }
        }
    }
}

impl Glyph for SheetIcon {
    fn path(&self) -> &'static str {
        match self {
            SheetIcon::Freeze => {
                r#"<path d="M3 4h18v16H3V4zm2 2v3h6V6H5zm8 0v3h6V6h-6zM5 11v7h6v-7H5zm8 0v7h6v-7h-6z"/>"#
            }
            SheetIcon::NamedRange => {
                r#"<path d="M21.4 11.6 12.4 2.6c-.36-.37-.86-.59-1.41-.59H4c-1.1 0-2 .9-2 2v7c0 .55.22 1.05.59 1.42l9 9c.36.36.86.58 1.41.58s1.05-.22 1.41-.59l7-7c.37-.36.59-.86.59-1.41s-.23-1.06-.59-1.42zM5.5 7C4.67 7 4 6.33 4 5.5S4.67 4 5.5 4 7 4.67 7 5.5 6.33 7 5.5 7z"/>"#
            }
            // Camera: body rectangle with lens circle and shutter notch.
            SheetIcon::Camera => {
                r#"<path d="M9 3 7.17 5H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V7c0-1.1-.9-2-2-2h-3.17L15 3H9zm3 15c-2.76 0-5-2.24-5-5s2.24-5 5-5 5 2.24 5 5-2.24 5-5 5zm0-8c-1.66 0-3 1.34-3 3s1.34 3 3 3 3-1.34 3-3-1.34-3-3-3z"/>"#
            }
        }
    }
}

impl Glyph for FileIcon {
    fn path(&self) -> &'static str {
        match self {
            FileIcon::Import => {
                r#"<path d="M11 20h2v-8.17l3.59 3.58L18 14l-6-6-6 6 1.41 1.41L11 11.83V20zM5 4h14v2H5z"/>"#
            }
            FileIcon::Download => {
                r#"<path d="M11 4h2v8.17l3.59-3.58L18 10l-6 6-6-6 1.41-1.41L11 12.17V4zM5 18h14v2H5z"/>"#
            }
            FileIcon::Share => {
                r#"<path d="M18 16.08c-.76 0-1.44.3-1.96.77L8.91 12.7c.05-.23.09-.46.09-.7s-.04-.47-.09-.7l7.05-4.11c.54.5 1.25.81 2.04.81 1.66 0 3-1.34 3-3s-1.34-3-3-3-3 1.34-3 3c0 .24.04.47.09.7L8.04 9.81C7.5 9.31 6.79 9 6 9c-1.66 0-3 1.34-3 3s1.34 3 3 3c.79 0 1.5-.31 2.04-.81l7.12 4.16c-.05.21-.08.43-.08.65 0 1.61 1.31 2.92 2.92 2.92s2.92-1.31 2.92-2.92-1.31-2.92-2.92-2.92z"/>"#
            }
        }
    }
}

impl Glyph for ChromeIcon {
    fn path(&self) -> &'static str {
        match self {
            ChromeIcon::Menu => r#"<path d="M3 6h18v2H3V6zm0 5h18v2H3v-2zm0 5h18v2H3v-2z"/>"#,
            ChromeIcon::GitHub => {
                r#"<path d="M12 .5C5.37.5 0 5.78 0 12.29c0 5.21 3.44 9.63 8.21 11.19.6.11.82-.25.82-.56 0-.28-.01-1.02-.02-2-3.34.71-4.04-1.58-4.04-1.58-.55-1.37-1.34-1.74-1.34-1.74-1.09-.73.08-.72.08-.72 1.21.08 1.84 1.22 1.84 1.22 1.07 1.8 2.81 1.28 3.5.98.11-.76.42-1.28.76-1.58-2.67-.3-5.47-1.31-5.47-5.84 0-1.29.47-2.34 1.24-3.17-.12-.3-.54-1.52.12-3.17 0 0 1.01-.32 3.3 1.21.96-.26 1.98-.39 3-.4 1.02.01 2.04.14 3 .4 2.28-1.53 3.29-1.21 3.29-1.21.66 1.65.24 2.87.12 3.17.77.83 1.24 1.88 1.24 3.17 0 4.54-2.81 5.54-5.49 5.83.43.36.81 1.08.81 2.18 0 1.58-.01 2.85-.01 3.24 0 .31.21.68.83.56C20.57 21.91 24 17.5 24 12.29 24 5.78 18.63.5 12 .5z"/>"#
            }
            ChromeIcon::Sun => {
                r#"<path d="M6.76 4.84l-1.8-1.79-1.41 1.41 1.79 1.79 1.42-1.41zM4 10.5H1v2h3v-2zm9-9.95h-2V3.5h2V.55zm7.45 3.91l-1.41-1.41-1.79 1.79 1.41 1.41 1.79-1.79zm-3.21 13.7l1.79 1.8 1.41-1.41-1.8-1.79-1.4 1.4zM20 10.5v2h3v-2h-3zm-8-5c-3.31 0-6 2.69-6 6s2.69 6 6 6 6-2.69 6-6-2.69-6-6-6zm-1 16.95h2V19.5h-2v2.95zm-7.45-3.91l1.41 1.41 1.79-1.8-1.41-1.41-1.79 1.8z"/>"#
            }
            ChromeIcon::Moon => {
                r#"<path d="M9 2c-1.05 0-2.05.16-3 .46 4.06 1.27 7 5.06 7 9.54 0 4.48-2.94 8.27-7 9.54.95.3 1.95.46 3 .46 5.52 0 10-4.48 10-10S14.52 2 9 2z"/>"#
            }
        }
    }
}

impl Glyph for BorderIcon {
    fn path(&self) -> &'static str {
        match self {
            BorderIcon::All => {
                r#"<rect x="3" y="3" width="18" height="2"/><rect x="3" y="19" width="18" height="2"/><rect x="3" y="3" width="2" height="18"/><rect x="19" y="3" width="2" height="18"/><rect x="11" y="5" width="2" height="14"/><rect x="5" y="11" width="14" height="2"/>"#
            }
            BorderIcon::Inner => {
                r#"<g opacity="0.4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-dasharray="2.5 2"><rect x="4" y="4" width="16" height="16"/></g><rect x="11" y="5" width="2" height="14"/><rect x="5" y="11" width="14" height="2"/>"#
            }
            BorderIcon::Outer => {
                r#"<rect x="3" y="3" width="18" height="2"/><rect x="3" y="19" width="18" height="2"/><rect x="3" y="3" width="2" height="18"/><rect x="19" y="3" width="2" height="18"/><g opacity="0.4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-dasharray="2.5 2"><line x1="12" y1="4" x2="12" y2="20"/><line x1="4" y1="12" x2="20" y2="12"/></g>"#
            }
            BorderIcon::Top => {
                r#"<g opacity="0.4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-dasharray="2.5 2"><rect x="4" y="4" width="16" height="16"/></g><rect x="3" y="3" width="18" height="2"/>"#
            }
            BorderIcon::Right => {
                r#"<g opacity="0.4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-dasharray="2.5 2"><rect x="4" y="4" width="16" height="16"/></g><rect x="19" y="3" width="2" height="18"/>"#
            }
            BorderIcon::Bottom => {
                r#"<g opacity="0.4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-dasharray="2.5 2"><rect x="4" y="4" width="16" height="16"/></g><rect x="3" y="19" width="18" height="2"/>"#
            }
            BorderIcon::Left => {
                r#"<g opacity="0.4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-dasharray="2.5 2"><rect x="4" y="4" width="16" height="16"/></g><rect x="3" y="3" width="2" height="18"/>"#
            }
            BorderIcon::CenterH => {
                r#"<g opacity="0.4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-dasharray="2.5 2"><rect x="4" y="4" width="16" height="16"/></g><rect x="5" y="11" width="14" height="2"/>"#
            }
            BorderIcon::CenterV => {
                r#"<g opacity="0.4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-dasharray="2.5 2"><rect x="4" y="4" width="16" height="16"/></g><rect x="11" y="5" width="2" height="14"/>"#
            }
            BorderIcon::None => {
                r#"<g opacity="0.4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-dasharray="2.5 2"><rect x="4" y="4" width="16" height="16"/></g>"#
            }
        }
    }
}

/// Renders a group glyph as an inline SVG. `width`/`height` come from `.tb-ic` CSS.
#[component]
pub fn Icon<G>(icon: G) -> impl IntoView
where
    G: Glyph + 'static,
{
    view! {
        <svg class="tb-ic" viewBox="0 0 24 24" fill="currentColor" inner_html=icon.path()></svg>
    }
}

#[cfg(test)]
impl EditIcon {
    pub fn all() -> [EditIcon; 2] {
        [EditIcon::Undo, EditIcon::Redo]
    }
}

#[cfg(test)]
impl TextIcon {
    pub fn all() -> [TextIcon; 4] {
        [
            TextIcon::Bold,
            TextIcon::Italic,
            TextIcon::Underline,
            TextIcon::Strikethrough,
        ]
    }
}

#[cfg(test)]
impl AlignIcon {
    pub fn all() -> [AlignIcon; 3] {
        [AlignIcon::Left, AlignIcon::Center, AlignIcon::Right]
    }
}

#[cfg(test)]
impl SheetIcon {
    pub fn all() -> [SheetIcon; 3] {
        [SheetIcon::Freeze, SheetIcon::NamedRange, SheetIcon::Camera]
    }
}

#[cfg(test)]
impl FileIcon {
    pub fn all() -> [FileIcon; 3] {
        [FileIcon::Import, FileIcon::Download, FileIcon::Share]
    }
}

#[cfg(test)]
impl ChromeIcon {
    pub fn all() -> [ChromeIcon; 4] {
        [
            ChromeIcon::Menu,
            ChromeIcon::GitHub,
            ChromeIcon::Sun,
            ChromeIcon::Moon,
        ]
    }
}

#[cfg(test)]
impl BorderIcon {
    pub fn all() -> [BorderIcon; 10] {
        [
            BorderIcon::All,
            BorderIcon::Inner,
            BorderIcon::Outer,
            BorderIcon::Top,
            BorderIcon::Right,
            BorderIcon::Bottom,
            BorderIcon::Left,
            BorderIcon::CenterH,
            BorderIcon::CenterV,
            BorderIcon::None,
        ]
    }
}
