//! Paragraph properties selected from source FKP endpoints, with raw fragment gaps preserved.
use crate::office::doc::{
    Error, Result, bytes,
    compound::Compound,
    document::fib,
    formatting_inspection::{PieceProperties, decode_piece_metadata},
    paragraph_formatting::{ParagraphInspection, ParagraphRun, ParagraphSpan, decode_papx, record},
    structure::{ParagraphEndpoint, physical_endpoints},
    u16_at, u32_at,
};
use serde::Serialize;

const MAX_RECORDS: usize = 262_144;
const MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct ParagraphEndInspection {
    pub status: &'static str,
    pub formatting: ParagraphInspection,
    /// One endpoint per formatting span. PCD modifiers remain separate and unapplied.
    pub endpoints: Vec<ParagraphEndpoint>,
    pub uncovered_fragments: Vec<UncoveredFragment>,
    pub uncertain_terminals: Vec<UncertainTerminal>,
    pub trailing_nonstory_cp_range: Option<[usize; 2]>,
}

#[derive(Debug, Serialize)]
pub struct UncertainTerminal {
    pub source_cp: usize,
    pub unit: u16,
}

#[derive(Debug, Serialize)]
pub struct UncoveredFragment {
    pub source_cp: [usize; 2],
    pub physical_fc: [usize; 2],
    pub piece_index: usize,
}

/// MS-DOC 2.4.2 advances through logical pieces until an actual FKP endpoint is found.
/// Section 2.4.6.1 selects PAPX and PCD from that endpoint, including after edited fragments.
/// The selected PAPX excludes PCD modifiers; consumers append the endpoint piece's PRM once.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/61b635c3-2c44-4155-bf17-fec281b30c71>
pub fn inspect_paragraph_ends(data: &[u8]) -> Result<ParagraphEndInspection> {
    let container = Compound::open(data)?;
    let word = container.stream("WordDocument")?;
    let (version, table_name, lengths, clx_off, clx_len) = fib(&word)?;
    let table = container.stream(table_name)?;
    let (pieces, _, total) = decode_piece_metadata(&word, bytes(&table, clx_off, clx_len)?)?;
    let authored = lengths.iter().sum::<usize>();
    if total < authored || total - authored > 1 {
        return Err(Error::Invalid("paragraph endpoint story coverage"));
    }
    let offset = u32_at(&word, 154 + 13 * 8)? as usize;
    let length = u32_at(&word, 158 + 13 * 8)? as usize;
    select(
        &word,
        bytes(&table, offset, length)?,
        &pieces,
        version,
        &lengths,
    )
}

