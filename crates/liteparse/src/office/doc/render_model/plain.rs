//! Complete source qualification for the first Western body-text rendering path.
//! Rejected documents retain their ordinary inspection path; this API does not
//! flatten unsupported stories, fields, drawings, tables or revision topology.
use super::{paragraphs, runs, sections, settings};
use crate::office::doc::{Error, Result};
use serde::Serialize;
pub mod cached_fields;
pub mod columns;
pub mod comments;
pub mod drawings;
pub mod headers;
pub mod inline_pictures;
pub mod line_numbers;
pub mod lists;
pub mod multi_sections;
pub mod notes;
pub mod section_options;
mod symbols;
pub mod textboxes;

/// Validate grid dimensions independently of separately retained section metadata.
pub fn qualified_line_grid_dimensions(section: &sections::SectionGeometry) -> bool {
    match section.grid_mode.value {
        0 => true,
        2 => section
            .grid_line_pitch_twips
            .is_some_and(|v| (1..=31680).contains(&v.value)),
        _ => false,
    }
}

/// Lines-only grids use explicit source pitch; other geometry issues stay unresolved.
pub fn qualified_line_grid(section: &sections::SectionGeometry) -> bool {
    match section.grid_mode.value {
        0 => section.issues.is_empty(),
        2 => {
            section
                .grid_line_pitch_twips
                .is_some_and(|v| (1..=31680).contains(&v.value))
                && section
                    .issues
                    .iter()
                    .all(|i| i.reason == "active_document_grid_layout_unresolved")
        }
        _ => false,
    }
}

// Prefix maximum ends keep overlapping source ranges safe without rescanning
// every table for every object. Callers bound the range count before allocation.
fn qualify_grid_cell_objects(
    mut ranges: Vec<[usize; 2]>,
    paragraphs: &[PlainParagraph],
) -> Result<()> {
    ranges.sort_unstable();
    let mut end = 0;
    for range in &mut ranges {
        end = end.max(range[1]);
        range[1] = end;
    }
    for paragraph in paragraphs {
        for run in &paragraph.runs {
            let mut cp = run.source_cp[0];
            for c in run.text.chars() {
                if matches!(c, '\u{1}' | '\u{8}') {
                    let index = ranges.partition_point(|r| r[0] <= cp);
                    if index > 0 && cp < ranges[index - 1][1] {
                        return Err(Error::Unsupported("render object-cell line grid"));
                    }
                }
                cp = cp
                    .checked_add(c.len_utf16())
                    .ok_or(Error::Limit("render grid source CP"))?;
            }
        }
    }
    Ok(())
}

// Copts fDontAdjustLineHeightInTable affects document-grid snapping only.
// The source bit stays in settings; a disabled grid has no snapping adjustment.
fn unsupported_table_compatibility(name: &str, grid_mode: u16) -> bool {
    (name == "do_not_adjust_line_height_in_table" && !matches!(grid_mode, 0 | 2))
        || matches!(
            name,
            "word2002_table_style_rules"
                | "grow_autofit"
                | "same_style_spacing_in_table"
                | "do_not_autofit_constrained_tables"
                | "autofit_like_word11"
                | "do_not_vertically_align_cell_with_shapes"
                | "do_not_break_constrained_forced_tables"
                | "word11_kerning_pairs"
        )
}

// These punctuation characters use the non-ASCII source font under the admitted
// default hint. En-space retains its selected font advance. Other special spaces,
// directional controls and discretionary breaks retain their layout guards.
// Raw source repertoire is checked again against the resolved complex-font run.
// Generated field values use the narrower context-free character predicate.
fn supported_source_character(c: char) -> bool {
    supported_character(c as u32) || super::paragraphs::direction::arabic(c)
}

fn supported_character(c: u32) -> bool {
    super::runs::east_asian_character(c)
        || matches!(c, 0x0384 | 0x0386 | 0x0388..=0x038a | 0x038c | 0x038e..=0x03a1
            | 0x03a3..=0x03ce | 0x0400..=0x045f)
        || c != 0xad
            && matches!(c, 9 | 11 | 12 | 13 | 14 | 31 | 0x20..=0x7e | 0xa0..=0x24f
            | 0x201a | 0x201e | 0x2020 | 0x2021 | 0x2030 | 0x2039 | 0x203a | 0x203b | 0x20ac
            | 0x2122 | 0x221a | 0x2264 | 0x2265 | 0x2500 | 0x25a0 | 0x25a1 | 0x25cb | 0x25cf
            | 0x2002 | 0x2013 | 0x2014 | 0x2018 | 0x2019 | 0x201c | 0x201d | 0x2022 | 0x2026)
}

#[derive(Debug, Serialize)]
pub struct PlainDocument {
    pub source_utf16_units: usize,
    pub settings: settings::DocumentSettings,
    pub section: sections::SectionGeometry,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub following_sections: Vec<multi_sections::FollowingSection>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unmodeled_section_options: Vec<section_options::SectionOption>,
    pub paragraphs: Vec<PlainParagraph>,
    pub tables: Vec<super::tables::source::SourceTable>,
    pub font_names: Vec<String>,
    pub font_charsets: Vec<u8>,
    pub headers: Option<headers::HeaderDocument>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footnotes: Option<notes::FootnoteDocument>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endnotes: Option<notes::FootnoteDocument>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments: Option<comments::CommentDocument>,
    pub list_labels: Vec<lists::PaintLabel>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub textbox_storage: Vec<textboxes::TextboxStorage>,
    pub drawing_proof: Option<drawings::DrawingProof>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub image_assets: Vec<crate::office::doc::pictures::raster_store::StoreImage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline_pictures: Option<inline_pictures::InlinePictures>,
}
#[derive(Debug, Clone, Serialize)]
pub struct PlainParagraph {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cached_fields: Vec<cached_fields::CachedHyperlink>,
    pub source_cp: [usize; 2],
    pub source_style: usize,
    pub properties: paragraphs::ParagraphProperties,
    /// Runs include the final CR. Rendering consumes it as a paragraph mark.
    pub runs: Vec<PlainRun>,
}
#[derive(Debug, Clone, Serialize)]
pub struct PlainRun {
    pub source_cp: [usize; 2],
    pub text: String,
    pub properties: runs::RunProperties,
}

impl PlainDocument {
    pub fn body_end_cp(&self) -> usize {
        self.following_sections
            .last()
            .map_or(self.section.source_cp[1], |s| s.geometry.source_cp[1])
    }

    pub fn active_notes(&self) -> Option<&notes::FootnoteDocument> {
        self.footnotes.as_ref().or(self.endnotes.as_ref()).or(self
            .comments
            .as_ref()
            .filter(|c| !c.plan.notes.is_empty())
            .map(|c| &c.plan))
    }

