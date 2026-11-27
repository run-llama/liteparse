use super::*;
use crate::office::docx::testutil::{W, docx, with_comments};

fn read_comments(inner: &str, external: bool) -> Vec<Value> {
    let xml = format!(r#"<w:comments xmlns:w="{W}">{inner}</w:comments>"#);
    let bytes = with_comments(docx("", None, None), "reviews/notes.xml", &xml, external);
    let pkg = Package::open(bytes, &Default::default()).unwrap();
    read(&pkg, "word/document.xml")
}

#[test]
fn related_comments_preserve_metadata_rich_runs_and_paragraphs() {
    let comments = read_comments(
        r#"<w:comment w:id="17" w:author="Reviewer" w:initials="R" w:date="2026-01-01T12:00:00Z"><w:p><w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">First </w:t></w:r><w:hyperlink><w:r><w:t>link</w:t><w:tab/><w:t>cell</w:t><w:br/><w:t>line</w:t></w:r></w:hyperlink></w:p><w:p/><w:p><w:r><w:t>Last </w:t></w:r></w:p></w:comment>"#,
        false,
    );
    assert_eq!(comments.len(), 1);
    assert_eq!(
        comments[0],
        json!({
            "id": "17", "subtype": "Text", "title": "Reviewer", "initials": "R",
            "created": "2026-01-01T12:00:00Z", "sourcePart": "word/reviews/notes.xml",
            "contents": "First link\tcell\nline\n\nLast "
        })
    );
}

#[test]
fn unreferenced_and_unidentified_comments_retain_content() {
    let comments = read_comments(
        r#"<w:comment><w:p><w:del><w:r><w:delText>deleted</w:delText></w:r></w:del><w:moveFrom><w:r><w:t>moved</w:t></w:r></w:moveFrom><w:ins><w:r><w:t>kept</w:t></w:r></w:ins><w:r><w:instrText> DATE </w:instrText><w:t>cached</w:t></w:r></w:p></w:comment><w:comment w:id="2"><w:p/></w:comment>"#,
        false,
    );
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[0]["id"], Value::Null);
    assert_eq!(comments[0]["contents"], "keptcached");
    assert_eq!(comments[1]["contents"], "");
    assert!(comments[0].get("title").is_none());
}

#[test]
fn external_comment_relationship_is_not_read_even_if_part_exists() {
    assert!(
        read_comments(
            r#"<w:comment w:id="1"><w:p><w:r><w:t>external</w:t></w:r></w:p></w:comment>"#,
            true
        )
        .is_empty()
    );
}

#[test]
fn strict_namespace_and_rebound_prefix_keep_comment_content() {
    let xml = r#"<q:comments xmlns:q="http://purl.oclc.org/ooxml/wordprocessingml/main"><q:comment q:id="7"><q:p><q:r><q:t>Strict text</q:t></q:r></q:p></q:comment></q:comments>"#;
    let bytes = with_comments(docx("", None, None), "comments.xml", xml, false);
    let pkg = Package::open(bytes, &Default::default()).unwrap();
    let comments = read(&pkg, "word/document.xml");
    assert_eq!(comments[0]["id"], "7");
    assert_eq!(comments[0]["contents"], "Strict text");
    assert!(read(&pkg, "word/header1.xml").is_empty());
}

#[test]
fn missing_or_malformed_comments_part_yields_no_metadata() {
    let pkg = Package::open(docx("", None, None), &Default::default()).unwrap();
    assert!(read(&pkg, "word/document.xml").is_empty());
    for xml in ["<broken>", "<notComments/>"] {
        let bytes = with_comments(docx("", None, None), "comments.xml", xml, false);
        let pkg = Package::open(bytes, &Default::default()).unwrap();
        assert!(read(&pkg, "word/document.xml").is_empty());
    }
}
