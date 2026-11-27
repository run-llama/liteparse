//! Source-backed list labels, added after source-length-dependent projections.
mod counter;
mod direction;
mod inherited_bullet;
mod linked_style;
mod rtl;
mod table_style;
mod template;
mod textbox;
use crate::office::doc::{
    Error, Inspection, Result, bytes,
    field_inspection::FieldInspection,
    formatting::{PropertyModifier, decode_character_modifiers},
    formatting_inspection::FormattingInspection,
    lists::{ListDefinitions, ListLevel},
    numbering_revision::{self, NumberingRevision, NumberingRevisionEvidence},
    paragraph_formatting::ParagraphInspection,
    property_revision::{self, PropertyRevision, PropertyRevisionEvidence},
    styles::Stylesheet,
    symbols::{FontTable, map_symbol},
    transcription::{Diagnostic, SourceSegment, TranscribedStory, Transcription},
    u16_at,
    visibility::RevisionEvidence,
};
pub use direction::{ParagraphDirectionEvidence, TableDirectionEvidence};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

const MAX_ITEMS: usize = 262_144;
const MAX_REVISION_RECORDS: usize = 16_384;
const MAX_REVISION_BYTES: usize = 4 * 1024 * 1024;
const MAX_UNITS: usize = 16 * 1024 * 1024;

pub struct ListContext<'a> {
    pub source: &'a Inspection,
    pub definitions: &'a ListDefinitions,
    pub paragraphs: &'a ParagraphInspection,
    pub styles: &'a Stylesheet,
    pub formatting: &'a FormattingInspection,
    pub revisions: &'a RevisionEvidence,
    pub fonts: &'a FontTable,
    /// Source FIB/DOP excludes installation font overrides and template style updates.
    pub font_defaults_preserved: Option<bool>,
    pub fields: &'a FieldInspection,
}

#[derive(Clone, Default)]
struct Selection {
    ilfo: usize,
    level: u8,
    visible_style: bool,
    rtl_visible_style: bool,
    right_to_left: bool,
    direction_explicit: bool,
    language: LabelLanguage,
    in_table: bool,
    table_depth: u32,
    table_depth_explicit: bool,
    table_columns: Option<u8>,
    horizontal_merges: u64,
    vertical_merges: u64,
    border_color_columns: [Option<u8>; 4],
    table_row: bool,
    inner_cell: bool,
    inner_row: bool,
    nested_scope: bool,
    table_style: Option<u16>,
    table_options: bool,
    table_bidi: Option<bool>,
    table_bidi90: Option<bool>,
    table_placement: bool,
    table_appearance: bool,
    numbering_revision: Option<Arc<NumberingRevision>>,
    number_inserted: Option<bool>,
    paragraph_revision: Option<PropertyRevision>,
    table_revision: Option<PropertyRevision>,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct LabelLanguage {
    latin: Option<u16>,
    hint: u8,
}

struct Label {
    cp: usize,
    text: String,
}

/// Source selection for an emitted zero-width list label.
/// Layout resolves formatting separately from the already-qualified logical label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelSource {
    pub start_cp: usize,
    pub list_id: u32,
    /// One-based index into the source list-override table (iLfo).
    pub override_index: usize,
    /// Zero-based level selected by the physical paragraph.
    pub level: u8,
}

pub struct ListProjectionReport {
    pub transcription: Transcription,
    pub label_sources: Vec<LabelSource>,
    pub numbering_revision_evidence: Vec<NumberingRevisionEvidence>,
    pub property_revision_evidence: Vec<PropertyRevisionEvidence>,
    /// Active-RTL physical paragraphs visited by successful list planning.
    /// Empty records do not establish direction coverage for skipped stories or absent lists.
    pub paragraph_direction_evidence: Vec<ParagraphDirectionEvidence>,
    pub table_direction_evidence: Vec<TableDirectionEvidence>,
}

struct ListPlan {
    labels: Vec<Label>,
    label_sources: Vec<LabelSource>,
    diagnostics: Vec<Diagnostic>,
    numbering_revision_evidence: Vec<NumberingRevisionEvidence>,
    property_revision_evidence: Vec<PropertyRevisionEvidence>,
    paragraph_direction_evidence: Vec<ParagraphDirectionEvidence>,
    table_direction_evidence: Vec<TableDirectionEvidence>,
}

/// Insert supported labels at their paragraph-start CP without consuming source characters.
/// Zero-width generated_list_label segments are explicit anchors; other segments retain CP ownership.
/// MS-DOC §§2.4.6.3–4 define list identity, templates and sequence progression.
pub fn apply_lists(input: &Transcription, c: &ListContext<'_>) -> Result<Transcription> {
    Ok(project_lists(input, c)?.transcription)
}

/// Project accepted-current numbering and retain bounded historical snapshots at physical marks.
/// Failure leaves source PAPX and the input transcription available to the caller.
pub fn project_lists(input: &Transcription, c: &ListContext<'_>) -> Result<ListProjectionReport> {
    project_lists_with_font_source(input, c, None)
}

/// Project lists with the source embedded-font table state used by new symbol overrides.
/// An absent table permits published font encodings; unknown or present data stays guarded.
pub fn project_lists_with_font_source(
    input: &Transcription,
    c: &ListContext<'_>,
    embedded_fonts: Option<bool>,
) -> Result<ListProjectionReport> {
    project_lists_with_sources(input, c, embedded_fonts, None)
}

/// Project list labels with source-qualified font identity and physical table paragraphs.
/// Missing table evidence preserves unresolved table-member families.
pub fn project_lists_with_sources(
    input: &Transcription,
    c: &ListContext<'_>,
    embedded_fonts: Option<bool>,
    table_evidence: Option<&crate::office::doc::hidden_visibility::TableHiddenEvidence>,
) -> Result<ListProjectionReport> {
    project_lists_with_textboxes(input, c, embedded_fonts, table_evidence, None)
}

/// Project body lists and source-owned bullet paragraphs in active textbox records.
/// Textbox numeric sequences and page/shape visibility remain unresolved.
pub fn project_lists_with_textboxes(
    input: &Transcription,
    c: &ListContext<'_>,
    embedded_fonts: Option<bool>,
    table_evidence: Option<&crate::office::doc::hidden_visibility::TableHiddenEvidence>,
    textboxes: Option<&crate::office::doc::textbox_anchors::TextboxInspection>,
) -> Result<ListProjectionReport> {
    let mut label_sources = Vec::new();
    let mut numbering_revision_evidence = Vec::new();
    let mut property_revision_evidence = Vec::new();
    let mut paragraph_direction_evidence = Vec::new();
    let mut table_direction_evidence = Vec::new();
    let mut remaining_evidence_bytes = MAX_REVISION_BYTES;
    let mut out = Transcription {
        schema_version: input.schema_version,
        status: "partial_transcription",
        unresolved: input.unresolved.clone(),
        stories: Vec::new(),
    };
    out.unresolved
        .push("list_multilevel_overrides_styles_and_unsupported_templates");
    for story in &input.stories {
        let source = c
            .source
            .stories
            .iter()
            .find(|s| s.kind == story.kind)
            .ok_or(Error::Invalid("list source story"))?;
        validate(story, source.start_cp, source.utf16_units)?;
        if (story.kind != "body"
            && (!matches!(story.kind, "textboxes" | "header_textboxes") || textboxes.is_none()))
            || c.definitions.lists.is_empty()
        {
            out.stories.push(copy_story(story));
            continue;
        }
        let active = if story.kind == "body" {
            None
        } else {
            match textbox::active_ranges(
                source,
                textboxes.ok_or(Error::Invalid("textbox list metadata"))?,
            ) {
                Ok(ranges) => Some(ranges),
                Err(error @ Error::Limit(_)) => return Err(error),
                Err(_) => {
                    let mut retained = copy_story(story);
                    if retained.diagnostics.len() < 256 {
                        retained.diagnostics.push(Diagnostic {
                            code: "textbox_list_source_ranges_unresolved",
                            cp: source.start_cp,
                        });
                    }
                    out.stories.push(retained);
                    continue;
                }
            }
        };
        let planned = match plan(source, c, embedded_fonts, table_evidence, active.as_deref()) {
            Ok(value) => value,
            Err(error) if story.kind == "body" => return Err(error),
            Err(error @ Error::Limit(_)) => return Err(error),
            Err(_) => {
                let mut retained = copy_story(story);
                if retained.diagnostics.len() < 256 {
                    retained.diagnostics.push(Diagnostic {
                        code: "textbox_list_context_unresolved",
                        cp: source.start_cp,
                    });
                }
                out.stories.push(retained);
                continue;
            }
        };
        let ListPlan {
            labels,
            label_sources: sources,
            diagnostics,
            numbering_revision_evidence: evidence,
            property_revision_evidence: property_evidence,
            paragraph_direction_evidence: direction_evidence,
            table_direction_evidence: row_direction_evidence,
        } = planned;
        if label_sources.len() + sources.len() > MAX_ITEMS {
            return Err(Error::Limit("list label source records"));
        }
        label_sources.extend(sources);
        if !evidence.is_empty() {
            out.unresolved.push("historical_numbering_view_unresolved");
        }
        if numbering_revision_evidence.len() + evidence.len() > MAX_REVISION_RECORDS {
            return Err(Error::Limit("numbering revision evidence records"));
        }
        for record in &evidence {
            remaining_evidence_bytes = remaining_evidence_bytes
                .checked_sub(revision_evidence_size(record.snapshot.as_ref()))
                .ok_or(Error::Limit("numbering revision evidence bytes"))?;
        }
        numbering_revision_evidence.extend(evidence);
        if !property_evidence.is_empty() {
            out.unresolved.push("historical_property_view_unresolved");
        }
        if property_revision_evidence.len() + property_evidence.len() > MAX_REVISION_RECORDS {
            return Err(Error::Limit("property revision evidence records"));
        }
        remaining_evidence_bytes = remaining_evidence_bytes
            .checked_sub(property_evidence.len() * std::mem::size_of::<PropertyRevisionEvidence>())
            .ok_or(Error::Limit("property revision evidence bytes"))?;
        property_revision_evidence.extend(property_evidence);
        if !direction_evidence.is_empty() {
            out.unresolved
                .push("rtl_visual_order_and_shaping_unresolved");
        }
        if direction_evidence
            .iter()
            .any(|r| r.list_override_index.is_some() && r.label_properties.is_none())
        {
            out.unresolved.push("rtl_list_label_properties_unresolved");
        }
        if paragraph_direction_evidence.len() + direction_evidence.len() > MAX_REVISION_RECORDS {
            return Err(Error::Limit("paragraph direction evidence records"));
        }
        remaining_evidence_bytes = remaining_evidence_bytes
            .checked_sub(
                direction_evidence.len() * std::mem::size_of::<ParagraphDirectionEvidence>(),
            )
            .ok_or(Error::Limit("paragraph direction evidence bytes"))?;
        paragraph_direction_evidence.extend(direction_evidence);
        if table_direction_evidence.len() + row_direction_evidence.len() > MAX_REVISION_RECORDS {
            return Err(Error::Limit("table direction evidence records"));
        }
        remaining_evidence_bytes = remaining_evidence_bytes
            .checked_sub(
                row_direction_evidence.len() * std::mem::size_of::<TableDirectionEvidence>(),
            )
            .ok_or(Error::Limit("table direction evidence bytes"))?;
        if row_direction_evidence.iter().any(|e| e.right_to_left) {
            out.unresolved
                .push("table_direction_visual_order_unresolved");
        }
        table_direction_evidence.extend(row_direction_evidence);
        let mut projected = insert(story, &labels)?;
        if diagnostics
            .iter()
            .any(|d| d.code == "table_visual_placement_unresolved")
        {
            out.unresolved.push("table_visual_placement_unresolved");
        }
        if diagnostics
            .iter()
            .any(|d| d.code == "merged_cell_visible_ownership_unresolved")
        {
            out.unresolved
                .push("merged_cell_visible_ownership_unresolved");
        }
        if diagnostics
            .iter()
            .any(|d| d.code == "table_visual_appearance_unresolved")
        {
            out.unresolved.push("table_visual_appearance_unresolved");
        }
        for d in diagnostics {
            if projected.diagnostics.len() < 256 {
                projected.diagnostics.push(d);
            }
        }
        out.stories.push(projected);
    }
    Ok(ListProjectionReport {
        transcription: out,
        label_sources,
        numbering_revision_evidence,
        property_revision_evidence,
        paragraph_direction_evidence,
        table_direction_evidence,
    })
}

