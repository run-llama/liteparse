use super::*;
use crate::office::docx::model::{Block, Inline};
use crate::office::docx::testutil::{W, docx, with_part, with_related_part};

const CORE: &str = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Annual Plan</dc:title><dc:creator></dc:creator></cp:coreProperties>"#;
const CUSTOM: &str = r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="Doc Number"><vt:lpwstr>N-42</vt:lpwstr></property></Properties>"#;

/// A complex field with instruction `instr` and saved result `saved`.
fn field(instr: &str, saved: &str) -> String {
    format!(
        r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> {instr} </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>{saved}</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#
    )
}

fn text(blocks: &[Block]) -> String {
    blocks
        .iter()
        .filter_map(|b| match b {
            Block::Para(p) => Some(p.inlines.iter().filter_map(|i| match i {
                Inline::Text(t, _) => Some(t.as_str()),
                _ => None,
            })),
            _ => None,
        })
        .flatten()
        .collect()
}

/// Header and body text of a document whose header and body both hold `fields`.
fn header_and_body(fields: &str) -> (String, String) {
    let body = format!(
        r#"<w:p>{fields}</w:p><w:sectPr><w:headerReference w:type="default" r:id="rheader"/></w:sectPr>"#
    );
    let header = format!(r#"<w:hdr xmlns:w="{W}"><w:p>{fields}</w:p></w:hdr>"#);
    let data = with_related_part(
        docx(&body, None, None),
        "header",
        "header1.xml",
        &header,
        false,
    );
    let data = with_part(
        with_part(data, "docProps/core.xml", CORE),
        "docProps/custom.xml",
        CUSTOM,
    );
    let opened =
        crate::office::docx::extract::open(data, &crate::office::pptx::config::Config::default())
            .unwrap();
    (
        text(&opened.hdr_ftr["word/header1.xml"]),
        text(&opened.sections[0].blocks),
    )
}

#[test]
fn header_property_fields_show_the_current_property() {
    let fields = field("TITLE \\* Upper \\* MERGEFORMAT", "old title")
        + &field("DOCPROPERTY \"Doc Number\"", "N-1");
    let (header, body) = header_and_body(&fields);
    assert_eq!(header, "ANNUAL PLANN-42");
    // Body fields keep their saved result.
    assert_eq!(body, "old titleN-1");
}

#[test]
fn an_empty_or_missing_property_keeps_the_saved_result() {
    let fields = field("AUTHOR", "Saved Author") + &field("SUBJECT", "Saved Subject");
    assert_eq!(header_and_body(&fields).0, "Saved AuthorSaved Subject");
}

#[test]
fn a_title_field_with_a_new_value_keeps_its_result() {
    let props = DocProps([("TITLE".to_string(), "Annual Plan".to_string())].into());
    assert_eq!(props.field_text("TITLE \"Replacement\""), None);
    assert_eq!(
        props.field_text("TITLE \\* Caps").as_deref(),
        Some("Annual Plan")
    );
}
