//! Builders for the structural objects (`Catalog`, `Pages`) that every
//! PDF document needs regardless of content. Per-page bodies live in
//! `page`, the content-stream body in `stream`, and the resources +
//! font dict in `font`.

/// `"N 0 R"` — the inline syntax for referencing indirect object `num`.
/// All our objects live at generation 0, so the helper hard-codes it.
pub fn indirect_ref(num: u32) -> String {
    format!("{num} 0 R")
}

/// `/Catalog` body. The `/Pages` reference points to the root `/Pages`
/// tree (object `pages_num`).
pub fn catalog_object(pages_num: u32) -> Vec<u8> {
    format!(
        "<< /Type /Catalog /Pages {pages_ref} >>\n",
        pages_ref = indirect_ref(pages_num)
    )
    .into_bytes()
}

/// `/Pages` body listing a single `/Kids` entry. Multi-page output would
/// take a slice; we ship single-page now and grow when pagination lands.
pub fn pages_object(kid_num: u32) -> Vec<u8> {
    format!(
        "<< /Type /Pages /Kids [{kid_ref}] /Count 1 >>\n",
        kid_ref = indirect_ref(kid_num)
    )
    .into_bytes()
}