fn override_font_source_known(
    level: &ListLevel,
    fonts: &FontTable,
    embedded_fonts: Option<bool>,
) -> Result<bool> {
    if embedded_fonts == Some(false) {
        return Ok(true);
    }
    let Some(&code) = level.number_text_utf16.first() else {
        return Ok(false);
    };
    if code & 0xf000 != 0 {
        return Ok(false);
    }
    let op = if (0x20..=0x7f).contains(&code) {
        0x4a4f
    } else {
        0x4a51
    };
    let properties = decode_character_modifiers(&level.raw_chpx)?;
    let Some(font) = properties
        .iter()
        .rev()
        .find(|p| p.sprm == op)
        .and_then(|p| u16_at(&p.operand, 0).ok())
        .and_then(|i| fonts.fonts.get(usize::from(i)))
    else {
        return Ok(false);
    };
    Ok(font.charset != 2)
}

/// Select the complete formatting override for a simple bullet level.
/// MS-DOC §2.4.6.3 part1step7 replaces the LVL, and §2.4.6.4 gives bullets no counter.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/1a0bc623-211f-44f6-828b-51993f16e586>
fn effective_bullet_override<'a>(
    list: &'a crate::office::doc::lists::ListDefinition,
    over: &'a crate::office::doc::lists::ListOverride,
    index: u8,
) -> Result<Option<&'a ListLevel>> {
    let base = list
        .levels
        .get(usize::from(index))
        .ok_or(Error::Invalid("list paragraph level"))?;
    if over.levels.is_empty() {
        return Ok(Some(base));
    }
    if list.flags & 1 == 0 || list.levels.len() != 1 || index != 0 || over.levels.len() != 1 {
        return Ok(None);
    }
    let entry = &over.levels[0];
    if entry.level != 0
        || entry.raw_flags & 15 != 0
        || entry.raw_flags & 0x30 != 0x20
        || entry.start_override.is_some()
    {
        return Ok(None);
    }
    let Some(level) = &entry.format_override else {
        return Ok(None);
    };
    if level.number_format != 0x17 {
        return Ok(None);
    }
    Ok(Some(level))
}

struct EffectiveLevel<'a> {
    level: &'a ListLevel,
    first_start: Option<i32>,
    counter_initial: i32,
}

/// LFOLVL replaces the complete level when fFormatting is set. Its scalar start
/// is defined only for a start-only override; combined flags use the embedded LVL.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/2555c6ec-604b-48fd-8484-a2fdbb95b42c>
fn effective_level<'a>(
    list: &'a crate::office::doc::lists::ListDefinition,
    over: &'a crate::office::doc::lists::ListOverride,
    index: u8,
) -> Result<Option<EffectiveLevel<'a>>> {
    let base = list
        .levels
        .get(usize::from(index))
        .ok_or(Error::Invalid("list paragraph level"))?;
    let mut seen = 0u16;
    for entry in &over.levels {
        if usize::from(entry.level) >= list.levels.len()
            || entry.raw_flags & 15 != u32::from(entry.level)
            || seen & (1 << entry.level) != 0
            || (entry.raw_flags & 0x20 != 0) != entry.format_override.is_some()
            || (entry.raw_flags & 0x30 == 0x10) != entry.start_override.is_some()
        {
            return Ok(None);
        }
        seen |= 1 << entry.level;
    }
    let mut entries = over.levels.iter().filter(|entry| entry.level == index);
    let entry = entries.next();
    if entries.next().is_some() {
        return Ok(None);
    }
    let level = entry
        .and_then(|e| e.format_override.as_ref())
        .unwrap_or(base);
    if level.number_format == 0x17 {
        return Ok(
            effective_bullet_override(list, over, index)?.map(|level| EffectiveLevel {
                level,
                first_start: None,
                counter_initial: 0,
            }),
        );
    }
    if level.number_format == 0xff || level.flags & 8 != 0 && level.restart_limit > index {
        return Ok(None);
    }
    let first_start = if let Some(entry) = entry {
        if entry.raw_flags & 15 != u32::from(index)
            || (entry.raw_flags & 0x20 != 0) != entry.format_override.is_some()
            || (entry.raw_flags & 0x30 == 0x10) != entry.start_override.is_some()
        {
            return Ok(None);
        }
        if entry.raw_flags & 0x10 != 0 {
            Some(if entry.raw_flags & 0x20 != 0 {
                level.start_at
            } else {
                entry.start_override.unwrap()
            })
        } else {
            None
        }
    } else {
        None
    };
    if first_start.is_some_and(|value| !(0..=0x7fff).contains(&value)) {
        return Ok(None);
    }
    // Word 97–2007 specification p48, paragraph formatting steps 2–4:
    // formatting-only overrides obtain their missing start-at value from the LST.
    let counter_initial = if entry.is_some_and(|e| e.raw_flags & 0x30 == 0x30) {
        level.start_at
    } else {
        base.start_at
    };
    Ok(Some(EffectiveLevel {
        level,
        first_start,
        counter_initial,
    }))
}

