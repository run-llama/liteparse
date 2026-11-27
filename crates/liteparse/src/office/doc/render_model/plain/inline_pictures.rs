//! Inline raster occurrences retain source controls, PICF dimensions and encoded assets.
use super::drawings::image_properties::{self, ImageProperties};
use crate::office::doc::{
    Error, Result, bytes, compound::Compound, pictures::ImageAsset, u16_at, u32_at,
};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Serialize)]
pub struct InlinePicture {
    pub source_cp: usize,
    pub asset_index: usize,
    pub size_pt: [f64; 2],
    pub shape_id: u32,
    pub picf_range: [usize; 2],
    pub picf_header: Vec<u8>,
    pub properties: ImageProperties,
    pub raw_modifiers: Vec<crate::office::doc::formatting::PropertyModifier>,
    pub shape_record: Vec<u8>,
    pub client_anchor: Option<u32>,
    pub shape_offset: usize,
}
#[derive(Debug, Serialize)]
pub struct InlinePictures {
    pub references: Vec<InlinePicture>,
    pub assets: Vec<ImageAsset>,
}

impl InlinePictures {
    /// Recheck mutable model evidence before a renderer indexes or paints it.
    pub fn validate(&self) -> Result<()> {
        if self.references.len() > 4096 || self.assets.len() > self.references.len() {
            return Err(Error::Limit("inline picture model"));
        }
        let mut encoded_bytes = 0usize;
        for asset in &self.assets {
            encoded_bytes = encoded_bytes
                .checked_add(asset.bytes().len())
                .filter(|n| *n <= 32 * 1024 * 1024)
                .ok_or(Error::Limit("inline picture encoded bytes"))?;
        }
        let mut previous = None;
        let mut used = std::collections::BTreeSet::new();
        let mut total = 0usize;
        for r in &self.references {
            total = total
                .checked_add(r.shape_record.len())
                .and_then(|n| n.checked_add(r.picf_header.len()))
                .filter(|n| *n <= 8 * 1024 * 1024)
                .ok_or(Error::Limit("inline picture model evidence"))?;
            if previous.is_some_and(|cp| cp >= r.source_cp) || r.source_cp >= 1024 * 1024 {
                return Err(Error::Invalid("inline picture model CP"));
            }
            previous = Some(r.source_cp);
            if r.picf_range[0] > u32::MAX as usize || r.raw_modifiers.len() > 4096 {
                return Err(unsupported());
            }
            for modifier in &r.raw_modifiers {
                total = total
                    .checked_add(modifier.operand.len())
                    .filter(|n| *n <= 8 * 1024 * 1024)
                    .ok_or(Error::Limit("inline picture model evidence"))?;
            }
            let length = r.picf_range[1]
                .checked_sub(r.picf_range[0])
                .ok_or_else(unsupported)?;
            if !(68..=32 * 1024 * 1024).contains(&length)
                || u32_at(&r.picf_header, 0)? as usize != length
                || dimensions(&r.picf_header)? != r.size_pt
                || r.shape_offset < r.picf_range[0] + 68
            {
                return Err(unsupported());
            }
            match u16_at(&r.picf_header, 6)? {
                100 if r.shape_offset == r.picf_range[0] + 68 => {}
                102 if (69..=324).contains(&(r.shape_offset - r.picf_range[0])) => {}
                _ => return Err(unsupported()),
            }
            let (id, properties, raw, anchor) =
                appearance(&r.shape_record, 0, r.shape_record.len())?;
            if id != r.shape_id
                || properties != r.properties
                || raw.len() != r.shape_record.len()
                || anchor != r.client_anchor
            {
                return Err(unsupported());
            }
            let after_shape = r
                .shape_offset
                .checked_add(raw.len())
                .ok_or_else(unsupported)?;
            let asset = self.assets.get(r.asset_index).ok_or_else(unsupported)?;
            validate_asset(asset, after_shape, r.picf_range[1])?;
            let last = |sprm| r.raw_modifiers.iter().rev().find(|p| p.sprm == sprm);
            if last(0x0855).map(|p| p.operand.as_slice()) != Some(&[1][..])
                || last(0x0806).is_some_and(|p| p.operand.as_slice() != [0])
                || last(0x6a03)
                    .filter(|p| p.operand.len() == 4)
                    .map(|p| u32_at(&p.operand, 0))
                    .transpose()?
                    .map(|p| p as usize)
                    != Some(r.picf_range[0])
            {
                return Err(unsupported());
            }
            used.insert(r.asset_index);
        }
        if used.len() != self.assets.len() {
            return Err(unsupported());
        }
        Ok(())
    }
}

