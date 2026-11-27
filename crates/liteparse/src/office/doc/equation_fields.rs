//! Indexed EQ literals and overstrikes with absent or empty cached results.
use crate::office::doc::{
    Error, Inspection, Result, Transcription, field_display_visibility as visibility,
    field_inspection::FieldInspection,
    hidden_visibility::HiddenEvidence,
    ruby_fields::RubyInspection,
    source_literal_projection::{self, Candidate},
    visibility::RevisionEvidence,
};
use serde::Serialize;
mod composite;
const MAX_FIELDS: usize = 16_384;

#[derive(Debug, Serialize)]
pub struct EquationInspection {
    pub status: &'static str,
    pub fields: Vec<EquationField>,
    pub unresolved: Vec<&'static str>,
}
#[derive(Debug, Serialize)]
pub struct EquationField {
    pub story: &'static str,
    pub field_range: [usize; 2],
    pub separator_cp: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub composite: Option<composite::Composite>,
    pub literal: Option<String>,
    pub literal_source_range: Option<[usize; 2]>,
    pub eligible: bool,
    pub diagnostic: Option<String>,
    pub projected_literal_utf8_range: Option<[usize; 2]>,
}

/// Microsoft RTF1.9.1, p.197 documents null EQ results requiring instruction interpretation.
/// Literal and bounded overstrike operands require indexed source and visibility checks.
/// <https://officeprotocoldoc.z19.web.core.windows.net/files/Archive_References/%5bMSFT-RTF%5d.pdf>
pub fn inspect_equations(
    source: &Inspection,
    fields: &FieldInspection,
    revisions: &RevisionEvidence,
    hidden: &HiddenEvidence,
) -> Result<EquationInspection> {
    if fields.binary_version != source.binary_version {
        return Err(Error::Invalid("equation source version"));
    }
    let mut result = EquationInspection {
        status: "partial_indexed_empty_result_equations",
        fields: Vec::new(),
        unresolved: vec![
            "equation_visual_placement",
            "unsupported_composite_equation_grammar",
        ],
    };
    let mut budget = 4 * 1024 * 1024usize;
    for story in &source.stories {
        if story.utf16_units > 16 * 1024 * 1024 {
            return Err(Error::Limit("equation story units"));
        }
        let units: Vec<u16> = story.text.encode_utf16().collect();
        if units.len() != story.utf16_units {
            return Err(Error::Invalid("equation story units"));
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
            return Err(Error::Limit("equation indexed fields"));
        }
        for field in &table.ranges {
            if field
                .result
                .as_ref()
                .is_some_and(|cache| cache.start != cache.end)
            {
                continue;
            }
            let start = field
                .instruction
                .start
                .checked_sub(story.start_cp)
                .ok_or(Error::Invalid("equation instruction range"))?;
            let end = field
                .instruction
                .end
                .checked_sub(story.start_cp)
                .ok_or(Error::Invalid("equation instruction range"))?;
            let raw = units
                .get(start..end)
                .ok_or(Error::Invalid("equation instruction bounds"))?;
            let prefix: String = raw
                .iter()
                .take(64)
                .map(|u| char::from_u32(*u as u32).unwrap_or('\u{fffd}'))
                .collect();
            let name = prefix.trim_start();
            if !name.get(..2).is_some_and(|s| s.eq_ignore_ascii_case("EQ"))
                || !name
                    .get(2..)
                    .is_some_and(|s| s.starts_with(char::is_whitespace))
            {
                continue;
            }
            // Ruby instructions have their own typed guide records and share the same final pass.
            if name[2..].trim_start().strip_prefix("\\*").is_some_and(|s| {
                s.trim_start()
                    .get(..2)
                    .is_some_and(|v| v.eq_ignore_ascii_case("jc"))
            }) {
                continue;
            }
            if result.fields.len() >= MAX_FIELDS {
                return Err(Error::Limit("equation field records"));
            }
            let mut item = EquationField {
                story: story.kind,
                field_range: [field.source.start, field.source.end],
                separator_cp: field.result.as_ref().map(|_| field.instruction.end),
                composite: None,
                literal: None,
                literal_source_range: None,
                eligible: false,
                diagnostic: None,
                projected_literal_utf8_range: None,
            };
            let decoded = (|| {
                if raw.len() > 16_384 {
                    return Err(Error::Limit("equation instruction units"));
                }
                budget = budget
                    .checked_sub(raw.len() * 2)
                    .ok_or(Error::Limit("equation metadata bytes"))?;
                let code = String::from_utf16(raw)
                    .map_err(|_| Error::Invalid("equation instruction UTF16"))?;
                let ranges = if let Some(range) =
                    crate::office::doc::transcription::equation_display_range(&code)
                {
                    let start =
                        field.instruction.start + code[..range.start].encode_utf16().count();
                    let source_range = [start, start + code[range.clone()].encode_utf16().count()];
                    item.literal = Some(code[range].to_owned());
                    item.literal_source_range = Some(source_range);
                    vec![source_range]
                } else {
                    let value = composite::parse(&code, field.instruction.start)?;
                    let ranges = value.operands.iter().map(|o| o.source_range).collect();
                    item.composite = Some(value);
                    ranges
                };
                if field.parent_begin_cp.is_some() || raw.iter().any(|u| matches!(u, 0x13..=0x15)) {
                    return Err(Error::Unsupported("equation nested field"));
                }
                visibility::validate_layout(field, story, &units)?;
                visibility::visible(field, revisions, hidden, &mut budget)?;
                for range in ranges {
                    visibility::plain(range, revisions, &mut budget)?;
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

/// Project all eligible EQ components together, preserving both annotation sets' final offsets.
/// Missing metadata does not disable the other independently inspected component set.
pub fn apply_fields(
    input: &Transcription,
    source: &Inspection,
    mut ruby: Option<&mut RubyInspection>,
    mut equations: Option<&mut EquationInspection>,
) -> Result<Transcription> {
    let mut candidates = Vec::new();
    let mut owners = Vec::new();
    if let Some(ruby) = &ruby {
        if ruby.fields.len() > MAX_FIELDS {
            return Err(Error::Limit("ruby projection fields"));
        }
        for (index, f) in ruby.fields.iter().enumerate() {
            if f.projected_base_utf8_range.is_some() {
                return Err(Error::Invalid("ruby projection already applied"));
            }
            if !f.eligible {
                continue;
            }
            let c = f
                .components
                .as_ref()
                .ok_or(Error::Invalid("ruby missing components"))?;
            candidates.push(Candidate {
                story: f.story,
                field_range: f.field_range,
                source_range: c.base_source_range,
                text: &c.base,
            });
            owners.push((true, index, None));
        }
    }
    if let Some(equations) = &equations {
        if equations.fields.len() > MAX_FIELDS {
            return Err(Error::Limit("equation projection fields"));
        }
        for (index, f) in equations.fields.iter().enumerate() {
            if f.projected_literal_utf8_range.is_some() {
                return Err(Error::Invalid("equation projection already applied"));
            }
            if !f.eligible {
                continue;
            }
            if let Some(composite) = &f.composite {
                for (operand, value) in composite.operands.iter().enumerate() {
                    candidates.push(Candidate {
                        story: f.story,
                        field_range: f.field_range,
                        source_range: value.source_range,
                        text: &value.text,
                    });
                    owners.push((false, index, Some(operand)));
                }
                continue;
            }
            candidates.push(Candidate {
                story: f.story,
                field_range: f.field_range,
                source_range: f
                    .literal_source_range
                    .ok_or(Error::Invalid("equation literal range"))?,
                text: f
                    .literal
                    .as_deref()
                    .ok_or(Error::Invalid("equation literal"))?,
            });
            owners.push((false, index, None));
        }
    }
    let mut projection = source_literal_projection::project(input, source, &candidates)?;
    drop(candidates);
    let mut ruby_projected = false;
    let mut equation_projected = false;
    for (candidate, range) in projection.projected {
        let (is_ruby, index, operand) = owners[candidate];
        if is_ruby {
            ruby.as_mut().unwrap().fields[index].projected_base_utf8_range = Some(range);
            ruby_projected = true;
        } else {
            let field = &mut equations.as_mut().unwrap().fields[index];
            if let Some(operand) = operand {
                field.composite.as_mut().unwrap().operands[operand].projected_utf8_range =
                    Some(range);
                field.projected_literal_utf8_range =
                    Some(field.projected_literal_utf8_range.map_or(range, |prior| {
                        [prior[0].min(range[0]), prior[1].max(range[1])]
                    }));
                field.diagnostic =
                    Some("equation_overstrike_operands_linearized_layout_unresolved".into());
            } else {
                field.projected_literal_utf8_range = Some(range);
            }
            equation_projected = true;
        }
    }
    for candidate in projection.skipped {
        let (is_ruby, index, _) = owners[candidate];
        if is_ruby {
            ruby.as_mut().unwrap().fields[index].diagnostic =
                Some("ruby existing output or ownership".into());
        } else {
            equations.as_mut().unwrap().fields[index].diagnostic =
                Some("equation existing output or ownership".into());
        }
    }
    let mut resolved = std::collections::BTreeMap::new();
    if let Some(ruby) = &ruby {
        for field in &ruby.fields {
            if field.projected_base_utf8_range.is_some() {
                resolved.insert((field.story, field.field_range[0]), false);
            }
        }
    }
    if let Some(equations) = &equations {
        for field in &equations.fields {
            if field.projected_literal_utf8_range.is_some() {
                resolved.insert(
                    (field.story, field.field_range[0]),
                    field.composite.is_some(),
                );
            }
        }
    }
    // Replace only a diagnostic whose indexed field actually published source operands.
    // Composite output retains an explicit layout-uncertainty diagnostic at that owner.
    for story in &mut projection.transcription.stories {
        story.diagnostics.retain(|d| {
            d.code != "equation_display_unresolved" || !resolved.contains_key(&(story.kind, d.cp))
        });
        for (&(kind, cp), &composite) in &resolved {
            if kind == story.kind && composite {
                story
                    .diagnostics
                    .push(crate::office::doc::transcription::Diagnostic {
                        code: "equation_overstrike_linearized_layout_unresolved",
                        cp,
                    });
            }
        }
    }
    if ruby_projected {
        projection
            .transcription
            .unresolved
            .push("ruby_guide_annotation_visual_placement_unresolved");
    }
    if equation_projected {
        projection
            .transcription
            .unresolved
            .push("equation_literal_visual_placement_unresolved");
    }
    Ok(projection.transcription)
}

#[cfg(test)]
mod tests;
