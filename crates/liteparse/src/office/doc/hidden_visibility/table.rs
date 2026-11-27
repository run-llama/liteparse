//! Qualify table-style text contributions for physical row ownership.
mod appearance;
mod borders;
mod conditional;
mod language;
mod nested;
use super::{MAX_PROPERTY_WORK, charge, paragraph};
use crate::office::doc::{
    Error, Inspection, Result, formatting_inspection::FormattingInspection,
    paragraph_formatting::ParagraphInspection, structure::StructurePlan, styles::Stylesheet,
    table_structure::TableOwnership, visibility::RevisionEvidence,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

const MAX_ROWS: usize = 65_536;
const MAX_ANCESTORS: usize = 64;

#[derive(Debug, Serialize)]
pub struct TableHiddenEvidence {
    pub status: &'static str,
    pub rows: Vec<TableHiddenRow>,
    #[serde(skip)]
    pub(super) paragraph_baselines: BTreeMap<usize, usize>,
    #[serde(skip)]
    list_paragraphs: BTreeSet<[usize; 2]>,
    #[serde(skip)]
    language_paragraph_baselines: BTreeMap<usize, usize>,
    #[serde(skip)]
    nested_paragraphs: BTreeMap<[usize; 2], nested::Paragraph>,
    #[serde(skip)]
    form_display_guards: BTreeSet<[usize; 2]>,
    #[serde(skip)]
    conditional_paragraphs: BTreeMap<usize, (usize, usize)>,
}
impl TableHiddenEvidence {
    /// Populate source-owned effective runs after table-role and font decoding.
    /// Unsupported character properties remain explicit in each run record.
    pub fn resolve_conditional_runs(
        &mut self,
        hidden: &super::HiddenEvidence,
        formatting: &FormattingInspection,
        styles: &Stylesheet,
        fonts: &crate::office::doc::symbols::FontTable,
    ) -> Result<()> {
        conditional::populate(self, hidden, formatting, styles, fonts)
    }

    pub(crate) fn conditional_text(&self, end: usize) -> Option<&conditional::TextState> {
        let &(row, cell) = self.conditional_paragraphs.get(&end)?;
        self.rows
            .get(row)?
            .conditional_cell_text
            .get(cell)
            .map(|c| &c.state)
    }

    /// Retain current FFData values while withholding projection inside unresolved
    /// merged continuations. A known hidden property alone does not prove display.
    pub fn retain_form_display_uncertainty(
        &self,
        forms: &mut crate::office::doc::form_fields::FormInspection,
    ) {
        let reason = "form_value_merged_continuation_display_unresolved";
        let mut changed = false;
        for field in &mut forms.fields {
            if field.eligible
                && self
                    .form_display_guards
                    .range(..[field.field_end_cp, 0])
                    .next_back()
                    .is_some_and(|range| range[1] > field.field_start_cp)
            {
                field.eligible = false;
                field.diagnostic = Some(match field.diagnostic.take() {
                    Some(prior) => format!("{prior}; {reason}"),
                    None => reason.to_string(),
                });
                changed = true;
            }
        }
        if changed && !forms.unresolved.contains(&reason) {
            forms.unresolved.push(reason);
        }
    }

    pub(crate) fn nested_list_context(&self, range: [usize; 2]) -> Option<nested::Paragraph> {
        self.nested_paragraphs.get(&range).copied()
    }

    pub(super) fn visibility_basis(&self, paragraph_end: usize) -> Option<&'static str> {
        if self.paragraph_baselines.contains_key(&paragraph_end) {
            Some("selected_row_style_has_empty_text_properties")
        } else {
            self.language_paragraph_baselines
                .get(&paragraph_end)
                .and_then(|index| {
                    let row = self.rows.get(*index)?;
                    (row.language_contribution.is_some() && row.unresolved.is_empty())
                        .then_some("selected_row_style_has_language_only_character_properties")
                })
        }
    }

    /// Qualify an exact cell paragraph with complete unmerged physical ownership
    /// and an empty selected table-style text contribution. Row marks are excluded.
    pub(crate) fn permits_list_paragraph(&self, source_range: [usize; 2]) -> bool {
        self.list_paragraphs.contains(&source_range)
    }

    /// Confirm a source span belongs to a physical paragraph whose selected table
    /// style contributes no character or paragraph text properties.
    pub(crate) fn has_empty_text_contribution(
        &self,
        source_range: [usize; 2],
        paragraph_mark_cp: usize,
    ) -> bool {
        let Some(end) = paragraph_mark_cp.checked_add(1) else {
            return false;
        };
        let Some(&index) = self.paragraph_baselines.get(&end) else {
            return false;
        };
        let Some(row) = self.rows.get(index) else {
            return false;
        };
        let start = self
            .paragraph_baselines
            .range(..end)
            .next_back()
            .map_or(row.source_range[0], |(&cp, _)| cp.max(row.source_range[0]));
        row.empty_text_contribution
            && row.unresolved.is_empty()
            && row.source_range[0] <= start
            && start <= source_range[0]
            && source_range[0] < source_range[1]
            && source_range[1] <= end
            && end <= row.source_range[1]
    }
}
#[derive(Debug, Serialize)]
pub struct TableHiddenRow {
    pub source_range: [usize; 2],
    pub row_mark_cp: usize,
    pub selected_style: Option<usize>,
    pub style_ancestors: Vec<usize>,
    pub empty_text_contribution: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language_contribution: Option<Box<language::TableLanguageContribution>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conditional_cell_text: Vec<conditional::CellText>,
    pub unresolved: Vec<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub physical_ownership_unresolved: Vec<&'static str>,
}