    /// Validate a caller-supplied model before shared layout indexes fonts or ranges.
    /// Source provenance itself is established by resolve(); this checks its invariants.
    /// Actual font availability and ANSI kerning equivalence require renderer qualification.
    pub fn validate(&self) -> Result<()> {
        textboxes::validate_all(&self.textbox_storage)?;
        multi_sections::validate(self)?;
        if self.source_utf16_units > 1024 * 1024
            || self.paragraphs.len() > 16384
            || self.font_names.len() > 32768
            || self.font_names.len() != self.font_charsets.len()
            || self
                .font_names
                .iter()
                .any(|s| s.is_empty() || s.len() > 1024)
        {
            return Err(Error::Limit("render model size"));
        }
        let mut evidence_work = 0usize;
        for p in self
            .paragraphs
            .iter()
            .chain(
                self.headers
                    .iter()
                    .flat_map(|h| &h.slots)
                    .flat_map(|s| &s.paragraphs),
            )
            .chain(
                self.textbox_storage
                    .iter()
                    .flat_map(|s| &s.textboxes)
                    .flat_map(|b| &b.paragraphs),
            )
            .chain(self.active_notes().into_iter().flat_map(|n| {
                n.notes
                    .iter()
                    .flat_map(|n| &n.paragraphs)
                    .chain(n.separators.iter().flat_map(|s| &s.paragraphs))
            }))
        {
            cached_fields::charge_evidence(&p.cached_fields, &mut evidence_work)?;
            for run in &p.runs {
                if (run.properties.bidi || run.properties.hint == 2)
                    && run.text.chars().any(|c| !matches!(c, ' ' | '\t' | '\r'))
                    && run
                        .properties
                        .complex
                        .font
                        .and_then(|i| self.font_charsets.get(i))
                        == Some(&2)
                {
                    return Err(Error::Unsupported("render complex symbol font context"));
                }
                for property in run
                    .properties
                    .applied
                    .iter()
                    .chain(&run.properties.complex_property_evidence)
                {
                    evidence_work = evidence_work
                        .checked_add(property.operand.len() + 3)
                        .filter(|n| *n <= 8 * 1024 * 1024)
                        .ok_or(Error::Limit("render model source property evidence"))?;
                }
            }
        }
        lists::validate(
            &self.list_labels,
            &self.paragraphs,
            &self.font_charsets,
            &mut evidence_work,
        )?;
        if self
            .headers
            .iter()
            .flat_map(|h| &h.slots)
            .flat_map(|s| &s.paragraphs)
            .any(|p| p.properties.tabs.iter().any(|t| t.alignment == 6))
        {
            return Err(Error::Unsupported("render secondary list tab"));
        }
        for p in &self.paragraphs {
            cached_fields::validate(p)?;
        }
        for p in self
            .headers
            .iter()
            .flat_map(|h| &h.slots)
            .flat_map(|s| &s.paragraphs)
        {
            cached_fields::validate(p)?;
        }
        if self
            .textbox_storage
            .iter()
            .flat_map(|s| &s.textboxes)
            .flat_map(|b| &b.paragraphs)
            .any(|p| !p.cached_fields.is_empty())
        {
            return Err(Error::Unsupported("render cached secondary field"));
        }
        qualify_settings(&self.settings)?;
        // Page controls retain source ownership only outside tables and list labels.
        for p in &self.paragraphs {
            if p.runs.iter().any(|r| r.text.contains(['\u{c}', '\u{e}']))
                && ((p.source_cp[1] >= self.body_end_cp() && self.endnotes.is_none())
                    || self.tables.iter().any(|t| {
                        p.source_cp[0] < t.source_cp[1] && t.source_cp[0] < p.source_cp[1]
                    })
                    || self
                        .list_labels
                        .iter()
                        .any(|l| l.source.anchor_cp == p.source_cp[0]))
            {
                return Err(Error::Unsupported("render page break context"));
            }
        }
        if self
            .headers
            .iter()
            .flat_map(|h| &h.slots)
            .flat_map(|s| &s.paragraphs)
            .flat_map(|p| &p.runs)
            .any(|r| r.text.contains(['\u{c}', '\u{e}']))
        {
            return Err(Error::Unsupported("render secondary page break"));
        }
        if self
            .active_notes()
            .iter()
            .flat_map(|n| &n.notes)
            .flat_map(|n| &n.paragraphs)
            .chain(
                self.textbox_storage
                    .iter()
                    .flat_map(|s| &s.textboxes)
                    .flat_map(|b| &b.paragraphs),
            )
            .flat_map(|p| &p.runs)
            .any(|r| r.text.contains('\u{e}'))
        {
            return Err(Error::Unsupported("render secondary column break"));
        }
        let g = &self.section;
        columns::width_twips(g)?;
        section_options::validate(g, &self.unmodeled_section_options)?;
        if g.source_cp[0] != 0
            || g.source_cp[1] > self.source_utf16_units
            || g.vertical_justification.value != 0
            || g.gutter_twips.value != 0
        {
            return Err(Error::Unsupported("render model section"));
        }
        let left = g
            .left_margin_twips
            .ok_or(Error::Invalid("render left margin"))?
            .value;
        let right = g
            .right_margin_twips
            .ok_or(Error::Invalid("render right margin"))?
            .value;
        let top = g
            .top_margin_twips
            .ok_or(Error::Invalid("render top margin"))?
            .value
            .unsigned_abs();
        let bottom = g
            .bottom_margin_twips
            .ok_or(Error::Invalid("render bottom margin"))?
            .value
            .unsigned_abs();
        if left as u32 + right as u32 >= g.page_width_twips.value as u32
            || top as u32 + bottom as u32 >= g.page_height_twips.value as u32
        {
            return Err(Error::Invalid("render model body dimensions"));
        }
        let table_marks = super::tables::source::validate_ledger(
            &self.tables,
            &self.paragraphs,
            self.settings.binary_version,
        )?;
        for paragraph in &self.paragraphs {
            if paragraph.runs.iter().any(|r| r.properties.hidden)
                && self
                    .tables
                    .get(
                        self.tables
                            .partition_point(|t| t.source_cp[1] <= paragraph.source_cp[0]),
                    )
                    .is_some_and(|t| t.source_cp[0] < paragraph.source_cp[1])
            {
                return Err(Error::Unsupported("render hidden table paragraph"));
            }
        }
        textboxes::validate_drawings(&self.textbox_storage, self.drawing_proof.as_ref())?;
        let mut shape_anchors = textboxes::anchor_cps(&self.textbox_storage)?;
        shape_anchors.extend(
            self.drawing_proof
                .iter()
                .flat_map(|d| &d.omitted_shapes)
                .filter(|s| s.source_rectangle_twips.is_some())
                .map(|s| s.group_anchor_cp),
        );
        shape_anchors.extend(
            self.drawing_proof
                .iter()
                .flat_map(|d| d.images())
                .filter(|s| s.anchor_story == "body")
                .map(|s| s.anchor_cp),
        );
        let note_markers: std::collections::BTreeSet<_> = self
            .active_notes()
            .into_iter()
            .flat_map(|n| n.marker_cps())
            .collect();
        for p in &self.paragraphs {
            cached_fields::validate_shape_anchors(&p.cached_fields, &shape_anchors)?;
        }
        let mut picture_cps = std::collections::BTreeSet::new();
        if let Some(pictures) = &self.inline_pictures {
            pictures.validate()?;
            let mut owners: Vec<_> = self
                .paragraphs
                .iter()
                .chain(
                    self.headers
                        .iter()
                        .flat_map(|h| &h.slots)
                        .flat_map(|slot| &slot.paragraphs),
                )
                .collect();
            owners.sort_by_key(|p| p.source_cp);
            if owners.iter().any(|p| p.source_cp[0] >= p.source_cp[1])
                || owners
                    .windows(2)
                    .any(|pair| pair[0].source_cp[1] > pair[1].source_cp[0])
            {
                return Err(Error::Invalid("ambiguous inline picture story ownership"));
            }
            for reference in &pictures.references {
                let cp = reference.source_cp;
                let paragraph = owners
                    .get(owners.partition_point(|p| p.source_cp[1] <= cp))
                    .filter(|p| p.source_cp[0] <= cp)
                    .ok_or(Error::Invalid("inline picture paragraph ownership"))?;
                let run = paragraph
                    .runs
                    .get(paragraph.runs.partition_point(|r| r.source_cp[1] <= cp))
                    .ok_or(Error::Invalid("inline picture run ownership"))?;
                if run.source_cp != [cp, cp + 1] || run.text != "\u{1}" {
                    return Err(Error::Invalid("inline picture source control"));
                }
                picture_cps.insert(cp);
            }
        }
        for paragraph in self.paragraphs.iter().chain(
            self.headers
                .iter()
                .flat_map(|h| h.slots.iter())
                .flat_map(|s| &s.paragraphs),
        ) {
            cached_fields::validate_picture_results(&paragraph.cached_fields, &picture_cps)?;
        }
        validate_partition_with_section_marks(
            &self.paragraphs,
            [0, self.body_end_cp()],
            self.font_names.len(),
            &table_marks,
            &shape_anchors,
            &note_markers,
            PartitionControls {
                pictures: &picture_cps,
                sections: &multi_sections::terminators(self).collect(),
            },
        )?;
        if self.footnotes.is_some() && self.endnotes.is_some() {
            return Err(Error::Unsupported(
                "render mixed footnote and endnote placement",
            ));
        }
        if self.section.grid_mode.value == 2
            && self
                .settings
                .compatibility
                .iter()
                .any(|p| p.enabled && p.name == "do_not_snap_to_grid_in_cell")
        {
            if self.tables.len() > self.paragraphs.len() {
                return Err(Error::Limit("render grid table ranges"));
            }
            qualify_grid_cell_objects(
                self.tables.iter().map(|t| t.source_cp).collect(),
                &self.paragraphs,
            )?;
        }
        let secondary_start = if let Some(n) = &self.footnotes {
            if n.kind != notes::NoteKind::Footnote {
                return Err(Error::Invalid("render footnote kind"));
            }
            n.validate(&self.paragraphs, self.font_names.len())?;
            n.validate_separators(self.headers.as_ref(), self.font_names.len())?;
            n.source_cp[1]
        } else {
            self.body_end_cp()
        };
        if let Some(headers) = &self.headers {
            headers.validate_activation(
                self.settings.different_even_and_odd_headers,
                g.different_first_page.value,
            )?;
            let image_anchors: std::collections::BTreeSet<_> = self
                .drawing_proof
                .iter()
                .flat_map(|d| d.images())
                .filter(|s| s.anchor_story == "headers")
                .map(|s| s.anchor_cp)
                .chain(
                    self.drawing_proof
                        .iter()
                        .flat_map(|d| {
                            d.lines()
                                .iter()
                                .chain(d.rectangles().iter().map(|r| &r.frame))
                        })
                        .map(|s| s.anchor_cp),
                )
                .chain(
                    self.textbox_storage
                        .iter()
                        .filter(|s| s.story == "header_textboxes")
                        .flat_map(|s| &s.textboxes)
                        .map(|b| b.anchor_cp),
                )
                .collect();
            if !picture_cps.is_empty() {
                headers.validate_after_notes_with_pictures(
                    self.body_end_cp(),
                    secondary_start,
                    headers.ownership.source.end_cp,
                    self.font_names.len(),
                    &image_anchors,
                    &picture_cps,
                )?;
            } else if self.footnotes.is_some() {
                headers.validate_after_notes(
                    self.body_end_cp(),
                    secondary_start,
                    headers.ownership.source.end_cp,
                    self.font_names.len(),
                    &image_anchors,
                )?;
            } else if image_anchors.is_empty() {
                headers.validate(
                    self.body_end_cp(),
                    headers.ownership.source.end_cp,
                    self.font_names.len(),
                )?;
            } else {
                headers.validate_with_images(
                    self.body_end_cp(),
                    headers.ownership.source.end_cp,
                    self.font_names.len(),
                    &image_anchors,
                )?;
            }
        }
        let mut covered = self
            .headers
            .as_ref()
            .map_or(secondary_start, |h| h.ownership.source.end_cp);
        if let Some(c) = &self.comments {
            if ((!c.plan.notes.is_empty()) && (self.endnotes.is_some() || self.footnotes.is_some()))
                || c.plan.kind != notes::NoteKind::Comment
                || c.plan.source_cp[0] != covered
            {
                return Err(Error::Invalid("render comment storage partition"));
            }
            c.validate_ownership()?;
            if c.plan.notes.is_empty() {
                if !c.ownership.comments.is_empty()
                    || c.plan.source_text != "\r"
                    || c.plan.source_cp[1].checked_sub(c.plan.source_cp[0]) != Some(1)
                    || c.plan.terminal_guard != c.plan.source_cp
                    || c.ownership
                        .terminal_source
                        .is_none_or(|t| [t.start_cp, t.end_cp] != c.plan.source_cp)
                {
                    return Err(Error::Invalid("empty comment storage sentinel"));
                }
            } else {
                c.plan.validate(&self.paragraphs, self.font_names.len())?;
            }
            covered = c.plan.source_cp[1];
        }
        if let Some(n) = &self.endnotes {
            if n.kind != notes::NoteKind::Endnote || n.source_cp[0] != covered {
                return Err(Error::Invalid("render endnote storage partition"));
            }
            n.validate(&self.paragraphs, self.font_names.len())?;
            if !n.separators.is_empty() {
                return Err(Error::Unsupported("render endnote separators"));
            }
            covered = n.source_cp[1];
        }
        for story in &self.textbox_storage {
            if story.source_cp[0] != covered {
                return Err(Error::Invalid("render secondary storage partition"));
            }
            covered = story.source_cp[1];
            for textbox in &story.textboxes {
                validate_partition(
                    &textbox.paragraphs,
                    textbox.source_cp,
                    self.font_names.len(),
                )?;
                if textbox
                    .paragraphs
                    .iter()
                    .flat_map(|p| &p.runs)
                    .map(|r| r.text.as_str())
                    .collect::<String>()
                    != textbox.source_text
                {
                    return Err(Error::Invalid("render textbox paragraph text"));
                }
            }
        }
        if covered != self.source_utf16_units {
            return Err(Error::Invalid("render unowned source story"));
        }
        Ok(())
    }
}

