//! Document properties (`docProps/core.xml`, `app.xml`, `custom.xml`) and the
//! header and footer fields that show them. Word recomputes TITLE, SUBJECT,
//! AUTHOR, KEYWORDS, COMMENTS and DOCPROPERTY fields in headers and footers when
//! it lays out pages, so their saved results can be stale.

use std::collections::HashMap;

use crate::office::pptx::package::Package;

/// Property values keyed by upper-case name: the built-in field names (TITLE,
/// AUTHOR...) and DOCPROPERTY names (built-in, app.xml and custom properties).
#[derive(Debug, Default)]
pub struct DocProps(HashMap<String, String>);

impl DocProps {
    pub fn read(pkg: &Package) -> DocProps {
        let mut map = HashMap::new();
        if let Some(core) = crate::office::docx::extract::read_xml(pkg, "docProps/core.xml") {
            for c in &core.children {
                let key = match c.local() {
                    "title" => "TITLE",
                    "subject" => "SUBJECT",
                    "creator" => "AUTHOR",
                    "keywords" => "KEYWORDS",
                    "description" => "COMMENTS",
                    "lastModifiedBy" => "LASTSAVEDBY",
                    "category" => "CATEGORY",
                    _ => continue,
                };
                map.insert(key.to_string(), c.text.clone());
            }
        }
        if let Some(app) = crate::office::docx::extract::read_xml(pkg, "docProps/app.xml") {
            for c in &app.children {
                if matches!(c.local(), "Company" | "Manager") {
                    map.insert(c.local().to_ascii_uppercase(), c.text.clone());
                }
            }
        }
        if let Some(custom) = crate::office::docx::extract::read_xml(pkg, "docProps/custom.xml") {
            for p in &custom.children {
                // The value is the property's one typed child (`vt:lpwstr`, `vt:i4`...).
                if let (Some(name), Some(v)) = (p.attr("name"), p.children.first()) {
                    map.insert(name.to_uppercase(), v.text.clone());
                }
            }
        }
        DocProps(map)
    }

    /// The text a header or footer property field shows, or `None` to keep its
    /// saved result (another field, a property that is missing or empty, or a
    /// TITLE-style field given a new value as its argument).
    pub fn field_text(&self, instr: &str) -> Option<String> {
        let mut words = instr.split_whitespace();
        let word = words.next()?.to_ascii_uppercase();
        let rest = instr.trim_start()[word.len()..].trim_start();
        let key = match word.as_str() {
            "TITLE" | "SUBJECT" | "AUTHOR" | "KEYWORDS" | "COMMENTS" | "LASTSAVEDBY" => {
                if !(rest.is_empty() || rest.starts_with('\\')) {
                    return None;
                }
                word
            }
            "DOCPROPERTY" => {
                let name = match rest.strip_prefix('"') {
                    Some(q) => &q[..q.find('"')?],
                    None => rest.split_whitespace().next()?,
                };
                // Built-in names map to their field names ("Author" is AUTHOR).
                match name.to_uppercase().as_str() {
                    "LASTAUTHOR" => "LASTSAVEDBY".to_string(),
                    n => n.to_string(),
                }
            }
            _ => return None,
        };
        let value = self.0.get(&key).filter(|v| !v.trim().is_empty())?;
        Some(apply_case(value, instr))
    }
}

/// The `\* Upper`, `\* Lower`, `\* Caps` and `\* FirstCap` text formats.
fn apply_case(value: &str, instr: &str) -> String {
    let mut words = instr.split_whitespace();
    let mut fmt = None;
    while let Some(w) = words.next() {
        if w == "\\*" {
            let w = words.next().map(str::to_ascii_lowercase);
            if let Some(w) =
                w.filter(|w| matches!(w.as_str(), "upper" | "lower" | "caps" | "firstcap"))
            {
                fmt = Some(w);
            }
        }
    }
    let cap = |w: &str| {
        let mut c = w.chars();
        c.next()
            .map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
    };
    match fmt.as_deref() {
        Some("upper") => value.to_uppercase(),
        Some("lower") => value.to_lowercase(),
        Some("caps") => value.split(' ').map(cap).collect::<Vec<_>>().join(" "),
        Some("firstcap") => cap(value),
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests;
