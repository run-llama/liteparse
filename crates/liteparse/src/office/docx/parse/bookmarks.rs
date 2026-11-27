use super::*;
use crate::office::pptx::scene::{SourceBookmark, TextSource};
use std::sync::Arc;

#[derive(Default)]
pub(super) struct Registry {
    nodes: Vec<Rc<XNode>>,
    by_node: HashMap<(String, usize), Arc<SourceBookmark>>,
    next: HashMap<String, usize>,
    pub definitions: Vec<Arc<SourceBookmark>>,
}

impl Parser<'_> {
    pub fn bookmark_definitions(&self) -> Vec<Arc<SourceBookmark>> {
        self.bookmarks.borrow().definitions.clone()
    }

    pub fn share_bookmarks(&mut self, parent: &Self) {
        self.bookmarks = parent.bookmarks.clone();
    }

    pub(super) fn source_bookmark(
        &self,
        node: &Rc<XNode>,
        part: &str,
        block_boundary: bool,
    ) -> Option<Arc<SourceBookmark>> {
        let name = node.attr("name").filter(|name| !name.is_empty())?;
        let key = (part.to_owned(), Rc::as_ptr(node) as usize);
        let mut registry = self.bookmarks.borrow_mut();
        if let Some(marker) = registry.by_node.get(&key) {
            return Some(marker.clone());
        }
        let next = registry.next.entry(part.to_owned()).or_default();
        let id = format!("{part}:bookmark-{next}");
        *next += 1;
        let source = self.source.borrow();
        let marker = Arc::new(SourceBookmark {
            block_boundary,
            id,
            xml_id: node.attr("id").unwrap_or("").to_owned(),
            name: name.to_owned(),
            source: TextSource {
                part: part.to_owned(),
                paragraph: source.paragraph,
                cells: source.cells.clone(),
                link: None,
            },
        });
        // Retain nodes so pointer identities remain stable across parsed stories.
        registry.nodes.push(node.clone());
        registry.by_node.insert(key, marker.clone());
        registry.definitions.push(marker.clone());
        Some(marker)
    }
}

/// Attaches semantic block boundaries to existing flow owners without adding a paragraph.
pub(super) fn place_boundaries(
    blocks: &mut [Block],
    boundaries: &mut Vec<(usize, Arc<SourceBookmark>)>,
) {
    let mut grouped: std::collections::BTreeMap<usize, Vec<Arc<SourceBookmark>>> =
        Default::default();
    for (index, marker) in boundaries.drain(..) {
        grouped.entry(index).or_default().push(marker);
    }
    for (index, markers) in grouped {
        let end = index == blocks.len();
        let Some(block) = blocks.get_mut(if end { index.saturating_sub(1) } else { index }) else {
            continue;
        };
        match block {
            Block::Para(para) => {
                let at = if end { para.inlines.len() } else { 0 };
                para.inlines
                    .splice(at..at, markers.into_iter().map(Inline::Bookmark));
            }
            Block::Table(table) => {
                if end {
                    table.bookmarks_after.extend(markers);
                } else {
                    table.bookmarks_before.extend(markers);
                }
            }
        }
    }
}
