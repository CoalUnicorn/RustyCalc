pub fn xml_escape(s: &str, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
}

/// Escape a string for emission inside a PDF literal-string `(...)`.
/// `(`, `)`, `\` get backslash-escaped; bytes outside printable ASCII
/// become `\nnn` octal escapes (which keeps the content stream
/// 7-bit-clean and avoids encoding ambiguity with WinAnsi-mapped bytes).
///
/// Limitation: Helvetica uses WinAnsiEncoding (a Latin-1 superset).
/// Octal-escaping a multi-byte UTF-8 sequence emits each byte separately,
/// which renders as garbage `.notdef` glyphs for any char outside
/// WinAnsi. See "Text encoding limitation" in OUTPUT_REFACTOR_PLAN.md.
pub fn pdf_string_escape(s: &str, out: &mut Vec<u8>) {
    for b in s.bytes() {
        match b {
            b'(' => out.extend_from_slice(b"\\("),
            b')' => out.extend_from_slice(b"\\)"),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b if !(0x20..=0x7E).contains(&b) => {
                out.extend_from_slice(format!("\\{b:03o}").as_bytes());
            }
            b => out.push(b),
        }
    }
}
