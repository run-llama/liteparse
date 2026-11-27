//! Source font identities and symbol characters, with unsupported encodings retained.
use crate::office::doc::visibility::RevisionEvidence;
use crate::office::doc::{Error, Result, bytes, compound::Compound, document::fib, u16_at, u32_at};
use crate::office::doc::{
    Inspection,
    transcription::{Diagnostic, SourceSegment, TranscribedStory, Transcription},
};
use serde::Serialize;

mod tables;

const MAX_FONT_BYTES: usize = 8 * 1024 * 1024;
const MAX_SYMBOL_SPANS: usize = 262_144;

#[derive(Debug, Serialize)]
pub struct SymbolEvidence {
    pub status: &'static str,
    pub spans: Vec<SymbolSpan>,
}

#[derive(Debug, Serialize)]
pub struct SymbolSpan {
    pub start_cp: usize,
    pub end_cp: usize,
    pub font_index: usize,
    pub source_code: u16,
    pub special: Option<bool>,
    pub unicode: Option<char>,
    pub basis: &'static str,
}

/// Resolve explicit CSymbol operands after character-property ordering is established.
/// CIstd, CPlain and CMajority preserve symbols; styles cannot introduce CSymbol.
/// Unsupported operators already invalidate the associated revision evidence.
pub fn resolve_symbol_evidence(
    fonts: &FontTable,
    revisions: &RevisionEvidence,
) -> Result<SymbolEvidence> {
    if revisions.spans.len() > MAX_SYMBOL_SPANS {
        return Err(Error::Limit("symbol evidence spans"));
    }
    let mut spans = Vec::new();
    for span in &revisions.spans {
        let Some(symbol) = span.evidence.iter().rev().find(|p| p.sprm == 0x6a09) else {
            continue;
        };
        if symbol.operand.len() != 4 {
            return Err(Error::Invalid("symbol operand size"));
        }
        let font_index = u16_at(&symbol.operand, 0)? as usize;
        let source_code = u16_at(&symbol.operand, 2)?;
        let special = span.properties.special;
        let (unicode, basis) = if special != Some(true) || !span.unresolved.is_empty() {
            (None, "special_character_state_unresolved_or_disabled")
        } else if let Some(font) = fonts
            .fonts
            .get(font_index)
            .filter(|f| f.index == font_index)
        {
            map_symbol(font, source_code)
        } else {
            (None, "font_index_unavailable")
        };
        spans.push(SymbolSpan {
            start_cp: span.start_cp,
            end_cp: span.end_cp,
            font_index,
            source_code,
            special,
            unicode,
            basis,
        });
    }
    Ok(SymbolEvidence {
        status: "partial_source_symbol_mapping",
        spans,
    })
}

/// Factual mappings from Adobe Symbol and Unicode WG2 N4384's source-reference table.
/// Unsupported private-use slots, ambiguous assignments and unspecified fonts retain source codes.
/// <https://unicode.org/Public/MAPPINGS/VENDORS/ADOBE/symbol.txt>
/// <https://www.unicode.org/wg2/docs/n4384.pdf>
/// Arrow extender identity: <https://www.unicode.org/L2/L1999/99346-mathbraces.htm>
/// Final assignment: <https://www.unicode.org/charts/PDF/U2300.pdf>
pub fn map_symbol(font: &Font, source: u16) -> (Option<char>, &'static str) {
    if font.charset != 2 {
        return (None, "non_symbol_charset_unresolved");
    }
    let code = match source {
        0x20..=0xff => source,
        0xf020..=0xf0ff => source - 0xf000,
        _ => return (None, "symbol_code_outside_supported_encoding"),
    };
    let name = font.name.to_ascii_lowercase();
    // L2/99-346 identifies PostScript arrowhorizex as HORIZONTAL LINE EXTENSION.
    // Unicode assigns that character U+23AF; it retains the extender's semantics.
    if name == "symbol" && code == 0xbe {
        return (Some('\u{23af}'), "unicode_standardized_arrow_extender");
    }
    let table = match name.as_str() {
        "symbol" => Some(&tables::SYMBOL),
        "wingdings" => Some(&tables::WINGDINGS),
        "wingdings 2" => Some(&tables::WINGDINGS_2),
        "wingdings 3" => Some(&tables::WINGDINGS_3),
        "webdings" => Some(&tables::WEBDINGS),
        _ => None,
    };
    let mapped = table.and_then(|t| char::from_u32(t[code as usize]).filter(|c| *c != '\0'));
    (
        mapped,
        if mapped.is_some() {
            "published_symbol_encoding"
        } else {
            "symbol_mapping_unresolved"
        },
    )
}

