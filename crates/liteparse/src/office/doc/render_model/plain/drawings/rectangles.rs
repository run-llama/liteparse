//! Solid preset rectangles with source-owned header anchors and no attached text.
use super::{Reader, Record, anchor, lines, primitive::StrokeFrame, unsupported};
use crate::office::doc::Result;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct RectangleShape {
    pub frame: StrokeFrame,
    pub fill_rgb: u32,
    pub positioning_basis: &'static str,
}

fn style(properties: &[[u32; 2]]) -> Result<(u32, u32)> {
    let mut stroke = Vec::new();
    for &[key, value] in properties {
        match key {
            // Solid RGB fill and an explicitly disabled shadow retain exact paint.
            0x181 if value & 0xff000000 == 0 => {}
            0x180 if value == 0 => {}
            0x1bf if value == 0x100010 => {}
            0x23f if value == 0x20000 => {}
            // Retain the Word zero-based compatibility interpretation also used by images.
            // The FSPA frame remains the placement authority and the result stays partial.
            0x390 | 0x392 if value == 1 => {}
            0x3bf if value == 0x80000000 => {}
            0x1c0 | 0x1cb | 0x1ce | 0x1ff => stroke.push([key, value]),
            _ => return Err(unsupported()),
        }
    }
    lines::style(&stroke)
}

pub(super) fn decode(reader: &mut Reader, record: Record<'_>, story: u8) -> Result<RectangleShape> {
    let frame = lines::decode_preset(reader, record, story, 1, style)?;
    let [left, top, right, bottom] = frame.rectangle_twips;
    if right <= left || bottom <= top {
        return Err(unsupported());
    }
    for &[key, _] in &frame.raw_properties {
        if key == 0x390 && frame.horizontal_origin != anchor::HorizontalOrigin::Page
            || key == 0x392 && frame.vertical_origin != anchor::VerticalOrigin::Page
        {
            return Err(unsupported());
        }
    }
    let color = frame
        .raw_properties
        .iter()
        .find(|p| p[0] == 0x181)
        .map_or(0xffffff, |p| p[1]);
    let fill_rgb = ((color & 255) << 16) | (color & 0xff00) | ((color >> 16) & 255);
    Ok(RectangleShape {
        frame,
        fill_rgb,
        positioning_basis: "word_zero_based_reference_compatibility_agrees_with_spa",
    })
}

#[cfg(test)]
mod tests;
