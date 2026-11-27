//! Legacy Word comment content, kept at document scope until anchors are mapped
//! through pagination. Annotation field names match the shared Text annotations.

use crate::office::pptx::{package::Package, xml::XNode};
use serde_json::{Value, json};

/// Read related comment parts in package order. Unreferenced comments and missing
/// ids retain their content; their presence does not imply a visible page anchor.
pub fn read(pkg: &Package, main: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for rel in pkg.rels(main).iter() {
        if rel.kind() != "comments" || rel.external {
            continue;
        }
        let Some(root) = crate::office::docx::extract::read_xml(pkg, &rel.target) else {
            continue;
        };
        if root.name != "w:comments" {
            continue;
        }
        for node in root.children_named("w:comment") {
            let mut text = String::new();
            comment_text(node, &mut text);
            // Paragraph separators are emitted after their content, including
            // empty paragraphs. Remove only the final structural separator.
            if text.ends_with('\n') {
                text.pop();
            }
            let mut value = json!({
                "subtype": "Text", "contents": text,
                "id": node.attr("id"), "sourcePart": rel.target,
            });
            for (source, target) in [
                ("author", "title"),
                ("initials", "initials"),
                ("date", "created"),
            ] {
                if let Some(v) = node.attr(source) {
                    value[target] = json!(v);
                }
            }
            out.push(value);
        }
    }
    out
}

/// Plain comment content with cached field results and final-view revisions.
/// Formatting, drawings and automatic list labels need a richer metadata model.
fn comment_text(node: &XNode, out: &mut String) {
    match node.name.as_str() {
        "w:t" => out.push_str(&node.text),
        "w:tab" => out.push('\t'),
        "w:br" | "w:cr" => out.push('\n'),
        "w:noBreakHyphen" => out.push('\u{2011}'),
        "w:softHyphen" => out.push('\u{00ad}'),
        "w:del" | "w:moveFrom" | "w:instrText" | "w:delText" => {}
        "m:oMath" | "m:oMathPara" => out.push_str(&crate::office::pptx::math::linearize(node)),
        name if name.ends_with("Pr") => {}
        _ => {
            for child in &node.children {
                comment_text(child, out);
            }
            if node.name == "w:p" {
                out.push('\n');
            }
        }
    }
}

#[cfg(test)]
mod tests;
