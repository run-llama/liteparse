//! Best-effort conversion of a binary Word document into a WordprocessingML package.
//!
//! The converter favours coverage over exactness: every readable story is written,
//! unsupported structures degrade to their text (or are skipped) and each fallback is
//! recorded in `Converted::diagnostics`. Layout is left to the DOCX renderer.
//! Only an unreadable container, an encrypted file or a pre-Word97 FIB fails.
pub mod blips;
mod body;
pub mod drawing;
mod package;
mod props;
mod source;
mod styles;
mod summary;
#[cfg(test)]
mod tests;

use crate::office::doc::Result;

/// Development dump: one line per paragraph with its CP range, style, flags and modifiers.
pub fn debug_paragraphs(data: &[u8]) -> Result<String> {
    let source = source::Source::open(data)?;
    let mut out = String::new();
    if std::env::var("DOC_DUMP_CHPX").is_ok()
        && let Some(t) = source
            .pair(source::pair::STTBFFFN)
            .and_then(|b| crate::office::doc::symbols::decode_font_table_lenient(b).ok())
    {
        for (i, f) in t.fonts.iter().enumerate() {
            out.push_str(&format!("font {i} {}\n", f.name));
        }
    }
    for p in &source.paras {
        let text: String = String::from_utf16_lossy(&source.text[p.start_cp..p.end_cp])
            .chars()
            .take(40)
            .map(|c| if (c as u32) < 0x20 { '·' } else { c })
            .collect();
        let sprms: Vec<String> = source::sprms(&p.grpprl)
            .map(|(s, o)| {
                if std::env::var("DOC_DUMP_BYTES").is_ok() {
                    format!("{s:04x}:{o:02x?}")
                } else {
                    format!("{s:04x}:{}", o.len())
                }
            })
            .collect();
        if std::env::var("DOC_DUMP_CHPX").is_ok() {
            // Every character run that overlaps the paragraph: cp range and sprms.
            for span in source
                .chars
                .iter()
                .filter(|s| s.end_cp > p.start_cp && s.start_cp < p.end_cp)
            {
                let chpx: Vec<String> = source::sprms(&span.grpprl)
                    .map(|(s, o)| format!("{s:04x}:{o:02x?}"))
                    .collect();
                out.push_str(&format!(
                    "  chpx {}..{} [{}]\n",
                    span.start_cp,
                    span.end_cp,
                    chpx.join(" ")
                ));
            }
        }
        out.push_str(&format!(
            "{}..{} istd={} [{}] {:?}\n",
            p.start_cp,
            p.end_cp,
            p.istd,
            sprms.join(" "),
            text
        ));
    }
    Ok(out)
}

pub struct Converted {
    pub docx: Vec<u8>,
    pub diagnostics: Vec<String>,
}

pub fn convert(data: &[u8]) -> Result<Converted> {
    let source = source::Source::open(data)?;
    let styles = styles::Styles::read(&source);
    let mut writer = body::Writer::new(&source, &styles, data);
    let mut parts = writer.document();
    add_properties(&mut parts, &summary::read(data));
    let docx = package::zip(&parts).map_err(|_| crate::office::doc::Error::Invalid("zip write"))?;
    let mut diagnostics = source.diagnostics.clone();
    diagnostics.extend(writer.diagnostics);
    Ok(Converted { docx, diagnostics })
}

/// Adds the document-property parts with their root relationships and content types.
fn add_properties(parts: &mut Vec<package::Part>, props: &summary::Properties) {
    let (extra, rels) = summary::parts(props);
    if extra.is_empty() {
        return;
    }
    for part in parts.iter_mut() {
        let mut xml = String::from_utf8_lossy(&part.bytes).into_owned();
        match part.name.as_str() {
            "_rels/.rels" => {
                for (i, (kind, target)) in rels.iter().enumerate() {
                    let rel = format!(
                        "<Relationship Id=\"rIdProps{i}\" Type=\"{kind}\" Target=\"{target}\"/>"
                    );
                    xml = xml.replace("</Relationships>", &(rel + "</Relationships>"));
                }
            }
            "[Content_Types].xml" => {
                for p in &extra {
                    let kind = match p.name.as_str() {
                        "docProps/core.xml" => {
                            "application/vnd.openxmlformats-package.core-properties+xml"
                        }
                        "docProps/app.xml" => {
                            "application/vnd.openxmlformats-officedocument.extended-properties+xml"
                        }
                        _ => "application/vnd.openxmlformats-officedocument.custom-properties+xml",
                    };
                    let o = format!(
                        "<Override PartName=\"/{}\" ContentType=\"{kind}\"/>",
                        p.name
                    );
                    xml = xml.replace("</Types>", &(o + "</Types>"));
                }
            }
            _ => continue,
        }
        part.bytes = xml.into_bytes();
    }
    parts.extend(extra);
}
