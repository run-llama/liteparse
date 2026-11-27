use crate::office::doc::{
    Error, Result, Story, fields::FieldRange, hidden_visibility::HiddenEvidence,
    visibility::RevisionEvidence,
};

/// Empty EQ caches retain a source separator; nonempty results remain the cached view.
pub(crate) fn validate_layout(field: &FieldRange, story: &Story, units: &[u16]) -> Result<()> {
    if field.source.start < story.start_cp
        || field.source.start >= field.source.end
        || field.source.start.checked_add(1) != Some(field.instruction.start)
    {
        return Err(Error::Invalid("literal field indexed ownership"));
    }
    let end = field
        .source
        .end
        .checked_sub(story.start_cp)
        .ok_or(Error::Invalid("literal field end"))?;
    if end > units.len()
        || units.get(field.source.start - story.start_cp) != Some(&0x13)
        || units.get(end - 1) != Some(&0x15)
    {
        return Err(Error::Invalid("literal field delimiter units"));
    }
    match &field.result {
        None if field.instruction.end.checked_add(1) == Some(field.source.end) => {}
        Some(result) if result.start == result.end => {
            if field.instruction.end.checked_add(1) != Some(result.start)
                || result.end.checked_add(1) != Some(field.source.end)
                || units.get(
                    field
                        .instruction
                        .end
                        .checked_sub(story.start_cp)
                        .ok_or(Error::Invalid("literal field separator"))?,
                ) != Some(&0x14)
            {
                return Err(Error::Invalid("literal field empty result topology"));
            }
        }
        Some(_) => return Err(Error::Unsupported("literal field nonempty cached result")),
        _ => return Err(Error::Invalid("literal field instruction topology")),
    }
    Ok(())
}
pub(crate) fn visible(
    field: &FieldRange,
    revisions: &RevisionEvidence,
    hidden: &HiddenEvidence,
    budget: &mut usize,
) -> Result<()> {
    let mut cp = field.source.start;
    while cp < field.source.end {
        *budget = budget
            .checked_sub(1)
            .ok_or(Error::Limit("literal field visibility work"))?;
        let span = revisions
            .spans
            .get(revisions.spans.partition_point(|s| s.end_cp <= cp))
            .filter(|s| s.start_cp <= cp && cp < s.end_cp)
            .ok_or(Error::Invalid("literal field revision coverage"))?;
        if span.properties.deleted != Some(false) || !span.unresolved.is_empty() {
            return Err(Error::Unsupported("literal field accepted visibility"));
        }
        cp = span.end_cp.min(field.source.end);
    }
    let mut markers = vec![field.source.start, field.source.end - 1];
    if field.result.is_some() {
        markers.push(field.instruction.end);
    }
    for cp in markers {
        let span = revisions
            .spans
            .get(revisions.spans.partition_point(|s| s.end_cp <= cp))
            .filter(|s| s.start_cp <= cp && cp < s.end_cp)
            .ok_or(Error::Invalid("literal field delimiter coverage"))?;
        if span.properties.special != Some(true) || span.properties.field_hidden != Some(false) {
            return Err(Error::Unsupported("literal field delimiter properties"));
        }
    }
    let mut cp = field.source.start;
    while cp < field.source.end {
        *budget = budget
            .checked_sub(1)
            .ok_or(Error::Limit("literal field visibility work"))?;
        let span = hidden
            .spans
            .get(hidden.spans.partition_point(|s| s.end_cp <= cp))
            .filter(|s| s.start_cp <= cp && cp < s.end_cp)
            .ok_or(Error::Invalid("literal field hidden coverage"))?;
        if span.property.hidden != Some(false) || !span.property.unresolved.is_empty() {
            return Err(Error::Unsupported("literal field hidden visibility"));
        }
        cp = span.end_cp.min(field.source.end);
    }
    Ok(())
}

pub(crate) fn plain(
    range: [usize; 2],
    revisions: &RevisionEvidence,
    budget: &mut usize,
) -> Result<()> {
    let mut cp = range[0];
    while cp < range[1] {
        *budget = budget
            .checked_sub(1)
            .ok_or(Error::Limit("literal field component work"))?;
        let span = revisions
            .spans
            .get(revisions.spans.partition_point(|s| s.end_cp <= cp))
            .filter(|s| s.start_cp <= cp && cp < s.end_cp)
            .ok_or(Error::Invalid("literal field component coverage"))?;
        if span.properties.special != Some(false) {
            return Err(Error::Unsupported(
                "literal field component special characters",
            ));
        }
        cp = span.end_cp.min(range[1]);
    }
    Ok(())
}
