//! Font + shading resources. Today: Helvetica (Type1 base-14, no embedding).
//!
//! Helvetica is one of the 14 fonts every PDF reader carries built-in,
//! so we reference it by `BaseFont` without shipping any font program.
//! The trade-off is WinAnsiEncoding — glyphs outside Latin-1 render as
//! `.notdef`. See the "Text encoding limitation" section of
//! `OUTPUT_REFACTOR_PLAN.md`.
//!
//! Horizontal-gradient data bars add one axial shading (`/ShadingType 2`) per
//! distinct color pair. Both the shading and its interpolation function are
//! plain dictionaries, so they inline here with no extra indirect objects.

use crate::common::color::parse_css_color;

/// Body of the `/Resources` object — wires `/F1` to Helvetica and one
/// `/Sh{n}` axial shading per `(from, to)` color pair in `shadings` (first-use
/// order, matching the `/Sh{index}` names the painter emitted). Returned as a
/// complete dict ready for `PdfDocument::add_object`.
pub fn resources_object_with_helvetica(shadings: &[(String, String)]) -> Vec<u8> {
    let mut out = Vec::from(
        &b"<< /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >>"[..],
    );
    if !shadings.is_empty() {
        out.extend_from_slice(b" /Shading <<");
        for (index, (from, to)) in shadings.iter().enumerate() {
            out.extend_from_slice(shading_dict(index, from, to).as_bytes());
        }
        out.extend_from_slice(b" >>");
    }
    out.extend_from_slice(b" >>\n");
    out
}

/// One `/Sh{n}` entry: an axial shading over the unit x-axis (`0 0` -> `1 0`)
/// that interpolates `from` at offset 0 to `to` at offset 1.
fn shading_dict(index: usize, from: &str, to: &str) -> String {
    let (r0, g0, b0) = parse_css_color(from);
    let (r1, g1, b1) = parse_css_color(to);
    format!(
        " /Sh{index} << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 1 0] \
         /Function << /FunctionType 2 /Domain [0 1] /C0 [{r0:.3} {g0:.3} {b0:.3}] \
         /C1 [{r1:.3} {g1:.3} {b1:.3}] /N 1 >> /Extend [true true] >>"
    )
}
