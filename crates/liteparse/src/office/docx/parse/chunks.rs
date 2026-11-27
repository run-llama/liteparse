//! Internal alternate-format imports, projected through the ordinary text flow.
use super::*;

mod html;

/// Limits cover decoded import expansion across all stories, including repeated anchors.
/// The byte budget precedes inflation; the node budget precedes synthetic XML allocation.
pub(super) struct Imports {
    bytes: u64,
    nodes: usize,
    warnings: Vec<(String, String)>,
    types: Option<Option<Rc<XNode>>>,
    assets: HashMap<(String, String), String>,
    next_asset: usize,
    trees: Vec<Rc<XNode>>,
}

impl Default for Imports {
    fn default() -> Self {
        Self {
            bytes: 8 * 1024 * 1024,
            nodes: 65_536,
            warnings: Vec::new(),
            types: None,
            assets: HashMap::new(),
            next_asset: 0,
            trees: Vec::new(),
        }
    }
}

impl Parser<'_> {
    pub(super) fn imported_asset(&self, part: &str, id: &str) -> Option<String> {
        self.imports
            .borrow()
            .assets
            .get(&(part.to_owned(), id.to_owned()))
            .cloned()
    }

    fn register_imported_asset(&self, part: &str, target: String) -> String {
        let mut imports = self.imports.borrow_mut();
        loop {
            let id = format!("native-import-asset-{}", imports.next_asset);
            imports.next_asset += 1;
            let key = (part.to_owned(), id.clone());
            if self.pkg.rel_by_id(part, &id).is_none() && !imports.assets.contains_key(&key) {
                imports.assets.insert(key, target);
                return id;
            }
        }
    }
    pub(super) fn retain_import_tree(&self, tree: Rc<XNode>) {
        self.imports.borrow_mut().trees.push(tree);
    }

    pub fn share_imports(&mut self, parent: &Self) {
        self.imports = parent.imports.clone();
    }

    pub fn import_warnings(&self) -> Vec<(String, String)> {
        self.imports.borrow().warnings.clone()
    }

    pub(super) fn import_chunk(
        &self,
        anchor: &XNode,
        part: &str,
        table: Option<&TableText>,
    ) -> Vec<Block> {
        let nodes = self.materialize_chunk(anchor, part);
        self.index_source_tables(&nodes, part);
        let blocks = self.blocks(&nodes, part, table);
        self.imports.borrow_mut().trees.extend(nodes);
        blocks
    }

    /// Resolve each active import once before table eligibility and source ownership.
    /// Unchanged subtrees retain their original nodes; incomplete imports carry an
    /// invisible marker that prevents replacing their table with text-only HTML.
    pub(super) fn materialized_table(&self, node: &XNode, part: &str) -> Option<Rc<XNode>> {
        let mut changed = false;
        let mut children = Vec::with_capacity(node.children.len());
        for child in &node.children {
            if child.name == "w:altChunk" {
                children.extend(self.materialize_chunk(child, part));
                changed = true;
            } else if matches!(
                child.name.as_str(),
                "w:tr"
                    | "w:tc"
                    | "w:tbl"
                    | "w:customXml"
                    | "w:ins"
                    | "w:moveTo"
                    | "w:sdt"
                    | "w:sdtContent"
            ) {
                if let Some(view) = self.materialized_table(child, part) {
                    children.push(view);
                    changed = true;
                } else {
                    children.push(child.clone());
                }
            } else {
                children.push(child.clone());
            }
        }
        changed.then(|| {
            Rc::new(XNode {
                children,
                ..node.clone()
            })
        })
    }

    fn materialize_chunk(&self, anchor: &XNode, part: &str) -> Vec<Rc<XNode>> {
        let fail = |code: &str, why: &str| {
            let warning = (
                code.to_string(),
                format!(
                    "{part} [{}]: alternate content not imported: {why}",
                    anchor.attr("r:id").unwrap_or("missing relationship")
                ),
            );
            let mut imports = self.imports.borrow_mut();
            if !imports.warnings.contains(&warning) {
                imports.warnings.push(warning);
            }
            vec![incomplete_marker()]
        };
        let Some(rel) = anchor
            .attr("r:id")
            .and_then(|id| self.pkg.rel_by_id(part, id))
        else {
            return fail("ALTCHUNK_INVALID", "relationship is missing");
        };
        let known = [
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/",
            "http://purl.oclc.org/ooxml/officeDocument/relationships/",
        ]
        .iter()
        .any(|base| {
            rel.rel_type
                .strip_prefix(base)
                .is_some_and(|kind| matches!(kind, "aFChunk" | "afChunk"))
        });
        if rel.external || !known {
            return fail(
                "ALTCHUNK_INVALID",
                "requires an internal alternative-format relationship",
            );
        }
        if self.imports.borrow().types.is_none() {
            let types = crate::office::docx::extract::read_xml(self.pkg, "[Content_Types].xml")
                .filter(|n| n.name == "ct:Types");
            self.imports.borrow_mut().types = Some(types);
        }
        let cached_types = self.imports.borrow().types.clone().flatten();
        let Some(types) = cached_types else {
            return fail("ALTCHUNK_INVALID", "content types are unavailable");
        };
        let content_type = types
            .children
            .iter()
            .find(|n| {
                n.name == "ct:Override"
                    && n.attr("PartName").is_some_and(|p| {
                        p.trim_start_matches('/').eq_ignore_ascii_case(&rel.target)
                    })
            })
            .or_else(|| {
                types.children.iter().find(|n| {
                    n.name == "ct:Default"
                        && n.attr("Extension").is_some_and(|ext| {
                            rel.target
                                .rsplit('.')
                                .next()
                                .is_some_and(|p| p.eq_ignore_ascii_case(ext))
                        })
                })
            })
            .and_then(|n| n.attr("ContentType"));
        let Some(content_type) = content_type else {
            return fail("ALTCHUNK_INVALID", "content type is missing");
        };
        let mut parameters = content_type.split(';');
        let mime = parameters.next().unwrap_or("").trim();
        let is_html = mime.eq_ignore_ascii_case("text/html");
        if !is_html && !mime.eq_ignore_ascii_case("text/plain") {
            return fail("ALTCHUNK_UNSUPPORTED", "content type is unsupported");
        }
        let mut charset = None;
        for parameter in parameters {
            let Some((name, value)) = parameter.trim().split_once('=') else {
                return fail("ALTCHUNK_DECODE_FAILED", "encoding declaration is invalid");
            };
            if name.trim().eq_ignore_ascii_case("charset") {
                if charset.is_some() {
                    return fail(
                        "ALTCHUNK_DECODE_FAILED",
                        "encoding declaration is ambiguous",
                    );
                }
                charset = Some(value.trim().trim_matches('"').to_ascii_lowercase());
            }
        }
        let Some(size) = self.pkg.part_size(&rel.target) else {
            return fail("ALTCHUNK_INVALID", "part is missing");
        };
        if size > self.imports.borrow().bytes {
            return fail(
                "ALTCHUNK_RESOURCE_LIMIT",
                "exceeds the shared import byte budget",
            );
        }
        let Ok(bytes) = self.pkg.read(&rel.target) else {
            return fail("ALTCHUNK_INVALID", "part cannot be read");
        };
        self.imports.borrow_mut().bytes -= size;
        if is_html {
            return match self.html_chunk(&bytes, charset.as_deref(), &rel.target, part) {
                Ok((mut nodes, warnings)) => {
                    if !warnings.is_empty() {
                        nodes.push(incomplete_marker());
                    }
                    for warning in warnings {
                        let detail = format!(
                            "{part} [{}]: HTML import is incomplete: {warning}",
                            anchor.attr("r:id").unwrap_or("missing relationship")
                        );
                        let entry = ("ALTCHUNK_HTML_UNSUPPORTED".to_string(), detail);
                        let mut imports = self.imports.borrow_mut();
                        if !imports.warnings.contains(&entry) {
                            imports.warnings.push(entry);
                        }
                    }
                    nodes
                }
                Err((code, reason)) => fail(code, reason),
            };
        }
        let text = match decode(&bytes, charset.as_deref()) {
            Ok(text) => text,
            Err(reason) => return fail("ALTCHUNK_DECODE_FAILED", reason),
        };
        let controls = text
            .chars()
            .filter(|c| matches!(c, '\r' | '\n' | '\t' | '\u{c}'))
            .count();
        let nodes = controls.saturating_mul(4).saturating_add(4);
        if nodes > self.imports.borrow().nodes {
            return fail(
                "ALTCHUNK_RESOURCE_LIMIT",
                "exceeds the shared import node budget",
            );
        }
        self.imports.borrow_mut().nodes -= nodes;
        if text.is_empty() {
            return Vec::new();
        }
        // Ordinary paragraph parsing supplies host styles and cell ownership.
        // Explicit controls preserve line/tab/page boundaries and source ordering.
        paragraphs(&text)
    }
}

