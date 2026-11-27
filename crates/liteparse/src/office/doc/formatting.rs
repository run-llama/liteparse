//! Source character modifiers, before stylesheet and piece-property resolution.
//! MS-DOC §§2.4.6.2, 2.8.5, 2.9.33 and 2.9.327 define these structures.
use crate::office::doc::{Error, Result, bytes, u16_at, u32_at};
use serde::Serialize;

const MAX_RUNS: usize = 262_144;
const MAX_PROPERTY_BYTES: usize = 8 * 1024 * 1024;

struct Budget {
    runs: usize,
    property_bytes: usize,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            runs: MAX_RUNS,
            property_bytes: MAX_PROPERTY_BYTES,
        }
    }
}
impl Budget {
    fn reserve(&mut self, properties: &[PropertyModifier]) -> Result<()> {
        self.runs = self
            .runs
            .checked_sub(1)
            .ok_or(Error::Limit("character formatting runs"))?;
        for property in properties {
            let cost = std::mem::size_of::<PropertyModifier>()
                .checked_add(property.operand.len())
                .ok_or(Error::Limit("character formatting property bytes"))?;
            self.property_bytes = self
                .property_bytes
                .checked_sub(cost)
                .ok_or(Error::Limit("character formatting property bytes"))?;
        }
        Ok(())
    }
}

