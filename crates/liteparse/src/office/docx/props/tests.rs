use super::*;

fn node(xml: &str) -> std::rc::Rc<XNode> {
    let wrapped = xml.replacen(
        "<w:pPr",
        &format!("<w:pPr xmlns:w=\"{}\"", crate::office::docx::testutil::W),
        1,
    );
    crate::office::pptx::xml::parse_word(&wrapped).unwrap()
}

#[test]
fn hanging_indent_is_negative_first_line() {
    let p = PPr::parse(&node(
        r#"<w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr>"#,
    ));
    assert_eq!(p.ind_left, Some(36.0));
    assert_eq!(p.ind_first, Some(-18.0));
}

#[test]
fn tab_clear_removes_inherited_stop() {
    let mut base = PPr::parse(&node(
        r#"<w:pPr><w:tabs><w:tab w:val="left" w:pos="1440"/><w:tab w:val="right" w:pos="2880"/></w:tabs></w:pPr>"#,
    ));
    let direct = PPr::parse(&node(
        r#"<w:pPr><w:tabs><w:tab w:val="clear" w:pos="1440"/></w:tabs></w:pPr>"#,
    ));
    base.apply(&direct);
    let stops = base.tab_stops();
    assert_eq!(stops.len(), 1);
    assert_eq!(stops[0].pos, 144.0);
    assert_eq!(stops[0].kind, TabKind::Right);
}

#[test]
fn spacing_line_defaults_to_auto_rule() {
    let p = PPr::parse(&node(
        r#"<w:pPr><w:spacing w:after="160" w:line="259"/></w:pPr>"#,
    ));
    assert_eq!(p.after, Some(8.0));
    assert_eq!(p.line, Some(259.0));
    assert_eq!(p.line_rule, Some(LineRule::Auto));
}

#[test]
fn frame_anchors_default_to_text_and_preserve_explicit_positions() {
    let implicit = PPr::parse(&node(r#"<w:pPr><w:framePr/></w:pPr>"#));
    let frame = implicit.frame.unwrap();
    assert_eq!(frame.h_anchor, "text");
    assert_eq!(frame.v_anchor, "text");

    let explicit = PPr::parse(&node(
        r#"<w:pPr><w:framePr w:hAnchor="page" w:vAnchor="margin" w:x="720" w:y="1440"/></w:pPr>"#,
    ));
    let frame = explicit.frame.unwrap();
    assert_eq!(frame.h_anchor, "page");
    assert_eq!(frame.v_anchor, "margin");
    assert_eq!(frame.x, Ok(36.0));
    assert_eq!(frame.y, Ok(72.0));
}

#[test]
fn page_frame_candidate_keeps_style_coordinates_under_direct_wrap_override() {
    let mut base = PPr::parse(&node(
        r#"<w:pPr><w:framePr w:w="4000" w:y="2400" w:vAnchor="page" w:hAnchor="margin"/></w:pPr>"#,
    ));
    base.apply(&PPr::parse(&node(
        r#"<w:pPr><w:framePr w:wrap="notBeside" w:h="600" w:hRule="exact"/></w:pPr>"#,
    )));
    let frame = base.resolved_frame.unwrap();
    assert_eq!(frame.w, Some(200.0));
    assert_eq!(frame.y, Ok(120.0));
    assert_eq!(frame.v_anchor, "page");
    assert_eq!(frame.h, Some(30.0));
    assert!(frame.h_exact);
    assert_eq!(frame.wrap, "notBeside");
}

#[test]
fn only_values_with_standard_ligatures_turn_them_on() {
    let lig = |v: &str| {
        let xml = format!(
            r#"<w:rPr xmlns:w="{}" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w14:ligatures w14:val="{v}"/></w:rPr>"#,
            crate::office::docx::testutil::W
        );
        RPr::parse(&crate::office::pptx::xml::parse_word(&xml).unwrap()).ligatures
    };
    assert_eq!(lig("standard"), Some(true));
    assert_eq!(lig("standardContextual"), Some(true));
    assert_eq!(lig("all"), Some(true));
    assert_eq!(lig("contextual"), Some(false));
    assert_eq!(lig("none"), Some(false));
}
