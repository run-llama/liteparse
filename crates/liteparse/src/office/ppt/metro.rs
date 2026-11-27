//! PowerPoint 2007+ stores a shape that the binary format cannot express (3-D
//! effects, some tables and placeholders) twice: a fallback (often a picture) and,
//! in the shape's `metroBlob` property (0x03A9), a small OPC zip whose
//! `drs/shapexml.xml` is the original DrawingML shape. PowerPoint renders the
//! zip copy when it re-opens the .ppt, so the converter emits that XML.
//!
//! The XML's text is masked: every character is replaced by `_` (spaces are kept).
//! [`fill_text`] puts the shape's binary text back, character for character.

use crate::office::ppt::records::{self as r, Rec};
use std::io::Read;

const METRO_BLOB: u16 = 0x03A9;
/// Largest inflated part read from a blob.
const MAX_PART_BYTES: u64 = 32 << 20;

/// A relationship the shape XML uses (`r:embed`, `r:id`, `r:link`).
pub struct Rel {
    pub id: String,
    /// Full relationship type URI.
    pub kind: String,
    pub target: Target,
}

pub enum Target {
    /// A part of the blob, as a path below its `drs/` folder.
    Part(String),
    External(String),
}

pub struct MetroShape {
    /// The shape element (`p:sp`, `p:pic`, `p:cxnSp`, `p:graphicFrame`) without the
    /// XML declaration.
    pub xml: String,
    /// True for a graphic frame (table, chart, SmartArt): its text is not masked,
    /// and it replaces the binary group that stands in for it.
    pub frame: bool,
    pub rels: Vec<Rel>,
    /// Every other part below `drs/` (images, chart and diagram parts with their own
    /// relationships), by path below `drs/`. The writer copies them as one folder so
    /// relative targets between them keep working.
    pub parts: Vec<(String, Vec<u8>)>,
}

/// Main part of a blob for each kind of shape; `drs/e2oDoc.xml` holds a graphic frame.
const MAIN_PARTS: [&str; 4] = [
    "drs/shapexml.xml",
    "drs/connectorxml.xml",
    "drs/picturexml.xml",
    "drs/e2oDoc.xml",
];

/// The metroBlob bytes of an OfficeArtFOPT-style property table (OPT, secondary or
/// tertiary OPT). Each property is a u16 id (complex flag 0x8000) and a u32 value;
/// complex values' bytes follow the table in property order, `value` bytes each.
pub fn blob(data: &[u8], opt: Rec) -> Option<&[u8]> {
    let count = opt.inst as usize;
    let mut extra = opt.start.checked_add(6 * count)?;
    for i in 0..count {
        let id = r::u16_at(data, opt.start + 6 * i)?;
        let value = r::u32_at(data, opt.start + 6 * i + 2)? as usize;
        if id & 0x8000 == 0 {
            continue;
        }
        let end = extra.checked_add(value)?.min(opt.end);
        if id & 0x3FFF == METRO_BLOB {
            return data.get(extra..end);
        }
        extra = end;
    }
    None
}

/// Reads the shape XML and the parts it references. `Ok(None)` for a blob without
/// one of [`MAIN_PARTS`] (PowerPoint writes empty ones, and group shapes'
/// `groupshapexml.xml`, whose members also carry their own blobs).
pub fn read(blob: &[u8]) -> Result<Option<MetroShape>, String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(blob)).map_err(|e| e.to_string())?;
    let Some(name) = MAIN_PARTS
        .into_iter()
        .find(|n| zip.index_for_name(n).is_some())
    else {
        return Ok(None);
    };
    read_part(&mut zip, name).map(Some)
}

/// A round-trip atom's zip read like a metroBlob, with `name` as its main part (a
/// slide layout, for example).
pub fn read_named(blob: &[u8], name: &str) -> Result<Option<MetroShape>, String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(blob)).map_err(|e| e.to_string())?;
    if zip.index_for_name(name).is_none() {
        return Ok(None);
    }
    read_part(&mut zip, name).map(Some)
}

fn read_part(
    zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    name: &str,
) -> Result<MetroShape, String> {
    let xml = part(zip, name)?;
    let xml = String::from_utf8(xml).map_err(|_| "shape XML is not UTF-8".to_string())?;
    let mut body = match xml.find("?>") {
        Some(i) if xml.trim_start().starts_with("<?xml") => xml[i + 2..].trim_start(),
        _ => xml.trim_start(),
    }
    .to_string();
    let frame = name == "drs/e2oDoc.xml";
    if frame {
        body = body.replacen(":E2oFrame", ":graphicFrame", 1);
        if let Some(i) = body.rfind(":E2oFrame>") {
            body.replace_range(i..i + 9, ":graphicFrame");
        }
    }
    let (dir, file) = name.rsplit_once('/').unwrap_or(("", name));
    let rels_name = format!("{dir}/_rels/{file}.rels");
    let mut rels = Vec::new();
    if let Ok(rels_xml) = part(zip, &rels_name) {
        let rels_xml = String::from_utf8_lossy(&rels_xml).into_owned();
        for rel in rels_xml.split("<Relationship ").skip(1) {
            let (Some(id), Some(kind), Some(target)) =
                (attr(rel, "Id"), attr(rel, "Type"), attr(rel, "Target"))
            else {
                continue;
            };
            let target = if attr(rel, "TargetMode").as_deref() == Some("External") {
                Target::External(target)
            } else {
                let path = resolve(&format!("{dir}/"), &target);
                match path.strip_prefix("drs/") {
                    Some(p) => Target::Part(p.to_string()),
                    None => continue,
                }
            };
            rels.push(Rel { id, kind, target });
        }
    }
    let mut parts = Vec::new();
    let names: Vec<String> = zip
        .file_names()
        .map(|n| n.map(String::from))
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    for n in names {
        let Some(rest) = n.strip_prefix("drs/") else {
            continue;
        };
        if n == name || n == rels_name || rest == "downrev.xml" || n.ends_with('/') {
            continue;
        }
        parts.push((rest.to_string(), part(zip, &n)?));
    }
    Ok(MetroShape {
        xml: body,
        frame,
        rels,
        parts,
    })
}

