//! Real font metrics for the SVG/PDF export backends.
//!
//! The core crate's `approx_text_width` (flat `CHAR_WIDTH_FACTOR = 1.0` per
//! char) is ~2x wider than any real proportional font, so wrap math built on
//! it disagrees badly with what's actually painted — that mismatch is what
//! clips exported text (see `docs/reviews/2026-07-01-export-font-metrics-plan.md`).
//!
//! Both functions below measure the font each backend actually *draws*, not
//! the cell's declared family: SVG always renders (and now always emits) the
//! bundled Inter TTF; PDF always draws the base-14 Helvetica font regardless
//! of the cell's style. Any character missing from a table falls back to
//! `approx_text_width` for that one character only, so every other script
//! keeps working exactly as before.

use std::sync::LazyLock;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use iron_canvas_core::painter::approx_text_width;

//  Inter (SVG)

/// Inter Regular (v13, Latin subset) — see `assets/PROVENANCE.md` for
/// source, submodule commit, and license (`assets/OFL.txt`).
const INTER_TTF: &[u8] = include_bytes!("../../assets/inter-regular.ttf");

fn inter_face() -> &'static ttf_parser::Face<'static> {
    static FACE: LazyLock<ttf_parser::Face<'static>> = LazyLock::new(|| {
        ttf_parser::Face::parse(INTER_TTF, 0).expect("bundled Inter TTF must parse")
    });
    &FACE
}

/// Sum per-character advances for `text`.
///
/// `advance` returns the real advance for a character, or `None` when that
/// font's table has no entry for it — such a character alone falls back to the
/// flat [`approx_text_width`] estimate, so every mapped script keeps working
/// exactly as before. Each backend passes its own per-char lookup, so the
/// fallback and the summing live here once.
fn sum_advances(text: &str, size_px: f64, advance: impl Fn(char) -> Option<f64>) -> f64 {
    text.chars()
        .map(|c| {
            advance(c).unwrap_or_else(|| approx_text_width(size_px, c.encode_utf8(&mut [0u8; 4])))
        })
        .sum()
}

/// Sum of real Inter glyph advances for `text` at `size_px`. Characters with
/// no glyph in the embedded subset fall back to the flat estimate for that
/// character alone.
pub fn inter_advance_width(text: &str, size_px: f64) -> f64 {
    let face = inter_face();
    let units_per_em = f64::from(face.units_per_em());
    sum_advances(text, size_px, |c| {
        face.glyph_index(c)
            .and_then(|id| face.glyph_hor_advance(id))
            .map(|adv| f64::from(adv) / units_per_em * size_px)
    })
}

/// Base64 (standard alphabet, padded) of the embedded Inter TTF, for the SVG
/// `@font-face` data URI. Encoded once per process — the TTF is a build-time
/// constant, and every document that drew text carries the same data URI.
pub fn inter_base64() -> &'static str {
    static ENCODED: LazyLock<String> = LazyLock::new(|| STANDARD.encode(INTER_TTF));
    &ENCODED
}

//  Helvetica (PDF)

/// Published Helvetica AFM advance widths (1000-unit em), printable ASCII
/// `0x20..=0x7E` only — `HELVETICA_ASCII_WIDTHS[c as usize - 0x20]`. Standard
/// PDF base-14 metrics: the font every `PdfPainter::fill_text` call actually
/// draws (`/F1`), regardless of the cell's declared family. Scoped to ASCII
/// (rather than full Latin-1) because that's the range with well-published,
/// unambiguous reference values; anything outside it uses the flat fallback,
/// same as an unmapped Inter glyph.
#[rustfmt::skip]
const HELVETICA_ASCII_WIDTHS: [u16; 95] = [
    // 0x20-0x2F: space ! " # $ % & ' ( ) * + , - . /
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278,
    // 0x30-0x3F: 0-9 : ; < = > ?
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556,
    // 0x40-0x4F: @ A-O
    1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778,
    // 0x50-0x5F: P-Z [ \ ] ^ _
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556,
    // 0x60-0x6F: ` a-o
    333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556,
    // 0x70-0x7E: p-z { | } ~
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

/// Sum of Helvetica advances for `text` at `size_px`. Codepoints outside
/// printable ASCII fall back to the flat estimate for that character alone.
pub fn helvetica_advance_width(text: &str, size_px: f64) -> f64 {
    sum_advances(text, size_px, |c| {
        let idx = c as u32;
        (0x20..=0x7E)
            .contains(&idx)
            .then(|| f64::from(HELVETICA_ASCII_WIDTHS[(idx - 0x20) as usize]) / 1000.0 * size_px)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inter_word_measures_below_flat_estimate() {
        let flat = approx_text_width(13.0, "mortgage");
        let real = inter_advance_width("mortgage", 13.0);
        assert!(
            real < flat * 0.8,
            "real Inter advance ({real}) should be well below the flat 1.0-factor estimate ({flat})"
        );
        assert!(real > 0.0);
    }

    #[test]
    fn inter_unmapped_glyph_falls_back_instead_of_zero() {
        // Not in the Latin subset — must not silently measure 0.
        let w = inter_advance_width("📋", 13.0);
        assert_eq!(w, approx_text_width(13.0, "📋"));
    }

    #[test]
    fn inter_euro_sign_is_covered_by_the_embedded_subset() {
        // The Google Fonts "latin" subset explicitly includes U+20AC.
        let flat = approx_text_width(13.0, "€");
        let real = inter_advance_width("€", 13.0);
        assert!(real > 0.0);
        assert_ne!(
            real, flat,
            "€ should resolve to a real glyph advance, not the flat fallback"
        );
    }

    #[test]
    fn helvetica_ascii_word_measures_below_flat_estimate() {
        let flat = approx_text_width(13.0, "mortgage");
        let real = helvetica_advance_width("mortgage", 13.0);
        assert!(real < flat * 0.8);
        assert!(real > 0.0);
    }

    #[test]
    fn helvetica_non_ascii_falls_back_to_flat_estimate() {
        let w = helvetica_advance_width("€", 13.0);
        assert_eq!(w, approx_text_width(13.0, "€"));
    }

    #[test]
    fn inter_base64_decodes_to_the_embedded_ttf() {
        use base64::Engine as _;

        let decoded = STANDARD
            .decode(inter_base64())
            .expect("inter_base64 must be standard-alphabet base64");
        assert_eq!(decoded, INTER_TTF);
    }
}
