//! Retain nontext rectangle decoration and its exact body anchor for partial scenes.
use super::*;

pub(super) fn rectangle(reader: &mut Reader, record: Record<'_>, story: u8) -> Result<()> {
    if story != 0 {
        return Err(unsupported());
    }
    require(&record, 0xf004, 15, Some(0))?;
    let mut at = 0;
    let shape = reader.record(record.data, &mut at, record.payload_offset)?;
    require(&shape, 0xf00a, 2, Some(1))?;
    if shape.data.len() != 8 || u32_at(shape.data, 4)? != 0xa00 {
        return Err(unsupported());
    }
    let id = u32_at(shape.data, 0)?;
    let props = reader.record(record.data, &mut at, record.payload_offset)?;
    require(&props, 0xf00b, 3, None)?;
    if props.data.len() != props.instance as usize * 6 {
        return Err(unsupported());
    }
    let mut ids = std::collections::BTreeSet::new();
    for property in props.data.as_chunks::<6>().0 {
        let key = u16_at(property, 0)?;
        let value = u32_at(property, 2)?;
        let valid = match key {
            0x1cb => value <= 0x0132_f540,
            0x1ce => value <= 10,
            0x1d7 => value <= 2,
            0x3bf => true,
            _ => false,
        };
        if !valid || !ids.insert(key) {
            return Err(unsupported());
        }
    }
    let placement = reader.record(record.data, &mut at, record.payload_offset)?;
    require(&placement, 0xf010, 0, Some(0))?;
    let client = reader.record(record.data, &mut at, record.payload_offset)?;
    require(&client, 0xf011, 0, Some(0))?;
    if placement.data.len() != 4 || client.data.len() != 4 || at != record.data.len() {
        return Err(unsupported());
    }
    let index = u32_at(placement.data, 0)? as usize;
    let anchor = reader.anchors.get(index).ok_or_else(unsupported)?;
    if anchor.shape_id != id || !reader.shape_ids.insert(id) || !reader.used_anchors.insert(index) {
        return Err(unsupported());
    }
    reader.omitted_shapes.push(groups::OmittedShape {
        shape_id: id,
        shape_type: 1,
        table_range: [
            record.payload_offset - 8,
            record.payload_offset + record.data.len(),
        ],
        group_anchor_cp: anchor.cp,
        reason: "source-owned nontext rectangle decoration omitted from partial scene",
        source_record_payload: record.data.to_vec(),
        source_rectangle_twips: Some(anchor.rectangle),
        source_anchor_flags: Some(anchor.flags),
    });
    Ok(())
}

#[cfg(test)]
mod tests;