const DRAWINGML: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

/// The shape XML with its masked `a:t` text replaced by `text`, the shape's binary
/// text (`\r` between paragraphs, U+000B for `a:br`). Each `a:t` character takes the
/// next binary character; a field (`a:fld`) keeps its own text and consumes the
/// binary `*` that stands for it. XML whose text is not masked comes back as it is.
/// `None` when the two do not line up.
pub fn fill_text(xml: &str, text: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let masked = doc
        .descendants()
        .filter(|n| n.is_text() && n.parent().is_some_and(|p| p.tag_name().name() == "t"))
        .all(|n| {
            n.text()
                .unwrap_or("")
                .chars()
                .all(|c| c == '_' || c.is_whitespace())
        });
    if !masked {
        return Some(xml.to_string());
    }
    let mut src = text.chars().peekable();
    let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    let paras: Vec<_> = doc
        .descendants()
        .filter(|n| n.tag_name().namespace() == Some(DRAWINGML) && n.tag_name().name() == "p")
        .collect();
    for (i, p) in paras.iter().enumerate() {
        if i > 0 {
            match src.next() {
                Some('\r') => {}
                _ => return None,
            }
        }
        for run in p.children().filter(|n| n.is_element()) {
            match run.tag_name().name() {
                "r" => {
                    let Some(t) = run.children().find(|n| n.tag_name().name() == "t") else {
                        continue;
                    };
                    let Some(node) = t.first_child().filter(|n| n.is_text()) else {
                        continue;
                    };
                    let mut out = String::new();
                    for _ in node.text()?.chars() {
                        out.push(src.next().filter(|&c| c != '\r')?);
                    }
                    if !out.contains('\u{b}') {
                        edits.push((node.range(), escape(&out)));
                        continue;
                    }
                    // A line break the XML kept as a masked character in the run:
                    // split the run around an `a:br` with the run's properties.
                    let prefix = run
                        .tag_name()
                        .namespace()
                        .and_then(|ns| run.lookup_prefix(ns));
                    let p = prefix.map_or(String::new(), |p| format!("{p}:"));
                    let rpr = run
                        .children()
                        .find(|n| n.tag_name().name() == "rPr")
                        .map_or("", |n| &xml[n.range()]);
                    let mut split = String::new();
                    for (i, piece) in out.split('\u{b}').enumerate() {
                        if i > 0 {
                            split.push_str(&format!("<{p}br>{rpr}</{p}br>"));
                        }
                        if !piece.is_empty() {
                            split.push_str(&format!(
                                "<{p}r>{rpr}<{p}t>{}</{p}t></{p}r>",
                                escape(piece)
                            ));
                        }
                    }
                    edits.push((run.range(), split));
                }
                "fld" if src.peek() == Some(&'*') => {
                    src.next();
                }
                "br" if src.next() != Some('\u{b}') => return None,
                _ => {}
            }
        }
    }
    // A trailing paragraph mark ends the last paragraph.
    if src.peek() == Some(&'\r') {
        src.next();
    }
    if src.next().is_some() {
        return None;
    }
    let mut out = xml.to_string();
    for (range, text) in edits.into_iter().rev() {
        out.replace_range(range, &text);
    }
    Some(out)
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c if (c as u32) < 0x20 && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

fn part(zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>, String> {
    let f = zip.by_name(name).map_err(|e| format!("{name}: {e}"))?;
    let mut out = Vec::new();
    f.take(MAX_PART_BYTES)
        .read_to_end(&mut out)
        .map_err(|e| format!("{name}: {e}"))?;
    Ok(out)
}

/// Value of `name="..."` in a tag's attribute text, with the five XML entities decoded.
fn attr(tag: &str, name: &str) -> Option<String> {
    let padded = format!(" {tag}");
    let key = format!(" {name}=\"");
    let start = padded.find(&key)? + key.len();
    let rest = &padded[start..];
    let value = &rest[..rest.find('"')?];
    Some(
        value
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&amp;", "&"),
    )
}

/// Resolves a relative target against the folder of its source part.
fn resolve(base: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let mut parts: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
    for seg in target.split('/') {
        match seg {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            s => parts.push(s),
        }
    }
    parts.join("/")
}
