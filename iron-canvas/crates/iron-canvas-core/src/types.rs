//! Compatibility forwarders for the former `types::*` paths.
//!
//! The vocabulary they held now lives in its stage homes:
//!
//! - `types::coord` → [`crate::address`]
//! - `types::ui` → [`crate::chrome::hit`] and [`crate::geometry::prim`]
//! - `types::fetched` → [`crate::model::fetched`]

pub mod coord {
    pub use crate::address::*;
}

pub mod ui {
    pub use crate::chrome::hit::{HitTest, RefZone, ResizeTarget};
    pub use crate::geometry::prim::{RectCorner, Side};
}

pub mod fetched {
    pub use crate::model::fetched::Fetched;
}