/// MS-DOC §2.4.6.6 selects the innermost row's TIstd before paragraph formatting.
/// Empty character and identity-only paragraph groups leave no conditional text branch.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/d8b66123-1a3d-4e06-94c5-5ab16e9b6417>
pub(super) fn resolve(
    source: &Inspection,
    formatting: &FormattingInspection,
    paragraphs: &ParagraphInspection,
    styles: &Stylesheet,
    structure: &StructurePlan,
    tables: &TableOwnership,
    revisions: &RevisionEvidence,
) -> Result<TableHiddenEvidence> {
    if source.stories.len() != structure.stories.len()
        || source.stories.len() != tables.stories.len()
        || paragraphs.status != "reference_expanded_endpoint_selected_paragraphs"
        || revisions.spans.len() > 262_144
    {
        return Err(Error::Invalid("hidden table source identity"));
    }
    if source.stories.len() > 8 {
        return Err(Error::Limit("hidden table stories"));
    }
    let mut previous = 0;
    for span in &revisions.spans {
        if span.start_cp != previous || span.end_cp <= span.start_cp {
            return Err(Error::Invalid("hidden table revision coverage"));
        }
        previous = span.end_cp;
    }
    let mut work = MAX_PROPERTY_WORK;
    let mut list_work = MAX_PROPERTY_WORK;
    let mut language_work = MAX_PROPERTY_WORK;
    let mut cache = BTreeMap::new();
    let mut result = TableHiddenEvidence {
        status: "partial_source_table_hidden_baselines",
        rows: Vec::new(),
        paragraph_baselines: BTreeMap::new(),
        list_paragraphs: BTreeSet::new(),
        language_paragraph_baselines: BTreeMap::new(),
        nested_paragraphs: BTreeMap::new(),
        form_display_guards: BTreeSet::new(),
        conditional_paragraphs: BTreeMap::new(),
    };
    for ((raw, part), table) in source
        .stories
        .iter()
        .zip(&structure.stories)
        .zip(&tables.stories)
    {
        if raw.kind != part.kind || raw.kind != table.kind {
            return Err(Error::Invalid("hidden table story identity"));
        }
        if table.rows.len() > MAX_ROWS {
            return Err(Error::Limit("hidden table rows"));
        }
        result
            .nested_paragraphs
            .extend(nested::collect(part, table, revisions, &mut list_work)?);
        let nested: std::collections::BTreeSet<_> = table
            .rows
            .iter()
            .filter(|r| r.depth > 1)
            .map(|r| r.source_range[0])
            .collect();
        charge(&mut work, table.rows.len())?;
        let story_end = raw
            .start_cp
            .checked_add(raw.utf16_units)
            .ok_or(Error::Limit("hidden table story range"))?;
        for (row_index, row) in table.rows.iter().enumerate() {
            charge(
                &mut work,
                // Language proof uses its own work budget, keeping core row work separate.
                std::mem::size_of::<TableHiddenRow>()
                    - std::mem::size_of::<Option<Box<language::TableLanguageContribution>>>()
                    - std::mem::size_of::<Vec<conditional::CellText>>()
                    + row.cells.len(),
            )?;
            if result.rows.len() == MAX_ROWS {
                return Err(Error::Limit("hidden table row count"));
            }
            let mark = part
                .paragraphs
                .get(row.row_mark_paragraph)
                .ok_or(Error::Invalid("hidden table row mark"))?;
            let [start, end] = row.source_range;
            if start < raw.start_cp
                || start >= end
                || end > story_end
                || mark.source_range[1] != end
                || mark.source_range[0] < start
            {
                return Err(Error::Invalid("hidden table row range"));
            }
            let mut evidence = TableHiddenRow {
                source_range: row.source_range,
                row_mark_cp: end - 1,
                selected_style: None,
                style_ancestors: Vec::new(),
                empty_text_contribution: false,
                language_contribution: None,
                conditional_cell_text: Vec::new(),
                unresolved: Vec::new(),
                physical_ownership_unresolved: row
                    .unresolved
                    .iter()
                    .copied()
                    .filter(|reason| *reason == "merged_cell_visible_ownership_unresolved")
                    .collect(),
            };
            let has_nested = row.depth > 1 || nested.range(start..end).next().is_some();
            let nested_members = if has_nested {
                nested::direct_members(
                    table,
                    row_index,
                    part,
                    &result.nested_paragraphs,
                    paragraphs,
                    formatting,
                    &mut work,
                )?
            } else {
                None
            };
            // Merge ownership affects displayed cell contents, independently of a proven
            // empty table-style text baseline. Keep its uncertainty in the physical ledger;
            // list eligibility below additionally requires an individually unmerged cell.
            if (has_nested && nested_members.is_none())
                || row
                    .unresolved
                    .iter()
                    .any(|reason| *reason != "merged_cell_visible_ownership_unresolved")
                || mark.terminal_unit != Some(if row.depth == 1 { 7 } else { 13 })
            {
                evidence
                    .unresolved
                    .push("table_membership_or_topology_unresolved");
            }
            let mut revision_cp = start;
            for span in revisions
                .spans
                .iter()
                .skip(revisions.spans.partition_point(|s| s.end_cp <= start))
            {
                if span.start_cp >= end {
                    break;
                }
                charge(&mut work, 1)?;
                if span.start_cp > revision_cp
                    || span.properties.deleted != Some(false)
                    || span.properties.inserted != Some(false)
                    || !span.unresolved.is_empty()
                {
                    evidence.unresolved.push("table_revision_state_unresolved");
                    break;
                }
                revision_cp = span.end_cp.min(end);
            }
            if revision_cp != end
                && !evidence
                    .unresolved
                    .contains(&"table_revision_state_unresolved")
            {
                evidence.unresolved.push("table_revision_state_unresolved");
            }
            if evidence.unresolved.is_empty() {
                let pap = paragraphs
                    .spans
                    .get(paragraphs.spans.partition_point(|p| p.end_cp < end))
                    .filter(|p| p.end_cp == end && p.start_cp == mark.source_range[0])
                    .ok_or(Error::Invalid("hidden table terminating PAPX"))?;
                let piece = formatting
                    .pieces
                    .get(formatting.pieces.partition_point(|p| p.end_cp < end))
                    .filter(|p| p.start_cp < end && p.end_cp >= end)
                    .ok_or(Error::Invalid("hidden table terminating piece"))?;
                if mark.endpoint.as_ref().is_none_or(|p| {
                    p.end_cp != end
                        || p.piece_index != formatting.pieces.partition_point(|p| p.end_cp < end)
                }) {
                    return Err(Error::Invalid("hidden table physical endpoint"));
                }
                let mut properties = paragraph::group(&pap.raw_modifiers, 5, &mut work)?;
                properties.extend(
                    paragraph::piece(piece.raw_prm, &formatting.raw_property_groups, 5, &mut work)?
                        .into_iter()
                        .filter(|p| (p.sprm >> 10) & 7 == 5),
                );
                let mut known = true;
                let mut columns = 0;
                for p in &properties {
                    // Physical ownership already validated definitions; replay their counts
                    // so border ranges cannot refer to cells introduced by later properties.
                    if p.sprm == 0xd608 {
                        columns = p.operand[2] as usize;
                    } else if p.sprm == 0x7621 {
                        columns += p.operand[1] as usize;
                    }
                    if p.sprm == 0x563a {
                        evidence.selected_style =
                            Some(crate::office::doc::u16_at(&p.operand, 0)? as usize);
                    } else if matches!(p.sprm, 0x5400 | 0x548a) {
                        // Physical and logical table alignment preserve character properties.
                        known &= matches!(p.operand.as_slice(), [0..=2, 0]);
                    } else if p.sprm == 0x3404 {
                        // Header repetition changes page placement. Conditional text remains
                        // guarded by the complete style proof below.
                        known &= matches!(p.operand.as_slice(), [0] | [1]);
                    } else if matches!(p.sprm, 0x360d | 0x940e | 0x940f | 0x9410 | 0x941e) {
                        // Anchors and wrapping distances preserve character properties.
                        // Absolute page bounds and overlap still require layout resolution.
                        known &= valid_table_placement(p.sprm, &p.operand);
                    } else if p.sprm == 0xf617 {
                        // Preferred leading indentation leaves table text properties intact.
                        known &= valid_table_part_width(&p.operand);
                    } else if matches!(p.sprm, 0x3615 | 0x3403 | 0x3466) {
                        // Autofit changes widths; both cant-split variants change row pagination.
                        // Each has a Bool8 operand and preserves the selected text style.
                        // https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/b39a6648-501c-4361-8366-4f042f579469
                        known &= matches!(p.operand.as_slice(), [0] | [1]);
                    } else if matches!(p.sprm, 0xd639 | 0xd642 | 0xd660 | 0xd633) {
                        known &= appearance::preserves_text(p.sprm, &p.operand, columns);
                    } else if p.sprm == 0xd62b {
                        // Vertical merge assignment changes display ownership, while the
                        // selected style remains unchanged. Ordered columns bound its target.
                        known &= matches!(p.operand.as_slice(), [2, cell, 0 | 1 | 3] if usize::from(*cell) < columns);
                    } else if p.sprm == 0xd605 {
                        known &= borders::legacy_array(&p.operand);
                    } else if matches!(p.sprm, 0xd61a..=0xd61d) {
                        known &= borders::cell_colors(&p.operand, columns);
                    } else if p.sprm == 0xd62f {
                        known &= valid_cell_border(&p.operand, columns);
                    } else if p.sprm == 0xd632 {
                        known &= valid_cell_padding(&p.operand, columns);
                    } else if matches!(p.sprm, 0xd609 | 0xd612) {
                        known &= valid_cell_shading(p.sprm, &p.operand, columns);
                    } else if p.sprm == 0xd634 {
                        known &= valid_default_padding(&p.operand);
                    } else if !row_style_preserving(p.sprm) {
                        known = false;
                    }
                }
                if !known {
                    evidence.unresolved.push("table_style_selection_unresolved");
                } else if let Some(index) = evidence.selected_style {
                    match empty_style(index, styles, &mut cache, &mut work)? {
                        Some(chain) => {
                            evidence.style_ancestors = chain;
                            evidence.empty_text_contribution = true;
                        }
                        None if has_nested
                            || !evidence.physical_ownership_unresolved.is_empty() =>
                        {
                            evidence
                                .unresolved
                                .push("table_style_text_properties_unresolved");
                        }
                        None => {
                            let first = part
                                .paragraphs
                                .partition_point(|p| p.source_range[1] <= start);
                            let proof = language_style(index, styles, &mut language_work).and_then(
                                |proof| {
                                    let Some(proof) = proof else {
                                        return Ok(None);
                                    };
                                    let paragraphs = part
                                        .paragraphs
                                        .get(first..=row.row_mark_paragraph)
                                        .ok_or(Error::Invalid("table language paragraph range"))?;
                                    let mut cp = start;
                                    for paragraph in paragraphs {
                                        charge(&mut language_work, 32)?;
                                        if paragraph.source_range[0] != cp
                                            || paragraph.source_range[1] <= cp
                                            || !paragraph.unresolved.is_empty()
                                            || result
                                                .language_paragraph_baselines
                                                .contains_key(&paragraph.source_range[1])
                                        {
                                            return Err(Error::Invalid(
                                                "table language paragraph ownership",
                                            ));
                                        }
                                        cp = paragraph.source_range[1];
                                    }
                                    if cp != end {
                                        return Err(Error::Invalid("table language row end"));
                                    }
                                    Ok(Some(proof))
                                },
                            );
                            if matches!(&proof, Err(Error::Limit(_))) {
                                language_work = 0;
                            }
                            if let Ok(Some((chain, contribution))) = proof {
                                evidence.style_ancestors = chain;
                                evidence.language_contribution = Some(Box::new(contribution));
                                for paragraph in &part.paragraphs[first..=row.row_mark_paragraph] {
                                    result
                                        .language_paragraph_baselines
                                        .insert(paragraph.source_range[1], result.rows.len());
                                }
                            } else {
                                evidence
                                    .unresolved
                                    .push("table_style_text_properties_unresolved");
                            }
                        }
                    }
                } else {
                    evidence.empty_text_contribution = true;
                }
            }
            if evidence.empty_text_contribution
                && !evidence.physical_ownership_unresolved.is_empty()
            {
                for cell in &row.cells {
                    if cell.definition.as_ref().is_some_and(|definition| {
                        definition.vertical_merge == 1 || definition.horizontal_merge == 1
                    }) {
                        result.form_display_guards.insert(cell.source_range);
                    }
                }
            }
            if evidence.empty_text_contribution {
                let first = part
                    .paragraphs
                    .partition_point(|p| p.source_range[1] <= start);
                if first > row.row_mark_paragraph {
                    return Err(Error::Invalid("hidden table paragraph interval"));
                }
                let mut cp = start;
                for (offset, paragraph) in part.paragraphs[first..=row.row_mark_paragraph]
                    .iter()
                    .enumerate()
                {
                    charge(&mut work, 32)?;
                    if paragraph.source_range[0] != cp
                        || paragraph.source_range[1] <= cp
                        || !paragraph.unresolved.is_empty()
                    {
                        return Err(Error::Invalid("hidden table paragraph ownership"));
                    }
                    cp = paragraph.source_range[1];
                    if nested_members
                        .as_ref()
                        .is_some_and(|members| !members.contains(&(first + offset)))
                    {
                        continue;
                    }
                    if result
                        .paragraph_baselines
                        .insert(cp, result.rows.len())
                        .is_some()
                    {
                        return Err(Error::Invalid("hidden table duplicate paragraph ownership"));
                    }
                }
            }
            if evidence.empty_text_contribution
                && evidence.unresolved.is_empty()
                && (row.depth == 1 || nested_members.is_some())
                && row.definition_cell_count == Some(row.cells.len())
            {
                let first = part
                    .paragraphs
                    .partition_point(|p| p.source_range[1] <= start);
                for (index, paragraph) in part
                    .paragraphs
                    .iter()
                    .enumerate()
                    .take(row.row_mark_paragraph)
                    .skip(first)
                {
                    let range = paragraph.source_range;
                    // Exhausted list evidence leaves later paragraphs unqualified while
                    // preserving the independently bounded hidden-property evidence.
                    let Some(remaining) = list_work.checked_sub(row.cells.len() + 32) else {
                        break;
                    };
                    list_work = remaining;
                    if index != row.row_mark_paragraph
                        && result.paragraph_baselines.get(&range[1]) == Some(&result.rows.len())
                        && paragraph.unresolved.is_empty()
                        && matches!(paragraph.terminal_unit, Some(7 | 13))
                        && row.cells.iter().any(|cell| {
                            cell.definition.as_ref().is_some_and(|definition| {
                                definition.horizontal_merge == 0 && definition.vertical_merge == 0
                            }) && cell.source_range[0] <= range[0]
                                && range[0] < range[1]
                                && range[1] <= cell.source_range[1]
                        })
                    {
                        result.list_paragraphs.insert(range);
                    }
                }
            }
            result.rows.push(evidence);
        }
    }
    let mut conditional_work = MAX_PROPERTY_WORK;
    conditional::enrich(
        &mut result,
        styles,
        tables,
        structure,
        paragraphs,
        formatting,
        &mut conditional_work,
    )?;
    Ok(result)
}

