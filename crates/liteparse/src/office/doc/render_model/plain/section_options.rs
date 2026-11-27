//! Retained section metadata whose layout effects are explicitly unimplemented.
use crate::office::doc::{
    Error, Result, formatting::PropertyModifier, render_model::sections::SectionGeometry,
};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SectionOption {
    pub modifier_index: usize,
    pub name: &'static str,
    pub property: PropertyModifier,
}

/// Bind each accepted unresolved issue to its original ordered section operator.
pub fn retain(
    geometry: &SectionGeometry,
    properties: &[PropertyModifier],
) -> Result<Vec<SectionOption>> {
    if !super::qualified_line_grid_dimensions(geometry) {
        return Err(unsupported());
    }
    let mut retained = Vec::new();
    for issue in &geometry.issues {
        if qualified_grid_issue(geometry, issue.reason) {
            continue;
        }
        let index = issue.modifier_index.ok_or_else(unsupported)?;
        let property = properties.get(index).ok_or_else(unsupported)?;
        if issue.reason != "section_property_unresolved" || issue.sprm != Some(property.sprm) {
            return Err(unsupported());
        }
        retained.push(SectionOption {
            modifier_index: index,
            name: name(property)?,
            property: property.clone(),
        });
    }
    validate(geometry, &retained)?;
    Ok(retained)
}

/// Geometry failures remain errors even when unrelated known options are retained.
pub fn validate(geometry: &SectionGeometry, retained: &[SectionOption]) -> Result<()> {
    if !super::qualified_line_grid_dimensions(geometry) || retained.len() > 8192 {
        return Err(unsupported());
    }
    let issues = geometry
        .issues
        .iter()
        .filter(|issue| !qualified_grid_issue(geometry, issue.reason));
    if retained.len() != issues.clone().count() {
        return Err(unsupported());
    }
    let mut previous = None;
    for (issue, option) in issues.zip(retained) {
        if issue.reason != "section_property_unresolved"
            || issue.modifier_index != Some(option.modifier_index)
            || issue.sprm != Some(option.property.sprm)
            || previous.is_some_and(|i| i >= option.modifier_index)
            || name(&option.property)? != option.name
        {
            return Err(unsupported());
        }
        previous = Some(option.modifier_index);
    }
    Ok(())
}

// Only the qualified lines-only grid consumes this issue; mode zero still rejects it.
fn qualified_grid_issue(geometry: &SectionGeometry, reason: &str) -> bool {
    geometry.grid_mode.value == 2
        && super::qualified_line_grid_dimensions(geometry)
        && reason == "active_document_grid_layout_unresolved"
}

fn unsupported() -> Error {
    Error::Unsupported("render unqualified section layout property")
}

// MS-DOC 2.6.4 defines these operands; SPgbPropOperand defines its three enums.
// Legacy header presence uses the six role bits already interpreted by headers.
fn name(property: &PropertyModifier) -> Result<&'static str> {
    let b = property.operand.as_slice();
    if paragraph_border(property)?.is_some() {
        return Ok("page_border");
    }
    match (property.sprm, b) {
        (0x3013, [0..=2]) => Ok("line_number_restart"),
        (0x5015, [a, b]) if u16::from_le_bytes([*a, *b]) <= 100 => Ok("line_number_interval"),
        (0x501b, [_, _]) => Ok("line_number_start"),
        (0x3011, [0 | 1]) => Ok("page_number_restart"),
        (0x3012, [0 | 1]) => Ok("endnote_section_placement"),
        (0x3014, [flags]) if *flags <= 0x3f => Ok("legacy_header_presence"),
        (0x501c, [_, _]) => Ok("page_number_start_legacy"),
        (0x7044, [a, b, c, d]) if u32::from_le_bytes([*a, *b, *c, *d]) <= 2147483646 => {
            Ok("page_number_start")
        }
        (0x9016, [a, b]) if u16::from_le_bytes([*a, *b]) <= 31680 => Ok("line_number_offset"),
        (0x522f, [flags, 0]) if flags & 7 <= 2 && (flags >> 3) & 3 <= 1 && flags >> 5 <= 1 => {
            Ok("page_border_options")
        }
        _ => Err(unsupported()),
    }
}

/// Page and paragraph borders share Brc80/Brc operand grammar. Preserve the
/// source opcode in the retained option while validating through that grammar.
pub fn paragraph_border(property: &PropertyModifier) -> Result<Option<PropertyModifier>> {
    let sprm = match property.sprm {
        0x702b..=0x702e => 0x6424 + (property.sprm - 0x702b),
        0xd234..=0xd237 => 0xc64e + (property.sprm - 0xd234),
        _ => return Ok(None),
    };
    let mapped = PropertyModifier {
        sprm,
        operand: property.operand.clone(),
    };
    crate::office::doc::render_model::paragraphs::presentation::decode(&mapped)?;
    Ok(Some(mapped))
}

#[cfg(test)]
mod tests;
