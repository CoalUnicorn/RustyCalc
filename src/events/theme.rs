//! Theme-domain events: appearance, palette, locale.

use crate::theme::Theme;

#[derive(Clone, PartialEq, Debug)]
pub enum ThemeEvent {
    ThemeToggled {
        new_theme: Theme,
    },
    #[allow(dead_code)]
    PaletteUpdated,
    #[allow(dead_code)]
    LocaleChanged {
        new_locale: String,
    },
}
