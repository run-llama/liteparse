//! Source PAPX records and logical fragments; paragraph/style resolution remains pending.
use crate::office::doc::{
    Error, Result, bytes, compound::Compound, document::fib, formatting::TextPiece,
    formatting_inspection::decode_piece_metadata, u16_at, u32_at,
};
use serde::Serialize;

const MAX_RECORDS: usize = 262_144;
const MAX_METADATA_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct ParagraphRun {
    pub start_fc: usize,
    pub end_fc: usize,
    /// None denotes BxPap's default-property record; Some is the source stylesheet index.
    pub style_index: Option<u16>,
    pub raw_modifiers: Vec<u8>,
}

/// Logical fragments of physical PAPX records, before paragraph-end property selection.
#[derive(Debug, Serialize)]
pub struct ParagraphSpan {
    pub start_cp: usize,
    pub end_cp: usize,
    pub style_index: Option<u16>,
    pub raw_modifiers: Vec<u8>,
}

#[derive(Debug, Serialize)]
pub struct ParagraphInspection {
    pub schema_version: u32,
    pub status: &'static str,
    pub binary_version: u16,
    pub spans: Vec<ParagraphSpan>,
    /// Raw inspection retains this range; an authored expansion excludes its modifiers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excluded_nonstory_cp_range: Option<[usize; 2]>,
    pub unresolved: [&'static str; 4],
}

/// Read the FIB's PlcBtePapx using source piece metadata independently from CHPX.
pub fn inspect_paragraph_formatting(data: &[u8]) -> Result<ParagraphInspection> {
    let container = Compound::open(data)?;
    let word = container.stream("WordDocument")?;
    let (version, table_name, lengths, clx_off, clx_len) = fib(&word)?;
    let table = container.stream(table_name)?;
    let (pieces, _, total_cp) = decode_piece_metadata(&word, bytes(&table, clx_off, clx_len)?)?;
    let story_units = lengths.iter().sum::<usize>();
    if total_cp < story_units || total_cp - story_units > 1 {
        return Err(Error::Invalid("paragraph piece story coverage"));
    }
    // FibRgFcLcb97 pair 13 references PlcBtePapx in the selected Table stream.
    let offset = u32_at(&word, 154 + 13 * 8)? as usize;
    let length = u32_at(&word, 158 + 13 * 8)? as usize;
    if length == 0 {
        return Err(Error::Invalid("missing paragraph formatting PLC"));
    }
    let runs = decode_papx(&word, bytes(&table, offset, length)?)?;
    let pieces: Vec<_> = pieces.iter().map(|piece| piece.text_piece()).collect();
    Ok(ParagraphInspection {
        schema_version: 1,
        status: "unresolved_paragraph_fragments",
        binary_version: version,
        spans: map_runs_to_pieces(&runs, &pieces)?,
        excluded_nonstory_cp_range: None,
        unresolved: [
            "paragraph-end selection of PAPX and PCD properties",
            "stylesheet built-in identity and inheritance",
            "raw paragraph/table modifiers and huge-PAPX Data stream references",
            "outline levels, numbering and revision-aware paragraph boundaries",
        ],
    })
}

fn reserve(count: &mut usize, budget: &mut usize, payload: usize) -> Result<()> {
    *count = count
        .checked_sub(1)
        .ok_or(Error::Limit("paragraph formatting records"))?;
    *budget = budget
        .checked_sub(payload + std::mem::size_of::<ParagraphRun>())
        .ok_or(Error::Limit("paragraph formatting bytes"))?;
    Ok(())
}

/// Decode MS-DOC 2.9.174/175, preserving GrpPrlAndIstd bytes without applying SPRMs.
/// Both PapxInFkp length forms are supported; BxPap zero means default properties.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/580510b8-df7a-467e-a51c-0d71eb15c7cd>
pub fn decode_papx(word: &[u8], plc: &[u8]) -> Result<Vec<ParagraphRun>> {
    if plc.len() < 4 || !(plc.len() - 4).is_multiple_of(8) {
        return Err(Error::Invalid("paragraph formatting PLC size"));
    }
    let pages = (plc.len() - 4) / 8;
    if pages == 0 || pages > MAX_RECORDS {
        return Err(Error::Limit("paragraph formatting pages"));
    }
    let mut count = MAX_RECORDS;
    let mut budget = MAX_METADATA_BYTES;
    let mut output = Vec::new();
    for i in 0..pages {
        let start = u32_at(plc, i * 4)? as usize;
        let end = u32_at(plc, (i + 1) * 4)? as usize;
        if start >= end {
            return Err(Error::Invalid("paragraph formatting PLC offsets"));
        }
        let page = (u32_at(plc, (pages + 1) * 4 + i * 4)? & 0x003f_ffff) as usize;
        let fkp = bytes(word, page * 512, 512)?;
        let runs = fkp[511] as usize;
        if runs == 0 || runs > 29 {
            return Err(Error::Invalid("paragraph formatting run count"));
        }
        if u32_at(fkp, 0)? as usize > start || (u32_at(fkp, runs * 4)? as usize) < end {
            return Err(Error::Invalid("paragraph formatting page bounds"));
        }
        for j in 0..runs {
            let from = u32_at(fkp, j * 4)? as usize;
            let to = u32_at(fkp, (j + 1) * 4)? as usize;
            if from >= to {
                return Err(Error::Invalid("paragraph formatting run offsets"));
            }
            if to <= start || from >= end {
                continue;
            }
            let (style_index, raw) = record(fkp, j)?;
            reserve(&mut count, &mut budget, raw.len())?;
            output.push(ParagraphRun {
                start_fc: from.max(start),
                end_fc: to.min(end),
                style_index,
                raw_modifiers: raw.to_vec(),
            });
        }
    }
    Ok(output)
}

/// Read one validated FKP row's source style and modifier bytes.
pub(crate) fn record(fkp: &[u8], index: usize) -> Result<(Option<u16>, &[u8])> {
    let runs = *bytes(fkp, 511, 1)?.first().unwrap() as usize;
    if runs == 0 || runs > 29 || index >= runs {
        return Err(Error::Invalid("paragraph formatting record index"));
    }
    let rgbx = (runs + 1) * 4;
    let payload_start = rgbx + runs * 13;
    let offset = fkp[rgbx + index * 13] as usize * 2;
    let result = if offset == 0 {
        (None, &[][..])
    } else {
        if offset < payload_start || offset >= 511 {
            return Err(Error::Invalid("paragraph formatting payload offset"));
        }
        let cb = fkp[offset] as usize;
        let (data_offset, length) = if cb == 0 {
            let cb2 = *bytes(&fkp[..511], offset + 1, 1)?.first().unwrap() as usize;
            if cb2 == 0 {
                return Err(Error::Invalid("paragraph formatting extended length"));
            }
            (offset + 2, cb2 * 2)
        } else {
            (offset + 1, cb * 2 - 1)
        };
        let payload = bytes(&fkp[..511], data_offset, length)?;
        let style = u16_at(payload, 0)?;
        (Some(style), &payload[2..])
    };
    Ok(result)
}

/// Map source record fragments to UTF-16 CPs; these spans do not resolve whole paragraphs.
/// Logical piece order may differ from physical FC order after edits.
pub fn map_runs_to_pieces(
    runs: &[ParagraphRun],
    pieces: &[TextPiece],
) -> Result<Vec<ParagraphSpan>> {
    if runs.windows(2).any(|w| w[0].end_fc > w[1].start_fc)
        || runs.iter().any(|r| r.start_fc >= r.end_fc)
    {
        return Err(Error::Invalid("unordered paragraph formatting runs"));
    }
    let mut count = MAX_RECORDS;
    let mut budget = MAX_METADATA_BYTES;
    let mut output = Vec::new();
    let mut next_cp = 0;
    for piece in pieces {
        if piece.start_cp != next_cp || piece.end_cp <= piece.start_cp {
            return Err(Error::Invalid("paragraph piece character positions"));
        }
        let width = if piece.compressed { 1 } else { 2 };
        let length = (piece.end_cp - piece.start_cp)
            .checked_mul(width)
            .ok_or(Error::Invalid("paragraph piece length overflow"))?;
        let end = piece
            .file_offset
            .checked_add(length)
            .ok_or(Error::Invalid("paragraph piece offset overflow"))?;
        let mut fc = piece.file_offset;
        let first = runs.partition_point(|r| r.end_fc <= fc);
        for run in &runs[first..] {
            if fc == end || run.start_fc > fc {
                break;
            }
            let stop = run.end_fc.min(end);
            if (stop - piece.file_offset) % width != 0 {
                return Err(Error::Invalid("paragraph formatting splits UTF-16 unit"));
            }
            reserve(&mut count, &mut budget, run.raw_modifiers.len())?;
            output.push(ParagraphSpan {
                start_cp: piece.start_cp + (fc - piece.file_offset) / width,
                end_cp: piece.start_cp + (stop - piece.file_offset) / width,
                style_index: run.style_index,
                raw_modifiers: run.raw_modifiers.clone(),
            });
            fc = stop;
        }
        if fc != end {
            return Err(Error::Invalid("paragraph formatting coverage gap"));
        }
        next_cp = piece.end_cp;
    }
    Ok(output)
}

#[cfg(test)]
mod tests;
