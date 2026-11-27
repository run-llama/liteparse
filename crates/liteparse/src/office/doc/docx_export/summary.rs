//! Document properties from the OLE property-set streams, written as
//! `docProps/core.xml`, `docProps/app.xml` and `docProps/custom.xml`.
//!
//! `\x05SummaryInformation` holds one section (title, subject, author...).
//! `\x05DocumentSummaryInformation` holds the built-in section (category,
//! manager, company) and, second, the user-defined section whose property 0 is
//! a dictionary of names. Word recomputes header and footer TITLE and
//! DOCPROPERTY fields from these values, so the renderer needs them.

use super::package::Part;
use super::props::esc;
use super::source::{i16_le, u16_le, u32_le};
use crate::office::doc::compound::Compound;

/// Property values: core.xml elements, app.xml elements and named custom values.
#[derive(Debug, Default, PartialEq)]
pub struct Properties {
    pub core: Vec<(&'static str, String)>,
    pub app: Vec<(&'static str, String)>,
    pub custom: Vec<(String, String)>,
}

pub fn read(data: &[u8]) -> Properties {
    let mut out = Properties::default();
    let Ok(c) = Compound::open(data) else {
        return out;
    };
    if let Ok(s) = c.stream("\u{5}SummaryInformation")
        && let Some(sec) = sections(&s).first()
    {
        for (pid, v) in values(sec) {
            if pid == 0x12 {
                out.app.push(("Application", v));
                continue;
            }
            let key = match pid {
                2 => "dc:title",
                3 => "dc:subject",
                4 => "dc:creator",
                5 => "cp:keywords",
                6 => "dc:description",
                8 => "cp:lastModifiedBy",
                _ => continue,
            };
            out.core.push((key, v));
        }
    }
    if let Ok(s) = c.stream("\u{5}DocumentSummaryInformation") {
        let secs = sections(&s);
        if let Some(sec) = secs.first() {
            for (pid, v) in values(sec) {
                match pid {
                    2 => out.core.push(("cp:category", v)),
                    0x0e => out.app.push(("Manager", v)),
                    0x0f => out.app.push(("Company", v)),
                    _ => {}
                }
            }
        }
        if let Some(sec) = secs.get(1) {
            let names = dictionary(sec);
            for (pid, v) in values(sec) {
                if let Some((_, name)) = names.iter().find(|(p, _)| *p == pid) {
                    out.custom.push((name.clone(), v));
                }
            }
        }
    }
    out
}

/// The package parts for `p`, with their root relationships (type, target).
pub fn parts(p: &Properties) -> (Vec<Part>, Vec<(&'static str, &'static str)>) {
    const REL: &str = "http://schemas.openxmlformats.org";
    let mut parts = Vec::new();
    let mut rels = Vec::new();
    if !p.core.is_empty() {
        let mut x = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" \
             xmlns:dc=\"http://purl.org/dc/elements/1.1/\">",
        );
        for (k, v) in &p.core {
            x.push_str(&format!("<{k}>{}</{k}>", esc(v)));
        }
        x.push_str("</cp:coreProperties>");
        parts.push(Part {
            name: "docProps/core.xml".into(),
            bytes: x.into_bytes(),
        });
        rels.push((
            "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties",
            "docProps/core.xml",
        ));
    }
    if !p.app.is_empty() {
        let mut x = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <Properties xmlns=\"{REL}/officeDocument/2006/extended-properties\">"
        );
        for (k, v) in &p.app {
            x.push_str(&format!("<{k}>{}</{k}>", esc(v)));
        }
        x.push_str("</Properties>");
        parts.push(Part {
            name: "docProps/app.xml".into(),
            bytes: x.into_bytes(),
        });
        rels.push((
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties",
            "docProps/app.xml",
        ));
    }
    if !p.custom.is_empty() {
        let mut x = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <Properties xmlns=\"{REL}/officeDocument/2006/custom-properties\" \
             xmlns:vt=\"{REL}/officeDocument/2006/docPropsVTypes\">"
        );
        // Custom property ids start at 2.
        for (i, (k, v)) in p.custom.iter().enumerate() {
            x.push_str(&format!(
                "<property fmtid=\"{{D5CDD505-2E9C-101B-9397-08002B2CF9AE}}\" pid=\"{}\" name=\"{}\"><vt:lpwstr>{}</vt:lpwstr></property>",
                i + 2,
                esc(k),
                esc(v)
            ));
        }
        x.push_str("</Properties>");
        parts.push(Part {
            name: "docProps/custom.xml".into(),
            bytes: x.into_bytes(),
        });
        rels.push((
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties",
            "docProps/custom.xml",
        ));
    }
    (parts, rels)
}