fn unsupported() -> Error {
    Error::Unsupported("render inline picture ownership or appearance")
}

// PICMID dimensions precede scaling; its reserved fields remain in the raw header.
// https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/388c0671-fd3b-4597-813a-1c67fea7dc46
fn dimensions(header: &[u8]) -> Result<[f64; 2]> {
    if header.len() != 68 || u16_at(header, 4)? != 68 {
        return Err(Error::Invalid("inline PICF header"));
    }
    if header[46..62].iter().any(|b| *b != 0) {
        return Err(Error::Unsupported("inline picture border"));
    }
    let mut size = [0.; 2];
    for axis in 0..2 {
        let initial = i16::from_le_bytes([header[28 + axis * 2], header[29 + axis * 2]]);
        let scale = u16_at(header, 32 + axis * 2)?;
        let twips = f64::from(initial) * f64::from(scale) / 1000.;
        if initial <= 0 || !(15.0..=31680.0).contains(&twips) {
            return Err(Error::Invalid("inline picture dimensions"));
        }
        size[axis] = twips / 20.;
    }
    Ok(size)
}

struct Record<'a> {
    kind: u16,
    version: u16,
    count: usize,
    data: &'a [u8],
    end: usize,
}
fn record(data: &[u8], at: usize, end: usize) -> Result<Record<'_>> {
    let h = bytes(data, at, 8)?;
    let size = u32_at(h, 4)? as usize;
    let next = at
        .checked_add(8)
        .and_then(|v| v.checked_add(size))
        .filter(|v| *v <= end)
        .ok_or(Error::Invalid("inline picture record bounds"))?;
    let info = u16_at(h, 0)?;
    Ok(Record {
        kind: u16_at(h, 2)?,
        version: info & 15,
        count: usize::from(info >> 4),
        data: bytes(data, at + 8, size)?,
        end: next,
    })
}

// Inline BLIPs follow property order; their stored index and flag bits are ignored.
// https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-odraw/d634b55c-9916-4d93-a270-c109f14ed794
fn inline_options(data: &[u8], count: usize) -> Result<Vec<u8>> {
    if count > 4096 || data.len() > 65_536 {
        return Err(Error::Limit("inline picture properties"));
    }
    bytes(data, 0, count * 6)?;
    let mut normalized = data.to_vec();
    for index in 0..count {
        let offset = index * 6;
        if u16_at(data, offset)? & 0x3fff == 0x104 {
            normalized[offset..offset + 2].copy_from_slice(&0x4104u16.to_le_bytes());
            normalized[offset + 2..offset + 6].copy_from_slice(&1u32.to_le_bytes());
        }
    }
    Ok(normalized)
}

fn appearance(
    data: &[u8],
    start: usize,
    end: usize,
) -> Result<(u32, ImageProperties, Vec<u8>, Option<u32>)> {
    let shape = record(data, start, end)?;
    if shape.kind != 0xf004 || shape.version != 15 || shape.count != 0 {
        return Err(unsupported());
    }
    let identity = record(data, start + 8, shape.end)?;
    if identity.kind != 0xf00a
        || identity.version != 2
        || identity.count != 75
        || identity.data.len() != 8
        || !matches!(u32_at(identity.data, 4)? & 0xfff, 0x800 | 0xa00)
    {
        return Err(unsupported());
    }
    let id = u32_at(identity.data, 0)?;
    let mut primary = None;
    let mut tertiary = None;
    let mut anchor = None;
    let mut at = identity.end;
    while at < shape.end {
        let r = record(data, at, shape.end)?;
        match r.kind {
            0xf00b if r.version == 3 && primary.is_none() => primary = Some((r.data, r.count)),
            0xf122 if r.version == 3 && tertiary.is_none() => tertiary = Some((r.data, r.count)),
            0xf010 if r.version == 0 && r.count == 0 && r.data.len() == 4 && anchor.is_none() => {
                anchor = Some(u32_at(r.data, 0)?)
            }
            _ => return Err(unsupported()),
        }
        at = r.end;
    }
    if (u32_at(identity.data, 4)? & 0x200 != 0) != anchor.is_some() {
        return Err(unsupported());
    }
    let (primary_data, primary_count) = primary.ok_or_else(unsupported)?;
    let primary_data = inline_options(primary_data, primary_count)?;
    let tertiary_data = tertiary
        .map(|(data, count)| inline_options(data, count))
        .transpose()?;
    let properties = image_properties::decode_inline(
        (&primary_data, primary_count),
        tertiary_data
            .as_ref()
            .zip(tertiary)
            .map(|(data, (_, count))| (data.as_slice(), count)),
    )?;
    if properties.blip_index != 1
        || properties.behind_document == Some(true)
        || properties.positioning.iter().any(Option::is_some)
        || properties.wrap_polygon_points.is_some()
    {
        return Err(unsupported());
    }
    Ok((id, properties, data[start..shape.end].to_vec(), anchor))
}

