//! Buffered two-pass PDF document builder.
//!
//! Callers register indirect objects in allocation order (1, 2, 3, ...);
//! `finish()` emits the PDF header, body with tracked byte offsets, the
//! cross-reference table, and the trailer in one pass. The two-pass
//! shape is forced by PDF's xref requirement: every entry is the byte
//! offset of its object, which is only known once every preceding
//! object has been serialised.

pub mod font;
pub mod object;
pub mod page;
pub mod stream;

pub use object::indirect_ref;
pub use stream::ContentStream;

/// PDF 1.7 magic + a 4-byte binary marker so naive `file`/transport
/// heuristics tag the output as binary rather than text.
const PDF_HEADER: &[u8] = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n";

pub struct PdfDocument {
    /// `(obj_num, gen_num, raw_bytes)`. Generation is always 0 for our
    /// single-version output. `raw_bytes` is the object body — what
    /// would sit between `N 0 obj\n` and `\nendobj\n`.
    objects: Vec<(u32, u16, Vec<u8>)>,
}

impl Default for PdfDocument {
    fn default() -> Self {
        Self::new()
    }
}

impl PdfDocument {
    pub fn new() -> Self {
        Self {
            objects: Vec::new(),
        }
    }

    pub fn add_object(&mut self, num: u32, r#gen: u16, bytes: Vec<u8>) {
        self.objects.push((num, r#gen, bytes));
    }

    pub fn finish(self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(PDF_HEADER);

        let n = self.objects.len();
        let mut offsets: Vec<u64> = Vec::with_capacity(n);
        for (num, r#gen, bytes) in &self.objects {
            offsets.push(out.len() as u64);
            out.extend_from_slice(format!("{num} {gen} obj\n").as_bytes());
            out.extend_from_slice(bytes);
            if !bytes.ends_with(b"\n") {
                out.push(b'\n');
            }
            out.extend_from_slice(b"endobj\n");
        }

        let xref_offset = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n", n + 1).as_bytes());
        // Object 0 is the head of the free list, generation 65535 by spec.
        out.extend_from_slice(b"0000000000 65535 f \n");
        for offset in &offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }

        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\n", n + 1).as_bytes());
        out.extend_from_slice(format!("startxref\n{xref_offset}\n%%EOF").as_bytes());
        out
    }
}
