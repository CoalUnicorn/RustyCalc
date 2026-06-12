//! Floating Camera widgets: view-only live pictures of a cell range,
//! rendered by iron-canvas-datagrid next to the worksheet canvas.

use serde::{Deserialize, Serialize};

use crate::coord::{CellArea, SheetRange};

/// One camera widget. `pos`/`size` are workspace CSS pixels; `scroll` is
/// the DataGrid viewport anchor (1-based top row / left col), kept here so
/// persistence can restore it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraSpec {
    pub id: u32,
    pub source: SheetRange,
    pub pos: (f64, f64),
    pub size: (f64, f64),
    pub scroll: (i32, i32),
    /// Set on creation; consumed once by lazy init to fit columns to text.
    /// Not persisted — restored cameras load `false`.
    pub autosize: bool,
}

impl CameraSpec {
    pub fn new(id: u32, source: SheetRange) -> Self {
        Self {
            id,
            source,
            pos: (80.0, 80.0),
            size: (320.0, 180.0),
            scroll: (1, 1),
            autosize: true,
        }
    }

    pub fn next_id(existing: &[CameraSpec]) -> u32 {
        existing.iter().map(|c| c.id).max().unwrap_or(0) + 1
    }
}

/// Storage mirror of [`CameraSpec`]: coord types carry no serde, so the
/// range flattens to plain ints here and nowhere else.
#[derive(Serialize, Deserialize)]
pub struct PersistedCamera {
    pub id: u32,
    pub sheet: u32,
    pub r1: i32,
    pub c1: i32,
    pub r2: i32,
    pub c2: i32,
    pub pos: (f64, f64),
    pub size: (f64, f64),
    pub scroll: (i32, i32),
}

impl From<&CameraSpec> for PersistedCamera {
    fn from(s: &CameraSpec) -> Self {
        let a = s.source.area;
        Self {
            id: s.id,
            sheet: s.source.sheet,
            r1: a.r1,
            c1: a.c1,
            r2: a.r2,
            c2: a.c2,
            pos: s.pos,
            size: s.size,
            scroll: s.scroll,
        }
    }
}

impl From<&PersistedCamera> for CameraSpec {
    fn from(p: &PersistedCamera) -> Self {
        Self {
            id: p.id,
            source: SheetRange {
                sheet: p.sheet,
                area: CellArea {
                    r1: p.r1,
                    c1: p.c1,
                    r2: p.r2,
                    c2: p.c2,
                },
            },
            pos: p.pos,
            size: p.size,
            scroll: p.scroll,
            autosize: false,
        }
    }
}

impl PersistedCamera {
    pub fn storage_key(uuid: &str) -> String {
        format!("rustycalc_cameras_{uuid}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coord::SheetRange;

    fn spec(id: u32) -> CameraSpec {
        CameraSpec::new(id, SheetRange::new(0, 1, 1, 4, 3))
    }

    #[test]
    fn next_id_is_max_plus_one() {
        assert_eq!(CameraSpec::next_id(&[]), 1);
        assert_eq!(CameraSpec::next_id(&[spec(1), spec(7), spec(3)]), 8);
    }

    #[test]
    fn persisted_round_trip() {
        let cams = [spec(1), spec(4)];
        let stored: Vec<PersistedCamera> = cams.iter().map(PersistedCamera::from).collect();
        let Ok(json) = serde_json::to_string(&stored) else {
            panic!("serialize failed");
        };
        let back: Vec<PersistedCamera> = match serde_json::from_str(&json) {
            Ok(v) => v,
            Err(e) => panic!("deserialize failed: {e}"),
        };
        let restored: Vec<CameraSpec> = back.iter().map(CameraSpec::from).collect();
        // autosize is not persisted: originals have true, restored have false.
        for (orig, res) in cams.iter().zip(restored.iter()) {
            assert_eq!(res.id, orig.id);
            assert_eq!(res.source, orig.source);
            assert_eq!(res.pos, orig.pos);
            assert_eq!(res.size, orig.size);
            assert_eq!(res.scroll, orig.scroll);
            assert!(!res.autosize);
        }
    }
}