// MS-DOC PositionCodeOperand ignores padding; each two-bit anchor accepts 0..=3.
// XAS/YAS_plusOne and XAS_nonNeg have distinct signed distance bounds.
fn valid_table_placement(sprm: u16, operand: &[u8]) -> bool {
    match (sprm, operand) {
        (0x360d, [_]) => true,
        (0x940e | 0x940f, [low, high]) => {
            (-31_679..=31_681).contains(&i16::from_le_bytes([*low, *high]))
        }
        (0x9410 | 0x941e, [low, high]) => u16::from_le_bytes([*low, *high]) <= 31_680,
        _ => false,
    }
}

// MS-DOC FtsWWidth_TablePart ignores Nil width bytes and forbids DxaSys.
// https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/78aa9e68-a732-4bd6-9344-9c9d2b8a6e55
fn valid_table_part_width(operand: &[u8]) -> bool {
    let [unit, low, high] = operand else {
        return false;
    };
    let width = u16::from_le_bytes([*low, *high]);
    match unit {
        0 => true,
        1 => width == 0,
        2 => width <= 5000,
        3 => width <= 31680,
        _ => false,
    }
}

// These row operators change geometry, borders, shading or save provenance.
// TIstd is handled separately; property-history walls and unknown operators stay unresolved.
// TDefTable has already passed ordered cell-definition and merge checks in plan_tables.
// https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/de06ec41-a0ac-4046-9096-cdfaa0091ad9
fn row_style_preserving(sprm: u16) -> bool {
    matches!(
        sprm,
        0xd608
            | 0x740a
            | 0x7479
            | 0x7621
            | 0x7623
            | 0x9601
            | 0x9602
            | 0x9407
            | 0xd613
            | 0xd62c
            | 0xd635
            | 0xd670
            | 0xf614
            | 0xf618
            | 0xf661
    )
}

