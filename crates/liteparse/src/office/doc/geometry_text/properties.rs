//! Complex payloads follow the complete fixed property array in property order.
use crate::office::doc::{Error, Result, bytes, u16_at, u32_at};
use serde::Serialize;
#[derive(Debug, Serialize)]
pub struct TextProperty {
    pub property_table_kind: u16,
    pub property_table_range: [usize; 2],
    pub raw_property_id: u16,
    pub payload_table_range: Option<[usize; 2]>,
    pub utf16_units: usize,
    pub text: String,
}
pub(super) fn decode(
    data: &[u8],
    count: usize,
    base: usize,
    kind: u16,
    remaining: &mut usize,
) -> Result<Vec<TextProperty>> {
    let mut tail = count
        .checked_mul(6)
        .ok_or(Error::Limit("geometry text property count"))?;
    bytes(data, 0, tail)?;
    // Other shape properties do not contribute text to this component.
    // Their fixed entries establish absence without interpreting opaque arrays.
    let mut contains_text = false;
    for index in 0..count {
        contains_text |= u16_at(data, index * 6)? & 0x3fff == 0xc0;
    }
    if !contains_text {
        return Ok(Vec::new());
    }

    let mut output = Vec::new();
    for index in 0..count {
        let at = index * 6;
        let raw = u16_at(data, at)?;
        let key = raw & 0x3fff;
        let size = u32_at(data, at + 2)? as usize;
        let payload = if raw & 0x8000 != 0 {
            let value = bytes(data, tail, size)?;
            let begin = tail;
            tail = tail
                .checked_add(size)
                .ok_or(Error::Limit("geometry text payload offset"))?;
            Some((begin, value))
        } else {
            None
        };
        if key != 0xc0 {
            continue;
        }
        let (text, range) = if let Some((start, value)) = payload {
            if value.len() < 2
                || value.len() % 2 != 0
                || value.len() > 65536
                || !value.ends_with(&[0, 0])
            {
                return Err(Error::Invalid("geometry text string extent"));
            }
            *remaining = remaining
                .checked_sub(value.len())
                .ok_or(Error::Limit("geometry text strings"))?;
            let units: Vec<_> = value[..value.len() - 2]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect();
            if units.contains(&0) {
                return Err(Error::Invalid("geometry text embedded terminator"));
            }
            let text =
                String::from_utf16(&units).map_err(|_| Error::Invalid("geometry text UTF16"))?;
            (text, Some([base + start, base + tail]))
        } else {
            if size != 0 {
                return Err(Error::Invalid("geometry text simple value"));
            }
            (String::new(), None)
        };
        output.push(TextProperty {
            property_table_kind: kind,
            property_table_range: [base + at, base + at + 6],
            raw_property_id: raw,
            payload_table_range: range,
            utf16_units: text.encode_utf16().count(),
            text,
        });
    }
    if tail != data.len() {
        return Err(Error::Invalid("geometry text property trailing bytes"));
    }
    Ok(output)
}
