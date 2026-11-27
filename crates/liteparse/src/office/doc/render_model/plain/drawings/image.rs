//! Source identity and overlay frames for embedded rectangular raster shapes.
use super::{Reader, Record, anchor, image_properties, require, unsupported};
use crate::office::doc::{Result, u16_at, u32_at};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ImageShape {
    pub shape_id: u32,
    pub anchor_story: &'static str,
    pub anchor_cp: usize,
    pub rectangle_twips: [i32; 4],
    pub horizontal_origin: anchor::HorizontalOrigin,
    pub vertical_origin: anchor::VerticalOrigin,
    pub behind_text: bool,
    pub locked: bool,
    pub properties: image_properties::ImageProperties,
    pub table_range: [usize; 2],
    pub positioning_basis: &'static str,
}

pub(super) fn decode(reader: &mut Reader, record: Record<'_>, story: u8) -> Result<ImageShape> {
    require(&record, 0xf004, 15, Some(0))?;
    let mut at = 0;
    let shape = reader.record(record.data, &mut at, record.payload_offset)?;
    require(&shape, 0xf00a, 2, Some(75))?;
    if shape.data.len() != 8 || u32_at(shape.data, 4)? != 0xa00 {
        return Err(unsupported());
    }
    let shape_id = u32_at(shape.data, 0)?;
    if !reader.shape_ids.insert(shape_id) {
        return Err(unsupported());
    }
    let primary = reader.record(record.data, &mut at, record.payload_offset)?;
    require(&primary, 0xf00b, 3, None)?;
    let tertiary = if at < record.data.len() && u16_at(record.data, at + 2)? == 0xf122 {
        let value = reader.record(record.data, &mut at, record.payload_offset)?;
        require(&value, 0xf122, 3, None)?;
        Some(value)
    } else {
        None
    };
    let properties = image_properties::decode(
        (primary.data, primary.instance as usize),
        tertiary.as_ref().map(|p| (p.data, p.instance as usize)),
    )?;
    if properties.blip_index as usize > reader.store_count {
        return Err(unsupported());
    }
    let client_anchor = reader.record(record.data, &mut at, record.payload_offset)?;
    require(&client_anchor, 0xf010, 0, Some(0))?;
    if client_anchor.data.len() != 4 {
        return Err(unsupported());
    }
    let index = u32_at(client_anchor.data, 0)? as usize;
    let client = reader.record(record.data, &mut at, record.payload_offset)?;
    require(&client, 0xf011, 0, Some(0))?;
    if client.data.len() != 4 || at != record.data.len() {
        return Err(unsupported());
    }
    let raw = match story {
        0 if reader.used_anchors.insert(index) => reader.anchors.get(index),
        1 if reader.used_header_anchors.insert(index) => reader.header_anchors.get(index),
        _ => None,
    }
    .ok_or_else(unsupported)?;
    if raw.shape_id != shape_id {
        return Err(unsupported());
    }
    let frame = anchor::overlay(raw.rectangle, raw.flags)?;
    if properties
        .behind_document
        .is_some_and(|behind| behind != frame.behind_text)
    {
        return Err(unsupported());
    }
    // Saved Word reference enums are qualified only when they agree with Spa origins.
    // This bounded compatibility interpretation retains the raw OfficeArt operands.
    if properties.positioning[1].is_some_and(|v| v != u32::from((raw.flags >> 1) & 3))
        || properties.positioning[3].is_some_and(|v| v != u32::from((raw.flags >> 3) & 3))
        || [properties.positioning[0], properties.positioning[2]]
            .iter()
            .flatten()
            .any(|&v| v > 3)
    {
        return Err(unsupported());
    }
    Ok(ImageShape {
        shape_id,
        anchor_story: if story == 0 { "body" } else { "headers" },
        anchor_cp: raw.cp,
        rectangle_twips: frame.rectangle_twips,
        horizontal_origin: frame.horizontal_origin,
        vertical_origin: frame.vertical_origin,
        behind_text: frame.behind_text,
        locked: frame.locked,
        properties,
        table_range: [
            record.payload_offset - 8,
            record.payload_offset + record.data.len(),
        ],
        positioning_basis: "word_zero_based_reference_compatibility_agrees_with_spa",
    })
}

#[cfg(test)]
mod tests;
