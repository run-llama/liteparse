//! Source paragraph ownership, kept distinct from revision-aware visible topology.
use crate::office::doc::{
    Error, Inspection, Result, Transcription, bytes,
    compound::Compound,
    document::fib,
    formatting_inspection::{PieceProperties, decode_piece_metadata},
    u32_at,
};
use serde::Serialize;
use std::collections::BTreeMap;
const MAX_RECORDS: usize = 262_144;
const MAX_UNITS: usize = 16 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct StructurePlan {
    pub status: &'static str,
    pub stories: Vec<StoryStructure>,
    pub unresolved: [&'static str; 3],
}
#[derive(Debug, Serialize)]
pub struct StoryStructure {
    pub kind: &'static str,
    pub paragraphs: Vec<ParagraphOwnership>,
}
#[derive(Debug, Serialize)]
pub struct ParagraphOwnership {
    pub source_range: [usize; 2],
    pub projected_bytes: Option<[usize; 2]>,
    pub terminal_unit: Option<u16>,
    pub endpoint: Option<ParagraphEndpoint>,
    pub terminal_projection: &'static str,
    pub unresolved: Vec<&'static str>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ParagraphEndpoint {
    pub end_cp: usize,
    pub piece_index: usize,
    pub fkp_offset: usize,
    pub fkp_run: usize,
    pub end_fc: usize,
}

/// Resolve physical paragraph ends before classifying source controls.
/// Piece boundaries can split paragraphs; projected mark suppression does not establish
/// the formatting or ownership of a merged visible paragraph.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/30461a5b-e3ad-44cd-a3fe-038f86639b13>
pub fn inspect_structure(
    data: &[u8],
    source: &Inspection,
    projected: &Transcription,
) -> Result<StructurePlan> {
    if source.stories.len() > 8 || source.stories.iter().any(|s| s.utf16_units > MAX_UNITS) {
        return Err(Error::Limit("structure source stories"));
    }
    let c = Compound::open(data)?;
    let word = c.stream("WordDocument")?;
    let (version, table_name, lengths, clx_offset, clx_length) = fib(&word)?;
    validate_source_identity(version, &lengths, source)?;
    let table = c.stream(table_name)?;
    let (pieces, _, _) = decode_piece_metadata(&word, bytes(&table, clx_offset, clx_length)?)?;
    let offset = u32_at(&word, 154 + 13 * 8)? as usize;
    let length = u32_at(&word, 158 + 13 * 8)? as usize;
    let ends = physical_endpoints(&word, bytes(&table, offset, length)?, &pieces)?;
    plan(source, projected, &ends)
}

fn validate_source_identity(version: u16, lengths: &[usize; 8], source: &Inspection) -> Result<()> {
    let kinds = [
        "body",
        "footnotes",
        "headers",
        "reserved_macro",
        "comments",
        "endnotes",
        "textboxes",
        "header_textboxes",
    ];
    if version != source.binary_version || source.stories.len() != 8 {
        return Err(Error::Invalid("structure source identity"));
    }
    let mut cp = 0;
    for ((story, length), kind) in source.stories.iter().zip(lengths).zip(kinds) {
        if story.kind != kind || story.start_cp != cp || story.utf16_units != *length {
            return Err(Error::Invalid("structure source story identity"));
        }
        cp += length;
    }
    Ok(())
}

/// Follow MS-DOC 2.4.2's forward endpoint search within each source piece.
/// FKP limits beyond a piece are discarded and the search continues in the next piece.
pub(crate) fn physical_endpoints(
    word: &[u8],
    plc: &[u8],
    pieces: &[PieceProperties],
) -> Result<Vec<ParagraphEndpoint>> {
    if plc.len() < 12 || !(plc.len() - 4).is_multiple_of(8) {
        return Err(Error::Invalid("structure paragraph PLC"));
    }
    let n = (plc.len() - 4) / 8;
    if n > MAX_RECORDS || pieces.len() > MAX_RECORDS {
        return Err(Error::Limit("structure source records"));
    }
    let limits: Vec<_> = (0..=n)
        .map(|i| u32_at(plc, i * 4).map(|v| v as usize))
        .collect::<Result<_>>()?;
    if limits.windows(2).any(|w| w[0] >= w[1]) {
        return Err(Error::Invalid("structure PLC order"));
    }
    let mut out = Vec::new();
    let mut next_cp = 0;
    let mut work = MAX_RECORDS * 4;
    for (piece_index, piece) in pieces.iter().enumerate() {
        if piece.start_cp != next_cp
            || piece.end_cp <= piece.start_cp
            || piece.end_cp > MAX_UNITS + 1
        {
            return Err(Error::Invalid("structure piece order"));
        }
        next_cp = piece.end_cp;
        let width = if piece.compressed { 1 } else { 2 };
        let end = piece
            .file_offset
            .checked_add((piece.end_cp - piece.start_cp) * width)
            .ok_or(Error::Invalid("structure piece offset"))?;
        bytes(word, piece.file_offset, end - piece.file_offset)?;
        let mut fc = piece.file_offset;
        while fc < end {
            work = work
                .checked_sub(1)
                .ok_or(Error::Limit("structure FKP traversal"))?;
            if fc >= limits[n] {
                break;
            }
            let index = limits
                .partition_point(|x| *x <= fc)
                .checked_sub(1)
                .ok_or(Error::Invalid("structure PLC coverage"))?;
            let page = (u32_at(plc, (n + 1) * 4 + index * 4)? & 0x003f_ffff) as usize;
            let fkp_offset = page * 512;
            let fkp = bytes(word, fkp_offset, 512)?;
            let count = fkp[511] as usize;
            if count == 0 || count > 29 {
                return Err(Error::Invalid("structure FKP count"));
            }
            let rgfc: Vec<_> = (0..=count)
                .map(|i| u32_at(fkp, i * 4).map(|x| x as usize))
                .collect::<Result<_>>()?;
            if rgfc.windows(2).any(|w| w[0] >= w[1]) {
                return Err(Error::Invalid("structure FKP order"));
            }
            if rgfc[0] > limits[index] || rgfc[count] < limits[index + 1] {
                return Err(Error::Invalid("structure FKP PLC interval"));
            }
            // MS-DOC 2.4.2 compares the actual FKP limit with the piece limit.
            // A PLC interval selects the page; clipping to it would invent an endpoint.
            let k = rgfc
                .partition_point(|x| *x <= fc)
                .checked_sub(1)
                .ok_or(Error::Invalid("structure FKP coverage"))?;
            let stop = *rgfc.get(k + 1).ok_or(Error::Invalid("structure FKP end"))?;
            if stop > end {
                break;
            }
            if (stop - piece.file_offset) % width != 0 {
                return Err(Error::Invalid("structure split UTF-16 unit"));
            }
            if out.len() == MAX_RECORDS {
                return Err(Error::Limit("structure paragraph count"));
            }
            out.push(ParagraphEndpoint {
                end_cp: piece.start_cp + (stop - piece.file_offset) / width,
                piece_index,
                fkp_offset,
                fkp_run: k,
                end_fc: stop,
            });
            fc = stop;
        }
    }
    Ok(out)
}

fn plan(
    source: &Inspection,
    projected: &Transcription,
    ends: &[ParagraphEndpoint],
) -> Result<StructurePlan> {
    if source.stories.len() > 8
        || projected.stories.len() != source.stories.len()
        || ends.len() > MAX_RECORDS
        || ends.windows(2).any(|w| w[0].end_cp >= w[1].end_cp)
    {
        return Err(Error::Invalid("structure story or endpoint order"));
    }
    let mut budget = MAX_UNITS;
    let mut bytes_budget = 64 * 1024 * 1024usize;
    let mut segment_budget = MAX_RECORDS;
    let mut records = MAX_RECORDS;
    let mut stories = Vec::new();
    let mut next_cp = 0;
    for (raw, visible) in source.stories.iter().zip(&projected.stories) {
        if raw.kind != visible.kind || raw.start_cp != next_cp {
            return Err(Error::Invalid("structure story identity"));
        }
        budget = budget
            .checked_sub(raw.utf16_units)
            .ok_or(Error::Limit("structure text units"))?;
        bytes_budget = bytes_budget
            .checked_sub(raw.text.len())
            .and_then(|v| v.checked_sub(visible.text.len()))
            .ok_or(Error::Limit("structure text bytes"))?;
        segment_budget = segment_budget
            .checked_sub(visible.segments.len())
            .ok_or(Error::Limit("structure projected segments"))?;
        let units: Vec<_> = raw.text.encode_utf16().collect();
        if units.len() != raw.utf16_units {
            return Err(Error::Invalid("structure source length"));
        }
        let story_end = raw.start_cp + raw.utf16_units;
        next_cp = story_end;
        let a = ends.partition_point(|e| e.end_cp <= raw.start_cp);
        let b = ends.partition_point(|e| e.end_cp <= story_end);
        let mut cuts = BTreeMap::new();
        cuts.insert(raw.start_cp, None);
        cuts.insert(story_end, None);
        for end in &ends[a..b] {
            cuts.insert(end.end_cp, None);
        }
        projected_cuts(visible, raw.start_cp, story_end, &mut cuts)?;
        let mut paragraphs = Vec::new();
        let mut start = raw.start_cp;
        for endpoint in ends[a..b].iter().map(Some).chain(
            (ends[a..b].last().map(|e| e.end_cp) != Some(story_end) && start < story_end)
                .then_some(None),
        ) {
            let end = endpoint.map_or(story_end, |e| e.end_cp);
            if end == start {
                continue;
            }
            records = records
                .checked_sub(1)
                .ok_or(Error::Limit("structure paragraph count"))?;
            let terminal = units[end - raw.start_cp - 1];
            let mut unresolved = Vec::new();
            if endpoint.is_none() {
                unresolved.push("source_paragraph_endpoint_missing");
            }
            if !matches!(terminal, 7 | 12 | 13) {
                unresolved.push("source_paragraph_terminal_unexpected");
            }
            let projected_bytes = cuts[&start].zip(cuts[&end]).map(|(x, y)| [x, y]);
            if projected_bytes.is_none() {
                unresolved.push("projected_boundary_inside_nonuniform_segment");
            }
            let index = visible.segments.partition_point(|s| s.end_cp < end);
            let mark = visible
                .segments
                .get(index)
                .filter(|s| s.start_cp < end && s.end_cp >= end);
            let terminal_projection = match mark {
                Some(s) if s.output_start == s.output_end => {
                    unresolved.push("suppressed_terminal_visible_topology_unresolved");
                    s.kind
                }
                Some(s) => s.kind,
                None => {
                    unresolved.push("terminal_projection_unresolved");
                    "unresolved"
                }
            };
            paragraphs.push(ParagraphOwnership {
                source_range: [start, end],
                projected_bytes,
                terminal_unit: Some(terminal),
                endpoint: endpoint.cloned(),
                terminal_projection,
                unresolved,
            });
            start = end;
        }
        stories.push(StoryStructure {
            kind: raw.kind,
            paragraphs,
        });
    }
    Ok(StructurePlan {
        status: "source_paragraph_ownership",
        stories,
        unresolved: [
            "visible_paragraph_merges_and_formatting",
            "table_cell_row_and_table_ownership",
            "section_and_story_placement",
        ],
    })
}

/// Map requested CP cuts into existing output, retaining generated labels on the following range.
/// A nonuniform projected atom can only be cut at its source edges; ambiguous cuts stay absent.
fn projected_cuts(
    story: &crate::office::doc::TranscribedStory,
    start: usize,
    end: usize,
    cuts: &mut BTreeMap<usize, Option<usize>>,
) -> Result<()> {
    let mut cp = start;
    let mut output = 0;
    for s in &story.segments {
        if s.start_cp != cp
            || s.end_cp < s.start_cp
            || s.end_cp > end
            || s.output_start != output
            || s.output_end < s.output_start
        {
            return Err(Error::Invalid("structure projection order"));
        }
        let text = story
            .text
            .get(s.output_start..s.output_end)
            .ok_or(Error::Invalid("structure output boundary"))?;
        if let Some(value) = cuts.get_mut(&cp)
            && value.is_none()
        {
            *value = Some(s.output_start);
        }
        if s.end_cp > s.start_cp {
            if text.is_empty() {
                for (_, v) in cuts.range_mut(s.start_cp + 1..s.end_cp) {
                    *v = Some(s.output_start);
                }
            } else if !s.kind.starts_with("generated_")
                && text.encode_utf16().count() == s.end_cp - s.start_cp
            {
                let mut pos = s.start_cp;
                for (byte, ch) in text.char_indices() {
                    if pos > s.start_cp
                        && let Some(v) = cuts.get_mut(&pos)
                    {
                        *v = Some(s.output_start + byte);
                    }
                    pos += ch.len_utf16();
                }
            }
            if let Some(v) = cuts.get_mut(&s.end_cp) {
                *v = Some(s.output_end);
            }
        }
        cp = s.end_cp;
        output = s.output_end;
    }
    if cp != end || output != story.text.len() {
        return Err(Error::Invalid("structure projection coverage"));
    }
    if start == end {
        cuts.insert(start, Some(0));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