fn plan(
    source: &crate::office::doc::Story,
    c: &ListContext<'_>,
    embedded_fonts: Option<bool>,
    table_evidence: Option<&crate::office::doc::hidden_visibility::TableHiddenEvidence>,
    active: Option<&[[usize; 2]]>,
) -> Result<ListPlan> {
    if source.utf16_units > MAX_UNITS
        || c.paragraphs.spans.len() > MAX_ITEMS
        || c.revisions.spans.len() > MAX_ITEMS
    {
        return Err(Error::Limit("list projection input"));
    }
    if c.source.binary_version != c.paragraphs.binary_version
        || c.source.binary_version != c.fields.binary_version
        || c.source.binary_version != c.formatting.binary_version
    {
        return Err(Error::Invalid("list metadata version alignment"));
    }
    if c.formatting.pieces.len() > MAX_ITEMS
        || c.formatting
            .pieces
            .windows(2)
            .any(|p| p[0].end_cp != p[1].start_cp)
        || c.revisions
            .spans
            .windows(2)
            .any(|p| p[0].end_cp != p[1].start_cp)
    {
        return Err(Error::Invalid("list metadata span order or limit"));
    }
    if source.text.encode_utf16().count() != source.utf16_units {
        return Err(Error::Invalid("list source units"));
    }
    let legacy_unstyled_tables = matches!(c.source.binary_version, 0x00c1 | 0x00d9)
        && c.styles.styles.get(11).is_none_or(Option::is_none)
        && !c.styles.styles.iter().flatten().any(|s| s.kind == 3);
    // MS-DOC Appendix A, notes 9–10: Word 97/2000 have no table styles.
    // An absent legacy default is valid only without a table-style definition;
    // explicit style selections below still require a resolved default.
    let default_table_direction = if source.text.contains('\u{7}') && !legacy_unstyled_tables {
        validate_default_table_style(c.styles)?
    } else {
        (None, None)
    };
    let mut validated_table_styles = BTreeSet::new();
    let mut table_style_bytes = MAX_REVISION_BYTES;
    if c.definitions.lists.iter().any(|list| {
        list.levels.is_empty()
            || list.levels.len() > 9
            || list
                .levels
                .iter()
                .enumerate()
                .any(|(i, l)| l.flags & 8 != 0 && l.restart_limit as usize > i)
    }) {
        return Err(Error::Invalid("list levels or restart limit"));
    }
    let end = source
        .start_cp
        .checked_add(source.utf16_units)
        .ok_or(Error::Invalid("list source overflow"))?;
    let mut covered = source.start_cp;
    for p in c
        .formatting
        .pieces
        .iter()
        .filter(|p| p.end_cp > source.start_cp && p.start_cp < end)
    {
        if p.start_cp > covered || p.end_cp <= covered || p.raw_prm != 0 {
            return Err(Error::Unsupported("list piece properties"));
        }
        covered = p.end_cp.min(end);
    }
    if covered != end {
        return Err(Error::Invalid("list piece coverage"));
    }
    covered = source.start_cp;
    for s in c
        .revisions
        .spans
        .iter()
        .filter(|s| s.end_cp > source.start_cp && s.start_cp < end)
    {
        if s.start_cp > covered || s.end_cp <= covered || s.properties.deleted != Some(false) {
            return Err(Error::Unsupported("list revision visibility"));
        }
        covered = s.end_cp.min(end);
    }
    if covered != end {
        return Err(Error::Invalid("list revision coverage"));
    }
    if c.paragraphs.spans.iter().any(|p| p.start_cp >= p.end_cp)
        || c.paragraphs
            .spans
            .windows(2)
            .any(|p| p[0].end_cp > p[1].start_cp)
    {
        return Err(Error::Invalid("list paragraph span order"));
    }
    if active.is_some()
        && c.fields
            .stories
            .iter()
            .filter(|s| s.kind == source.kind)
            .count()
            != 1
    {
        return Err(Error::Invalid("textbox list field story identity"));
    }
    let row = c
        .fields
        .stories
        .iter()
        .find(|s| s.kind == source.kind)
        .ok_or(Error::Invalid("list field story"))?;
    if row.start_cp != source.start_cp || row.utf16_units != source.utf16_units {
        return Err(Error::Invalid("list field story alignment"));
    }
    let fields = row
        .table
        .as_ref()
        .filter(|t| t.topology_valid)
        .ok_or(Error::Unsupported("list field topology"))?;
    if fields.unindexed_marker_units != 0 {
        return Err(Error::Unsupported("list unindexed fields"));
    }
    if c.formatting.character_spans.len() > MAX_ITEMS
        || c.formatting
            .character_spans
            .iter()
            .any(|s| s.start_cp >= s.end_cp)
        || c.formatting
            .character_spans
            .windows(2)
            .any(|s| s[0].end_cp != s[1].start_cp)
    {
        return Err(Error::Invalid("list character span order"));
    }
    let mut alphabet_default_safe = true;
    if let Some(style) = c.styles.styles.get(10).and_then(Option::as_ref) {
        if style.base_style.is_some() || style.revision_marked || !style.raw_revision.is_empty() {
            return Err(Error::Unsupported("list default character style"));
        }
        for p in style.properties.iter().filter(|p| p.kind == "character") {
            let modifiers = decode_character_modifiers(&p.bytes)?;
            alphabet_default_safe &= !modifiers
                .iter()
                .any(|p| matches!(p.sprm, 0x486d | 0x4873 | 0x286f));
            if !modifiers.iter().all(visible_modifier) {
                return Err(Error::Unsupported("list hidden default character style"));
            }
        }
    }
    if fields.ranges.len() > MAX_ITEMS {
        return Err(Error::Limit("list field ranges"));
    }
    let mut field_ranges: Vec<_> = fields
        .ranges
        .iter()
        .map(|f| (f.source.start, f.source.end))
        .collect();
    field_ranges.sort_unstable();
    let mut greatest_end = 0;
    for (_, end) in &mut field_ranges {
        greatest_end = greatest_end.max(*end);
        *end = greatest_end;
    }
    let mut styles = BTreeMap::new();
    let mut items = Vec::new();
    let mut numbering_revision_evidence = Vec::new();
    let mut property_revision_evidence = Vec::new();
    let mut paragraph_direction_evidence = Vec::new();
    let mut table_direction_evidence = Vec::new();
    let mut remaining_revision_bytes = MAX_REVISION_BYTES;
    let mut start = source.start_cp;
    let mut cp = start;
    let mut paragraph_count = 0;
    let mut previous = None;
    let mut open_row = false;
    let mut row_cells = 0u8;
    let mut row_requires_definition = false;
    let mut placement_anchor = None;
    let mut appearance_anchor = None;
    let mut merge_anchor = None;
    let mut section_only_anchor = None;
    for ch in source.text.chars() {
        if c.paragraphs.status == "reference_expanded_endpoint_selected_paragraphs"
            && !matches!(ch, '\r' | '\u{7}' | '\u{c}')
            && c.paragraphs
                .spans
                .get(c.paragraphs.spans.partition_point(|p| p.end_cp <= cp))
                .is_some_and(|p| p.end_cp == cp + ch.len_utf16())
        {
            return Err(Error::Unsupported("list physical paragraph terminal"));
        }
        if matches!(ch, '\r' | '\u{7}')
            || (ch == '\u{c}'
                && c.paragraphs.status == "reference_expanded_endpoint_selected_paragraphs")
        {
            paragraph_count += 1;
            if paragraph_count > MAX_ITEMS {
                return Err(Error::Limit("list paragraphs"));
            }
            let index = c.paragraphs.spans.partition_point(|p| p.end_cp <= cp);
            if c.paragraphs.status == "reference_expanded_endpoint_selected_paragraphs"
                && c.paragraphs
                    .spans
                    .get(index)
                    .is_some_and(|p| p.start_cp <= cp && p.end_cp > cp + 1)
            {
                cp += ch.len_utf16();
                previous = Some(ch);
                continue;
            }
            if let Some(ranges) = active {
                let i = ranges.partition_point(|range| range[1] <= start);
                if !ranges
                    .get(i)
                    .is_some_and(|range| range[0] <= start && cp < range[1])
                {
                    start = cp + 1;
                    cp += ch.len_utf16();
                    previous = Some(ch);
                    continue;
                }
            }
            let p = c
                .paragraphs
                .spans
                .get(index)
                .filter(|p| p.start_cp <= cp && p.end_cp == cp + 1)
                .ok_or(Error::Invalid("list paragraph coverage"))?;
            let mut selection = style_selection(
                p.style_index.unwrap_or(0) as usize,
                c.styles,
                &mut styles,
                0,
            )?;
            apply_paragraph(&mut selection, &p.raw_modifiers)?;

            retain_numbering_revision(
                &mut numbering_revision_evidence,
                &mut remaining_revision_bytes,
                cp,
                &selection,
            )?;
            retain_property_revision(
                &mut property_revision_evidence,
                &mut remaining_revision_bytes,
                cp,
                &selection,
            )?;
            if let Some(index) = selection.table_style {
                if legacy_unstyled_tables {
                    return Err(Error::Unsupported("list nondefault table style"));
                }
                if index != 11 {
                    if table_evidence
                        .and_then(|proof| proof.nested_list_context([start, cp + 1]))
                        .is_some()
                    {
                        table_style::validate_nested_visibility(
                            index as usize,
                            c.styles,
                            &mut table_style_bytes,
                        )?;
                    } else {
                        table_style::validate(
                            index as usize,
                            c.styles,
                            &mut validated_table_styles,
                            &mut table_style_bytes,
                        )?;
                    }
                }
            }
            if selection.table_options && !selection.in_table {
                return Err(Error::Unsupported("list table options outside table"));
            }
            if selection.table_appearance {
                if !selection.in_table {
                    return Err(Error::Unsupported("list table appearance outside table"));
                }
                appearance_anchor.get_or_insert(start);
            }
            if selection.table_placement {
                if !selection.in_table {
                    return Err(Error::Unsupported("list table placement outside table"));
                }
                placement_anchor.get_or_insert(start);
            }
            // Word 97 processes paragraph modifiers only through ispmd 0x48;
            // PItap (0x49) is unavailable. Its nonnested cells use U+0007 marks.
            // Require a matching TDefTable row definition before using this depth.
            if c.source.binary_version == 0x00c1
                && selection.in_table
                && !selection.table_depth_explicit
            {
                selection.table_depth = 1;
                row_requires_definition = true;
            }
            let nested_context =
                table_evidence.and_then(|proof| proof.nested_list_context([start, cp + 1]));
            if let Some(proof) = nested_context {
                if c.source.binary_version == 0x00c1
                    || !selection.in_table
                    || !selection.table_depth_explicit
                    || selection.table_depth != proof.depth
                    || selection.table_row != (proof.depth == 1 && proof.row_end)
                    || selection.inner_row != (proof.depth > 1 && proof.row_end)
                    || (!proof.row_end
                        && selection.inner_cell != (proof.depth > 1 && proof.cell_end))
                    || (proof.depth == 1 && selection.inner_cell)
                    || (start == proof.root[0] && open_row)
                    || selection
                        .table_columns
                        .is_some_and(|n| !proof.row_end || n != proof.columns)
                    || selection
                        .border_color_columns
                        .iter()
                        .flatten()
                        .any(|n| !proof.row_end || *n != proof.columns)
                {
                    return Err(Error::Unsupported("list nested table ownership mismatch"));
                }
                selection.nested_scope = true;
                open_row = cp + 1 != proof.root[1];
            } else {
                if selection.inner_cell || selection.inner_row {
                    return Err(Error::Unsupported("list nested table ownership unresolved"));
                }
                // Modern row definitions require an explicit nonnested depth; Word 97 uses the
                // validated implicit-depth rule above. Physical cell marks still bound every row.
                if selection.table_columns.is_some()
                    && (!selection.table_row
                        || !selection.in_table
                        || (c.source.binary_version != 0x00c1
                            && (!selection.table_depth_explicit || selection.table_depth != 1)))
                {
                    return Err(Error::Unsupported("list table definition scope"));
                }
                if selection.border_color_columns.iter().any(Option::is_some)
                    && (!selection.in_table || !selection.table_row)
                {
                    return Err(Error::Unsupported(
                        "list border colors outside row definition",
                    ));
                }
                if selection.in_table {
                    if ch == '\u{c}' {
                        return Err(Error::Unsupported("list section endpoint inside table"));
                    }
                    if selection.table_depth != 1
                        || (selection.table_row && (ch != '\u{7}' || previous != Some('\u{7}')))
                    {
                        return Err(Error::Unsupported("list table row topology"));
                    }
                    if selection.table_row {
                        if selection
                            .border_color_columns
                            .iter()
                            .flatten()
                            .any(|n| *n != row_cells)
                        {
                            return Err(Error::Invalid("list border color cell count"));
                        }
                        if (row_requires_definition || selection.table_columns.is_some())
                            && selection.table_columns != Some(row_cells)
                        {
                            return Err(Error::Invalid("list legacy table cell count"));
                        }
                        if selection.horizontal_merges | selection.vertical_merges != 0 {
                            merge_anchor.get_or_insert(start);
                        }
                        row_cells = 0;
                        row_requires_definition = false;
                    } else if ch == '\u{7}' {
                        row_cells = row_cells
                            .checked_add(1)
                            .filter(|n| *n <= 63)
                            .ok_or(Error::Limit("list table cells"))?;
                    }
                    open_row = !selection.table_row;
                } else if selection.table_depth != 0
                    || selection.table_row
                    || ch == '\u{7}'
                    || open_row
                {
                    return Err(Error::Unsupported("list table paragraph boundaries"));
                }
            }
            if (selection.table_bidi.is_some() || selection.table_bidi90.is_some())
                && (!selection.in_table || !(selection.table_row || selection.inner_row))
            {
                return Err(Error::Unsupported("list table direction row scope"));
            }
            if selection.in_table && selection.table_row {
                let bidi = selection.table_bidi.or(default_table_direction.0);
                let bidi90 = selection.table_bidi90.or(default_table_direction.1);
                if bidi.is_some() || bidi90.is_some() {
                    if table_direction_evidence.len() >= MAX_REVISION_RECORDS {
                        return Err(Error::Limit("table direction evidence records"));
                    }
                    remaining_revision_bytes = remaining_revision_bytes
                        .checked_sub(std::mem::size_of::<TableDirectionEvidence>())
                        .ok_or(Error::Limit("table direction evidence bytes"))?;
                    table_direction_evidence.push(TableDirectionEvidence {
                        paragraph_start_cp: start,
                        row_mark_cp: cp,
                        tf_bidi: bidi,
                        tf_bidi90: bidi90,
                        right_to_left: bidi.unwrap_or(false) || bidi90.unwrap_or(false),
                        visual_order: "unresolved",
                    });
                }
            }
            retain_direction(
                &mut paragraph_direction_evidence,
                &mut remaining_revision_bytes,
                start,
                cp,
                &selection,
            )?;
            if selection.ilfo != 0 && selection.level != 12 {
                // An exact section-only paragraph retains its properties without generating ink.
                // Its visible numbering remains explicit until independently verified in Word.
                if ch == '\u{c}' && start == cp {
                    section_only_anchor.get_or_insert(start);
                } else {
                    items.push((start, cp, selection));
                }
            }
            start = cp + 1;
        }
        cp += ch.len_utf16();
        previous = Some(ch);
    }
    if open_row {
        return Err(Error::Unsupported("list incomplete table row"));
    }
    if start != end {
        return Err(Error::Unsupported("list unterminated paragraph"));
    }
    let mut blocked = BTreeSet::new();
    let mut resolved = Vec::new();
    let mut bullet_override_families = BTreeSet::new();
    let mut numbered_families = BTreeSet::new();
    let mut bullet_families = BTreeSet::new();
    let mut numeric_override_families = BTreeSet::new();
    let mut font_unresolved_families = BTreeSet::new();
    let mut diagnostics = Vec::new();
    if let Some(cp) = section_only_anchor {
        diagnostics.push(Diagnostic {
            code: "section_only_list_visibility_unresolved",
            cp,
        });
    }
    if let Some(cp) = appearance_anchor {
        diagnostics.push(Diagnostic {
            code: "table_visual_appearance_unresolved",
            cp,
        });
    }
    if let Some(cp) = placement_anchor {
        diagnostics.push(Diagnostic {
            code: "table_visual_placement_unresolved",
            cp,
        });
    }
    if let Some(cp) = merge_anchor {
        diagnostics.push(Diagnostic {
            code: "merged_cell_visible_ownership_unresolved",
            cp,
        });
    }
    let mut rtl_property_budget = 8 * 1024 * 1024usize;
    let mut linked_styles = linked_style::Cache::new();
    let mut inherited_bullet_budget = 8 * 1024 * 1024usize;
    for (start, mark, selection) in items {
        let over = c
            .definitions
            .overrides
            .get(selection.ilfo - 1)
            .ok_or(Error::Invalid("list paragraph override index"))?;
        let list = c
            .definitions
            .lists
            .get(over.list_index)
            .filter(|l| l.lsid == over.lsid)
            .ok_or(Error::Invalid("list override identity"))?;
        let base_level = list
            .levels
            .get(selection.level as usize)
            .ok_or(Error::Invalid("list paragraph level"))?;
        let effective = effective_level(list, over, selection.level)?;
        let level = effective.as_ref().map_or(base_level, |e| e.level);
        if active.is_some() && level.number_format != 0x17 {
            blocked.insert(list.lsid);
            if diagnostics.len() < 256 {
                diagnostics.push(Diagnostic {
                    code: "textbox_numeric_list_sequence_unresolved",
                    cp: start,
                });
            }
            continue;
        }
        let first_start = effective.as_ref().and_then(|e| e.first_start);
        let counter_initial = effective
            .as_ref()
            .map_or(base_level.start_at, |e| e.counter_initial);
        if !over.levels.is_empty() && effective.is_some() {
            if level.number_format == 0x17 {
                bullet_override_families.insert(over.lsid);
            } else {
                numeric_override_families.insert(over.lsid);
            }
        }
        if matches!(level.number_format, 0x17 | 0xff) {
            bullet_families.insert(over.lsid);
        }
        if level.number_format != 0x17 {
            numbered_families.insert(over.lsid);
        }
        if !override_font_source_known(level, c.fonts, embedded_fonts).unwrap_or(false) {
            font_unresolved_families.insert(over.lsid);
        }
        let at_start = field_ranges.partition_point(|f| f.0 < start);
        let at_end = field_ranges.partition_point(|f| f.0 <= mark);
        let field_overlap = (at_start != 0 && field_ranges[at_start - 1].1 > start)
            || (at_end != 0 && field_ranges[at_end - 1].1 > mark);
        let revision_index = c.revisions.spans.partition_point(|s| s.end_cp <= mark);
        let visible_paragraph_mark = c
            .revisions
            .spans
            .get(revision_index)
            .is_some_and(|s| s.start_cp <= mark && s.properties.field_hidden == Some(false));
        let mut supported = (!selection.nested_scope
            || table_evidence.is_some_and(|proof| proof.permits_list_paragraph([start, mark + 1])))
            && (!selection.in_table
                || (!selection.table_row
                    && table_evidence
                        .is_some_and(|proof| proof.permits_list_paragraph([start, mark + 1]))))
            && (if selection.right_to_left {
                selection.rtl_visible_style
            } else {
                selection.visible_style
            })
            && !field_overlap
            && visible_paragraph_mark
            && (selection.right_to_left || visible_mark(c.formatting, mark))
            && effective.is_some()
            && (over.levels.is_empty()
                || override_font_source_known(level, c.fonts, embedded_fonts)?)
            && matches!(over.field_type, 0 | 0xff)
            && list.flags & 4 == 0
            && !(list.flags & 16 != 0 && level.flags & 128 != 0);
        let mut language = selection.language;
        let mark_index = c
            .formatting
            .character_spans
            .partition_point(|s| s.end_cp <= mark);
        if let Some(span) = c
            .formatting
            .character_spans
            .get(mark_index)
            .filter(|s| s.start_cp <= mark)
        {
            language.apply(&span.properties)?;
        }
        let linked = list.style_links[selection.level as usize] as usize;
        if linked != 0xfff {
            let paragraph = c
                .paragraphs
                .spans
                .get(c.paragraphs.spans.partition_point(|p| p.end_cp <= mark));
            let qualified = paragraph
                .filter(|p| p.end_cp == mark + 1 && p.style_index.unwrap_or(0) as usize == linked)
                .and_then(|_| styles.get(&linked))
                .zip(
                    c.formatting
                        .character_spans
                        .get(mark_index)
                        .filter(|p| p.start_cp <= mark),
                )
                .and_then(|(baseline, direct)| {
                    linked_styles
                        .qualify(c, linked, baseline, &selection, level, &direct.properties)
                        .ok()
                });
            if let Some(baseline_language) = qualified {
                language = baseline_language;
            } else {
                supported = false;
            }
        }
        let inherited_font = if supported
            && level.number_format == 0x17
            && c.font_defaults_preserved == Some(true)
            && embedded_fonts == Some(false)
            && !selection.right_to_left
            && !selection.in_table
            && linked == 0xfff
            && over.levels.is_empty()
            && !decode_character_modifiers(&level.raw_chpx)?
                .iter()
                .any(|p| p.sprm == 0x4a4f)
        {
            match inherited_bullet::resolve(c, start, mark, level, &mut inherited_bullet_budget) {
                Ok(proof) => Some(proof),
                Err(error @ Error::Limit(_)) => return Err(error),
                Err(_) => None,
            }
        } else {
            None
        };
        let mut rtl_properties = None;
        let text_check = if selection.right_to_left {
            let paragraph = c
                .paragraphs
                .spans
                .get(c.paragraphs.spans.partition_point(|p| p.end_cp <= mark))
                .filter(|p| p.end_cp == mark + 1)
                .ok_or(Error::Invalid("list RTL paragraph endpoint"))?;
            let direct = c
                .formatting
                .character_spans
                .get(mark_index)
                .filter(|s| s.start_cp <= mark)
                .map(|s| s.properties.as_slice())
                .ok_or(Error::Invalid("list RTL character coverage"))?;
            match rtl::resolve(
                c.styles,
                paragraph.style_index.unwrap_or(0) as usize,
                direct,
                level,
                c.fonts,
                &mut rtl_property_budget,
            ) {
                Ok(properties) => {
                    rtl_properties = Some(properties);
                    rtl_label_text(level, level.start_at, selection.level)
                }
                Err(error) => Err(error),
            }
        } else if matches!(level.number_format, 3 | 4) && !alphabet_default_safe {
            Err(Error::Unsupported("list alphabet language"))
        } else {
            label_text_with_ancestors(
                level,
                if !over.levels.is_empty() && level.number_format == 0x17 {
                    0
                } else {
                    level.start_at
                },
                c.fonts,
                selection.level,
                language,
                inherited_font,
                |_| Ok(String::new()),
            )
        };
        if !supported || text_check.is_err() {
            blocked.insert(over.lsid);
            if diagnostics.len() < 256 {
                diagnostics.push(Diagnostic {
                    code: if matches!(
                        text_check,
                        Err(Error::Unsupported("list alphabet language"))
                    ) {
                        "list_alphabet_language_unresolved"
                    } else if selection.right_to_left
                        || matches!(
                            text_check,
                            Err(Error::Unsupported("list active right-to-left paragraph"))
                        )
                    {
                        "list_active_right_to_left_unresolved"
                    } else {
                        "list_sequence_unsupported"
                    },
                    cp: start,
                });
            }
        }
        resolved.push((
            start,
            over.lsid,
            selection.ilfo,
            selection.level,
            list,
            level,
            language,
            rtl_properties,
            first_start,
            counter_initial,
            inherited_font,
        ));
    }
    // Numbered members sharing an overridden bullet family require separate counter semantics.
    for lsid in bullet_override_families.intersection(&numbered_families) {
        blocked.insert(*lsid);
        if diagnostics.len() < 256
            && let Some(row) = resolved.iter().find(|r| r.1 == *lsid)
        {
            diagnostics.push(Diagnostic {
                code: "list_mixed_bullet_override_sequence_unresolved",
                cp: row.0,
            });
        }
    }
    // Previously blocked plain members become eligible with the overriding family.
    // Their symbol-font identities require the same source proof as overriding members.
    for lsid in bullet_override_families.intersection(&font_unresolved_families) {
        blocked.insert(*lsid);
        if diagnostics.len() < 256
            && let Some(row) = resolved.iter().find(|r| r.1 == *lsid)
        {
            diagnostics.push(Diagnostic {
                code: "list_override_font_source_unresolved",
                cp: row.0,
            });
        }
    }
    for lsid in numeric_override_families.intersection(&bullet_families) {
        blocked.insert(*lsid);
        if diagnostics.len() < 256
            && let Some(row) = resolved.iter().find(|r| r.1 == *lsid)
        {
            diagnostics.push(Diagnostic {
                code: "list_mixed_numeric_override_sequence_unresolved",
                cp: row.0,
            });
        }
    }
    for lsid in numeric_override_families.intersection(&font_unresolved_families) {
        blocked.insert(*lsid);
    }
    let mut counters = counter::Counters::default();
    let mut labels = Vec::new();
    let mut ancestors: BTreeMap<usize, [Option<template::Ancestor>; 9]> = BTreeMap::new();
    let mut remaining_label_bytes = 8 * 1024 * 1024usize;
    for (
        cp,
        lsid,
        ilfo,
        index,
        _list,
        level,
        language,
        rtl_properties,
        first_start,
        counter_initial,
        inherited_font,
    ) in resolved
    {
        if blocked.contains(&lsid) {
            continue;
        }
        let (initial, threshold) = if level.number_format == 0x17 {
            (0, index)
        } else {
            (
                counter_initial,
                if level.flags & 8 != 0 {
                    level.restart_limit
                } else {
                    index
                },
            )
        };
        let current_value =
            counters.advance(lsid, ilfo, index, cp, initial, threshold, first_start)?;
        let history = ancestors.entry(ilfo).or_insert([None; 9]);
        let is_rtl = rtl_properties.is_some();
        let text_result = if let Some(properties) = rtl_properties {
            let text = rtl_label_text(level, current_value, index);
            let evidence_index = paragraph_direction_evidence.partition_point(|r| r.start_cp < cp);
            if let Some(record) = paragraph_direction_evidence
                .get_mut(evidence_index)
                .filter(|r| r.start_cp == cp)
            {
                record.label_properties = Some(properties);
                record.list_label_status = "logical_label_generated_visual_shaping_unresolved";
            }
            text
        } else {
            label_text_with_ancestors(
                level,
                current_value,
                c.fonts,
                index,
                language,
                inherited_font,
                |ancestor| {
                    let prior = history[ancestor as usize]
                        .ok_or(Error::Unsupported("list absent ancestor paragraph"))?;
                    prior.render(level.flags & 4 != 0)
                },
            )
        };
        let text = match text_result {
            Ok(text) => text,
            Err(error) => {
                // A missing ancestor prevents this label's display text. Its qualified
                // counter event still determines later current-level numbers and restarts.
                let missing_ancestor_only =
                    matches!(error, Error::Unsupported("list absent ancestor paragraph"))
                        && label_text_with_ancestors(
                            level,
                            current_value,
                            c.fonts,
                            index,
                            language,
                            inherited_font,
                            |ancestor| {
                                history[ancestor as usize].map_or_else(
                                    || Ok(String::new()),
                                    |prior| prior.render(level.flags & 4 != 0),
                                )
                            },
                        )
                        .is_ok();
                if missing_ancestor_only {
                    history[index as usize] = Some(template::Ancestor::new(
                        level,
                        current_value,
                        language,
                        is_rtl,
                    )?);
                } else {
                    blocked.insert(lsid);
                }
                if diagnostics.len() < 256 {
                    diagnostics.push(Diagnostic {
                        code: "list_ancestor_number_unresolved",
                        cp,
                    });
                }
                continue;
            }
        };
        // Counter state holds the next value; ancestor lookup retains the actual prior paragraph.
        // Ancestor identity is iLfo within this story, while counter identity is LSID.
        history[index as usize] = Some(template::Ancestor::new(
            level,
            current_value,
            language,
            is_rtl,
        )?);
        remaining_label_bytes = remaining_label_bytes
            .checked_sub(text.len())
            .ok_or(Error::Limit("list generated label bytes"))?;
        let inferred = literal_unicode_bullet(level, c.fonts)?;
        labels.push((lsid, ilfo, index, inferred, Label { cp, text }));
    }
    let mut label_sources = Vec::new();
    let labels: Vec<_> = labels
        .into_iter()
        .filter_map(|(lsid, ilfo, level, inferred, label)| {
            if blocked.contains(&lsid) {
                return None;
            }
            if inferred {
                // Each inferred literal keeps its own source anchor and qualification warning.
                // A full diagnostic budget leaves the label unresolved.
                if diagnostics.len() >= 256 {
                    return None;
                }
                diagnostics.push(Diagnostic {
                    code: "list_literal_unicode_bullet_inferred",
                    cp: label.cp,
                });
            }
            label_sources.push(LabelSource {
                start_cp: label.cp,
                list_id: lsid,
                override_index: ilfo,
                level,
            });
            Some(label)
        })
        .collect();
    for record in &mut paragraph_direction_evidence {
        if record.list_label_status == "logical_label_generated_visual_shaping_unresolved"
            && labels
                .binary_search_by_key(&record.start_cp, |label| label.cp)
                .is_err()
        {
            record.list_label_status = "unsupported_list_sequence";
        }
    }
    Ok(ListPlan {
        labels,
        label_sources,
        diagnostics,
        numbering_revision_evidence,
        property_revision_evidence,
        paragraph_direction_evidence,
        table_direction_evidence,
    })
}

