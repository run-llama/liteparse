//! Comment appendix ownership retains source text, anchor metadata and revision evidence.
use crate::office::doc::{Error, Inspection, Result, comments::CommentInspection};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct CommentDocument {
    pub plan: super::notes::FootnoteDocument,
    pub ownership: CommentInspection,
}

pub fn inspect(
    data: &[u8],
    source: &Inspection,
) -> Result<Option<(super::notes::FootnoteDocument, CommentInspection)>> {
    let story = source
        .stories
        .iter()
        .find(|s| s.kind == "comments")
        .ok_or(Error::Invalid("comment render story"))?;
    if story.utf16_units == 0 {
        return Ok(None);
    }
    if story.utf16_units > 1024 * 1024 {
        return Err(Error::Limit("comment render source"));
    }
    let formatting = crate::office::doc::formatting_inspection::inspect_character_formatting(data)?;
    let revisions = crate::office::doc::visibility::resolve_revision_evidence(&formatting)?;
    let ownership = crate::office::doc::comments::inspect_comments(data, source, Some(&revisions))?;
    prepare(source, ownership).map(Some)
}

fn prepare(
    source: &Inspection,
    ownership: CommentInspection,
) -> Result<(super::notes::FootnoteDocument, CommentInspection)> {
    let story = source
        .stories
        .iter()
        .find(|s| s.kind == "comments")
        .ok_or(Error::Invalid("comment render story"))?;
    let body = source
        .stories
        .first()
        .filter(|s| s.kind == "body" && s.start_cp == 0)
        .ok_or(Error::Invalid("comment render body"))?;
    let end = story
        .start_cp
        .checked_add(story.utf16_units)
        .ok_or(Error::Limit("comment render CP"))?;
    let raw: Vec<_> = story.text.encode_utf16().collect();
    let body_raw: Vec<_> = body.text.encode_utf16().collect();
    if raw.len() != story.utf16_units
        || body_raw.len() != body.utf16_units
        || ownership.source.start_cp != story.start_cp
        || ownership.source.end_cp != end
        || ownership.comments.len() > 4096
    {
        return Err(Error::Invalid("comment render source identity"));
    }
    let mut cursor = story.start_cp;
    let mut references = std::collections::BTreeSet::new();
    for (index, comment) in ownership.comments.iter().enumerate() {
        if comment.index != index
            || comment.source.start_cp != cursor
            || comment.source.end_cp <= cursor
            || comment.source.end_cp > end
            || comment.initial_marker.source_cp != cursor
            || !references.insert(comment.body_reference.source_cp)
            || usize::from(comment.author_index) >= ownership.authors.len()
        {
            return Err(Error::Invalid("comment render partition"));
        }
        for marker in [&comment.body_reference, &comment.initial_marker] {
            let expected = crate::office::doc::visibility::RevisionProperties {
                deleted: Some(false),
                inserted: Some(false),
                field_hidden: Some(false),
                special: Some(true),
            };
            if marker.formatting_unresolved
                || marker.properties != Some(expected)
                || marker.source_unit != 5
            {
                return Err(Error::Unsupported("comment render marker properties"));
            }
        }
        if body_raw.get(comment.body_reference.source_cp) != Some(&5)
            || raw.get(cursor - story.start_cp) != Some(&5)
            || raw.get(comment.source.end_cp - story.start_cp - 1) != Some(&13)
        {
            return Err(Error::Invalid("comment render source markers"));
        }
        if comment.extended.as_ref().is_some_and(|e| e.ink)
            || comment
                .anchor
                .is_some_and(|a| a.start_cp > a.end_cp || a.end_cp > body.utf16_units)
        {
            return Err(Error::Unsupported("comment render ink or anchor"));
        }
        cursor = comment.source.end_cp;
    }
    if ownership
        .terminal_source
        .is_none_or(|t| t.start_cp != cursor || t.end_cp != end)
        || cursor + 1 != end
        || raw.last() != Some(&13)
    {
        return Err(Error::Invalid("comment render terminal partition"));
    }
    let notes = ownership
        .comments
        .iter()
        .map(|c| super::notes::Footnote {
            id: (c.index + 1) as i64,
            body_cp: c.body_reference.source_cp,
            raw_reference_index: 0,
            section_index: 0,
            marker_status: crate::office::doc::notes::FootnoteMarkerStatus::AutomaticSpecial,
            source_cp: [c.source.start_cp, c.source.end_cp],
            marker_cp: Some(c.initial_marker.source_cp),
            authored_reference: None,
            label: format!("[C{}]", c.index + 1),
            paragraphs: Vec::new(),
        })
        .collect();
    Ok((
        super::notes::FootnoteDocument {
            kind: super::notes::NoteKind::Comment,
            source_cp: [story.start_cp, end],
            source_text: story.text.clone(),
            terminal_guard: [cursor, end],
            position: Some(crate::office::doc::notes::NotePosition::SectionEnd),
            position_basis: Some("partial_comments_appended_in_source_order"),
            sections: Vec::new(),
            coexisting_endnotes: Vec::new(),
            coexisting_endnote_source: None,
            notes,
            separators: Vec::new(),
        },
        ownership,
    ))
}

impl CommentDocument {
    pub(super) fn validate_ownership(&self) -> Result<()> {
        if self.plan.kind != super::notes::NoteKind::Comment
            || self.plan.source_cp != [self.ownership.source.start_cp, self.ownership.source.end_cp]
            || self.plan.notes.len() != self.ownership.comments.len()
        {
            return Err(Error::Invalid("comment appendix ownership"));
        }
        for (note, source) in self.plan.notes.iter().zip(&self.ownership.comments) {
            if note.id != (source.index + 1) as i64
                || note.label != format!("[C{}]", source.index + 1)
                || note.body_cp != source.body_reference.source_cp
                || note.marker_cp != Some(source.initial_marker.source_cp)
                || note.source_cp != [source.source.start_cp, source.source.end_cp]
            {
                return Err(Error::Invalid("comment appendix attribution"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