fn validate_partition(
    paragraphs: &[PlainParagraph],
    range: [usize; 2],
    font_count: usize,
) -> Result<()> {
    validate_partition_marks(paragraphs, range, font_count, &Default::default())
}

fn validate_partition_marks(
    paragraphs: &[PlainParagraph],
    range: [usize; 2],
    font_count: usize,
    table_marks: &std::collections::BTreeSet<usize>,
) -> Result<()> {
    validate_partition_controls(
        paragraphs,
        range,
        font_count,
        table_marks,
        &Default::default(),
        &Default::default(),
    )
}
fn validate_partition_controls(
    paragraphs: &[PlainParagraph],
    range: [usize; 2],
    font_count: usize,
    table_marks: &std::collections::BTreeSet<usize>,
    shape_anchors: &std::collections::BTreeSet<usize>,
    note_markers: &std::collections::BTreeSet<usize>,
) -> Result<()> {
    validate_partition_controls_with_pictures(
        paragraphs,
        range,
        font_count,
        table_marks,
        shape_anchors,
        note_markers,
        &Default::default(),
    )
}
fn validate_partition_controls_with_pictures(
    paragraphs: &[PlainParagraph],
    range: [usize; 2],
    font_count: usize,
    table_marks: &std::collections::BTreeSet<usize>,
    shape_anchors: &std::collections::BTreeSet<usize>,
    note_markers: &std::collections::BTreeSet<usize>,
    picture_cps: &std::collections::BTreeSet<usize>,
) -> Result<()> {
    validate_partition_with_section_marks(
        paragraphs,
        range,
        font_count,
        table_marks,
        shape_anchors,
        note_markers,
        PartitionControls {
            pictures: picture_cps,
            sections: &Default::default(),
        },
    )
}
struct PartitionControls<'a> {
    pictures: &'a std::collections::BTreeSet<usize>,
    sections: &'a std::collections::BTreeSet<usize>,
}
fn validate_partition_with_section_marks(
    paragraphs: &[PlainParagraph],
    range: [usize; 2],
    font_count: usize,
    table_marks: &std::collections::BTreeSet<usize>,
    shape_anchors: &std::collections::BTreeSet<usize>,
    note_markers: &std::collections::BTreeSet<usize>,
    controls: PartitionControls<'_>,
) -> Result<()> {
    let PartitionControls {
        pictures: picture_cps,
        sections: section_marks,
    } = controls;
    let [mut cp, units] = range;
    if cp >= units {
        return Err(Error::Invalid("render empty content range"));
    }
    let mut total_runs = 0usize;
    for paragraph in paragraphs {
        if paragraph.source_cp[0] != cp
            || paragraph.source_cp[1] <= cp
            || paragraph.source_cp[1] > units
            || paragraph.runs.is_empty()
            || paragraph.properties.justification > 3
            || !(15..=31680).contains(&paragraph.properties.line_multiple_240ths)
            || match paragraph.properties.line_spacing {
                Some(super::paragraphs::LineSpacing::ExactTwips(v)) => !(1..=31680).contains(&v),
                Some(super::paragraphs::LineSpacing::MinimumTwips(v)) => v > 31680,
                None => false,
            }
            || paragraph.properties.tabs.len() > 64
            || paragraph.properties.tabs.iter().any(|t| {
                !matches!(t.alignment, 0..=2 | 6) || !(-31680..=31680).contains(&t.position_twips)
            })
            || paragraph
                .properties
                .tabs
                .windows(2)
                .any(|v| v[0].position_twips >= v[1].position_twips)
        {
            return Err(Error::Invalid("render model paragraph ownership"));
        }
        for run in &paragraph.runs {
            if run.properties.hidden
                && (paragraph.runs.len() != 1
                    || run.text != "\r"
                    || run.source_cp != paragraph.source_cp
                    || table_marks.contains(&run.source_cp[0]))
            {
                return Err(Error::Unsupported("render hidden paragraph content"));
            }
            if run.properties.language == 0x0400
                && (run.text != "\r"
                    && !(run.text == "\u{8}" && shape_anchors.contains(&run.source_cp[0]))
                    && !(run.text == "\u{1}" && picture_cps.contains(&run.source_cp[0]))
                    && !(matches!(run.text.as_str(), "\u{2}" | "\u{5}")
                        && note_markers.contains(&run.source_cp[0]))
                    && !(run.text.chars().all(|c| c == '\u{2002}')
                        && cached_fields::placeholder_matches(
                            &paragraph.cached_fields,
                            run.source_cp[0],
                            run.source_cp[1],
                        ))
                    && !(run.text.chars().all(|c| c.is_ascii_digit())
                        && paragraph.cached_fields.iter().any(|field| {
                            let range = field.result_cp();
                            range[0] <= run.source_cp[0] && run.source_cp[1] <= range[1]
                        }))
                    || run.properties.bidi
                    || run.properties.hint != 0)
            {
                return Err(Error::Unsupported("render no-language source run"));
            }
            if run.properties.bidi || run.properties.hint == 2 {
                if !run
                    .text
                    .chars()
                    .all(super::paragraphs::direction::complex_character)
                    || run.properties.complex.font.is_none_or(|f| f >= font_count)
                    || run
                        .properties
                        .complex
                        .size_half_points
                        .is_none_or(|v| !(2..=3276).contains(&v))
                {
                    return Err(Error::Unsupported(
                        "render unresolved complex-script context",
                    ));
                }
            } else if run.text.chars().any(super::paragraphs::direction::arabic) {
                return Err(Error::Unsupported("render Arabic complex font context"));
            } else if run.properties.hint > 1 {
                return Err(Error::Unsupported("render source font hint"));
            }
            total_runs += 1;
            if total_runs > 65536 {
                return Err(Error::Limit("render model runs"));
            }
            if run.source_cp[0] != cp
                || run.source_cp[1] <= cp
                || run.source_cp[1] > paragraph.source_cp[1]
                || run.text.encode_utf16().count() != run.source_cp[1] - cp
                || run.properties.fonts.iter().any(|i| *i >= font_count)
                || !(2..=3276).contains(&run.properties.size_half_points)
                || run.properties.kerning_threshold_half_points > 3276
                || !(1..=600).contains(&run.properties.scale_percent)
                || run.properties.vertical_alignment > 2
            {
                return Err(Error::Invalid("render model run ownership or font"));
            }
            for c in run.text.chars() {
                if c == '\r' || (c == '\u{7}' && table_marks.contains(&cp)) {
                    if cp + 1 != paragraph.source_cp[1] {
                        return Err(Error::Invalid("render model interior paragraph mark"));
                    }
                } else if matches!(c, '\u{2}' | '\u{5}') {
                    if !note_markers.contains(&cp) || run.source_cp != [cp, cp + 1] {
                        return Err(Error::Invalid("render note marker run"));
                    }
                } else if c == '\u{8}' && shape_anchors.contains(&cp) {
                    if run.text != "\u{8}" || run.source_cp != [cp, cp + 1] {
                        return Err(Error::Invalid("render shape anchor run"));
                    }
                } else if c == '\u{1}' && picture_cps.contains(&cp) {
                    if run.text != "\u{1}" || run.source_cp != [cp, cp + 1] || run.properties.hidden
                    {
                        return Err(Error::Invalid("inline picture source run"));
                    }
                } else if !(supported_source_character(c)
                    || matches!(c, '\u{1}' | '\u{13}'..='\u{15}')
                        && cached_fields::marker(&paragraph.cached_fields, cp))
                {
                    return Err(Error::Unsupported("render model character"));
                }
                cp += c.len_utf16();
            }
        }
        let expected_mark = if section_marks.contains(&(paragraph.source_cp[1] - 1)) {
            '\u{c}'
        } else if table_marks.contains(&(paragraph.source_cp[1] - 1)) {
            '\u{7}'
        } else {
            '\r'
        };
        if cp != paragraph.source_cp[1]
            || !paragraph.runs.last().unwrap().text.ends_with(expected_mark)
        {
            return Err(Error::Invalid("render model paragraph end"));
        }
    }
    if paragraphs.is_empty() || cp != units {
        return Err(Error::Invalid("render model body partition"));
    }
    Ok(())
}

