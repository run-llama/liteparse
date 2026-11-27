//! Source-ordered language contributions from a qualified table-style hierarchy.
use crate::office::doc::{
    Error, Result,
    hidden_visibility::{charge, paragraph},
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct TableLanguageContribution {
    pub scope: &'static str,
    pub ordered_properties: Vec<StyleLanguageProperty>,
    pub complex_script_language: u16,
    pub visibility_properties: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct StyleLanguageProperty {
    pub style_index: usize,
    pub sprm: u16,
    pub operand: [u8; 2],
}

/// Decode ancestor-to-descendant CHPX groups after their style identities and
/// conditional/history exclusions have been qualified by the enclosing row proof.
/// The result is a table baseline; paragraph and direct language overrides follow.
pub(super) fn decode(
    groups: &[(usize, &[u8])],
    budget: &mut usize,
) -> Result<Option<TableLanguageContribution>> {
    if groups.len() > 64 {
        return Err(Error::Limit("table language style depth"));
    }
    let mut ordered_properties = Vec::new();
    let mut language = None;
    for &(style_index, bytes) in groups {
        for property in paragraph::group(bytes, 0, budget)? {
            // CLidBi changes the complex-script language, independently of hidden state.
            // MS-DOC §§2.6.1 and 2.9.134 define an unsigned 16-bit LID operand.
            if property.sprm != 0x485f {
                return Err(Error::Unsupported("table character contribution"));
            }
            let [low, high] = property.operand.as_slice() else {
                return Err(Error::Invalid("table language operand"));
            };
            charge(budget, std::mem::size_of::<StyleLanguageProperty>())?;
            language = Some(u16::from_le_bytes([*low, *high]));
            ordered_properties.push(StyleLanguageProperty {
                style_index,
                sprm: property.sprm,
                operand: [*low, *high],
            });
        }
    }
    Ok(
        language.map(|complex_script_language| TableLanguageContribution {
            scope: "table_style_baseline_before_paragraph_and_direct",
            ordered_properties,
            complex_script_language,
            visibility_properties: "unchanged_by_language_only_contribution",
        }),
    )
}

#[cfg(test)]
mod tests;
