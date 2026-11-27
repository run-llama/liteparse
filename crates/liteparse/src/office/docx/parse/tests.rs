use super::*;
use crate::office::docx::testutil::{docx, para};

#[test]
fn wrapping_distances_override_anchor_per_edge_including_zero() {
    let anchor = |inner: &str| {
        crate::office::pptx::xml::parse_word(&format!(
        r#"<wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" distT="12700" distB="25400" distL="38100" distR="50800">{inner}</wp:anchor>"#)).unwrap()
    };
    for kind in ["wrapSquare", "wrapTight", "wrapThrough"] {
        let xml = anchor(&format!(r#"<wp:{kind} distL="0" distR="63500"/>"#));
        assert_eq!(drawing_wrap_distances(&xml), [1.0, 2.0, 0.0, 5.0]);
    }
    let xml = anchor(r#"<wp:wrapTopAndBottom distT="76200" distB="0"/>"#);
    assert_eq!(drawing_wrap_distances(&xml), [6.0, 0.0, 3.0, 4.0]);
    assert_eq!(
        drawing_wrap_distances(&anchor("<wp:wrapSquare/>")),
        [1.0, 2.0, 3.0, 4.0]
    );
}

#[test]
fn ruby_guide_size_overrides_run_size_and_preserves_base_style() {
    let s =
        sections(r#"<w:p><w:r><w:rPr><w:sz w:val="30"/><w:b/></w:rPr><w:t>base</w:t></w:r></w:p>"#);
    let Block::Para(p) = &s[0].blocks[0] else {
        panic!("paragraph")
    };
    let props = |align: &str| {
        crate::office::pptx::xml::parse_word(&format!(
        r#"<w:rubyPr xmlns:w="{}"><w:rubyAlign w:val="{align}"/><w:hps w:val="13"/><w:hpsRaise w:val="-2"/><w:hpsBaseText w:val="22"/></w:rubyPr>"#, crate::office::docx::testutil::W)).unwrap()
    };
    let r = ruby_group(&p.inlines, &p.inlines, &props("center")).unwrap();
    assert_eq!(r.base[0].1.size, 15.0);
    assert_eq!(r.guide[0].1.size, 6.5);
    assert!(r.guide[0].1.bold);
    assert_eq!(r.raise, -1.0);
    assert!(ruby_group(&p.inlines, &p.inlines, &props("rightVertical")).is_none());
    assert!(ruby_group(&[Inline::Tab(p.mark.clone())], &p.inlines, &props("left")).is_none());
}

fn sections(body: &str) -> Vec<Section> {
    let cfg = crate::office::pptx::config::Config::default();
    crate::office::docx::extract::open(docx(body, None, None), &cfg)
        .unwrap()
        .sections
}

#[test]
fn paragraph_section_break_splits_sections() {
    let body = format!(
        r#"{}<w:p><w:pPr><w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:type w:val="continuous"/></w:sectPr></w:pPr></w:p>{}<w:sectPr><w:pgSz w:w="12240" w:h="15840"/></w:sectPr>"#,
        para("a"),
        para("b")
    );
    let s = sections(&body);
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].blocks.len(), 2);
    assert!((s[0].props.page_w - 595.3).abs() < 0.1);
    assert_eq!(s[0].props.start, SectionStart::Continuous);
    assert_eq!(s[1].props.page_w, 612.0);
}

#[test]
fn page_field_result_is_kept_in_the_body() {
    let body = r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> PAGE \* MERGEFORMAT </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>7</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r><w:r><w:t>x</w:t></w:r></w:p>"#;
    let s = sections(body);
    let Block::Para(p) = &s[0].blocks[0] else {
        panic!("paragraph expected")
    };
    // In the body the saved result stays; headers and footers compute it.
    assert!(
        p.inlines
            .iter()
            .any(|i| matches!(i, Inline::Text(t, _) if t.contains('7')))
    );
    assert!(
        !p.inlines
            .iter()
            .any(|i| matches!(i, Inline::Field(FieldKind::Page(_), _)))
    );
}

#[test]
fn deleted_text_is_dropped_and_inserted_kept() {
    let body = r#"<w:p><w:del><w:r><w:delText>old</w:delText></w:r></w:del><w:ins><w:r><w:t>new</w:t></w:r></w:ins></w:p>"#;
    let s = sections(body);
    let Block::Para(p) = &s[0].blocks[0] else {
        panic!("paragraph expected")
    };
    assert_eq!(p.inlines.len(), 1);
    assert!(matches!(&p.inlines[0], Inline::Text(t, _) if t == "new"));
}

#[test]
fn a_deleted_paragraph_mark_joins_its_text_onto_the_next_paragraph() {
    let deleted_mark = r#"<w:pPr><w:rPr><w:del w:id="1" w:author="a"/></w:rPr></w:pPr>"#;
    let body = format!(
        r#"<w:p>{deleted_mark}<w:r><w:t xml:space="preserve">(1) </w:t></w:r></w:p><w:p>{deleted_mark}<w:del><w:r><w:delText>gone</w:delText></w:r></w:del></w:p><w:p><w:pPr><w:jc w:val="right"/></w:pPr><w:r><w:t>text</w:t></w:r></w:p>"#
    );
    let s = sections(&body);
    assert_eq!(s[0].blocks.len(), 1);
    let Block::Para(p) = &s[0].blocks[0] else {
        panic!("paragraph expected")
    };
    let text: String = p
        .inlines
        .iter()
        .filter_map(|i| match i {
            Inline::Text(t, _) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "(1) text");
    assert_eq!(p.style.jc, crate::office::docx::styles::Jc::Right);
}

#[test]
fn a_paragraph_mark_inside_field_code_joins_the_next_paragraph() {
    // The HYPERLINK's code runs past the first paragraph mark; Word hides the
    // code with the mark, so only the result's paragraph remains.
    let body = r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>HYPERLINK \l "a" \o "tip</w:instrText></w:r></w:p><w:p><w:r><w:instrText>more"</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Title</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p><w:p><w:r><w:t>body</w:t></w:r></w:p>"#;
    let s = sections(body);
    assert_eq!(s[0].blocks.len(), 2);
    let Block::Para(p) = &s[0].blocks[0] else {
        panic!("paragraph expected")
    };
    let text: String = p
        .inlines
        .iter()
        .filter_map(|i| match i {
            Inline::Text(t, _) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "Title");
}

#[test]
fn css_lengths() {
    assert_eq!(css_len("72pt"), Some(72.0));
    assert_eq!(css_len("1in"), Some(72.0));
    assert_eq!(css_len("96px"), Some(72.0));
}

#[test]
fn page_field_formats() {
    use crate::office::docx::model::FieldKind;
    assert_eq!(
        field_kind(r"PAGE \* roman \* MERGEFORMAT"),
        Some(FieldKind::Page(Some("lowerRoman")))
    );
    assert_eq!(
        field_kind(r" NUMPAGES  \* Arabic "),
        Some(FieldKind::NumPages(Some("decimal")))
    );
    assert_eq!(
        field_kind("SECTIONPAGES"),
        Some(FieldKind::SectionPages(None))
    );
}

/// A right-aligned shape of 200×100 pt rotated by `rot` (60000ths of a degree).
fn turned_shape_bounds(rot: i64) -> tiny_skia::Rect {
    let shape = format!(
        r#"<w:p><w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" relativeHeight="1" behindDoc="0"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="margin"><wp:align>right</wp:align></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="2540000" cy="1270000"/><wp:wrapNone/><wp:docPr id="1" name="s"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:spPr><a:xfrm rot="{rot}"><a:off x="0" y="0"/><a:ext cx="2540000" cy="1270000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></wps:spPr><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r></w:p>"#
    );
    let sect = r#"<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>"#;
    let pages = crate::office::docx::testutil::pages(docx(&format!("{shape}{sect}"), None, None));
    pages[0]
        .items
        .iter()
        .find_map(|it| match it {
            crate::office::pptx::scene::Item::Path(p) if p.fill.is_some() => Some(p.path.bounds()),
            _ => None,
        })
        .expect("filled shape")
}

#[test]
fn aligned_shapes_turned_a_quarter_align_their_turned_box() {
    // Right margin at 540 pt: the box turned upright (100 pt wide) ends there.
    let b = turned_shape_bounds(5_400_000);
    assert!((b.right() - 540.0).abs() < 0.5, "{b:?}");
    assert!((b.width() - 100.0).abs() < 0.5, "{b:?}");
    let b = turned_shape_bounds(0);
    assert!((b.right() - 540.0).abs() < 0.5 && (b.width() - 200.0).abs() < 0.5);
}

/// Indents a list style inherits give way to the level's; its own replace them.
#[test]
fn list_style_left_indent_keeps_the_level_hanging_indent() {
    let styles = r#"<w:style w:type="paragraph" w:styleId="Base"><w:pPr><w:ind w:left="720" w:right="720"/></w:pPr></w:style><w:style w:type="paragraph" w:styleId="L"><w:basedOn w:val="Base"/><w:pPr><w:numPr><w:numId w:val="1"/></w:numPr></w:pPr></w:style><w:style w:type="paragraph" w:styleId="H"><w:pPr><w:numPr><w:numId w:val="1"/></w:numPr><w:ind w:left="450"/></w:pPr></w:style>"#;
    let numbering = r#"<w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/><w:lvlText w:val="-"/><w:pPr><w:ind w:left="1440" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#;
    let body = r#"<w:p><w:pPr><w:pStyle w:val="L"/><w:ind w:left="360"/></w:pPr><w:r><w:t>x</w:t></w:r></w:p><w:p><w:pPr><w:pStyle w:val="H"/></w:pPr><w:r><w:t>y</w:t></w:r></w:p>"#;
    let cfg = crate::office::pptx::config::Config::default();
    let s = crate::office::docx::extract::open(docx(body, Some(styles), Some(numbering)), &cfg)
        .unwrap()
        .sections;
    let Block::Para(p) = &s[0].blocks[0] else {
        panic!("paragraph expected");
    };
    assert!(p.num.is_some());
    assert_eq!((p.style.left, p.style.first), (18.0, -18.0));
    let Block::Para(h) = &s[0].blocks[1] else {
        panic!("paragraph expected");
    };
    assert_eq!((h.style.left, h.style.first), (22.5, 0.0));
}

#[test]
fn drop_down_form_field_shows_its_selected_entry() {
    let field = |dd: &str, result: &str| {
        format!(
            r#"<w:p><w:r><w:fldChar w:fldCharType="begin"><w:ffData><w:ddList>{dd}<w:listEntry w:val="First"/><w:listEntry w:val="Second"/></w:ddList></w:ffData></w:fldChar></w:r><w:r><w:instrText> FORMDROPDOWN </w:instrText></w:r>{result}<w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#
        )
    };
    let sep = r#"<w:r><w:fldChar w:fldCharType="separate"/></w:r>"#;
    for (dd, result, want) in [
        ("", "", "First"),
        (r#"<w:result w:val="1"/>"#, sep, "Second"),
        (r#"<w:default w:val="1"/>"#, "", "Second"),
    ] {
        let s = sections(&field(dd, result));
        let Block::Para(p) = &s[0].blocks[0] else {
            panic!("paragraph expected")
        };
        let text: String = p
            .inlines
            .iter()
            .filter_map(|i| match i {
                Inline::Text(t, _) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, want);
    }
}

#[test]
fn footnote_separator_is_a_two_inch_rule() {
    let sheet = StyleSheet::parse(None, Default::default());
    let style = Rc::new(sheet.run_style(&RPr {
        sz: Some(11.0),
        ..Default::default()
    }));
    let Inline::Picture(p, _) = note_separator(&style, 144.0) else {
        panic!("picture expected")
    };
    assert_eq!(p.w, 144.0);
    assert!((p.h - 6.6).abs() < 1e-9);
    assert_eq!(p.shape.map(|s| s.items.len()), Some(1));
}

#[test]
fn ruby_preserves_formatted_base_text_and_respects_hidden_runs() {
    let ruby = r#"<w:ruby><w:rubyPr><w:hps w:val="12"/></w:rubyPr><w:rt><w:r><w:t>かんじ</w:t></w:r></w:rt><w:rubyBase><w:r><w:rPr><w:b/></w:rPr><w:t>漢字</w:t></w:r></w:rubyBase></w:ruby>"#;
    let body = format!(
        r#"<w:p><w:r><w:t>before</w:t>{ruby}<w:t>after</w:t></w:r><w:r><w:rPr><w:vanish/></w:rPr>{ruby}</w:r></w:p>"#
    );
    let sections = sections(&body);
    let Block::Para(p) = &sections[0].blocks[0] else {
        panic!("paragraph expected")
    };
    let text: String = p
        .inlines
        .iter()
        .filter_map(|i| match i {
            Inline::Text(t, _) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "before漢字after");
    assert!(
        p.inlines
            .iter()
            .any(|i| matches!(i, Inline::Text(t, s) if t == "漢字" && s.bold))
    );
}

#[test]
fn through_wrap_keeps_its_polygon_in_image_relative_coordinates() {
    let body = r#"<w:p><w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wp:extent cx="1270000" cy="635000"/><wp:wrapThrough wrapText="bothSides"><wp:wrapPolygon><wp:start x="-2160" y="0"/><wp:lineTo x="21600" y="0"/><wp:lineTo x="21600" y="21600"/></wp:wrapPolygon></wp:wrapThrough><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:spPr><a:prstGeom prst="rect"/></wps:spPr><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r></w:p>"#;
    let s = sections(body);
    let Block::Para(p) = &s[0].blocks[0] else {
        panic!("paragraph expected")
    };
    let a = p
        .inlines
        .iter()
        .find_map(|i| match i {
            Inline::Anchor(a) => Some(a),
            _ => None,
        })
        .unwrap();
    let contour = a.contour.as_ref().unwrap();
    assert!(contour.through);
    assert_eq!(contour.points, vec![(-0.1, 0.0), (1.0, 0.0), (1.0, 1.0)]);
}

#[test]
fn nil_vertical_cell_margins_preserve_inherited_padding_when_enabled() {
    let node = |body: &str| {
        crate::office::pptx::xml::parse_word(&format!(
            r#"<w:tcMar xmlns:w="{}">{body}</w:tcMar>"#,
            crate::office::docx::testutil::W
        ))
        .unwrap()
    };
    let mut margins = [8.0, 5.4, 12.0, 5.4];
    apply_margins(
        &mut margins,
        &node(r#"<w:top w:w="20" w:type="nil"/><w:bottom w:w="0" w:type="nil"/>"#),
    );
    assert_eq!(margins, [8.0, 5.4, 12.0, 5.4]);
    apply_margins(
        &mut margins,
        &node(r#"<w:top w:w="0" w:type="dxa"/><w:bottom w:w="40" w:type="dxa"/>"#),
    );
    assert_eq!(margins, [0.0, 5.4, 2.0, 5.4]);
}

#[test]
fn empty_none_labels_keep_level_indents_and_preserve_literal_labels() {
    for (format, label) in [("none", ""), ("none", "Prefix"), ("decimal", "")] {
        let numbering = format!(
            r#"<w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="{format}"/><w:lvlText w:val="{label}"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#
        );
        let body = r#"<w:p><w:pPr><w:numPr><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>body</w:t></w:r></w:p>"#;
        let opened = crate::office::docx::extract::open(
            docx(body, None, Some(&numbering)),
            &Default::default(),
        )
        .unwrap();
        let Block::Para(p) = &opened.sections[0].blocks[0] else {
            panic!("paragraph")
        };
        assert_eq!(p.style.left, 36.0);
        assert_eq!(p.style.first, -18.0);
        let hidden = format == "none" && label.is_empty();
        assert_eq!(p.num.is_none(), hidden);
        if !label.is_empty() {
            assert_eq!(p.num.as_ref().unwrap().text, label);
        }
    }
}

#[test]
fn character_grid_spacing_is_signed_and_distinct_from_line_pitch() {
    for (kind, raw) in [
        ("linesAndChars", "1276"),
        ("linesAndChars", "-1683"),
        ("snapToChars", "4096"),
        ("lines", "4096"),
    ] {
        let body = format!(
            r#"<w:p/><w:sectPr><w:docGrid w:type="{kind}" w:linePitch="301" w:charSpace="{raw}"/></w:sectPr>"#
        );
        let s = sections(&body);
        assert_eq!(
            s[0].props.grid_char_space,
            Some(raw.parse::<f64>().unwrap() / 4096.0)
        );
        assert_eq!(s[0].props.grid_pitch, Some(15.05));
        assert_eq!(
            s[0].props.character_grid().is_some(),
            kind == "linesAndChars"
        );
    }
    assert_eq!(sections("<w:p/>")[0].props.grid_char_space, None);
}

#[test]
fn character_grid_unsigned_signed32_compatibility_is_bounded() {
    for (raw, signed) in [
        ("4294964838", -2458.0),
        ("4294967295", -1.0),
        ("2147483648", -2147483648.0),
    ] {
        let body = format!(
            r#"<w:p/><w:sectPr><w:docGrid w:type="linesAndChars" w:charSpace="{raw}"/></w:sectPr>"#
        );
        let expected = signed;
        assert_eq!(
            sections(&body)[0].props.grid_char_space,
            Some(expected / 4096.0)
        );
    }
    for raw in ["-2458", "0", "4096", "2147483647", "4294967296", "1276.5"] {
        assert_eq!(
            super::character_grid_space(raw),
            Some(raw.parse::<f64>().unwrap() / 4096.0)
        );
    }
    assert_eq!(super::character_grid_space("invalid"), None);
}

#[test]
fn empty_level_suffix_preserves_placeholder_missing_and_null_patterns() {
    for level_text in [
        r#"<w:lvlText w:val="%1"/>"#,
        "",
        r#"<w:lvlText/>"#,
        r#"<w:lvlText w:val="" w:null="true"/>"#,
    ] {
        let numbering = format!(
            r#"<w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:numFmt w:val="none"/>{level_text}</w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#
        );
        let body = r#"<w:p><w:pPr><w:numPr><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>body</w:t></w:r></w:p>"#;
        let opened = crate::office::docx::extract::open(
            docx(body, None, Some(&numbering)),
            &Default::default(),
        )
        .unwrap();
        let Block::Para(p) = &opened.sections[0].blocks[0] else {
            panic!("paragraph")
        };
        assert!(p.num.is_some(), "{level_text}");
        if let Some(label) = &p.num {
            assert!(label.text.is_empty());
            assert!(matches!(label.suffix, Suffix::Tab));
        }
    }
}

#[test]
fn section_footnote_position_preserves_word_mapping_and_document_scope() {
    for (value, expected) in [
        ("", FootnotePosition::PageBottom),
        ("pageBottom", FootnotePosition::PageBottom),
        ("beneathText", FootnotePosition::BeneathText),
        ("sectEnd", FootnotePosition::BeneathText),
        ("docEnd", FootnotePosition::DocumentEnd),
    ] {
        let position = if value.is_empty() {
            String::new()
        } else {
            format!(r#"<w:footnotePr><w:pos w:val="{value}"/></w:footnotePr>"#)
        };
        let body = format!("{}<w:sectPr>{position}</w:sectPr>", para("body"));
        let settings = format!(
            r#"<w:settings xmlns:w="{}"><w:footnotePr><w:pos w:val="beneathText"/></w:footnotePr></w:settings>"#,
            crate::office::docx::testutil::W
        );
        let bytes = crate::office::docx::testutil::with_related_part(
            docx(&body, None, None),
            "settings",
            "settings.xml",
            &settings,
            false,
        );
        let cfg = crate::office::pptx::config::Config::default();
        let parsed = crate::office::docx::extract::open(bytes, &cfg).unwrap();
        assert_eq!(parsed.sections[0].props.note_position, expected, "{value}");
    }
}

#[test]
fn section_footnote_columns_resolves_namespace_and_bounds_values() {
    for (value, expected) in [
        (None, FootnoteColumns::Auto),
        (Some("0"), FootnoteColumns::Auto),
        (Some("1"), FootnoteColumns::Count(1)),
        (Some("2"), FootnoteColumns::Count(2)),
        (Some("-1"), FootnoteColumns::Invalid),
        (Some("4294967296"), FootnoteColumns::Invalid),
        (Some("oops"), FootnoteColumns::Invalid),
    ] {
        let count = value.map_or(String::new(), |v| {
            format!(r#"<renamed:footnoteColumns w:val="{v}"/>"#)
        });
        let body = format!(
            r#"<w:p/><w:sectPr xmlns:renamed="http://schemas.microsoft.com/office/word/2012/wordml">{count}</w:sectPr>"#
        );
        assert_eq!(sections(&body)[0].props.note_columns, expected);
    }
}

fn math_block_text(blocks: &[Block]) -> String {
    blocks
        .iter()
        .map(|block| match block {
            Block::Para(p) => p
                .inlines
                .iter()
                .filter_map(|i| match i {
                    Inline::Text(t, _) => Some(t.as_str()),
                    _ => None,
                })
                .collect::<String>(),
            Block::Table(t) => t
                .rows
                .iter()
                .flat_map(|r| &r.cells)
                .map(|c| math_block_text(&c.blocks))
                .collect(),
        })
        .collect()
}

fn authored_math(text: &str, para: bool) -> String {
    let math = format!(
        r#"<m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:r><m:t>{text}</m:t></m:r></m:oMath>"#
    );
    if para {
        format!(
            r#"<m:oMathPara xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math">{math}</m:oMathPara>"#
        )
    } else {
        math
    }
}

#[test]
fn direct_math_blocks_preserve_body_cell_and_revision_order() {
    let first = authored_math("MATH001", true);
    let second = authored_math("MATH002", false);
    let body = format!(
        "{}{}<w:del>{}</w:del><w:ins>{}</w:ins><w:moveFrom>{}</w:moveFrom><w:moveTo>{}</w:moveTo>{}",
        para("START"),
        first,
        authored_math("DELETED", true),
        second,
        authored_math("OLD", false),
        authored_math("MOVED", false),
        para("END")
    );
    let s = sections(&body);
    assert_eq!(math_block_text(&s[0].blocks), "STARTMATH001MATH002MOVEDEND");
    let Block::Para(p) = &s[0].blocks[1] else {
        panic!("math paragraph")
    };
    assert_eq!(p.style.jc, crate::office::docx::styles::Jc::Center);
    let cell = sections(&format!("<w:tbl><w:tr><w:tc>{body}</w:tc></w:tr></w:tbl>"));
    assert_eq!(
        math_block_text(&cell[0].blocks),
        math_block_text(&s[0].blocks)
    );
}

#[test]
fn direct_math_story_uses_document_default_and_local_justification() {
    let math = authored_math("STORY001", true);
    let body = format!(
        r#"{math}<w:sectPr><w:headerReference w:type="default" r:id="rheader"/></w:sectPr>"#
    );
    let settings = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:mathPr><m:defJc m:val="right"/></m:mathPr></w:settings>"#;
    let data = crate::office::docx::testutil::with_related_part(
        docx(&body, None, None),
        "settings",
        "settings.xml",
        settings,
        false,
    );
    let header = format!(
        r#"<w:hdr xmlns:w="{}">{math}</w:hdr>"#,
        crate::office::docx::testutil::W
    );
    let data = crate::office::docx::testutil::with_related_part(
        data,
        "header",
        "header1.xml",
        &header,
        false,
    );
    let opened =
        crate::office::docx::extract::open(data, &crate::office::pptx::config::Config::default())
            .unwrap();
    assert_eq!(
        math_block_text(&opened.hdr_ftr["word/header1.xml"]),
        "STORY001"
    );
    let Block::Para(p) = &opened.hdr_ftr["word/header1.xml"][0] else {
        panic!("math paragraph")
    };
    assert_eq!(p.style.jc, crate::office::docx::styles::Jc::Right);
}

#[test]
fn math_revision_content_matches_accepted_word_revision_policy() {
    let math = r#"<m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:r><m:t>KEEP</m:t></m:r><w:del><m:r><m:t>DELETED</m:t></m:r></w:del><w:moveFrom><m:r><m:t>OLD</m:t></m:r></w:moveFrom><w:ins><m:r><m:t>INSERT</m:t></m:r></w:ins><w:moveTo><m:r><m:t>MOVED</m:t></m:r></w:moveTo></m:oMath>"#;
    for body in [math.to_owned(), format!("<w:p>{math}</w:p>")] {
        assert_eq!(
            math_block_text(&sections(&body)[0].blocks),
            "KEEPINSERTMOVED"
        );
    }
}

#[test]
fn math_paragraph_preserves_direct_runs_and_local_alignment() {
    for (jc, expected) in [
        ("left", crate::office::docx::styles::Jc::Left),
        ("right", crate::office::docx::styles::Jc::Right),
        ("centerGroup", crate::office::docx::styles::Jc::Center),
    ] {
        let math = format!(
            r#"<m:oMathPara xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:oMathParaPr><m:jc m:val="{jc}"/></m:oMathParaPr><m:r><m:t>FIRST</m:t></m:r><w:ins><m:r><m:t>SECOND</m:t></m:r></w:ins><w:del><m:r><m:t>DELETED</m:t></m:r></w:del><m:oMath><m:r><m:t>EQUATION</m:t></m:r></m:oMath><m:r><m:t>LAST</m:t></m:r></m:oMathPara>"#
        );
        for body in [math.clone(), format!("<w:p>{math}</w:p>")] {
            assert_eq!(
                math_block_text(&sections(&body)[0].blocks),
                "FIRSTSECOND EQUATION LAST"
            );
        }
        let parsed = sections(&math);
        let Block::Para(p) = &parsed[0].blocks[0] else {
            panic!("math paragraph")
        };
        assert_eq!(p.style.jc, expected);
    }
}

#[test]
fn math_paragraph_keeps_equations_nested_in_content_wrappers() {
    let math = authored_math("NESTED", false);
    for wrapper in [
        format!("<w:sdt><w:sdtContent>{math}</w:sdtContent></w:sdt>"),
        format!("<w:customXml>{math}</w:customXml>"),
        format!("<w:hyperlink>{math}</w:hyperlink>"),
    ] {
        let body = format!(
            r#"<m:oMathPara xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:r><m:t>FIRST</m:t></m:r>{wrapper}<m:r><m:t>LAST</m:t></m:r></m:oMathPara>"#
        );
        assert_eq!(
            math_block_text(&sections(&body)[0].blocks),
            "FIRST NESTED LAST"
        );
    }
}

#[test]
fn table_layout_ignores_style_and_keeps_direct_and_row_precedence() {
    let styles = r#"<w:style w:type="table" w:styleId="Base"><w:tblPr><w:tblW w:type="pct" w:w="5000"/><w:tblLayout w:type="fixed"/></w:tblPr></w:style><w:style w:type="table" w:styleId="Derived"><w:basedOn w:val="Base"/></w:style>"#;
    for (direct, fraction, layout) in [
        ("", None, TableLayout::AutoFit),
        (r#"<w:tblLayout w:type="fixed"/>"#, None, TableLayout::Fixed),
        (
            r#"<w:tblW w:type="pct" w:w="125%"/><w:tblLayout w:type="autofit"/>"#,
            Some(1.25),
            TableLayout::AutoFit,
        ),
        (
            r#"<w:tblW w:type="dxa" w:w="5000"/>"#,
            None,
            TableLayout::AutoFit,
        ),
        (
            r#"<w:tblW w:type="auto" w:w="0"/>"#,
            None,
            TableLayout::AutoFit,
        ),
        (
            r#"<w:tblW w:type="pct" w:w="inf"/>"#,
            None,
            TableLayout::AutoFit,
        ),
    ] {
        let body = format!(
            r#"<w:tbl><w:tblPr><w:tblStyle w:val="Derived"/>{direct}</w:tblPr><w:tr><w:tblPrEx><w:tblLayout w:type="autofit"/></w:tblPrEx><w:tc><w:p/></w:tc></w:tr><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>"#
        );
        let opened = crate::office::docx::extract::open(
            docx(&body, Some(styles), None),
            &Default::default(),
        )
        .unwrap();
        let Block::Table(table) = &opened.sections[0].blocks[0] else {
            panic!("table")
        };
        assert_eq!(table.preferred_width.fraction(), fraction);
        assert_eq!(table.layout, layout);
        assert_eq!(table.rows[0].layout, TableLayout::AutoFit);
        assert_eq!(table.rows[1].layout, layout);
    }
    let parsed = sections("<w:tbl><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>");
    let Block::Table(table) = &parsed[0].blocks[0] else {
        panic!("table")
    };
    assert_eq!(table.layout, TableLayout::AutoFit);
}

#[test]
fn table_preferred_width_keeps_dxa_distinct_from_layout_and_auto() {
    for (properties, expected) in [
        (r#"<w:tblW w:w="12000"/>"#, TableWidth::Dxa(600.0)),
        (
            r#"<w:tblW w:type="dxa" w:w="12000"/>"#,
            TableWidth::Dxa(600.0),
        ),
        (
            r#"<w:tblW w:type="pct" w:w="125%"/>"#,
            TableWidth::Percent(1.25),
        ),
        (r#"<w:tblW w:type="auto" w:w="0"/>"#, TableWidth::Auto),
        (r#"<w:tblW w:type="nil" w:w="12000"/>"#, TableWidth::Auto),
        (r#"<w:tblW w:type="dxa" w:w="-1"/>"#, TableWidth::Auto),
        (r#"<w:tblW w:type="dxa" w:w="0"/>"#, TableWidth::Auto),
        (
            r#"<w:tblW w:type="dxa" w:w="125%"/>"#,
            TableWidth::Percent(1.25),
        ),
    ] {
        let parsed = sections(&format!(
            "<w:tbl><w:tblPr>{properties}</w:tblPr><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>"
        ));
        let Block::Table(table) = &parsed[0].blocks[0] else {
            panic!("table")
        };
        assert_eq!(table.preferred_width, expected);
        assert_eq!(table.layout, TableLayout::AutoFit);
    }
}

#[test]
fn table_width_uses_only_direct_or_first_row_exception() {
    for (first, later, expected) in [
        (
            "",
            r#"<w:tblW w:type="dxa" w:w="1000"/>"#,
            TableWidth::Dxa(300.0),
        ),
        (
            r#"<w:tblW w:type="dxa" w:w="2400"/>"#,
            "",
            TableWidth::Dxa(120.0),
        ),
        (r#"<w:tblW w:type="auto" w:w="0"/>"#, "", TableWidth::Auto),
        (r#"<w:tblW w:type="nil" w:w="0"/>"#, "", TableWidth::Auto),
    ] {
        let parsed = sections(&format!(
            r#"<w:tbl><w:tblPr><w:tblW w:type="dxa" w:w="6000"/></w:tblPr><w:tr><w:tblPrEx>{first}</w:tblPrEx><w:tc><w:p/></w:tc></w:tr><w:tr><w:tblPrEx>{later}</w:tblPrEx><w:tc><w:p/></w:tc></w:tr></w:tbl>"#
        ));
        let Block::Table(table) = &parsed[0].blocks[0] else {
            panic!("table")
        };
        assert_eq!(table.preferred_width, expected);
    }
}

#[test]
fn preferred_width_uses_wrapped_first_logical_row() {
    for (open, close) in [
        ("<w:sdt><w:sdtContent>", "</w:sdtContent></w:sdt>"),
        ("<w:customXml>", "</w:customXml>"),
    ] {
        let parsed = sections(&format!(
            r#"<w:tbl><w:tblPr><w:tblW w:type="dxa" w:w="6000"/></w:tblPr>{open}<w:tr><w:tblPrEx><w:tblW w:type="dxa" w:w="2400"/></w:tblPrEx><w:tc><w:tcPr><w:tcW w:type="dxa" w:w="2400"/></w:tcPr><w:p/></w:tc></w:tr>{close}<w:tr><w:tblPrEx><w:tblW w:type="dxa" w:w="1000"/></w:tblPrEx><w:tc><w:p/></w:tc></w:tr></w:tbl>"#
        ));
        let Block::Table(table) = &parsed[0].blocks[0] else {
            panic!("table")
        };
        assert_eq!(table.preferred_width, TableWidth::Dxa(120.0));
        assert_eq!(
            table.rows[0].cells[0].preferred_width,
            TableWidth::Dxa(120.0)
        );
        assert_eq!(table.rows.len(), 2);
    }
}

#[test]
fn no_wrap_retains_source_presence_and_explicit_false() {
    for (value, expected) in [
        ("", true),
        (r#" w:val="1""#, true),
        (r#" w:val="0""#, false),
        (r#" w:val="false""#, false),
        (r#" w:val="off""#, false),
    ] {
        let parsed = sections(&format!(
            r#"<w:tbl><w:tr><w:tc><w:tcPr><w:noWrap{value}/></w:tcPr><w:p/></w:tc></w:tr></w:tbl>"#
        ));
        let Block::Table(table) = &parsed[0].blocks[0] else {
            panic!("table")
        };
        assert_eq!(table.rows[0].cells[0].no_wrap, expected);
    }
}

#[test]
fn no_wrap_cascades_table_style_region_and_direct_reset() {
    let styles = r#"<w:style w:type="table" w:styleId="WrapPolicy"><w:tcPr><w:noWrap/></w:tcPr><w:tblStylePr w:type="firstRow"><w:tcPr><w:noWrap w:val="0"/></w:tcPr></w:tblStylePr></w:style>"#;
    let source = r#"<w:tbl><w:tblPr><w:tblStyle w:val="WrapPolicy"/><w:tblLook w:firstRow="1" w:noHBand="1" w:noVBand="1"/></w:tblPr><w:tr><w:tc><w:p/></w:tc><w:tc><w:tcPr><w:noWrap/></w:tcPr><w:p/></w:tc></w:tr><w:tr><w:tc><w:p/></w:tc><w:tc><w:tcPr><w:noWrap w:val="false"/></w:tcPr><w:p/></w:tc></w:tr></w:tbl>"#;
    let opened =
        crate::office::docx::extract::open(docx(source, Some(styles), None), &Default::default())
            .unwrap();
    let Block::Table(table) = &opened.sections[0].blocks[0] else {
        panic!("table")
    };
    assert!(!table.rows[0].cells[0].no_wrap);
    assert!(table.rows[0].cells[1].no_wrap);
    assert!(table.rows[1].cells[0].no_wrap);
    assert!(!table.rows[1].cells[1].no_wrap);
}

#[test]
fn conditional_column_no_wrap_applies_to_row_with_direct_cell_reset() {
    let styles = r#"<w:style w:type="table" w:styleId="WrapPolicy"><w:tblStylePr w:type="firstCol"><w:tcPr><w:noWrap/></w:tcPr></w:tblStylePr></w:style>"#;
    let source = r#"<w:tbl><w:tblPr><w:tblStyle w:val="WrapPolicy"/><w:tblLook w:firstColumn="1" w:firstRow="0" w:noHBand="1" w:noVBand="1"/></w:tblPr><w:tr><w:tc><w:p/></w:tc><w:tc><w:p/></w:tc><w:tc><w:tcPr><w:noWrap w:val="false"/></w:tcPr><w:p/></w:tc></w:tr></w:tbl>"#;
    let opened =
        crate::office::docx::extract::open(docx(source, Some(styles), None), &Default::default())
            .unwrap();
    let Block::Table(table) = &opened.sections[0].blocks[0] else {
        panic!("table")
    };
    assert!(table.rows[0].cells[0].no_wrap);
    assert!(table.rows[0].cells[1].no_wrap);
    assert!(!table.rows[0].cells[2].no_wrap);
}

#[test]
fn inline_quarter_turn_layout_rebases_shape_and_text_with_effect_sides() {
    for (angle, swapped) in [
        (0.0, false),
        (44.99, false),
        (45.0, true),
        (90.0, true),
        (134.99, true),
        (135.0, false),
        (180.0, false),
        (270.0, true),
    ] {
        let rot = angle * 60000.0;
        let body = format!(
            r#"<w:p><w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wp:extent cx="508000" cy="1270000"/><wp:effectExtent l="12700" t="25400" r="38100" b="50800"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:spPr><a:xfrm rot="{rot}"><a:off x="0" y="0"/><a:ext cx="508000" cy="1270000"/></a:xfrm><a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="0080FF"/></a:solidFill></wps:spPr><wps:txbx><w:txbxContent><w:p><w:r><w:t>OWNER</w:t></w:r></w:p></w:txbxContent></wps:txbx><wps:bodyPr upright="1"/></wps:wsp></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#
        );
        let parsed = sections(&body);
        let Block::Para(p) = &parsed[0].blocks[0] else {
            panic!("paragraph")
        };
        let Inline::Picture(pic, _) = &p.inlines[0] else {
            panic!("picture")
        };
        assert_eq!(
            (pic.w, pic.h),
            if swapped {
                (104.0, 46.0)
            } else {
                (44.0, 106.0)
            }
        );
        let spec = pic.shape.as_ref().unwrap();
        let tb = spec.texts[0].upright_frame();
        let expected = if swapped { (51.0, 22.0) } else { (20.0, 50.0) };
        assert!((tb.rect.x + tb.rect.w / 2.0 - expected.0).abs() < 0.001);
        assert!((tb.rect.y + tb.rect.h / 2.0 - expected.1).abs() < 0.001);
        if angle == 90.0 || angle == 270.0 {
            let bounds = spec
                .items
                .iter()
                .find_map(|it| match it {
                    crate::office::pptx::scene::Item::Path(p) => Some(p.path.bounds()),
                    _ => None,
                })
                .unwrap();
            assert!((bounds.left() - 1.0).abs() < 0.001);
            assert!((bounds.top() - 2.0).abs() < 0.001);
            assert!((bounds.width() - 100.0).abs() < 0.001);
        }
    }
}

#[test]
fn large_merge_spans_keep_complete_grid_identity() {
    for continuation in [70, 64] {
        let body = format!(
            r#"<w:tbl><w:tblGrid>{}</w:tblGrid><w:tr><w:trPr><w:gridBefore w:val="1"/></w:trPr><w:tc><w:tcPr><w:gridSpan w:val="70"/><w:vMerge w:val="restart"/></w:tcPr>{}</w:tc></w:tr><w:tr><w:trPr><w:gridBefore w:val="1"/></w:trPr><w:tc><w:tcPr><w:gridSpan w:val="{continuation}"/><w:vMerge/></w:tcPr>{}</w:tc></w:tr></w:tbl>"#,
            r#"<w:gridCol w:w="80"/>"#.repeat(76),
            para("MERGED"),
            para("TAIL")
        );
        let sections = sections(&body);
        let Block::Table(table) = &sections[0].blocks[0] else {
            panic!("table")
        };
        assert_eq!(table.rows[0].cells[0].span, 70);
        let Block::Para(p) = &table.rows[0].cells[0].blocks[0] else {
            panic!("paragraph")
        };
        let Inline::Text(_, style) = &p.inlines[0] else {
            panic!("text")
        };
        let source = style.source.as_ref().unwrap();
        let cell = &source.cells[0];
        assert_eq!(cell.span, 70);
        assert_eq!(cell.vertical_merges.len(), usize::from(continuation == 70));
        if continuation == 70 {
            assert_eq!(cell.vertical_merges[0].grid_start, 1);
            assert_eq!(cell.vertical_merges[0].span, 70);
        }
    }
}

#[test]
fn an_empty_header_distance_is_zero() {
    let margins = |header: &str| {
        let body = format!(
            r#"{}<w:sectPr><w:pgMar w:top="216" w:bottom="216" w:left="216" w:right="216" {header}/></w:sectPr>"#,
            para("body")
        );
        let cfg = crate::office::pptx::config::Config::default();
        let parsed = crate::office::docx::extract::open(docx(&body, None, None), &cfg).unwrap();
        (
            parsed.sections[0].props.header,
            parsed.sections[0].props.footer,
        )
    };
    assert_eq!(margins(r#"w:header="" w:footer="""#), (0.0, 0.0));
    // A missing distance keeps the half-inch default.
    assert_eq!(margins(""), (36.0, 36.0));
}
