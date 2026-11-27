//! CIstd resets presentation to the paragraph baseline, then applies a character style.
use super::*;

pub(super) fn select(
    state: &mut RunProperties,
    paragraph: &RunProperties,
    styles: &Stylesheet,
    fonts: &FontTable,
    property: &PropertyModifier,
    work: &mut usize,
) -> Result<()> {
    let [lo, hi] = property.operand.as_slice() else {
        return Err(Error::Invalid("render character style operand"));
    };
    let index = u16::from_le_bytes([*lo, *hi]) as usize;
    let mut next = Some(index);
    let mut seen = std::collections::BTreeSet::new();
    let mut layers = Vec::new();
    while let Some(index) = next {
        if !seen.insert(index) || seen.len() > 4094 {
            return Err(Error::Invalid("render character style ancestry"));
        }
        charge(work, 1)?;
        let style = styles
            .styles
            .get(index)
            .and_then(Option::as_ref)
            .ok_or(Error::Invalid("render selected character style"))?;
        if style.index != index
            || style.kind != 2
            || style.revision_marked
            || !style.raw_revision.is_empty()
        {
            return Err(Error::Unsupported(
                "render character style identity or revision",
            ));
        }
        if let Some(partner) = style.linked_style
            && !styles
                .styles
                .get(partner)
                .and_then(Option::as_ref)
                .is_some_and(|p| p.index == partner && p.kind == 1)
        {
            return Err(Error::Invalid("render linked paragraph style"));
        }
        let mut properties = Vec::new();
        for group in &style.properties {
            charge(work, group.bytes.len() + 1)?;
            if group.kind != "character" {
                return Err(Error::Unsupported("render character style property group"));
            }
            properties.extend(decode_character_modifiers(&group.bytes)?);
        }
        layers.push(properties);
        next = style.base_style;
    }
    charge(work, evidence_bytes(paragraph) + evidence_bytes(state))?;
    let mut result = paragraph.clone();
    // CIstd preserves direction, hint and highlighting through reset and style application.
    // Revision and special-marker flags require separate source-owned qualification.
    // Other preservation exceptions remain guarded by the closed property grammar.
    result.bidi = state.bidi;
    result.hint = state.hint;
    result.highlight_rgb = state.highlight_rgb;
    result.applied = state.applied.clone();
    result.applied.push(property.clone());
    for layer in layers.iter().rev() {
        charge(work, evidence_bytes(&result))?;
        let baseline = result.clone();
        for modifier in layer {
            validate_font(modifier, fonts.fonts.len())?;
            apply(&mut result, modifier, &baseline)?;
            result.bidi = state.bidi;
            result.hint = state.hint;
            result.highlight_rgb = state.highlight_rgb;
        }
    }
    *state = result;
    Ok(())
}

pub(super) fn charge_state(work: &mut usize, state: &RunProperties) -> Result<()> {
    charge(work, evidence_bytes(state))
}

fn evidence_bytes(state: &RunProperties) -> usize {
    state
        .applied
        .iter()
        .chain(&state.complex_property_evidence)
        .map(|p| p.operand.len() + 3)
        .sum()
}

pub(super) fn charge(work: &mut usize, amount: usize) -> Result<()> {
    *work = work
        .checked_add(amount)
        .filter(|n| *n <= 8 * 1024 * 1024)
        .ok_or(Error::Limit("render character style work"))?;
    Ok(())
}

#[cfg(test)]
mod tests;