/// TableBrcOperand applies an eight-byte border to existing cell columns.
/// NilBrc ignores its first four bytes; other borders retain COLORREF/type validation.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/d8a2205c-93cd-4f79-a0be-2656cd6d6fcc>
fn valid_cell_border(data: &[u8], columns: usize) -> bool {
    data.len() == 12
        && data[0] == 11
        && (data[1] as usize) < columns
        && data[2] >= data[1]
        && (data[2] as usize) <= columns
        && data[3] & !0x3f == 0
        && (data[8..12] == [255; 4]
            || (matches!(data[7], 0 | 255) && matches!(data[9], 0 | 1 | 3 | 5..=27)))
}

/// Default padding applies to the whole row through the fixed range 0..1.
/// CSSA permits only side bits, and the nil unit requires zero width.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/6c5d3dfc-988d-4e2b-a9dd-72b62cb64356>
fn valid_default_padding(data: &[u8]) -> bool {
    data.len() == 7
        && data[0] == 6
        && data[1..3] == [0, 1]
        && data[3] & !0x0f == 0
        && match data[4] {
            0 => data[5..7] == [0, 0],
            3 => u16::from_le_bytes([data[5], data[6]]) <= 31680,
            _ => false,
        }
}

/// Cell margins preserve text properties and address cells already defined in the row.
/// CSSA uses a six-byte payload; empty ranges still require an existing first cell.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/fd43c8b3-8d6e-484a-8bc5-efebf377f422>
fn valid_cell_padding(data: &[u8], columns: usize) -> bool {
    data.len() == 7
        && data[0] == 6
        && (data[1] as usize) < columns
        && data[2] >= data[1]
        && (data[2] as usize) <= columns
        && data[3] & !0x0f == 0
        && match data[4] {
            0 => data[5..7] == [0, 0],
            3 => u16::from_le_bytes([data[5], data[6]]) <= 31680,
            _ => false,
        }
}

