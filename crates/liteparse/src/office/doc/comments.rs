//! Source comment bodies, reference markers and annotation bookmark ownership.
use crate::office::doc::{
    Error, Inspection, Result, bytes,
    compound::Compound,
    document::fib,
    headers::SourceRange,
    u16_at, u32_at,
    visibility::{RevisionEvidence, RevisionProperties},
};
use serde::Serialize;
use std::collections::BTreeMap;

const MAX_COMMENTS: usize = 65_536;
const MAX_BOOKMARKS: usize = 0x3ffb;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_UNITS: usize = 16 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct CommentInspection {
    pub status: &'static str,
    pub binary_version: u16,
    pub source: SourceRange,
    pub authors: Vec<Author>,
    pub comments: Vec<Comment>,
    pub bookmarks: Vec<AnnotationBookmark>,
    pub terminal_source: Option<SourceRange>,
    pub ignored_reference_cp: Option<u32>,
    pub ignored_text_cp: Option<u32>,
    pub ignored_bookmark_start_cp: Option<u32>,
    pub ignored_bookmark_end_cp: Option<u32>,
    pub diagnostics: Vec<&'static str>,
    pub unresolved: [&'static str; 4],
}
#[derive(Debug, Serialize)]
pub struct Author {
    pub utf16: Vec<u16>,
    pub text: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct MarkerEvidence {
    pub source_cp: usize,
    pub source_unit: u16,
    pub properties: Option<RevisionProperties>,
    pub formatting_unresolved: bool,
}
#[derive(Debug, Serialize)]
pub struct Comment {
    pub index: usize,
    pub body_reference: MarkerEvidence,
    pub source: SourceRange,
    pub initial_marker: MarkerEvidence,
    pub author_index: u16,
    pub initials_utf16: Vec<u16>,
    pub initials: Option<String>,
    pub raw_initials_length: u16,
    pub raw_unused: [u16; 2],
    pub tag: u32,
    pub anchor_status: &'static str,
    pub anchor: Option<SourceRange>,
    pub bookmark_index: Option<usize>,
    pub extended: Option<ExtendedComment>,
    pub unresolved: Vec<&'static str>,
}
#[derive(Debug, Serialize)]
pub struct AnnotationBookmark {
    pub index: usize,
    pub tag: u32,
    pub raw_old_tag: u32,
    pub raw_start_cp: u32,
    pub raw_end_cp: Option<u32>,
    pub end_index: u16,
    pub raw_flags: u16,
    pub source: Option<SourceRange>,
    pub unresolved: Vec<&'static str>,
}
#[derive(Debug, Serialize)]
pub struct ExtendedComment {
    pub raw_date_time: u32,
    pub depth: u32,
    pub parent_delta: i32,
    pub parent_index: Option<usize>,
    pub ink: bool,
    pub raw_flags: u32,
    pub unresolved: Vec<&'static str>,
}

/// Reads parallel comment PLCs and joins bookmark tags without moving source text.
/// Revision-deleted markers remain present with their source property evidence.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/486f5a89-fba5-412f-8ac6-1c551654ddcd>
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/f2327847-8ba3-4b9c-b9a3-b0bdfac1206c>
pub fn inspect_comments(
    data: &[u8],
    source: &Inspection,
    revisions: Option<&RevisionEvidence>,
) -> Result<CommentInspection> {
    let c = Compound::open(data)?;
    let word = c.stream("WordDocument")?;
    let (version, table_name, lengths, _, _) = fib(&word)?;
    validate_source(source, version, &lengths)?;
    let table = c.stream(table_name)?;
    let mut blocks = Vec::new();
    let mut budget = MAX_BYTES;
    for index in [4, 5, 36, 37, 42, 43, 112] {
        let block = fib_range(&word, &table, index)?;
        budget = budget
            .checked_sub(block.len())
            .ok_or(Error::Limit("comment metadata bytes"))?;
        blocks.push(block);
    }
    decode(source, revisions, &blocks)
}
fn fib_range<'a>(word: &[u8], table: &'a [u8], index: usize) -> Result<&'a [u8]> {
    if index >= u16_at(word, 152)? as usize {
        return Ok(&[]);
    }
    let len = u32_at(word, 158 + index * 8)? as usize;
    if len == 0 {
        return Ok(&[]);
    }
    bytes(table, u32_at(word, 154 + index * 8)? as usize, len)
}
fn validate_source(source: &Inspection, version: u16, lengths: &[usize; 8]) -> Result<()> {
    let names = [
        "body",
        "footnotes",
        "headers",
        "reserved_macro",
        "comments",
        "endnotes",
        "textboxes",
        "header_textboxes",
    ];
    if source.binary_version != version || source.stories.len() != 8 {
        return Err(Error::Invalid("comment source identity"));
    }
    let mut cp = 0usize;
    let mut byte_count = 0usize;
    for ((story, length), name) in source.stories.iter().zip(lengths).zip(names) {
        cp = cp
            .checked_add(*length)
            .ok_or(Error::Limit("comment source units"))?;
        byte_count = byte_count
            .checked_add(story.text.len())
            .ok_or(Error::Limit("comment source bytes"))?;
        if cp > MAX_UNITS || byte_count > 64 * 1024 * 1024 {
            return Err(Error::Limit("comment source size"));
        }
        if story.kind != name
            || story.start_cp != cp - length
            || story.utf16_units != *length
            || story.text.encode_utf16().count() != *length
        {
            return Err(Error::Invalid("comment source identity"));
        }
    }
    Ok(())
}
fn validate_revisions(revisions: Option<&RevisionEvidence>, total: usize) -> Result<()> {
    if let Some(revisions) = revisions {
        if revisions.spans.len() > 262_144 {
            return Err(Error::Limit("comment revision spans"));
        }
        let mut end = 0;
        for span in &revisions.spans {
            if span.start_cp < end || span.start_cp >= span.end_cp || span.end_cp > total {
                return Err(Error::Invalid("comment revision span order"));
            }
            end = span.end_cp;
        }
    }
    Ok(())
}
fn marker(cp: usize, unit: u16, revisions: Option<&RevisionEvidence>) -> MarkerEvidence {
    let span = revisions.and_then(|r| {
        let index = r.spans.partition_point(|s| s.end_cp <= cp);
        r.spans.get(index).filter(|s| s.start_cp <= cp)
    });
    MarkerEvidence {
        source_cp: cp,
        source_unit: unit,
        properties: span.map(|s| s.properties),
        formatting_unresolved: span.is_none_or(|s| !s.unresolved.is_empty()),
    }
}
fn boundary(units: &[u16], cp: usize) -> bool {
    cp <= units.len()
        && !(cp > 0
            && cp < units.len()
            && (0xd800..=0xdbff).contains(&units[cp - 1])
            && (0xdc00..=0xdfff).contains(&units[cp]))
}
fn decode(
    source: &Inspection,
    revisions: Option<&RevisionEvidence>,
    blocks: &[&[u8]],
) -> Result<CommentInspection> {
    let [refs, text, owners, tags, starts, ends, extra] = blocks else {
        return Err(Error::Invalid("comment block count"));
    };
    let body: Vec<_> = source.stories[0].text.encode_utf16().collect();
    let story = &source.stories[4];
    let units: Vec<_> = story.text.encode_utf16().collect();
    let total = source
        .stories
        .last()
        .map_or(0, |s| s.start_cp + s.utf16_units);
    validate_revisions(revisions, total)?;
    let mut output = CommentInspection {
        status: "source_comment_ownership",
        binary_version: source.binary_version,
        source: SourceRange {
            start_cp: story.start_cp,
            end_cp: story.start_cp + units.len(),
        },
        authors: authors(owners)?,
        comments: Vec::new(),
        bookmarks: Vec::new(),
        terminal_source: None,
        ignored_reference_cp: None,
        ignored_text_cp: None,
        ignored_bookmark_start_cp: None,
        ignored_bookmark_end_cp: None,
        diagnostics: Vec::new(),
        unresolved: [
            "annotation_bookmark_terminal_cp_conformance",
            "field_and_table_valid_selection_constraints",
            "comment_terminal_paragraph_table_depth",
            "visible_comment_placement_and_hidden_text",
        ],
    };
    let mut author_names = std::collections::BTreeSet::new();
    if output.authors.iter().any(|a| a.text.is_none()) {
        output.diagnostics.push("comment_author_encoding");
    }
    for author in &output.authors {
        if !author_names.insert(author.utf16.as_slice())
            && !output.diagnostics.contains(&"duplicate_comment_author")
        {
            output.diagnostics.push("duplicate_comment_author");
        }
    }
    let count = if refs.is_empty() {
        0
    } else {
        if refs.len() < 4 || (refs.len() - 4) % 34 != 0 {
            return Err(Error::Invalid("comment reference PLC size"));
        }
        output.ignored_reference_cp = Some(u32_at(refs, refs.len() / 34 * 4)?);
        (refs.len() - 4) / 34
    };
    if count > MAX_COMMENTS {
        return Err(Error::Limit("comment count"));
    }
    output.bookmarks = bookmarks(tags, starts, ends, &body)?;
    if !starts.is_empty() {
        output.ignored_bookmark_start_cp = Some(u32_at(starts, (output.bookmarks.len()) * 4)?);
    }
    if !ends.is_empty() {
        output.ignored_bookmark_end_cp = Some(u32_at(ends, ends.len() - 4)?);
    }
    if count == 0 && units.is_empty() && text.is_empty() {
        if !owners.is_empty() || !tags.is_empty() || !extra.is_empty() {
            output
                .diagnostics
                .push("comment_metadata_without_comment_text");
        }
        return Ok(output);
    }
    if text.len() != (count + 2) * 4
        || units.is_empty()
        || u32_at(text, 0)? != 0
        || u32_at(text, count * 4)? as usize != units.len() - 1
    {
        return Err(Error::Invalid("comment text PLC coverage"));
    }
    output.ignored_text_cp = Some(u32_at(text, (count + 1) * 4)?);
    output.terminal_source = Some(SourceRange {
        start_cp: story.start_cp + units.len() - 1,
        end_cp: story.start_cp + units.len(),
    });
    let mut by_tag = BTreeMap::new();
    for bookmark in &output.bookmarks {
        by_tag
            .entry(bookmark.tag)
            .and_modify(|v| *v = None)
            .or_insert(Some(bookmark.index));
    }
    let extended = if source.binary_version < 0x101 && !extra.is_empty() {
        output
            .diagnostics
            .push("comment_extended_version_unsupported");
        (0..count).map(|_| None).collect()
    } else {
        extended(extra, count, &mut output.diagnostics)?
    };
    let mut previous_cp = None;
    for (index, extra_record) in extended.into_iter().enumerate() {
        let cp = u32_at(refs, index * 4)? as usize;
        let start = u32_at(text, index * 4)? as usize;
        let end = u32_at(text, (index + 1) * 4)? as usize;
        if cp >= body.len()
            || previous_cp.is_some_and(|p| p >= cp)
            || start >= end
            || end >= units.len()
        {
            return Err(Error::Invalid("comment source range"));
        }
        previous_cp = Some(cp);
        let raw = bytes(refs, (count + 1) * 4 + index * 30, 30)?;
        let tag = u32_at(raw, 26)?;
        let author_index = u16_at(raw, 20)?;
        let initial_len = u16_at(raw, 0)?;
        let initials_utf16: Vec<_> = (0..initial_len.min(9) as usize)
            .map(|i| u16_at(raw, 2 + i * 2))
            .collect::<Result<_>>()?;
        let initials = (initial_len <= 9)
            .then(|| String::from_utf16(&initials_utf16).ok())
            .flatten();
        let mut comment = Comment {
            index,
            body_reference: marker(cp, body[cp], revisions),
            source: SourceRange {
                start_cp: story.start_cp + start,
                end_cp: story.start_cp + end,
            },
            initial_marker: marker(story.start_cp + start, units[start], revisions),
            author_index,
            initials_utf16,
            initials,
            raw_initials_length: initial_len,
            raw_unused: [u16_at(raw, 22)?, u16_at(raw, 24)?],
            tag,
            anchor_status: "unresolved_bookmark",
            anchor: None,
            bookmark_index: None,
            extended: extra_record,
            unresolved: Vec::new(),
        };
        if author_index as usize >= output.authors.len() {
            comment.unresolved.push("comment_author_index");
        }
        if comment.initials.is_none() {
            comment.unresolved.push("comment_initials_encoding");
        }
        if !boundary(&units, start) || !boundary(&units, end) {
            comment.unresolved.push("comment_surrogate_boundary");
        }
        if units[end - 1] != 13 {
            comment
                .unresolved
                .push("comment_terminal_paragraph_missing");
        }
        for (label, evidence) in [
            ("body_comment_marker", &comment.body_reference),
            ("comment_initial_marker", &comment.initial_marker),
        ] {
            if evidence.source_unit != 5
                || evidence.formatting_unresolved
                || evidence.properties.is_none_or(|p| p.special != Some(true))
            {
                comment.unresolved.push(label);
            }
        }
        if tag == u32::MAX {
            comment.anchor_status = "point_at_reference";
            comment.anchor = Some(SourceRange {
                start_cp: cp,
                end_cp: cp,
            });
        } else if let Some(Some(index)) = by_tag.get(&tag) {
            let bookmark = &output.bookmarks[*index];
            comment.bookmark_index = Some(*index);
            comment.anchor = bookmark.source;
            if bookmark.unresolved.is_empty() {
                comment.anchor_status = "bookmark_range";
            } else {
                comment.unresolved.push("comment_bookmark_constraints");
            }
        } else {
            comment
                .unresolved
                .push("comment_bookmark_tag_missing_or_ambiguous");
        }
        output.comments.push(comment);
    }
    Ok(output)
}
fn authors(raw: &[u8]) -> Result<Vec<Author>> {
    let mut out = Vec::new();
    let mut offset = 0;
    while offset < raw.len() {
        if out.len() >= 0x7fff {
            return Err(Error::Limit("comment authors"));
        }
        let len = u16_at(raw, offset)? as usize;
        if len >= 56 {
            return Err(Error::Invalid("comment author length"));
        }
        let data = bytes(raw, offset + 2, len * 2)?;
        let utf16: Vec<_> = data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let text = String::from_utf16(&utf16).ok();
        out.push(Author { utf16, text });
        offset += 2 + len * 2;
    }
    Ok(out)
}