/// Style-relative values refer to the current style, not the preceding modifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToggleOperand {
    Off,
    On,
    Style,
    NotStyle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PropertyModifier {
    pub sprm: u16,
    /// Exact operand bytes, including any variable-length prefix.
    pub operand: Vec<u8>,
}
impl PropertyModifier {
    /// Returns a source toggle without resolving styles or subsequent piece modifiers.
    pub fn toggle(&self) -> Result<Option<ToggleOperand>> {
        if self.sprm >> 13 != 0 || (self.sprm >> 10) & 7 != 2 {
            return Ok(None);
        }
        match self.operand.as_slice() {
            [0] => Ok(Some(ToggleOperand::Off)),
            [1] => Ok(Some(ToggleOperand::On)),
            [0x80] => Ok(Some(ToggleOperand::Style)),
            [0x81] => Ok(Some(ToggleOperand::NotStyle)),
            _ => Err(Error::Invalid("character toggle operand")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CharacterRun {
    pub start_fc: usize,
    pub end_fc: usize,
    pub properties: Vec<PropertyModifier>,
}

/// A piece's FC is its decoded byte offset, with compression flags already removed.
#[derive(Debug, Clone, Copy)]
pub struct TextPiece {
    pub start_cp: usize,
    pub end_cp: usize,
    pub file_offset: usize,
    pub compressed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CharacterSpan {
    pub start_cp: usize,
    pub end_cp: usize,
    /// Ordered direct CHPX modifiers only. PCD PRM and style properties remain unresolved.
    pub properties: Vec<PropertyModifier>,
}

/// Decode CHPX character modifiers while preserving order and unfamiliar character SPRMs.
/// Non-character groups require their own operand rules and are explicitly unsupported here.
pub fn decode_character_modifiers(group: &[u8]) -> Result<Vec<PropertyModifier>> {
    let mut offset = 0;
    let mut result = Vec::new();
    let mut budget = Budget::default();
    while offset < group.len() {
        let sprm = u16_at(group, offset)?;
        if (sprm >> 10) & 7 != 2 {
            return Err(Error::Unsupported("non-character modifier in CHPX"));
        }
        offset += 2;
        let len = match sprm >> 13 {
            0 | 1 => 1,
            2 | 4 | 5 => 2,
            3 => 4,
            6 => 1 + *bytes(group, offset, 1)?.first().unwrap() as usize,
            7 => 3,
            _ => unreachable!(),
        };
        let property = PropertyModifier {
            sprm,
            operand: bytes(group, offset, len)?.to_vec(),
        };
        property.toggle()?;
        budget.reserve(std::slice::from_ref(&property))?;
        result.push(property);
        offset += len;
    }
    Ok(result)
}

/// Decode the FIB-referenced PlcBteChpx and its 512-byte pages into FC-ordered runs.
/// Empty or malformed metadata is reported rather than interpreted as visible text.
pub fn decode_chpx(word: &[u8], plc: &[u8]) -> Result<Vec<CharacterRun>> {
    if plc.len() < 12 || !(plc.len() - 4).is_multiple_of(8) {
        return Err(Error::Invalid("character formatting PLC size"));
    }
    let count = (plc.len() - 4) / 8;
    if count > MAX_RUNS {
        return Err(Error::Limit("character formatting pages"));
    }
    let mut result = Vec::new();
    let mut budget = Budget::default();
    for i in 0..count {
        let start = u32_at(plc, i * 4)? as usize;
        let end = u32_at(plc, (i + 1) * 4)? as usize;
        if start >= end {
            return Err(Error::Invalid("character formatting PLC offsets"));
        }
        // The high ten bits of PnFkpChpx are undefined and ignored by MS-DOC.
        let page = (u32_at(plc, (count + 1) * 4 + i * 4)? & 0x003f_ffff) as usize;
        let fkp = bytes(word, page * 512, 512)?;
        let runs = fkp[511] as usize;
        if runs == 0 || runs > 101 {
            return Err(Error::Invalid("character formatting run count"));
        }
        let rgb_start = (runs + 1) * 4;
        let payload_start = rgb_start + runs;
        if u32_at(fkp, 0)? as usize > start || (u32_at(fkp, runs * 4)? as usize) < end {
            return Err(Error::Invalid("character formatting page bounds"));
        }
        for j in 0..runs {
            let start_fc = u32_at(fkp, j * 4)? as usize;
            let end_fc = u32_at(fkp, (j + 1) * 4)? as usize;
            if start_fc >= end_fc {
                return Err(Error::Invalid("character formatting run offsets"));
            }
            if end_fc <= start || start_fc >= end {
                continue;
            }
            let start_fc = start_fc.max(start);
            let end_fc = end_fc.min(end);
            let offset = fkp[rgb_start + j] as usize * 2;
            let properties = if offset == 0 {
                Vec::new()
            } else {
                if offset < payload_start || offset >= 511 {
                    return Err(Error::Invalid("character formatting payload offset"));
                }
                let len = fkp[offset] as usize;
                decode_character_modifiers(bytes(&fkp[..511], offset + 1, len)?)?
            };
            budget.reserve(&properties)?;
            result.push(CharacterRun {
                start_fc,
                end_fc,
                properties,
            });
        }
    }
    Ok(result)
}

/// Intersect physical formatting runs with logical pieces, retaining UTF-16 CP units.
/// Piece order can differ from FC order after editing. Every piece byte must be covered.
pub fn map_runs_to_pieces(
    runs: &[CharacterRun],
    pieces: &[TextPiece],
) -> Result<Vec<CharacterSpan>> {
    if runs.windows(2).any(|w| w[0].end_fc > w[1].start_fc)
        || runs.iter().any(|r| r.start_fc >= r.end_fc)
    {
        return Err(Error::Invalid("unordered character formatting runs"));
    }
    let mut output = Vec::new();
    let mut budget = Budget::default();
    let mut next_cp = 0;
    for piece in pieces {
        if piece.start_cp != next_cp || piece.end_cp <= piece.start_cp {
            return Err(Error::Invalid("formatting piece character positions"));
        }
        let width = if piece.compressed { 1 } else { 2 };
        let length = (piece.end_cp - piece.start_cp)
            .checked_mul(width)
            .ok_or(Error::Invalid("formatting piece length overflow"))?;
        let end = piece
            .file_offset
            .checked_add(length)
            .ok_or(Error::Invalid("formatting piece offset overflow"))?;
        let mut fc = piece.file_offset;
        let first = runs.partition_point(|r| r.end_fc <= fc);
        for run in &runs[first..] {
            if fc == end {
                break;
            }
            if run.start_fc > fc {
                break;
            }
            let stop = run.end_fc.min(end);
            if (stop - piece.file_offset) % width != 0 {
                return Err(Error::Invalid("character formatting splits UTF-16 unit"));
            }
            budget.reserve(&run.properties)?;
            output.push(CharacterSpan {
                start_cp: piece.start_cp + (fc - piece.file_offset) / width,
                end_cp: piece.start_cp + (stop - piece.file_offset) / width,
                properties: run.properties.clone(),
            });
            fc = stop;
        }
        if fc != end {
            return Err(Error::Invalid("character formatting coverage gap"));
        }
        next_cp = piece.end_cp;
    }
    Ok(output)
}

pub(crate) mod section;

#[cfg(test)]
mod tests;
