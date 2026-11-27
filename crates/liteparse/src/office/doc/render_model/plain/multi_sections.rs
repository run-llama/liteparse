//! Section partitions retain their PLC terminator independently of manual breaks.
use super::*;

#[derive(Debug, Serialize)]
pub struct FollowingSection {
    pub geometry: sections::SectionGeometry,
    pub unmodeled_options: Vec<section_options::SectionOption>,
}

pub fn geometries(document: &PlainDocument) -> impl Iterator<Item = &sections::SectionGeometry> {
    std::iter::once(&document.section)
        .chain(document.following_sections.iter().map(|s| &s.geometry))
}

pub fn terminators(document: &PlainDocument) -> impl Iterator<Item = usize> + '_ {
    geometries(document)
        .take(document.following_sections.len())
        .map(|s| s.source_cp[1] - 1)
}

// Equal effective layout permits the existing whole-body replay to retain global owners.
// Source provenance and first-page header activation remain distinct for each section.
fn same_layout(a: &sections::SectionGeometry, b: &sections::SectionGeometry) -> bool {
    macro_rules! values { ($($f:ident),*) => {true $(&& a.$f.value == b.$f.value)*}; }
    macro_rules! optional { ($($f:ident),*) => {true $(&& a.$f.map(|v|v.value) == b.$f.map(|v|v.value))*}; }
    values!(
        page_width_twips,
        page_height_twips,
        landscape,
        gutter_twips,
        column_count,
        evenly_spaced_columns,
        column_rule,
        vertical_justification,
        grid_mode,
        grid_character_spacing
    ) && optional!(
        left_margin_twips,
        right_margin_twips,
        top_margin_twips,
        bottom_margin_twips,
        header_distance_twips,
        footer_distance_twips,
        column_spacing_twips,
        grid_line_pitch_twips
    )
}

pub fn validate(document: &PlainDocument) -> Result<()> {
    if document.following_sections.len() >= 8192 {
        return Err(Error::Limit("render section count"));
    }
    if document.following_sections.is_empty() {
        return Ok(());
    }
    if document.active_notes().is_some() {
        return Err(Error::Unsupported("multi-section note ownership"));
    }
    let mut cp = 0;
    for (index, g) in geometries(document).enumerate() {
        if g.section_index != index
            || g.source_cp[0] != cp
            || g.source_cp[1] <= cp
            || g.source_cp[1] > document.source_utf16_units
        {
            return Err(Error::Invalid("multi-section source partition"));
        }
        if (index < document.following_sections.len() && !matches!(g.section_break.value, 0..=4))
            || !same_layout(&document.section, g)
        {
            return Err(Error::Unsupported("multi-section equal-layout subset"));
        }
        columns::width_twips(g)?;
        if index > 0 {
            section_options::validate(
                g,
                &document.following_sections[index - 1].unmodeled_options,
            )?;
        }
        cp = g.source_cp[1];
    }
    let geometry: Vec<_> = geometries(document).collect();
    for pair in geometry.windows(2) {
        if pair[0].section_break.value > 1 {
            continue;
        }
        if pair.iter().any(|g| g.different_first_page.value) {
            return Err(Error::Unsupported(
                "shared-page section first-header activation",
            ));
        }
        if let Some(headers) = &document.headers {
            let left = headers
                .ownership
                .sections
                .get(pair[0].section_index)
                .ok_or(Error::Invalid("shared-page section header owner"))?;
            let right = headers
                .ownership
                .sections
                .get(pair[1].section_index)
                .ok_or(Error::Invalid("shared-page section header owner"))?;
            if left.references.len() != right.references.len()
                || left
                    .references
                    .iter()
                    .zip(&right.references)
                    .any(|(a, b)| a.role != b.role || a.origin_story_index != b.origin_story_index)
            {
                return Err(Error::Unsupported("shared-page section header change"));
            }
        }
    }
    for marker in terminators(document) {
        let paragraph = document
            .paragraphs
            .get(
                document
                    .paragraphs
                    .partition_point(|p| p.source_cp[1] <= marker),
            )
            .ok_or(Error::Invalid("section terminator paragraph"))?;
        if paragraph.source_cp[1] != marker + 1
            || !paragraph.cached_fields.is_empty()
            || document
                .tables
                .iter()
                .any(|t| t.source_cp[0] <= marker && marker < t.source_cp[1])
            || document
                .list_labels
                .iter()
                .any(|l| l.source.anchor_cp == paragraph.source_cp[0])
        {
            return Err(Error::Unsupported("section terminator compound ownership"));
        }
        let run = paragraph
            .runs
            .iter()
            .find(|r| r.source_cp[0] <= marker && marker < r.source_cp[1])
            .ok_or(Error::Invalid("section terminator run"))?;
        if run.text.encode_utf16().nth(marker - run.source_cp[0]) != Some(12) {
            return Err(Error::Invalid("section terminator source control"));
        }
    }
    if let Some(headers) = &document.headers {
        if headers.ownership.sections.len() != document.following_sections.len() + 1 {
            return Err(Error::Invalid("section header count"));
        }
        for (g, s) in geometries(document).zip(&headers.ownership.sections) {
            if g.source_cp != [s.source.start_cp, s.source.end_cp]
                || s.different_first_page != Some(g.different_first_page.value)
            {
                return Err(Error::Invalid("section header geometry identity"));
            }
        }
    }
    for table in &document.tables {
        if !geometries(document)
            .any(|g| g.source_cp[0] <= table.source_cp[0] && table.source_cp[1] <= g.source_cp[1])
        {
            return Err(Error::Unsupported("table crosses section boundary"));
        }
    }
    Ok(())
}
