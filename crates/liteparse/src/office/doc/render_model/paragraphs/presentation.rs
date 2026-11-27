//! Validated paragraph presentation retained for explicitly approximate text flow.
//! MS-DOC 2.6.2, 2.9.16–22, 2.9.121 and 2.9.247–249 define these operands.
use crate::office::doc::{Error, Result, formatting::PropertyModifier};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Presentation {
    Border { legacy: bool },
    Frame { sprm: u16, stored_value: u16 },
    CharacterIndent { right: bool, hundredths: i16 },
    ContextualSpacing { enabled: bool },
    Shading { legacy: bool, pattern: u16 },
    LineSpacing { before: bool, hundredths: i16 },
    LineBreaking { sprm: u16, enabled: bool },
}

/// Decode presentation only. The original ordered operands remain in `applied`.
/// Structural, revision, direction and unknown operators stay with their owners.
pub fn decode(property: &PropertyModifier) -> Result<Option<Presentation>> {
    let b = property.operand.as_slice();
    let invalid = || Error::Invalid("paragraph presentation operand");
    let result = match property.sprm {
        0x8418 | 0x8419 | 0x841a | 0x842e | 0x842f | 0x442b => {
            if b.len() != 2 {
                return Err(invalid());
            }
            let value = u16::from_le_bytes([b[0], b[1]]);
            let valid = match property.sprm {
                0x8418 | 0x8419 => (-31679..=31681).contains(&(value as i16)),
                0x442b => value & 0x7fff <= 31680 && value != 0x8000,
                _ => value <= 31680,
            };
            if !valid {
                return Err(invalid());
            }
            Presentation::Frame {
                sprm: property.sprm,
                stored_value: value,
            }
        }
        0x261b | 0x2423 => {
            if b.len() != 1 || (property.sprm == 0x2423 && b[0] > 5) {
                return Err(invalid());
            }
            Presentation::Frame {
                sprm: property.sprm,
                stored_value: u16::from(b[0]),
            }
        }
        0x4455 | 0x4456 => {
            if b.len() != 2 {
                return Err(invalid());
            }
            Presentation::CharacterIndent {
                right: property.sprm == 0x4455,
                hundredths: i16::from_le_bytes([b[0], b[1]]),
            }
        }
        0x246d => {
            let enabled = match b {
                [0] => false,
                [1] => true,
                _ => return Err(invalid()),
            };
            Presentation::ContextualSpacing { enabled }
        }
        0x6424..=0x6428 | 0x6629 => {
            if b.len() != 4 || b[2] > 16 || !border_type(b[1], true) {
                return Err(invalid());
            }
            Presentation::Border { legacy: true }
        }
        0xc64e..=0xc653 => {
            if b.len() != 9 || b[0] != 8 || !color(&b[1..5]) || !border_type(b[6], false) {
                return Err(invalid());
            }
            Presentation::Border { legacy: false }
        }
        0x442d => {
            if b.len() != 2 {
                return Err(invalid());
            }
            let value = u16::from_le_bytes([b[0], b[1]]);
            let pattern = value >> 10;
            if value != 0xffff
                && ((value & 31) > 16 || ((value >> 5) & 31) > 16 || !shading(pattern))
            {
                return Err(invalid());
            }
            Presentation::Shading {
                legacy: true,
                pattern,
            }
        }
        0xc64d => {
            if b.len() != 11 || b[0] != 10 || !color(&b[1..5]) || !color(&b[5..9]) {
                return Err(invalid());
            }
            let pattern = u16::from_le_bytes([b[9], b[10]]);
            if !shading(pattern) {
                return Err(invalid());
            }
            Presentation::Shading {
                legacy: false,
                pattern,
            }
        }
        0x4458 | 0x4459 => {
            if b.len() != 2 {
                return Err(invalid());
            }
            let value = i16::from_le_bytes([b[0], b[1]]);
            if !(-20..=31680).contains(&value) {
                return Err(invalid());
            }
            Presentation::LineSpacing {
                before: property.sprm == 0x4458,
                hundredths: value,
            }
        }
        0x2433..=0x2436 => {
            let enabled = match b {
                [0] => false,
                [1] => true,
                _ => return Err(invalid()),
            };
            Presentation::LineBreaking {
                sprm: property.sprm,
                enabled,
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(result))
}

fn border_type(value: u8, legacy: bool) -> bool {
    matches!(value, 0 | 1 | 3 | 5..=25) || (!legacy && matches!(value, 26 | 27))
}

fn color(value: &[u8]) -> bool {
    value.len() == 4 && matches!(value[3], 0 | 255)
}

fn shading(value: u16) -> bool {
    matches!(value, 0..=25 | 35..=61 | 0xffff)
}

#[cfg(test)]
mod tests;
