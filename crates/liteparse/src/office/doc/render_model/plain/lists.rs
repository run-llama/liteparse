//! Source selection for generated list paint; logical labels remain inspector-owned.
use crate::office::doc::{
    Error, Result,
    list_projection::{LabelSource, ListContext},
    lists::{ListDefinitions, ListLevel},
};
use serde::Serialize;
#[derive(Debug, Serialize)]
pub struct Label {
    pub anchor_cp: usize,
    pub list_id: u32,
    pub override_index: usize,
    pub level: u8,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_code: Option<u16>,
    pub following_character: u8,
    pub justification: u8,
    pub number_format: u8,
    pub raw_papx: Vec<u8>,
    pub raw_chpx: Vec<u8>,
}
/// Preserve exact logical label selection and complete chosen formatting level.
/// Consumers resolve paragraph and label presentation before admitting any label paint.
pub fn prepare(c: &ListContext<'_>, embedded_fonts: bool) -> Result<Vec<Label>> {
    prepare_with_tables(c, embedded_fonts, None)
}

pub(super) fn prepare_with_tables(
    c: &ListContext<'_>,
    embedded_fonts: bool,
    table_evidence: Option<&crate::office::doc::hidden_visibility::TableHiddenEvidence>,
) -> Result<Vec<Label>> {
    let input = crate::office::doc::transcription::transcribe(c.source);
    let report = crate::office::doc::list_projection::project_lists_with_sources(
        &input,
        c,
        Some(embedded_fonts),
        table_evidence,
    )?;
    let mut result = Vec::new();
    let mut budget = 8 * 1024 * 1024usize;
    let story = report
        .transcription
        .stories
        .iter()
        .find(|s| s.kind == "body")
        .ok_or(Error::Invalid("render list body story"))?;
    let mut anchors = std::collections::BTreeMap::new();
    for segment in story
        .segments
        .iter()
        .filter(|s| s.kind == "generated_list_label")
    {
        if segment.start_cp != segment.end_cp || anchors.insert(segment.start_cp, segment).is_some()
        {
            return Err(Error::Invalid("render list generated anchor"));
        }
    }
    if anchors.len() > 262_144 {
        return Err(Error::Limit("render list labels"));
    }
    for selected in &report.label_sources {
        let level = selected_level(c.definitions, selected)?;
        let segment = anchors
            .get(&selected.start_cp)
            .ok_or(Error::Unsupported("render list nonbody anchor"))?;
        let displayed = story
            .text
            .get(segment.output_start..segment.output_end)
            .ok_or(Error::Invalid("render list label bytes"))?;
        let text = match level.following_character {
            0 => displayed.strip_suffix('\t'),
            1 => displayed.strip_suffix(' '),
            2 => Some(displayed),
            _ => None,
        }
        .ok_or(Error::Invalid("render list following character"))?;
        if text.is_empty()
            || text.chars().any(char::is_control)
            || result
                .last()
                .is_some_and(|p: &Label| p.anchor_cp >= selected.start_cp)
        {
            return Err(Error::Unsupported("render list label sequence"));
        }
        budget = budget
            .checked_sub(text.len() + level.raw_papx.len() + level.raw_chpx.len() + 1)
            .ok_or(Error::Limit("render list source bytes"))?;
        result.push(Label {
            anchor_cp: selected.start_cp,
            list_id: selected.list_id,
            override_index: selected.override_index,
            level: selected.level,
            text: text.into(),
            symbol_code: (!text.is_ascii()
                && level.number_format == 23
                && level.number_text_utf16.len() == 1)
                .then(|| level.number_text_utf16[0]),
            following_character: level.following_character,
            justification: level.flags & 3,
            number_format: level.number_format,
            raw_papx: level.raw_papx.clone(),
            raw_chpx: level.raw_chpx.clone(),
        });
    }
    Ok(result)
}
fn selected_level<'a>(defs: &'a ListDefinitions, source: &LabelSource) -> Result<&'a ListLevel> {
    let over = source
        .override_index
        .checked_sub(1)
        .and_then(|i| defs.overrides.get(i))
        .ok_or(Error::Invalid("render list override index"))?;
    let list = defs
        .lists
        .get(over.list_index)
        .filter(|l| l.lsid == source.list_id && over.lsid == source.list_id)
        .ok_or(Error::Invalid("render list identity"))?;
    if list.style_links.get(source.level as usize) != Some(&4095) {
        return Err(Error::Unsupported("render linked list style"));
    }
    let mut overrides = over.levels.iter().filter(|v| v.level == source.level);
    let selected = overrides.next();
    if overrides.next().is_some() {
        return Err(Error::Invalid("render duplicate list level override"));
    }
    if let Some(level) = selected.and_then(|o| o.format_override.as_ref()) {
        return Ok(level);
    }
    list.levels
        .get(source.level as usize)
        .ok_or(Error::Invalid("render list level index"))
}