fn validate_payload(
    source: &crate::office::doc::pictures::PayloadSource,
    byte_length: usize,
    actual_length: usize,
) -> Result<()> {
    if source.compression != "none"
        || source.payload_range[1].checked_sub(source.payload_range[0]) != Some(byte_length)
        || byte_length != actual_length
    {
        return Err(unsupported());
    }
    Ok(())
}

// Metafile bytes were bounded and decompressed by the binary BLIP reader.
// Compression describes the source payload; byte_length describes decoded bytes.
fn metafile_signature(format: &str, bytes: &[u8]) -> bool {
    match format {
        "emf" => bytes.len() >= 44 && bytes[..4] == [1, 0, 0, 0] && &bytes[40..44] == b" EMF",
        "wmf" => {
            bytes.starts_with(&[0xd7, 0xcd, 0xc6, 0x9a])
                || bytes.len() >= 4
                    && matches!(&bytes[..2], [1, 0] | [2, 0])
                    && bytes[2..4] == [9, 0]
        }
        _ => false,
    }
}

// Every occurrence checks its own PICF bounds, including references sharing encoded bytes.
fn validate_asset(asset: &ImageAsset, after_shape: usize, picf_end: usize) -> Result<()> {
    let source = &asset.source;
    if matches!(asset.format, "wmf" | "emf") {
        let encoded = source.payload_range[1].checked_sub(source.payload_range[0]);
        if !matches!(source.compression, "none" | "zlib")
            || encoded.is_none_or(|n| n == 0 || n > 32 * 1024 * 1024)
            || asset.byte_length != asset.bytes().len()
            || asset.byte_length > 32 * 1024 * 1024
            || source.compression == "none" && encoded != Some(asset.byte_length)
            || !metafile_signature(asset.format, asset.bytes())
        {
            return Err(unsupported());
        }
    } else {
        validate_payload(source, asset.byte_length, asset.bytes().len())?;
    }
    if !matches!(asset.format, "png" | "jpeg" | "wmf" | "emf")
        || source.stream != "Data"
        || source.record_range[0] < after_shape
        || source.record_range[1] != picf_end
        || source.payload_range[0] < source.record_range[0]
        || source.payload_range[0] >= source.payload_range[1]
        || source.payload_range[1] != picf_end
        || asset.bytes().len() != asset.byte_length
    {
        return Err(unsupported());
    }
    Ok(())
}

// Picture inspection also reports floating U+0008 drawings with role "picture".
// Only raw U+0001 positions select the Data-stream PICF decoder.
fn source_controls(
    story: &str,
    start: usize,
    text: &str,
    headers: &std::collections::BTreeSet<usize>,
) -> Vec<usize> {
    text.encode_utf16()
        .enumerate()
        .filter_map(|(offset, unit)| {
            let cp = start + offset;
            (unit == 1 && (story == "body" || story == "headers" && headers.contains(&cp)))
                .then_some(cp)
        })
        .collect()
}

/// Select body occurrences whose picture, direct special marker and embedded payload agree.
/// Field data without a qualified raster remains subject to the caller's control guards.
pub fn inspect(data: &[u8]) -> Result<Option<InlinePictures>> {
    inspect_with_header_controls(data, &Default::default())
}

