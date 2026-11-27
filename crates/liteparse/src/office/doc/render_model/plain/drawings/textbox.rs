//! Join a rectangular textbox to its exact OfficeArt and PlcfSpa source records.
use super::{Reader, Record, anchor, require, textbox_style, unsupported};
use crate::office::doc::{Result, u16_at, u32_at};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct TextboxShape {
    #[serde(skip_serializing_if = "is_body")]
    pub anchor_story: &'static str,
    pub shape_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_shape_type: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_next_shape: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub appearance_approximations: Vec<super::defaults::DefaultEvidence>,
    pub textbox_record_index: usize,
    pub anchor_cp: usize,
    pub rectangle_twips: [i32; 4],
    pub horizontal_origin: anchor::HorizontalOrigin,
    pub vertical_origin: anchor::VerticalOrigin,
    pub behind_text: bool,
    pub locked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub square_wrap: Option<SquareWrap>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_bottom_wrap: Option<TopBottomWrap>,
    pub text_insets_emu: [u32; 4],
    pub fill_rgb: Option<u32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub default_line: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<super::groups::GroupPlacement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_rectangle_twips: Option<[f64; 4]>,
    pub table_range: [usize; 2],
}

/// Source wrapping operands are retained separately from inferred layout units.
#[derive(Debug, Serialize)]
pub struct SquareWrap {
    pub raw_anchor_flags: u16,
    pub side: anchor::WrapSide,
    /// Left, top, right, bottom in the OfficeArt application's coordinate units.
    pub source_distances: [Option<i32>; 4],
}

/// Top/bottom flow retains side bits as raw evidence; they have no layout effect.
#[derive(Debug, Serialize)]
pub struct TopBottomWrap {
    pub raw_anchor_flags: u16,
    /// Left, top, right, bottom in the OfficeArt application's coordinate units.
    pub source_distances: [Option<i32>; 4],
}

fn is_body(story: &&str) -> bool {
    *story == "body"
}

pub(super) struct RawAnchor {
    pub shape_id: u32,
    pub cp: usize,
    pub rectangle: [i32; 4],
    pub flags: u16,
}

pub(super) fn anchors(data: &[u8], body_units: usize) -> Result<Vec<RawAnchor>> {
    if data.is_empty() {
        return Ok(Vec::new());
    }
    if data.len() < 4 || !(data.len() - 4).is_multiple_of(30) {
        return Err(unsupported());
    }
    let count = (data.len() - 4) / 30;
    if count > 4096 {
        return Err(crate::office::doc::Error::Limit("render textbox anchors"));
    }
    let mut output = Vec::with_capacity(count);
    let mut prior = None;
    let mut ids = std::collections::BTreeSet::new();
    for index in 0..count {
        let cp = u32_at(data, index * 4)? as usize;
        let at = (count + 1) * 4 + index * 26;
        let shape_id = u32_at(data, at)?;
        if cp >= body_units || prior.is_some_and(|old| cp <= old) || !ids.insert(shape_id) {
            return Err(unsupported());
        }
        prior = Some(cp);
        output.push(RawAnchor {
            shape_id,
            cp,
            rectangle: [
                u32_at(data, at + 4)? as i32,
                u32_at(data, at + 8)? as i32,
                u32_at(data, at + 12)? as i32,
                u32_at(data, at + 16)? as i32,
            ],
            flags: u16_at(data, at + 20)?,
        });
    }
    if prior.is_some_and(|cp| u32_at(data, count * 4).map_or(true, |end| end as usize <= cp)) {
        return Err(unsupported());
    }
    Ok(output)
}

pub(super) fn has_client_textbox(record: &Record<'_>) -> Result<bool> {
    let mut at = 0usize;
    let mut found = false;
    while at < record.data.len() {
        let kind = u16_at(record.data, at + 2)?;
        let length = u32_at(record.data, at + 4)? as usize;
        at = at
            .checked_add(8)
            .and_then(|at| at.checked_add(length))
            .ok_or_else(unsupported)?;
        if at > record.data.len() {
            return Err(unsupported());
        }
        found |= kind == 0xf00d;
    }
    Ok(found)
}