fn incomplete_marker() -> Rc<XNode> {
    Rc::new(XNode {
        name: "native:incompleteImport".into(),
        attrs: Vec::new(),
        children: Vec::new(),
        text: String::new(),
    })
}

/// Unicode BOMs identify imported Unicode text (ECMA-376 §11.3.1 recommends UTF-16).
/// MIME text/plain otherwise defaults to US-ASCII (RFC 2046 §4.1.2); decoding is strict.
#[expect(
    clippy::unwrap_in_result,
    reason = "the US-ASCII arm runs only on ASCII bytes, which are valid UTF-8"
)]
fn decode(bytes: &[u8], charset: Option<&str>) -> Result<String, &'static str> {
    let bom = if bytes.starts_with(&[0xff, 0xfe]) {
        Some(("utf-16le", 2))
    } else if bytes.starts_with(&[0xfe, 0xff]) {
        Some(("utf-16be", 2))
    } else if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        Some(("utf-8", 3))
    } else {
        None
    };
    let encoding = charset.or(bom.map(|b| b.0)).unwrap_or("us-ascii");
    if let (Some(explicit), Some((detected, _))) = (charset, bom)
        && explicit != detected
        && !(explicit == "utf-16" && detected.starts_with("utf-16"))
    {
        return Err("encoding and byte-order mark disagree");
    }
    let bytes = &bytes[bom.map_or(0, |b| b.1)..];
    let text = match encoding {
        "us-ascii" if bytes.is_ascii() => String::from_utf8(bytes.to_vec()).unwrap(),
        "utf-8" => std::str::from_utf8(bytes)
            .map_err(|_| "contains invalid UTF-8")?
            .to_string(),
        "utf-16" | "utf-16le" | "utf-16be" => {
            if !bytes.len().is_multiple_of(2) {
                return Err("contains incomplete UTF-16");
            }
            let little = encoding == "utf-16le"
                || (encoding == "utf-16" && bom.is_some_and(|b| b.0 == "utf-16le"));
            let units = bytes.as_chunks::<2>().0.iter().map(|b| {
                if little {
                    u16::from_le_bytes([b[0], b[1]])
                } else {
                    u16::from_be_bytes([b[0], b[1]])
                }
            });
            char::decode_utf16(units)
                .collect::<Result<String, _>>()
                .map_err(|_| "contains invalid UTF-16")?
        }
        "us-ascii" => return Err("contains non-ASCII bytes without a Unicode declaration"),
        _ => return Err("uses an unsupported character encoding"),
    };
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t' | '\u{c}'))
    {
        return Err("contains unsupported control characters");
    }
    Ok(text)
}

