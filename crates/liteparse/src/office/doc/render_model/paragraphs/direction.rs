//! Resolve source paragraph direction separately from physical and logical alignment.
use super::ParagraphProperties;
use crate::office::doc::{Error, Result};

pub struct Direction {
    pub rtl: bool,
    pub physical_justification: u8,
}

/// PFBiDi selects the base direction; PJc80 is physical and PJc is logical.
/// The first RTL subset excludes indentation and tabs until their source origins
/// can be retained through logical-to-physical conversion.
pub fn resolve(p: &ParagraphProperties) -> Result<Direction> {
    let mut rtl = false;
    let mut logical_alignment = false;
    for modifier in &p.applied {
        match modifier.sprm {
            0x2441 => {
                rtl = match modifier.operand.as_slice() {
                    [0] => false,
                    [1] => true,
                    _ => return Err(Error::Invalid("render paragraph direction Boolean")),
                }
            }
            0x2403 => logical_alignment = false,
            0x2461 => logical_alignment = true,
            _ => {}
        }
    }
    if rtl && (p.left_twips != 0 || p.right_twips != 0 || p.first_twips != 0 || !p.tabs.is_empty())
    {
        return Err(Error::Unsupported("render RTL indentation or tabs"));
    }
    Ok(Direction {
        rtl,
        physical_justification: if rtl && logical_alignment {
            match p.justification {
                0 => 2,
                2 => 0,
                value => value,
            }
        } else {
            p.justification
        },
    })
}

/// Arabic letters, marks, punctuation and digits supported by the initial shaping path.
/// Script extensions and directional formatting controls retain separate admission.
pub fn arabic(c: char) -> bool {
    matches!(c as u32, 0x060c | 0x061b | 0x061f | 0x0621..=0x063a | 0x0640..=0x065f | 0x0660..=0x0669)
}

/// The initial complex-font route covers Arabic plus ASCII text and source whitespace.
/// Other scripts and explicit bidi formatting controls need their own qualification.
pub fn complex_character(c: char) -> bool {
    arabic(c) || matches!(c, '\t' | '\r' | ' '..='~')
}

#[cfg(test)]
mod tests;
