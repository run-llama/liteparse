use super::*;
use crate::office::pptx::scene::{SourceCell, SourceLink, SourceLinkTarget, TextSource};
use std::sync::Arc;

struct MergeChain {
    grid_start: usize,
    span: usize,
    members: Vec<(usize, usize)>,
}

/// Match adjacent continuation cells by their complete logical grid interval.
pub(super) fn vertical_merges(
    rows: &[&Rc<XNode>],
    grids: &[super::grid::RowGrid],
) -> Vec<Vec<crate::office::pptx::scene::SourceVerticalMerge>> {
    let mut chains: Vec<MergeChain> = Vec::new();
    let mut previous = HashMap::new();
    for (ri, row) in rows.iter().enumerate() {
        let mut column = grids[ri].before;
        let mut cells = Vec::new();
        collect_named(&row.children, "w:tc", &mut cells);
        let mut current = HashMap::new();
        for (ci, cell) in cells.iter().enumerate() {
            let pr = cell.child("w:tcPr");
            let span = grids[ri].spans[ci];
            if let Some(merge) = pr.and_then(|pr| pr.child("w:vMerge")) {
                let key = (column, span);
                let chain = if merge.attr("val") == Some("restart") {
                    chains.push(MergeChain {
                        grid_start: column,
                        span,
                        members: Vec::new(),
                    });
                    Some(chains.len() - 1)
                } else if matches!(merge.attr("val"), None | Some("continue")) {
                    previous.get(&key).copied()
                } else {
                    None
                };
                if let Some(index) = chain {
                    chains[index].members.push((ri, ci));
                    current.insert(key, index);
                }
            }
            column += span;
        }
        previous = current;
    }
    let mut result = vec![Vec::new(); rows.len()];
    for MergeChain {
        grid_start,
        span,
        members,
    } in chains
    {
        if members.len() < 2 {
            continue;
        }
        let start_row = members[0].0;
        let end_row = members[members.len() - 1].0;
        for (row, cell) in members {
            result[row].push(crate::office::pptx::scene::SourceVerticalMerge {
                cell,
                grid_start,
                span,
                start_row,
                end_row,
            });
        }
    }
    result
}

#[derive(Default)]
pub(super) struct SourceState {
    pub cells: Vec<SourceCell>,
    pub link: Option<SourceLink>,
    // Explicit link scopes remember which complex fields already enclosed them.
    link_field_depth: usize,
    link_is_reference: bool,
    tables: HashMap<(String, usize), usize>,
    next_table: HashMap<String, usize>,
    next_link: usize,
    next_bookmark_link: usize,
    pub paragraph: usize,
    pub next_paragraph: usize,
}

