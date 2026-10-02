//! Shared shape definitions and placement.
//!
//! [`cf`] defines every conditional-formatting glyph in one place, in unit-box
//! coordinates. [`place`] maps a unit-box definition into a target pixel box
//! with a uniform scale, an optional rotation, and no per-glyph allocation.
//! The CF renderer keeps the exhaustive `IconGlyph` dispatch and drives the
//! painter; backends stay primitive-only.

pub mod cf;
pub mod place;