fn node(name: &str, text: String, children: Vec<Rc<XNode>>) -> Rc<XNode> {
    Rc::new(XNode {
        name: name.into(),
        attrs: Vec::new(),
        children,
        text,
    })
}

fn paragraphs(text: &str) -> Vec<Rc<XNode>> {
    let mut paragraphs = Vec::new();
    let mut children = Vec::new();
    let mut segment = String::new();
    let flush = |segment: &mut String, children: &mut Vec<Rc<XNode>>| {
        if !segment.is_empty() {
            let mut text = node("w:t", std::mem::take(segment), Vec::new());
            Rc::make_mut(&mut text)
                .attrs
                .push(("space".into(), "preserve".into()));
            children.push(text);
        }
    };
    let finish = |children: &mut Vec<Rc<XNode>>| {
        node(
            "w:p",
            String::new(),
            vec![node("w:r", String::new(), std::mem::take(children))],
        )
    };
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if !matches!(c, '\r' | '\n' | '\t' | '\u{c}') {
            segment.push(c);
            continue;
        }
        flush(&mut segment, &mut children);
        if matches!(c, '\r' | '\n') {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            paragraphs.push(finish(&mut children));
        } else {
            let mut control = node(
                if c == '\t' { "w:tab" } else { "w:br" },
                String::new(),
                Vec::new(),
            );
            if c == '\u{c}' {
                Rc::make_mut(&mut control)
                    .attrs
                    .push(("type".into(), "page".into()));
            }
            children.push(control);
        }
    }
    flush(&mut segment, &mut children);
    if !children.is_empty() {
        paragraphs.push(finish(&mut children));
    }
    paragraphs
}

#[cfg(test)]
mod tests;