fn select(
    word: &[u8],
    plc: &[u8],
    pieces: &[PieceProperties],
    version: u16,
    lengths: &[usize; 8],
) -> Result<ParagraphEndInspection> {
    let authored = lengths
        .iter()
        .try_fold(0usize, |sum, n| sum.checked_add(*n))
        .ok_or(Error::Limit("paragraph endpoint source size"))?;
    if authored > 16 * 1024 * 1024 {
        return Err(Error::Limit("paragraph endpoint source size"));
    }
    let mut endpoints = physical_endpoints(word, plc, pieces)?;
    let total = pieces.last().map_or(0, |p| p.end_cp);
    if total < authored || total - authored > 1 {
        return Err(Error::Invalid("paragraph endpoint story coverage"));
    }
    endpoints.retain(|e| e.end_cp <= authored);
    let runs = decode_papx(word, plc)?;
    let uncovered_fragments = gaps(&runs, pieces, authored)?;
    let mut start = 0;
    let mut budget = MAX_BYTES;
    let mut spans = Vec::new();
    let mut uncertain_terminals = Vec::new();
    for endpoint in &endpoints {
        if endpoint.end_cp <= start {
            return Err(Error::Invalid("paragraph endpoint order"));
        }
        let piece = &pieces[endpoint.piece_index];
        let width = if piece.compressed { 1 } else { 2 };
        let mark_fc = endpoint
            .end_fc
            .checked_sub(width)
            .ok_or(Error::Invalid("paragraph endpoint terminal offset"))?;
        let unit = if width == 1 {
            bytes(word, mark_fc, 1)?[0] as u16
        } else {
            u16_at(word, mark_fc)?
        };
        if !matches!(unit, 7 | 12 | 13) {
            uncertain_terminals.push(UncertainTerminal {
                source_cp: endpoint.end_cp - 1,
                unit,
            });
        }
        let (style_index, raw) = record(bytes(word, endpoint.fkp_offset, 512)?, endpoint.fkp_run)?;
        budget = budget
            .checked_sub(
                raw.len()
                    + std::mem::size_of::<ParagraphSpan>()
                    + std::mem::size_of::<ParagraphEndpoint>(),
            )
            .ok_or(Error::Limit("paragraph endpoint property bytes"))?;
        spans.push(ParagraphSpan {
            start_cp: start,
            end_cp: endpoint.end_cp,
            style_index,
            raw_modifiers: raw.to_vec(),
        });
        start = endpoint.end_cp;
    }
    if start != authored {
        return Err(Error::Invalid("missing authored paragraph endpoint"));
    }
    let mut end = 0;
    for length in lengths {
        end += length;
        if *length != 0 && endpoints.binary_search_by_key(&end, |e| e.end_cp).is_err() {
            return Err(Error::Invalid("paragraph endpoint crosses story boundary"));
        }
    }
    Ok(ParagraphEndInspection {
        status: "source_endpoint_selected_paragraph_properties",
        formatting: ParagraphInspection {
            schema_version: 1,
            status: "source_endpoint_selected_paragraph_properties",
            binary_version: version,
            spans,
            excluded_nonstory_cp_range: (total > authored).then_some([authored, total]),
            unresolved: [
                "endpoint PCD modifiers remain separate for ordered application",
                "stylesheet built-in identity and inheritance",
                "raw paragraph/table modifiers and Data stream references",
                "revision-aware visible paragraph topology",
            ],
        },
        endpoints,
        uncovered_fragments,
        uncertain_terminals,
        trailing_nonstory_cp_range: (total > authored).then_some([authored, total]),
    })
}

fn gaps(
    runs: &[ParagraphRun],
    pieces: &[PieceProperties],
    authored: usize,
) -> Result<Vec<UncoveredFragment>> {
    let mut result = Vec::new();
    let mut work = MAX_RECORDS * 4;
    for (piece_index, piece) in pieces
        .iter()
        .enumerate()
        .filter(|(_, p)| p.start_cp < authored)
    {
        let width = if piece.compressed { 1 } else { 2 };
        let stop = piece.file_offset + (piece.end_cp.min(authored) - piece.start_cp) * width;
        let mut fc = piece.file_offset;
        let mut index = runs.partition_point(|r| r.end_fc <= fc);
        while fc < stop {
            work = work
                .checked_sub(1)
                .ok_or(Error::Limit("paragraph gap traversal"))?;
            let run = runs.get(index);
            if let Some(run) = run.filter(|r| r.start_fc <= fc) {
                fc = run.end_fc.min(stop);
                index += 1;
                continue;
            }
            let end = run.map_or(stop, |r| r.start_fc.min(stop));
            if (fc - piece.file_offset) % width != 0 || (end - piece.file_offset) % width != 0 {
                return Err(Error::Invalid("paragraph gap splits UTF-16 unit"));
            }
            if result.len() == MAX_RECORDS {
                return Err(Error::Limit("paragraph uncovered fragments"));
            }
            result.push(UncoveredFragment {
                source_cp: [
                    piece.start_cp + (fc - piece.file_offset) / width,
                    piece.start_cp + (end - piece.file_offset) / width,
                ],
                physical_fc: [fc, end],
                piece_index,
            });
            fc = end;
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
