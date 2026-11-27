//! A small owned XML tree over `roxmltree`.
//!
//! Element and attribute names are rewritten to canonical prefixes keyed by
//! namespace URI (`a:`, `p:`, `r:`, `c:`...), so lookups read like the spec no matter
//! which prefixes the producer chose. Unknown namespaces get `x:`. Unprefixed
//! attributes keep their bare local name. `mc:AlternateContent` is resolved while
//! building: the `mc:Fallback` branch is spliced in when present, else the first
//! `mc:Choice`. Fallbacks are what PowerPoint itself renders for unknown extensions.

use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct XNode {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Rc<XNode>>,
    /// Concatenated direct text content (only kept for leaf-ish elements).
    pub text: String,
}

const NAMESPACES: &[(&str, &str)] = &[
    ("http://schemas.openxmlformats.org/drawingml/2006/main", "a"),
    ("http://purl.oclc.org/ooxml/drawingml/main", "a"),
    (
        "http://schemas.openxmlformats.org/presentationml/2006/main",
        "p",
    ),
    ("http://purl.oclc.org/ooxml/presentationml/main", "p"),
    (
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
        "r",
    ),
    (
        "http://purl.oclc.org/ooxml/officeDocument/relationships",
        "r",
    ),
    (
        "http://schemas.openxmlformats.org/drawingml/2006/chart",
        "c",
    ),
    ("http://purl.oclc.org/ooxml/drawingml/chart", "c"),
    (
        "http://schemas.openxmlformats.org/drawingml/2006/diagram",
        "dgm",
    ),
    (
        "http://schemas.microsoft.com/office/drawing/2008/diagram",
        "dsp",
    ),
    (
        "http://schemas.openxmlformats.org/officeDocument/2006/math",
        "m",
    ),
    (
        "http://schemas.openxmlformats.org/markup-compatibility/2006",
        "mc",
    ),
    (
        "http://schemas.microsoft.com/office/drawing/2010/main",
        "a14",
    ),
    (
        "http://schemas.microsoft.com/office/powerpoint/2010/main",
        "p14",
    ),
    (
        "http://schemas.microsoft.com/office/powerpoint/2012/main",
        "p15",
    ),
    (
        "http://schemas.microsoft.com/office/drawing/2014/main",
        "a16",
    ),
    (
        "http://schemas.microsoft.com/office/drawing/2016/SVG/main",
        "asvg",
    ),
    (
        "http://schemas.microsoft.com/office/drawing/2014/chartex",
        "cx",
    ),
    (
        "http://schemas.openxmlformats.org/drawingml/2006/table",
        "a",
    ),
    (
        "http://schemas.openxmlformats.org/package/2006/relationships",
        "pr",
    ),
    (
        "http://schemas.openxmlformats.org/package/2006/metadata/core-properties",
        "cp",
    ),
    ("http://purl.org/dc/elements/1.1/", "dc"),
    ("http://purl.org/dc/terms/", "dcterms"),
    (
        "http://schemas.openxmlformats.org/officeDocument/2006/extended-properties",
        "ep",
    ),
    (
        "http://schemas.openxmlformats.org/package/2006/content-types",
        "ct",
    ),
    (
        "http://schemas.microsoft.com/office/powerpoint/2018/8/main",
        "p188",
    ),
    (
        "http://schemas.microsoft.com/office/powerpoint/2021/06/main",
        "p216",
    ),
    // WordprocessingML, for the docx decoder that shares this tree.
    (
        "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
        "w",
    ),
    ("http://purl.oclc.org/ooxml/wordprocessingml/main", "w"),
    (
        "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing",
        "wp",
    ),
    (
        "http://purl.oclc.org/ooxml/drawingml/wordprocessingDrawing",
        "wp",
    ),
    (
        "http://schemas.openxmlformats.org/drawingml/2006/picture",
        "pic",
    ),
    ("http://purl.oclc.org/ooxml/drawingml/picture", "pic"),
    (
        "http://schemas.microsoft.com/office/word/2010/wordprocessingShape",
        "wps",
    ),
    (
        "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup",
        "wpg",
    ),
    (
        "http://schemas.microsoft.com/office/word/2010/wordprocessingCanvas",
        "wpc",
    ),
    (
        "http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing",
        "wp14",
    ),
    (
        "http://schemas.microsoft.com/office/word/2010/wordml",
        "w14",
    ),
    (
        "http://schemas.microsoft.com/office/word/2012/wordml",
        "w15",
    ),
    ("urn:schemas-microsoft-com:vml", "v"),
    ("urn:schemas-microsoft-com:office:office", "o"),
];

fn prefix_for(uri: Option<&str>) -> Option<&'static str> {
    let uri = uri?;
    Some(
        NAMESPACES
            .iter()
            .find(|(u, _)| *u == uri)
            .map(|(_, p)| *p)
            .unwrap_or("x"),
    )
}

pub fn parse(text: &str) -> Result<Rc<XNode>, String> {
    let opts = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let doc = roxmltree::Document::parse_with_options(text, opts).map_err(|e| e.to_string())?;
    let mut out = convert(doc.root_element());
    // A root element is never AlternateContent; take the single converted node.
    Ok(Rc::new(out.remove(0)))
}

