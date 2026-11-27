//! Source-owned, unlinked preset lines with a solid stroke and explicit overlay anchor.
use super::{Reader, Record, anchor, require, unsupported};
use crate::office::doc::{Result, u16_at, u32_at};

pub type LineShape = super::primitive::StrokeFrame;

// MS-ODRAW 2.3.8 specifies black, 9525 EMU and solid as stroke defaults.
// Editing flags do not establish arrowheads; explicit arrowhead properties stay guarded.
pub(super) fn style(properties: &[[u32; 2]]) -> Result<(u32, u32)> {
    let mut rgb = 0;
    let mut width = 9525;
    for &[key, value] in properties {
        match key {
            0x1c0 if value & 0xff000000 == 0 => {
                rgb = ((value & 255) << 16) | (value & 0xff00) | ((value >> 16) & 255);
            }
            0x1cb if value > 0 && value <= 0x0132f540 => width = value,
            0x1ce if value == 0 => {}
            0x1ff => {
                let effective = |bit: u32, default: bool| {
                    if value & (1 << (bit + 16)) != 0 {
                        value & (1 << bit) != 0
                    } else {
                        default
                    }
                };
                if !effective(3, true)
                    || effective(9, false)
                    || effective(6, false) && effective(5, true)
                {
                    return Err(unsupported());
                }
                if value & 0x180 != 0 {
                    return Err(unsupported());
                }
            }
            0x1bf if matches!(value, 0x100000 | 0x110000 | 0x110001) => {}
            0x144 if value == 4 => {}
            0x17f if value == 0x10000 => {}
            0x53f if value == 0x10000 => {}
            _ => return Err(unsupported()),
        }
    }
    Ok((rgb, width))
}

pub(super) fn decode(reader: &mut Reader, record: Record<'_>, story: u8) -> Result<LineShape> {
    decode_preset(reader, record, story, 20, style)
}

type StrokeQualifier = fn(&[[u32; 2]]) -> Result<(u32, u32)>;

pub(super) fn decode_preset(
    reader: &mut Reader,
    record: Record<'_>,
    story: u8,
    preset: u16,
    qualify_style: StrokeQualifier,
) -> Result<LineShape> {
    require(&record, 0xf004, 15, Some(0))?;
    if story != 1 || !reader.document_defaults.is_empty() {
        return Err(unsupported());
    }
    let mut at = 0;
    let fsp = reader.record(record.data, &mut at, record.payload_offset)?;
    require(&fsp, 0xf00a, 2, Some(preset))?;
    if fsp.data.len() != 8 {
        return Err(unsupported());
    }
    let flags = u32_at(fsp.data, 4)?;
    if flags & !0xc0 != 0xa00 {
        return Err(unsupported());
    }
    let shape_id = u32_at(fsp.data, 0)?;
    if !reader.shape_ids.insert(shape_id) {
        return Err(unsupported());
    }
    let mut properties = Vec::new();
    let mut keys = std::collections::BTreeSet::new();
    let mut kinds = std::collections::BTreeSet::new();
    while at < record.data.len() && matches!(u16_at(record.data, at + 2)?, 0xf00b | 0xf121 | 0xf122)
    {
        let p = reader.record(record.data, &mut at, record.payload_offset)?;
        require(&p, p.kind, 3, None)?;
        if !kinds.insert(p.kind) || p.data.len() != usize::from(p.instance) * 6 {
            return Err(unsupported());
        }
        let mut previous = None;
        for raw in p.data.as_chunks::<6>().0 {
            let key = u16_at(raw, 0)?;
            if previous.is_some_and(|old| old >= key) || !keys.insert(key) {
                return Err(unsupported());
            }
            previous = Some(key);
            properties.push([u32::from(key), u32_at(raw, 2)?]);
        }
    }
    let (stroke_rgb, stroke_width_emu) = qualify_style(&properties)?;
    let placement = reader.record(record.data, &mut at, record.payload_offset)?;
    require(&placement, 0xf010, 0, Some(0))?;
    if placement.data.len() != 4 {
        return Err(unsupported());
    }
    let index = u32_at(placement.data, 0)? as usize;
    let client = reader.record(record.data, &mut at, record.payload_offset)?;
    require(&client, 0xf011, 0, Some(0))?;
    if client.data.len() != 4
        || at != record.data.len()
        || !reader.used_header_anchors.insert(index)
    {
        return Err(unsupported());
    }
    let raw = reader.header_anchors.get(index).ok_or_else(unsupported)?;
    if raw.shape_id != shape_id {
        return Err(unsupported());
    }
    let [left, top, right, bottom] = raw.rectangle;
    if right < left || bottom < top || right == left && bottom == top {
        return Err(unsupported());
    }
    // The origin decoder expects an area; a one-twip auxiliary box resolves only
    // the reference system. The original zero-height/width line stays retained.
    let frame = anchor::overlay(
        [
            left,
            top,
            if right == left {
                right.checked_add(1).ok_or_else(unsupported)?
            } else {
                right
            },
            if bottom == top {
                bottom.checked_add(1).ok_or_else(unsupported)?
            } else {
                bottom
            },
        ],
        raw.flags,
    )?;
    Ok(LineShape {
        shape_id,
        anchor_story: "headers",
        anchor_cp: raw.cp,
        rectangle_twips: raw.rectangle,
        horizontal_origin: frame.horizontal_origin,
        vertical_origin: frame.vertical_origin,
        behind_text: frame.behind_text,
        flip_horizontal: flags & 0x40 != 0,
        flip_vertical: flags & 0x80 != 0,
        stroke_rgb,
        stroke_width_emu,
        table_range: [
            record.payload_offset - 8,
            record.payload_offset + record.data.len(),
        ],
        raw_properties: properties,
    })
}

#[cfg(test)]
mod tests;