#[derive(Debug, Serialize)]
pub struct Presentation {
    pub value: crate::office::doc::render_model::runs::RunProperties,
    pub candidate_evidence: Vec<crate::office::doc::render_model::runs::RunProperties>,
    pub basis: &'static str,
}
/// Resolve a bounded initial font contract by agreement after full level formatting.
/// Source evidence retains the paragraph-style, first-text and physical-mark cases.
pub fn presentation(
    c: &ListContext<'_>,
    label: &Label,
    paragraph_style: usize,
    first: &[crate::office::doc::formatting::PropertyModifier],
    mark: &[crate::office::doc::formatting::PropertyModifier],
    work: &mut usize,
) -> Result<Presentation> {
    if label.symbol_code.is_none()
        && (!label.text.is_ascii() || label.text.bytes().any(|b| !(32..127).contains(&b)))
    {
        return Err(Error::Unsupported("render list glyph route"));
    }
    *work = work
        .checked_add(label.raw_chpx.len() + 1)
        .filter(|n| *n <= 8 * 1024 * 1024)
        .ok_or(Error::Limit("render list presentation work"))?;
    let modifiers = crate::office::doc::formatting::decode_character_modifiers(&label.raw_chpx)?;
    let mut picture = false;
    for p in &modifiers {
        match p.sprm {
            0x6887 => {
                let b: [u8; 4] = p
                    .operand
                    .as_slice()
                    .try_into()
                    .map_err(|_| Error::Invalid("render picture bullet CP"))?;
                if i32::from_le_bytes(b) < 0 {
                    return Err(Error::Invalid("render picture bullet CP"));
                }
            }
            0x4888 => {
                let b: [u8; 2] = p
                    .operand
                    .as_slice()
                    .try_into()
                    .map_err(|_| Error::Invalid("render picture bullet flags"))?;
                picture = u16::from_le_bytes(b) & 1 != 0;
            }
            _ => {}
        }
    }
    if picture {
        return Err(Error::Unsupported("render picture bullet"));
    }
    let mut candidate_evidence = Vec::new();
    for source in [&[][..], first, mark] {
        if source.len() > 262_144 {
            return Err(Error::Limit("render list presentation modifiers"));
        }
        let expanded = source
            .iter()
            .chain(&modifiers)
            .try_fold(0usize, |sum, p| sum.checked_add(p.operand.len() + 2))
            .ok_or(Error::Limit("render list presentation bytes"))?;
        *work = work
            .checked_add(expanded)
            .filter(|n| *n <= 8 * 1024 * 1024)
            .ok_or(Error::Limit("render list presentation work"))?;
        let mut direct = source.to_vec();
        direct.extend(
            modifiers
                .iter()
                .filter(|p| !matches!(p.sprm, 0x6887 | 0x4888))
                .cloned(),
        );
        candidate_evidence.push(crate::office::doc::render_model::runs::resolve_with_budget(
            c.styles,
            c.fonts,
            paragraph_style,
            &direct,
            work,
        )?);
    }
    let evidence_bytes = candidate_evidence
        .iter()
        .flat_map(|p| p.applied.iter().chain(&p.complex_property_evidence))
        .try_fold(0usize, |sum, p| sum.checked_add(p.operand.len() + 2))
        .ok_or(Error::Limit("render list comparison bytes"))?;
    *work = work
        .checked_add(evidence_bytes)
        .filter(|n| *n <= 8 * 1024 * 1024)
        .ok_or(Error::Limit("render list comparison work"))?;
    let normalized = |p: &crate::office::doc::render_model::runs::RunProperties| {
        let mut p = p.clone();
        p.applied.clear();
        p.complex_property_evidence.clear();
        p
    };
    let mut value = normalized(&candidate_evidence[0]);
    let mut basis = "paragraph_style_first_text_and_mark_agree_after_level";
    if candidate_evidence
        .iter()
        .skip(1)
        .any(|p| !same_selected_baseline(&normalized(p), &value, label))
    {
        if !same_selected_baseline(
            &normalized(&candidate_evidence[1]),
            &normalized(&candidate_evidence[2]),
            label,
        ) {
            return Err(Error::Unsupported("render list initial character baseline"));
        }
        value = normalized(&candidate_evidence[1]);
        basis = "approximate_first_text_and_mark_agree_after_level";
    }
    let symbol = label.symbol_code.is_some();
    if let Some(code) = label.symbol_code {
        let font = c
            .fonts
            .fonts
            .get(value.fonts[0])
            .ok_or(Error::Invalid("render list symbol font"))?;
        if value.fonts[0] != value.fonts[2]
            || font.charset != 2
            || crate::office::doc::symbols::map_symbol(font, code)
                .0
                .map(|c| c.to_string())
                .as_deref()
                != Some(&label.text)
        {
            return Err(Error::Unsupported("render list symbol mapping"));
        }
    }
    if value.hidden
        || value.hint != 0
        || value.bidi
        || c.fonts
            .fonts
            .get(value.fonts[0])
            .is_none_or(|f| f.index != value.fonts[0] || f.charset != if symbol { 2 } else { 0 })
    {
        return Err(Error::Unsupported("render list visible font route"));
    }
    Ok(Presentation {
        value,
        candidate_evidence,
        basis,
    })
}

