use crate::office::docx::model::{Block, Inline};
use std::io::Write;

pub(super) fn package(body: &str, content_type: &str, data: &[u8], external: bool) -> Vec<u8> {
    let w = crate::office::docx::testutil::W;
    let r = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let rels = "http://schemas.openxmlformats.org/package/2006/relationships";
    let mut output = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut output);
        let parts = [
            (
                "_rels/.rels",
                format!(
                    r#"<Relationships xmlns="{rels}"><Relationship Id="main" Type="{r}/officeDocument" Target="word/document.xml"/></Relationships>"#
                ),
            ),
            (
                "word/document.xml",
                format!(
                    r#"<w:document xmlns:w="{w}" xmlns:r="{r}"><w:body>{body}</w:body></w:document>"#
                ),
            ),
            (
                "word/_rels/document.xml.rels",
                format!(
                    r#"<Relationships xmlns="{rels}"><Relationship Id="chunk" Type="{r}/aFChunk" Target="chunk.txt" TargetMode="{}"/></Relationships>"#,
                    if external { "External" } else { "Internal" }
                ),
            ),
            (
                "[Content_Types].xml",
                format!(
                    r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="txt" ContentType="{content_type}"/></Types>"#
                ),
            ),
        ];
        for (name, value) in parts {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(value.as_bytes()).unwrap();
        }
        zip.start_file("word/chunk.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(data).unwrap();
        zip.finish().unwrap();
    }
    output.into_inner()
}