fn style_selection(
    index: usize,
    styles: &Stylesheet,
    cache: &mut BTreeMap<usize, Selection>,
    depth: usize,
) -> Result<Selection> {
    if let Some(s) = cache.get(&index) {
        return Ok(s.clone());
    }
    if depth > 128 {
        return Err(Error::Limit("list style inheritance"));
    }
    let style = styles
        .styles
        .get(index)
        .and_then(Option::as_ref)
        .filter(|s| s.kind == 1 && !s.revision_marked && s.raw_revision.is_empty())
        .ok_or(Error::Unsupported("list paragraph style"))?;
    let mut result = if let Some(parent) = style.base_style {
        style_selection(parent, styles, cache, depth + 1)?
    } else {
        Selection {
            visible_style: true,
            rtl_visible_style: true,
            ..Default::default()
        }
    };
    for p in &style.properties {
        if p.kind == "paragraph" {
            if u16_at(&p.bytes, 0)? as usize != index {
                return Err(Error::Invalid("list style PAPX identity"));
            }
            reject_numbering_revision_style(&p.bytes[2..])?;
            apply_paragraph(&mut result, &p.bytes[2..])?;
        } else if p.kind == "character" {
            let modifiers = decode_character_modifiers(&p.bytes)?;
            result.visible_style &= modifiers.iter().all(visible_modifier);
            result.rtl_visible_style &= modifiers.iter().all(rtl::visible);
            result.language.apply(&modifiers)?;
        }
    }
    cache.insert(index, result.clone());
    Ok(result)
}