// ASCII and explicit symbol labels use the Western route after bidi/hint checks.
// Preserve inactive complex properties in evidence while comparing painted values.
fn same_selected_baseline(
    left: &crate::office::doc::render_model::runs::RunProperties,
    right: &crate::office::doc::render_model::runs::RunProperties,
    label: &Label,
) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    if (label.text.is_ascii() || label.symbol_code.is_some())
        && !left.bidi
        && !right.bidi
        && left.hint == 0
        && right.hint == 0
    {
        left.complex = Default::default();
        right.complex = Default::default();
    }
    left == right
}

#[derive(Debug, Serialize)]
pub struct PaintLabel {
    pub source: Label,
    pub presentation: Presentation,
    pub source_paragraph_papx: Vec<u8>,
}
/// Retain selection operands separately while applying the chosen LVL PAPX last.
pub fn paragraph_modifiers(
    label: &Label,
    direct: &[crate::office::doc::formatting::PropertyModifier],
    work: &mut usize,
) -> Result<Vec<crate::office::doc::formatting::PropertyModifier>> {
    let bytes = direct
        .iter()
        .try_fold(label.raw_papx.len() + 1, |n, p| {
            n.checked_add(p.operand.len() + 3)
        })
        .ok_or(Error::Limit("render list paragraph work"))?;
    *work = work
        .checked_add(bytes)
        .filter(|n| *n <= 8 * 1024 * 1024)
        .ok_or(Error::Limit("render list paragraph work"))?;
    let mut last_level = None;
    let mut last_override = None;
    for p in direct {
        match p.sprm {
            0x260a => {
                if p.operand.len() != 1 || p.operand[0] > 8 {
                    return Err(Error::Invalid("render list paragraph level"));
                }
                last_level = Some(p.operand[0]);
            }
            0x460b => {
                if p.operand.len() != 2 {
                    return Err(Error::Invalid("render list paragraph override"));
                }
                last_override = Some(u16::from_le_bytes([p.operand[0], p.operand[1]]) as usize);
            }
            _ => {}
        }
    }
    if last_level.is_some_and(|v| v != label.level)
        || last_override.is_some_and(|v| v != label.override_index)
    {
        return Err(Error::Unsupported("render list paragraph selection"));
    }
    let mut result = direct
        .iter()
        .filter(|p| !matches!(p.sprm, 0x260a | 0x460b))
        .cloned()
        .collect::<Vec<_>>();
    result.extend(crate::office::doc::render_model::paragraphs::decode(
        &label.raw_papx,
    )?);
    Ok(result)
}

/// List stops before the hanging label origin cannot be reached by its suffix tab.
/// Source tab records remain in the paragraph model for reconstruction.
pub fn painting_tabs(
    p: &crate::office::doc::render_model::paragraphs::ParagraphProperties,
) -> impl Iterator<Item = &crate::office::doc::render_model::paragraphs::TabStop> {
    p.tabs.iter().filter(move |t| {
        !(t.alignment == 6
            && p.first_twips < 0
            && i64::from(t.position_twips) < i64::from(p.left_twips) + i64::from(p.first_twips))
    })
}

