//! Footnote ownership keeps authored text, automatic marks and terminal story guards distinct.
use super::{PlainParagraph, Result};
use crate::office::doc::{Error, Inspection};
use serde::Serialize;

pub use crate::office::doc::notes::NotePosition;
mod endnotes;
pub(super) use endnotes::inspect as inspect_endnotes;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteKind {
    Footnote,
    Endnote,
    Comment,
}
impl NoteKind {
    pub fn marker(&self) -> char {
        if *self == Self::Comment {
            '\u{5}'
        } else {
            '\u{2}'
        }
    }
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde skip_serializing_if passes a reference"
    )]
    fn is_footnote(&self) -> bool {
        *self == Self::Footnote
    }
}

#[derive(Debug, Serialize)]
pub struct FootnoteDocument {
    #[serde(skip_serializing_if = "NoteKind::is_footnote")]
    pub kind: NoteKind,
    pub source_cp: [usize; 2],
    pub source_text: String,
    pub terminal_guard: [usize; 2],
    pub position: Option<NotePosition>,
    pub position_basis: Option<&'static str>,
    pub sections: Vec<crate::office::doc::notes::FootnoteSection>,
    pub coexisting_endnotes: Vec<crate::office::doc::notes::NoteLink>,
    pub coexisting_endnote_source: Option<[usize; 2]>,
    pub notes: Vec<Footnote>,
    pub separators: Vec<FootnoteSeparator>,
}

#[derive(Debug, Serialize)]
pub struct FootnoteSeparator {
    pub index: usize,
    pub source_cp: [usize; 2],
    pub source_text: String,
    pub paragraphs: Vec<PlainParagraph>,
}

#[derive(Debug, Serialize)]
pub struct Footnote {
    pub id: i64,
    pub body_cp: usize,
    pub raw_reference_index: u16,
    pub section_index: usize,
    pub marker_status: crate::office::doc::notes::FootnoteMarkerStatus,
    pub source_cp: [usize; 2],
    pub marker_cp: Option<usize>,
    pub authored_reference: Option<u16>,
    pub label: String,
    pub paragraphs: Vec<PlainParagraph>,
}

/// Establish source ownership before paragraph formatting or pagination is attempted.
/// Separator layout and ordinary hidden-text qualification are independent requirements.
pub fn inspect(data: &[u8], source: &Inspection) -> Result<Option<FootnoteDocument>> {
    let Some(owned) = crate::office::doc::notes::inspect_note_ownership(data)? else {
        return Ok(None);
    };
    let story = source
        .stories
        .get(1)
        .ok_or(Error::Invalid("note adapter source story"))?;
    if story.kind != "footnotes"
        || story.start_cp != owned.source_cp[0]
        || story.utf16_units != owned.source_cp[1] - owned.source_cp[0]
        || story.text != owned.source_text
    {
        return Err(Error::Invalid("note adapter source identity"));
    }
    Ok(Some(FootnoteDocument {
        kind: NoteKind::Footnote,
        source_cp: owned.source_cp,
        source_text: owned.source_text,
        terminal_guard: owned.terminal_guard,
        position: owned.position,
        position_basis: owned.position_basis,
        sections: owned.sections,
        coexisting_endnotes: owned.coexisting_endnotes,
        coexisting_endnote_source: owned.coexisting_endnote_source,
        notes: owned
            .notes
            .into_iter()
            .map(|n| Footnote {
                id: n.id,
                body_cp: n.body_cp,
                raw_reference_index: n.raw_reference_index,
                section_index: n.section_index,
                marker_status: n.marker_status,
                source_cp: n.source_cp,
                marker_cp: n.marker_cp,
                authored_reference: n.authored_reference,
                label: n.label,
                paragraphs: Vec::new(),
            })
            .collect(),
        separators: owned
            .separators
            .into_iter()
            .map(|s| FootnoteSeparator {
                index: s.index,
                source_cp: s.source_cp,
                source_text: s.source_text,
                paragraphs: Vec::new(),
            })
            .collect(),
    }))
}

impl FootnoteSeparator {
    pub fn graphic_controls_only(&self) -> bool {
        self.source_text
            .chars()
            .any(|c| matches!(c, '\u{3}' | '\u{4}'))
            && self
                .source_text
                .chars()
                .all(|c| matches!(c, '\u{3}' | '\u{4}' | '\r'))
    }
}