pub(super) fn text(blocks: &[Block]) -> String {
    blocks
        .iter()
        .filter_map(|b| match b {
            Block::Para(p) => Some(p),
            _ => None,
        })
        .flat_map(|p| &p.inlines)
        .filter_map(|i| match i {
            Inline::Text(t, _) => Some(t.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn plaintext_altchunk_imports_body_at_its_source_position() {
    let body = r#"<w:p><w:r><w:t>BEFORE</w:t></w:r></w:p><w:altChunk r:id="chunk"/><w:p><w:r><w:t>AFTER</w:t></w:r></w:p>"#;
    let opened = crate::office::docx::extract::open(
        package(body, "text/plain", b"IMPORTED", false),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(text(&opened.sections[0].blocks), "BEFOREIMPORTEDAFTER");
}

#[test]
fn plaintext_decoding_is_strict_and_unicode_boms_are_preserved_as_encoding() {
    use super::decode;
    assert_eq!(decode(b"ASCII", None).unwrap(), "ASCII");
    assert_eq!(decode("é水😀".as_bytes(), Some("utf-8")).unwrap(), "é水😀");
    assert_eq!(decode(b"\xef\xbb\xbfUTF8", None).unwrap(), "UTF8");
    for little in [false, true] {
        let mut bytes = if little {
            vec![0xff, 0xfe]
        } else {
            vec![0xfe, 0xff]
        };
        for unit in "é水😀".encode_utf16() {
            bytes.extend(if little {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            });
        }
        assert_eq!(decode(&bytes, None).unwrap(), "é水😀");
        assert_eq!(decode(&bytes, Some("utf-16")).unwrap(), "é水😀");
    }
    assert_eq!(decode(&[0, 65], Some("utf-16")).unwrap(), "A");
    for (bytes, charset) in [
        (&b"\xff"[..], None),
        (&b"\xff"[..], Some("utf-8")),
        (&b"\xff\xfe\x00"[..], None),
        (&b"\xff\xfe\x00\xd8"[..], None),
        (&b"\xff\xfeA\x00"[..], Some("utf-8")),
        (&b"A\0"[..], None),
    ] {
        assert!(decode(bytes, charset).is_err());
    }
}

#[test]
fn plaintext_controls_retain_blank_lines_tabs_pagebreaks_and_literal_markup() {
    let body = r#"<w:altChunk r:id="chunk"/>"#;
    let opened = crate::office::docx::extract::open(
        package(
            body,
            "text/plain; charset=utf-8",
            " A\r\n\r\nB\tC\nD\rE\u{c}<tag>&水 ".as_bytes(),
            false,
        ),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(opened.sections[0].blocks.len(), 5);
    let parts: Vec<Vec<_>> = opened.sections[0]
        .blocks
        .iter()
        .map(|b| {
            let Block::Para(p) = b else {
                panic!("paragraph expected")
            };
            p.inlines
                .iter()
                .map(|i| match i {
                    Inline::Text(t, _) => t.clone(),
                    Inline::Tab(_) => "TAB".into(),
                    Inline::Break(crate::office::docx::model::BreakKind::Page, _) => "PAGE".into(),
                    _ => panic!("unexpected inline"),
                })
                .collect()
        })
        .collect();
    assert_eq!(
        parts,
        vec![
            vec![" A"],
            vec![],
            vec!["B", "TAB", "C"],
            vec!["D"],
            vec!["E", "PAGE", "<tag>&水 "]
        ]
    );
    assert!(opened.import_warnings.is_empty());
}

#[test]
fn plaintext_import_uses_active_traversal_and_preserves_cell_identity() {
    let body = r#"<w:del><w:altChunk r:id="chunk"/></w:del><w:tbl><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>CELL</w:t></w:r></w:p><w:sdt><w:sdtContent><w:altChunk r:id="chunk"/></w:sdtContent></w:sdt></w:tc></w:tr></w:tbl>"#;
    let opened = crate::office::docx::extract::open(
        package(body, "text/plain", b"IMPORTED", false),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(opened.sections[0].blocks.len(), 1);
    let Block::Table(t) = &opened.sections[0].blocks[0] else {
        panic!("table expected")
    };
    let blocks = &t.rows[0].cells[0].blocks;
    assert_eq!(text(blocks), "CELLIMPORTED");
    let Block::Para(p) = &blocks[1] else {
        panic!("paragraph expected")
    };
    let Inline::Text(_, style) = &p.inlines[0] else {
        panic!("text expected")
    };
    let source = style.source.as_ref().unwrap();
    assert_eq!(source.part, "word/document.xml");
    assert_eq!(source.cells[0].row, 0);
    assert_eq!(source.cells[0].cell, 0);
}

#[test]
fn unsupported_invalid_and_undecodable_imports_have_distinct_diagnostics() {
    let body = r#"<w:altChunk r:id="chunk"/>"#;
    for (kind, bytes, external, code) in [
        (
            "application/xhtml+xml",
            &b"<p>HTML</p>"[..],
            false,
            "ALTCHUNK_UNSUPPORTED",
        ),
        (
            "application/rtf",
            &b"{\\rtf1 RTF}"[..],
            false,
            "ALTCHUNK_UNSUPPORTED",
        ),
        ("text/plain", &b"\xff"[..], false, "ALTCHUNK_DECODE_FAILED"),
        ("text/plain", &b"EXTERNAL"[..], true, "ALTCHUNK_INVALID"),
    ] {
        let opened = crate::office::docx::extract::open(
            package(body, kind, bytes, external),
            &Default::default(),
        )
        .unwrap();
        assert!(opened.sections[0].blocks.is_empty());
        assert_eq!(opened.import_warnings.len(), 1);
        assert_eq!(opened.import_warnings[0].0, code);
    }
}

#[test]
fn plaintext_import_restores_exported_content_and_pagebreak() {
    let body = r#"<w:p><w:r><w:t>BEFORE</w:t></w:r></w:p><w:altChunk r:id="chunk"/><w:p><w:r><w:t>AFTER</w:t></w:r></w:p>"#;
    let pages = crate::office::docx::testutil::pages(package(
        body,
        "text/plain",
        b"FIRST\x0cSECOND",
        false,
    ));
    assert_eq!(pages.len(), 2);
    let raw = |p: &crate::office::docx::paginate::OutPage| {
        p.items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<String>()
    };
    assert_eq!(raw(&pages[0]), "BEFOREFIRST");
    assert_eq!(raw(&pages[1]), "SECONDAFTER");
}

#[test]
fn plaintext_line_endings_preserve_empty_paragraphs_without_extra_terminal_paragraph() {
    use super::paragraphs;
    assert_eq!(paragraphs("A\r\nB\r\n").len(), 2);
    assert_eq!(paragraphs("A\n\n").len(), 2);
    assert_eq!(paragraphs("\r\n").len(), 1);
    assert!(paragraphs("").is_empty());
}

#[test]
fn unchosen_alternate_content_does_not_import_or_warn() {
    let body = r#"<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:future="urn:unknown"><mc:Choice Requires="future"><w:altChunk r:id="chunk"/></mc:Choice><mc:Fallback><w:p><w:r><w:t>FALLBACK</w:t></w:r></w:p></mc:Fallback></mc:AlternateContent>"#;
    let opened = crate::office::docx::extract::open(
        package(body, "text/html", b"UNSELECTED", false),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(text(&opened.sections[0].blocks), "FALLBACK");
    assert!(opened.import_warnings.is_empty());
}

#[test]
fn import_expansion_limits_report_incomplete_content_without_partial_chunk() {
    let body = r#"<w:altChunk r:id="chunk"/>"#;
    for data in [vec![b'a'; 8 * 1024 * 1024 + 1], vec![b'\n'; 16_384]] {
        let opened = crate::office::docx::extract::open(
            package(body, "text/plain", &data, false),
            &Default::default(),
        )
        .unwrap();
        assert!(opened.sections[0].blocks.is_empty());
        assert_eq!(opened.import_warnings[0].0, "ALTCHUNK_RESOURCE_LIMIT");
    }
}

#[test]
fn import_warning_reaches_public_index_without_payload() {
    let dir = std::env::temp_dir().join(format!("docx-altchunk-warning-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("source.docx");
    std::fs::write(
        &source,
        package(
            r#"<w:altChunk r:id="chunk"/><w:p><w:r><w:t>BODY</w:t></w:r></w:p>"#,
            "application/rtf",
            b"PRIVATE_CHUNK_PAYLOAD",
            false,
        ),
    )
    .unwrap();
    let index = dir.join("index.json");
    crate::office::docx::extract::run_read(
        r#"{"text":{"enabled":true}}"#,
        source.to_str().unwrap(),
        index.to_str().unwrap(),
        dir.join("pages.jsonl").to_str().unwrap(),
    )
    .unwrap();
    let raw = std::fs::read_to_string(index).unwrap();
    let index: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(
        index["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["code"] == "ALTCHUNK_UNSUPPORTED")
    );
    assert!(!raw.contains("PRIVATE_CHUNK_PAYLOAD"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn repeated_imports_keep_distinct_paragraph_ownership_and_warning_context() {
    let body =
        r#"<w:altChunk r:id="chunk"/><w:altChunk r:id="missing"/><w:altChunk r:id="chunk"/>"#;
    let opened = crate::office::docx::extract::open(
        package(body, "text/plain", b"REPEAT", false),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(text(&opened.sections[0].blocks), "REPEATREPEAT");
    assert_eq!(opened.sections[0].blocks.len(), 2);
    assert_eq!(opened.import_warnings.len(), 1);
    assert!(opened.import_warnings[0].1.contains("[missing]"));
    assert!(opened.import_warnings[0].1.contains("not imported"));
}
