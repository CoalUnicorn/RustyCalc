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

use std::sync::OnceLock;

use iron_canvas_core::painter::approx_text_width;

//  Inter (SVG)

/// Inter Regular (v13, Latin subset) — see `assets/PROVENANCE.md` for
/// source, submodule commit, and license (`assets/OFL.txt`).
const INTER_TTF: &[u8] = include_bytes!("../../assets/inter-regular.ttf");

fn inter_face() -> &'static ttf_parser::Face<'static> {
    static FACE: OnceLock<ttf_parser::Face<'static>> = OnceLock::new();
    FACE.get_or_init(|| {
        ttf_parser::Face::parse(INTER_TTF, 0).expect("bundled Inter TTF must parse")
    })
}

/// Sum of real Inter glyph advances for `text` at `size_px`. Characters with
/// no glyph in the embedded subset fall back to the flat estimate for that
/// character alone.
pub fn inter_advance_width(text: &str, size_px: f64) -> f64 {
    let face = inter_face();
    let units_per_em = f64::from(face.units_per_em());
    text.chars()
        .map(|c| {
            face.glyph_index(c)
                .and_then(|id| face.glyph_hor_advance(id))
                .map(|adv| f64::from(adv) / units_per_em * size_px)
                .unwrap_or_else(|| approx_text_width(size_px, &c.to_string()))
        })
        .sum()
}

/// Base64 (standard alphabet, padded) of the embedded Inter TTF, for the SVG
/// `@font-face` data URI. Hand-rolled instead of pulling in a `base64` crate
/// for one call site — the workspace has none today.
pub fn inter_base64() -> &'static str {
    static ENCODED: OnceLock<String> = OnceLock::new();
    ENCODED.get_or_init(|| encode_base64(INTER_TTF))
}

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn encode_base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        out.push(BASE64_ALPHABET[(n >> 18 & 0x3F) as usize] as char);
        out.push(BASE64_ALPHABET[(n >> 12 & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 {
            BASE64_ALPHABET[(n >> 6 & 0x3F) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            BASE64_ALPHABET[(n & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
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
    text.chars()
        .map(|c| {
            let idx = c as u32;
            if (0x20..=0x7E).contains(&idx) {
                f64::from(HELVETICA_ASCII_WIDTHS[(idx - 0x20) as usize]) / 1000.0 * size_px
            } else {
                approx_text_width(size_px, &c.to_string())
            }
        })
        .sum()
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
    fn base64_round_trips_known_bytes() {
        // RFC 4648 test vectors.
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
        assert_eq!(encode_base64(b"foob"), "Zm9vYg==");
        assert_eq!(encode_base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode_base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn inter_base64_is_nonempty_and_stable() {
        let a = inter_base64();
        let b = inter_base64();
        assert!(!a.is_empty());
        assert_eq!(a, b, "OnceLock must return the same encoded string");
    }
}
