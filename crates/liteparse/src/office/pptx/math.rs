//! Office Math (OMML, `m:oMath`) to linear text, for extraction. The result
//! follows the conventions of Office's own linear format: `(a+b)/c`, `x^2`,
//! `x_i`, `√(x)`, `∑_(i=1)^n▒x`, matrices as `[a,b;c,d]`.

use crate::office::pptx::xml::XNode;

/// Linear text of an equation container (`a14:m`, `m:oMathPara`, `m:oMath`).
pub fn linearize(node: &XNode) -> String {
    let mut out = String::new();
    let paras: Vec<&XNode> = collect(node, "m:oMath");
    for (i, m) in paras.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&children(m));
    }
    out.trim().to_string()
}

fn collect<'a>(node: &'a XNode, name: &str) -> Vec<&'a XNode> {
    if node.name == name {
        return vec![node];
    }
    node.children
        .iter()
        .flat_map(|c| collect(c, name))
        .collect()
}

fn children(node: &XNode) -> String {
    node.children.iter().map(|c| expr(c)).collect()
}

fn child_text(node: &XNode, name: &str) -> String {
    node.child(name).map(|c| children(c)).unwrap_or_default()
}

/// Wraps an operand in parentheses unless it is a single symbol or number.
fn group(s: String) -> String {
    let simple = s.chars().count() <= 1 || s.chars().all(|c| c.is_alphanumeric() || c == '.');
    if simple { s } else { format!("({s})") }
}

fn prop<'a>(node: &'a XNode, pr: &str, key: &str) -> Option<&'a str> {
    node.child(pr)
        .and_then(|p| p.child(key))
        .and_then(|k| k.attr("val"))
}

fn expr(node: &XNode) -> String {
    match node.name.as_str() {
        "m:r" => node.children_named("m:t").map(|t| t.text.clone()).collect(),
        "m:f" => format!(
            "{}/{}",
            group(child_text(node, "m:num")),
            group(child_text(node, "m:den"))
        ),
        "m:sSup" => format!(
            "{}^{}",
            child_text(node, "m:e"),
            group(child_text(node, "m:sup"))
        ),
        "m:sSub" => format!(
            "{}_{}",
            child_text(node, "m:e"),
            group(child_text(node, "m:sub"))
        ),
        "m:sSubSup" => format!(
            "{}_{}^{}",
            child_text(node, "m:e"),
            group(child_text(node, "m:sub")),
            group(child_text(node, "m:sup"))
        ),
        "m:sPre" => format!(
            "_{}^{}{}",
            group(child_text(node, "m:sub")),
            group(child_text(node, "m:sup")),
            child_text(node, "m:e")
        ),
        "m:rad" => {
            let deg = child_text(node, "m:deg");
            let e = child_text(node, "m:e");
            if deg.trim().is_empty() {
                format!("√{}", group(e))
            } else {
                format!("√({deg}&{e})")
            }
        }
        "m:d" => {
            let beg = prop(node, "m:dPr", "m:begChr").unwrap_or("(");
            let end = prop(node, "m:dPr", "m:endChr").unwrap_or(")");
            let sep = prop(node, "m:dPr", "m:sepChr").unwrap_or("|");
            let parts: Vec<String> = node.children_named("m:e").map(|e| children(e)).collect();
            format!("{beg}{}{end}", parts.join(sep))
        }
        "m:nary" => {
            let chr = prop(node, "m:naryPr", "m:chr").unwrap_or("∫");
            let sub = child_text(node, "m:sub");
            let sup = child_text(node, "m:sup");
            let mut s = chr.to_string();
            if !sub.trim().is_empty() {
                s.push('_');
                s.push_str(&group(sub));
            }
            if !sup.trim().is_empty() {
                s.push('^');
                s.push_str(&group(sup));
            }
            s.push('▒');
            s.push_str(&child_text(node, "m:e"));
            s
        }
        "m:func" => format!(
            "{} {}",
            child_text(node, "m:fName").trim(),
            child_text(node, "m:e")
        ),
        "m:acc" => {
            let chr = prop(node, "m:accPr", "m:chr").unwrap_or("\u{0302}");
            format!("{}{chr}", child_text(node, "m:e"))
        }
        "m:bar" => format!("{}\u{0305}", child_text(node, "m:e")),
        "m:limLow" => format!(
            "{}_{}",
            child_text(node, "m:e"),
            group(child_text(node, "m:lim"))
        ),
        "m:limUpp" => format!(
            "{}^{}",
            child_text(node, "m:e"),
            group(child_text(node, "m:lim"))
        ),
        "m:groupChr" => child_text(node, "m:e"),
        "m:m" => {
            let rows: Vec<String> = node
                .children_named("m:mr")
                .map(|r| {
                    r.children_named("m:e")
                        .map(|e| children(e))
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .collect();
            format!("[{}]", rows.join(";"))
        }
        "m:eqArr" => node
            .children_named("m:e")
            .map(|e| children(e))
            .collect::<Vec<_>>()
            .join("; "),
        // Property elements carry no text.
        n if n.ends_with("Pr") => String::new(),
        _ => children(node),
    }
}

#[cfg(test)]
mod tests;
