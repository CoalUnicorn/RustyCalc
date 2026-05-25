//! Commit 2 smoke test: hand-build the minimum spec-conformant PDF
//! (Catalog → Pages → Page → Contents + Resources) and parse it back
//! to prove the `PdfDocument` writer emits a valid header, body, xref
//! table, and trailer.
//!
//! End-to-end painter→PDF verification lives in `pdf_painter_smoke`
//! (Commit 3); this test only covers the document writer.

#![cfg(feature = "pdf")]

use iron_canvas_export::pdf::doc::{font, object, page, ContentStream, PdfDocument};
use iron_canvas_export::pdf::PdfDocument as ReexportedDoc;

const PAGE_W: u32 = 200;
const PAGE_H: u32 = 100;

fn build_minimal_doc() -> Vec<u8> {
    // Object numbering by convention: 1=Catalog, 2=Pages, 3=Page,
    // 4=Contents, 5=Resources. Matches the table in OUTPUT_REFACTOR_PLAN.md.
    let mut doc = PdfDocument::new();
    doc.add_object(1, 0, object::catalog_object(2));
    doc.add_object(2, 0, object::pages_object(3));
    doc.add_object(3, 0, page::page_object(2, 4, 5, PAGE_W, PAGE_H));

    let mut stream = ContentStream::new();
    // A no-op graphics-state save/restore — proves a non-empty stream
    // round-trips without needing the real painter.
    stream.write(b"q\nQ\n");
    doc.add_object(4, 0, stream.into_object());

    doc.add_object(5, 0, font::resources_object_with_helvetica());
    doc.finish()
}

#[test]
fn reexport_points_at_doc_module() {
    // `iron_canvas_export::pdf::PdfDocument` must be the same type as the one in
    // `iron_canvas_export::pdf::doc` — guards the convenience re-export.
    let _: ReexportedDoc = PdfDocument::new();
}

#[test]
fn header_announces_pdf_1_7_with_binary_marker() {
    let bytes = build_minimal_doc();
    assert!(
        bytes.starts_with(b"%PDF-1.7\n"),
        "missing PDF 1.7 header — first 16 bytes: {:?}",
        &bytes[..bytes.len().min(16)]
    );
    // The 4-byte binary marker after the version comment is what tells
    // transports (mail, git diff, `file`) to treat the file as binary.
    assert!(
        bytes[9..].starts_with(b"%\xE2\xE3\xCF\xD3\n"),
        "missing 4-byte binary marker comment after header"
    );
}

#[test]
fn trailer_ends_with_eof_marker() {
    let bytes = build_minimal_doc();
    assert!(
        bytes.ends_with(b"%%EOF"),
        "missing %%EOF terminator (last 16 bytes: {:?})",
        &bytes[bytes.len().saturating_sub(16)..]
    );
}

#[test]
fn xref_offsets_land_on_obj_lines() {
    let bytes = build_minimal_doc();

    // 1. Find `startxref\n<offset>\n%%EOF` at the tail.
    let Some(startxref_pos) = find_subslice(&bytes, b"startxref\n") else {
        panic!("startxref marker not found");
    };
    let after_startxref = startxref_pos + b"startxref\n".len();
    let Some(eof_pos) = find_subslice(&bytes[after_startxref..], b"\n%%EOF") else {
        panic!("trailing newline before %%EOF not found");
    };
    let Ok(xref_offset_str) =
        std::str::from_utf8(&bytes[after_startxref..after_startxref + eof_pos])
    else {
        panic!("xref offset must be ASCII");
    };
    let Ok(xref_offset) = xref_offset_str.parse::<usize>() else {
        panic!("startxref payload not a usize: {xref_offset_str:?}");
    };

    // 2. xref table starts with `xref\n0 N\n` — find N (the total count).
    assert_eq!(
        &bytes[xref_offset..xref_offset + 5],
        b"xref\n",
        "startxref offset does not point at `xref\\n`"
    );
    let header_start = xref_offset + 5;
    let Some(header_nl) = find_subslice(&bytes[header_start..], b"\n") else {
        panic!("xref subsection header missing newline");
    };
    let Ok(header_str) = std::str::from_utf8(&bytes[header_start..header_start + header_nl])
    else {
        panic!("xref header must be ASCII");
    };
    let Some((first_str, count_str)) = header_str.split_once(' ') else {
        panic!("xref header not `first count`: {header_str:?}");
    };
    assert_eq!(first_str, "0", "first xref entry must be object 0");
    let Ok(count) = count_str.parse::<usize>() else {
        panic!("xref count not a usize: {count_str:?}");
    };
    assert_eq!(
        count, 6,
        "expected 6 xref entries (object 0 + 5 indirect objects)"
    );

    // 3. Each entry is exactly 20 bytes: `oooooooooo ggggg n \n`.
    let entries_start = header_start + header_nl + 1;
    // Entry 0 is the free-list head — generation 65535, free.
    assert_eq!(
        &bytes[entries_start..entries_start + 20],
        b"0000000000 65535 f \n",
        "first xref entry must be the free-list head"
    );

    // Entries 1..count must each point at a `N 0 obj\n` line.
    for obj_num in 1..count {
        let entry_off = entries_start + obj_num * 20;
        let entry = &bytes[entry_off..entry_off + 20];
        let Ok(offset_str) = std::str::from_utf8(&entry[..10]) else {
            panic!("xref offset must be ASCII");
        };
        let Ok(obj_offset) = offset_str.parse::<usize>() else {
            panic!("xref entry {obj_num} offset not a usize: {offset_str:?}");
        };
        let expected = format!("{obj_num} 0 obj\n");
        let landed = &bytes[obj_offset..obj_offset + expected.len()];
        let landed_str = std::str::from_utf8(landed).unwrap_or("<non-utf8>");
        assert_eq!(
            landed,
            expected.as_bytes(),
            "xref entry {obj_num} points to offset {obj_offset}, expected `{expected}` but found `{landed_str}`",
        );
    }
}

#[test]
fn content_stream_carries_declared_length() {
    let mut stream = ContentStream::new();
    stream.write(b"q\n");
    stream.write(b"Q\n");
    assert_eq!(stream.len(), 4);
    let obj = stream.into_object();
    let Ok(obj_str) = std::str::from_utf8(&obj) else {
        panic!("stream object must be ASCII");
    };
    assert!(
        obj_str.starts_with("<< /Length 4 >>\nstream\n"),
        "stream object missing declared /Length 4 prefix: {obj_str:?}"
    );
    assert!(
        obj_str.ends_with("\nendstream\n"),
        "stream object missing endstream terminator: {obj_str:?}"
    );
}

#[test]
fn indirect_ref_is_n_gen_r() {
    assert_eq!(object::indirect_ref(1), "1 0 R");
    assert_eq!(object::indirect_ref(42), "42 0 R");
}

/// Linear subslice search — no `windows().position()` dependency, no
/// `expect()` on absence. Returns the start index of `needle` in
/// `haystack`, or `None` if absent.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}