/// Header callers supply exact U+0001 controls from source-owned formatted slots.
pub fn inspect_with_header_controls(
    data: &[u8],
    header_controls: &std::collections::BTreeSet<usize>,
) -> Result<Option<InlinePictures>> {
    let source = crate::office::doc::inspect(data)?;
    let controls: std::collections::BTreeSet<_> = source
        .stories
        .iter()
        .flat_map(|story| source_controls(story.kind, story.start_cp, &story.text, header_controls))
        .collect();
    let inspection = crate::office::doc::pictures::inspect_pictures(data)?;
    let references: Vec<_> = inspection
        .references
        .into_iter()
        .filter(|r| {
            (r.story == "body"
                || r.story == "headers"
                    && r.source_cp.is_some_and(|cp| header_controls.contains(&cp)))
                && r.source_cp.is_some_and(|cp| controls.contains(&cp))
                && r.role == "picture"
                && r.asset_index.is_some()
        })
        .collect();
    if references.is_empty() {
        return Ok(None);
    }
    if references.len() > 4096 {
        return Err(Error::Limit("inline picture occurrences"));
    }
    let stream = Compound::open(data)?.stream("Data")?;
    let formatting = crate::office::doc::formatting_inspection::inspect_character_formatting(data)?;
    let mut available: Vec<_> = inspection.assets.into_iter().map(Some).collect();
    let mut remap = BTreeMap::new();
    let mut assets = Vec::new();
    let mut result = Vec::new();
    let mut evidence_bytes = 0usize;
    for reference in references {
        let cp = reference.source_cp.ok_or_else(unsupported)?;
        let span = formatting
            .character_spans
            .get(
                formatting
                    .character_spans
                    .partition_point(|s| s.end_cp <= cp),
            )
            .filter(|s| s.start_cp <= cp && cp < s.end_cp)
            .ok_or_else(unsupported)?;
        formatting
            .pieces
            .get(formatting.pieces.partition_point(|p| p.end_cp <= cp))
            .filter(|p| p.start_cp <= cp && cp < p.end_cp && p.raw_prm == 0)
            .ok_or_else(unsupported)?;
        let last = |sprm| span.properties.iter().rev().find(|p| p.sprm == sprm);
        if last(0x0855).map(|p| p.operand.as_slice()) != Some(&[1][..])
            || last(0x0806).is_some_and(|p| p.operand.as_slice() != [0])
        {
            return Err(unsupported());
        }
        let location = last(0x6a03)
            .filter(|p| p.operand.len() == 4)
            .ok_or_else(unsupported)?;
        let offset = u32_at(&location.operand, 0)? as usize;
        let length = u32_at(&stream, offset)? as usize;
        if !(68..=32 * 1024 * 1024).contains(&length) {
            return Err(Error::Invalid("inline PICF length"));
        }
        let raw = bytes(&stream, offset, length)?;
        let header = &raw[..68];
        let size_pt = dimensions(header)?;
        let mut shape_start = offset + 68;
        match u16_at(header, 6)? {
            100 => {}
            102 => {
                let n = *stream.get(shape_start).ok_or_else(unsupported)? as usize;
                shape_start = shape_start
                    .checked_add(1 + n)
                    .filter(|p| *p < offset + length)
                    .ok_or_else(unsupported)?;
            }
            _ => return Err(unsupported()),
        }
        let (shape_id, properties, shape_record, client_anchor) =
            appearance(&stream, shape_start, offset + length)?;
        if reference.shape_id != Some(shape_id) {
            return Err(unsupported());
        }
        let old = reference.asset_index.unwrap();
        let index = if let Some(index) = remap.get(&old) {
            *index
        } else {
            let asset = available
                .get_mut(old)
                .and_then(Option::take)
                .ok_or_else(unsupported)?;
            let index = assets.len();
            assets.push(asset);
            remap.insert(old, index);
            index
        };
        validate_asset(
            &assets[index],
            shape_start + shape_record.len(),
            offset + length,
        )?;
        evidence_bytes = evidence_bytes
            .checked_add(
                68 + shape_record.len()
                    + span
                        .properties
                        .iter()
                        .map(|p| p.operand.len() + 2)
                        .sum::<usize>(),
            )
            .filter(|n| *n <= 8 * 1024 * 1024)
            .ok_or(Error::Limit("inline picture source evidence"))?;
        result.push(InlinePicture {
            source_cp: cp,
            asset_index: index,
            size_pt,
            shape_id,
            picf_range: [offset, offset + length],
            picf_header: header.to_vec(),
            properties,
            raw_modifiers: span.properties.clone(),
            shape_record,
            client_anchor,
            shape_offset: shape_start,
        });
    }
    result.sort_by_key(|r| r.source_cp);
    if result.windows(2).any(|r| r[0].source_cp == r[1].source_cp) {
        return Err(Error::Invalid("duplicate inline picture occurrence"));
    }
    let pictures = InlinePictures {
        references: result,
        assets,
    };
    pictures.validate()?;
    Ok(Some(pictures))
}

#[cfg(test)]
mod tests;
