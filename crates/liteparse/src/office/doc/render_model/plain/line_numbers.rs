//! Ordered line-number settings retained separately from authored body characters.
use super::section_options::SectionOption;
use crate::office::doc::{Error, Result, render_model::paragraphs::ParagraphProperties};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Settings {
    pub interval: u16,
    pub restart: u8,
    pub authored_start_zero_based: u16,
    pub distance_twips: u16,
}
impl Settings {
    /// Continuous numbering starts at one in the first section (SLncOperand).
    pub fn effective_start_zero_based(&self) -> u16 {
        if self.restart == 2 {
            0
        } else {
            self.authored_start_zero_based
        }
    }
}
pub fn resolve(options: &[SectionOption]) -> Result<Option<Settings>> {
    let mut value = Settings {
        interval: 0,
        restart: 0,
        authored_start_zero_based: 0,
        distance_twips: 0,
    };
    for option in options {
        let p = &option.property;
        match (p.sprm, p.operand.as_slice()) {
            (0x3013, [mode @ 0..=2]) => value.restart = *mode,
            (0x5015, [a, b]) if u16::from_le_bytes([*a, *b]) <= 100 => {
                value.interval = u16::from_le_bytes([*a, *b])
            }
            (0x501b, [a, b]) => value.authored_start_zero_based = u16::from_le_bytes([*a, *b]),
            (0x9016, [a, b]) if u16::from_le_bytes([*a, *b]) <= 31680 => {
                value.distance_twips = u16::from_le_bytes([*a, *b])
            }
            (0x3013 | 0x5015 | 0x501b | 0x9016, _) => {
                return Err(Error::Invalid("line numbering operand"));
            }
            _ => {}
        }
    }
    Ok((value.interval != 0).then_some(value))
}
pub fn suppressed(paragraph: &ParagraphProperties) -> Result<bool> {
    let mut suppressed = false;
    for p in &paragraph.applied {
        if p.sprm == 0x240c {
            suppressed = match p.operand.as_slice() {
                [0] => false,
                [1] => true,
                _ => return Err(Error::Invalid("line numbering suppression Boolean")),
            };
        }
    }
    Ok(suppressed)
}

#[cfg(test)]
mod tests;