pub(super) fn decode(reader: &mut Reader, record: Record<'_>, story: u8) -> Result<TextboxShape> {
    decode_inner(reader, record, story, None)
}
pub(super) fn decode_group(
    reader: &mut Reader,
    record: Record<'_>,
    story: u8,
    group: &super::groups::GroupPlacement,
) -> Result<TextboxShape> {
    decode_inner(reader, record, story, Some(group))
}
fn decode_inner(
    reader: &mut Reader,
    record: Record<'_>,
    story: u8,
    group: Option<&super::groups::GroupPlacement>,
) -> Result<TextboxShape> {
    require(&record, 0xf004, 15, Some(0))?;
    if story > 1 || (story == 1 && group.is_some()) {
        return Err(unsupported());
    }
    let mut at = 0;
    let shape = reader.record(record.data, &mut at, record.payload_offset)?;
    require(&shape, 0xf00a, 2, None)?;
    if shape.data.len() != 8
        || u32_at(shape.data, 4)? != if group.is_some() { 0xa02 } else { 0xa00 }
    {
        return Err(unsupported());
    }
    let shape_id = u32_at(shape.data, 0)?;
    if !reader.shape_ids.insert(shape_id) {
        return Err(unsupported());
    }
    let options = reader.record(record.data, &mut at, record.payload_offset)?;
    require(&options, 0xf00b, 3, None)?;
    let mut property_records = vec![options];
    while at < record.data.len() && matches!(u16_at(record.data, at + 2)?, 0xf121 | 0xf122) {
        property_records.push(reader.record(record.data, &mut at, record.payload_offset)?);
    }
    let placement_record = reader.record(record.data, &mut at, record.payload_offset)?;
    let (anchor_index, group_rectangle) = if let Some(group) = group {
        require(&placement_record, 0xf00f, 0, Some(0))?;
        if placement_record.data.len() != 16 {
            return Err(unsupported());
        }
        let rect = std::array::from_fn(|i| {
            i32::from_le_bytes(placement_record.data[i * 4..i * 4 + 4].try_into().unwrap())
        });
        (None, Some(group.rectangle(rect)?))
    } else {
        require(&placement_record, 0xf010, 0, Some(0))?;
        if placement_record.data.len() != 4 {
            return Err(unsupported());
        }
        (Some(u32_at(placement_record.data, 0)? as usize), None)
    };
    let client = reader.record(record.data, &mut at, record.payload_offset)?;
    require(&client, 0xf011, 0, Some(0))?;
    if client.data.len() != 4 {
        return Err(unsupported());
    }
    let text = reader.record(record.data, &mut at, record.payload_offset)?;
    require(&text, 0xf00d, 0, Some(0))?;
    if text.data.len() != 4 || at != record.data.len() {
        return Err(unsupported());
    }
    let appearance =
        super::appearance::resolve(&property_records, &reader.default_text_properties, shape_id)?;
    let style = textbox_style::resolve(&appearance.properties, u32_at(text.data, 0)?)?;
    let (cp, rectangle, flags) = if let Some(group) = group {
        (
            group.anchor_cp,
            group.anchor_rectangle_twips,
            group.raw_anchor_flags,
        )
    } else {
        let index = anchor_index.ok_or_else(unsupported)?;
        let (anchors, used) = if story == 0 {
            (&reader.anchors, &mut reader.used_anchors)
        } else {
            (&reader.header_anchors, &mut reader.used_header_anchors)
        };
        let raw = anchors.get(index).ok_or_else(unsupported)?;
        if raw.shape_id != shape_id || !used.insert(index) {
            return Err(unsupported());
        }
        (raw.cp, raw.rectangle, raw.flags)
    };
    let (frame, square_wrap, top_bottom_wrap) =
        if story == 0 && group.is_none() && (flags >> 5) & 15 == 2 {
            let (frame, side) = anchor::square(rectangle, flags)?;
            (
                frame,
                Some(SquareWrap {
                    raw_anchor_flags: flags,
                    side,
                    source_distances: appearance.wrap_distances,
                }),
                None,
            )
        } else if story == 0 && group.is_none() && (flags >> 5) & 15 == 1 {
            (
                anchor::top_bottom(rectangle, flags)?,
                None,
                Some(TopBottomWrap {
                    raw_anchor_flags: flags,
                    source_distances: appearance.wrap_distances,
                }),
            )
        } else {
            (anchor::overlay(rectangle, flags)?, None, None)
        };
    Ok(TextboxShape {
        anchor_story: if story == 0 { "body" } else { "headers" },
        shape_id,
        source_shape_type: (shape.instance != 202).then_some(shape.instance),
        source_next_shape: appearance.source_next_shape,
        appearance_approximations: appearance.approximations,
        textbox_record_index: style.record_index,
        anchor_cp: cp,
        rectangle_twips: frame.rectangle_twips,
        horizontal_origin: frame.horizontal_origin,
        vertical_origin: frame.vertical_origin,
        behind_text: frame.behind_text,
        locked: frame.locked,
        square_wrap,
        top_bottom_wrap,
        text_insets_emu: style.insets_emu,
        fill_rgb: style.fill_rgb,
        default_line: style.default_line,
        group: group.cloned(),
        group_rectangle_twips: group_rectangle,
        table_range: [
            record.payload_offset - 8,
            record.payload_offset + record.data.len(),
        ],
    })
}

#[cfg(test)]
mod tests;