impl FootnoteDocument {
    pub fn source_prefix(&self) -> &'static str {
        match self.kind {
            NoteKind::Footnote => "WordDocument/footnote/",
            NoteKind::Endnote => "WordDocument/endnote/",
            NoteKind::Comment => "WordDocument/comment/",
        }
    }

    pub(super) fn marker_cps(&self) -> impl Iterator<Item = usize> + '_ {
        self.notes
            .iter()
            .filter(|n| n.authored_reference.is_none())
            .flat_map(|n| std::iter::once(n.body_cp).chain(n.marker_cp))
    }

    pub(super) fn ranges(&self) -> Vec<[usize; 2]> {
        self.notes
            .iter()
            .map(|n| n.source_cp)
            .chain(
                self.separators
                    .iter()
                    .filter(|s| !s.graphic_controls_only())
                    .map(|s| s.source_cp),
            )
            .collect()
    }

    pub(super) fn validate_separators(
        &self,
        headers: Option<&super::headers::HeaderDocument>,
        fonts: usize,
    ) -> Result<()> {
        let expected = headers
            .into_iter()
            .flat_map(|h| h.ownership.stories.iter().take(3))
            .filter_map(|s| s.content.map(|r| (s.index, [r.start_cp, r.end_cp])))
            .collect::<Vec<_>>();
        if self
            .separators
            .iter()
            .map(|s| (s.index, s.source_cp))
            .collect::<Vec<_>>()
            != expected
        {
            return Err(Error::Invalid("note separator slot coverage"));
        }
        for separator in &self.separators {
            if !separator.graphic_controls_only() {
                super::validate_partition(&separator.paragraphs, separator.source_cp, fonts)?;
            } else if !separator.paragraphs.is_empty() {
                return Err(Error::Invalid(
                    "unresolved separator control acquired paragraphs",
                ));
            }
            let text: String = if separator.graphic_controls_only() {
                separator.source_text.clone()
            } else {
                separator
                    .paragraphs
                    .iter()
                    .flat_map(|p| &p.runs)
                    .map(|r| r.text.as_str())
                    .collect()
            };
            if text != separator.source_text || text.contains('\u{2}') {
                return Err(Error::Invalid("note separator text ownership"));
            }
            let header = headers.ok_or(Error::Invalid("note separator header source"))?;
            let raw: Vec<_> = header.source_text.encode_utf16().collect();
            let a = separator.source_cp[0]
                .checked_sub(header.ownership.source.start_cp)
                .ok_or(Error::Invalid("note separator header start"))?;
            let b = separator.source_cp[1]
                .checked_sub(header.ownership.source.start_cp)
                .ok_or(Error::Invalid("note separator header end"))?;
            if raw.get(a..b) != Some(text.encode_utf16().collect::<Vec<_>>().as_slice()) {
                return Err(Error::Invalid("note separator raw text"));
            }
        }
        Ok(())
    }

    pub(super) fn validate(&self, body: &[PlainParagraph], fonts: usize) -> Result<()> {
        let body_end = body
            .last()
            .map(|p| p.source_cp[1])
            .ok_or(Error::Invalid("render footnote body partition"))?;
        let source_start = self.source_cp[0];
        if (self.kind == NoteKind::Footnote && source_start != body_end)
            || source_start < body_end
            || self.source_cp[1] <= source_start
            || self.source_cp[1] - source_start > 1024 * 1024
            || self.notes.is_empty()
            || self.notes.len() > 4096
            || self
                .notes
                .windows(2)
                .any(|pair| pair[0].body_cp >= pair[1].body_cp)
            || self.terminal_guard != [self.source_cp[1] - 1, self.source_cp[1]]
        {
            return Err(Error::Invalid("render footnote model extent"));
        }
        let raw: Vec<_> = self.source_text.encode_utf16().collect();
        if raw.len() != self.source_cp[1] - source_start || raw.last() != Some(&13) {
            return Err(Error::Invalid("render footnote raw story"));
        }
        let markers: Vec<_> = body
            .iter()
            .flat_map(|p| &p.runs)
            .flat_map(|r| {
                r.text.encode_utf16().enumerate().filter_map(move |(i, c)| {
                    (c == self.kind.marker() as u16).then_some(r.source_cp[0] + i)
                })
            })
            .collect();
        if markers
            != self
                .notes
                .iter()
                .filter(|n| n.authored_reference.is_none())
                .map(|n| n.body_cp)
                .collect::<Vec<_>>()
        {
            return Err(Error::Invalid("render footnote body marker coverage"));
        }
        let marker_cps = self.marker_cps().collect();
        let mut cp = source_start;
        for (index, note) in self.notes.iter().enumerate() {
            if note.id != (index + 1) as i64
                || note.source_cp[0] != cp
                || note.source_cp[1] <= cp
                || note.source_cp[1] > self.terminal_guard[0]
                || (note.authored_reference.is_none() && note.label.is_empty())
                || note.label.len() > 64
                || note.label.chars().any(char::is_whitespace)
                || note.label.chars().any(char::is_control)
            {
                return Err(Error::Invalid("render footnote model ownership"));
            }
            if let Some(code) = note.authored_reference {
                let actual = body.iter().flat_map(|p| &p.runs).find_map(|r| {
                    note.body_cp
                        .checked_sub(r.source_cp[0])
                        .filter(|i| *i < r.source_cp[1] - r.source_cp[0])
                        .and_then(|i| r.text.encode_utf16().nth(i))
                });
                if actual != Some(code)
                    || code < 32
                    || (0xd800..=0xdfff).contains(&code)
                    || note.marker_cp.is_some()
                    || !note.label.is_empty()
                {
                    return Err(Error::Invalid("render authored note reference identity"));
                }
            }
            super::validate_partition_controls(
                &note.paragraphs,
                note.source_cp,
                fonts,
                &Default::default(),
                &Default::default(),
                &marker_cps,
            )?;
            let actual: Vec<_> = note
                .paragraphs
                .iter()
                .flat_map(|p| &p.runs)
                .flat_map(|r| r.text.encode_utf16())
                .collect();
            if actual != raw[cp - source_start..note.source_cp[1] - source_start]
                || actual
                    .iter()
                    .enumerate()
                    .filter_map(|(i, c)| (*c == self.kind.marker() as u16).then_some(cp + i))
                    .collect::<Vec<_>>()
                    != note.marker_cp.into_iter().collect::<Vec<_>>()
            {
                return Err(Error::Invalid("render footnote text and marker coverage"));
            }
            cp = note.source_cp[1];
        }
        if cp != self.terminal_guard[0] {
            return Err(Error::Invalid("render footnote guard coverage"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