// UpxPapx/UpxTapx exclude revision annotations preserved across PIstd/TIstd.
fn reject_numbering_revision_style(data: &[u8]) -> Result<()> {
    let mut offset = 0;
    while offset < data.len() {
        let sprm = u16_at(data, offset)?;
        offset += 2;
        if matches!(sprm, 0x244b | 0x244c) {
            return Err(Error::Unsupported("nested table membership in style"));
        }
        if matches!(sprm, 0xc645 | 0x2443) {
            return Err(Error::Unsupported("numbering revision property in style"));
        }
        if matches!(sprm, 0xc66f | 0xd667) {
            return Err(Error::Unsupported("property revision in style"));
        }
        let len = crate::office::doc::paragraph_references::operand_length(sprm, &data[offset..])?;
        bytes(data, offset, len)?;
        offset += len;
    }
    Ok(())
}

fn revision_evidence_size(snapshot: Option<&NumberingRevision>) -> usize {
    std::mem::size_of::<NumberingRevisionEvidence>()
        + snapshot.map_or(0, |s| s.raw_numrm.len() + s.template_utf16.len() * 2)
}

fn retain_numbering_revision(
    records: &mut Vec<NumberingRevisionEvidence>,
    remaining_bytes: &mut usize,
    mark: usize,
    selection: &Selection,
) -> Result<()> {
    if selection.numbering_revision.is_none() && selection.number_inserted.is_none() {
        return Ok(());
    }
    let snapshot = selection.numbering_revision.as_deref();
    numbering_revision::validate_flags(snapshot, selection.number_inserted.unwrap_or(false))?;
    // Include fixed decoded fields and record overhead, as well as both bounded owned buffers.
    let size = revision_evidence_size(snapshot);
    if records.len() >= MAX_REVISION_RECORDS {
        return Err(Error::Limit("numbering revision evidence records"));
    }
    *remaining_bytes = remaining_bytes
        .checked_sub(size)
        .ok_or(Error::Limit("numbering revision evidence bytes"))?;
    records.push(NumberingRevisionEvidence::accepted_current(
        mark,
        snapshot.cloned(),
        selection.number_inserted.unwrap_or(false),
    )?);
    Ok(())
}

