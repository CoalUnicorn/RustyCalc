//! `/Page` object body builder.

use crate::pdf::doc::object::indirect_ref;

/// `/Page` body. `MediaBox` is in PDF user-space units (1/72 inch),
/// which we map 1:1 from CSS pixels — see the DPI note in
/// `OUTPUT_REFACTOR_PLAN.md`.
pub fn page_object(
    parent_num: u32,
    contents_num: u32,
    resources_num: u32,
    width: u32,
    height: u32,
) -> Vec<u8> {
    format!(
        "<< /Type /Page /Parent {parent} /MediaBox [0 0 {width} {height}] \
         /Contents {contents} /Resources {resources} >>\n",
        parent = indirect_ref(parent_num),
        contents = indirect_ref(contents_num),
        resources = indirect_ref(resources_num),
    )
    .into_bytes()
}
