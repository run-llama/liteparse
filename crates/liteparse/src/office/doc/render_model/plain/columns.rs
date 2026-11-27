//! Equal-width source column geometry, before shared layout applies minimum widths.
use crate::office::doc::{Error, Result, render_model::sections::SectionGeometry};

pub fn width_twips(section: &SectionGeometry) -> Result<f64> {
    let left = section
        .left_margin_twips
        .ok_or(Error::Unsupported("column left margin"))?
        .value;
    let right = section
        .right_margin_twips
        .ok_or(Error::Unsupported("column right margin"))?
        .value;
    let body = f64::from(section.page_width_twips.value) - f64::from(left) - f64::from(right);
    let count = section.column_count.value;
    if count == 1 {
        return Ok(body);
    }
    if body < 720.0 || !(2..=44).contains(&count) || !section.evenly_spaced_columns.value {
        return Err(Error::Unsupported("render unequal column geometry"));
    }
    let gap = section
        .column_spacing_twips
        .ok_or(Error::Unsupported("render column spacing default"))?
        .value;
    let width = (body - f64::from(count - 1) * f64::from(gap)) / f64::from(count);
    if !width.is_finite() || width < 180.0 {
        return Err(Error::Unsupported(
            "render column width below shared minimum",
        ));
    }
    Ok(width)
}