fn retain_direction(
    records: &mut Vec<ParagraphDirectionEvidence>,
    remaining_bytes: &mut usize,
    start: usize,
    mark: usize,
    selection: &Selection,
) -> Result<()> {
    if !selection.right_to_left {
        return Ok(());
    }
    if records.len() >= MAX_REVISION_RECORDS {
        return Err(Error::Limit("paragraph direction evidence records"));
    }
    *remaining_bytes = remaining_bytes
        .checked_sub(std::mem::size_of::<ParagraphDirectionEvidence>())
        .ok_or(Error::Limit("paragraph direction evidence bytes"))?;
    let list =
        (selection.ilfo != 0 && selection.level != 12).then_some((selection.ilfo, selection.level));
    records.push(ParagraphDirectionEvidence::active_rtl(start, mark, list));
    Ok(())
}

fn retain_property_revision(
    records: &mut Vec<PropertyRevisionEvidence>,
    remaining_bytes: &mut usize,
    mark: usize,
    selection: &Selection,
) -> Result<()> {
    if selection.paragraph_revision.is_none() && selection.table_revision.is_none() {
        return Ok(());
    }
    if selection.table_revision.is_some() && !selection.in_table {
        return Err(Error::Unsupported("table property revision outside table"));
    }
    if records.len() >= MAX_REVISION_RECORDS {
        return Err(Error::Limit("property revision evidence records"));
    }
    *remaining_bytes = remaining_bytes
        .checked_sub(std::mem::size_of::<PropertyRevisionEvidence>())
        .ok_or(Error::Limit("property revision evidence bytes"))?;
    records.push(PropertyRevisionEvidence::accepted_current(
        mark,
        selection.paragraph_revision.clone(),
        selection.table_revision.clone(),
    ));
    Ok(())
}

fn apply_paragraph(selection: &mut Selection, data: &[u8]) -> Result<()> {
    let mut offset = 0;
    while offset < data.len() {
        let sprm = u16_at(data, offset)?;
        offset += 2;
        let len = crate::office::doc::paragraph_references::operand_length(sprm, &data[offset..])?;
        let operand = bytes(data, offset, len)?;
        offset += len;
        match sprm {
            // PFBiDi is an ordered Boolean: false restores the default paragraph direction.
            0x2441 => {
                if operand[0] > 1 {
                    return Err(Error::Invalid("list paragraph direction Boolean"));
                }
                selection.right_to_left = operand[0] != 0;
                selection.direction_explicit = true;
            }
            // TFBiDi and TFBiDi90 are independent slots; either true makes the table RTL.
            0x560b | 0x5664 => {
                let value = u16_at(operand, 0)?;
                if value > 1 {
                    return Err(Error::Invalid("list table direction Boolean"));
                }
                if sprm == 0x560b {
                    selection.table_bidi = Some(value != 0);
                } else {
                    selection.table_bidi90 = Some(value != 0);
                }
                selection.table_options = true;
            }
            0xc66f => {
                selection.paragraph_revision = Some(property_revision::decode_operand(operand)?)
            }
            0xd667 => selection.table_revision = Some(property_revision::decode_operand(operand)?),
            0x2664 | 0x3668 => {
                return Err(Error::Unsupported("list property revision history wall"));
            }
            0xc645 => {
                selection.numbering_revision =
                    Some(Arc::new(numbering_revision::decode_numrm_operand(operand)?));
            }
            0x2443 => {
                selection.number_inserted =
                    Some(numbering_revision::decode_numbering_inserted(operand)?)
            }
            0x244b | 0x244c => {
                if operand[0] > 1 {
                    return Err(Error::Invalid("list nested table Boolean"));
                }
                if sprm == 0x244b {
                    selection.inner_cell = operand[0] != 0;
                } else {
                    selection.inner_row = operand[0] != 0;
                }
            }
            0x2416 | 0x2417 => {
                if operand[0] > 1 {
                    return Err(Error::Invalid("list table Boolean"));
                }
                if sprm == 0x2416 {
                    selection.in_table = operand[0] != 0;
                } else {
                    selection.table_row = operand[0] != 0;
                }
            }
            0x6649 => {
                selection.table_depth = crate::office::doc::u32_at(operand, 0)?;
                selection.table_depth_explicit = true;
            }
            0xd608 => {
                let (columns, horizontal, vertical) = table_columns(operand)?;
                selection.table_columns = Some(columns);
                selection.horizontal_merges = horizontal;
                selection.vertical_merges = vertical;
            }
            // MS-DOC TInsertOperand: insert physical cell definitions in property order.
            // The two bitsets retain merge membership without resolving visible ownership.
            0x7621 => {
                let at = operand[0];
                let count = operand[1];
                let columns = selection.table_columns.unwrap_or(0);
                let width = u16_at(operand, 2)? as i16;
                if count == 0
                    || at > columns
                    || u16::from(columns) + u16::from(count) > 63
                    || width < 0
                {
                    return Err(Error::Invalid("list table insertion"));
                }
                let low = (1u64 << at) - 1;
                for mask in [
                    &mut selection.horizontal_merges,
                    &mut selection.vertical_merges,
                ] {
                    *mask = (*mask & low) | ((*mask & !low) << count);
                }
                selection.table_columns = Some(columns + count);
            }
            // MS-DOC §2.9.343 counts physical cells even when vertically merged.
            // All list sequences with any table member remain blocked below.
            0xd62b => {
                if operand.len() != 3 || operand[0] != 2 || !matches!(operand[2], 0 | 1 | 3) {
                    return Err(Error::Invalid("list vertical merge operand"));
                }
                let index = operand[1];
                if selection
                    .table_columns
                    .is_none_or(|columns| index >= columns)
                {
                    return Err(Error::Invalid("list vertical merge cell"));
                }
                let bit = 1u64 << index;
                selection.vertical_merges &= !bit;
                if operand[2] != 0 {
                    selection.vertical_merges |= bit;
                }
            }
            // MS-DOC §2.9.19: one COLORREF per row cell; only the border appearance changes.
            // Check the resolved row cardinality after style and direct properties are applied.
            0xd61a..=0xd61d => {
                let count = operand[0];
                if count == 0 || count % 4 != 0 || usize::from(count) + 1 != operand.len() {
                    return Err(Error::Invalid("list border color operand"));
                }
                selection.border_color_columns[usize::from(sprm - 0xd61a)] = Some(count / 4);
            }
            // MS-DOC SHDOperand contains two COLORREFs and an Ipat shading pattern.
            // Shading preserves source membership; visible background appearance is unresolved.
            0xd660 => {
                if operand.len() != 11
                    || operand[0] != 10
                    || !matches!(operand[4], 0 | 0xff)
                    || !matches!(operand[8], 0 | 0xff)
                    || !matches!(u16_at(operand, 9)?, 0..=0x19 | 0x23..=0x3d | 0xffff)
                {
                    return Err(Error::Invalid("list table shading operand"));
                }
                selection.table_appearance = true;
            }
            // MS-DOC TCellSpacingDefault uses a CSSA for all four sides of each cell.
            // Its width is nonnegative twips, bounded to eleven inches, for every allowed unit.
            0xd633 => {
                if operand.len() != 7
                    || operand[..4] != [6, 0, 1, 0x0f]
                    || !matches!(operand[4], 0 | 3 | 0x13)
                    || u16_at(operand, 5)? > 15_840
                    || (operand[4] == 0 && u16_at(operand, 5)? != 0)
                {
                    return Err(Error::Invalid("list table spacing operand"));
                }
                selection.table_appearance = true;
            }
            0x563a => {
                selection.table_style = Some(u16_at(operand, 0)?);
            }
            // TLP selects optional style formatting. The table-style guard resolves its effects.
            // Its autoformat index records a past choice; current formatting lives in row properties.
            0x740a => selection.table_options = true,
            // MS-DOC §2.9.208: both two-bit anchor codes accept 0..=3; padding is ignored.
            // Placement affects page geometry, while source list membership stays unchanged.
            0x360d => selection.table_placement = true,
            // XAS/YAS_plusOne store signed twips plus one. Absolute page bounds need layout.
            0x940e | 0x940f => {
                let value = u16_at(operand, 0)? as i16;
                if !(-31_679..=31_681).contains(&value) {
                    return Err(Error::Invalid("list table position range"));
                }
                selection.table_placement = true;
            }
            // XAS/YAS_nonNeg bound the four wrapping distances to 22 inches in twips.
            0x9410 | 0x9411 | 0x941e | 0x941f => {
                if u16_at(operand, 0)? > 31_680 {
                    return Err(Error::Invalid("list table wrapping distance"));
                }
                selection.table_placement = true;
            }
            0x9602 => {
                if u16_at(operand, 0)? > i16::MAX as u16 {
                    return Err(Error::Invalid("list table gap"));
                }
            }
            0x5400 | 0x548a => {
                if u16_at(operand, 0)? > 2 {
                    return Err(Error::Invalid("list table justification"));
                }
            }
            // MS-DOC §2.6.3: row pagination and automatic width adjustment preserve list membership.
            0x3403 | 0x3404 | 0x3615 | 0x3466 => {
                if operand[0] > 1 {
                    return Err(Error::Invalid("list table layout Boolean"));
                }
            }
            // Borders, shading, alignment, wrapping and cell margins do not select paragraph lists.
            // TRsid records the save session; revision-state operators remain unsupported.
            0xd605 | 0xd609 | 0xd612 | 0xd613 | 0xf618 | 0xd62c | 0xd62f | 0xd632 | 0xd639
            | 0xd642 | 0xd670 | 0x7479 => {}
            // These table operators affect cells, widths and padding without selecting paragraph lists.
            0x9601 | 0xd635 | 0x7623 | 0x9407 | 0xf614 | 0xf617 | 0xd634 | 0xf661 => {}
            0x460b => {
                let raw = u16_at(operand, 0)?;
                selection.ilfo = match raw {
                    0 | 0xf801 => 0,
                    1..=0x7fe => raw as usize,
                    0xf802..=0xffff => (-(raw as i16)) as usize,
                    _ => return Err(Error::Invalid("list paragraph ilfo")),
                };
            }
            0x260a => {
                if operand[0] > 8 && operand[0] != 12 {
                    return Err(Error::Invalid("list paragraph level"));
                }
                selection.level = operand[0];
            }
            // MS-DOC §2.6.2: these Bool8 properties control line breaking, grids and spacing.
            0x242a | 0x2431 | 0x2433 | 0x2434 | 0x2435 | 0x2436 | 0x2447 | 0x2462 | 0x246d => {
                if operand[0] > 1 {
                    return Err(Error::Invalid("list paragraph layout Boolean"));
                }
            }
            // Frame geometry, drop caps, shading and modern borders preserve source list membership.
            0x8418 | 0x8419 | 0x841a | 0x261b | 0x2423 | 0x442b | 0x442c | 0x442d | 0x842e
            | 0x842f | 0xc64d | 0xc64e | 0xc64f | 0xc650 | 0xc651 | 0xc652 | 0xc653 | 0x6629
            | 0xc66c => {}
            // Character-unit indents, line-unit spacing and vertical alignment only affect layout.
            0x4455 | 0x4456 | 0x4457 | 0x4458 | 0x4459 | 0x4439 => {}
            // Pagination, spacing, indentation, borders and source IDs leave list selection intact.
            0x2403 | 0x2405 | 0x2406 | 0x2407 | 0x240c | 0xc60d | 0x840e | 0x840f | 0x4610
            | 0x8411 | 0x6412 | 0xa413 | 0xa414 | 0xc615 | 0x6424 | 0x6425 | 0x6426 | 0x6427
            | 0x6428 | 0x6429 | 0x2437 | 0x2438 | 0x2448 | 0x245b | 0x245c | 0x845d | 0x845e
            | 0x8460 | 0x2461 | 0x6467 | 0x2640 => {}
            _ => return Err(Error::Unsupported("list paragraph modifier")),
        }
    }
    Ok(())
}

