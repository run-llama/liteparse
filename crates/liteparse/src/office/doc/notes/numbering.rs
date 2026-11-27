use super::*;
use crate::office::doc::formatting::PropertyModifier;

#[derive(Debug, Serialize)]
pub struct LegacyNumbering {
    pub raw_footnote_start_restart: u16,
    pub raw_endnote_start_restart: u16,
    pub footnote_format: u16,
    pub endnote_format: u16,
}
#[derive(Debug, Serialize)]
pub struct NoteSections {
    pub sections: Vec<NoteSection>,
    pub basis: &'static str,
    pub legacy_numbering: Option<LegacyNumbering>,
}
#[derive(Debug, Serialize)]
pub struct NoteSection {
    pub start_cp: usize,
    pub end_cp: usize,
    pub footnotes: Numbering,
    pub endnotes: Numbering,
    pub properties: Vec<PropertyModifier>,
    pub unresolved: Vec<&'static str>,
}
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Numbering {
    pub restart: u8,
    pub offset: u16,
    pub format: u16,
}

/// Decode section numbering for the final property view, retaining the ordered SEPX evidence.
/// Legacy header flags: <https://download.microsoft.com/download/0/b/e/0be8bdd7-e5e8-422a-abfd-4342ed7ad886/word97-2007binaryfileformat%28doc%29specification.pdf>
/// MS-DOC §2.6.4 defines continuous offsets and section defaults for post-Word-2000 files.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/46c3ec54-53ff-4c0a-b0d6-07ad15d2546e>
pub fn inspect_note_sections(data: &[u8], source: &Inspection) -> Result<NoteSections> {
    let c = Compound::open(data)?;
    let word = c.stream("WordDocument")?;
    let (version, table_name, _, _, _) = fib(&word)?;
    if version != source.binary_version {
        return Err(Error::Invalid("note numbering source version"));
    }
    let table = c.stream(table_name)?;
    let body = source
        .stories
        .iter()
        .find(|s| s.kind == "body")
        .ok_or(Error::Invalid("note body story"))?;
    if body.start_cp != 0 {
        return Err(Error::Invalid("note section body CP origin"));
    }
    let plc = fib_range(&word, &table, 6)?;
    let mut sections = decode_sections(&word, plc, body)?;
    if version <= 0xd9 {
        apply_legacy_numbering(&mut sections, fib_range(&word, &table, 31)?)?;
    }
    Ok(sections)
}

fn decode_sections(word: &[u8], plc: &[u8], body: &Story) -> Result<NoteSections> {
    if plc.len() < 20 || !(plc.len() - 4).is_multiple_of(16) {
        return Err(Error::Invalid("note section PLC"));
    }
    let count = (plc.len() - 4) / 16;
    if count > 65536 || body.utf16_units > 16 * 1024 * 1024 {
        return Err(Error::Limit("note sections"));
    }
    let units: Vec<_> = body.text.encode_utf16().collect();
    if units.len() != body.utf16_units || u32_at(plc, 0)? != 0 {
        return Err(Error::Invalid("note section source"));
    }
    let mut sections = Vec::new();
    let mut budget = 8 * 1024 * 1024usize;
    for i in 0..count {
        let start_cp = u32_at(plc, i * 4)? as usize;
        let end = u32_at(plc, (i + 1) * 4)? as usize;
        if start_cp >= end
            || start_cp >= units.len()
            || (i + 1 < count && (end >= units.len() || units[end - 1] != 0x0c))
            || (i + 1 == count && end < units.len())
        {
            return Err(Error::Invalid("note section range"));
        }
        let fc = u32_at(plc, (count + 1) * 4 + i * 12 + 2)? as usize;
        if fc == u32::MAX as usize {
            return Err(Error::Unsupported("absent section property record"));
        }
        let length = u16_at(word, fc)? as usize;
        if length > 32767 {
            return Err(Error::Invalid("section property length"));
        }
        budget = budget
            .checked_sub(length)
            .ok_or(Error::Limit("section property bytes"))?;
        let properties = decode_section_properties(bytes(word, fc + 2, length)?)?;
        budget = budget
            .checked_sub(properties.len() * std::mem::size_of::<PropertyModifier>())
            .ok_or(Error::Limit("section property metadata"))?;
        let mut section = NoteSection {
            start_cp,
            end_cp: end.min(units.len()),
            footnotes: Numbering {
                restart: 0,
                offset: 1,
                format: 0,
            },
            endnotes: Numbering {
                restart: 0,
                offset: 1,
                format: 2,
            },
            properties,
            unresolved: Vec::new(),
        };
        for p in &section.properties {
            match p.sprm {
                0xd1ff
                    if crate::office::doc::formatting::section::ignored_d1ff(
                        p.sprm, &p.operand,
                    ) => {}
                0x303c => section.footnotes.restart = p.operand[0],
                0x303e => section.endnotes.restart = p.operand[0],
                0x503f => section.footnotes.offset = u16_at(&p.operand, 0)?,
                0x5041 => section.endnotes.offset = u16_at(&p.operand, 0)?,
                0x5040 => section.footnotes.format = u16_at(&p.operand, 0)?,
                0x5042 => section.endnotes.format = u16_at(&p.operand, 0)?,
                0xd202 | 0xd238 => {
                    if !crate::office::doc::formatting::section::outline_operand(p.sprm, &p.operand)
                    {
                        section.unresolved.push("invalid_outline_section_operand");
                    }
                }
                // Word 97-2007 specification p. 63 defines 0x3014 as header/footer presence flags.
                0x3014 => {}
                // These section operators affect page geometry, decoration, positioning or revision metadata.
                // SWall preserves the old property view; subsequent operators define the accepted view.
                0x3000 | 0x3001 | 0xf203 | 0xf204 | 0x3005 | 0x3006 | 0x5007 | 0x5008 | 0x3009
                | 0x300a | 0x500b | 0x900c | 0x300e | 0x3011 | 0x3012 | 0x3013 | 0x5015
                | 0x9016 | 0xb017 | 0xb018 | 0x3019 | 0x301a | 0x501b | 0x501c | 0x301d
                | 0xb01f | 0xb020 | 0xb021 | 0xb022 | 0x9023 | 0x9024 | 0xb025 | 0x5026
                | 0x3228 | 0x322a | 0x702b | 0x702c | 0x702d | 0x702e | 0x522f | 0x7030
                | 0x9031 | 0x5032 | 0x5033 | 0xd234 | 0xd235 | 0xd236 | 0xd237 | 0x3239
                | 0x703a | 0x303b | 0xd243 | 0x7044 => {}
                _ => {
                    if section.unresolved.is_empty() {
                        section.unresolved.push("unknown_section_operator");
                    }
                }
            }
        }
        if section.footnotes.restart > 2
            || section.endnotes.restart > 1
            || section.footnotes.offset > 16383
            || section.endnotes.offset > 16383
        {
            return Err(Error::Invalid("note numbering property"));
        }
        sections.push(section);
    }
    Ok(NoteSections {
        sections,
        basis: "modern_section_final_property_view",
        legacy_numbering: None,
    })
}

