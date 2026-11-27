//! Deferred special-note stories retain source markup and relationship context.

use std::rc::Rc;

use crate::office::pptx::xml::XNode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialNoteKind {
    Separator,
    ContinuationSeparator,
    ContinuationNotice,
}

impl SpecialNoteKind {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "separator" => Some(Self::Separator),
            "continuationSeparator" => Some(Self::ContinuationSeparator),
            "continuationNotice" => Some(Self::ContinuationNotice),
            _ => None,
        }
    }
}

/// Source is deferred so retaining a story does not mutate numbering or graphics.
/// Its part resolves relationships when a future fragment plan lays it out.
#[derive(Debug, Clone)]
pub struct SpecialNoteStory {
    pub id: i64,
    pub kind: SpecialNoteKind,
    pub part: String,
    pub source: Rc<XNode>,
}

#[derive(Debug, Clone, Default)]
pub struct SpecialNoteStories {
    pub definitions: Vec<SpecialNoteStory>,
    /// Settings references retain source order, missing IDs and duplicates.
    pub references: Vec<i64>,
}

impl SpecialNoteStories {
    /// Selected empty stories remain present. Duplicate definitions are retained;
    /// this inventory does not decide their rendering precedence or fallback.
    pub fn selected(&self, kind: SpecialNoteKind) -> impl Iterator<Item = &SpecialNoteStory> {
        self.definitions
            .iter()
            .filter(move |s| s.kind == kind && self.references.contains(&s.id))
    }

    pub(super) fn read(
        root: Option<&XNode>,
        part: &str,
        settings: Option<&XNode>,
        kind: &str,
    ) -> Self {
        let element = format!("w:{kind}");
        let properties = format!("w:{kind}Pr");
        let references = settings
            .and_then(|s| s.child(&properties))
            .into_iter()
            .flat_map(|s| s.children_named(&element))
            .filter_map(|s| s.attr_i64("id"))
            .collect();
        let definitions = root
            .into_iter()
            .flat_map(|r| r.children_named(&element))
            .filter_map(|s| {
                Some(SpecialNoteStory {
                    id: s.attr_i64("id")?,
                    kind: SpecialNoteKind::parse(s.attr("type")?)?,
                    part: part.to_owned(),
                    source: s.clone(),
                })
            })
            .collect();
        Self {
            definitions,
            references,
        }
    }
}

#[cfg(test)]
mod tests;
