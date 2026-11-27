//! Shared source anchor, stroke and provenance for preset primitive geometry.
use super::anchor;
use serde::Serialize;
#[derive(Debug, Serialize)]
pub struct StrokeFrame {
    pub shape_id: u32,
    pub anchor_story: &'static str,
    pub anchor_cp: usize,
    pub rectangle_twips: [i32; 4],
    pub horizontal_origin: anchor::HorizontalOrigin,
    pub vertical_origin: anchor::VerticalOrigin,
    pub behind_text: bool,
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
    pub stroke_rgb: u32,
    pub stroke_width_emu: u32,
    pub table_range: [usize; 2],
    pub raw_properties: Vec<[u32; 2]>,
}

impl StrokeFrame {
    /// Local endpoints retain source flips independently of the paginated origin.
    pub fn endpoints_pt(&self) -> [[f64; 2]; 2] {
        let w =
            (i64::from(self.rectangle_twips[2]) - i64::from(self.rectangle_twips[0])) as f64 / 20.0;
        let h =
            (i64::from(self.rectangle_twips[3]) - i64::from(self.rectangle_twips[1])) as f64 / 20.0;
        [
            [
                if self.flip_horizontal { w } else { 0.0 },
                if self.flip_vertical { h } else { 0.0 },
            ],
            [
                if self.flip_horizontal { 0.0 } else { w },
                if self.flip_vertical { 0.0 } else { h },
            ],
        ]
    }
}
