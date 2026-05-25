//! Font resources. Today: Helvetica (Type1 base-14, no embedding).
//!
//! Helvetica is one of the 14 fonts every PDF reader carries built-in,
//! so we reference it by `BaseFont` without shipping any font program.
//! The trade-off is WinAnsiEncoding — glyphs outside Latin-1 render as
//! `.notdef`. See the "Text encoding limitation" section of
//! `OUTPUT_REFACTOR_PLAN.md`.

/// Body of the `/Resources` object — wires `/F1` to Helvetica.
/// Returned as a complete dict ready for `PdfDocument::add_object`.
pub fn resources_object_with_helvetica() -> Vec<u8> {
    b"<< /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >>\n".to_vec()
}