/// Source-known layout options whose effects are not implemented by this adapter.
/// Consumers must mark any produced scene partial while retaining these source bits.
pub fn unmodeled_compatibility_options(
    settings: &settings::DocumentSettings,
) -> Vec<&settings::CompatibilityOption> {
    settings
        .compatibility
        .iter()
        .filter(|option| option.enabled && !modeled_compatibility_name(option.name))
        .collect()
}

/// Retains source table-layout options whose effects require partial scene status.
pub fn unmodeled_table_compatibility_options(
    document: &PlainDocument,
) -> Vec<&settings::CompatibilityOption> {
    document
        .settings
        .compatibility
        .iter()
        .filter(|option| {
            !document.tables.is_empty()
                && option.enabled
                && unsupported_table_compatibility(option.name, document.section.grid_mode.value)
        })
        .collect()
}

fn modeled_compatibility_name(name: &str) -> bool {
    matches!(
        name,
        "word2002_table_style_rules"
            | "grow_autofit"
            | "leave_backslash_alone"
            | "expand_shift_return"
            | "do_not_underline_trailing_spaces"
            | "do_not_balance_single_double_byte_width"
            | "do_not_make_space_for_underline"
            | "do_not_adjust_line_height_in_table"
            | "normal_style_for_list"
            | "ignore_numbering_indent_tab"
            | "alternate_east_asian_line_breaking"
            | "same_style_spacing_in_table"
            | "word11_indent_rules"
            | "do_not_autofit_constrained_tables"
            | "autofit_like_word11"
            | "hangul_width_like_word11"
            | "split_page_break_and_paragraph_mark"
            | "do_not_vertically_align_cell_with_shapes"
            | "do_not_break_constrained_forced_tables"
            | "do_not_vertically_align_textbox"
            | "word11_kerning_pairs"
            | "cached_column_balance"
            | "do_not_break_wrapped_tables"
            | "do_not_snap_to_grid_in_cell"
            | "do_not_allow_field_end_selection"
            | "do_not_wrap_punctuation"
            | "do_not_use_asian_break_rules"
            | "ethiopic_amharic_breaking_rules"
            | "do_not_use_html_auto_spacing"
    )
}

