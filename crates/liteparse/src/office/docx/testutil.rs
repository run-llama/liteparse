//! Synthetic `.docx` packages for tests.

use std::io::Write;

pub const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// A package whose `word/document.xml` body is `body`, with optional
/// `word/styles.xml` and `word/numbering.xml` contents (inner XML).
pub fn docx(body: &str, styles: Option<&str>, numbering: Option<&str>) -> Vec<u8> {
    let rels = |inner: String| {
        format!(
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{inner}</Relationships>"#
        )
    };
    let mut parts = vec![
        (
            "_rels/.rels".to_string(),
            rels(format!(
                r#"<Relationship Id="rId1" Type="{R}/officeDocument" Target="word/document.xml"/>"#
            )),
        ),
        (
            "word/document.xml".to_string(),
            format!(
                r#"<w:document xmlns:w="{W}" xmlns:r="{R}"><w:body>{body}</w:body></w:document>"#
            ),
        ),
    ];
    let mut doc_rels = String::new();
    if let Some(s) = styles {
        parts.push((
            "word/styles.xml".into(),
            format!(r#"<w:styles xmlns:w="{W}">{s}</w:styles>"#),
        ));
        doc_rels.push_str(&format!(
            r#"<Relationship Id="rS" Type="{R}/styles" Target="styles.xml"/>"#
        ));
    }
    if let Some(n) = numbering {
        parts.push((
            "word/numbering.xml".into(),
            format!(r#"<w:numbering xmlns:w="{W}">{n}</w:numbering>"#),
        ));
        doc_rels.push_str(&format!(
            r#"<Relationship Id="rN" Type="{R}/numbering" Target="numbering.xml"/>"#
        ));
    }
    parts.push(("word/_rels/document.xml.rels".into(), rels(doc_rels)));
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        for (name, body) in &parts {
            z.start_file(name.as_str(), zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }
    buf.into_inner()
}

/// A paragraph with one run of text.
pub fn para(text: &str) -> String {
    format!(r#"<w:p><w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>"#)
}

/// Adds a related comments part at a caller-selected location.
pub fn with_comments(bytes: Vec<u8>, target: &str, xml: &str, external: bool) -> Vec<u8> {
    with_related_part(bytes, "comments", target, xml, external)
}

/// Adds a main-document relationship and its XML part.
pub fn with_related_part(
    bytes: Vec<u8>,
    kind: &str,
    target: &str,
    xml: &str,
    external: bool,
) -> Vec<u8> {
    use std::io::Read;
    let mut zin = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        for i in 0..zin.len() {
            let mut f = zin.by_index(i).unwrap();
            let name = f.name().unwrap().to_string();
            let mut data = String::new();
            f.read_to_string(&mut data).unwrap();
            if name == "word/_rels/document.xml.rels" {
                let mode = if external {
                    r#" TargetMode="External""#
                } else {
                    ""
                };
                data = data.replace("</Relationships>", &format!(
                    r#"<Relationship Id="r{kind}" Type="{R}/{kind}" Target="{target}"{mode}/></Relationships>"#));
            }
            z.start_file(name.as_str(), zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(data.as_bytes()).unwrap();
        }
        z.start_file(
            format!("word/{target}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        z.write_all(xml.as_bytes()).unwrap();
        z.finish().unwrap();
    }
    buf.into_inner()
}

/// Opens and lays out a synthetic package with the system fonts.
pub fn pages(bytes: Vec<u8>) -> Vec<crate::office::docx::paginate::OutPage> {
    let cfg = crate::office::pptx::config::Config::default();
    let opened = crate::office::docx::extract::open(bytes, &cfg).expect("open");
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], true, &Default::default());
    crate::office::docx::extract::layout(&opened, &fonts).pages
}

/// Adds `word/footnotes.xml` with a separator and footnote id 1 (`text`).
pub fn with_footnote(bytes: Vec<u8>, text: &str) -> Vec<u8> {
    use std::io::Read;
    let mut zin = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        for i in 0..zin.len() {
            let mut f = zin.by_index(i).unwrap();
            let name = f.name().unwrap().to_string();
            let mut data = String::new();
            f.read_to_string(&mut data).unwrap();
            if name == "word/_rels/document.xml.rels" {
                data = data.replace(
                    "</Relationships>",
                    &format!(r#"<Relationship Id="rF" Type="{R}/footnotes" Target="footnotes.xml"/></Relationships>"#),
                );
            }
            z.start_file(name.as_str(), zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(data.as_bytes()).unwrap();
        }
        z.start_file(
            "word/footnotes.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        let xml = format!(
            r#"<w:footnotes xmlns:w="{W}"><w:footnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:footnote><w:footnote w:id="1"><w:p><w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p></w:footnote></w:footnotes>"#
        );
        z.write_all(xml.as_bytes()).unwrap();
        z.finish().unwrap();
    }
    buf.into_inner()
}

/// Adds `word/settings.xml` with the given `compatibilityMode`.
pub fn with_compat_mode(bytes: Vec<u8>, mode: u32) -> Vec<u8> {
    with_settings(
        bytes,
        &format!(
            r#"<w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="{mode}"/></w:compat>"#
        ),
    )
}

/// Adds `word/settings.xml` holding `inner`.
pub fn with_settings(bytes: Vec<u8>, inner: &str) -> Vec<u8> {
    use std::io::{Read, Write};
    let mut input = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut output = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut output);
        for i in 0..input.len() {
            let mut file = input.by_index(i).unwrap();
            let name = file.name().unwrap().to_string();
            let mut data = String::new();
            file.read_to_string(&mut data).unwrap();
            if name == "word/_rels/document.xml.rels" {
                data = data.replace("</Relationships>", &format!(r#"<Relationship Id="rSettings" Type="{R}/settings" Target="settings.xml"/></Relationships>"#));
            }
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(data.as_bytes()).unwrap();
        }
        zip.start_file(
            "word/settings.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        write!(zip, r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{inner}</w:settings>"#).unwrap();
        zip.finish().unwrap();
    }
    output.into_inner()
}

/// Adds a package part (`docProps/core.xml`...) with no relationship.
pub fn with_part(bytes: Vec<u8>, name: &str, xml: &str) -> Vec<u8> {
    use std::io::Read;
    let mut zin = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        for i in 0..zin.len() {
            let mut f = zin.by_index(i).unwrap();
            let entry = f.name().unwrap().to_string();
            let mut data = Vec::new();
            f.read_to_end(&mut data).unwrap();
            z.start_file(entry.as_str(), zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(&data).unwrap();
        }
        z.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(xml.as_bytes()).unwrap();
        z.finish().unwrap();
    }
    buf.into_inner()
}