/// The sections of a property-set stream: a 28-byte header whose last field
/// (offset 24) counts the sections, then a 16-byte FMTID and 4-byte offset each.
fn sections(s: &[u8]) -> Vec<&[u8]> {
    let n = u32_le(s, 24).unwrap_or(0).min(8) as usize;
    (0..n)
        .filter_map(|i| {
            let at = u32_le(s, 28 + i * 20 + 16)? as usize;
            let size = u32_le(s, at)? as usize;
            s.get(at..at.checked_add(size)?)
        })
        .collect()
}

/// (id, offset) pairs of a section: its size and count, then 8 bytes each.
fn entries(sec: &[u8]) -> impl Iterator<Item = (u32, usize)> + '_ {
    let n = u32_le(sec, 4).unwrap_or(0).min(1000) as usize;
    (0..n).filter_map(move |i| Some((u32_le(sec, 8 + i * 8)?, u32_le(sec, 12 + i * 8)? as usize)))
}

/// Property 1: the code page of 8-bit strings.
fn code_page(sec: &[u8]) -> u16 {
    entries(sec)
        .find(|(pid, _)| *pid == 1)
        .and_then(|(_, at)| i16_le(sec, at + 4))
        .map_or(1252, |cp| cp as u16)
}

/// String and number values of a section, by property id.
fn values(sec: &[u8]) -> Vec<(u32, String)> {
    let cp = code_page(sec);
    entries(sec)
        .filter(|(pid, _)| *pid > 1)
        .filter_map(|(pid, at)| {
            let v = match u16_le(sec, at)? {
                // VT_I2, VT_I4.
                0x02 => i16_le(sec, at + 4)?.to_string(),
                0x03 => (u32_le(sec, at + 4)? as i32).to_string(),
                // VT_LPSTR: a byte count, then the string in the section's code page.
                0x1e => {
                    let n = u32_le(sec, at + 4)? as usize;
                    ansi(sec.get(at + 8..at + 8 + n)?, cp)?
                }
                // VT_LPWSTR: a character count, then UTF-16.
                0x1f => {
                    let n = u32_le(sec, at + 4)? as usize;
                    utf16(sec.get(at + 8..at + 8 + n * 2)?)
                }
                _ => return None,
            };
            let v = v.trim_end_matches('\0').to_string();
            Some((pid, v))
        })
        .collect()
}

/// Property 0 of a user-defined section: (id, name) entries, each a 4-byte id,
/// a 4-byte character count and the name (UTF-16 padded to 4 bytes under code page 1200).
fn dictionary(sec: &[u8]) -> Vec<(u32, String)> {
    let cp = code_page(sec);
    let Some((_, mut at)) = entries(sec).find(|(pid, _)| *pid == 0) else {
        return Vec::new();
    };
    let n = u32_le(sec, at).unwrap_or(0).min(1000);
    at += 4;
    let mut out = Vec::new();
    for _ in 0..n {
        let (Some(pid), Some(len)) = (u32_le(sec, at), u32_le(sec, at + 4)) else {
            break;
        };
        let len = len as usize;
        at += 8;
        let name = if cp == 1200 {
            let s = sec.get(at..at + len * 2).map(utf16);
            at += (len * 2).next_multiple_of(4);
            s
        } else {
            let s = sec.get(at..at + len).and_then(|b| ansi(b, cp));
            at += len;
            s
        };
        if let Some(name) = name {
            out.push((pid, name.trim_end_matches('\0').to_string()));
        }
    }
    out
}

fn utf16(b: &[u8]) -> String {
    let units: Vec<u16> = b
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u16::from_le_bytes([p[0], p[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

/// An 8-bit string under code page `cp`: Windows-1252, UTF-8 or UTF-16, or
/// plain ASCII under any other code page.
fn ansi(b: &[u8], cp: u16) -> Option<String> {
    match cp {
        1200 => Some(utf16(b)),
        65001 => String::from_utf8(b.to_vec()).ok(),
        1252 => Some(
            b.iter()
                .map(|&c| char::from_u32(super::source::compressed_unit(c) as u32).unwrap_or('?'))
                .collect(),
        ),
        _ => b.is_ascii().then(|| b.iter().map(|&c| c as char).collect()),
    }
}

#[cfg(test)]
mod tests;
