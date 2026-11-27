//! EQ phonetic guides preserve base text and pronunciation as separate source-owned components.
mod grammar;
mod projection;
use crate::office::doc::{
    Error, Inspection, Result, field_inspection::FieldInspection,
    hidden_visibility::HiddenEvidence, visibility::RevisionEvidence,
};
pub use projection::apply_ruby_fields;
use serde::Serialize;

const MAX_FIELDS: usize = 16_384;
const MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct RubyInspection {
    pub status: &'static str,
    pub fields: Vec<RubyField>,
    pub unresolved: Vec<&'static str>,
}
#[derive(Debug, Serialize)]
pub struct RubyField {
    pub story: &'static str,
    pub field_range: [usize; 2],
    pub components: Option<RubyComponents>,
    pub eligible: bool,
    pub diagnostic: Option<String>,
    pub projected_base_utf8_range: Option<[usize; 2]>,
}
#[derive(Debug, Serialize)]
pub struct RubyComponents {
    pub base: String,
    pub base_source_range: [usize; 2],
    pub guide: String,
    pub guide_source_range: [usize; 2],
    pub guide_font: String,
    pub guide_size_half_points: u16,
    pub shift_points: u16,
    pub guide_position: &'static str,
    pub alignment: u8,
    pub guide_coverage: &'static str,
}

/// Inspect literal ruby instructions through indexed fields and resolved source visibility.
/// Components remain available when projection is blocked; eligibility is independent of parsing.
pub fn inspect_ruby_fields(
    source: &Inspection,
    fields: &FieldInspection,
    revisions: &RevisionEvidence,
    hidden: &HiddenEvidence,
) -> Result<RubyInspection> {
    if fields.binary_version != source.binary_version {
        return Err(Error::Invalid("ruby source version"));
    }
    let mut result = RubyInspection {
        status: "partial_source_ruby_components",
        fields: Vec::new(),
        unresolved: vec![
            "ruby_visual_placement_and_font_rendering",
            "ruby_guide_retained_as_annotation",
        ],
    };
    let mut budget = MAX_BYTES;
    for story in &source.stories {
        if story.utf16_units > 16 * 1024 * 1024 {
            return Err(Error::Limit("ruby story units"));
        }
        let units: Vec<u16> = story.text.encode_utf16().collect();
        if units.len() != story.utf16_units {
            return Err(Error::Invalid("ruby story units"));
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
            return Err(Error::Limit("ruby indexed fields"));
        }
        for field in &table.ranges {
            let start = field
                .instruction
                .start
                .checked_sub(story.start_cp)
                .ok_or(Error::Invalid("ruby instruction range"))?;
            let end = field
                .instruction
                .end
                .checked_sub(story.start_cp)
                .ok_or(Error::Invalid("ruby instruction range"))?;
            let raw = units
                .get(start..end)
                .ok_or(Error::Invalid("ruby instruction bounds"))?;
            // An explicit ruby formatting switch distinguishes candidates from other EQ constructs.
            let prefix: String = raw
                .iter()
                .take(64)
                .map(|u| char::from_u32(*u as u32).unwrap_or('\u{fffd}'))
                .collect();
            if !prefix
                .trim_start()
                .get(..2)
                .is_some_and(|s| s.eq_ignore_ascii_case("EQ"))
                || !prefix.to_ascii_lowercase().contains("jc")
            {
                continue;
            }
            if result.fields.len() >= MAX_FIELDS {
                return Err(Error::Limit("ruby field records"));
            }
            let mut item = RubyField {
                story: story.kind,
                field_range: [field.source.start, field.source.end],
                components: None,
                eligible: false,
                diagnostic: None,
                projected_base_utf8_range: None,
            };
            let decoded = (|| {
                if raw.len() > 16_384 {
                    return Err(Error::Limit("ruby instruction units"));
                }
                budget = budget
                    .checked_sub(raw.len() * 2)
                    .ok_or(Error::Limit("ruby metadata bytes"))?;
                let text = String::from_utf16(raw)
                    .map_err(|_| Error::Invalid("ruby instruction UTF16"))?;
                let parsed = grammar::parse(&text)?;
                let range = |r: &std::ops::Range<usize>| {
                    let start = field.instruction.start + text[..r.start].encode_utf16().count();
                    [start, start + text[r.clone()].encode_utf16().count()]
                };
                item.components = Some(RubyComponents {
                    base: text[parsed.base.clone()].to_owned(),
                    base_source_range: range(&parsed.base),
                    guide: text[parsed.guide.clone()].to_owned(),
                    guide_source_range: range(&parsed.guide),
                    guide_font: parsed.font.to_owned(),
                    guide_size_half_points: parsed.half_points,
                    shift_points: parsed.shift,
                    guide_position: parsed.position,
                    alignment: parsed.alignment,
                    guide_coverage: "source_annotation_visual_placement_unresolved",
                });
                if field.parent_begin_cp.is_some() || raw.iter().any(|u| matches!(u, 0x13..=0x15)) {
                    return Err(Error::Unsupported("ruby nested field"));
                }
                crate::office::doc::field_display_visibility::validate_layout(
                    field, story, &units,
                )?;
                crate::office::doc::field_display_visibility::visible(
                    field,
                    revisions,
                    hidden,
                    &mut budget,
                )?;
                for range in [
                    item.components.as_ref().unwrap().base_source_range,
                    item.components.as_ref().unwrap().guide_source_range,
                ] {
                    crate::office::doc::field_display_visibility::plain(
                        range,
                        revisions,
                        &mut budget,
                    )?;
                }
                item.eligible = true;
                Ok(())
            })();
            if let Err(error) = decoded {
                item.diagnostic = Some(error.to_string().replace("literal field ", "ruby "));
            }
            result.fields.push(item);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