fn decode_section_properties(group: &[u8]) -> Result<Vec<PropertyModifier>> {
    let mut offset = 0;
    let mut result = Vec::new();
    while offset < group.len() {
        let sprm = u16_at(group, offset)?;
        if (sprm >> 10) & 7 != 4 {
            return Err(Error::Unsupported("non-section SEPX modifier"));
        }
        offset += 2;
        let length = match sprm >> 13 {
            0 | 1 => 1,
            2 | 4 | 5 => 2,
            3 => 4,
            6 => 1 + bytes(group, offset, 1)?[0] as usize,
            7 => 3,
            _ => unreachable!(),
        };
        result.push(PropertyModifier {
            sprm,
            operand: bytes(group, offset, length)?.to_vec(),
        });
        offset += length;
    }
    Ok(result)
}

/// Word 97/2000 use document-level start/restart fields, including the start value after restarts.
/// DopBase words at bytes 2/52 hold two restart bits followed by fourteen start-number bits.
/// Dop97 full-width format fields are bytes 492/494; the earlier four-bit copies are ignored.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/f885b87a-15b3-460e-aecc-213bd17f960e>
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/2feeb178-f901-4151-bf5c-8496d5a0700b>
fn apply_legacy_numbering(sections: &mut NoteSections, dop: &[u8]) -> Result<()> {
    bytes(dop, 0, 500)?;
    let legacy = LegacyNumbering {
        raw_footnote_start_restart: u16_at(dop, 2)?,
        raw_endnote_start_restart: u16_at(dop, 52)?,
        footnote_format: u16_at(dop, 492)?,
        endnote_format: u16_at(dop, 494)?,
    };
    let footnotes = Numbering {
        restart: (legacy.raw_footnote_start_restart & 3) as u8,
        offset: legacy.raw_footnote_start_restart >> 2,
        format: legacy.footnote_format,
    };
    let endnotes = Numbering {
        restart: (legacy.raw_endnote_start_restart & 3) as u8,
        offset: legacy.raw_endnote_start_restart >> 2,
        format: legacy.endnote_format,
    };
    if footnotes.restart > 2 || endnotes.restart > 2 {
        return Err(Error::Invalid("legacy note restart mode"));
    }
    for section in &mut sections.sections {
        section.footnotes = footnotes;
        section.endnotes = endnotes;
    }
    sections.legacy_numbering = Some(legacy);
    sections.basis = "legacy_document_note_numbering";
    Ok(())
}

#[cfg(test)]
mod tests;
