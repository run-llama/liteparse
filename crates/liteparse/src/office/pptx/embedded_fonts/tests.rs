use super::*;

/// A slide-less package whose `embeddedFontLst` points one typeface's slots at
/// the given relationship targets (regular, bold).
fn package(regular: &str, bold: &str) -> Document {
    use std::io::Write;
    let ns_r = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let rel = |id: &str, kind: &str, target: &str| {
        format!(r#"<Relationship Id="{id}" Type="{ns_r}/{kind}" Target="{target}"/>"#)
    };
    let rels = |body: String| {
        format!(
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#
        )
    };
    let parts = [
        (
            "_rels/.rels",
            rels(rel("rId1", "officeDocument", "ppt/presentation.xml")),
        ),
        (
            "ppt/presentation.xml",
            format!(
                r#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="{ns_r}"><p:sldSz cx="9144000" cy="6858000"/><p:embeddedFontLst><p:embeddedFont><p:font typeface="Body"/><p:regular r:id="rId1"/><p:bold r:id="rId2"/></p:embeddedFont></p:embeddedFontLst></p:presentation>"#
            ),
        ),
        (
            "ppt/_rels/presentation.xml.rels",
            rels(rel("rId1", "font", regular) + &rel("rId2", "font", bold)),
        ),
    ];
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        for (name, body) in &parts {
            z.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }
    let cfg = crate::office::pptx::config::Config::default();
    Document::open(buf.into_inner(), &cfg.limits).unwrap()
}

#[test]
fn null_target_marks_an_absent_slot_not_a_failure() {
    let fonts = FontSystem::new(&[], false, &Default::default());
    let doc = package("NULL", "fonts/font2.fntdata");
    let failures = register(&doc, &fonts);
    // Only the genuinely missing bold part is reported.
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(failures[0].0, "ppt/fonts/font2.fntdata");
}
