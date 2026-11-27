//! Provisional source projection; formatting and revision visibility remain unresolved.
use crate::office::doc::{Inspection, Story};
use serde::Serialize;
use std::{collections::BTreeMap, ops::Range};

// Per-story caps bound metadata expansion; limit diagnostics preserve remaining raw text.
const MAX_FIELD_DEPTH: usize = 1024;
const MAX_DIAGNOSTICS: usize = 256;
const MAX_SEGMENTS: usize = 262_144;

#[derive(Debug, Serialize)]
pub struct Transcription {
    pub schema_version: u32,
    pub status: &'static str,
    pub unresolved: Vec<&'static str>,
    pub stories: Vec<TranscribedStory>,
}

#[derive(Debug, Serialize)]
pub struct TranscribedStory {
    pub kind: &'static str,
    pub text: String,
    pub segments: Vec<SourceSegment>,
    pub diagnostics: Vec<Diagnostic>,
    pub fields_balanced: bool,
    pub unresolved_control_units: usize,
}

/// Half-open source ranges count UTF-16 units; output ranges count UTF-8 bytes.
/// Segments partition the source, including suppressed instructions and field markers.
/// Generated list labels use explicit zero-width anchors alongside that source partition.
#[derive(Debug, Serialize)]
pub struct SourceSegment {
    pub kind: &'static str,
    pub start_cp: usize,
    pub end_cp: usize,
    pub output_start: usize,
    pub output_end: usize,
}

#[derive(Debug, Serialize)]
pub struct Diagnostic {
    pub code: &'static str,
    pub cp: usize,
}

/// Projects cached field results and structural separators from raw stories.
/// Field markers are provisional until Plcfld and sprmCFSpec are decoded; this output
/// cannot establish accepted revisions, hidden text, table structure or story placement.
/// Malformed fields preserve instruction text. Resource limits retain an unprojected tail.
/// Source control mappings follow Microsoft Word 97-2007 Binary File Format, Text (p. 29).
/// MS-DOC 2.8.25 defines nested field instructions/results and optional separators:
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/751b09bb-72f0-45ef-8e87-666dea68219f>
pub fn transcribe(inspection: &Inspection) -> Transcription {
    let mut unresolved = vec![
        "character_properties_and_revision_visibility",
        "field_marker_properties_and_plcfld",
        "field_result_freshness",
        "table_structure_and_list_labels",
        "story_placement_and_object_content",
    ];
    if !inspection.trailing_utf16_units.is_empty() {
        unresolved.push("trailing_source_units_retained_in_inspection");
    }
    Transcription {
        schema_version: 1,
        status: "partial_transcription",
        unresolved,
        stories: inspection.stories.iter().map(project_story).collect(),
    }
}

fn field_diagnostics(story: &Story) -> Vec<Diagnostic> {
    let mut stack = Vec::new();
    let mut diagnostics = Vec::new();
    let mut cp = story.start_cp;
    for ch in story.text.chars() {
        if diagnostics.len() == MAX_DIAGNOSTICS {
            diagnostics.push(Diagnostic {
                code: "diagnostic_limit",
                cp,
            });
            return diagnostics;
        }
        let code = match ch {
            '\u{13}' => {
                if stack.len() == MAX_FIELD_DEPTH {
                    diagnostics.push(Diagnostic {
                        code: "field_depth_limit",
                        cp,
                    });
                    return diagnostics;
                }
                stack.push((cp, false));
                None
            }
            '\u{14}' => match stack.last_mut() {
                Some((_, separated)) if !*separated => {
                    *separated = true;
                    None
                }
                Some(_) => Some("duplicate_field_separator"),
                None => Some("orphan_field_separator"),
            },
            '\u{15}' if stack.pop().is_none() => Some("orphan_field_end"),
            _ => None,
        };
        if let Some(code) = code {
            diagnostics.push(Diagnostic { code, cp });
        }
        cp += ch.len_utf16();
    }
    diagnostics.extend(
        stack
            .into_iter()
            .take(MAX_DIAGNOSTICS.saturating_sub(diagnostics.len()))
            .map(|(cp, _)| Diagnostic {
                code: "unclosed_field",
                cp,
            }),
    );
    diagnostics
}