/// Replace explicit source symbol markers while preserving all unrelated source/output ranges.
/// Suppressed fields/revisions and generated-number segments retain their existing projection.
/// Unsupported mappings retain their source marker with explicit unresolved-symbol provenance.
pub fn apply_symbols(
    source: &Inspection,
    input: &Transcription,
    symbols: &SymbolEvidence,
) -> Result<Transcription> {
    if symbols.spans.len() > MAX_SYMBOL_SPANS {
        return Err(Error::Limit("symbol projection spans"));
    }
    if symbols.spans.iter().any(|s| s.start_cp >= s.end_cp)
        || symbols
            .spans
            .windows(2)
            .any(|w| w[0].end_cp > w[1].start_cp)
    {
        return Err(Error::Invalid("symbol projection span order"));
    }
    let mut stories = Vec::new();
    for story in &input.stories {
        let raw = source
            .stories
            .iter()
            .find(|s| s.kind == story.kind)
            .ok_or(Error::Invalid("symbol source story"))?;
        if raw.utf16_units > 16 * 1024 * 1024 {
            return Err(Error::Limit("symbol source units"));
        }
        let units: Vec<u16> = raw.text.encode_utf16().collect();
        if units.len() != raw.utf16_units {
            return Err(Error::Invalid("symbol source length"));
        }
        let mut output = TranscribedStory {
            kind: story.kind,
            text: String::new(),
            segments: Vec::new(),
            diagnostics: story
                .diagnostics
                .iter()
                .map(|d| Diagnostic {
                    code: d.code,
                    cp: d.cp,
                })
                .collect(),
            fields_balanced: story.fields_balanced,
            unresolved_control_units: story.unresolved_control_units,
        };
        let mut previous_cp = raw.start_cp;
        let mut previous_output = 0;
        for segment in &story.segments {
            if segment.start_cp != previous_cp
                || segment.end_cp <= segment.start_cp
                || segment.output_start != previous_output
            {
                return Err(Error::Invalid("symbol source segment partition"));
            }
            let text = story
                .text
                .get(segment.output_start..segment.output_end)
                .ok_or(Error::Invalid("symbol output offsets"))?;
            let first = symbols
                .spans
                .partition_point(|s| s.end_cp <= segment.start_cp);
            let intersects = symbols
                .spans
                .get(first)
                .is_some_and(|s| s.start_cp < segment.end_cp);
            if !intersects
                || text.is_empty()
                || !matches!(segment.kind, "text" | "unresolved_control")
            {
                append_symbol_text(
                    &mut output,
                    segment.kind,
                    segment.start_cp,
                    segment.end_cp,
                    text,
                )?;
            } else if text.encode_utf16().count() != segment.end_cp - segment.start_cp {
                symbol_diagnostic(
                    &mut output,
                    "symbol_source_mapping_unresolved",
                    segment.start_cp,
                );
                append_symbol_text(
                    &mut output,
                    segment.kind,
                    segment.start_cp,
                    segment.end_cp,
                    text,
                )?;
            } else {
                let mut cp = segment.start_cp;
                let mut index = first;
                for ch in text.chars() {
                    while index < symbols.spans.len() && symbols.spans[index].end_cp <= cp {
                        index += 1;
                    }
                    let end = cp + ch.len_utf16();
                    let symbol = symbols
                        .spans
                        .get(index)
                        .filter(|s| s.start_cp <= cp && end <= s.end_cp);
                    let (mapped, kind) = if let Some(symbol) = symbol {
                        let raw_unit = cp.checked_sub(raw.start_cp).and_then(|i| units.get(i));
                        if ch != '(' || raw_unit != Some(&0x28) || symbol.special == Some(false) {
                            symbol_diagnostic(&mut output, "symbol_source_marker_unresolved", cp);
                            (ch, segment.kind)
                        } else if let Some(mapped) =
                            symbol.unicode.filter(|_| symbol.special == Some(true))
                        {
                            (mapped, "symbol")
                        } else {
                            symbol_diagnostic(&mut output, "symbol_encoding_unresolved", cp);
                            (ch, "unresolved_symbol")
                        }
                    } else {
                        (ch, segment.kind)
                    };
                    append_symbol_text(
                        &mut output,
                        kind,
                        cp,
                        end,
                        mapped.encode_utf8(&mut [0; 4]),
                    )?;
                    cp = end;
                }
            }
            previous_cp = segment.end_cp;
            previous_output = segment.output_end;
        }
        if previous_cp != raw.start_cp + raw.utf16_units || previous_output != story.text.len() {
            return Err(Error::Invalid("symbol incomplete source partition"));
        }
        stories.push(output);
    }
    let mut unresolved = input.unresolved.clone();
    if symbols.spans.iter().any(|s| s.unicode.is_none()) {
        unresolved.push("unsupported_symbol_encodings");
    }
    Ok(Transcription {
        schema_version: input.schema_version,
        status: input.status,
        unresolved,
        stories,
    })
}