/// D609 and D612 set legacy cell backgrounds; table-style readers ignore them above nFib D9.
/// Both version paths preserve character properties. This validates source shading without
/// determining its rendered appearance or changing the selected table style.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/b39a6648-501c-4361-8366-4f042f579469>
fn valid_cell_shading(sprm: u16, data: &[u8], columns: usize) -> bool {
    let Some(&size) = data.first() else {
        return false;
    };
    let stride = match sprm {
        0xd609 => 2,
        0xd612 if size <= 220 => 10,
        _ => return false,
    };
    if data.len() != usize::from(size) + 1
        || usize::from(size) % stride != 0
        || usize::from(size) / stride > columns
    {
        return false;
    }
    data[1..].chunks_exact(stride).all(|shading| {
        if stride == 2 {
            // Shd80Nil uses all bits set; other entries use two five-bit palette indices.
            let value = u16::from_le_bytes([shading[0], shading[1]]);
            value == 0xffff
                || (value & 31 <= 16
                    && (value >> 5) & 31 <= 16
                    && matches!(value >> 10, 0..=0x19 | 0x23..=0x3d))
        } else {
            // ShdAuto and ShdNil satisfy the COLORREF and Ipat grammar.
            matches!(shading[3], 0 | 255)
                && matches!(shading[7], 0 | 255)
                && matches!(
                    u16::from_le_bytes([shading[8], shading[9]]),
                    0..=0x19 | 0x23..=0x3d | 0xffff
                )
        }
    })
}

