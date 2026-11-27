//! Ordinary hidden-text properties for an accepted source view.
//!
//! MS-DOC §2.6.1 defines CFVanish, CIstd and CPlain; §2.9.327 makes relative
//! toggles depend on the style baseline. This module does not infer visibility
//! from field-hidden or revision properties.
//! <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/7022285b-9621-42e9-ad4d-4e02c115ef18>
use crate::office::doc::{
    Error, Result,
    formatting::{PropertyModifier, ToggleOperand, decode_character_modifiers},
    styles::Stylesheet,
};
use serde::Serialize;

pub(crate) mod appearance;

const MAX_STYLE_DEPTH: usize = 4094;
const MAX_PROPERTY_WORK: usize = 8 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct HiddenProperty {
    pub hidden: Option<bool>,
    pub paragraph_style: Option<usize>,
    pub character_style: usize,
    pub unresolved: Vec<&'static str>,
}

fn toggle(property: &PropertyModifier, baseline: Option<bool>) -> Result<Option<bool>> {
    Ok(match property.toggle()? {
        Some(ToggleOperand::Off) => Some(false),
        Some(ToggleOperand::On) => Some(true),
        Some(ToggleOperand::Style) => baseline,
        Some(ToggleOperand::NotStyle) => baseline.map(|value| !value),
        None => return Err(Error::Invalid("hidden property toggle")),
    })
}

/// Resolve a run once its paragraph style and table contribution are established.
/// `paragraph_base` is the hidden property before paragraph-style application;
/// callers pass None when table conditional formatting remains unresolved.
/// Direct modifiers must already be ordered CHPX followed by the piece's PRM.
pub fn resolve_hidden_property(
    styles: &Stylesheet,
    paragraph_style: usize,
    paragraph_base: Option<bool>,
    direct: &[PropertyModifier],
) -> Result<HiddenProperty> {
    let mut budget = MAX_PROPERTY_WORK;
    resolve_with_budget(styles, paragraph_style, paragraph_base, direct, &mut budget)
}

fn resolve_with_budget(
    styles: &Stylesheet,
    paragraph_style: usize,
    paragraph_base: Option<bool>,
    direct: &[PropertyModifier],
    budget: &mut usize,
) -> Result<HiddenProperty> {
    resolve_toggle_with_budget(
        styles,
        paragraph_style,
        paragraph_base,
        direct,
        budget,
        0x083c,
    )
}

// Visibility and capitalization share style-relative toggles and CPlain resets.
// CMajority affects ordinary hidden text and capitalization, but not CFSpecVanish.
pub(crate) fn resolve_capitalization_toggle(
    styles: &Stylesheet,
    paragraph_style: usize,
    paragraph_base: Option<bool>,
    direct: &[PropertyModifier],
    budget: &mut usize,
    target: u16,
) -> Result<HiddenProperty> {
    if !matches!(target, 0x083a | 0x083b) {
        return Err(Error::Invalid("capitalization toggle selector"));
    }
    let mut result = resolve_toggle_with_budget(
        styles,
        paragraph_style,
        paragraph_base,
        direct,
        budget,
        target,
    )
    .map_err(|error| match error {
        Error::Invalid(_) => Error::Invalid("capitalization property or style"),
        Error::Unsupported(_) => Error::Unsupported("capitalization property or style"),
        Error::Limit(_) => Error::Limit("capitalization property work"),
    })?;
    for reason in &mut result.unresolved {
        if *reason == "style_or_table_hidden_property_unresolved" {
            *reason = "style_or_table_capitalization_unresolved";
        }
    }
    Ok(result)
}

pub(crate) fn capitalization_piece_properties(
    prm: u16,
    groups: &[Vec<u8>],
    budget: &mut usize,
) -> Result<Vec<PropertyModifier>> {
    paragraph::piece(prm, groups, 2, budget)
}

fn resolve_toggle_with_budget(
    styles: &Stylesheet,
    paragraph_style: usize,
    paragraph_base: Option<bool>,
    direct: &[PropertyModifier],
    budget: &mut usize,
    target: u16,
) -> Result<HiddenProperty> {
    let paragraph = style_toggle(styles, paragraph_style, 1, paragraph_base, budget, target)?;
    let initial = if matches!(target, 0x083a | 0x083b) {
        style_toggle(styles, 10, 2, paragraph, budget, target)?
    } else {
        paragraph
    };
    let mut state = HiddenProperty {
        hidden: initial,
        paragraph_style: styles
            .styles
            .get(paragraph_style)
            .and_then(Option::as_ref)
            .map(|_| paragraph_style),
        character_style: 10,
        unresolved: Vec::new(),
    };
    let mut baseline = initial;
    let mut unsupported = false;
    for property in direct {
        charge(budget, property.operand.len() + 2)?;
        match property.sprm {
            sprm if sprm == target => state.hidden = toggle(property, baseline)?,
            0x2a83 if matches!(target, 0x083a | 0x083b) => {
                unsupported = true;
                state
                    .unresolved
                    .push("capitalization_revision_history_unresolved");
            }
            0x4a30 => {
                let [lo, hi] = property.operand.as_slice() else {
                    return Err(Error::Invalid("hidden character style operand"));
                };
                state.character_style = u16::from_le_bytes([*lo, *hi]) as usize;
                baseline =
                    style_toggle(styles, state.character_style, 2, paragraph, budget, target)?;
                state.hidden = baseline;
            }
            0x2a33 => {
                if property.operand != [0] {
                    return Err(Error::Invalid("hidden plain operand"));
                }
                state.character_style = 10;
                baseline = paragraph;
                state.hidden = paragraph;
            }
            0xca47 if state.character_style != 10 || target == 0x0818 => {}
            0xca31 | 0xca47 => {
                unsupported = true;
                state
                    .unresolved
                    .push("character_style_permutation_or_majority");
            }
            _ if neutral_property(property) || (target != 0x083c && property.sprm == 0x083c) => {}
            _ => {
                unsupported = true;
                state.unresolved.push("unrecognized_character_property");
            }
        }
    }
    if unsupported {
        state.hidden = None;
    }
    if state.hidden.is_none() && state.unresolved.is_empty() {
        state
            .unresolved
            .push("style_or_table_hidden_property_unresolved");
    }
    state.unresolved.sort_unstable();
    state.unresolved.dedup();
    Ok(state)
}

