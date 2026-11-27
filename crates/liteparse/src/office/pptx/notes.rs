//! Speaker notes and slide comments.

use serde_json::{Value, json};

use crate::office::pptx::document::Document;
use crate::office::pptx::xml::XNode;

/// Speaker notes as `OpenXMLSpeakerNote` content: one string per paragraph,
/// with `* ` / `N. ` list prefixes indented 4 spaces per level. Only the notes
/// body placeholder is read, so the slide-number field is left out.
pub fn speaker_notes(doc: &Document, slide_part: &str) -> Option<Value> {
    let notes_part = doc.pkg.rel_by_kind(slide_part, "notesSlide")?.target;
    let notes = doc.xml(&notes_part)?;
    let tree = notes.path(&["p:cSld", "p:spTree"])?;
    let mut content: Vec<String> = Vec::new();
    let mut bodies = Vec::new();
    collect_bodies(tree, &mut bodies);
    for tx in bodies {
        let mut numbering: Vec<u32> = vec![1];
        for p in tx.children_named("a:p") {
            let ppr = p.child("a:pPr");
            let level = ppr
                .and_then(|x| x.attr_i64("lvl"))
                .filter(|l| *l >= 0)
                .unwrap_or(0) as usize;
            numbering.resize(level + 1, 1);
            let indent = " ".repeat(4 * level.saturating_sub(1));
            let mut prefix = String::new();
            let mut numbered = false;
            if let Some(ppr) = ppr {
                if let Some(auto) = ppr.child("a:buAutoNum") {
                    numbered = true;
                    if let Some(start) = auto.attr_i64("startAt") {
                        numbering[level] = start.max(0) as u32;
                    }
                    prefix = format!("{indent}{}. ", numbering[level]);
                } else if ppr.child("a:buChar").is_some() || ppr.child("a:buBlip").is_some() {
                    prefix = format!("{indent}* ");
                }
            }
            numbering[level] = if numbered { numbering[level] + 1 } else { 1 };
            let text: String = p
                .children
                .iter()
                .map(|r| match r.name.as_str() {
                    "a:r" | "a:fld" => r.child("a:t").map(|t| t.text.clone()).unwrap_or_default(),
                    "a:br" => "\n".to_string(),
                    _ => String::new(),
                })
                .collect();
            content.push(format!("{prefix}{}", text.trim()));
        }
    }
    while content.last().map(|s| s.is_empty()).unwrap_or(false) {
        content.pop();
    }
    if content.iter().all(|s| s.trim().is_empty()) {
        return None;
    }
    Some(json!({ "content": content }))
}

fn collect_bodies<'a>(tree: &'a XNode, out: &mut Vec<&'a XNode>) {
    for c in &tree.children {
        match c.local() {
            "sp" => {
                let ph_type = c
                    .path(&["p:nvSpPr", "p:nvPr", "p:ph"])
                    .map(|ph| ph.attr("type").unwrap_or("body"));
                if ph_type == Some("body")
                    && let Some(tx) = c.child("p:txBody")
                {
                    out.push(tx);
                }
            }
            "grpSp" => collect_bodies(c, out),
            _ => {}
        }
    }
}

/// Slide comments as `Text` annotations (legacy `p:cmLst` and modern threaded
/// comments).
pub fn comments(doc: &Document, slide_part: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for rel in doc.pkg.rels(slide_part).iter() {
        let kind = rel.kind();
        if kind != "comments" && !rel.rel_type.ends_with("/comments") {
            continue;
        }
        let Some(root) = doc.xml(&rel.target) else {
            continue;
        };
        for cm in &root.children {
            if cm.local() != "cm" {
                continue;
            }
            let text = cm
                .child("p:text")
                .map(|t| t.text.clone())
                .unwrap_or_else(|| {
                    // Modern comments keep their text in a txBody.
                    cm.find("p188:txBody")
                        .map(|t| crate::office::pptx::slide::para_texts(t))
                        .unwrap_or_default()
                });
            if text.trim().is_empty() {
                continue;
            }
            let author_id = cm.attr("authorId").unwrap_or("");
            let mut v = json!({ "subtype": "Text", "contents": text });
            if let Some(name) = author_name(doc, author_id) {
                v["title"] = json!(name);
            }
            if let Some(dt) = cm.attr("dt").or(cm.attr("created")) {
                v["created"] = json!(dt);
            }
            out.push(v);
        }
    }
    out
}

fn author_name(doc: &Document, id: &str) -> Option<String> {
    for kind in ["commentAuthors", "authors"] {
        let Some(rel) = doc.pkg.rel_by_kind(&doc.pres_part, kind) else {
            continue;
        };
        let Some(root) = doc.xml(&rel.target) else {
            continue;
        };
        for a in &root.children {
            if a.attr("id") == Some(id) {
                return a.attr("name").map(str::to_string);
            }
        }
    }
    None
}
