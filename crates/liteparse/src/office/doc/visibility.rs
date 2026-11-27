//! Evidence for revision and special-character properties; text is retained unchanged.
//!
//! Primary specifications:
//! - [Character operators and reset preservation](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/7022285b-9621-42e9-ad4d-4e02c115ef18)
//! - [Style restrictions](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/0188ecda-b590-4cb4-bb95-e76a47a9a2e2)
//! - [Prm0 mapping](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/35226a0b-9038-4427-83c2-3830a8554267)
//! - [Implicit fSpec assignments, p. 76](https://download.microsoft.com/download/0/b/e/0be8bdd7-e5e8-422a-abfd-4342ed7ad886/word97-2007binaryfileformat%28doc%29specification.pdf)
use crate::office::doc::{
    Error, Result,
    formatting::{PropertyModifier, ToggleOperand, decode_character_modifiers},
    formatting_inspection::FormattingInspection,
};
use serde::Serialize;
use std::collections::BTreeMap;

const MAX_SPANS: usize = 262_144;
const MAX_EVIDENCE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct RevisionEvidence {
    pub status: &'static str,
    pub basis: &'static str,
    pub hidden_text: &'static str,
    pub spans: Vec<RevisionSpan>,
}

#[derive(Debug, Serialize)]
pub struct RevisionSpan {
    pub start_cp: usize,
    pub end_cp: usize,
    pub source_span_index: usize,
    pub piece_index: usize,
    pub properties: RevisionProperties,
    pub evidence: Vec<RevisionModifier>,
    pub unresolved: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RevisionProperties {
    pub deleted: Option<bool>,
    pub inserted: Option<bool>,
    pub field_hidden: Option<bool>,
    pub special: Option<bool>,
}
impl Default for RevisionProperties {
    fn default() -> Self {
        Self {
            deleted: Some(false),
            inserted: Some(false),
            field_hidden: Some(false),
            special: Some(false),
        }
    }
}
impl RevisionProperties {
    fn unknown() -> Self {
        Self {
            deleted: None,
            inserted: None,
            field_hidden: None,
            special: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RevisionModifier {
    pub source: &'static str,
    pub sprm: u16,
    pub operand: Vec<u8>,
}

/// Resolve only properties excluded from styles by MS-DOC UpxChpx's CIstd preservation rule.
/// Source styles are assumed conforming. Hidden text and text suppression remain unresolved.
/// Unsupported operators invalidate a span's evidence rather than establishing visibility.
pub fn resolve_revision_evidence(formatting: &FormattingInspection) -> Result<RevisionEvidence> {
    if formatting.character_spans.len() > MAX_SPANS || formatting.pieces.len() > MAX_SPANS {
        return Err(Error::Limit("revision evidence spans"));
    }
    let mut pc = BTreeMap::new();
    let mut budget = MAX_EVIDENCE_BYTES;
    let mut piece_end = 0;
    for piece in &formatting.pieces {
        if piece.start_cp != piece_end || piece.end_cp <= piece.start_cp {
            return Err(Error::Invalid("revision evidence piece order"));
        }
        piece_end = piece.end_cp;
        if let std::collections::btree_map::Entry::Vacant(entry) = pc.entry(piece.raw_prm) {
            let decoded = piece_modifiers(piece.raw_prm, &formatting.raw_property_groups);
            if let Ok(properties) = &decoded {
                for property in properties {
                    let cost = std::mem::size_of::<PropertyModifier>() + property.operand.len();
                    budget = budget
                        .checked_sub(cost)
                        .ok_or(Error::Limit("revision piece property bytes"))?;
                }
            }
            entry.insert(decoded);
        }
    }
    let mut spans = Vec::new();
    let mut next_cp = 0;
    for (source_span_index, span) in formatting.character_spans.iter().enumerate() {
        if span.start_cp != next_cp || span.end_cp <= span.start_cp {
            return Err(Error::Invalid("revision evidence character positions"));
        }
        let piece_index = formatting
            .pieces
            .partition_point(|p| p.end_cp <= span.start_cp);
        let piece = formatting
            .pieces
            .get(piece_index)
            .filter(|p| p.start_cp <= span.start_cp && p.end_cp >= span.end_cp)
            .ok_or(Error::Invalid("revision evidence piece coverage"))?;
        let mut state = RevisionSpan {
            start_cp: span.start_cp,
            end_cp: span.end_cp,
            source_span_index,
            piece_index,
            properties: RevisionProperties::default(),
            evidence: Vec::new(),
            unresolved: Vec::new(),
        };
        for property in &span.properties {
            apply(&mut state, property, "chpx", &mut budget)?;
        }
        match &pc[&piece.raw_prm] {
            Ok(properties) => {
                for property in properties {
                    apply(&mut state, property, "pcd_prm", &mut budget)?;
                }
            }
            Err(error) => record_unresolved(&mut state, error.to_string(), &mut budget)?,
        }
        if !state.unresolved.is_empty() {
            state.properties = RevisionProperties::unknown();
        }
        next_cp = span.end_cp;
        spans.push(state);
    }
    let coverage_matches = match formatting.trailing_nonstory_cp_range {
        Some([start, end]) => {
            start == formatting.authored_story_end_cp
                && end == piece_end
                && end.checked_sub(start) == Some(1)
        }
        None => formatting.authored_story_end_cp == piece_end,
    };
    if !coverage_matches || next_cp != formatting.authored_story_end_cp {
        return Err(Error::Invalid("revision evidence incomplete coverage"));
    }
    Ok(RevisionEvidence {
        status: "revision_evidence_only",
        basis: "Final revision-property view: MS-DOC UpxChpx forbids these properties in conforming styles; CWall preserves history while later operators set current values",
        hidden_text: "unresolved_style_dependent",
        spans,
    })
}

fn apply(
    state: &mut RevisionSpan,
    property: &PropertyModifier,
    source: &'static str,
    budget: &mut usize,
) -> Result<()> {
    // Word 97–2007 Binary File Format, p. 76: both operators also set fSpec.
    // Later CFSpec modifiers still override this implicit assignment in source order.
    if matches!(property.sprm, 0x6a03 | 0x6a09) {
        if property.operand.len() != 4 {
            return Err(Error::Invalid("special character operand"));
        }
        state.properties.special = Some(true);
        let cost = std::mem::size_of::<RevisionModifier>() + property.operand.len();
        *budget = budget
            .checked_sub(cost)
            .ok_or(Error::Limit("revision evidence bytes"))?;
        state.evidence.push(RevisionModifier {
            source,
            sprm: property.sprm,
            operand: property.operand.clone(),
        });
        return Ok(());
    }
    // CWall preserves the pre-revision formatting for an original view. The final
    // property view applies later operators normally, retaining the wall as evidence.
    if property.sprm == 0x2a83 {
        if !matches!(property.operand.as_slice(), [0] | [1]) {
            return Err(Error::Invalid("character formatting revision wall"));
        }
        let cost = std::mem::size_of::<RevisionModifier>() + property.operand.len();
        *budget = budget
            .checked_sub(cost)
            .ok_or(Error::Limit("revision evidence bytes"))?;
        state.evidence.push(RevisionModifier {
            source,
            sprm: property.sprm,
            operand: property.operand.clone(),
        });
        return Ok(());
    }
    let target = match property.sprm {
        0x0800 => Some(&mut state.properties.deleted),
        0x0801 => Some(&mut state.properties.inserted),
        0x0802 => Some(&mut state.properties.field_hidden),
        0x0855 => Some(&mut state.properties.special),
        _ => None,
    };
    if let Some(target) = target {
        let value = match property.toggle()? {
            Some(ToggleOperand::Off | ToggleOperand::Style) => false,
            Some(ToggleOperand::On | ToggleOperand::NotStyle) => true,
            None => return Err(Error::Invalid("revision evidence toggle")),
        };
        *target = Some(value);
        let cost = std::mem::size_of::<RevisionModifier>() + property.operand.len();
        *budget = budget
            .checked_sub(cost)
            .ok_or(Error::Limit("revision evidence bytes"))?;
        state.evidence.push(RevisionModifier {
            source,
            sprm: property.sprm,
            operand: property.operand.clone(),
        });
        return Ok(());
    }
    match property.sprm {
        // CIstd and CPlain preserve these four properties; CIstdPermute uses CIstd semantics.
        // CMajority's enumerated reset properties also exclude them (MS-DOC §2.6.1).
        0x4a30 | 0xca31 | 0x2a33 | 0xca47 => {}
        0x6865 | 0xca72
            if crate::office::doc::hidden_visibility::appearance::character_border(
                property.sprm,
                &property.operand,
            ) => {}
        // These properties concern other formatting or revision metadata, and are retained
        // in the source CharacterSpan. CFVanish remains unresolved in the result header.
        0x6815 | 0x6816 | 0x6817 | 0x0835 | 0x4a43 | 0x4a61 | 0x0806 | 0x4a50 | 0x486e | 0x286f
        | 0x4874 | 0x4a4f | 0x4a51 | 0x4a5e | 0x2a42 | 0x6870 | 0x486d | 0x0875 | 0x2a3e
        | 0x083c | 0x085c | 0x484b | 0x0836 | 0x4852 | 0x4873 | 0x0882 | 0x8840 | 0x083b
        | 0x2a86 | 0x2a48 | 0x4866 | 0xca71 | 0x2a0c | 0x085d | 0x0811 | 0x083a | 0x485f
        | 0x080a | 0x0856 | 0x4845 | 0x085a | 0xc81a | 0x4804 | 0x6805 | 0x4863 | 0x6864
        | 0xca89 | 0x0868 | 0x0837 | 0x484e | 0x2859 | 0xca76 | 0x6877 | 0x0818 | 0x0858
        | 0x0838 | 0x0839 | 0x2a53 | 0x0854 | 0x2a90 | 0x2879 => {}
        _ => record_unresolved(
            state,
            format!("unresolved character modifier 0x{:04x}", property.sprm),
            budget,
        )?,
    }
    Ok(())
}

fn record_unresolved(state: &mut RevisionSpan, reason: String, budget: &mut usize) -> Result<()> {
    let cost = std::mem::size_of::<String>() + reason.len();
    *budget = budget
        .checked_sub(cost)
        .ok_or(Error::Limit("revision evidence bytes"))?;
    state.unresolved.push(reason);
    Ok(())
}

pub(crate) fn piece_modifiers(prm: u16, groups: &[Vec<u8>]) -> Result<Vec<PropertyModifier>> {
    if prm == 0 {
        return Ok(Vec::new());
    }
    if prm & 1 != 0 {
        let group = groups
            .get((prm >> 1) as usize)
            .ok_or(Error::Invalid("revision piece property group"))?;
        return decode_character_modifiers(group);
    }
    let index = (prm >> 1) & 0x7f;
    let sprm = match index {
        0x41 => 0x0800,
        0x42 => 0x0801,
        0x43 => 0x0802,
        0x75 => 0x0855,
        0x00 => 0x2879,
        0x47 => 0x0806,
        0x4b => 0x080a,
        0x4d => 0x2a0c,
        0x4e => 0x0858,
        0x4f => 0x2859,
        0x50 => 0x0811,
        0x51 => 0x0818,
        0x53 => 0x2a33,
        0x55 => 0x0835,
        0x56 => 0x0836,
        0x57 => 0x0837,
        0x58 => 0x0838,
        0x59 => 0x0839,
        0x5a => 0x083a,
        0x5b => 0x083b,
        0x5c => 0x083c,
        0x5e => 0x2a3e,
        0x62 => 0x2a42,
        0x68 => 0x2a48,
        0x73 => 0x2a53,
        0x74 => 0x0854,
        0x76 => 0x0856,
        0x7b => 0x2a90,
        0x7c => 0x2a86,
        // Paragraph PRMs are applied in the paragraph-property algorithm, separately.
        0x04 | 0x05 | 0x07 | 0x08 | 0x09 | 0x0c | 0x0d | 0x0e | 0x0f | 0x18 | 0x19 | 0x1d
        | 0x25 | 0x2c | 0x32 | 0x33 | 0x35 | 0x36 | 0x37 | 0x38 | 0x39 | 0x3a | 0x78 | 0x7e => {
            return Ok(Vec::new());
        }
        _ => return Err(Error::Unsupported("revision piece PRM0 modifier")),
    };
    Ok(vec![PropertyModifier {
        sprm,
        operand: vec![(prm >> 8) as u8],
    }])
}

#[cfg(test)]
mod tests;