/// Validate a nonempty TDefTable row and retain its physical merge membership.
/// Visible ownership across merged cells is explicitly unresolved by list projection.
fn table_columns(data: &[u8]) -> Result<(u8, u64, u64)> {
    let definition = crate::office::doc::table_structure::decode_table_definition(data)?;
    if definition.cells.is_empty() {
        return Err(Error::Invalid("list legacy table columns"));
    }
    let mut horizontal = 0;
    let mut vertical = 0;
    for (index, cell) in definition.cells.iter().enumerate() {
        if cell.horizontal_merge != 0 {
            horizontal |= 1u64 << index;
        }
        if cell.vertical_merge != 0 {
            vertical |= 1u64 << index;
        }
    }
    Ok((definition.cells.len() as u8, horizontal, vertical))
}

fn validate_default_table_style(styles: &Stylesheet) -> Result<(Option<bool>, Option<bool>)> {
    let style = styles
        .styles
        .get(11)
        .and_then(Option::as_ref)
        .filter(|s| {
            s.kind == 3
                && s.builtin_id == 105
                && s.base_style.is_none()
                && !s.revision_marked
                && s.raw_revision.is_empty()
        })
        .ok_or(Error::Unsupported("list default table style"))?;
    let mut selection = Selection::default();
    for p in &style.properties {
        match p.kind {
            "paragraph" => {
                if u16_at(&p.bytes, 0)? != 11 {
                    return Err(Error::Invalid("list table style identity"));
                }
                reject_numbering_revision_style(&p.bytes[2..])?;
                apply_paragraph(&mut selection, &p.bytes[2..])?;
            }
            "table" => {
                reject_numbering_revision_style(&p.bytes)?;
                apply_paragraph(&mut selection, &p.bytes)?;
            }
            "character"
                if decode_character_modifiers(&p.bytes)?
                    .iter()
                    .all(visible_modifier) => {}
            _ => return Err(Error::Unsupported("list table style properties")),
        }
    }
    validate_direction(&selection)?;
    if selection.ilfo != 0 || selection.level != 0 || selection.table_style.is_some_and(|i| i != 11)
    {
        return Err(Error::Unsupported("list default table style numbering"));
    }
    Ok((selection.table_bidi, selection.table_bidi90))
}

// Current support covers resolved non-RTL properties; active RTL label layout stays unresolved.
fn validate_direction(selection: &Selection) -> Result<()> {
    if selection.right_to_left {
        return Err(Error::Unsupported("list active right-to-left paragraph"));
    }
    Ok(())
}

fn visible_modifier(p: &PropertyModifier) -> bool {
    // Highlight selects background color without changing logical label characters.
    // MS-DOC §§2.6.1 and 2.9.119 require one palette index below 0x11.
    if p.sprm == 0x2a0c {
        return matches!(p.operand.as_slice(), [0..=0x10]);
    }
    // Complex-script weight and slant preserve logical label characters.
    // Validate their toggle grammar without selecting a font or resolving appearance.
    if matches!(p.sprm, 0x085c | 0x085d) {
        return matches!(p.toggle(), Ok(Some(_)));
    }
    // Proofing exclusion controls spelling and grammar analysis without changing list text.
    if p.sprm == 0x0875 {
        return matches!(p.toggle(), Ok(Some(_)));
    }
    // Emboss changes glyph appearance while preserving logical list characters.
    if p.sprm == 0x0858 {
        return matches!(p.toggle(), Ok(Some(_)));
    }
    // Document-grid participation affects display spacing; logical label text is retained.
    if p.sprm == 0x0868 {
        return matches!(p.toggle(), Ok(Some(_)));
    }
    // MS-DOC §§2.6.1, 2.9.43: sprmCCv changes RGB color and preserves text membership.
    // COLORREF has four bytes and an automatic-color byte restricted to 0 or 0xFF.
    if p.sprm == 0x6870 {
        return p.operand.len() == 4 && matches!(p.operand[3], 0 | 0xff);
    }
    matches!(
        p.sprm,
        0x0835
            | 0x0836
            | 0x0837
            | 0x0838
            | 0x0839
            | 0x083a
            | 0x083b
            | 0x2a3e
            | 0x8840
            | 0x2a42
            | 0x4a43
            | 0x4845
            | 0x2a48
            | 0x484b
            | 0x4a4f
            | 0x4a50
            | 0x4a51
            | 0x4a5e
            | 0x4852
            | 0x2a53
            | 0x0854
            | 0x485f
            | 0x4a61
            | 0x486d
            | 0x486e
            | 0x4873
            | 0x4874
            | 0x6815
            | 0x6816
            | 0x6817
            | 0x286f
    )
}
/// Resolve picture-bullet state within one ordered mark or list-level property layer.
/// PbiGrfOperand bit 0 enables a picture; its size bit and 14 unused bits do not.
/// Style groups retain their separate prohibition on these paragraph-mark properties.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/a01919d1-3601-4aec-8adb-ea40bed1d77d>
fn visible_list_modifiers(properties: &[PropertyModifier]) -> bool {
    let mut picture = false;
    for property in properties {
        match property.sprm {
            0x6887 => {
                let Ok(bytes) = <[u8; 4]>::try_from(property.operand.as_slice()) else {
                    return false;
                };
                if i32::from_le_bytes(bytes) < 0 {
                    return false;
                }
            }
            0x4888 => {
                let Ok(bytes) = <[u8; 2]>::try_from(property.operand.as_slice()) else {
                    return false;
                };
                picture = u16::from_le_bytes(bytes) & 1 != 0;
            }
            _ if !visible_modifier(property) => return false,
            _ => {}
        }
    }
    !picture
}

fn visible_mark(formatting: &FormattingInspection, cp: usize) -> bool {
    let i = formatting
        .character_spans
        .partition_point(|s| s.end_cp <= cp);
    formatting
        .character_spans
        .get(i)
        .is_some_and(|s| s.start_cp <= cp && visible_list_modifiers(&s.properties))
}

#[cfg(test)]
fn label_text(
    level: &ListLevel,
    value: i32,
    fonts: &FontTable,
    index: u8,
    language: LabelLanguage,
) -> Result<String> {
    label_text_with_ancestors(level, value, fonts, index, language, None, |_| {
        Err(Error::Unsupported("list absent ancestor paragraph"))
    })
}

fn label_text_with_ancestors(
    level: &ListLevel,
    value: i32,
    fonts: &FontTable,
    index: u8,
    mut language: LabelLanguage,
    inherited_font: Option<inherited_bullet::Agreement>,
    ancestor: impl FnMut(u8) -> Result<String>,
) -> Result<String> {
    if level.number_text_utf16.len() > 512 {
        return Err(Error::Unsupported("list long number template"));
    }
    if value < 0 {
        return Err(Error::Invalid("list negative sequence value"));
    }
    let modifiers = decode_character_modifiers(&level.raw_chpx)?;
    // Unicode numeric formats preserve scalar identity under the East Asian font hint.
    // Symbol bullets still require their separately qualified font-selection path.
    if modifiers.iter().any(|p| {
        p.sprm == 0x286f
            && p.operand != [0]
            && !(matches!(level.number_format, 11 | 14 | 18 | 21 | 37 | 54) && p.operand == [1])
    }) {
        return Err(Error::Unsupported("list bullet font selection"));
    }
    if !visible_list_modifiers(&modifiers) {
        return Err(Error::Unsupported("list label character properties"));
    }
    let mut selection = Selection::default();
    apply_paragraph(&mut selection, &level.raw_papx)?;
    validate_direction(&selection)?;
    if selection.ilfo != 0 || selection.level != 0 {
        return Err(Error::Unsupported("list level paragraph properties"));
    }
    if level.flags & 4 != 0 && !matches!(level.number_format, 0 | 0x16 | 0x17) {
        return Err(Error::Unsupported("list legal number formatting"));
    }
    language.apply(&modifiers)?;
    let mut text = match level.number_format {
        0..=4 | 11 | 14 | 18 | 21 | 37 | 54 | 0x16 => {
            template::render(level, index, value, language, ancestor)?
        }
        0x17 => {
            if level.number_text_utf16.len() != 1 {
                return Err(Error::Invalid("list bullet text"));
            }
            let code = level.number_text_utf16[0];
            let displayed_code = if code & 0xf000 != 0 {
                code & 0x0fff
            } else {
                code
            };
            let font_sprm = if (0x20..=0x7f).contains(&displayed_code) {
                0x4a4f
            } else {
                0x4a51
            };
            let index = modifiers
                .iter()
                .rev()
                .find(|p| p.sprm == font_sprm)
                .map(|p| u16_at(&p.operand, 0))
                .transpose()?
                .or_else(|| inherited_font.map(|proof| proof.font()));
            if let Some(index) = index {
                let font = fonts
                    .fonts
                    .get(index as usize)
                    .filter(|f| f.index == index as usize)
                    .ok_or(Error::Invalid("list bullet font"))?;
                if font.charset == 2 {
                    map_symbol(font, code)
                        .0
                        .ok_or(Error::Unsupported("list symbol bullet"))?
                        .to_string()
                } else if code & 0xf000 == 0 || literal_unicode_bullet(level, fonts)? {
                    char::from_u32(code as u32)
                        .ok_or(Error::Invalid("list bullet scalar"))?
                        .to_string()
                } else {
                    return Err(Error::Unsupported("list bullet code page"));
                }
            } else {
                return Err(Error::Unsupported("list bullet font unavailable"));
            }
        }
        _ => return Err(Error::Unsupported("list number format")),
    };
    match level.following_character {
        0 => text.push('\t'),
        1 => text.push(' '),
        2 => {}
        _ => return Err(Error::Invalid("list label follow character")),
    }
    Ok(text)
}

