//! A source-accounting view over existing extraction, independent of page layout.
use crate::office::doc::{Error, Inspection, Result, Transcription};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ExtractionSummary {
    pub schema_version: u32,
    pub fidelity: &'static str,
    pub raw_text_field: &'static str,
    pub projected_text_field: &'static str,
    pub visibility_evidence_field: &'static str,
    pub range_semantics: &'static str,
    pub layout: &'static str,
    pub source_retention: &'static str,
    pub trailing_source_units: usize,
    pub unresolved: Vec<&'static str>,
    pub stories: Vec<StorySummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supplementary_geometry_text: Option<GeometryTextSummary>,
}
#[derive(Debug, Serialize)]
pub struct GeometryTextSummary {
    pub source_text_field: &'static str,
    pub source_byte_ownership_field: &'static str,
    pub shapes_with_stored_text: usize,
    pub stored_text_candidates: usize,
    pub stored_utf16_units: usize,
    pub display_status: &'static str,
    pub accounting: &'static str,
}
impl ExtractionSummary {
    /// Supplementary strings retain stream-byte ownership outside CLX story partitions.
    /// Their presence establishes source text, independently of displayed content or loss.
    pub fn attach_geometry_text(
        &mut self,
        geometry: &crate::office::doc::geometry_text::GeometryText,
    ) {
        let shapes = geometry
            .shapes
            .iter()
            .filter(|s| s.candidates.iter().any(|p| !p.text.is_empty()))
            .count();
        if shapes == 0 {
            return;
        }
        let candidates: Vec<_> = geometry
            .shapes
            .iter()
            .flat_map(|s| &s.candidates)
            .filter(|p| !p.text.is_empty())
            .collect();
        self.supplementary_geometry_text = Some(GeometryTextSummary {
            source_text_field: "geometry_text.shapes[].source_text",
            source_byte_ownership_field: "geometry_text.shapes[].candidates[].payload_table_range",
            shapes_with_stored_text: shapes,
            stored_text_candidates: candidates.len(),
            stored_utf16_units: candidates.iter().map(|p| p.utf16_units).sum(),
            display_status: "visibility_and_reading_order_unresolved",
            accounting: "supplementary_stream_text_not_story_CP_or_definite_missing_text",
        });
        let code = "geometry_text_visibility_and_reading_order_unresolved";
        if !self.unresolved.contains(&code) {
            self.unresolved.push(code);
        }
    }
}
#[derive(Debug, Serialize)]
pub struct StorySummary {
    pub kind: &'static str,
    pub source_range: [usize; 2],
    pub raw_utf16_units: usize,
    pub projected_utf8_bytes: usize,
    pub projected_unicode_scalars: usize,
    pub source_partition_complete: bool,
    pub suppressed_source_units: usize,
    pub unresolved_source_units: usize,
    pub suppressed: Vec<SourceRange>,
    pub unresolved_ranges: Vec<SourceRange>,
    pub generated: Vec<SourceRange>,
    pub diagnostics: Vec<Diagnostic>,
}
#[derive(Debug, Serialize)]
pub struct SourceRange {
    pub kind: &'static str,
    pub source_range: [usize; 2],
    pub output_bytes: [usize; 2],
}
#[derive(Debug, Serialize)]
pub struct Diagnostic {
    pub code: &'static str,
    pub source_cp: usize,
}

/// Summarizes the final logical projection without requiring render qualification.
/// Suppressed ranges retain their exact segment reason and raw source in `stories`.
/// Counts describe accounting, not extraction accuracy or visible reading order.
pub fn summarize(source: &Inspection, projected: &Transcription) -> Result<ExtractionSummary> {
    if source.stories.len() != projected.stories.len() {
        return Err(Error::Invalid("extraction story cardinality"));
    }
    let mut stories = Vec::with_capacity(source.stories.len());
    for (raw, text) in source.stories.iter().zip(&projected.stories) {
        let end = raw
            .start_cp
            .checked_add(raw.utf16_units)
            .ok_or(Error::Limit("extraction source range"))?;
        if raw.kind != text.kind || raw.text.encode_utf16().count() != raw.utf16_units {
            return Err(Error::Invalid("extraction source story"));
        }
        let mut story = StorySummary {
            kind: raw.kind,
            source_range: [raw.start_cp, end],
            raw_utf16_units: raw.utf16_units,
            projected_utf8_bytes: text.text.len(),
            projected_unicode_scalars: text.text.chars().count(),
            source_partition_complete: true,
            suppressed_source_units: 0,
            unresolved_source_units: 0,
            suppressed: vec![],
            unresolved_ranges: vec![],
            generated: vec![],
            diagnostics: text
                .diagnostics
                .iter()
                .map(|d| Diagnostic {
                    code: d.code,
                    source_cp: d.cp,
                })
                .collect(),
        };
        let mut cp = raw.start_cp;
        let mut output = 0;
        for segment in &text.segments {
            if segment.start_cp != cp
                || segment.end_cp < cp
                || segment.end_cp > end
                || segment.output_start != output
                || segment.output_end < output
                || !text.text.is_char_boundary(segment.output_start)
                || !text.text.is_char_boundary(segment.output_end)
            {
                return Err(Error::Invalid("extraction segment partition"));
            }
            let range = || SourceRange {
                kind: segment.kind,
                source_range: [segment.start_cp, segment.end_cp],
                output_bytes: [segment.output_start, segment.output_end],
            };
            let units = segment.end_cp - segment.start_cp;
            if units == 0 {
                story.generated.push(range());
            } else if segment.output_start == segment.output_end {
                story.suppressed_source_units += units;
                story.suppressed.push(range());
            }
            if segment.kind.contains("unresolved") || segment.kind == "unprojected_tail" {
                story.unresolved_source_units += units;
                story.unresolved_ranges.push(range());
            }
            cp = segment.end_cp;
            output = segment.output_end;
        }
        if cp != end || output != text.text.len() {
            return Err(Error::Invalid("extraction incomplete partition"));
        }
        stories.push(story);
    }
    Ok(ExtractionSummary {
        schema_version: 1,
        fidelity: "partial_logical_extraction",
        raw_text_field: "stories[].text",
        projected_text_field: "transcription.stories[].text",
        visibility_evidence_field: "hidden_properties.spans",
        range_semantics: "segment_accounting_not_visibility_or_feature_completeness",
        layout: "not_established",
        source_retention: "raw_stories_and_trailing_utf16_units",
        trailing_source_units: source.trailing_utf16_units.len(),
        unresolved: projected.unresolved.clone(),
        stories,
        supplementary_geometry_text: None,
    })
}
#[cfg(test)]
mod tests;
