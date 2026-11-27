//! Font-encoded private-use characters with source-owned Unicode transcription.
//!
//! Standard CHP initialization and Appendix B of the Microsoft Word binary format
//! specification initialize idctHint to zero and select rgFtc2 for shared PUA text.
//! <https://download.microsoft.com/download/0/b/e/0be8bdd7-e5e8-422a-abfd-4342ed7ad886/word97-2007binaryfileformat%28doc%29specification.pdf>
//! Current character operators: <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/7022285b-9621-42e9-ad4d-4e02c115ef18>
use crate::office::doc::{
    Error, Inspection, Result, Transcription,
    formatting::{PropertyModifier, decode_character_modifiers},
    formatting_inspection::FormattingInspection,
    hidden_visibility::HiddenEvidence,
    paragraph_formatting::ParagraphInspection,
    styles::Stylesheet,
    symbols::FontTable,
    visibility::RevisionEvidence,
};
use serde::Serialize;
const MAX: usize = 262_144;
const WORK: usize = 8 * 1024 * 1024;

pub struct Context<'a> {
    pub source: &'a Inspection,
    pub embedded_fonts: Option<bool>,
    pub formatting: &'a FormattingInspection,
    pub paragraphs: &'a ParagraphInspection,
    pub styles: &'a Stylesheet,
    pub fonts: &'a FontTable,
    pub revisions: &'a RevisionEvidence,
    pub hidden: &'a HiddenEvidence,
}
#[derive(Debug, Serialize)]
pub struct OrdinarySymbols {
    pub status: &'static str,
    pub spans: Vec<OrdinarySymbol>,
    pub embedded_font_table_present: Option<bool>,
}
#[derive(Debug, Serialize)]
pub struct OrdinarySymbol {
    pub story: &'static str,
    pub source_cp: usize,
    pub source_code: u16,
    pub font_index: Option<usize>,
    pub paragraph_style: Option<usize>,
    pub unicode: Option<char>,
    pub basis: &'static str,
    pub unresolved: Option<&'static str>,
}

/// Establish whether the source references embedded font data before using published encodings.
/// FibRgFcLcb97 pair 61 is SttbTtmbd; present data may replace a named font's glyphs.
pub fn inspect_embedded_fonts(data: &[u8]) -> Result<bool> {
    let c = crate::office::doc::compound::Compound::open(data)?;
    let word = c.stream("WordDocument")?;
    let (_, table_name, _, _, _) = crate::office::doc::document::fib(&word)?;
    if crate::office::doc::u16_at(&word, 152)? <= 61 {
        return Err(Error::Invalid("embedded font FIB entry missing"));
    }
    let offset = crate::office::doc::u32_at(&word, 154 + 61 * 8)? as usize;
    let length = crate::office::doc::u32_at(&word, 158 + 61 * 8)? as usize;
    if length == 0 {
        return Ok(false);
    }
    let table = c.stream(table_name)?;
    crate::office::doc::bytes(&table, offset, length)?;
    Ok(true)
}

