//! Content-stream builder.
//!
//! The `PdfPainter` (Commit 3) accumulates graphics ops here; this
//! commit only needs the builder shape so `pdf_surface_smoke` can
//! produce a valid (possibly empty) content-stream object.

/// Append-only buffer of raw PDF content-stream ops. Whitespace and
/// newlines between ops are the caller's responsibility — graphics ops
/// are space-delimited within a line, operator boundaries are
/// newline-delimited by convention.
pub struct ContentStream {
    body: Vec<u8>,
}

impl Default for ContentStream {
    fn default() -> Self {
        Self::new()
    }
}

impl ContentStream {
    pub fn new() -> Self {
        Self { body: Vec::new() }
    }

    /// Append raw bytes (one or more ops). Caller decides newline
    /// placement — most ops end with `\n` for human-readable output.
    pub fn write(&mut self, bytes: &[u8]) {
        self.body.extend_from_slice(bytes);
    }

    pub fn len(&self) -> usize {
        self.body.len()
    }

    pub fn is_empty(&self) -> bool {
        self.body.is_empty()
    }

    /// Borrow the raw stream bytes — used by `PdfPainter` snapshots in
    /// tests and by `PdfSurface::finish` when assembling the page.
    pub fn bytes(&self) -> &[u8] {
        &self.body
    }

    /// Serialise as a stream object body: `<< /Length N >>\nstream\n…\nendstream`.
    /// Hand to `PdfDocument::add_object` as the `bytes` payload.
    pub fn into_object(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.body.len() + 32);
        out.extend_from_slice(format!("<< /Length {} >>\nstream\n", self.body.len()).as_bytes());
        out.extend_from_slice(&self.body);
        if !self.body.ends_with(b"\n") {
            out.push(b'\n');
        }
        out.extend_from_slice(b"endstream\n");
        out
    }
}
