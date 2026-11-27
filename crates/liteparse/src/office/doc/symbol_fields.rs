//! Source-owned logical characters from indexed SYMBOL instructions without cached text.
mod grammar;
mod projection;
use crate::office::doc::{
    Error, Inspection, Result, field_display_visibility, field_inspection::FieldInspection,
    fields::FieldCharacter, hidden_visibility::HiddenEvidence, symbols::FontTable,
    visibility::RevisionEvidence,
};
pub use projection::apply_symbol_fields;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct SymbolFields {
    pub status: &'static str,
    pub fields: Vec<SymbolField>,
}

#[derive(Debug, Serialize)]
pub struct SymbolField {
    story: &'static str,
    source_range: [usize; 2],
    instruction: String,
    source_code: Option<u16>,
    font_index: Option<usize>,
    unicode: Option<char>,
    mapping_basis: Option<&'static str>,
    eligible: bool,
    diagnostic: Option<String>,
}

/// Resolve logical SYMBOL values with explicit source font, field identity and visibility.
/// Published font encoding supplies character semantics; glyph appearance remains separate.
pub fn inspect(
    source: &Inspection,
    fields: &FieldInspection,
    fonts: &FontTable,
    revisions: &RevisionEvidence,
    hidden: &HiddenEvidence,
    embedded_fonts: Option<bool>,
) -> Result<SymbolFields> {
    if source.binary_version != fields.binary_version {
        return Err(Error::Invalid("symbol field source version"));
    }
    let mut result = SymbolFields {
        status: "partial_source_symbol_fields",
        fields: Vec::new(),
    };
    let mut budget = 4 * 1024 * 1024usize;
    let mut font_names = std::collections::BTreeMap::new();
    for (index, font) in fonts.fonts.iter().enumerate() {
        budget = budget
            .checked_sub(font.name.len() + 32)
            .ok_or(Error::Limit("symbol field font work"))?;
        if font.index != index {
            return Err(Error::Invalid("symbol field font index"));
        }
        font_names
            .entry(font.name.to_ascii_lowercase())
            .and_modify(|entry| *entry = None)
            .or_insert(Some(index));
    }
    for story in &source.stories {
        if story.utf16_units > 16 * 1024 * 1024 {
            return Err(Error::Limit("symbol field story units"));
        }
        let units: Vec<u16> = story.text.encode_utf16().collect();
        if units.len() != story.utf16_units {
            return Err(Error::Invalid("symbol field story units"));
        }
        let Some(table) = fields
            .stories
            .iter()
            .find(|s| s.kind == story.kind)
            .and_then(|s| s.table.as_ref())
            .filter(|t| t.topology_valid)
        else {
            continue;
        };
        if table.ranges.len() > 262_144 {
            return Err(Error::Limit("symbol field index"));
        }
        for field in &table.ranges {
            let start = field
                .instruction
                .start
                .checked_sub(story.start_cp)
                .ok_or(Error::Invalid("symbol field instruction start"))?;
            let end = field
                .instruction
                .end
                .checked_sub(story.start_cp)
                .ok_or(Error::Invalid("symbol field instruction end"))?;
            let raw = units
                .get(start..end)
                .ok_or(Error::Invalid("symbol field instruction bounds"))?;
            let prefix: String = raw
                .iter()
                .take(32)
                .map(|&u| char::from_u32(u as u32).unwrap_or('\u{fffd}'))
                .collect();
            let cached_symbol_type = table
                .events
                .get(table.events.partition_point(|e| e.cp < field.source.start))
                .is_some_and(|e| {
                    e.cp == field.source.start
                        && e.kind == FieldCharacter::Begin
                        && e.metadata == 57
                });
            if !cached_symbol_type
                && !prefix
                    .split_ascii_whitespace()
                    .next()
                    .is_some_and(|s| s.eq_ignore_ascii_case("SYMBOL"))
            {
                continue;
            }
            if result.fields.len() >= 16_384 {
                return Err(Error::Limit("symbol field count"));
            }
            let mut item = SymbolField {
                story: story.kind,
                source_range: [field.source.start, field.source.end],
                instruction: String::new(),
                source_code: None,
                font_index: None,
                unicode: None,
                mapping_basis: None,
                eligible: false,
                diagnostic: None,
            };
            let decoded = (|| {
                if raw.len() > 1024 {
                    return Err(Error::Limit("symbol field instruction units"));
                }
                budget = budget
                    .checked_sub(raw.len() * 2)
                    .ok_or(Error::Limit("symbol field metadata bytes"))?;
                item.instruction = String::from_utf16(raw)
                    .map_err(|_| Error::Invalid("symbol field instruction UTF16"))?;
                let instruction = grammar::parse(&item.instruction)?;
                item.source_code = Some(instruction.code);
                if field.parent_begin_cp.is_some() || raw.iter().any(|u| matches!(u, 0x13..=0x15)) {
                    return Err(Error::Unsupported("symbol nested field"));
                }
                field_display_visibility::validate_layout(field, story, &units)?;
                let event = |cp, kind| {
                    table
                        .events
                        .get(table.events.partition_point(|e| e.cp < cp))
                        .filter(|e| e.cp == cp && e.kind == kind)
                        .map(|e| e.metadata)
                };
                if event(field.source.start, FieldCharacter::Begin) != Some(57)
                    || !matches!(
                        event(field.source.end - 1, FieldCharacter::End),
                        Some(0 | 4)
                    )
                {
                    return Err(Error::Unsupported(
                        "symbol field indexed identity or result state",
                    ));
                }
                field_display_visibility::visible(field, revisions, hidden, &mut budget)?;
                field_display_visibility::plain(
                    [field.instruction.start, field.instruction.end],
                    revisions,
                    &mut budget,
                )?;
                if embedded_fonts != Some(false) {
                    return Err(Error::Unsupported("symbol field embedded font mapping"));
                }
                let index = font_names
                    .get(&instruction.font.to_ascii_lowercase())
                    .copied()
                    .flatten()
                    .ok_or(Error::Unsupported(
                        "symbol field source font absent or ambiguous",
                    ))?;
                let font = &fonts.fonts[index];
                if font.charset != 2 || font.alternate_name.is_some() {
                    return Err(Error::Unsupported("symbol field font identity"));
                }
                item.font_index = Some(font.index);
                let (unicode, basis) =
                    crate::office::doc::symbols::map_symbol(font, instruction.code);
                let (unicode, basis) = if font.name.eq_ignore_ascii_case("Symbol")
                    && matches!(instruction.code, 0xd3 | 0xe3)
                {
                    // Adobe names these copyright glyph variants; logical text retains ©.
                    // Source code distinguishes serif and sans appearance for future rendering.
                    (
                        Some('©'),
                        if instruction.code == 0xd3 {
                            "symbol_copyright_serif_logical_normalization"
                        } else {
                            "symbol_copyright_sans_logical_normalization"
                        },
                    )
                } else {
                    (unicode, basis)
                };
                item.unicode = unicode;
                item.mapping_basis = Some(basis);
                if unicode.is_none() {
                    return Err(Error::Unsupported("symbol field character mapping"));
                }
                item.eligible = true;
                Ok(())
            })();
            if let Err(error) = decoded {
                item.diagnostic = Some(error.to_string());
            }
            result.fields.push(item);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