fn charge(work: &mut usize, n: usize) -> Result<()> {
    *work = work
        .checked_sub(n)
        .ok_or(Error::Limit("ordinary symbol property work"))?;
    Ok(())
}
fn ordered(ranges: impl Iterator<Item = (usize, usize)>) -> Result<()> {
    let mut end = 0;
    let mut count = 0;
    for (a, b) in ranges {
        count += 1;
        if count > MAX {
            return Err(Error::Limit("ordinary symbol spans"));
        }
        if a >= b || a < end {
            return Err(Error::Invalid("ordinary symbol source interval order"));
        }
        end = b;
    }
    Ok(())
}
#[derive(Default)]
struct State {
    font: Option<usize>,
    language: Option<u16>,
}
// A closed property set keeps unimplemented resets and font-selection modifiers explicit.
// Appearance-only properties retain their operand grammar even though they do not select a font.
fn property(state: &mut State, p: &PropertyModifier, fonts: usize) -> Result<bool> {
    let b = &p.operand;
    let u = || crate::office::doc::u16_at(b, 0);
    match p.sprm {
        0x4a4f | 0x4a50 | 0x4a51 | 0x4a5e => {
            if b.len() != 2 || usize::from(u()?) >= fonts {
                return Ok(false);
            }
            if p.sprm == 0x4a51 {
                state.font = Some(usize::from(u()?));
            }
        }
        0x486d | 0x4873 | 0x486e | 0x4874 | 0x485f => {
            if b.len() != 2 {
                return Ok(false);
            }
            if matches!(p.sprm, 0x486d | 0x4873) {
                state.language = Some(u()?);
            }
        }
        0x0835 | 0x0836 | 0x085c | 0x085d => {
            if b.len() != 1 || !matches!(b[0], 0 | 1 | 128 | 129) {
                return Ok(false);
            }
        }
        0x0868 => {
            if b.len() != 1 || b[0] > 1 {
                return Ok(false);
            }
        }
        0x6815 | 0x6816 => {
            if b.len() != 4 {
                return Ok(false);
            }
        }
        0x4a43 | 0x4a61 => {
            if b.len() != 2 || !(1..=3276).contains(&u()?) {
                return Ok(false);
            }
        }
        0x484b => {
            if b.len() != 2 || u()? > 3276 {
                return Ok(false);
            }
        }
        0x2a48 => {
            if b.len() != 1 || b[0] > 2 {
                return Ok(false);
            }
        }
        0x2a42 => {
            if b.len() != 1 || b[0] > 16 {
                return Ok(false);
            }
        }
        0x2a3e => {
            if b.len() != 1 || !matches!(b[0],0..=4|6|7|9..=13|20|23|25..=27|43|55) {
                return Ok(false);
            }
        }
        0x6870 => {
            if b.len() != 4 || !(b[3] == 0 || b.as_slice() == [0, 0, 0, 255]) {
                return Ok(false);
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}
fn style_properties(
    ctx: &Context<'_>,
    index: usize,
    work: &mut usize,
) -> Result<Option<Vec<PropertyModifier>>> {
    let Some(character) = ctx.styles.styles.get(10).and_then(Option::as_ref) else {
        return Ok(None);
    };
    if character.index != 10
        || character.kind != 2
        || character.base_style.is_some()
        || character.linked_style.is_some()
        || character.revision_marked
        || !character.raw_revision.is_empty()
        || character
            .properties
            .iter()
            .any(|p| p.kind != "character" || !p.bytes.is_empty())
    {
        return Ok(None);
    }
    let mut next = Some(index);
    let mut seen = Vec::new();
    let mut result = None;
    while let Some(i) = next {
        charge(work, 1)?;
        if seen.len() >= 64 || seen.contains(&i) {
            return Ok(None);
        }
        seen.push(i);
        let Some(s) = ctx.styles.styles.get(i).and_then(Option::as_ref) else {
            return Ok(None);
        };
        if s.index != i
            || s.kind != 1
            || s.linked_style.is_some()
            || s.revision_marked
            || !s.raw_revision.is_empty()
        {
            return Ok(None);
        }
        for group in &s.properties {
            charge(work, group.bytes.len() + 1)?;
            match group.kind {
                "paragraph" => {
                    if group.bytes.len() < 2
                        || usize::from(crate::office::doc::u16_at(&group.bytes, 0)?) != i
                    {
                        return Ok(None);
                    }
                }
                "character" if !group.bytes.is_empty() => {
                    // Multiple contributing ancestors require a separate inheritance proof.
                    if result.is_some() {
                        return Ok(None);
                    }
                    result = Some(decode_character_modifiers(&group.bytes)?);
                }
                "character" => {}
                _ => return Ok(None),
            }
        }
        next = s.base_style;
    }
    Ok(Some(result.unwrap_or_default()))
}
/// Resolve ordinary source characters independently of explicit CSymbol markers.
/// Unknown font, style, script, revision or visibility contexts retain their source code.
pub fn resolve(input: &Transcription, ctx: &Context<'_>) -> Result<OrdinarySymbols> {
    if ctx.source.binary_version != ctx.formatting.binary_version
        || ctx.source.binary_version != ctx.paragraphs.binary_version
    {
        return Err(Error::Invalid("ordinary symbol source version"));
    }
    let mut authored_end = 0usize;
    for story in &ctx.source.stories {
        if story.start_cp != authored_end {
            return Err(Error::Invalid("ordinary symbol source story order"));
        }
        authored_end = authored_end
            .checked_add(story.utf16_units)
            .ok_or(Error::Limit("ordinary symbol authored end"))?;
    }
    if authored_end != ctx.formatting.authored_story_end_cp {
        return Err(Error::Invalid("ordinary symbol authored source coverage"));
    }

    ordered(
        ctx.formatting
            .character_spans
            .iter()
            .map(|s| (s.start_cp, s.end_cp)),
    )?;
    ordered(ctx.formatting.pieces.iter().map(|s| (s.start_cp, s.end_cp)))?;
    ordered(ctx.paragraphs.spans.iter().map(|s| (s.start_cp, s.end_cp)))?;
    ordered(ctx.hidden.spans.iter().map(|s| (s.start_cp, s.end_cp)))?;
    ordered(ctx.revisions.spans.iter().map(|s| (s.start_cp, s.end_cp)))?;
    let mut result = OrdinarySymbols {
        status: "partial_ordinary_font_symbol_evidence",
        spans: Vec::new(),
        embedded_font_table_present: ctx.embedded_fonts,
    };
    let mut work = WORK;
    let mut total = 0usize;
    for raw in &ctx.source.stories {
        total = total
            .checked_add(raw.utf16_units)
            .ok_or(Error::Limit("ordinary symbol text"))?;
        if total > 16 * 1024 * 1024 {
            return Err(Error::Limit("ordinary symbol text"));
        }
        let story = input
            .stories
            .iter()
            .find(|s| s.kind == raw.kind)
            .ok_or(Error::Invalid("ordinary symbol story"))?;
        let mut cp = raw.start_cp;
        for ch in raw.text.chars() {
            if ('\u{f020}'..='\u{f0ff}').contains(&ch) {
                if result.spans.len() >= MAX {
                    return Err(Error::Limit("ordinary symbol candidates"));
                }
                let mut span = OrdinarySymbol {
                    story: raw.kind,
                    source_cp: cp,
                    source_code: ch as u16,
                    font_index: None,
                    paragraph_style: None,
                    unicode: None,
                    basis: "standard_chp_hint_zero_non_ascii_font",
                    unresolved: None,
                };
                span.unresolved = resolve_one(cp, story, ctx, &mut span, &mut work)?;
                result.spans.push(span);
            }
            cp += ch.len_utf16();
        }
        if cp != raw.start_cp + raw.utf16_units {
            return Err(Error::Invalid("ordinary symbol raw units"));
        }
    }
    Ok(result)
}
fn resolve_one(
    cp: usize,
    story: &crate::office::doc::transcription::TranscribedStory,
    ctx: &Context<'_>,
    out: &mut OrdinarySymbol,
    work: &mut usize,
) -> Result<Option<&'static str>> {
    if ctx.embedded_fonts != Some(false) {
        return Ok(Some("embedded_font_mapping_unresolved"));
    }
    macro_rules! containing {
        ($v:expr) => {{
            let v = $v;
            let at = v.partition_point(|s| s.end_cp <= cp);
            v.get(at).filter(|s| s.start_cp <= cp && cp < s.end_cp)
        }};
    }
    let Some(segment) = containing!(&story.segments) else {
        return Ok(Some("source_projection_unresolved"));
    };
    if segment.kind != "text" || segment.output_start == segment.output_end {
        return Ok(Some("not_ordinary_projected_text"));
    }
    let Some(hidden) = containing!(&ctx.hidden.spans) else {
        return Ok(Some("hidden_coverage_unresolved"));
    };
    if hidden.property.hidden != Some(false) || !hidden.property.unresolved.is_empty() {
        return Ok(Some("hidden_state_unresolved"));
    }
    if hidden.paragraph_table_depth != Some(0)
        || hidden.table_baseline_basis != "source_paragraph_depth_zero"
    {
        return Ok(Some("table_font_contribution_unresolved"));
    }
    let Some(rev) = containing!(&ctx.revisions.spans) else {
        return Ok(Some("revision_coverage_unresolved"));
    };
    if !rev.unresolved.is_empty()
        || rev.properties.deleted != Some(false)
        || rev.properties.field_hidden != Some(false)
        || rev.properties.special != Some(false)
    {
        return Ok(Some("revision_or_special_state_unresolved"));
    }
    let Some(piece) = containing!(&ctx.formatting.pieces) else {
        return Ok(Some("piece_coverage_unresolved"));
    };
    if piece.raw_prm != 0 {
        return Ok(Some("piece_font_properties_unresolved"));
    }
    let Some(paragraph) = containing!(&ctx.paragraphs.spans) else {
        return Ok(Some("paragraph_coverage_unresolved"));
    };
    let Some(style_index) = paragraph.style_index.map(usize::from) else {
        return Ok(Some("paragraph_style_unresolved"));
    };
    if hidden.property.paragraph_style != Some(style_index)
        || hidden.paragraph_end_cp != paragraph.end_cp.checked_sub(1)
        || hidden.property.character_style != 10
    {
        return Ok(Some("paragraph_or_character_style_unresolved"));
    }
    out.paragraph_style = Some(style_index);
    let Some(properties) = style_properties(ctx, style_index, work)? else {
        return Ok(Some("style_font_properties_unresolved"));
    };
    let Some(direct) = containing!(&ctx.formatting.character_spans) else {
        return Ok(Some("character_coverage_unresolved"));
    };
    let direct_index = ctx
        .formatting
        .character_spans
        .partition_point(|s| s.end_cp <= cp);
    let piece_index = ctx.formatting.pieces.partition_point(|s| s.end_cp <= cp);
    if hidden.source_span_index != direct_index
        || hidden.piece_index != piece_index
        || rev.source_span_index != direct_index
        || rev.piece_index != piece_index
    {
        return Ok(Some("character_evidence_identity_unresolved"));
    }
    let mut state = State::default();
    for p in properties.iter().chain(&direct.properties) {
        charge(work, p.operand.len() + 2)?;
        if !property(&mut state, p, ctx.fonts.fonts.len())? {
            return Ok(Some("font_operator_or_operand_unresolved"));
        }
    }
    if !matches!(state.language, Some(0x0409 | 0x0809 | 0x0c09 | 0x040c)) {
        return Ok(Some("font_script_language_unresolved"));
    }
    // A direct non-ASCII font makes stylesheet font defaults unnecessary in this scope.
    if !direct.properties.iter().any(|p| p.sprm == 0x4a51) {
        return Ok(Some("direct_non_ascii_font_unresolved"));
    }
    let Some(index) = state.font else {
        return Ok(Some("font_index_unresolved"));
    };
    if ctx.fonts.fonts[index].index != index {
        return Ok(Some("font_identity_unresolved"));
    }
    out.font_index = Some(index);
    let (mapped, basis) =
        crate::office::doc::symbols::map_symbol(&ctx.fonts.fonts[index], out.source_code);
    out.unicode = mapped;
    if mapped.is_none() {
        return Ok(Some(basis));
    }
    Ok(None)
}
mod projection;
pub use projection::apply;
#[cfg(test)]
mod tests;
