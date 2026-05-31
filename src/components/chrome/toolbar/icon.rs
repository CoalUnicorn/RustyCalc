//! Filled SVG icons. One inline `<svg fill="currentColor">` per glyph so
//! hover/active/disabled recolor for free via the existing `.tb-btn` rules.

use leptos::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconName {
    Undo,
    Redo,
    Bold,
    Italic,
    Underline,
    Strikethrough,
    AlignLeft,
    AlignCenter,
    AlignRight,
    Freeze,
    NamedRange,
}

impl IconName {
    #[cfg(test)]
    pub fn all() -> [IconName; 11] {
        use IconName::*;
        [
            Undo,
            Redo,
            Bold,
            Italic,
            Underline,
            Strikethrough,
            AlignLeft,
            AlignCenter,
            AlignRight,
            Freeze,
            NamedRange,
        ]
    }

    /// Inner SVG markup for a 0 0 24 24 viewBox. May contain multiple elements
    /// (e.g. the color icons carry a colored underline bar).
    pub fn path(self) -> &'static str {
        use IconName::*;
        match self {
            Undo => {
                r#"<path d="M12.5 8c-2.65 0-5.05.99-6.9 2.6L2 7v9h9l-3.62-3.62c1.39-1.16 3.16-1.88 5.12-1.88 3.54 0 6.55 2.31 7.6 5.5l2.37-.78C21.08 11.03 17.15 8 12.5 8z"/>"#
            }
            Redo => {
                r#"<path d="M18.4 10.6C16.55 8.99 14.15 8 11.5 8c-4.65 0-8.57 3.03-9.95 7.22L3.9 16c1.05-3.19 4.05-5.5 7.6-5.5 1.95 0 3.73.72 5.12 1.88L13 16h9V7l-3.6 3.6z"/>"#
            }
            Bold => {
                r#"<path d="M15.6 10.8c.97-.66 1.65-1.76 1.65-2.8 0-2.26-1.75-4-4-4H7v14h7.04c2.09 0 3.71-1.7 3.71-3.79 0-1.52-.86-2.82-2.15-3.41zM10 6.5h3c.83 0 1.5.67 1.5 1.5S13.83 9.5 13 9.5h-3v-3zm3.5 9H10v-3h3.5c.83 0 1.5.67 1.5 1.5s-.67 1.5-1.5 1.5z"/>"#
            }
            Italic => r#"<path d="M10 4v3h2.21l-3.42 8H6v3h8v-3h-2.21l3.42-8H18V4z"/>"#,
            Underline => {
                r#"<path d="M12 17c3.31 0 6-2.69 6-6V3h-2.5v8c0 1.93-1.57 3.5-3.5 3.5S8.5 12.93 8.5 11V3H6v8c0 3.31 2.69 6 6 6zm-7 2v2h14v-2H5z"/>"#
            }
            Strikethrough => {
                r#"<path d="M10 19h4v-3h-4v3zM5 4v3h5v3h4V7h5V4H5zM3 14h18v-2H3v2z"/>"#
            }
            AlignLeft => r#"<path d="M2 4h20v2H2zm0 4h12v2H2zm0 4h20v2H2zm0 4h12v2H2z"/>"#,
            AlignCenter => r#"<path d="M2 4h20v2H2zm4 4h12v2H6zm-4 4h20v2H2zm4 4h12v2H6z"/>"#,
            AlignRight => r#"<path d="M2 4h20v2H2zm8 4h12v2H10zm-8 4h20v2H2zm8 4h12v2H10z"/>"#,
            Freeze => {
                r#"<path d="M3 4h18v16H3V4zm2 2v3h6V6H5zm8 0v3h6V6h-6zM5 11v7h6v-7H5zm8 0v7h6v-7h-6z"/>"#
            }
            NamedRange => {
                r#"<path d="M21.4 11.6 12.4 2.6c-.36-.37-.86-.59-1.41-.59H4c-1.1 0-2 .9-2 2v7c0 .55.22 1.05.59 1.42l9 9c.36.36.86.58 1.41.58s1.05-.22 1.41-.59l7-7c.37-.36.59-.86.59-1.41s-.23-1.06-.59-1.42zM5.5 7C4.67 7 4 6.33 4 5.5S4.67 4 5.5 4 7 4.67 7 5.5 6.33 7 5.5 7z"/>"#
            }
        }
    }
}

#[component]
pub fn Icon(name: IconName) -> impl IntoView {
    // inner_html injects the path markup; width/height come from `.tb-ic` CSS.
    view! {
        <svg class="tb-ic" viewBox="0 0 24 24" fill="currentColor" inner_html=name.path()></svg>
    }
}