fn empty_style(
    index: usize,
    styles: &Stylesheet,
    cache: &mut BTreeMap<usize, Option<Vec<usize>>>,
    work: &mut usize,
) -> Result<Option<Vec<usize>>> {
    if let Some(value) = cache.get(&index) {
        charge(
            work,
            32 + value
                .as_ref()
                .map_or(0, |v| v.len() * std::mem::size_of::<usize>()),
        )?;
        return Ok(value.clone());
    }
    let result = style_chain(index, styles, work, |bytes, _| Ok(bytes.is_empty()))?;
    charge(
        work,
        result
            .as_ref()
            .map_or(32, |v| 32 + v.len() * 2 * std::mem::size_of::<usize>()),
    )?;
    cache.insert(index, result.clone());
    Ok(result)
}
fn style_chain(
    index: usize,
    styles: &Stylesheet,
    work: &mut usize,
    character_group: fn(&[u8], &mut usize) -> Result<bool>,
) -> Result<Option<Vec<usize>>> {
    let mut chain = Vec::new();
    let mut next = Some(index);
    let mut known = true;
    while let Some(current) = next {
        charge(work, 1)?;
        if chain.len() == MAX_ANCESTORS {
            return Err(Error::Limit("hidden table style depth"));
        }
        if chain.contains(&current) {
            return Err(Error::Invalid("hidden table style cycle"));
        }
        chain.push(current);
        let Some(style) = styles.styles.get(current).and_then(Option::as_ref) else {
            known = false;
            break;
        };
        if style.index != current
            || style.kind != 3
            || style.revision_marked
            || !style.raw_revision.is_empty()
            || style.linked_style.is_some()
        {
            known = false;
            break;
        }
        let mut seen = [false; 3];
        for group in &style.properties {
            charge(work, group.bytes.len() + 1)?;
            let slot = match group.kind {
                "table" => 0,
                "paragraph" => 1,
                "character" => 2,
                _ => {
                    known = false;
                    continue;
                }
            };
            if seen[slot] {
                return Err(Error::Invalid("hidden table duplicate style group"));
            }
            seen[slot] = true;
            match group.kind {
                "character" => known &= character_group(&group.bytes, work)?,
                "paragraph" => {
                    known &= group.bytes.len() == 2
                        && crate::office::doc::u16_at(&group.bytes, 0)? as usize == current
                }
                "table" => {
                    for p in paragraph::group(&group.bytes, 0, work)? {
                        known &= match p.sprm {
                            0x563a => {
                                crate::office::doc::u16_at(&p.operand, 0)? as usize == current
                            }
                            0xf617 | 0xf661 => {
                                p.operand.len() == 3
                                    && p.operand[0] == 3
                                    && crate::office::doc::u16_at(&p.operand, 1)? == 0
                            }
                            0xd634 => {
                                p.operand.len() == 7
                                    && p.operand[0] == 6
                                    && p.operand[1..3] == [0, 1]
                                    && matches!(p.operand[3], 5 | 10)
                                    && p.operand[4] == 3
                                    && crate::office::doc::u16_at(&p.operand, 5)? <= 31680
                            }
                            0xd613 => valid_borders(&p.operand),
                            _ => false,
                        };
                    }
                }
                _ => {}
            }
        }
        known &= seen.into_iter().all(|v| v);
        if !known {
            break;
        }
        next = style.base_style;
    }
    Ok(known.then_some(chain))
}

fn language_style(
    index: usize,
    styles: &Stylesheet,
    work: &mut usize,
) -> Result<Option<(Vec<usize>, language::TableLanguageContribution)>> {
    let Some(chain) = style_chain(index, styles, work, |bytes, work| {
        language::decode(&[(0, bytes)], work).map(|_| true)
    })?
    else {
        return Ok(None);
    };
    let mut groups = Vec::new();
    for &index in chain.iter().rev() {
        charge(work, 32)?;
        let style = styles
            .styles
            .get(index)
            .and_then(Option::as_ref)
            .ok_or(Error::Invalid("table language style identity"))?;
        let group = style
            .properties
            .iter()
            .find(|g| g.kind == "character")
            .ok_or(Error::Invalid("table language character group"))?;
        groups.push((index, group.bytes.as_slice()));
    }
    Ok(language::decode(&groups, work)?.map(|language| (chain, language)))
}

fn valid_borders(data: &[u8]) -> bool {
    data.len() == 49
        && data[0] == 48
        && data[1..]
            .as_chunks::<8>()
            .0
            .iter()
            .all(|b| matches!(b[3], 0 | 255) && matches!(b[5], 0 | 1 | 3 | 5..=27))
}

#[cfg(test)]
mod tests;
