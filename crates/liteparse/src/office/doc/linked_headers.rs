//! Logical header/footer text joined to source slots and section inheritance.
use crate::office::doc::{Error, Inspection, Result, Transcription, headers::HeaderInspection};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct LinkedHeaders {
    pub status: &'static str,
    pub page_instances: &'static str,
    pub different_even_and_odd_pages: Option<bool>,
    pub source_relationship_unresolved: Vec<&'static str>,
    pub contents: Vec<HeaderContent>,
    pub sections: Vec<SectionReferences>,
}
#[derive(Debug, Serialize)]
pub struct HeaderContent {
    pub story_index: usize,
    pub role: &'static str,
    pub source_cp: [usize; 2],
    pub explicit_blank: bool,
    pub logical_utf8_range: Option<[usize; 2]>,
    pub text: Option<String>,
    pub unresolved: Vec<&'static str>,
}
#[derive(Debug, Serialize)]
pub struct SectionReferences {
    pub section_index: usize,
    pub body_source_cp: [usize; 2],
    pub different_first_page: Option<bool>,
    pub references: Vec<ContentReference>,
    pub unresolved: Vec<&'static str>,
}
#[derive(Debug, Serialize)]
pub struct ContentReference {
    pub role: &'static str,
    pub status: &'static str,
    pub origin_story_index: Option<usize>,
    pub origin_section_index: Option<usize>,
}