thread_local! {
    /// Set while [`parse_word`] runs: AlternateContent keeps its `mc:Choice`.
    static PREFER_CHOICE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Extensions a WordprocessingML `mc:Choice` may require and still be kept.
const WORD_CHOICES: [&str; 8] = ["wps", "wpg", "wpc", "wp14", "w14", "w15", "a14", "v"];

/// [`parse`] for WordprocessingML parts. Word's `mc:Fallback` is usually VML
/// (`w:pict`), so an AlternateContent keeps its first `mc:Choice` whose
/// `Requires` prefixes are all in [`WORD_CHOICES`], else its fallback.
pub fn parse_word(text: &str) -> Result<Rc<XNode>, String> {
    PREFER_CHOICE.with(|c| c.set(true));
    let out = parse(text);
    PREFER_CHOICE.with(|c| c.set(false));
    out
}

/// The branch [`parse_word`] keeps from an AlternateContent.
fn word_branch<'a, 'i>(node: roxmltree::Node<'a, 'i>) -> Option<roxmltree::Node<'a, 'i>> {
    let elements = || node.children().filter(|c| c.is_element());
    elements()
        .find(|c| {
            c.tag_name().name() == "Choice"
                && c.attribute("Requires")
                    .is_some_and(|r| r.split_whitespace().all(|p| WORD_CHOICES.contains(&p)))
        })
        .or_else(|| elements().find(|c| c.tag_name().name() == "Fallback"))
}

/// Converts one element; returns several nodes when it was an AlternateContent
/// whose chosen branch held several children.
fn convert(node: roxmltree::Node) -> Vec<XNode> {
    let tag = node.tag_name();
    let prefix = prefix_for(tag.namespace());
    if prefix == Some("mc") && tag.name() == "AlternateContent" && PREFER_CHOICE.with(|c| c.get()) {
        return match word_branch(node) {
            Some(b) => b
                .children()
                .filter(|c| c.is_element())
                .flat_map(convert)
                .collect(),
            None => Vec::new(),
        };
    }
    if prefix == Some("mc") && tag.name() == "AlternateContent" {
        let element = |n: &str| {
            node.children()
                .find(|c| c.is_element() && c.tag_name().name() == n)
        };
        // Text with equations (a14:m): the fallback is a picture of the whole text
        // box. Keep it for the pixels, and add the choice under `x:textOnly` so its
        // text can be laid out invisibly for extraction.
        if let (Some(choice), Some(fallback)) = (element("Choice"), element("Fallback")) {
            // chartEx frames (waterfall, treemap, ...) are drawn natively; the
            // fallback picture rides along under `x:fallback` for unsupported layouts.
            let chartex = choice.descendants().any(|d| {
                d.is_element()
                    && d.tag_name().name() == "chart"
                    && prefix_for(d.tag_name().namespace()) == Some("cx")
            });
            if chartex {
                let mut out: Vec<XNode> = choice
                    .children()
                    .filter(|c| c.is_element())
                    .flat_map(convert)
                    .collect();
                let fb: Vec<Rc<XNode>> = fallback
                    .children()
                    .filter(|c| c.is_element())
                    .flat_map(convert)
                    .map(Rc::new)
                    .collect();
                if let Some(first) = out.first_mut() {
                    first.children.push(Rc::new(XNode {
                        name: "x:fallback".into(),
                        attrs: Vec::new(),
                        children: fb,
                        text: String::new(),
                    }));
                }
                return out;
            }
            let has_math = choice.descendants().any(|d| {
                d.is_element()
                    && d.tag_name().name() == "m"
                    && prefix_for(d.tag_name().namespace()) == Some("a14")
            });
            if has_math {
                let mut out: Vec<XNode> = fallback
                    .children()
                    .filter(|c| c.is_element())
                    .flat_map(convert)
                    .collect();
                let text_only: Vec<Rc<XNode>> = choice
                    .children()
                    .filter(|c| c.is_element())
                    .flat_map(convert)
                    .map(Rc::new)
                    .collect();
                out.push(XNode {
                    name: "x:textOnly".into(),
                    attrs: Vec::new(),
                    children: text_only,
                    text: String::new(),
                });
                return out;
            }
        }
        let branch = node
            .children()
            .find(|c| c.is_element() && c.tag_name().name() == "Fallback")
            .or_else(|| {
                node.children()
                    .find(|c| c.is_element() && c.tag_name().name() == "Choice")
            });
        return match branch {
            Some(b) => b
                .children()
                .filter(|c| c.is_element())
                .flat_map(convert)
                .collect(),
            None => Vec::new(),
        };
    }
    let name = match prefix {
        Some(p) => format!("{p}:{}", tag.name()),
        None => tag.name().to_string(),
    };
    let attrs = node
        .attributes()
        .map(|a| (attr_key(&a), a.value().to_string()))
        .collect();
    let mut children = Vec::new();
    let mut text = String::new();
    for child in node.children() {
        if child.is_element() {
            children.extend(convert(child).into_iter().map(Rc::new));
        } else if child.is_text() {
            text.push_str(child.text().unwrap_or(""));
        }
    }
    if !children.is_empty() && text.trim().is_empty() {
        text.clear();
    }
    vec![XNode {
        name,
        attrs,
        children,
        text,
    }]
}

/// Canonical attribute key: a few namespaces keep their prefix, the rest go bare.
fn attr_key(a: &roxmltree::Attribute) -> String {
    match prefix_for(a.namespace()) {
        Some(p) if p == "r" || p == "x" || p == "a14" || p == "p14" || p == "o" => {
            format!("{p}:{}", a.name())
        }
        Some(_) => a.name().to_string(),
        None => a.name().to_string(),
    }
}

/// The root element's attribute `key` (canonical naming, as [`parse`] gives it)
/// without building the tree: `None` when absent or when the text is not
/// well-formed. A text where `key` never appears as an attribute name is not parsed.
pub fn root_attr(text: &str, key: &str) -> Option<String> {
    let local = key.rsplit(':').next().unwrap_or(key);
    if !mentions_attr(text, local) {
        return None;
    }
    let opts = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let doc = roxmltree::Document::parse_with_options(text, opts).ok()?;
    let root = doc.root_element();

    root.attributes()
        .find(|a| attr_key(a) == key)
        .map(|a| a.value().to_string())
}

/// Whether `name` occurs as a whole attribute local name: preceded by whitespace or
/// a prefix colon and followed by optional whitespace and `=`.
fn mentions_attr(text: &str, name: &str) -> bool {
    let b = text.as_bytes();
    text.match_indices(name).any(|(i, _)| {
        let before = i.checked_sub(1).map(|j| b[j]);
        let ok_before = matches!(before, Some(c) if c == b':' || c.is_ascii_whitespace());
        let rest = &b[i + name.len()..];
        let eq = rest.iter().find(|c| !c.is_ascii_whitespace());
        ok_before && eq == Some(&b'=')
    })
}

impl XNode {
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn child(&self, name: &str) -> Option<&Rc<XNode>> {
        self.children.iter().find(|c| c.name == name)
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Rc<XNode>> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// Follows a path of child names: `node.path(&["p:spPr", "a:xfrm"])`.
    pub fn path(&self, names: &[&str]) -> Option<&Rc<XNode>> {
        let (first, rest) = names.split_first()?;
        let mut cur = self.child(first)?;
        for n in rest {
            cur = cur.child(n)?;
        }
        Some(cur)
    }

    /// Depth-first search for the first descendant with `name`.
    pub fn find(&self, name: &str) -> Option<&Rc<XNode>> {
        for c in &self.children {
            if c.name == name {
                return Some(c);
            }
            if let Some(f) = c.find(name) {
                return Some(f);
            }
        }
        None
    }

    pub fn find_all<'a>(&'a self, name: &str, out: &mut Vec<&'a Rc<XNode>>) {
        for c in &self.children {
            if c.name == name {
                out.push(c);
            }
            c.find_all(name, out);
        }
    }

    pub fn local(&self) -> &str {
        self.name.rsplit(':').next().unwrap_or(&self.name)
    }

    pub fn attr_i64(&self, key: &str) -> Option<i64> {
        self.attr(key).and_then(parse_i64)
    }

    pub fn attr_f64(&self, key: &str) -> Option<f64> {
        self.attr(key).and_then(|v| v.trim().parse::<f64>().ok())
    }

    /// OOXML booleans: `1`/`true`/`on` and `0`/`false`/`off`.
    pub fn attr_bool(&self, key: &str) -> Option<bool> {
        parse_bool(self.attr(key)?)
    }

    /// All `a:t` text below this node, concatenated in document order.
    pub fn all_text(&self) -> String {
        let mut out = String::new();
        collect_text(self, &mut out);
        out
    }
}

fn collect_text(node: &XNode, out: &mut String) {
    if node.name == "a:t" || node.name == "m:t" {
        out.push_str(&node.text);
    }
    for c in &node.children {
        collect_text(c, out);
    }
}

/// OOXML booleans: `1`/`true`/`on` and `0`/`false`/`off`.
pub fn parse_bool(v: &str) -> Option<bool> {
    match v {
        "1" | "true" | "on" | "t" => Some(true),
        "0" | "false" | "off" | "f" => Some(false),
        _ => None,
    }
}

/// Integers in OOXML are usually plain, but some producers write decimals
/// (`12700.5`) or percentages with a `%` suffix in strict files.
pub fn parse_i64(v: &str) -> Option<i64> {
    let t = v.trim();
    if let Ok(i) = t.parse::<i64>() {
        return Some(i);
    }
    if let Some(p) = t.strip_suffix('%') {
        return p
            .trim()
            .parse::<f64>()
            .ok()
            .map(|f| (f * 1000.0).round() as i64);
    }
    t.parse::<f64>().ok().map(|f| f.round() as i64)
}

#[cfg(test)]
mod tests;
