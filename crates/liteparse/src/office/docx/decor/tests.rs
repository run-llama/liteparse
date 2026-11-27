use crate::office::pptx::scene::{Item, Paint};

use crate::office::docx::testutil::{docx, pages, para};

fn texts(items: &[Item]) -> Vec<(String, f32, f32, crate::office::pptx::color::Rgba)> {
    items
        .iter()
        .filter_map(|it| match it {
            Item::Text(r) => Some((r.text.clone(), r.transform.tx, r.transform.ty, r.color)),
            _ => None,
        })
        .collect()
}

fn fills(items: &[Item]) -> Vec<(tiny_skia::Rect, crate::office::pptx::color::Rgba)> {
    items
        .iter()
        .filter_map(|it| match it {
            Item::Path(p) => match &p.fill {
                Some(Paint::Solid(c)) => Some((p.path.bounds(), *c)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[test]
fn page_border_offset_from_page_edge() {
    let body = format!(
        r#"{}<w:sectPr><w:pgBorders w:offsetFrom="page"><w:top w:val="single" w:sz="8" w:space="24"/><w:left w:val="single" w:sz="8" w:space="24"/><w:bottom w:val="single" w:sz="8" w:space="24"/><w:right w:val="single" w:sz="8" w:space="24"/></w:pgBorders></w:sectPr>"#,
        para("x")
    );
    let p = pages(docx(&body, None, None));
    let f = fills(&p[0].items);
    // A 1 pt line 24 pt inside each edge of the US Letter page.
    assert!(f.iter().any(|(r, _)| (r.left() - 24.0).abs() < 0.01
        && (r.top() - 24.0).abs() < 0.01
        && (r.width() - 564.0).abs() < 0.01
        && (r.height() - 1.0).abs() < 0.01));
    assert!(
        f.iter()
            .any(|(r, _)| (r.left() - 24.0).abs() < 0.01 && r.height() > 700.0)
    );
}

#[test]
fn page_border_art_styles_are_not_drawn() {
    let body = format!(
        r#"{}<w:sectPr><w:pgBorders><w:top w:val="apples" w:sz="8" w:space="24"/></w:pgBorders></w:sectPr>"#,
        para("x")
    );
    let p = pages(docx(&body, None, None));
    assert!(fills(&p[0].items).is_empty());
}

#[test]
fn automatic_text_is_white_on_dark_shading() {
    let body = r#"<w:p><w:pPr><w:shd w:val="clear" w:color="auto" w:fill="1F2A44"/></w:pPr><w:r><w:t>dark</w:t></w:r></w:p><w:p><w:pPr><w:shd w:val="clear" w:color="auto" w:fill="DDEEFF"/></w:pPr><w:r><w:t>light</w:t></w:r></w:p>"#;
    let p = pages(docx(body, None, None));
    let t = texts(&p[0].items);
    let dark = t.iter().find(|x| x.0.contains("dark")).unwrap();
    let light = t.iter().find(|x| x.0.contains("light")).unwrap();
    assert_eq!((dark.3.r, dark.3.g, dark.3.b), (1.0, 1.0, 1.0));
    assert_eq!((light.3.r, light.3.g, light.3.b), (0.0, 0.0, 0.0));
}

#[test]
fn table_style_bands_rows() {
    let styles = r#"<w:style w:type="table" w:styleId="Banded"><w:tblPr><w:tblStyleRowBandSize w:val="1"/></w:tblPr><w:tblStylePr w:type="band1Horz"><w:tcPr><w:shd w:val="clear" w:fill="AABBCC"/></w:tcPr></w:tblStylePr></w:style>"#;
    let row =
        |t: &str| format!(r#"<w:tr><w:tc><w:p><w:r><w:t>{t}</w:t></w:r></w:p></w:tc></w:tr>"#);
    let body = format!(
        r#"<w:tbl><w:tblPr><w:tblStyle w:val="Banded"/><w:tblLook w:firstRow="1" w:noHBand="0"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid>{}{}{}{}</w:tbl>{}"#,
        row("head"),
        row("one"),
        row("two"),
        row("three"),
        para("after")
    );
    let p = pages(docx(&body, Some(styles), None));
    let banded = fills(&p[0].items)
        .into_iter()
        .filter(|(_, c)| (c.r - 0xAA as f32 / 255.0).abs() < 0.01)
        .count();
    // Rows one and three (the first band after the header row) are shaded.
    assert_eq!(banded, 2);
}

#[test]
fn line_numbers_beside_body_lines() {
    let body = format!(
        r#"{}{}<w:p><w:pPr><w:suppressLineNumbers/></w:pPr><w:r><w:t>skip</w:t></w:r></w:p>{}<w:sectPr><w:lnNumType w:countBy="1"/></w:sectPr>"#,
        para("a"),
        para("b"),
        para("c")
    );
    let p = pages(docx(&body, None, None));
    let t = texts(&p[0].items);
    let nums: Vec<&str> = t
        .iter()
        .filter(|x| x.0.chars().all(|c| c.is_ascii_digit()))
        .map(|x| x.0.as_str())
        .collect();
    assert_eq!(nums, ["1", "2", "3"]);
    // Right-aligned 18 pt left of the 72 pt margin.
    assert!(
        t.iter()
            .filter(|x| x.0 == "1")
            .all(|x| x.1 < 54.0 && x.1 > 40.0)
    );
}
