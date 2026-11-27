use super::*;
use crate::office::docx::model::Inline;
use crate::office::docx::testutil::{W, docx, with_related_part};

fn package(position: &str, restart: &str) -> Vec<u8> {
    let reference = |id| {
        format!(r#"<w:p><w:r><w:t>Body{id}</w:t><w:endnoteReference w:id="{id}"/></w:r></w:p>"#)
    };
    let body = format!(
        r#"{}<w:p><w:pPr><w:sectPr><w:endnotePr><w:pos w:val="sectEnd"/><w:numFmt w:val="upperLetter"/><w:numStart w:val="3"/></w:endnotePr></w:sectPr></w:pPr></w:p>{}{}<w:sectPr><w:endnotePr><w:pos w:val="docEnd"/><w:numFmt w:val="lowerRoman"/><w:numStart w:val="5"/></w:endnotePr></w:sectPr>"#,
        reference(1),
        reference(2),
        reference(3)
    );
    let settings = format!(
        r#"<w:settings xmlns:w="{W}"><w:endnotePr>{position}{restart}<w:numFmt w:val="decimal"/><w:numStart w:val="2"/></w:endnotePr></w:settings>"#
    );
    let notes = (1..=3).map(|id| format!(r#"<w:endnote w:id="{id}"><w:p><w:r><w:endnoteRef/><w:t> Note{id}</w:t></w:r></w:p></w:endnote>"#)).collect::<String>();
    let notes = format!(
        r#"<w:endnotes xmlns:w="{W}"><w:endnote w:id="-1" w:type="separator"><w:p><w:r><w:t>Separator</w:t></w:r></w:p></w:endnote>{notes}</w:endnotes>"#
    );
    let bytes = with_related_part(
        docx(&body, None, None),
        "settings",
        "settings.xml",
        &settings,
        false,
    );
    with_related_part(bytes, "endnotes", "endnotes.xml", &notes, false)
}

fn paragraph_text(block: &Block) -> String {
    match block {
        Block::Para(p) => p
            .inlines
            .iter()
            .filter_map(|i| match i {
                Inline::Text(t, _) => Some(t.as_str()),
                _ => None,
            })
            .collect(),
        _ => String::new(),
    }
}

#[test]
fn section_endnotes_follow_their_references_and_restart_with_section_overrides() {
    let opened = open(
        package(
            r#"<w:pos w:val="sectEnd"/>"#,
            r#"<w:numRestart w:val="eachSect"/>"#,
        ),
        &Config::default(),
    )
    .unwrap();
    let texts: Vec<Vec<String>> = opened
        .sections
        .iter()
        .map(|s| s.blocks.iter().map(paragraph_text).collect())
        .collect();
    {
        assert_eq!(texts[0], ["Body1C", "", "Separator", "C Note1"]);
        assert_eq!(
            texts[1],
            ["Body2v", "Body3vi", "Separator", "v Note2", "vi Note3"]
        );
    }
}

#[test]
fn document_end_is_default_and_continuous_numbers_survive_format_changes() {
    for position in ["", r#"<w:pos w:val="docEnd"/>"#] {
        let opened = open(package(position, ""), &Config::default()).unwrap();
        let first: Vec<_> = opened.sections[0]
            .blocks
            .iter()
            .map(paragraph_text)
            .collect();
        let last: Vec<_> = opened.sections[1]
            .blocks
            .iter()
            .map(paragraph_text)
            .collect();
        assert_eq!(first.len(), 2);
        assert_eq!(last.len(), 6);
        {
            assert_eq!(first[0], "Body1C");
            assert_eq!(
                last,
                [
                    "Body2iv",
                    "Body3v",
                    "Separator",
                    "C Note1",
                    "iv Note2",
                    "v Note3"
                ]
            );
        }
    }
}

#[test]
fn section_restart_overrides_document_and_nested_references_keep_their_section() {
    for (document_restart, section_restart, expected) in [
        ("continuous", "eachSect", "D"),
        ("eachSect", "continuous", "B"),
    ] {
        let body = format!(
            r#"<w:p><w:pPr><w:sectPr/></w:pPr><w:r><w:t>First</w:t><w:endnoteReference w:id="1"/></w:r></w:p><w:sdt><w:sdtContent><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Table</w:t><w:endnoteReference w:id="2"/></w:r></w:p></w:tc></w:tr></w:tbl></w:sdtContent></w:sdt><w:sectPr><w:endnotePr><w:numRestart w:val="{section_restart}"/><w:numStart w:val="4"/><w:numFmt w:val="upperLetter"/></w:endnotePr></w:sectPr>"#
        );
        let settings = format!(
            r#"<w:settings xmlns:w="{W}"><w:endnotePr><w:pos w:val="sectEnd"/><w:numRestart w:val="{document_restart}"/></w:endnotePr></w:settings>"#
        );
        let notes = format!(
            r#"<w:endnotes xmlns:w="{W}"><w:endnote w:id="1"><w:p><w:r><w:endnoteRef/><w:t> Note1</w:t></w:r></w:p></w:endnote><w:endnote w:id="2"><w:p><w:r><w:endnoteRef/><w:t> Note2</w:t></w:r></w:p></w:endnote></w:endnotes>"#
        );
        let bytes = with_related_part(
            docx(&body, None, None),
            "settings",
            "settings.xml",
            &settings,
            false,
        );
        let bytes = with_related_part(bytes, "endnotes", "endnotes.xml", &notes, false);
        let opened = open(bytes, &Config::default()).unwrap();
        let Block::Table(table) = &opened.sections[1].blocks[0] else {
            panic!("table")
        };
        let table_text = paragraph_text(&table.rows[0].cells[0].blocks[0]);
        {
            assert_eq!(opened.sections[0].blocks.len(), 2);
            assert_eq!(paragraph_text(&opened.sections[0].blocks[1]), "i Note1");
            assert_eq!(table_text, format!("Table{expected}"));
            assert_eq!(
                paragraph_text(&opened.sections[1].blocks[1]),
                format!("{expected} Note2")
            );
        }
    }
}

#[test]
fn explicit_normal_note_types_preserve_references_and_exclude_special_notes() {
    for kind in ["footnote", "endnote"] {
        let mut results = Vec::new();
        for normal in ["", " w:type=\"normal\""] {
            let body = (1..=5).map(|id| format!(r#"<w:p><w:r><w:t>Body{id}</w:t><w:{kind}Reference w:id="{id}"/></w:r></w:p>"#)).collect::<String>();
            let mut notes = format!(
                r#"<w:{kind} w:id="1"{normal}><w:p><w:r><w:{kind}Ref/><w:t>NormalContent</w:t></w:r></w:p></w:{kind}>"#
            );
            for (id, typ) in [
                (2, "continuationSeparator"),
                (3, "continuationNotice"),
                (4, "unknown"),
                (5, "separator"),
            ] {
                notes.push_str(&format!(r#"<w:{kind} w:id="{id}" w:type="{typ}"><w:p><w:r><w:t>Special{id}</w:t></w:r></w:p></w:{kind}>"#));
            }
            let plural = format!("{kind}s");
            let xml = format!(r#"<w:{plural} xmlns:w="{W}">{notes}</w:{plural}>"#);
            let bytes = with_related_part(
                docx(&body, None, None),
                &plural,
                &format!("{plural}.xml"),
                &xml,
                false,
            );
            let opened = open(bytes, &Config::default()).unwrap();
            let body_text = opened.sections[0]
                .blocks
                .iter()
                .map(paragraph_text)
                .collect::<String>();
            assert!(
                body_text.contains(if kind == "footnote" {
                    "Body11"
                } else {
                    "Body1i"
                }),
                "{kind}: body reference number"
            );
            let note_text = if kind == "footnote" {
                assert_eq!(opened.notes.len(), 1);
                assert_eq!(
                    opened
                        .note_separator
                        .iter()
                        .map(paragraph_text)
                        .collect::<String>(),
                    "Special5"
                );
                opened.notes[&1]
                    .iter()
                    .map(paragraph_text)
                    .collect::<String>()
            } else {
                body_text.clone()
            };
            assert!(
                note_text.contains(if kind == "footnote" {
                    "1NormalContent"
                } else {
                    "iNormalContent"
                }),
                "{kind}: note text and number"
            );
            for id in 2..=4 {
                assert!(
                    !note_text.contains(&format!("Special{id}")),
                    "{kind}: special type {id}"
                );
            }
            results.push((body_text, note_text));
        }
        assert_eq!(
            results[0], results[1],
            "{kind}: explicit and implicit normal"
        );
    }
}

#[test]
fn custom_endnotes_preserve_bodies_and_do_not_advance_automatic_numbers() {
    for restart in ["continuous", "eachSect"] {
        let reference = |id, custom: bool| {
            format!(
                r#"<w:p><w:r><w:t>Body{id}</w:t><w:endnoteReference w:id="{id}" w:customMarkFollows="{custom}"/><w:t>|</w:t></w:r></w:p>"#
            )
        };
        let body = format!(
            r#"{}{}{}<w:p><w:pPr><w:sectPr/></w:pPr></w:p>{}{}<w:sectPr/>"#,
            reference(1, false),
            reference(2, true),
            reference(3, false),
            reference(4, true),
            reference(5, false)
        );
        let settings = format!(
            r#"<w:settings xmlns:w="{W}"><w:endnotePr><w:pos w:val="sectEnd"/><w:numStart w:val="3"/><w:numFmt w:val="decimal"/><w:numRestart w:val="{restart}"/></w:endnotePr></w:settings>"#
        );
        let notes = (1..=5).map(|id| format!(r#"<w:endnote w:id="{id}"><w:p><w:r><w:endnoteRef/><w:t>{}Note{id}</w:t></w:r></w:p></w:endnote>"#, if id == 2 || id == 4 { "*" } else { ":" })).collect::<String>();
        let notes = format!(r#"<w:endnotes xmlns:w="{W}">{notes}</w:endnotes>"#);
        let bytes = with_related_part(
            docx(&body, None, None),
            "settings",
            "settings.xml",
            &settings,
            false,
        );
        let bytes = with_related_part(bytes, "endnotes", "endnotes.xml", &notes, false);
        let opened = open(bytes, &Config::default()).unwrap();
        let texts: Vec<Vec<_>> = opened
            .sections
            .iter()
            .map(|s| s.blocks.iter().map(paragraph_text).collect())
            .collect();
        assert_eq!(
            texts[0],
            [
                "Body13|", "Body2|", "Body34|", "", "3:Note1", "*Note2", "4:Note3"
            ]
        );
        let last = if restart == "eachSect" { "3" } else { "5" };
        assert_eq!(
            texts[1],
            [
                "Body4|".to_string(),
                format!("Body5{last}|"),
                "*Note4".into(),
                format!("{last}:Note5")
            ]
        );
    }
}

#[test]
fn custom_only_document_endnote_retains_multiple_paragraphs_and_table() {
    for custom in ["true", "on", "1"] {
        let body = format!(
            r#"<w:p><w:r><w:t>Body</w:t><w:endnoteReference w:id="7" w:customMarkFollows="{custom}"/><w:t>*</w:t></w:r></w:p>"#
        );
        let notes = format!(
            r#"<w:endnotes xmlns:w="{W}"><w:endnote w:id="7"><w:p><w:r><w:endnoteRef/><w:t>*First</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:t>Last</w:t></w:r></w:p></w:endnote></w:endnotes>"#
        );
        let bytes = with_related_part(
            docx(&body, None, None),
            "endnotes",
            "endnotes.xml",
            &notes,
            false,
        );
        let opened = open(bytes, &Config::default()).unwrap();
        let blocks = &opened.sections[0].blocks;
        assert_eq!(blocks.len(), 4);
        assert_eq!(paragraph_text(&blocks[0]), "Body*");
        assert_eq!(paragraph_text(&blocks[1]), "*First");
        let Block::Table(table) = &blocks[2] else {
            panic!("note table missing")
        };
        assert_eq!(paragraph_text(&table.rows[0].cells[0].blocks[0]), "Cell");
        assert_eq!(paragraph_text(&blocks[3]), "Last");
    }
}