/// Expose each owned logical header/footer once, retaining inheritance by slot ID.
/// Note separators stay in existing raw headers metadata. Content ranges exclude
/// storage guards; selection of first/even/odd variants still requires page context.
pub fn extract(
    source: &Inspection,
    transcription: &Transcription,
    headers: &HeaderInspection,
    hidden: Option<&crate::office::doc::hidden_visibility::HiddenEvidence>,
) -> Result<LinkedHeaders> {
    crate::office::doc::extraction_summary::summarize(source, transcription)?;
    if headers.stories.len() > 49_158 || headers.sections.len() > 8192 {
        return Err(Error::Limit("linked header source metadata"));
    }
    let raw = source
        .stories
        .iter()
        .find(|s| s.kind == "headers")
        .ok_or(Error::Invalid("linked header raw story"))?;
    let projected = transcription
        .stories
        .iter()
        .find(|s| s.kind == "headers")
        .ok_or(Error::Invalid("linked header logical story"))?;
    let end = raw
        .start_cp
        .checked_add(raw.utf16_units)
        .ok_or(Error::Limit("linked header range"))?;
    if headers.source.start_cp != raw.start_cp || headers.source.end_cp != end {
        return Err(Error::Invalid("linked header source identity"));
    }
    if let Some(h) = hidden
        && (h.spans.windows(2).any(|w| w[0].end_cp > w[1].start_cp)
            || h.spans.iter().any(|s| s.start_cp >= s.end_cp))
    {
        return Err(Error::Invalid("linked header visibility partition"));
    }
    let mut work = 8 * 1024 * 1024usize;
    let mut copied = 0usize;
    let mut contents = Vec::new();
    let mut ids = std::collections::BTreeSet::new();
    let mut prior_end = raw.start_cp;
    for (index, slot) in headers.stories.iter().enumerate() {
        if slot.index != index
            || slot.source.start_cp < prior_end
            || slot.source.end_cp < slot.source.start_cp
            || slot.source.end_cp > end
        {
            return Err(Error::Invalid("linked header slot order"));
        }
        prior_end = slot.source.end_cp;
        let Some(role) = slot.role.filter(|r| is_role(r)) else {
            continue;
        };
        let Some(range) = slot.content else { continue };
        let [a, b] = [range.start_cp, range.end_cp];
        if a < slot.source.start_cp || b > slot.source.end_cp || a >= b {
            return Err(Error::Invalid("linked header content bounds"));
        }
        ids.insert(slot.index);
        let mut unresolved = slot.unresolved.clone();
        let range = match projected_range(projected, a, b) {
            Ok(value) => Some(value),
            Err(Error::Unsupported(_)) => {
                unresolved.push("logical_projection_boundary_unresolved");
                None
            }
            Err(e) => return Err(e),
        };
        let text = if let Some([lo, hi]) = range {
            let text = projected
                .text
                .get(lo..hi)
                .ok_or(Error::Invalid("linked header UTF8 range"))?;
            copied = copied
                .checked_add(text.len())
                .ok_or(Error::Limit("linked header text"))?;
            if copied > 64 * 1024 * 1024 {
                return Err(Error::Limit("linked header text"));
            }
            Some(text.to_owned())
        } else {
            None
        };
        let mut covered = a;
        if let Some(h) = hidden {
            let first = h.spans.partition_point(|s| s.end_cp <= a);
            for span in h.spans[first..].iter().take_while(|s| s.start_cp < b) {
                work = work
                    .checked_sub(1)
                    .ok_or(Error::Limit("linked header work"))?;
                if span.start_cp > covered || span.property.hidden.is_none() {
                    unresolved.push("hidden_visibility_unresolved");
                }
                if span.property.hidden == Some(true) {
                    unresolved.push("contains_source_hidden_text");
                }
                covered = covered.max(span.end_cp.min(b));
            }
        }
        if covered < b {
            unresolved.push("hidden_visibility_unresolved");
        }
        work = work
            .checked_sub(projected.diagnostics.len())
            .ok_or(Error::Limit("linked header work"))?;
        if projected.diagnostics.iter().any(|d| a <= d.cp && d.cp < b) {
            unresolved.push("transcription_diagnostics_in_content");
        }
        unresolved.sort_unstable();
        unresolved.dedup();
        contents.push(HeaderContent {
            story_index: slot.index,
            role,
            source_cp: [a, b],
            explicit_blank: slot.explicit_blank,
            logical_utf8_range: range,
            text,
            unresolved,
        });
    }
    let body = source
        .stories
        .iter()
        .find(|s| s.kind == "body")
        .ok_or(Error::Invalid("linked header body story"))?;
    let body_end = body
        .start_cp
        .checked_add(body.utf16_units)
        .ok_or(Error::Limit("linked header body range"))?;
    let mut section_end = body.start_cp;
    let mut latest_origins = std::collections::BTreeMap::new();
    let mut sections = Vec::new();
    for (index, section) in headers.sections.iter().enumerate() {
        if section.index != index || section.references.len() > 6 {
            return Err(Error::Invalid("linked header section identity"));
        }
        if section.source.start_cp != section_end
            || section.source.end_cp <= section.source.start_cp
            || section.source.end_cp > body_end
        {
            return Err(Error::Invalid("linked header body section partition"));
        }
        section_end = section.source.end_cp;
        let mut roles = std::collections::BTreeSet::new();
        let mut refs = Vec::new();
        let mut unresolved = section.unresolved.clone();
        if section.references.len() != 6 {
            unresolved.push("section_roles_incomplete");
        }
        for reference in &section.references {
            if !is_role(reference.role) || !roles.insert(reference.role) {
                return Err(Error::Invalid("linked header reference role"));
            }
            if reference.origin_story_index.is_some() != reference.origin_section_index.is_some() {
                return Err(Error::Invalid("linked header origin presence"));
            }
            let local = reference
                .local_story_index
                .map(|id| {
                    headers
                        .stories
                        .get(id)
                        .filter(|slot| {
                            slot.role == Some(reference.role) && slot.section_index == Some(index)
                        })
                        .ok_or(Error::Invalid("linked header local slot"))
                })
                .transpose()?;
            let local_present = local.is_some_and(|slot| slot.source.start_cp < slot.source.end_cp);
            if local_present {
                latest_origins.insert(reference.role, reference.local_story_index.unwrap());
            }
            if reference.status != "unresolved_slot_count"
                && reference.origin_story_index != latest_origins.get(reference.role).copied()
            {
                return Err(Error::Invalid("linked header latest origin"));
            }
            let valid_status = match reference.status {
                "local" => {
                    local_present && reference.origin_story_index == reference.local_story_index
                }
                "inherited" => {
                    !local_present && reference.origin_section_index.is_some_and(|i| i < index)
                }
                "absent" => !local_present && reference.origin_story_index.is_none(),
                "unresolved_slot_count" => {
                    reference.local_story_index.is_none() && reference.origin_story_index.is_none()
                }
                "unresolved_content" => {
                    reference.origin_story_index.is_some_and(|id| {
                        headers
                            .stories
                            .get(id)
                            .is_some_and(|slot| !slot.unresolved.is_empty())
                    }) && (!local_present
                        || reference.origin_story_index == reference.local_story_index)
                }
                _ => false,
            };
            if !valid_status {
                return Err(Error::Invalid("linked header reference status"));
            }
            if let Some(origin) = reference.origin_story_index {
                let slot = headers
                    .stories
                    .get(origin)
                    .ok_or(Error::Invalid("linked header origin"))?;
                if slot.role != Some(reference.role)
                    || slot.section_index != reference.origin_section_index
                    || slot.section_index.is_none_or(|i| i > index)
                {
                    return Err(Error::Invalid("linked header origin identity"));
                }
                if !ids.contains(&origin) {
                    unresolved.push("referenced_content_unavailable");
                }
            }
            refs.push(ContentReference {
                role: reference.role,
                status: reference.status,
                origin_story_index: reference.origin_story_index,
                origin_section_index: reference.origin_section_index,
            });
        }
        unresolved.sort_unstable();
        unresolved.dedup();
        sections.push(SectionReferences {
            section_index: index,
            body_source_cp: [section.source.start_cp, section.source.end_cp],
            different_first_page: section.different_first_page,
            references: refs,
            unresolved,
        });
    }
    if section_end != body_end {
        return Err(Error::Invalid("linked header incomplete body partition"));
    }
    let mut unresolved = headers.unresolved.to_vec();
    unresolved.extend(&headers.diagnostics);
    unresolved.sort_unstable();
    unresolved.dedup();
    Ok(LinkedHeaders {
        status: "linked_logical_header_footer_content",
        page_instances: "not_established",
        different_even_and_odd_pages: headers.different_even_and_odd_pages,
        source_relationship_unresolved: unresolved,
        contents,
        sections,
    })
}
fn is_role(role: &str) -> bool {
    matches!(
        role,
        "odd_header"
            | "even_header"
            | "first_header"
            | "odd_footer"
            | "even_footer"
            | "first_footer"
    )
}
// Only one-CP paragraph marks can be split inside a coalesced segment. Other
// source-to-output transformations keep complete segment ownership.
fn boundary(s: &crate::office::doc::SourceSegment, cp: usize, text: &str) -> Result<usize> {
    if cp == s.start_cp {
        return Ok(s.output_start);
    }
    if cp == s.end_cp {
        return Ok(s.output_end);
    }
    if s.start_cp < cp && cp < s.end_cp && s.kind == "paragraph" {
        let value = text
            .get(s.output_start..s.output_end)
            .ok_or(Error::Invalid("linked header paragraph"))?;
        if value.len() == s.end_cp - s.start_cp && value.bytes().all(|b| b == b'\n') {
            return Ok(s.output_start + cp - s.start_cp);
        }
    }
    Err(Error::Unsupported("linked header projection boundary"))
}
fn projected_range(
    story: &crate::office::doc::TranscribedStory,
    a: usize,
    b: usize,
) -> Result<[usize; 2]> {
    let start = story
        .segments
        .partition_point(|s| s.end_cp <= a && s.start_cp < a);
    let first = story
        .segments
        .get(start)
        .ok_or(Error::Invalid("linked header projection start"))?;
    let lo = boundary(first, a, &story.text)?;
    let mut hi = lo;
    let mut cp = a;
    for segment in &story.segments[start..] {
        if segment.start_cp >= b {
            break;
        }
        if segment.start_cp > cp || segment.end_cp < cp {
            return Err(Error::Invalid("linked header projection gap"));
        }
        cp = segment.end_cp.min(b);
        hi = boundary(segment, cp, &story.text)?;
        if cp == b {
            break;
        }
    }
    if cp != b {
        return Err(Error::Invalid("linked header projection coverage"));
    }
    Ok([lo, hi])
}

#[cfg(test)]
mod tests;