/// ATNBE entries are parallel to bookmark starts; FBKF indices select the ends.
/// Annotation bookmark ranges belong to the main story; duplicate tags stay ambiguous.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/0c9df81f-98d0-454e-ad84-b612cd05b1a4>
fn bookmarks(
    tags: &[u8],
    starts: &[u8],
    ends: &[u8],
    body: &[u16],
) -> Result<Vec<AnnotationBookmark>> {
    if tags.is_empty()
        && (starts.is_empty() || starts.len() == 4)
        && (ends.is_empty() || ends.len() == 4)
    {
        return Ok(Vec::new());
    }
    let count = u16_at(tags, 2)? as usize;
    if count > MAX_BOOKMARKS {
        return Err(Error::Limit("comment bookmarks"));
    }
    if u16_at(tags, 0)? != u16::MAX
        || u16_at(tags, 4)? != 10
        || tags.len() != 6 + count * 12
        || starts.len() != 4 + count * 8
        || ends.len() != 4 + count * 4
    {
        return Err(Error::Invalid("comment bookmark tables"));
    }
    for plc in [starts, ends] {
        for i in 1..count {
            if u32_at(plc, (i - 1) * 4)? > u32_at(plc, i * 4)? {
                return Err(Error::Invalid("comment bookmark CP order"));
            }
        }
    }
    let mut out = Vec::new();
    let mut end_counts = BTreeMap::new();
    for i in 0..count {
        *end_counts
            .entry(u16_at(starts, (count + 1) * 4 + i * 4)?)
            .or_insert(0usize) += 1;
    }
    for index in 0..count {
        let at = 6 + index * 12;
        if u16_at(tags, at)? != 0 || u16_at(tags, at + 2)? != 0x100 {
            return Err(Error::Invalid("comment annotation bookmark class"));
        }
        let entry = (count + 1) * 4 + index * 4;
        let end_index = u16_at(starts, entry)?;
        let raw_start_cp = u32_at(starts, index * 4)?;
        let raw_end_cp = if (end_index as usize) < count {
            Some(u32_at(ends, end_index as usize * 4)?)
        } else {
            None
        };
        let flags = u16_at(starts, entry + 2)?;
        let mut unresolved = Vec::new();
        let source = raw_end_cp
            .filter(|end| raw_start_cp <= *end && *end as usize <= body.len())
            .map(|end| SourceRange {
                start_cp: raw_start_cp as usize,
                end_cp: end as usize,
            });
        if source.is_none() {
            unresolved.push("bookmark_source_range");
        }
        if source.is_some_and(|s| !boundary(body, s.start_cp) || !boundary(body, s.end_cp)) {
            unresolved.push("bookmark_surrogate_boundary");
        }
        if flags & 0x8000 != 0 {
            unresolved.push("annotation_bookmark_column_flag");
        }
        if end_counts[&end_index] != 1 {
            unresolved.push("ambiguous_bookmark_end_index");
        }
        out.push(AnnotationBookmark {
            index,
            tag: u32_at(tags, at + 4)?,
            raw_old_tag: u32_at(tags, at + 8)?,
            raw_start_cp,
            raw_end_cp,
            end_index,
            raw_flags: flags,
            source,
            unresolved,
        });
    }
    Ok(out)
}