fn charge(budget: &mut usize, bytes: usize) -> Result<()> {
    *budget = budget
        .checked_sub(bytes)
        .ok_or(Error::Limit("hidden property work"))?;
    Ok(())
}

fn style_toggle(
    sheet: &Stylesheet,
    index: usize,
    kind: u8,
    baseline: Option<bool>,
    budget: &mut usize,
    target: u16,
) -> Result<Option<bool>> {
    let mut current = Some(index);
    let mut seen = Vec::new();
    let mut hidden = Vec::new();
    while let Some(index) = current {
        charge(budget, seen.len() + 1)?;
        if seen.len() >= MAX_STYLE_DEPTH || seen.contains(&index) {
            return Err(Error::Invalid("hidden style inheritance cycle"));
        }
        seen.push(index);
        let Some(style) = sheet.styles.get(index).and_then(Option::as_ref) else {
            return Ok(None);
        };
        if style.kind != kind || style.revision_marked {
            return Ok(None);
        }
        for group in style
            .properties
            .iter()
            .filter(|group| group.kind == "character")
        {
            charge(budget, group.bytes.len() + 1)?;
            for property in decode_character_modifiers(&group.bytes)? {
                if property.sprm == target {
                    hidden.push(property);
                } else if (matches!(target, 0x083a | 0x083b) && property.sprm == 0x2a83)
                    || !(neutral_property(&property) || target != 0x083c && property.sprm == 0x083c)
                {
                    return Ok(None);
                }
            }
        }
        current = style.base_style;
    }
    // The modern style-array wording and the historical UPE merge algorithm differ
    // for repeated inherited operators. Keep those cases explicit until an independent
    // source fixture establishes their precedence; unique operators are unambiguous.
    if hidden.len() > 1 {
        return Ok(None);
    }
    hidden
        .first()
        .map_or(Ok(baseline), |property| toggle(property, baseline))
}

// Border operands are checked before they can preserve style-relative text state.
fn neutral_property(property: &PropertyModifier) -> bool {
    match property.sprm {
        0x6865 | 0xca72 => appearance::character_border(property.sprm, &property.operand),
        _ => unrelated(property.sprm),
    }
}

// MS-DOC §2.6.1 assigns these operators properties other than ordinary hidden text.
// CIstd, CPlain, CIstdPermute and CMajority are handled separately because they reset it.
fn unrelated(sprm: u16) -> bool {
    matches!(
        sprm,
        0x0800
            | 0x0801
            | 0x0802
            | 0x0855
            | 0x6a03
            | 0x6a09
            | 0x2a83
            | 0x6815
            | 0x6816
            | 0x6817
            | 0x0835
            | 0x4a43
            | 0x4a61
            | 0x0806
            | 0x4a50
            | 0x486e
            | 0x286f
            | 0x4874
            | 0x4a4f
            | 0x4a51
            | 0x4a5e
            | 0x2a42
            | 0x6870
            | 0x486d
            | 0x0875
            | 0x2a3e
            | 0x085c
            | 0x484b
            | 0x0836
            | 0x4852
            | 0x4873
            | 0x0882
            | 0x8840
            | 0x083b
            | 0x2a86
            | 0x2a48
            | 0x4866
            | 0xca71
            | 0x2a0c
            | 0x085d
            | 0x0811
            | 0x083a
            | 0x485f
            | 0x080a
            | 0x0856
            | 0x4845
            | 0x085a
            | 0xc81a
            | 0x4804
            | 0x6805
            | 0x4863
            | 0x6864
            | 0xca89
            | 0x0868
            | 0x0837
            | 0x484e
            | 0x2859
            | 0xca76
            | 0x6877
            | 0x0818
            | 0x0858
            | 0x0838
            | 0x0839
            | 0x2a53
            | 0x0854
            | 0x2a90
            | 0x2879
    )
}

#[cfg(test)]
mod tests;

mod inspection;
pub use inspection::{
    HiddenEvidence, HiddenSpan, resolve_hidden_evidence, resolve_hidden_evidence_with_tables,
};
mod projection;
pub use projection::apply_hidden;

mod paragraph;

mod table;
pub use table::{TableHiddenEvidence, TableHiddenRow};