fn symbol_diagnostic(story: &mut TranscribedStory, code: &'static str, cp: usize) {
    if story.diagnostics.len() < 256 {
        story.diagnostics.push(Diagnostic { code, cp });
    }
}

fn append_symbol_text(
    story: &mut TranscribedStory,
    kind: &'static str,
    start_cp: usize,
    end_cp: usize,
    text: &str,
) -> Result<()> {
    let output_start = story.text.len();
    story.text.push_str(text);
    if let Some(last) = story
        .segments
        .last_mut()
        .filter(|s| s.kind == kind && s.end_cp == start_cp)
    {
        last.end_cp = end_cp;
        last.output_end = story.text.len();
    } else {
        if story.segments.len() >= MAX_SYMBOL_SPANS {
            return Err(Error::Limit("symbol projection segments"));
        }
        story.segments.push(SourceSegment {
            kind,
            start_cp,
            end_cp,
            output_start,
            output_end: story.text.len(),
        });
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct FontTable {
    pub status: &'static str,
    pub fonts: Vec<Font>,
}

#[derive(Debug, Serialize)]
pub struct Font {
    pub index: usize,
    pub name: String,
    pub alternate_name: Option<String>,
    pub charset: u8,
    pub family: u8,
    pub weight: u16,
    pub panose: Vec<u8>,
    pub signature: Vec<u8>,
}

/// Read FibRgFcLcb97 pair 15 (SttbfFfn) from the selected table stream.
pub fn inspect_symbol_fonts(data: &[u8]) -> Result<FontTable> {
    let container = Compound::open(data)?;
    let word = container.stream("WordDocument")?;
    let (_, table_name, _, _, _) = fib(&word)?;
    let table = container.stream(table_name)?;
    let offset = u32_at(&word, 154 + 15 * 8)? as usize;
    let length = u32_at(&word, 158 + 15 * 8)? as usize;
    if length == 0 {
        return Ok(FontTable {
            status: "source_font_table_absent",
            fonts: Vec::new(),
        });
    }
    decode_font_table(bytes(&table, offset, length)?)
}

/// Decode non-extended SttbfFfn records, whose one-byte lengths count FFN bytes.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/18b7d35b-ad29-4723-893b-82aa30c64ced>
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/ff407d64-3478-4b56-9b98-6dbcfc66a4ae>
pub fn decode_font_table(data: &[u8]) -> Result<FontTable> {
    decode_fonts(data, true)
}

/// [`decode_font_table`] for best-effort conversion: a record with an unknown charset
/// keeps its name, with charset 0 (ANSI), instead of failing the whole table.
pub fn decode_font_table_lenient(data: &[u8]) -> Result<FontTable> {
    decode_fonts(data, false)
}

fn decode_fonts(data: &[u8], strict: bool) -> Result<FontTable> {
    if data.len() > MAX_FONT_BYTES {
        return Err(Error::Limit("font table bytes"));
    }
    let count = u16_at(data, 0)? as usize;
    if count > 0x7ff0 || u16_at(data, 2)? != 0 {
        return Err(Error::Invalid("font table header"));
    }
    let mut offset = 4;
    let mut fonts = Vec::with_capacity(count);
    for index in 0..count {
        let length = bytes(data, offset, 1)?[0] as usize;
        offset += 1;
        let record = bytes(data, offset, length)?;
        offset += length;
        if strict && (length < 41 || !(length - 39).is_multiple_of(2)) {
            return Err(Error::Invalid("font record size"));
        }
        match font_record(index, record, strict) {
            Ok(font) => fonts.push(font),
            Err(e) if strict => return Err(e),
            Err(_) => fonts.push(salvage_font(index, record)),
        }
    }
    if offset != data.len() {
        return Err(Error::Invalid("font table trailing data"));
    }
    Ok(FontTable {
        status: "source_font_table",
        fonts,
    })
}

/// One FFN record of the font table; `strict` rejects unknown charsets.
fn font_record(index: usize, record: &[u8], strict: bool) -> Result<Font> {
    if record.len() < 41 || !(record.len() - 39).is_multiple_of(2) {
        return Err(Error::Invalid("font record size"));
    }
    let weight = u16_at(record, 1)?;
    if weight > 1000 {
        return Err(Error::Invalid("font weight"));
    }
    let mut charset = record[3];
    // 77-89 are the Mac script charsets (Roman, Japanese, Korean, Chinese, ...),
    // written by Word for Mac.
    if ![
        0, 1, 2, 128, 129, 134, 136, 255, 130, 177, 178, 161, 162, 163, 222, 238, 204, 186,
    ]
    .contains(&charset)
        && !(77..=89).contains(&charset)
    {
        if strict {
            return Err(Error::Invalid("font charset"));
        }
        charset = 0;
    }
    let names: Vec<u16> = record[39..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    let end = names
        .iter()
        .position(|&c| c == 0)
        .ok_or(Error::Invalid("font name terminator"))?;
    if end == 0 {
        return Err(Error::Invalid("empty font name"));
    }
    let name =
        String::from_utf16(&names[..end]).map_err(|_| Error::Invalid("font name unicode"))?;
    let alternate = record[4] as usize;
    let alternate_name = if alternate == 0 {
        if end + 1 != names.len() {
            return Err(Error::Invalid("font name trailing data"));
        }
        None
    } else {
        if alternate != end + 1
            || alternate + 1 >= names.len()
            || names.last() != Some(&0)
            || names[alternate..names.len() - 1].contains(&0)
        {
            return Err(Error::Invalid("alternate font name"));
        }
        Some(
            String::from_utf16(&names[alternate..names.len() - 1])
                .map_err(|_| Error::Invalid("alternate font unicode"))?,
        )
    };
    Ok(Font {
        index,
        name,
        alternate_name,
        charset,
        family: record[0],
        weight,
        panose: record[5..15].to_vec(),
        signature: record[15..39].to_vec(),
    })
}

/// A record [`font_record`] rejects, kept for best-effort conversion: its first
/// readable name (the primary, else the alternate), with charset 0.
fn salvage_font(index: usize, record: &[u8]) -> Font {
    let units: Vec<u16> = record
        .get(39..)
        .unwrap_or(&[])
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    let name = units
        .split(|&c| c == 0)
        .find(|n| !n.is_empty())
        .map(String::from_utf16_lossy)
        .unwrap_or_default();
    Font {
        index,
        name,
        alternate_name: None,
        charset: 0,
        family: record.first().copied().unwrap_or(0),
        weight: u16_at(record, 1).unwrap_or(400),
        panose: record.get(5..15).map_or_else(Vec::new, <[u8]>::to_vec),
        signature: record.get(15..39).map_or_else(Vec::new, <[u8]>::to_vec),
    }
}

#[cfg(test)]
mod tests;