/// ATRDPost10 records use signed relative parent indices in a preorder tree.
/// Keep invalid parent/depth and ink evidence separate from source text ownership.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/19181fe7-a62a-4e70-9a5f-a0cb1ebe0921>
fn extended(
    raw: &[u8],
    count: usize,
    diagnostics: &mut Vec<&'static str>,
) -> Result<Vec<Option<ExtendedComment>>> {
    if raw.is_empty() {
        return Ok((0..count).map(|_| None).collect());
    }
    if raw.len() != count * 18 {
        diagnostics.push("comment_extended_count_mismatch");
        return Ok((0..count).map(|_| None).collect());
    }
    let mut out = Vec::with_capacity(count);
    let mut stack = Vec::new();
    for index in 0..count {
        let data = bytes(raw, index * 18, 18)?;
        let depth = u32_at(data, 6)?;
        let delta = u32_at(data, 10)? as i32;
        let flags = u32_at(data, 14)?;
        let mut item = ExtendedComment {
            raw_date_time: u32_at(data, 0)?,
            depth,
            parent_delta: delta,
            parent_index: None,
            ink: flags & 2 != 0,
            raw_flags: flags,
            unresolved: Vec::new(),
        };
        let parent = (index as i64)
            .checked_add(delta as i64)
            .filter(|i| *i >= 0 && *i < index as i64)
            .map(|i| i as usize);
        let valid = if depth == 0 {
            delta == 0
        } else {
            depth as usize <= stack.len() && parent == stack.get(depth as usize - 1).copied()
        };
        if valid {
            stack.truncate(depth as usize);
            item.parent_index = if depth == 0 { None } else { parent };
            stack.push(index);
        } else {
            item.unresolved.push("comment_parent_depth_or_preorder");
            stack.clear();
        }
        if u16_at(data, 4)? != 0 || flags & !2 != 0 {
            item.unresolved.push("comment_extended_reserved_bits");
        }
        out.push(Some(item));
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