fn qualify_settings(settings: &settings::DocumentSettings) -> Result<()> {
    if !settings.issues.is_empty()
        || settings.update_styles_from_template
        || settings.automatic_hyphenation
        || settings.mirror_margins
        || settings.gutter_at_top
        || settings.two_pages_per_sheet
        || settings.override_normal_font_and_language
        || settings.override_section_geometry
        || settings.default_tab_twips == 0
    {
        return Err(Error::Unsupported("render document settings"));
    }
    if settings
        .compatibility
        .iter()
        .any(|option| !settings::known_compatibility_name(option.name))
    {
        return Err(Error::Unsupported("render unknown compatibility option"));
    }
    Ok(())
}

/// Qualify authored body and plain header/footer content in one Western-text section.
/// Preserve source paragraph ends and header slot ownership in the renderable model.
/// Unsupported components reject the whole document.
pub fn resolve(data: &[u8]) -> Result<PlainDocument> {
    let source = crate::office::doc::inspect(data)?;
    let body = source
        .stories
        .first()
        .ok_or(Error::Invalid("render body missing"))?;
    if body.start_cp != 0
        || source.stories.iter().enumerate().any(|(i, s)| {
            i != 0 && i != 1 && i != 2 && i != 4 && i != 5 && i != 6 && i != 7 && s.utf16_units != 0
        })
    {
        return Err(Error::Unsupported("render secondary story"));
    }
    let mut note_document = notes::inspect(data, &source)?;
    let endnotes = notes::inspect_endnotes(data, &source)?;
    if note_document.is_some() && endnotes.is_some() {
        return Err(Error::Unsupported(
            "render mixed footnote and endnote placement",
        ));
    }
    note_document = note_document.or(endnotes);
    let comment_source = comments::inspect(data, &source)?;
    if note_document.is_some()
        && comment_source
            .as_ref()
            .is_some_and(|(p, _)| !p.notes.is_empty())
    {
        return Err(Error::Unsupported("render comments mixed with notes"));
    }
    let mut empty_comment_plan = None;
    let comment_ownership = if let Some((plan, ownership)) = comment_source {
        if plan.notes.is_empty() {
            empty_comment_plan = Some(plan);
        } else {
            note_document = Some(plan);
        }
        Some(ownership)
    } else {
        None
    };
    let note_markers: std::collections::BTreeSet<_> =
        note_document.iter().flat_map(|n| n.marker_cps()).collect();
    let mut textbox_storage = textboxes::inspect(data, &source)?;
    let mut shape_anchors = textboxes::anchor_cps(&textbox_storage)?;
    if body.utf16_units > 1024 * 1024 {
        return Err(Error::Limit("render body units"));
    }
    let container = crate::office::doc::compound::Compound::open(data)?;
    let word = container.stream("WordDocument")?;
    let (_, table_name, _, _, _) = crate::office::doc::document::fib(&word)?;
    let table = container.stream(table_name)?;
    let drawing_proof = drawings::qualify(&word, &table)?;
    shape_anchors.extend(
        drawing_proof
            .iter()
            .flat_map(|d| &d.omitted_shapes)
            .filter(|s| s.source_rectangle_twips.is_some())
            .map(|s| s.group_anchor_cp),
    );
    shape_anchors.extend(
        drawing_proof
            .iter()
            .flat_map(|d| d.images())
            .filter(|s| s.anchor_story == "body")
            .map(|s| s.anchor_cp),
    );
    let mut cached = cached_fields::inspect(data, body)?;
    cached_fields::validate_shape_anchors(&cached, &shape_anchors)?;
    if crate::office::doc::ordinary_symbols::inspect_embedded_fonts(data)? {
        return Err(Error::Unsupported("render embedded fonts"));
    }
    textboxes::validate_drawings(&textbox_storage, drawing_proof.as_ref())?;
    let image_anchors: std::collections::BTreeSet<_> = drawing_proof
        .iter()
        .flat_map(|d| d.images())
        .map(|s| s.anchor_cp)
        .collect();
    if drawing_proof.iter().any(|d| {
        d.images()
            .iter()
            .any(|s| !matches!(s.anchor_story, "headers" | "body"))
    }) || image_anchors.len()
        != drawing_proof
            .iter()
            .map(|d| d.images().len())
            .sum::<usize>()
    {
        return Err(Error::Unsupported("render image anchor scope"));
    }
    shape_anchors.extend(image_anchors.iter().copied());
    for line in drawing_proof.iter().flat_map(|d| {
        d.lines()
            .iter()
            .chain(d.rectangles().iter().map(|r| &r.frame))
    }) {
        if line.anchor_story != "headers" || !shape_anchors.insert(line.anchor_cp) {
            return Err(Error::Invalid("duplicate or nonheader source line anchor"));
        }
    }
    let settings = settings::inspect_settings(data)?;
    qualify_settings(&settings)?;
    let headers = crate::office::doc::headers::inspect_headers(data, &source)?;
    if !headers.diagnostics.is_empty()
        || headers.sections.is_empty()
        || headers.sections.iter().any(|s| !s.unresolved.is_empty())
    {
        return Err(Error::Unsupported("render section ownership"));
    }
    let mut geometry = sections::resolve(&headers, body.utf16_units)?.into_iter();
    let section = geometry
        .next()
        .ok_or(Error::Invalid("render section missing"))?;
    let unmodeled_section_options =
        section_options::retain(&section, &headers.sections[0].properties)?;
    let following_sections = geometry
        .map(|geometry| {
            let unmodeled_options = section_options::retain(
                &geometry,
                &headers.sections[geometry.section_index].properties,
            )?;
            Ok(multi_sections::FollowingSection {
                geometry,
                unmodeled_options,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    columns::width_twips(&section)?;
    if section.vertical_justification.value != 0 || section.gutter_twips.value != 0 {
        return Err(Error::Unsupported("render section layout"));
    }
    let section_marks: std::collections::BTreeSet<_> = std::iter::once(&section)
        .chain(following_sections.iter().map(|s| &s.geometry))
        .take(following_sections.len())
        .map(|g| g.source_cp[1] - 1)
        .collect();
    let mut header_document = headers::prepare(headers, &source)?;
    if let Some(headers) = &mut header_document {
        headers.retain_inactive_uncached(&crate::office::doc::field_inspection::inspect_fields(
            data,
        )?)?;
        cached.extend(cached_fields::inspect_story_selected(
            data,
            &source.stories[2],
            &headers.inactive_ranges(),
        )?);
    }
    let mut header_picture_cps = std::collections::BTreeSet::new();
    if let Some(h) = &header_document {
        let ranges = h.ranges()?;
        let mut cp = source.stories[2].start_cp;
        for c in source.stories[2].text.chars() {
            if c == '\u{1}' && ranges.iter().any(|range| range[0] <= cp && cp < range[1]) {
                header_picture_cps.insert(cp);
            }
            cp += c.len_utf16();
        }
    }
    let inline_pictures = inline_pictures::inspect_with_header_controls(data, &header_picture_cps)?;
    let picture_cps: std::collections::BTreeSet<_> = inline_pictures
        .iter()
        .flat_map(|p| &p.references)
        .map(|p| p.source_cp)
        .collect();
    if body
        .text
        .chars()
        .scan(0usize, |cp, c| {
            let at = *cp;
            *cp += c.len_utf16();
            Some((at, c))
        })
        .any(|(cp, c)| {
            !(c == '\u{7}'
                || supported_source_character(c)
                || cached_fields::marker(&cached, cp)
                || c == '\u{8}' && shape_anchors.contains(&cp)
                || c == '\u{1}' && picture_cps.contains(&cp)
                || matches!(c, '\u{2}' | '\u{5}') && note_markers.contains(&cp))
        })
    {
        return Err(Error::Unsupported("render character, field or component"));
    }
    let mut ranges = vec![[0, body.utf16_units]];
    if let Some(n) = &note_document {
        ranges.extend(n.ranges());
    }
    if let Some(h) = &header_document {
        ranges.extend(h.ranges()?);
    }
    ranges.extend(
        textbox_storage
            .iter()
            .flat_map(|s| &s.textboxes)
            .map(|b| b.source_cp),
    );
    let units: Vec<u16> = source
        .stories
        .iter()
        .flat_map(|s| s.text.encode_utf16())
        .collect();
    for [a, b] in &ranges {
        qualify_source_range_with_pictures(
            &units[*a..*b],
            *a,
            &cached,
            &shape_anchors,
            &note_markers,
            &picture_cps,
        )?;
    }
    let overlaps = |a: usize, b: usize| ranges.iter().any(|r| a < r[1] && r[0] < b);
    let ends = crate::office::doc::paragraph_end_formatting::inspect_paragraph_ends(data)?;
    if !ends.uncertain_terminals.is_empty() || ends.endpoints.len() > 16384 {
        return Err(Error::Unsupported("render physical paragraph endpoints"));
    }
    let paragraphs = crate::office::doc::paragraph_references::inspect_paragraph_references(
        data,
        &ends.formatting,
    )?;
    let formatting = crate::office::doc::formatting_inspection::inspect_character_formatting(data)?;
    if formatting.pieces.iter().any(|p| p.raw_prm != 0) {
        return Err(Error::Unsupported("render piece property modifier"));
    }
    let styles = crate::office::doc::styles::inspect_styles(data)?;
    let mut table_structure = None;
    let (mut tables, mut pending_table_lists) = if body.text.contains('\u{7}') {
        let projected = crate::office::doc::transcribe(&source);
        let structure =
            crate::office::doc::structure::inspect_structure(data, &source, &projected)?;
        let ownership = crate::office::doc::table_structure::plan_tables(
            &source,
            &structure,
            &paragraphs,
            &formatting,
        )?;
        let bound = super::tables::source::bind_tables_deferred(
            &word,
            &source,
            &structure,
            &paragraphs,
            &formatting,
            &ownership,
            &styles,
        )?;
        table_structure = Some(structure);
        bound
    } else {
        (Vec::new(), std::collections::BTreeMap::new())
    };
    let fonts = crate::office::doc::symbols::inspect_symbol_fonts(data)?;
    let revisions = crate::office::doc::visibility::resolve_revision_evidence(&formatting)?;
    let source_symbols = symbols::inspect(&fonts, &revisions, &units)?;
    for cp in &shape_anchors {
        let span = revisions
            .spans
            .get(revisions.spans.partition_point(|s| s.end_cp <= *cp))
            .filter(|s| s.start_cp <= *cp && *cp < s.end_cp)
            .ok_or(Error::Invalid("render shape revision coverage"))?;
        if units.get(*cp) != Some(&8) || span.properties.special != Some(true) {
            return Err(Error::Unsupported("render shape anchor special property"));
        }
    }

    for cp in &picture_cps {
        let span = revisions
            .spans
            .get(revisions.spans.partition_point(|s| s.end_cp <= *cp))
            .filter(|s| s.start_cp <= *cp && *cp < s.end_cp)
            .ok_or(Error::Invalid("inline picture revision coverage"))?;
        if units.get(*cp) != Some(&1) || span.properties.special != Some(true) {
            return Err(Error::Unsupported("inline picture special property"));
        }
    }
    if revisions
        .spans
        .iter()
        .filter(|s| overlaps(s.start_cp, s.end_cp))
        .any(|s| {
            !s.unresolved.is_empty()
                || s.properties.deleted != Some(false)
                || s.properties.inserted != Some(false)
                || !(cached_fields::special_matches(
                    &cached,
                    s.start_cp,
                    s.end_cp,
                    s.properties.special,
                ) || cached_fields::special_with_picture_markers(
                    &cached,
                    &picture_cps,
                    s.start_cp,
                    s.end_cp,
                    s.properties.special,
                ) || (s.properties.special == Some(true)
                    && s.end_cp == s.start_cp + 1
                    && source_symbols.contains_key(&s.start_cp))
                    || (s.properties.special == Some(true)
                        && s.end_cp == s.start_cp + 1
                        && shape_anchors.contains(&s.start_cp)
                        && units.get(s.start_cp) == Some(&8))
                    || (s.properties.special == Some(true)
                        && ((s.start_cp < s.end_cp
                            && (s.start_cp..s.end_cp).all(|cp| {
                                picture_cps.contains(&cp) && units.get(cp) == Some(&1)
                            }))
                            || (s.end_cp == s.start_cp + 1
                                && note_markers.contains(&s.start_cp)
                                && matches!(units.get(s.start_cp), Some(2 | 5))))))
                || !cached_fields::field_visibility_matches(
                    &cached,
                    s.start_cp,
                    s.end_cp,
                    s.properties.field_hidden,
                )
        })
    {
        return Err(Error::Unsupported(
            "render revision or special character context",
        ));
    }
    let (hidden, table_hidden) = if let Some(structure) = &table_structure {
        let (hidden, table) =
            crate::office::doc::hidden_visibility::resolve_hidden_evidence_with_tables(
                &source,
                &formatting,
                &paragraphs,
                &styles,
                structure,
                &revisions,
            )?;
        (hidden, Some(table))
    } else {
        (
            crate::office::doc::hidden_visibility::resolve_hidden_evidence(
                &source,
                &formatting,
                &paragraphs,
                &styles,
            )?,
            None,
        )
    };
    if hidden
        .spans
        .iter()
        .filter(|s| overlaps(s.start_cp, s.end_cp))
        .any(|s| {
            !s.property.unresolved.is_empty()
                || !(s.property.hidden == Some(false)
                    || (s.property.hidden == Some(true)
                        && s.paragraph_table_depth == Some(0)
                        && s.end_cp == s.start_cp + 1
                        && units.get(s.start_cp) == Some(&13)
                        && paragraphs
                            .spans
                            .get(paragraphs.spans.partition_point(|p| p.end_cp <= s.start_cp))
                            .is_some_and(|p| p.start_cp == s.start_cp && p.end_cp == s.end_cp)))
                || (s.paragraph_table_depth != Some(0)
                    && !tables
                        .get(tables.partition_point(|t| t.source_cp[1] <= s.start_cp))
                        .is_some_and(|t| {
                            t.source_cp[0] <= s.start_cp && s.end_cp <= t.source_cp[1]
                        }))
        })
    {
        return Err(Error::Unsupported("render visibility or table context"));
    }
    let definitions = crate::office::doc::lists::inspect_lists(data)?;
    let fields = crate::office::doc::field_inspection::inspect_fields(data)?;
    let list_context = crate::office::doc::list_projection::ListContext {
        source: &source,
        definitions: &definitions,
        paragraphs: &paragraphs,
        styles: &styles,
        formatting: &formatting,
        revisions: &revisions,
        fonts: &fonts,
        font_defaults_preserved: Some(
            !settings.override_normal_font_and_language
                && !settings.update_styles_from_template
                && settings.issues.is_empty(),
        ),
        fields: &fields,
    };
    let selected_labels = lists::prepare_with_tables(&list_context, false, table_hidden.as_ref())?;
    let mut table_list_work = 0;
    super::tables::source::nested::visit_paragraphs_mut(&mut tables, &mut |member| {
        if let Some(direct) = pending_table_lists.remove(&member.source_cp) {
            let index = selected_labels.partition_point(|l| l.anchor_cp < member.source_cp[0]);
            let label = selected_labels
                .get(index)
                .filter(|l| l.anchor_cp == member.source_cp[0])
                .ok_or(Error::Unsupported("table list source label unresolved"))?;
            let resolved = lists::paragraph_modifiers(label, &direct, &mut table_list_work)?;
            member.properties = paragraphs::resolve(&styles, member.source_style, &resolved)?;
        }
        Ok(())
    })?;
    if !pending_table_lists.is_empty() {
        return Err(Error::Invalid("table list source ownership"));
    }
    let mut table_properties = std::collections::BTreeMap::new();
    for table in super::tables::source::nested::tables(&tables)? {
        for row in &table.rows {
            for cell in &row.cells {
                for member in &cell.paragraphs {
                    table_properties
                        .insert(member.source_cp, (member.source_style, &member.properties));
                }
            }
            table_properties.insert(
                [row.source_cp[1] - 1, row.source_cp[1]],
                (row.row_mark_style, &row.row_mark_properties),
            );
        }
    }

    let mut list_presentations = Vec::new();
    let mut outputs = Vec::new();
    let mut run_count = 0usize;
    let mut work = 0usize;
    let style_work: usize = styles
        .styles
        .iter()
        .flatten()
        .map(|s| {
            1 + s
                .properties
                .iter()
                .map(|p| p.bytes.len() + 1)
                .sum::<usize>()
        })
        .sum();
    for [range_start, range_end] in ranges {
        let mut output = Vec::new();
        let mut start = range_start;
        for paragraph in paragraphs
            .spans
            .iter()
            .filter(|p| p.start_cp < range_end && range_start < p.end_cp)
        {
            if paragraph.start_cp != start
                || !(paragraph_range(&units, start, paragraph.end_cp)
                    || (section_marks.contains(&(paragraph.end_cp - 1))
                        && units.get(paragraph.end_cp - 1) == Some(&12)
                        && !units[start..paragraph.end_cp - 1].contains(&13))
                    || (table_properties.contains_key(&[start, paragraph.end_cp])
                        && units[paragraph.end_cp - 1] == 7
                        && !units[start..paragraph.end_cp - 1]
                            .iter()
                            .any(|c| matches!(c, 7 | 13))))
            {
                return Err(Error::Unsupported("render paragraph control ownership"));
            }
            let style = paragraph
                .style_index
                .ok_or(Error::Unsupported("render paragraph style absent"))?
                as usize;
            let mark = paragraph.end_cp - 1;
            let end_piece = formatting
                .pieces
                .get(
                    formatting
                        .pieces
                        .partition_point(|p| p.start_cp <= mark)
                        .saturating_sub(1),
                )
                .filter(|p| p.start_cp <= mark && mark < p.end_cp)
                .ok_or(Error::Invalid("render paragraph piece"))?;
            let properties = if let Some((source_style, properties)) =
                table_properties.get(&[start, paragraph.end_cp])
            {
                if *source_style != style {
                    return Err(Error::Invalid("render table paragraph style"));
                }
                (*properties).clone()
            } else {
                let mut paragraph_properties = paragraphs::decode(&paragraph.raw_modifiers)?;
                paragraph_properties.extend(
                    crate::office::doc::visibility::piece_modifiers(
                        end_piece.raw_prm,
                        &formatting.raw_property_groups,
                    )?
                    .into_iter()
                    .filter(|p| (p.sprm >> 10) & 7 == 1),
                );
                if let Some(label) = selected_labels
                    .get(selected_labels.partition_point(|l| l.anchor_cp < start))
                    .filter(|l| l.anchor_cp == start)
                {
                    paragraph_properties =
                        lists::paragraph_modifiers(label, &paragraph_properties, &mut work)?;
                }
                paragraphs::resolve(&styles, style, &paragraph_properties)?
            };
            let mut paragraph_fields =
                cached_fields::for_paragraph(&cached, [start, paragraph.end_cp])?;
            let mut runs = Vec::new();
            let mut cp = start;
            while cp < paragraph.end_cp {
                run_count += 1;
                if run_count > 65536 {
                    return Err(Error::Limit("render run count"));
                }
                let span = formatting
                    .character_spans
                    .get(
                        formatting
                            .character_spans
                            .partition_point(|s| s.start_cp <= cp)
                            .saturating_sub(1),
                    )
                    .filter(|s| s.start_cp <= cp && cp < s.end_cp)
                    .ok_or(Error::Invalid("render character coverage"))?;
                let piece = formatting
                    .pieces
                    .get(
                        formatting
                            .pieces
                            .partition_point(|s| s.start_cp <= cp)
                            .saturating_sub(1),
                    )
                    .filter(|s| s.start_cp <= cp && cp < s.end_cp)
                    .ok_or(Error::Invalid("render piece coverage"))?;
                let mut end = cached_fields::next_boundary(
                    &paragraph_fields,
                    cp,
                    paragraph.end_cp.min(span.end_cp).min(piece.end_cp),
                );
                if shape_anchors.contains(&cp) || picture_cps.contains(&cp) {
                    end = end.min(cp + 1);
                } else {
                    if let Some(next) = shape_anchors.range(cp + 1..end).next() {
                        end = *next;
                    }
                    if let Some(next) = picture_cps.range(cp + 1..end).next() {
                        end = *next;
                    }
                }
                if note_markers.contains(&cp) {
                    end = end.min(cp + 1);
                } else if let Some(next) = note_markers.range(cp + 1..end).next() {
                    end = *next;
                }
                let mut direct = span.properties.clone();
                direct.extend(
                    crate::office::doc::visibility::piece_modifiers(
                        piece.raw_prm,
                        &formatting.raw_property_groups,
                    )?
                    .into_iter()
                    .filter(|p| (p.sprm >> 10) & 7 == 2),
                );
                work = work
                    .checked_add(
                        style_work + direct.iter().map(|p| p.operand.len() + 2).sum::<usize>() + 1,
                    )
                    .filter(|n| *n <= 8 * 1024 * 1024)
                    .ok_or(Error::Limit("render property expansion"))?;
                if cached_fields::marker(&paragraph_fields, cp) && end == cp + 1 {
                    cached_fields::record_marker(&mut paragraph_fields, cp, &direct, &mut work)?;
                }
                let special_placeholder =
                    cached_fields::placeholder_matches(&paragraph_fields, cp, end)
                        && revisions
                            .spans
                            .get(revisions.spans.partition_point(|s| s.end_cp <= cp))
                            .filter(|s| s.start_cp <= cp && end <= s.end_cp)
                            .is_some_and(|s| s.properties.special == Some(true));
                if special_placeholder {
                    cached_fields::record_placeholder(
                        &mut paragraph_fields,
                        [cp, end],
                        &direct,
                        &mut work,
                    )?;
                }
                let structural: Vec<_> = if special_placeholder {
                    direct
                        .iter()
                        .filter(|p| cached_fields::placeholder_structural_modifier(p))
                        .cloned()
                        .collect()
                } else if cached_fields::marker(&paragraph_fields, cp) && end == cp + 1 {
                    direct
                        .iter()
                        .filter(|p| cached_fields::structural_modifier(&paragraph_fields, cp, p))
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                };
                direct.retain(|p| !structural.contains(p));
                if picture_cps.contains(&cp) && end == cp + 1 {
                    let reference = inline_pictures
                        .as_ref()
                        .and_then(|p| {
                            p.references
                                .get(p.references.partition_point(|r| r.source_cp < cp))
                                .filter(|r| r.source_cp == cp)
                        })
                        .ok_or(Error::Invalid("inline picture source proof"))?;
                    if reference.raw_modifiers != direct {
                        return Err(Error::Invalid("inline picture modifier proof"));
                    }
                    let structural: Vec<_> = direct
                        .iter()
                        .filter(|p| matches!(p.sprm, 0x6a03 | 0x0855))
                        .collect();
                    if structural.len() != 2
                        || structural.iter().filter(|p| p.sprm == 0x6a03).count() != 1
                        || structural.iter().filter(|p| p.sprm == 0x0855).count() != 1
                    {
                        return Err(Error::Unsupported(
                            "inline picture structural property history",
                        ));
                    }
                    direct.retain(|p| !matches!(p.sprm, 0x6a03 | 0x0855));
                }
                let properties =
                    if let Some(symbol) = source_symbols.get(&cp).filter(|_| end == cp + 1) {
                        runs::symbols::resolve(&styles, &fonts, style, &direct, &mut work, symbol)?
                    } else if note_markers.contains(&cp) && end == cp + 1 {
                        runs::resolve_owned_note_marker(&styles, &fonts, style, &direct, &mut work)?
                    } else if shape_anchors.contains(&cp) && end == cp + 1 {
                        runs::resolve_shape_anchor(&styles, &fonts, style, &direct, &mut work)?
                    } else {
                        runs::resolve_with_budget(&styles, &fonts, style, &direct, &mut work)?
                    };
                let visibility = hidden
                    .spans
                    .get(hidden.spans.partition_point(|s| s.end_cp <= cp))
                    .filter(|s| s.start_cp <= cp && end <= s.end_cp)
                    .ok_or(Error::Invalid("render hidden property coverage"))?;
                if visibility.property.hidden != Some(properties.hidden) {
                    return Err(Error::Unsupported("render hidden property disagreement"));
                }
                let text = String::from_utf16(&units[cp..end])
                    .map_err(|_| Error::Invalid("render UTF16 range"))?;
                // Symbol-charsets and PUA use a different source-to-glyph contract.
                for c in text.chars() {
                    if properties.source_symbol.is_some() {
                        continue;
                    }
                    if (c == '\u{8}' && shape_anchors.contains(&cp)
                        || c == '\u{1}' && picture_cps.contains(&cp)
                        || matches!(c, '\u{2}' | '\u{5}') && note_markers.contains(&cp))
                        && end == cp + 1
                    {
                        continue;
                    }
                    qualify_character_charset(
                        c,
                        properties.fonts.map(|index| fonts.fonts[index].charset),
                        properties.hint,
                    )?;
                }
                if let Some(value) = cached_fields::generated_at(&paragraph_fields, cp)
                    && value.kind == "dropdown"
                {
                    for c in value.text.chars() {
                        qualify_character_charset(
                            c,
                            properties.fonts.map(|index| fonts.fonts[index].charset),
                            properties.hint,
                        )?;
                    }
                }
                runs.push(PlainRun {
                    source_cp: [cp, end],
                    text,
                    properties,
                });
                cp = end;
            }
            if let Some(label) = selected_labels
                .get(selected_labels.partition_point(|l| l.anchor_cp < start))
                .filter(|l| l.anchor_cp == start)
            {
                if range_start != 0 {
                    return Err(Error::Unsupported("render list table or secondary scope"));
                }
                let first = formatting
                    .character_spans
                    .get(
                        formatting
                            .character_spans
                            .partition_point(|s| s.end_cp <= start),
                    )
                    .ok_or(Error::Invalid("render list first span"))?;
                let mark_span = formatting
                    .character_spans
                    .get(
                        formatting
                            .character_spans
                            .partition_point(|s| s.end_cp <= mark),
                    )
                    .ok_or(Error::Invalid("render list mark span"))?;
                if first.start_cp > start || mark_span.start_cp > mark {
                    return Err(Error::Invalid("render list character gap"));
                }
                work = work
                    .checked_add(paragraph.raw_modifiers.len() + 1)
                    .filter(|n| *n <= 8 * 1024 * 1024)
                    .ok_or(Error::Limit("render list paragraph evidence"))?;
                list_presentations.push((
                    lists::presentation(
                        &list_context,
                        label,
                        style,
                        &first.properties,
                        &mark_span.properties,
                        &mut work,
                    )?,
                    paragraph.raw_modifiers.clone(),
                ));
            }
            output.push(PlainParagraph {
                cached_fields: paragraph_fields,
                source_cp: [start, paragraph.end_cp],
                source_style: style,
                properties,
                runs,
            });
            start = paragraph.end_cp;
        }
        if start != range_end || output.is_empty() {
            return Err(Error::Invalid("render body ownership"));
        }
        outputs.push(output);
    }
    let mut outputs = outputs.into_iter();
    let output = outputs.next().ok_or(Error::Invalid("render body output"))?;
    if let Some(n) = &mut note_document {
        for note in &mut n.notes {
            note.paragraphs = outputs.next().ok_or(Error::Invalid("render note output"))?;
        }
        for separator in n
            .separators
            .iter_mut()
            .filter(|s| !s.graphic_controls_only())
        {
            separator.paragraphs = outputs
                .next()
                .ok_or(Error::Invalid("render note separator output"))?;
        }
    }
    if let Some(h) = &mut header_document {
        for slot in &mut h.slots {
            slot.paragraphs = outputs
                .next()
                .ok_or(Error::Invalid("render header output"))?;
        }
    }
    for textbox in textbox_storage.iter_mut().flat_map(|s| &mut s.textboxes) {
        textbox.paragraphs = outputs
            .next()
            .ok_or(Error::Invalid("render textbox output"))?;
    }
    if outputs.next().is_some() {
        return Err(Error::Invalid("render output coverage"));
    }
    let image_assets = if image_anchors.is_empty() {
        Vec::new()
    } else {
        crate::office::doc::pictures::raster_store::inspect(data)?
    };
    let document = PlainDocument {
        image_assets,
        inline_pictures,
        source_utf16_units: units.len(),
        settings,
        section,
        following_sections,
        unmodeled_section_options,
        paragraphs: output,
        tables,
        font_charsets: fonts.fonts.iter().map(|f| f.charset).collect(),
        font_names: fonts.fonts.into_iter().map(|f| f.name).collect(),
        headers: header_document,
        comments: if let Some(ownership) = comment_ownership {
            Some(comments::CommentDocument {
                plan: if let Some(empty) = empty_comment_plan {
                    empty
                } else {
                    note_document
                        .take()
                        .ok_or(Error::Invalid("comment render plan"))?
                },
                ownership,
            })
        } else {
            None
        },
        endnotes: if note_document
            .as_ref()
            .is_some_and(|n| n.kind == notes::NoteKind::Endnote)
        {
            note_document.take()
        } else {
            None
        },
        footnotes: note_document,
        list_labels: {
            if selected_labels.len() != list_presentations.len() {
                return Err(Error::Invalid("render list label coverage"));
            }
            selected_labels
                .into_iter()
                .zip(list_presentations)
                .map(
                    |(source, (presentation, source_paragraph_papx))| lists::PaintLabel {
                        source,
                        presentation,
                        source_paragraph_papx,
                    },
                )
                .collect()
        },
        drawing_proof,
        textbox_storage,
    };
    document.validate()?;
    Ok(document)
}

fn paragraph_range(units: &[u16], start: usize, end: usize) -> bool {
    start < end
        && end <= units.len()
        && units[end - 1] == 13
        && !units[start..end - 1].contains(&13)
}

// Decode complete scalars while advancing their original UTF-16 positions.
// Supplementary ideographs occupy two source units and one shaped character.
#[cfg(test)]
fn qualify_source_range(
    units: &[u16],
    start: usize,
    fields: &[cached_fields::CachedHyperlink],
    shapes: &std::collections::BTreeSet<usize>,
    notes: &std::collections::BTreeSet<usize>,
) -> Result<()> {
    qualify_source_range_with_pictures(units, start, fields, shapes, notes, &Default::default())
}

fn qualify_source_range_with_pictures(
    units: &[u16],
    start: usize,
    fields: &[cached_fields::CachedHyperlink],
    shapes: &std::collections::BTreeSet<usize>,
    notes: &std::collections::BTreeSet<usize>,
    pictures: &std::collections::BTreeSet<usize>,
) -> Result<()> {
    let text =
        String::from_utf16(units).map_err(|_| Error::Invalid("render UTF16 source range"))?;
    let mut cp = start;
    for c in text.chars() {
        if !(c == '\u{7}'
            || supported_source_character(c)
            || c == '\u{8}' && shapes.contains(&cp)
            || matches!(c, '\u{2}' | '\u{5}') && notes.contains(&cp)
            || c == '\u{1}' && pictures.contains(&cp)
            || cached_fields::marker(fields, cp))
        {
            return Err(Error::Unsupported("render character, field or component"));
        }
        cp += c.len_utf16();
    }
    Ok(())
}

// Manual line breaks use the non-ASCII font for their source-owned metrics.
fn qualify_character_charset(c: char, charsets: [u8; 3], hint: u8) -> Result<()> {
    let slot = super::runs::font_slot(c, hint);
    if charsets[slot] == 2 {
        return Err(Error::Unsupported("render symbol font"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