// Some fields encode visible text in their instructions without a cached result.
// ECMA-376 17.16.5.34 defines MACROBUTTON; MS-OE376 §2.1.502 defines EQ alignment.
fn literal_field_displays(
    story: &Story,
    diagnostics: &mut Vec<Diagnostic>,
) -> BTreeMap<usize, Range<usize>> {
    let mut stack: Vec<(usize, usize, bool, bool, Option<usize>)> = Vec::new();
    let mut displays = BTreeMap::new();
    let mut cp = story.start_cp;
    for (offset, ch) in story.text.char_indices() {
        match ch {
            '\u{13}' => {
                if let Some(parent) = stack.last_mut() {
                    parent.3 = true;
                }
                stack.push((offset, cp, false, false, None));
            }
            '\u{14}' => {
                let field = stack.last_mut().unwrap();
                field.2 = true;
                field.4 = Some(offset);
            }
            '\u{15}' => {
                let (start, begin_cp, separated, nested, separator) = stack.pop().unwrap();
                let instruction_end = separator.unwrap_or(offset);
                let instruction = story.text[start + 1..instruction_end].trim_start();
                let listnum = instruction.eq_ignore_ascii_case("LISTNUM")
                    || has_field_name(instruction, "LISTNUM");
                // Word suppresses nested LISTNUM. A top-level empty result still needs
                // source list/sequence resolution before its visible number is available.
                if listnum
                    && stack.is_empty()
                    && separator.is_none_or(|value| value + 1 == offset)
                    && diagnostics.len() < MAX_DIAGNOSTICS
                {
                    diagnostics.push(Diagnostic {
                        code: "listnum_empty_result_unresolved",
                        cp: begin_cp,
                    });
                }
                if !separated {
                    let code = &story.text[start + 1..offset];
                    let trimmed = code.trim_start();
                    let is_macro = has_field_name(trimmed, "MACROBUTTON");
                    let is_equation = has_field_name(trimmed, "EQ");
                    if is_macro || is_equation {
                        let display = if nested || (is_equation && !stack.is_empty()) {
                            None
                        } else if is_macro {
                            macro_display_range(code)
                        } else {
                            equation_display_range(code)
                        };
                        if let Some(range) = display {
                            if displays.len() < MAX_SEGMENTS {
                                displays
                                    .insert(start, start + 1 + range.start..start + 1 + range.end);
                            } else if diagnostics.len() < MAX_DIAGNOSTICS {
                                diagnostics.push(Diagnostic {
                                    code: if is_macro {
                                        "macrobutton_display_limit"
                                    } else {
                                        "equation_display_limit"
                                    },
                                    cp: begin_cp,
                                });
                            }
                        } else if diagnostics.len() < MAX_DIAGNOSTICS {
                            diagnostics.push(Diagnostic {
                                code: if is_macro {
                                    "macrobutton_display_unresolved"
                                } else {
                                    "equation_display_unresolved"
                                },
                                cp: begin_cp,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
        cp += ch.len_utf16();
    }
    displays
}

fn has_field_name(code: &str, name: &str) -> bool {
    code.get(..name.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(name))
        && code
            .get(name.len()..)
            .is_some_and(|rest| rest.starts_with(char::is_whitespace))
}

fn macro_display_range(code: &str) -> Option<Range<usize>> {
    let code_start = code.len() - code.trim_start().len();
    let keyword_end = code[code_start..].find(char::is_whitespace)? + code_start;
    let rest = &code[keyword_end..];
    let macro_start = keyword_end + rest.len() - rest.trim_start().len();
    let macro_rest = &code[macro_start..];
    let macro_end = if let Some(quoted) = macro_rest.strip_prefix('"') {
        let end = quoted.find('"')?;
        if end == 0 {
            return None;
        }
        macro_start + 1 + end + 1
    } else {
        macro_start + macro_rest.find(char::is_whitespace)?
    };
    if code[macro_start..macro_end].contains('\\') || macro_end == macro_start {
        return None;
    }
    let remaining = &code[macro_end..];
    if !remaining.starts_with(char::is_whitespace) {
        return None;
    }
    let display = remaining.trim();
    let start = macro_end + remaining.len() - remaining.trim_start().len();
    if display.is_empty() || display.contains('\\') || display.chars().any(|c| c.is_control()) {
        return None;
    }
    if display.starts_with('"') && display.ends_with('"') && display.len() >= 2 {
        if display[1..display.len() - 1].contains('"') {
            return None;
        }
        Some(start + 1..start + display.len() - 1)
    } else if display.contains('"') {
        None
    } else {
        Some(start..start + display.len())
    }
}

// Overstrike overlays its operands; blank operands add no lexical content.
// Only one literal operand is projected; composite glyphs and nested equations stay unresolved.
// MS-OE376 §2.1.502(b) adds justified alignment (\ad) to left, center and right alignment.
// https://learn.microsoft.com/openspecs/office_standards/ms-oe376/1ed1a602-444f-40a4-a056-47f49f911a0a
pub(crate) fn equation_display_range(code: &str) -> Option<Range<usize>> {
    let expression = code.trim_start().get(2..)?.trim();
    if !expression.get(..2)?.eq_ignore_ascii_case("\\o") {
        return None;
    }
    let mut rest = expression[2..].trim_start();
    if rest.starts_with('\\') {
        let alignment = rest.get(..3)?;
        if !["\\al", "\\ac", "\\ar", "\\ad"]
            .iter()
            .any(|allowed| alignment.eq_ignore_ascii_case(allowed))
        {
            return None;
        }
        rest = rest[3..].trim_start();
    }
    let operands = rest.strip_prefix('(')?.strip_suffix(')')?;
    if operands
        .chars()
        .any(|c| c.is_control() || matches!(c, '\\' | '(' | ')' | ';' | '"'))
    {
        return None;
    }
    let mut offset = code.trim_end().len() - rest.len() + 1;
    let mut visible = None;
    for operand in operands.split(',') {
        if !operand.trim().is_empty() {
            if visible.is_some() {
                return None;
            }
            visible = Some(offset..offset + operand.len());
        }
        offset += operand.len() + 1;
    }
    visible
}

struct ProjectionField {
    visible: bool,
    display: Option<Range<usize>>,
}

fn project_story(story: &Story) -> TranscribedStory {
    let diagnostics = field_diagnostics(story);
    let mut result = TranscribedStory {
        kind: story.kind,
        text: String::new(),
        segments: Vec::new(),
        fields_balanced: diagnostics.is_empty(),
        diagnostics,
        unresolved_control_units: 0,
    };
    let displays = if result.fields_balanced {
        literal_field_displays(story, &mut result.diagnostics)
    } else {
        BTreeMap::new()
    };
    let mut stack: Vec<ProjectionField> = Vec::new();
    let mut instruction_depth = 0;
    let mut cp = story.start_cp;
    for (byte_offset, ch) in story.text.char_indices() {
        if result.segments.len() == MAX_SEGMENTS {
            let output_start = result.text.len();
            result.text.push_str(&story.text[byte_offset..]);
            result.segments.push(SourceSegment {
                kind: "unprojected_tail",
                start_cp: cp,
                end_cp: cp + story.text[byte_offset..].encode_utf16().count(),
                output_start,
                output_end: result.text.len(),
            });
            result.diagnostics.push(Diagnostic {
                code: "segment_limit",
                cp,
            });
            break;
        }
        if let Some(frame) = stack.last_mut()
            && let Some(display) = &frame.display
        {
            let visible = display.contains(&byte_offset);
            if visible != frame.visible {
                if visible {
                    instruction_depth -= 1;
                } else {
                    instruction_depth += 1;
                }
                frame.visible = visible;
            }
        }
        let mut output = Some(ch);
        let kind = if result.fields_balanced && matches!(ch, '\u{13}'..='\u{15}') {
            match ch {
                '\u{13}' => {
                    stack.push(ProjectionField {
                        visible: false,
                        display: displays.get(&byte_offset).cloned(),
                    });
                    instruction_depth += 1;
                }
                '\u{14}' => {
                    stack.last_mut().unwrap().visible = true;
                    instruction_depth -= 1;
                }
                _ => {
                    if !stack.pop().unwrap().visible {
                        instruction_depth -= 1;
                    }
                }
            }
            output = None;
            "field_marker"
        } else if instruction_depth > 0 {
            output = None;
            "field_instruction"
        } else {
            match ch {
                '\r' => {
                    output = Some('\n');
                    "paragraph"
                }
                '\t' => "tab",
                '\u{b}' => {
                    output = Some('\n');
                    "line_break"
                }
                '\u{c}' => "page_or_section_break",
                '\u{e}' => {
                    output = Some('\n');
                    "column_break"
                }
                '\u{7}' => {
                    // Table properties distinguish cell and row ends and nested tables.
                    output = Some('\t');
                    "cell_or_row_mark"
                }
                '\u{1e}' => {
                    output = Some('\u{2011}');
                    "nonbreaking_hyphen"
                }
                '\u{1f}' => {
                    output = Some('\u{ad}');
                    "optional_hyphen"
                }
                ch if ch.is_control() => {
                    // Notes, symbols and objects need properties or cross-story links.
                    // Keep an explicit placeholder and source span for unresolved content.
                    output = Some('\u{fffc}');
                    result.unresolved_control_units += ch.len_utf16();
                    "unresolved_control"
                }
                _ if stack.last().is_some_and(|f| f.display.is_some()) => "field_display_argument",
                _ => "text",
            }
        };
        let output_start = result.text.len();
        if let Some(ch) = output {
            result.text.push(ch);
        }
        let end_cp = cp + ch.len_utf16();
        if let Some(last) = result.segments.last_mut().filter(|s| s.kind == kind) {
            last.end_cp = end_cp;
            last.output_end = result.text.len();
        } else {
            result.segments.push(SourceSegment {
                kind,
                start_cp: cp,
                end_cp,
                output_start,
                output_end: result.text.len(),
            });
        }
        cp = end_cp;
    }
    result
}

#[cfg(test)]
mod tests;
