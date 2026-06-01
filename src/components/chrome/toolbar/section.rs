//! Toolbar section identity (pure view state — never derived from the model)
//! and the slot descriptor `OverflowRow` measures and collapses.

use leptos::prelude::AnyView;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ToolbarSection {
    #[default]
    Home,
    Data,
    View,
    File,
}

impl ToolbarSection {
    pub fn all() -> [ToolbarSection; 4] {
        [
            ToolbarSection::Home,
            ToolbarSection::Data,
            ToolbarSection::View,
            ToolbarSection::File,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            ToolbarSection::Home => "Home",
            ToolbarSection::Data => "Data",
            ToolbarSection::View => "View",
            ToolbarSection::File => "File",
        }
    }
}

#[derive(Clone)]
pub struct ToolSlot {
    pub label: &'static str,
    pub view: Rc<dyn Fn() -> AnyView>,
}

impl ToolSlot {
    pub fn new(label: &'static str, view: impl Fn() -> AnyView + 'static) -> Self {
        Self {
            label,
            view: Rc::new(view),
        }
    }
}