impl Parser<'_> {
    pub(super) fn index_source_tables(&self, nodes: &[Rc<XNode>], part: &str) {
        for node in nodes {
            if node.name == "w:tbl" {
                self.source_table_id(node, part);
            } else {
                self.index_source_tables(&node.children, part);
            }
        }
    }

    pub(super) fn source_table_id(&self, node: &XNode, part: &str) -> usize {
        let mut source = self.source.borrow_mut();
        if let Some(cell) = source.cells.first() {
            return cell.table;
        }
        let key = (part.to_owned(), std::ptr::from_ref(node) as usize);
        if let Some(id) = source.tables.get(&key) {
            return *id;
        }
        let next = source.next_table.entry(part.to_owned()).or_default();
        let id = *next;
        *next += 1;
        source.tables.insert(key, id);
        id
    }

    pub(super) fn next_source_link_id(&self) -> String {
        let mut source = self.source.borrow_mut();
        let id = source.next_link;
        source.next_link += 1;
        format!("link-{id}")
    }

    pub(super) fn enter_source_link(
        &self,
        uri: Option<String>,
        anchor: Option<&str>,
        part: &str,
    ) -> (Option<SourceLink>, usize, bool) {
        let target = if let Some(mut uri) = uri {
            if let Some(anchor) = anchor.filter(|s| !s.is_empty()) {
                uri.push('#');
                uri.push_str(anchor);
            }
            Some(SourceLinkTarget::ExternalUri(uri))
        } else {
            anchor
                .filter(|name| !name.is_empty())
                .map(|name| SourceLinkTarget::Bookmark {
                    part: part.to_owned(),
                    name: name.to_owned(),
                })
        };
        self.enter_source_target(target, false)
    }

    pub(super) fn enter_source_target(
        &self,
        target: Option<SourceLinkTarget>,
        reference: bool,
    ) -> (Option<SourceLink>, usize, bool) {
        let link = target.map(|target| {
            let id = if matches!(&target, SourceLinkTarget::Bookmark { .. }) {
                let mut source = self.source.borrow_mut();
                let id = format!("bookmark-link-{}", source.next_bookmark_link);
                source.next_bookmark_link += 1;
                id
            } else {
                self.next_source_link_id()
            };
            SourceLink { id, target }
        });
        let mut source = self.source.borrow_mut();
        let previous = (
            source.link.take(),
            source.link_field_depth,
            source.link_is_reference,
        );
        source.link = link;
        source.link_is_reference = reference;
        source.link_field_depth = self.fields.borrow().len();
        previous
    }

    pub(super) fn restore_source_link(&self, previous: (Option<SourceLink>, usize, bool)) {
        let mut source = self.source.borrow_mut();
        (
            source.link,
            source.link_field_depth,
            source.link_is_reference,
        ) = previous;
    }

    pub(super) fn with_source(&self, style: Rc<RunStyle>, part: &str) -> Rc<RunStyle> {
        let source = self.source.borrow();
        let fields = self.fields.borrow();
        let field_link = |reference: bool| {
            fields.iter().enumerate().rev().find_map(|(depth, field)| {
                if !field.separated
                    || field.replaced
                    || field.dynamic_instruction
                    || reference_instruction(&field.instr) != reference
                    || (reference && source.link.is_some() && depth < source.link_field_depth)
                {
                    return None;
                }
                hyperlink_target(&field.instr, part).map(|target| SourceLink {
                    id: field.source_id.clone(),
                    target,
                })
            })
        };
        // Explicit hyperlinks own their cached field results. REF-only nesting uses
        // lexical field depth while preserving an enclosing same-target owner.
        let explicit = source
            .link
            .as_ref()
            .filter(|_| !source.link_is_reference)
            .cloned()
            .or_else(|| field_link(false));
        let reference = field_link(true);
        let link = explicit.or_else(|| match (&source.link, &reference) {
            (Some(outer), Some(inner)) if outer.target == inner.target => Some(outer.clone()),
            _ => reference.or_else(|| source.link.clone()),
        });
        if source.cells.is_empty() && link.is_none() {
            return style;
        }
        // Internal ownership retains the style identity used before link attribution.
        // Cell attribution already creates a distinct style for each source run.
        let source_style = matches!(
            link.as_ref().map(|link| &link.target),
            Some(SourceLinkTarget::Bookmark { .. })
        )
        .then(|| {
            if source.cells.is_empty() {
                style.clone()
            } else {
                Rc::new(RunStyle {
                    source: Some(Arc::new(TextSource {
                        part: part.to_owned(),
                        paragraph: source.paragraph,
                        cells: source.cells.clone(),
                        link: None,
                    })),
                    ..(*style).clone()
                })
            }
        });
        Rc::new(RunStyle {
            source_style,
            source: Some(Arc::new(TextSource {
                part: part.to_owned(),
                paragraph: source.paragraph,
                cells: source.cells.clone(),
                link,
            })),
            ..(*style).clone()
        })
    }
}

/// ECMA-376 §17.16.1 decodes escaped quotes and backslashes in field arguments.
#[cfg(test)]
pub(super) fn hyperlink_instruction(instruction: &str) -> Option<String> {
    match hyperlink_target(instruction, "")? {
        SourceLinkTarget::ExternalUri(uri) => Some(uri),
        SourceLinkTarget::Bookmark { .. } => None,
    }
}

pub(super) fn reference_instruction(instruction: &str) -> bool {
    let kind = instruction
        .trim_start()
        .split(|c: char| c.is_whitespace() || c == '"' || c == '\\')
        .next()
        .unwrap_or("");
    kind.eq_ignore_ascii_case("REF") || kind.eq_ignore_ascii_case("PAGEREF")
}

