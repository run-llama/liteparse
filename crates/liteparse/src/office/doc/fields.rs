//! Source-indexed field metadata; resolved character properties remain required.
use crate::office::doc::{Error, Result, Story, u32_at};
use serde::Serialize;

const MAX_EVENTS: usize = 262_144;
const MAX_DEPTH: usize = 1024;
const MAX_DIAGNOSTICS: usize = 256;

/// Zero-based FibRgFcLcb97 pair indices, shared by the supported binary revisions.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/0c9df81f-98d0-454e-ad84-b612cd05b1a4>
pub fn fib_pair_index(story_kind: &str) -> Option<usize> {
    match story_kind {
        "body" => Some(16),
        "headers" => Some(17),
        "footnotes" => Some(18),
        "comments" => Some(19),
        "endnotes" => Some(48),
        "textboxes" => Some(57),
        "header_textboxes" => Some(59),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldCharacter {
    Begin,
    Separator,
    End,
}

#[derive(Debug, Serialize)]
pub struct FieldEvent {
    pub cp: usize,
    pub kind: FieldCharacter,
    /// Begin: cached field type. End: grffldEnd flags. Separator: unused.
    pub metadata: u8,
}

/// Half-open absolute UTF-16 character positions in the original source.
#[derive(Debug, Serialize)]
pub struct CpRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Serialize)]
pub struct FieldRange {
    pub source: CpRange,
    pub instruction: CpRange,
    pub result: Option<CpRange>,
    pub parent_begin_cp: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct FieldDiagnostic {
    pub code: &'static str,
    pub cp: usize,
}

#[derive(Debug, Serialize)]
pub struct FieldTable {
    pub status: &'static str,
    pub events: Vec<FieldEvent>,
    /// Present only when the indexed event topology is fully balanced.
    pub ranges: Vec<FieldRange>,
    pub diagnostics: Vec<FieldDiagnostic>,
    pub topology_valid: bool,
    /// Some field kinds are intentionally absent from Plcfld; these require CFSpec.
    pub unindexed_marker_units: usize,
}

/// Decodes a story's Plcfld and cross-checks each indexed character against source text.
/// MS-DOC 2.8.25 excludes XE, TC, RD, TA and PRIVATE from this table; missing entries
/// therefore leave control semantics unresolved. Even indexed fields need resolved CFSpec.
/// Structural errors return an error; topology errors retain events with diagnostics.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/751b09bb-72f0-45ef-8e87-666dea68219f>
pub fn decode(story: &Story, plc: &[u8]) -> Result<FieldTable> {
    let total_units = story.text.encode_utf16().count();
    if total_units != story.utf16_units || story.start_cp.checked_add(total_units).is_none() {
        return Err(Error::Invalid("field story character positions"));
    }
    let n = if plc.is_empty() {
        0
    } else if plc.len() >= 4 && (plc.len() - 4).is_multiple_of(6) {
        (plc.len() - 4) / 6
    } else {
        return Err(Error::Invalid("field PLC size"));
    };
    if n > MAX_EVENTS {
        return Err(Error::Limit("field PLC events"));
    }
    let mut events = Vec::with_capacity(n);
    let mut previous = None;
    let mut units = story.text.encode_utf16().enumerate().peekable();
    for i in 0..n {
        let relative_cp = u32_at(plc, i * 4)? as usize;
        if previous.is_some_and(|p| relative_cp <= p) || relative_cp >= total_units {
            return Err(Error::Invalid("field PLC character positions"));
        }
        previous = Some(relative_cp);
        let offset = (n + 1) * 4 + i * 2;
        // fldch stores the marker in five bits; its upper three bits are reserved.
        let marker = plc[offset] & 0x1f;
        let kind = match marker {
            0x13 => FieldCharacter::Begin,
            0x14 => FieldCharacter::Separator,
            0x15 => FieldCharacter::End,
            _ => return Err(Error::Invalid("field PLC marker type")),
        };
        while units.peek().is_some_and(|(cp, _)| *cp < relative_cp) {
            units.next();
        }
        if units.next() != Some((relative_cp, u16::from(marker))) {
            return Err(Error::Invalid("field PLC source mismatch"));
        }
        events.push(FieldEvent {
            cp: story.start_cp + relative_cp,
            kind,
            metadata: plc[offset + 1],
        });
    }
    // The terminal CP is an ordering sentinel; its other semantics are undefined.
    if !plc.is_empty() && previous.is_some_and(|p| u32_at(plc, n * 4).unwrap() as usize <= p) {
        return Err(Error::Invalid("field PLC terminal position"));
    }
    let marker_units = story
        .text
        .chars()
        .filter(|ch| matches!(ch, '\u{13}'..='\u{15}'))
        .count();
    let mut table = FieldTable {
        status: "indexed_fields_cf_spec_unresolved",
        unindexed_marker_units: marker_units - events.len(),
        events,
        ranges: Vec::new(),
        diagnostics: Vec::new(),
        topology_valid: true,
    };
    topology(&mut table)?;
    Ok(table)
}

struct OpenField {
    cp: usize,
    separator: Option<usize>,
    parent: Option<usize>,
}

fn topology(table: &mut FieldTable) -> Result<()> {
    let mut stack: Vec<OpenField> = Vec::new();
    for event in &table.events {
        let code = match event.kind {
            FieldCharacter::Begin => {
                if stack.len() == MAX_DEPTH {
                    return Err(Error::Limit("field PLC nesting"));
                }
                stack.push(OpenField {
                    cp: event.cp,
                    separator: None,
                    parent: stack.last().map(|f| f.cp),
                });
                None
            }
            FieldCharacter::Separator => match stack.last_mut() {
                Some(field) if field.separator.is_none() => {
                    field.separator = Some(event.cp);
                    None
                }
                Some(_) => Some("duplicate_field_separator"),
                None => Some("orphan_field_separator"),
            },
            FieldCharacter::End => match stack.pop() {
                Some(field) => {
                    table.ranges.push(FieldRange {
                        source: CpRange {
                            start: field.cp,
                            end: event.cp + 1,
                        },
                        instruction: CpRange {
                            start: field.cp + 1,
                            end: field.separator.unwrap_or(event.cp),
                        },
                        result: field.separator.map(|cp| CpRange {
                            start: cp + 1,
                            end: event.cp,
                        }),
                        parent_begin_cp: field.parent,
                    });
                    None
                }
                None => Some("orphan_field_end"),
            },
        };
        if let Some(code) = code {
            table.topology_valid = false;
            if table.diagnostics.len() < MAX_DIAGNOSTICS {
                table
                    .diagnostics
                    .push(FieldDiagnostic { code, cp: event.cp });
            }
        }
    }
    for field in stack {
        table.topology_valid = false;
        if table.diagnostics.len() < MAX_DIAGNOSTICS {
            table.diagnostics.push(FieldDiagnostic {
                code: "unclosed_field",
                cp: field.cp,
            });
        }
    }
    if !table.topology_valid {
        table.ranges.clear();
    }
    Ok(())
}

#[cfg(test)]
mod tests;