/// Preserve two source-authored Unicode bullet scalars with explicit ordinary-font agreement.
/// MS-DOC 2.4.6.3 masks high bits; local application evidence retains these literal scalars.
/// Callers annotate this interpretation and retain raw LVL/font records for inspection.
fn literal_unicode_bullet(level: &ListLevel, fonts: &FontTable) -> Result<bool> {
    if level.number_format != 0x17
        || !matches!(level.number_text_utf16.as_slice(), [0x2022] | [0x25cf])
    {
        return Ok(false);
    }
    let modifiers = decode_character_modifiers(&level.raw_chpx)?;
    let selector = |sprm| {
        modifiers
            .iter()
            .rev()
            .find(|p| p.sprm == sprm)
            .map(|p| u16_at(&p.operand, 0))
            .transpose()
    };
    let (Some(ascii), Some(other)) = (selector(0x4a4f)?, selector(0x4a51)?) else {
        return Ok(false);
    };
    Ok(ascii == other
        && fonts
            .fonts
            .get(other as usize)
            .is_some_and(|font| font.index == other as usize && font.charset == 0))
}

impl LabelLanguage {
    fn apply(&mut self, modifiers: &[PropertyModifier]) -> Result<()> {
        for modifier in modifiers {
            match modifier.sprm {
                0x486d | 0x4873 => self.latin = Some(u16_at(&modifier.operand, 0)?),
                0x286f => self.hint = bytes(&modifier.operand, 0, 1)?[0],
                _ => {}
            }
        }
        Ok(())
    }
}

// MS-LCID §2.2 lists these assigned English language IDs; reserved IDs stay unresolved.
fn english_language(lid: u16) -> bool {
    matches!(
        lid,
        0x0009
            | 0x0409
            | 0x0809
            | 0x0c09
            | 0x1009
            | 0x1409
            | 0x1809
            | 0x1c09
            | 0x2009
            | 0x2409
            | 0x2809
            | 0x2c09
            | 0x3009
            | 0x3409
            | 0x3c09
            | 0x4009
            | 0x4409
            | 0x4809
            | 0x4c09
    )
}

/// ISO 29500-1 §17.18.59 repeats the same alphabet character after each full cycle.
/// The caller resolves an English label language; other alphabets remain explicit gaps.
fn alphabetic(value: i32, uppercase: bool) -> Result<String> {
    if !(1..=26 * 512).contains(&value) {
        return Err(Error::Unsupported("list alphabetic range"));
    }
    let zero = (value - 1) as usize;
    let letter = (if uppercase { b'A' } else { b'a' }) + (zero % 26) as u8;
    Ok(char::from(letter).to_string().repeat(zero / 26 + 1))
}

fn roman(mut value: i32) -> Result<String> {
    if !(1..=3999).contains(&value) {
        return Err(Error::Unsupported("list Roman range"));
    }
    let mut text = String::new();
    for (amount, symbol) in [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ] {
        while value >= amount {
            text.push_str(symbol);
            value -= amount;
        }
    }
    Ok(text)
}

fn validate(story: &TranscribedStory, start: usize, units: usize) -> Result<()> {
    if story.segments.len() > MAX_ITEMS || story.text.len() > MAX_UNITS * 4 {
        return Err(Error::Limit("list source segments"));
    }
    let mut cp = start;
    let mut output = 0;
    for s in &story.segments {
        if s.start_cp != cp
            || s.end_cp <= cp
            || s.output_start != output
            || s.output_end < output
            || story.text.get(output..s.output_end).is_none()
        {
            return Err(Error::Invalid("list input partition"));
        }
        cp = s.end_cp;
        output = s.output_end;
    }
    if cp.checked_sub(start) != Some(units) || output != story.text.len() {
        return Err(Error::Invalid("list input coverage"));
    }
    Ok(())
}
fn copy_story(s: &TranscribedStory) -> TranscribedStory {
    TranscribedStory {
        kind: s.kind,
        text: s.text.clone(),
        segments: s
            .segments
            .iter()
            .map(|s| SourceSegment {
                kind: s.kind,
                start_cp: s.start_cp,
                end_cp: s.end_cp,
                output_start: s.output_start,
                output_end: s.output_end,
            })
            .collect(),
        diagnostics: s
            .diagnostics
            .iter()
            .map(|d| Diagnostic {
                code: d.code,
                cp: d.cp,
            })
            .collect(),
        fields_balanced: s.fields_balanced,
        unresolved_control_units: s.unresolved_control_units,
    }
}
fn insert(story: &TranscribedStory, labels: &[Label]) -> Result<TranscribedStory> {
    let mut out = copy_story(story);
    out.text.clear();
    out.segments.clear();
    let mut next = 0;
    for s in &story.segments {
        let mut cp = s.start_cp;
        let mut byte = s.output_start;
        while let Some(label) = labels.get(next).filter(|l| l.cp < s.end_cp) {
            if label.cp < cp {
                return Err(Error::Invalid("list label anchor order"));
            }
            let split = if label.cp == cp {
                byte
            } else {
                // Splits within source-derived text count UTF-16 units, not UTF-8 bytes.
                let mut cursor = cp;
                let mut offset = byte;
                for ch in story.text[byte..s.output_end].chars() {
                    if cursor == label.cp {
                        break;
                    }
                    cursor += ch.len_utf16();
                    offset += ch.len_utf8();
                }
                if cursor != label.cp {
                    return Err(Error::Unsupported(
                        "list anchor within generated or suppressed segment",
                    ));
                }
                offset
            };
            if cp < label.cp {
                append(&mut out, s.kind, cp, label.cp, &story.text[byte..split])?;
            }
            append(
                &mut out,
                "generated_list_label",
                label.cp,
                label.cp,
                &label.text,
            )?;
            cp = label.cp;
            byte = split;
            next += 1;
        }
        if cp < s.end_cp {
            append(
                &mut out,
                s.kind,
                cp,
                s.end_cp,
                &story.text[byte..s.output_end],
            )?;
        }
    }
    if next != labels.len() {
        return Err(Error::Invalid("list unused anchors"));
    }
    Ok(out)
}
fn append(
    out: &mut TranscribedStory,
    kind: &'static str,
    start_cp: usize,
    end_cp: usize,
    text: &str,
) -> Result<()> {
    if out.segments.len() == MAX_ITEMS {
        return Err(Error::Limit("list output segments"));
    }
    let output_start = out.text.len();
    out.text.push_str(text);
    out.segments.push(SourceSegment {
        kind,
        start_cp,
        end_cp,
        output_start,
        output_end: out.text.len(),
    });
    Ok(())
}

fn rtl_label_text(level: &ListLevel, value: i32, index: u8) -> Result<String> {
    if !matches!(level.number_format, 0 | 46) || level.number_text_utf16.len() > 512 {
        return Err(Error::Unsupported("list RTL number format"));
    }
    let mut selection = Selection::default();
    apply_paragraph(&mut selection, &level.raw_papx)?;
    if selection.ilfo != 0
        || selection.level != 0
        || selection.in_table
        || selection.table_depth != 0
        || selection.table_row
        || selection.inner_cell
        || selection.inner_row
        || selection.table_style.is_some()
        || selection.table_options
        || selection.table_columns.is_some()
    {
        return Err(Error::Unsupported("list RTL level paragraph properties"));
    }
    if selection.direction_explicit && !selection.right_to_left {
        return Err(Error::Unsupported("list RTL level direction override"));
    }
    if level.flags & 4 != 0 && level.number_format != 0 {
        return Err(Error::Unsupported("list RTL legal number formatting"));
    }
    let offset = level.placeholder_offsets[0] as usize;
    if offset == 0
        || level.placeholder_offsets[1] != 0
        || level.number_text_utf16.get(offset - 1) != Some(&(index as u16))
    {
        return Err(Error::Unsupported("list RTL current-level template"));
    }
    let prefix = String::from_utf16(&level.number_text_utf16[..offset - 1])
        .map_err(|_| Error::Invalid("list RTL template Unicode"))?;
    let suffix = String::from_utf16(&level.number_text_utf16[offset..])
        .map_err(|_| Error::Invalid("list RTL template Unicode"))?;
    if prefix.chars().chain(suffix.chars()).any(char::is_control) || value < 0 {
        return Err(Error::Unsupported("list RTL template literal"));
    }
    let number = if level.number_format == 46 {
        rtl::arabic_alpha(value)?
    } else {
        value.to_string()
    };
    let mut text = format!("{prefix}{number}{suffix}");
    match level.following_character {
        0 => text.push('\t'),
        1 => text.push(' '),
        2 => {}
        _ => return Err(Error::Invalid("list RTL follow character")),
    }
    Ok(text)
}

#[cfg(test)]
mod tests;
