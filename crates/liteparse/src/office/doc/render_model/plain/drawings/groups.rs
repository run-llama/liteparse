//! Preserve anchored group ownership and transform child rectangles in source order.
use super::{Reader, Record, require, textbox, unsupported};
use crate::office::doc::{Error, Result, u16_at, u32_at};
use serde::Serialize;
pub mod geometry;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GroupPlacement {
    pub anchor_shape_id: u32,
    pub anchor_cp: usize,
    pub raw_anchor_flags: u16,
    pub anchor_rectangle_twips: [i32; 4],
    pub transforms: Vec<geometry::Transform>,
}
impl GroupPlacement {
    pub fn rectangle(&self, child: [i32; 4]) -> Result<[f64; 4]> {
        self.transforms
            .last()
            .ok_or(Error::Invalid("textbox group transform missing"))?
            .apply(child)
    }
}

#[derive(Debug, Serialize)]
pub struct OmittedShape {
    pub shape_id: u32,
    pub shape_type: u16,
    pub table_range: [usize; 2],
    pub group_anchor_cp: usize,
    pub reason: &'static str,
    pub source_record_payload: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_rectangle_twips: Option<[i32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_anchor_flags: Option<u16>,
}

pub(super) fn decode(
    reader: &mut Reader,
    record: Record<'_>,
    story: u8,
    parent: Option<&GroupPlacement>,
) -> Result<()> {
    require(&record, 0xf003, 15, Some(0))?;
    if story != 0 || parent.is_some_and(|p| p.transforms.len() >= 16) {
        return Err(unsupported());
    }
    let mut at = 0;
    let head = reader.record(record.data, &mut at, record.payload_offset)?;
    require(&head, 0xf004, 15, Some(0))?;
    let mut pos = 0;
    let bounds = reader.record(head.data, &mut pos, head.payload_offset)?;
    require(&bounds, 0xf009, 1, Some(0))?;
    if bounds.data.len() != 16 {
        return Err(unsupported());
    }
    let coordinates = std::array::from_fn(|i| {
        i32::from_le_bytes(bounds.data[i * 4..i * 4 + 4].try_into().unwrap())
    });
    let prop = reader.record(head.data, &mut pos, head.payload_offset)?;
    require(&prop, 0xf00a, 2, Some(0))?;
    if prop.data.len() != 8 {
        return Err(unsupported());
    }
    let id = u32_at(prop.data, 0)?;
    let flags = u32_at(prop.data, 4)?;
    if flags != if parent.is_some() { 0x203 } else { 0x201 } || !reader.shape_ids.insert(id) {
        return Err(unsupported());
    }
    let mut child_rect = None;
    let mut anchor_index = None;
    let mut properties_present = false;
    while pos < head.data.len() {
        let item = reader.record(head.data, &mut pos, head.payload_offset)?;
        match item.kind {
            0xf00b | 0xf121 | 0xf122 => {
                if item.version != 3 || item.data.len() < item.instance as usize * 6 {
                    return Err(unsupported());
                }
                properties_present = true;
            }
            0xf010 if parent.is_none() && item.data.len() == 4 && anchor_index.is_none() => {
                anchor_index = Some(u32_at(item.data, 0)? as usize)
            }
            0xf00f if parent.is_some() && item.data.len() == 16 && child_rect.is_none() => {
                child_rect = Some(std::array::from_fn(|i| {
                    i32::from_le_bytes(item.data[i * 4..i * 4 + 4].try_into().unwrap())
                }))
            }
            0xf011 if item.data.len() == 4 => {}
            _ => return Err(unsupported()),
        }
    }
    let (mut placement, frame) = if let Some(parent) = parent {
        (
            parent.clone(),
            parent.rectangle(child_rect.ok_or_else(unsupported)?)?,
        )
    } else {
        let index = anchor_index.ok_or_else(unsupported)?;
        let raw = reader.anchors.get(index).ok_or_else(unsupported)?;
        if raw.shape_id != id || !reader.used_anchors.insert(index) {
            return Err(unsupported());
        }
        super::anchor::overlay(raw.rectangle, raw.flags)?;
        (
            GroupPlacement {
                anchor_shape_id: id,
                anchor_cp: raw.cp,
                raw_anchor_flags: raw.flags,
                anchor_rectangle_twips: raw.rectangle,
                transforms: vec![],
            },
            raw.rectangle.map(f64::from),
        )
    };
    let transform = geometry::Transform {
        group_shape_id: id,
        source_bounds: coordinates,
        parent_frame_twips: frame,
        table_range: [bounds.payload_offset - 8, bounds.payload_offset + 16],
    };
    transform.apply(coordinates)?;
    placement.transforms.push(transform);
    if properties_present {
        reader.group_approximations.push(id);
    }
    while at < record.data.len() {
        let child = reader.record(record.data, &mut at, record.payload_offset)?;
        if child.kind == 0xf003 {
            decode(reader, child, story, Some(&placement))?;
            continue;
        }
        require(&child, 0xf004, 15, Some(0))?;
        let header = u16_at(child.data, 0)?;
        if u16_at(child.data, 2)? != 0xf00a {
            return Err(unsupported());
        }
        if header >> 4 == 202 || textbox::has_client_textbox(&child)? {
            let shape = textbox::decode_group(reader, child, story, &placement)?;
            reader.add_textbox(shape)?;
        } else {
            let range = [
                child.payload_offset - 8,
                child.payload_offset + child.data.len(),
            ];
            let mut offset = 0;
            let identity = reader.record(child.data, &mut offset, child.payload_offset)?;
            require(&identity, 0xf00a, 2, None)?;
            if identity.data.len() != 8 {
                return Err(unsupported());
            }
            let shape_id = u32_at(identity.data, 0)?;
            if !reader.shape_ids.insert(shape_id) {
                return Err(unsupported());
            }
            while offset < child.data.len() {
                let item = reader.record(child.data, &mut offset, child.payload_offset)?;
                if item.kind == 0xf00d {
                    return Err(Error::Unsupported("group child text shape not yet modeled"));
                }
                if matches!(item.kind, 0xf00b | 0xf121 | 0xf122) {
                    if item.version != 3 || item.data.len() < usize::from(item.instance) * 6 {
                        return Err(unsupported());
                    }
                    for i in 0..usize::from(item.instance) {
                        if u16_at(item.data, i * 6)? & 0x3fff == 0x80
                            && u32_at(item.data, i * 6 + 2)? != 0
                        {
                            return Err(Error::Unsupported(
                                "group child text shape not yet modeled",
                            ));
                        }
                    }
                }
            }
            reader.omitted_shapes.push(OmittedShape {shape_id,shape_type:header>>4,table_range:range,group_anchor_cp:placement.anchor_cp,reason:"group child paint is retained as source evidence but omitted from this partial scene", source_record_payload: child.data.to_vec(), source_rectangle_twips: None, source_anchor_flags: None});
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
