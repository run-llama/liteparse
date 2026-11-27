//! Fonts embedded in the presentation (`p:embeddedFontLst` → `ppt/fonts/*.fntdata`).
//!
//! Each `p:embeddedFont` names a typeface and points up to four parts (regular,
//! bold, italic, bold italic) at EOT files. They are decoded ([`crate::office::pptx::eot`]) and
//! registered for this document only, under the typeface and slot the deck
//! declares; PowerPoint uses them only when that family is not installed, and so
//! does [`FontSystem::resolve`].

use crate::office::pptx::document::Document;
use crate::office::pptx::fonts::FontSystem;

const SLOTS: [(&str, bool, bool); 4] = [
    ("p:regular", false, false),
    ("p:bold", true, false),
    ("p:italic", false, true),
    ("p:boldItalic", true, true),
];

/// Registers every decodable embedded font. Returns `(part, reason)` for each
/// one that could not be used; those never fail the document.
pub fn register(doc: &Document, fonts: &FontSystem) -> Vec<(String, String)> {
    let mut failures = Vec::new();
    let Some(list) = doc.pres.child("p:embeddedFontLst") else {
        return failures;
    };
    for font in list.children_named("p:embeddedFont") {
        let Some(typeface) = font.child("p:font").and_then(|f| f.attr("typeface")) else {
            continue;
        };
        for (slot, bold, italic) in SLOTS {
            let Some(rid) = font.child(slot).and_then(|n| n.attr("r:id")) else {
                continue;
            };
            let Some(rel) = doc.pkg.rel_by_id(&doc.pres_part, rid) else {
                failures.push((format!("{typeface} ({rid})"), "missing relationship".into()));
                continue;
            };
            // Google Slides exports point an absent face at the literal target
            // "NULL"; that slot is simply not embedded.
            if rel.target == "NULL" || rel.target.ends_with("/NULL") {
                continue;
            }
            let part = rel.target;
            let decoded = doc
                .pkg
                .read(&part)
                .map_err(|e| e.to_string())
                .and_then(|bytes| crate::office::pptx::eot::decode(&bytes));
            match decoded {
                Ok(sfnt) => {
                    if !fonts.add_embedded(typeface, bold, italic, sfnt) {
                        failures.push((part, "not a usable font".into()));
                    }
                }
                Err(e) => failures.push((part, e)),
            }
        }
    }
    failures
}

#[cfg(test)]
mod tests;
