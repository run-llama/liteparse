//! Diagnostic character formatting with source positions and unresolved piece properties.
use crate::office::doc::{
    Error, Result, bytes,
    compound::Compound,
    document::fib,
    formatting::{CharacterSpan, TextPiece, decode_chpx, map_runs_to_pieces},
    u16_at, u32_at,
};
use serde::Serialize;

const MAX_PIECES: usize = 262_144;
const MAX_GROUPS: usize = 32_768;
const MAX_GROUP_BYTES: usize = 8 * 1024 * 1024;
const MAX_TEXT_UNITS: usize = 16 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct FormattingInspection {
    pub schema_version: u32,
    pub status: &'static str,
    pub binary_version: u16,
    pub pieces: Vec<PieceProperties>,
    /// Exclusive end of the authored stories described by FibRgLw97.
    pub authored_story_end_cp: usize,
    /// Optional CLX terminal CP outside all authored stories; retained in pieces.
    pub trailing_nonstory_cp_range: Option<[usize; 2]>,
    /// Exact CLX groups. Paragraph/table modifiers within these are not interpreted.
    pub raw_property_groups: Vec<Vec<u8>>,
    pub character_spans: Vec<CharacterSpan>,
    pub unresolved: [&'static str; 3],
}

#[derive(Debug, Serialize)]
pub struct PieceProperties {
    pub start_cp: usize,
    pub end_cp: usize,
    pub file_offset: usize,
    pub compressed: bool,
    pub raw_flags: u16,
    /// Raw Prm0 or Prm1 bits. A complex PRM indexes raw_property_groups with raw_prm >> 1.
    pub raw_prm: u16,
}
impl PieceProperties {
    pub(crate) fn text_piece(&self) -> TextPiece {
        TextPiece {
            start_cp: self.start_cp,
            end_cp: self.end_cp,
            file_offset: self.file_offset,
            compressed: self.compressed,
        }
    }
}

/// Inspect source formatting independently of raw text extraction and text visibility.
/// Success establishes CHPX coverage; styles, PAPX and PCD property application remain pending.
pub fn inspect_character_formatting(data: &[u8]) -> Result<FormattingInspection> {
    let container = Compound::open(data)?;
    let word = container.stream("WordDocument")?;
    let (version, table_name, story_lengths, clx_off, clx_len) = fib(&word)?;
    let table = container.stream(table_name)?;
    let (pieces, groups, total_cp) =
        decode_piece_metadata(&word, bytes(&table, clx_off, clx_len)?)?;
    let story_units = story_lengths.iter().sum::<usize>();
    if total_cp < story_units || total_cp - story_units > 1 {
        return Err(Error::Invalid("formatting piece story coverage"));
    }
    // FibRgFcLcb97 pair 12 references PlcBteChpx in the selected Table stream.
    let offset = u32_at(&word, 154 + 12 * 8)? as usize;
    let length = u32_at(&word, 158 + 12 * 8)? as usize;
    if offset == 0 || length == 0 {
        return Err(Error::Invalid("missing character formatting PLC"));
    }
    let runs = decode_chpx(&word, bytes(&table, offset, length)?)?;
    let mappings = authored_mappings(&pieces, story_units)?;
    let character_spans = map_runs_to_pieces(&runs, &mappings)?;
    Ok(FormattingInspection {
        schema_version: 1,
        status: "unresolved_character_formatting",
        binary_version: version,
        pieces,
        authored_story_end_cp: story_units,
        trailing_nonstory_cp_range: (total_cp > story_units).then_some([story_units, total_cp]),
        raw_property_groups: groups,
        character_spans,
        unresolved: [
            "paragraph properties and stylesheet inheritance",
            "PCD property modifiers and CLX property-group semantics",
            "accepted revision visibility, hidden text and special-character interpretation",
        ],
    })
}

/// PlcPcd may include one terminal CP after all FibRgLw97 stories.
/// Keep source pieces complete while requiring character properties only for authored CPs.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/1caae71f-35c4-49d7-adf0-af5fc766331c>
fn authored_mappings(pieces: &[PieceProperties], story_units: usize) -> Result<Vec<TextPiece>> {
    let mut result = Vec::new();
    let mut next_cp = 0;
    for piece in pieces {
        if piece.start_cp != next_cp || piece.end_cp <= piece.start_cp {
            return Err(Error::Invalid("authored formatting piece order"));
        }
        next_cp = piece.end_cp;
        if piece.start_cp < story_units {
            let mut mapped = piece.text_piece();
            mapped.end_cp = mapped.end_cp.min(story_units);
            result.push(mapped);
        }
    }
    if next_cp < story_units || next_cp - story_units > 1 {
        return Err(Error::Invalid("authored formatting piece coverage"));
    }
    Ok(result)
}

pub(crate) type PieceMetadata = (Vec<PieceProperties>, Vec<Vec<u8>>, usize);
pub(crate) fn decode_piece_metadata(word: &[u8], clx: &[u8]) -> Result<PieceMetadata> {
    let mut offset = 0;
    let mut groups = Vec::new();
    let mut remaining_group_bytes = MAX_GROUP_BYTES;
    while clx.get(offset) == Some(&1) {
        let len = u16_at(clx, offset + 1)? as usize;
        // PrcData.cbGrpprl is signed, nonnegative and at most 0x3fa2 bytes.
        if len > 0x3fa2 {
            return Err(Error::Invalid("CLX property group length"));
        }
        if groups.len() == MAX_GROUPS {
            return Err(Error::Limit("CLX property groups"));
        }
        remaining_group_bytes = remaining_group_bytes
            .checked_sub(len + std::mem::size_of::<Vec<u8>>())
            .ok_or(Error::Limit("CLX property group bytes"))?;
        groups.push(bytes(clx, offset + 3, len)?.to_vec());
        offset += 3 + len;
    }
    if clx.get(offset) != Some(&2) {
        return Err(Error::Invalid("missing formatting piece table"));
    }
    let len = u32_at(clx, offset + 1)? as usize;
    let plc = bytes(clx, offset + 5, len)?;
    if offset + 5 + len != clx.len() || len < 4 || !(len - 4).is_multiple_of(12) {
        return Err(Error::Invalid("formatting piece table size"));
    }
    let count = (len - 4) / 12;
    if count > MAX_PIECES {
        return Err(Error::Limit("formatting pieces"));
    }
    let total_cp = u32_at(plc, count * 4)? as usize;
    if total_cp > MAX_TEXT_UNITS {
        return Err(Error::Limit("formatting text length"));
    }
    if u32_at(plc, 0)? != 0 {
        return Err(Error::Invalid("initial formatting character position"));
    }
    let mut pieces = Vec::with_capacity(count);
    for i in 0..count {
        let start_cp = u32_at(plc, i * 4)? as usize;
        let end_cp = u32_at(plc, (i + 1) * 4)? as usize;
        if end_cp <= start_cp || end_cp > total_cp {
            return Err(Error::Invalid("formatting piece character positions"));
        }
        let pcd_offset = (count + 1) * 4 + i * 8;
        let raw_flags = u16_at(plc, pcd_offset)?;
        let fc = u32_at(plc, pcd_offset + 2)?;
        let raw_prm = u16_at(plc, pcd_offset + 6)?;
        if raw_prm & 1 != 0 && (raw_prm >> 1) as usize >= groups.len() {
            return Err(Error::Invalid("CLX property group reference"));
        }
        let compressed = fc & 0x4000_0000 != 0;
        let encoded_offset = (fc & 0x3fff_ffff) as usize;
        let file_offset = if compressed {
            if !encoded_offset.is_multiple_of(2) {
                return Err(Error::Invalid("compressed formatting text offset"));
            }
            encoded_offset / 2
        } else {
            encoded_offset
        };
        let width = if compressed { 1 } else { 2 };
        bytes(word, file_offset, (end_cp - start_cp) * width)?;
        pieces.push(PieceProperties {
            start_cp,
            end_cp,
            file_offset,
            compressed,
            raw_flags,
            raw_prm,
        });
    }
    Ok((pieces, groups, total_cp))
}

#[cfg(test)]
mod tests;
