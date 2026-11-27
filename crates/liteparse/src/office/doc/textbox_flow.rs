//! Textbox association plans reference projected stories without copying or reordering text.
use crate::office::doc::{
    Error, Inspection, Result, TranscribedStory, Transcription,
    textbox_anchors::{ShapeAnchor, TextboxInspection},
    visibility::RevisionEvidence,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

const MAX_ENTRIES: usize = 65_536;
const MAX_SEGMENTS: usize = 262_144;
const MAX_TEXT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct TextboxFlowPlan {
    pub status: &'static str,
    pub parts: Vec<FlowPart>,
    pub unresolved: [&'static str; 3],
}

#[derive(Debug, Serialize)]
pub struct FlowPart {
    pub story: &'static str,
    pub owner_story: &'static str,
    pub active_ranges: Vec<FlowEntry>,
    pub reusable_records: Vec<usize>,
    /// Indices into active_ranges; these establish source association, not page order.
    pub source_anchor_sequence: Vec<usize>,
    pub source_units_outside_active_ranges: usize,
}

#[derive(Debug, Serialize)]
pub struct FlowEntry {
    pub record_index: usize,
    pub source_range: [usize; 2],
    /// Half-open UTF-8 bytes in the existing projected story, including anchored labels.
    pub projected_bytes: Option<[usize; 2]>,
    pub anchor_cp: Option<usize>,
    pub source_association_supported: bool,
    pub placement: Option<Placement>,
    pub unresolved: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct Placement {
    pub rectangle_twips: [i32; 4],
    pub horizontal_origin: u16,
    pub vertical_origin: u16,
    pub wrapping: u16,
    pub wrapping_side: Option<u16>,
    pub behind_text: Option<bool>,
    pub anchor_locked: bool,
    pub reading_order: &'static str,
}

/// Preserve active/reusable coverage while checking shape and projected-source associations.
/// Hidden text, paragraph pagination, OfficeArt overrides and visual order remain unresolved.
pub fn plan_flow(
    source: &Inspection,
    projected: &Transcription,
    anchors: &TextboxInspection,
    revisions: &RevisionEvidence,
) -> Result<TextboxFlowPlan> {
    if revisions.spans.len() > MAX_SEGMENTS
        || projected.stories.len() > 8
        || source.stories.len() > 8
    {
        return Err(Error::Limit("textbox flow input metadata"));
    }
    if projected
        .stories
        .iter()
        .any(|s| s.segments.len() > MAX_SEGMENTS || s.text.len() > MAX_TEXT_BYTES)
        || source.stories.iter().any(|s| s.text.len() > MAX_TEXT_BYTES)
        || anchors.parts.iter().any(|p| p.shapes.len() > MAX_ENTRIES)
    {
        return Err(Error::Limit("textbox flow input size"));
    }
    if anchors.parts.iter().map(|p| p.records.len()).sum::<usize>() > MAX_ENTRIES {
        return Err(Error::Limit("textbox flow records"));
    }
    if revisions
        .spans
        .windows(2)
        .any(|w| w[0].end_cp != w[1].start_cp)
        || revisions.spans.iter().any(|s| s.start_cp >= s.end_cp)
    {
        return Err(Error::Invalid("textbox flow revision spans"));
    }
    if anchors.parts.len() > 2
        || anchors.parts.iter().enumerate().any(|(i, p)| {
            anchors.parts[..i]
                .iter()
                .any(|q| q.text_story == p.text_story)
        })
    {
        return Err(Error::Invalid("textbox flow owner parts"));
    }
    let mut parts = Vec::new();
    for part in &anchors.parts {
        let raw = unique(source.stories.iter().filter(|s| s.kind == part.text_story))?;
        let owner = unique(
            source
                .stories
                .iter()
                .filter(|s| s.kind == part.anchor_story),
        )?;
        if owner.utf16_units > 16 * 1024 * 1024 {
            return Err(Error::Limit("textbox flow owner units"));
        }
        let owner_units: Vec<_> = owner.text.encode_utf16().collect();
        if owner_units.len() != owner.utf16_units {
            return Err(Error::Invalid("textbox flow owner length"));
        }
        let text = unique(
            projected
                .stories
                .iter()
                .filter(|s| s.kind == part.text_story),
        )?;
        let end = raw
            .start_cp
            .checked_add(raw.utf16_units)
            .ok_or(Error::Invalid("textbox flow source overflow"))?;
        validate_projection(text, raw.start_cp, end)?;
        let requests: BTreeSet<_> = part
            .records
            .iter()
            .filter(|r| !r.reusable)
            .filter_map(|r| r.source.as_ref())
            .flat_map(|r| [r.start_cp, r.end_cp])
            .collect();
        let boundaries = projection_boundaries(text, &requests);
        let mut counts = BTreeMap::new();
        for record in &part.records {
            if !record.reusable
                && let Some(index) = record.anchor_index
            {
                *counts.entry(index).or_insert(0usize) += 1;
            }
        }
        let mut active = Vec::new();
        let mut reusable = Vec::new();
        let mut previous_end = raw.start_cp;
        let mut covered = 0;
        for record in &part.records {
            if record.reusable {
                reusable.push(record.record_index);
                continue;
            }
            let range = record
                .source
                .as_ref()
                .ok_or(Error::Invalid("textbox active source range"))?;
            if range.start_cp < previous_end || range.end_cp <= range.start_cp || range.end_cp > end
            {
                return Err(Error::Invalid("textbox flow source ranges"));
            }
            previous_end = range.end_cp;
            covered += range.end_cp - range.start_cp;
            let bytes = boundaries
                .get(&range.start_cp)
                .zip(boundaries.get(&range.end_cp))
                .map(|(&a, &b)| [a, b]);
            let mut reasons = Vec::new();
            if bytes.is_none() {
                reasons.push("projection_boundary_unresolved");
            }
            let shape = record.anchor_index.and_then(|i| part.shapes.get(i));
            let mut supported = bytes.is_some();
            if record.chain_shape_count != Some(1) {
                reasons.push("linked_shape_chain_unresolved");
                supported = false;
            }
            let placement = if let Some(shape) = shape {
                if counts.get(&record.anchor_index.unwrap()) != Some(&1) {
                    reasons.push("shared_shape_anchor");
                    supported = false;
                }
                let offset = shape.source_cp.checked_sub(owner.start_cp);
                if offset.and_then(|i| owner_units.get(i).copied()) != Some(8) {
                    return Err(Error::Invalid("textbox flow source anchor"));
                }
                let span = revisions
                    .spans
                    .get(
                        revisions
                            .spans
                            .partition_point(|s| s.end_cp <= shape.source_cp),
                    )
                    .filter(|s| s.start_cp <= shape.source_cp);
                match span {
                    Some(s)
                        if s.properties.special == Some(true)
                            && s.properties.deleted == Some(false)
                            && s.properties.field_hidden == Some(false) => {}
                    _ => {
                        reasons.push("anchor_special_or_revision_visibility_unresolved");
                        supported = false;
                    }
                }
                Some(placement(shape))
            } else {
                reasons.push("shape_hierarchy_unresolved");
                supported = false;
                None
            };
            reasons.push("hidden_text_visibility_unresolved");
            reasons.push("visual_placement_and_reading_order_unresolved");
            active.push(FlowEntry {
                record_index: record.record_index,
                source_range: [range.start_cp, range.end_cp],
                projected_bytes: bytes,
                anchor_cp: shape.map(|s| s.source_cp),
                source_association_supported: supported,
                placement,
                unresolved: reasons,
            });
        }
        let mut sequence: Vec<_> = active
            .iter()
            .enumerate()
            .filter(|(_, e)| e.source_association_supported)
            .map(|(i, _)| i)
            .collect();
        sequence.sort_by_key(|&i| active[i].anchor_cp);
        parts.push(FlowPart {
            story: part.text_story,
            owner_story: part.anchor_story,
            active_ranges: active,
            reusable_records: reusable,
            source_anchor_sequence: sequence,
            source_units_outside_active_ranges: raw.utf16_units - covered,
        });
    }
    Ok(TextboxFlowPlan {
        status: "source_association_plan_visual_order_unresolved",
        parts,
        unresolved: [
            "hidden text and final display visibility",
            "OfficeArt placement overrides and page layout",
            "linked chains, wrapping and visual reading order",
        ],
    })
}

fn unique<'a, T>(mut values: impl Iterator<Item = &'a T>) -> Result<&'a T> {
    let value = values
        .next()
        .ok_or(Error::Invalid("textbox flow missing story"))?;
    if values.next().is_some() {
        return Err(Error::Invalid("textbox flow duplicate story"));
    }
    Ok(value)
}

fn validate_projection(story: &TranscribedStory, start: usize, end: usize) -> Result<()> {
    let (mut cp, mut byte) = (start, 0);
    for s in &story.segments {
        if s.start_cp != cp
            || s.end_cp < cp
            || s.output_start != byte
            || story.text.get(s.output_start..s.output_end).is_none()
        {
            return Err(Error::Invalid("textbox flow projected partition"));
        }
        cp = s.end_cp;
        byte = s.output_end;
    }
    if cp != end || byte != story.text.len() {
        return Err(Error::Invalid("textbox flow projected coverage"));
    }
    Ok(())
}

// Scan each projected segment at most twice for all requested boundaries together.
// First occurrence wins at a shared CP, assigning generated labels to the following range.
fn projection_boundaries(
    story: &TranscribedStory,
    requests: &BTreeSet<usize>,
) -> BTreeMap<usize, usize> {
    let mut result = BTreeMap::new();
    for s in &story.segments {
        if requests.contains(&s.start_cp) {
            result.entry(s.start_cp).or_insert(s.output_start);
        }
        if requests.contains(&s.end_cp) {
            result.entry(s.end_cp).or_insert(s.output_end);
        }
        if s.start_cp == s.end_cp {
            continue;
        }
        let mut wanted = requests
            .range((
                std::ops::Bound::Excluded(s.start_cp),
                std::ops::Bound::Excluded(s.end_cp),
            ))
            .peekable();
        if wanted.peek().is_none() || !matches!(s.kind, "text" | "paragraph" | "tab" | "line_break")
        {
            continue;
        }
        let text = &story.text[s.output_start..s.output_end];
        if text.encode_utf16().count() != s.end_cp - s.start_cp {
            continue;
        }
        let mut cp = s.start_cp;
        for (byte, ch) in text.char_indices() {
            while wanted.peek().is_some_and(|&&target| target < cp) {
                wanted.next();
            }
            if wanted.peek().is_some_and(|&&target| target == cp) {
                result.insert(cp, s.output_start + byte);
                wanted.next();
            }
            if wanted.peek().is_none() {
                break;
            }
            cp += ch.len_utf16();
        }
    }
    result
}

#[cfg(test)]
fn projection_range(story: &TranscribedStory, start: usize, end: usize) -> Option<[usize; 2]> {
    let boundaries = projection_boundaries(story, &BTreeSet::from([start, end]));
    Some([*boundaries.get(&start)?, *boundaries.get(&end)?])
}

// MS-DOC 2.9.253: Spa origin/wrapping flags describe coordinate references, not page positions.
fn placement(shape: &ShapeAnchor) -> Placement {
    let f = shape.raw_flags;
    let wrapping = (f >> 5) & 15;
    Placement {
        rectangle_twips: shape.rectangle_twips,
        horizontal_origin: (f >> 1) & 3,
        vertical_origin: (f >> 3) & 3,
        wrapping,
        wrapping_side: if wrapping == 1 || wrapping == 3 {
            None
        } else {
            Some((f >> 9) & 15)
        },
        behind_text: if wrapping == 3 {
            Some(f & (1 << 14) != 0)
        } else {
            None
        },
        anchor_locked: f & (1 << 15) != 0,
        reading_order: "unresolved",
    }
}

#[cfg(test)]
mod tests;