/// Check bounded label evidence before public model consumers index fonts.
pub(super) fn validate(
    labels: &[PaintLabel],
    paragraphs: &[super::PlainParagraph],
    charsets: &[u8],
    work: &mut usize,
) -> Result<()> {
    for p in paragraphs {
        let tabs: Vec<_> = painting_tabs(&p.properties).collect();
        if tabs.iter().any(|t| t.alignment == 6) {
            let at = labels.partition_point(|l| l.source.anchor_cp < p.source_cp[0]);
            if labels.get(at).is_none_or(|l| {
                l.source.anchor_cp != p.source_cp[0] || l.source.following_character != 0
            }) || tabs.len() != 1
                || tabs[0].position_twips < p.properties.left_twips
                || (tabs[0].position_twips > p.properties.left_twips
                    && (p.properties.first_twips >= 0 || labels[at].source.justification != 0))
                || p.properties.justification != 0
                || p.runs.iter().any(|r| r.text.contains('\t'))
                || p.runs
                    .first()
                    .and_then(|r| r.text.chars().next())
                    .is_none_or(char::is_whitespace)
            {
                return Err(Error::Unsupported("render list tab source context"));
            }
        }
    }
    if labels.len() > paragraphs.len() {
        return Err(Error::Limit("render list count"));
    }
    let mut previous = None;
    for label in labels {
        let source = &label.source;
        let v = &label.presentation.value;
        let index = paragraphs.partition_point(|p| p.source_cp[0] < source.anchor_cp);
        if previous.is_some_and(|p| p >= source.anchor_cp)
            || paragraphs
                .get(index)
                .is_none_or(|p| p.source_cp[0] != source.anchor_cp)
            || source.text.is_empty()
            || (source.symbol_code.is_none()
                && (!source.text.is_ascii()
                    || source.text.bytes().any(|b| !(33..127).contains(&b))))
            || (source.symbol_code.is_some()
                && (source.number_format != 23
                    || source.text.chars().count() != 1
                    || v.fonts[0] != v.fonts[2]))
            || source.following_character > 2
            || source.justification > 2
            || source.level > 8
            || source.override_index == 0
            || v.fonts.iter().any(|f| *f >= charsets.len())
            || charsets.get(v.fonts[0]) != Some(&if source.symbol_code.is_some() { 2 } else { 0 })
            || v.bidi
            || v.hint != 0
            || v.hidden
            || !(2..=3276).contains(&v.size_half_points)
            || v.vertical_alignment > 2
            || v.kerning_threshold_half_points != 0
            || v.underline
            || v.shading_rgb.is_some()
        {
            return Err(Error::Unsupported(
                "render list model ownership or presentation",
            ));
        }
        previous = Some(source.anchor_cp);
        if !matches!(
            label.presentation.basis,
            "paragraph_style_first_text_and_mark_agree_after_level"
                | "approximate_first_text_and_mark_agree_after_level"
        ) || label.presentation.candidate_evidence.len() != 3
        {
            return Err(Error::Invalid("render list baseline evidence"));
        }
        let bytes = source.text.len()
            + source.raw_papx.len()
            + source.raw_chpx.len()
            + label.source_paragraph_papx.len()
            + 1;
        *work = work
            .checked_add(bytes)
            .filter(|n| *n <= 8 * 1024 * 1024)
            .ok_or(Error::Limit("render list model work"))?;
        for (evidence_index, evidence) in label.presentation.candidate_evidence.iter().enumerate() {
            for p in evidence
                .applied
                .iter()
                .chain(&evidence.complex_property_evidence)
            {
                *work = work
                    .checked_add(p.operand.len() + 3)
                    .filter(|n| *n <= 8 * 1024 * 1024)
                    .ok_or(Error::Limit("render list model work"))?;
            }
            let mut normalized = evidence.clone();
            normalized.applied.clear();
            normalized.complex_property_evidence.clear();
            if !(same_selected_baseline(&normalized, v, source)
                || evidence_index == 0
                    && label.presentation.basis
                        == "approximate_first_text_and_mark_agree_after_level")
            {
                return Err(Error::Invalid("render list baseline agreement"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
