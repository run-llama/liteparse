//! Affine placement from a group's local coordinate space into its parent frame.
use crate::office::doc::{Error, Result};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Transform {
    pub group_shape_id: u32,
    pub source_bounds: [i32; 4],
    pub parent_frame_twips: [f64; 4],
    pub table_range: [usize; 2],
}
impl Transform {
    pub fn apply(&self, child: [i32; 4]) -> Result<[f64; 4]> {
        let [l, t, r, b] = self.source_bounds.map(f64::from);
        let [x, y, right, bottom] = self.parent_frame_twips;
        if r <= l
            || b <= t
            || right <= x
            || bottom <= y
            || self.parent_frame_twips.iter().any(|v| !v.is_finite())
        {
            return Err(Error::Invalid("textbox group coordinate extent"));
        }
        let xscale = (right - x) / (r - l);
        let yscale = (bottom - y) / (b - t);
        let out = [
            x + (f64::from(child[0]) - l) * xscale,
            y + (f64::from(child[1]) - t) * yscale,
            x + (f64::from(child[2]) - l) * xscale,
            y + (f64::from(child[3]) - t) * yscale,
        ];
        if out
            .iter()
            .any(|v| !v.is_finite() || v.abs() > f64::from(i32::MAX))
        {
            return Err(Error::Limit("textbox group transformed coordinate"));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
