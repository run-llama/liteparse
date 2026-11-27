//! Stylesheet identity and source inheritance, before property resolution.
use crate::office::doc::{Error, Result, bytes, compound::Compound, document::fib, u16_at, u32_at};
use serde::Serialize;

const MAX_STYLESHEET_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct Stylesheet {
    pub schema_version: u32,
    pub status: &'static str,
    pub std_base_bytes: usize,
    pub raw_header: Vec<u8>,
    /// Empty style slots retain their istd positions.
    pub styles: Vec<Option<Style>>,
    pub unresolved: [&'static str; 3],
}

#[derive(Debug, Serialize)]
pub struct Style {
    pub index: usize,
    pub builtin_id: u16,
    pub kind: u8,
    pub base_style: Option<usize>,
    pub next_style: usize,
    pub linked_style: Option<usize>,
    pub revision_marked: bool,
    pub name: String,
    pub raw_flags: u16,
    pub properties: Vec<StyleProperties>,
    /// The optional pre-revision style payload retains its framing for later decoding.
    pub raw_revision: Vec<u8>,
}

#[derive(Debug, Serialize)]
pub struct StyleProperties {
    pub kind: &'static str,
    /// PAPX includes its two-byte istd prefix; each payload retains its original bytes.
    pub bytes: Vec<u8>,
}

/// Read FibRgFcLcb97 pair 1, preserving source style identities and property groups.
pub fn inspect_styles(data: &[u8]) -> Result<Stylesheet> {
    let container = Compound::open(data)?;
    let word = container.stream("WordDocument")?;
    let (_, table_name, _, _, _) = fib(&word)?;
    let table = container.stream(table_name)?;
    let offset = u32_at(&word, 154 + 8)? as usize;
    let length = u32_at(&word, 158 + 8)? as usize;
    if length == 0 {
        return Err(Error::Invalid("missing stylesheet"));
    }
    decode(bytes(&table, offset, length)?)
}

/// Decode STSH, supporting ten-byte and eighteen-byte Stdf layouts.
/// Links and parent chains are checked before callers can use style identity.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/c8ee0f39-02c3-4caa-b27a-6a97600130fe>
pub fn decode(data: &[u8]) -> Result<Stylesheet> {
    if data.len() > MAX_STYLESHEET_BYTES {
        return Err(Error::Limit("stylesheet bytes"));
    }
    let header_size = u16_at(data, 0)? as usize;
    let header = bytes(data, 2, header_size)?;
    if header.len() < 18 {
        return Err(Error::Invalid("stylesheet header size"));
    }
    let count = u16_at(header, 0)? as usize;
    if !(15..0xffe).contains(&count) {
        return Err(Error::Invalid("stylesheet style count"));
    }
    let base_size = u16_at(header, 2)? as usize;
    if ![10, 18].contains(&base_size) {
        return Err(Error::Unsupported("stylesheet base layout"));
    }
    let mut offset = 2 + header_size;
    let mut styles = Vec::with_capacity(count);
    for index in 0..count {
        // LPStd records start on even byte boundaries; their signed length excludes padding.
        if !offset.is_multiple_of(2) {
            bytes(data, offset, 1)?;
            offset += 1;
        }
        let length = u16_at(data, offset)? as usize;
        offset += 2;
        if length > 0x7fff {
            return Err(Error::Invalid("negative style length"));
        }
        if length == 0 {
            styles.push(None);
            continue;
        }
        let raw = bytes(data, offset, length)?;
        offset += length;
        let style = decode_style(raw, base_size, index)?;
        let fixed = match index {
            0..=9 => Some(index as u16),
            10 => Some(65),
            11 => Some(105),
            12 => Some(107),
            _ => None,
        };
        if (13..=14).contains(&index) || fixed.is_some_and(|id| style.builtin_id != id) {
            return Err(Error::Invalid("fixed stylesheet identity"));
        }
        styles.push(Some(style));
    }
    if data.len() != offset && !(offset % 2 == 1 && data.len() == offset + 1) {
        return Err(Error::Invalid("stylesheet trailing data"));
    }
    validate_links(&styles)?;
    Ok(Stylesheet {
        schema_version: 1,
        status: "source_stylesheet_metadata",
        std_base_bytes: base_size,
        raw_header: header.to_vec(),
        styles,
        unresolved: [
            "style property inheritance and modifier application",
            "revision-marked style visibility",
            "paragraph and character direct formatting overrides",
        ],
    })
}

fn decode_style(raw: &[u8], base_size: usize, index: usize) -> Result<Style> {
    bytes(raw, 0, base_size)?;
    let builtin_id = u16_at(raw, 0)? & 0x0fff;
    let kind = (u16_at(raw, 2)? & 0xf) as u8;
    let base = u16_at(raw, 2)? >> 4;
    let packed = u16_at(raw, 4)?;
    let count = (packed & 0xf) as usize;
    let next_style = (packed >> 4) as usize;
    if u16_at(raw, 6)? as usize != raw.len() {
        return Err(Error::Invalid("style size crosscheck"));
    }
    let post = if base_size == 18 { u16_at(raw, 10)? } else { 0 };
    let revision_marked = post & 0x1000 != 0;
    let kinds: &[&str] = match kind {
        1 => &["paragraph", "character"],
        2 => &["character"],
        3 => &["table", "paragraph", "character"],
        4 => &["paragraph"],
        _ => return Err(Error::Invalid("style kind")),
    };
    if (revision_marked && kind >= 3) || count != kinds.len() + usize::from(revision_marked) {
        return Err(Error::Invalid("style property group count"));
    }
    let name_units = u16_at(raw, base_size)? as usize;
    let name_bytes = bytes(raw, base_size + 2, name_units * 2)?;
    let mut offset = base_size + 2 + name_units * 2;
    if u16_at(raw, offset)? != 0 {
        return Err(Error::Invalid("style name terminator"));
    }
    offset += 2;
    let units: Vec<_> = name_bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    let name = String::from_utf16(&units).map_err(|_| Error::Invalid("style name UTF-16"))?;
    if name.is_empty() {
        return Err(Error::Invalid("empty style name"));
    }
    let mut properties = Vec::with_capacity(kinds.len());
    for kind in kinds {
        let length = u16_at(raw, offset)? as usize;
        offset += 2;
        let payload = bytes(raw, offset, length)?;
        offset += length;
        if !offset.is_multiple_of(2) {
            bytes(raw, offset, 1)?;
            offset += 1;
        }
        properties.push(StyleProperties {
            kind,
            bytes: payload.to_vec(),
        });
    }
    let raw_revision = if revision_marked {
        let payload = raw
            .get(offset..)
            .ok_or(Error::Invalid("style revision offset"))?;
        if payload.is_empty() {
            return Err(Error::Invalid("missing style revision payload"));
        }
        payload.to_vec()
    } else {
        if offset != raw.len() {
            return Err(Error::Invalid("style trailing data"));
        }
        Vec::new()
    };
    Ok(Style {
        index,
        builtin_id,
        kind,
        base_style: (base != 0xfff).then_some(base as usize),
        next_style,
        linked_style: (post & 0xfff != 0).then_some((post & 0xfff) as usize),
        revision_marked,
        name,
        raw_flags: u16_at(raw, 8)?,
        properties,
        raw_revision,
    })
}

fn validate_links(styles: &[Option<Style>]) -> Result<()> {
    let present = |index: usize| styles.get(index).is_some_and(Option::is_some);
    for style in styles.iter().flatten() {
        if style.base_style.is_some_and(|index| !present(index))
            || !present(style.next_style)
            || style.linked_style.is_some_and(|index| !present(index))
        {
            return Err(Error::Invalid("stylesheet reference"));
        }
    }
    // Iterative three-color traversal bounds inheritance validation to one visit per style.
    let mut colors = vec![0; styles.len()];
    for start in 0..styles.len() {
        if colors[start] != 0 {
            continue;
        }
        let mut current = Some(start);
        let mut chain = Vec::new();
        while let Some(index) = current {
            if colors[index] == 2 {
                break;
            }
            if colors[index] == 1 {
                return Err(Error::Invalid("stylesheet inheritance cycle"));
            }
            colors[index] = 1;
            chain.push(index);
            current = styles[index].as_ref().and_then(|s| s.base_style);
        }
        for index in chain {
            colors[index] = 2;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
