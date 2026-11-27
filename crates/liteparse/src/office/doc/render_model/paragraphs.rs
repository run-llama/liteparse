//! Physical-paragraph formatting for the initial Western, non-table render path.
//! <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/484822ee-a9d9-4af4-8423-29fda67a6a58>
use super::runs::style_layers;
use crate::office::doc::{Error, Result, formatting::PropertyModifier, styles::Stylesheet};
use serde::Serialize;

pub mod auto_spacing;
pub mod direction;
pub mod presentation;

pub(super) fn decode(data: &[u8]) -> Result<Vec<PropertyModifier>> {
    if data.len() > 8 * 1024 * 1024 {
        return Err(Error::Limit("render paragraph modifier bytes"));
    }
    let mut result = Vec::new();
    let mut offset = 0usize;
    while offset < data.len() {
        let sprm = crate::office::doc::u16_at(data, offset)?;
        if (sprm >> 10) & 7 != 1 {
            return Err(Error::Unsupported("render non-paragraph modifier"));
        }
        offset += 2;
        let length =
            crate::office::doc::paragraph_references::operand_length(sprm, &data[offset..])?;
        result.push(PropertyModifier {
            sprm,
            operand: crate::office::doc::bytes(data, offset, length)?.to_vec(),
        });
        offset += length;
        if result.len() > 262_144 {
            return Err(Error::Limit("render paragraph modifiers"));
        }
    }
    Ok(result)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TabStop {
    pub position_twips: i16,
    pub alignment: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum LineSpacing {
    ExactTwips(u16),
    MinimumTwips(u16),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ParagraphProperties {
    pub justification: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_alignment: Option<u16>,
    pub left_twips: i16,
    pub right_twips: i16,
    pub first_twips: i16,
    pub before_twips: u16,
    pub after_twips: u16,
    pub keep_next: bool,
    pub keep_lines: bool,
    pub page_break_before: bool,
    pub widow_control: bool,
    pub outline_level: u8,
    pub line_multiple_240ths: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_spacing: Option<LineSpacing>,
    pub tabs: Vec<TabStop>,
    pub applied: Vec<PropertyModifier>,
}

/// Resolve source-ordered paragraph layout and retain validated presentation effects.
/// The adapter reports unapplied effects; structural ownership and revision guards remain.
pub fn resolve(
    styles: &Stylesheet,
    style: usize,
    direct: &[PropertyModifier],
) -> Result<ParagraphProperties> {
    let mut result = ParagraphProperties {
        justification: 0,
        font_alignment: None,
        left_twips: 0,
        right_twips: 0,
        first_twips: 0,
        before_twips: 0,
        after_twips: 0,
        keep_next: false,
        keep_lines: false,
        page_break_before: false,
        widow_control: true,
        outline_level: if (1..=9).contains(&style) {
            style as u8 - 1
        } else {
            9
        },
        line_multiple_240ths: 240,
        line_spacing: None,
        tabs: Vec::new(),
        applied: Vec::new(),
    };
    let layers = style_layers(styles, style, "paragraph")?;
    // Based-on paragraph styles apply ancestor properties before derived
    // properties. Preserve every modifier, including cumulative tab operations.
    for layer in layers.iter().rev() {
        for property in layer {
            if matches!(property.sprm, 0xc645 | 0x2443) {
                return Err(Error::Unsupported("render style numbering revision"));
            }
            apply(&mut result, property, style)?;
        }
    }
    if direct.len() > 262_144 {
        return Err(Error::Limit("render paragraph properties"));
    }
    let mut bytes = 0usize;
    for property in direct {
        bytes = bytes
            .checked_add(property.operand.len())
            .filter(|n| *n <= 8 * 1024 * 1024)
            .ok_or(Error::Limit("render paragraph property bytes"))?;
        apply(&mut result, property, style)?;
    }
    auto_spacing::resolve(&result.applied)?;
    direction::resolve(&result)?;
    Ok(result)
}

fn apply(
    result: &mut ParagraphProperties,
    property: &PropertyModifier,
    style: usize,
) -> Result<()> {
    let b = &property.operand;
    let boolean = || match b.as_slice() {
        [0] => Ok(false),
        [1] => Ok(true),
        _ => Err(Error::Invalid("render paragraph Bool8")),
    };
    let unsigned = || {
        if b.len() != 2 {
            return Err(Error::Invalid("render paragraph operand"));
        }
        crate::office::doc::u16_at(b, 0)
    };
    let signed = || -> Result<i16> {
        let value = unsigned()? as i16;
        if !(-31680..=31680).contains(&value) {
            return Err(Error::Invalid("render paragraph distance"));
        }
        Ok(value)
    };
    if presentation::decode(property)?.is_some() {
        result.applied.push(property.clone());
        return Ok(());
    }
    match property.sprm {
        // PRsid identifies the editing session for document merging. Its raw
        // value remains in applied properties and does not change layout.
        0x6467 if b.len() == 4 => {}
        0x2443 => {
            boolean()?;
        }
        0xc645 => {
            crate::office::doc::numbering_revision::decode_numrm_operand(b)?;
        }
        // Baseline alignment maps to the shared line baseline. Explicit automatic
        // alignment retains its application-selected behavior as partial evidence.
        0x4439 => {
            let alignment = unsigned()?;
            if alignment > 4 {
                return Err(Error::Invalid("render font alignment value"));
            }
            if !matches!(alignment, 2 | 4) {
                return Err(Error::Unsupported("render font alignment mode"));
            }
            result.font_alignment = Some(alignment);
        }
        // PTIstdInfo has a fixed 16-byte reserved payload with no rendering effect.
        0xc66c => {
            if b.len() != 17 || b[0] != 16 {
                return Err(Error::Invalid("render paragraph style info operand"));
            }
        }
        0xc60d => result.tabs = tabs(&result.tabs, b)?,
        // With no close-range deletions, PChgTabs has the same add-array
        // framing as PChgTabsPapx. Nonempty close ranges need separate handling.
        0xc615 if b.get(1) == Some(&0) && b.first() != Some(&255) => {
            result.tabs = tabs(&result.tabs, b)?;
        }
        0x2403 | 0x2461 => {
            if b.len() != 1 || b[0] > 3 {
                return Err(Error::Unsupported("render paragraph justification"));
            }
            result.justification = b[0];
        }
        // Character-unit first indentation is retained for source-face metric resolution.
        0x4457 => {
            if signed()? < 0 {
                return Err(Error::Unsupported("render character hanging indent"));
            }
        }
        0x840e | 0x845d => result.right_twips = signed()?,
        0x840f | 0x845e => result.left_twips = signed()?,
        0x8411 | 0x8460 => result.first_twips = signed()?,
        0xa413 | 0xa414 => {
            let value = unsigned()?;
            if value > 31680 {
                return Err(Error::Invalid("render paragraph spacing"));
            }
            if property.sprm == 0xa413 {
                result.before_twips = value;
            } else {
                result.after_twips = value;
            }
        }
        0x245b | 0x245c => {
            boolean()?;
        }
        0x2405 => result.keep_lines = boolean()?,
        0x2406 => result.keep_next = boolean()?,
        0x2407 => result.page_break_before = boolean()?,
        0x2431 => result.widow_control = boolean()?,
        // Document-level automatic hyphenation is excluded by source
        // qualification, so this per-paragraph switch has no active effect.
        0x242a => {
            boolean()?;
        }
        0x2640 => {
            if b.len() != 1 || b[0] > 9 {
                return Err(Error::Invalid("render outline level"));
            }
            if !(1..=9).contains(&style) {
                result.outline_level = b[0];
            }
        }
        // Explicitly disabled East Asian line constraints and punctuation overflow
        // match the bounded Western flow; enabled preferences remain unsupported.
        0x2433 | 0x2435 if b.as_slice() == [0] => {}
        // East Asian automatic spacing and grid indentation have no effect in
        // the qualified Western text path with section grids disabled.
        0x2437 | 0x2438 | 0x2448 | 0x2447 => {
            boolean()?;
        }
        // LSPD uses negative values for exact twips, nonnegative values for
        // minimum twips or 240ths of a line. Each ordered operand replaces the mode.
        // https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/fcdd2b79-9d47-4e0c-b1fd-705aeab88e41
        0x6412 => {
            if b.len() != 4 || crate::office::doc::u16_at(b, 2)? > 1 {
                return Err(Error::Invalid("render line spacing operand"));
            }
            let value = crate::office::doc::u16_at(b, 0)? as i16;
            if !(-31680..=31680).contains(&value) {
                return Err(Error::Invalid("render line spacing value"));
            }
            if value < 0 {
                result.line_spacing = Some(LineSpacing::ExactTwips(value.unsigned_abs()));
                result.line_multiple_240ths = 240;
            } else if crate::office::doc::u16_at(b, 2)? == 0 {
                result.line_spacing = Some(LineSpacing::MinimumTwips(value as u16));
                result.line_multiple_240ths = 240;
            } else {
                if value < 15 {
                    return Err(Error::Unsupported("render subminimum line multiplier"));
                }
                result.line_multiple_240ths = value as u16;
                result.line_spacing = None;
            }
        }
        0x240c if matches!(b.as_slice(), [0 | 1]) => {}
        0x2441 => {
            boolean()?;
        }
        0x2416 | 0x2417 | 0x244b | 0x244c if b.as_slice() == [0] => {}
        0x6649 if b.as_slice() == [0, 0, 0, 0] => {}
        _ => return Err(Error::Unsupported("render paragraph property")),
    }
    result.applied.push(property.clone());
    Ok(())
}

/// PChgTabsPapx carries ordered deletions within 25 twips and custom tab additions.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/6ff24fa9-40d8-4b54-93ab-d66a88e9ba5b>
fn tabs(current: &[TabStop], bytes: &[u8]) -> Result<Vec<TabStop>> {
    if bytes.len() < 3 || bytes[0] as usize + 1 != bytes.len() || bytes[0] < 2 || bytes[1] > 64 {
        return Err(Error::Invalid("render tab operand"));
    }
    let deleted = bytes[1] as usize;
    let add_at = 2 + 2 * deleted;
    let count = *bytes
        .get(add_at)
        .ok_or(Error::Invalid("render tab counts"))? as usize;
    if count > 64 || bytes.len() != add_at + 1 + 3 * count {
        return Err(Error::Invalid("render tab arrays"));
    }
    let positions = |offset: usize, n: usize| -> Result<Vec<i16>> {
        let values: Vec<_> = (0..n)
            .map(|i| crate::office::doc::u16_at(bytes, offset + 2 * i).map(|v| v as i16))
            .collect::<Result<_>>()?;
        if values.iter().any(|v| !(-31680..=31680).contains(v))
            || values.windows(2).any(|v| v[0] >= v[1])
        {
            return Err(Error::Invalid("render tab locations"));
        }
        Ok(values)
    };
    let removed = positions(2, deleted)?;
    let added = positions(add_at + 1, count)?;
    let mut result = current.to_vec();
    result.retain(|t| {
        !removed
            .iter()
            .any(|v| (i32::from(*v) - i32::from(t.position_twips)).abs() <= 25)
    });
    for (i, position) in added.into_iter().enumerate() {
        let tbd = bytes[add_at + 1 + 2 * count + i];
        if !matches!(tbd & 7, 0..=2 | 6) || (tbd >> 3) & 7 != 0 {
            return Err(Error::Unsupported("render tab alignment or leader"));
        }
        if result.iter().any(|t| {
            t.position_twips != position
                && (i32::from(t.position_twips) - i32::from(position)).abs() <= 25
        }) {
            return Err(Error::Unsupported("render nearby custom tabs"));
        }
        result.retain(|t| t.position_twips != position);
        result.push(TabStop {
            position_twips: position,
            alignment: tbd & 7,
        });
    }
    if result.len() > 64 {
        return Err(Error::Limit("render custom tabs"));
    }
    result.sort_by_key(|t| t.position_twips);
    Ok(result)
}

#[cfg(test)]
mod tests;
