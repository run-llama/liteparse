//! Document-local Word font parts, decoded before shaping or font lookup.

use crate::office::pptx::{fonts::FontSystem, package::Package};

/// Registers the four source style slots while preserving shared font precedence.
/// Package reads enforce the document's existing decompression and part limits.
pub(crate) fn register(pkg: &Package, main: &str, fonts: &FontSystem) -> Vec<(String, String)> {
    let mut warnings = Vec::new();
    let Some(table) = pkg.rel_by_kind(main, "fontTable") else {
        return warnings;
    };
    let warn = |warnings: &mut Vec<_>, detail: String| {
        warnings.push(("EMBEDDED_FONT_INVALID".to_string(), detail));
    };
    if table.external || !relationship_type(&table.rel_type, "fontTable") {
        warn(
            &mut warnings,
            format!("{main}: invalid fontTable relationship {}", table.id),
        );
        return warnings;
    }
    let Some(root) = crate::office::docx::extract::read_xml(pkg, &table.target) else {
        warn(
            &mut warnings,
            format!("{}: unreadable or malformed font table", table.target),
        );
        return warnings;
    };
    if root.name != "w:fonts" {
        warn(&mut warnings, format!("{}: expected w:fonts", table.target));
        return warnings;
    }
    let content_types = crate::office::docx::extract::read_xml(pkg, "[Content_Types].xml");
    for font in root.children.iter().filter(|n| n.name == "w:font") {
        let family = font.attr("name").unwrap_or("");
        for (slot, bold, italic) in [
            ("w:embedRegular", false, false),
            ("w:embedBold", true, false),
            ("w:embedItalic", false, true),
            ("w:embedBoldItalic", true, true),
        ] {
            for embed in font.children.iter().filter(|n| n.name == slot) {
                let id = embed.attr("r:id").unwrap_or("");
                let result = (|| {
                    if family.trim().is_empty() {
                        return Err("missing font family".to_string());
                    }
                    let rel = pkg
                        .rel_by_id(&table.target, id)
                        .ok_or("missing font relationship")?;
                    if rel.external || !relationship_type(&rel.rel_type, "font") {
                        return Err("font relationship must be internal and of type font".into());
                    }
                    let key = embed.attr("fontKey").unwrap_or("");
                    let kind = content_types
                        .as_deref()
                        .and_then(|types| content_type(types, &rel.target));
                    match kind {
                        Some("application/vnd.openxmlformats-officedocument.obfuscatedFont") => {}
                        Some("application/x-font-ttf")
                            if guid_bytes(key)?.iter().all(|&b| b == 0) => {}
                        _ => return Err("missing or unsupported Word font content type/key".into()),
                    }
                    let bytes = pkg
                        .read(&rel.target)
                        .map_err(|e| format!("{}: {e}", rel.target))?;
                    let bytes = decode(bytes, key)?;
                    if rustybuzz::Face::from_slice(&bytes, 0).is_none()
                        || !fonts.add_embedded(family, bold, italic, bytes)
                    {
                        return Err("invalid TrueType/OpenType font".into());
                    }
                    Ok(())
                })();
                if let Err(reason) = result {
                    warn(
                        &mut warnings,
                        format!("{} {slot} relationship {id}: {reason}", table.target),
                    );
                }
            }
        }
    }
    warnings
}

fn relationship_type(value: &str, kind: &str) -> bool {
    [
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/",
        "http://purl.oclc.org/ooxml/officeDocument/relationships/",
    ]
    .iter()
    .any(|base| value.strip_prefix(base) == Some(kind))
}

fn content_type<'a>(types: &'a crate::office::pptx::xml::XNode, part: &str) -> Option<&'a str> {
    types
        .children
        .iter()
        .find(|node| {
            node.name.ends_with(":Override")
                && node.attr("PartName").map(|p| p.trim_start_matches('/')) == Some(part)
        })
        .or_else(|| {
            types.children.iter().find(|node| {
                node.name.ends_with(":Default")
                    && node.attr("Extension").is_some_and(|ext| {
                        part.rsplit('.')
                            .next()
                            .is_some_and(|p| p.eq_ignore_ascii_case(ext))
                    })
            })
        })
        .and_then(|node| node.attr("ContentType"))
}

/// ISO 29500 font obfuscation repeats the reversed GUID bytes over bytes 0..32.
/// A zero key preserves raw SFNT bytes, as specified by Word's implementation note.
fn guid_bytes(key: &str) -> Result<[u8; 16], String> {
    let key = key
        .strip_prefix('{')
        .and_then(|s| s.strip_suffix('}'))
        .unwrap_or(key);
    if key.len() != 36 || ![8, 13, 18, 23].iter().all(|&i| key.as_bytes()[i] == b'-') {
        return Err("invalid fontKey GUID".into());
    }
    let hex: Vec<_> = key.bytes().filter(|&b| b != b'-').collect();
    if hex.len() != 32 || !hex.iter().all(u8::is_ascii_hexdigit) {
        return Err("invalid fontKey GUID".into());
    }
    let mut guid = [0u8; 16];
    for (i, pair) in hex.as_chunks::<2>().0.iter().enumerate() {
        let digit = |b: u8| (b as char).to_digit(16).unwrap() as u8;
        guid[15 - i] = digit(pair[0]) * 16 + digit(pair[1]);
    }
    Ok(guid)
}

fn decode(mut bytes: Vec<u8>, key: &str) -> Result<Vec<u8>, String> {
    let guid = guid_bytes(key)?;
    if bytes.len() < 32 {
        return Err("font part shorter than 32 bytes".into());
    }
    for (i, byte) in bytes[..32].iter_mut().enumerate() {
        *byte ^= guid[i % 16];
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
