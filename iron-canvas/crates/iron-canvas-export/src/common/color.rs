//! `PaintColor`/CSS-color string → PDF `(r g b)` triple in `0.0..=1.0`.
//!
//! Handles the subset emitted by `CanvasTheme` + cell-style overrides:
//! `#rgb`, `#rrggbb`, `#rrggbbaa` (alpha dropped — PDF rg/RG is opaque),
//! `rgb(r,g,b)`, `rgba(r,g,b,a)`, and the two named colors that show up
//! through `clear_rect` (`white`) and default text (`black`). Malformed
//! or unknown input resolves to black — matches `CssColor::new`'s
//! empty-string fallback.

pub type Rgb = (f64, f64, f64);

const BLACK: Rgb = (0.0, 0.0, 0.0);

pub fn parse_css_color(css: &str) -> Rgb {
    let css = css.trim();
    if let Some(rest) = css.strip_prefix('#') {
        return parse_hex(rest).unwrap_or(BLACK);
    }
    if let Some(inner) = css
        .strip_prefix("rgba(")
        .and_then(|s| s.strip_suffix(')'))
        .or_else(|| css.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')))
    {
        return parse_rgb_tuple(inner).unwrap_or(BLACK);
    }
    match css {
        "white" | "WHITE" | "#fff" | "#FFF" => (1.0, 1.0, 1.0),
        "black" | "BLACK" => BLACK,
        _ => BLACK,
    }
}

fn parse_hex(rest: &str) -> Option<Rgb> {
    match rest.len() {
        // #rgb — each digit expanded to a byte (`0xF` → `0xFF`, `0x3` → `0x33`).
        3 => {
            let r = u8::from_str_radix(rest.get(0..1)?, 16).ok()?;
            let g = u8::from_str_radix(rest.get(1..2)?, 16).ok()?;
            let b = u8::from_str_radix(rest.get(2..3)?, 16).ok()?;
            Some((
                f64::from(r * 17) / 255.0,
                f64::from(g * 17) / 255.0,
                f64::from(b * 17) / 255.0,
            ))
        }
        // #rrggbb and #rrggbbaa — alpha is silently dropped for PDF opaque rg/RG.
        6 | 8 => {
            let r = u8::from_str_radix(rest.get(0..2)?, 16).ok()?;
            let g = u8::from_str_radix(rest.get(2..4)?, 16).ok()?;
            let b = u8::from_str_radix(rest.get(4..6)?, 16).ok()?;
            Some((
                f64::from(r) / 255.0,
                f64::from(g) / 255.0,
                f64::from(b) / 255.0,
            ))
        }
        _ => None,
    }
}

fn parse_rgb_tuple(inner: &str) -> Option<Rgb> {
    let mut parts = inner.split(',').map(str::trim);
    let r = parts.next()?.parse::<f64>().ok()? / 255.0;
    let g = parts.next()?.parse::<f64>().ok()? / 255.0;
    let b = parts.next()?.parse::<f64>().ok()? / 255.0;
    Some((r, g, b))
}