#[expect(
    clippy::unwrap_in_result,
    reason = "each `characters.next()` follows a `peek()` that matched a character"
)]
pub(super) fn hyperlink_target(instruction: &str, part: &str) -> Option<SourceLinkTarget> {
    let instruction = instruction.trim();
    let boundary = instruction.find(|c: char| c.is_whitespace() || c == '"' || c == '\\')?;
    let (kind, rest) = instruction.split_at(boundary);
    let reference = kind.eq_ignore_ascii_case("REF") || kind.eq_ignore_ascii_case("PAGEREF");
    if !reference && !kind.eq_ignore_ascii_case("HYPERLINK") {
        return None;
    }
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quoted = false;
    // Started preserves empty quoted arguments; switch records an unquoted prefix
    // before escape decoding, so a quoted or escaped UNC target stays an argument.
    let mut started = false;
    let mut switch = false;
    let mut characters = rest.chars().peekable();
    while let Some(character) = characters.next() {
        if character.is_whitespace() && !quoted {
            if started {
                tokens.push((std::mem::take(&mut token), switch));
                started = false;
                switch = false;
            }
            continue;
        }
        if character == '"' {
            if quoted || started {
                tokens.push((std::mem::take(&mut token), switch));
            }
            quoted = !quoted;
            started = quoted;
            switch = false;
        } else if character == '\\' && matches!(characters.peek(), Some('"' | '\\')) {
            token.push(characters.next().unwrap());
            started = true;
        } else if character == '\\'
            && !quoted
            && (!reference
                || characters
                    .clone()
                    .nth(1)
                    .is_none_or(|c| c.is_whitespace() || c == '"' || c == '\\'))
            && (matches!(
                characters.peek(),
                Some('l' | 'o' | 't' | 'm' | 'n' | 'h' | 's')
            ) || (reference
                && matches!(
                    characters.peek(),
                    Some('d' | 'p' | 'r' | 'w' | 'f' | '*' | '#' | '@')
                )))
        {
            if started {
                tokens.push((std::mem::take(&mut token), switch));
            }
            tokens.push((format!("\\{}", characters.next().unwrap()), true));
            started = false;
            switch = false;
        } else {
            if !started {
                switch = character == '\\' && !quoted;
            }
            token.push(character);
            started = true;
        }
    }
    if started {
        tokens.push((token, switch));
    }
    if reference {
        // REF/PAGEREF navigation uses the saved result and a real \h switch.
        // Format/separator arguments keep their lexical role even when empty.
        let (name, switch) = tokens.first()?;
        if *switch || name.is_empty() {
            return None;
        }
        let mut index = 1;
        let mut hyperlink = false;
        while index < tokens.len() {
            let (value, switch) = &tokens[index];
            if *switch {
                match value.as_str() {
                    "\\h" => hyperlink = true,
                    "\\d" | "\\*" | "\\#" | "\\@" => {
                        tokens.get(index + 1)?;
                        index += 1;
                    }
                    "\\f" | "\\n" | "\\p" | "\\r" | "\\t" | "\\w" => {}
                    _ => return None,
                }
            }
            index += 1;
        }
        return hyperlink.then(|| SourceLinkTarget::Bookmark {
            part: part.to_owned(),
            name: name.clone(),
        });
    }
    let mut uri = None;
    let mut anchor = None;
    let mut index = 0;
    while index < tokens.len() {
        let (value, switch) = &tokens[index];
        if *switch {
            match value.as_str() {
                "\\l" => {
                    index += 1;
                    anchor = tokens.get(index).map(|(value, _)| value.clone());
                }
                "\\o" | "\\t" => {
                    index += 1;
                }
                _ => {}
            }
        } else if uri.is_none() {
            uri = Some(value.clone());
        }
        index += 1;
    }
    if let Some(mut uri) = uri.filter(|value| !value.is_empty()) {
        if let Some(anchor) = anchor {
            uri.push('#');
            uri.push_str(&anchor);
        }
        Some(SourceLinkTarget::ExternalUri(uri))
    } else {
        anchor
            .filter(|name| !name.is_empty())
            .map(|name| SourceLinkTarget::Bookmark {
                part: part.to_owned(),
                name,
            })
    }
}

/// Fragment HTML supports visible text rows; other content stays in native page output.
pub(super) fn text_only_table(table: &XNode) -> bool {
    fn valid(node: &XNode) -> bool {
        if matches!(
            node.name.as_str(),
            "native:incompleteImport"
                | "w:altChunk"
                | "w:tbl"
                | "w:drawing"
                | "w:pict"
                | "w:object"
                | "w:vanish"
                | "w:webHidden"
                | "w:del"
                | "w:moveFrom"
                | "m:oMath"
                | "m:oMathPara"
        ) {
            return false;
        }
        if node.name == "w:tr" && !has_text(node) {
            return false;
        }
        node.children.iter().all(|child| valid(child))
    }
    fn has_text(node: &XNode) -> bool {
        (node.name == "w:t" && !node.text.trim().is_empty())
            || node.children.iter().any(|child| has_text(child))
    }
    table.children.iter().all(|child| valid(child))
}

#[cfg(test)]
mod tests;
