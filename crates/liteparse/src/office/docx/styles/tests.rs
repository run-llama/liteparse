use super::*;

fn sheet(inner: &str) -> StyleSheet {
    let xml = format!(
        r#"<w:styles xmlns:w="{}">{inner}</w:styles>"#,
        crate::office::docx::testutil::W
    );
    let root = crate::office::pptx::xml::parse_word(&xml).unwrap();
    StyleSheet::parse(Some(&root), ThemeFonts::default())
}

#[test]
fn toggles_flip_between_paragraph_and_character_styles() {
    let s = sheet(
        r#"<w:style w:type="paragraph" w:styleId="H"><w:rPr><w:b/></w:rPr></w:style>
           <w:style w:type="character" w:styleId="C"><w:rPr><w:b/></w:rPr></w:style>"#,
    );
    let direct = RPr {
        r_style: Some("C".into()),
        ..Default::default()
    };
    assert_eq!(s.resolve_rpr(Some("H"), &direct, None, None).b, Some(false));
    assert_eq!(
        s.resolve_rpr(Some("H"), &RPr::default(), None, None).b,
        Some(true)
    );
}

#[test]
fn based_on_chain_merges_ancestors() {
    let s = sheet(
        r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:pPr><w:spacing w:after="160"/></w:pPr></w:style>
           <w:style w:type="paragraph" w:styleId="Child"><w:basedOn w:val="Normal"/><w:pPr><w:jc w:val="center"/></w:pPr></w:style>"#,
    );
    let p = s.style_ppr("Child");
    assert_eq!(p.after, Some(8.0));
    assert_eq!(p.jc.as_deref(), Some("center"));
}

#[test]
fn table_style_sits_between_document_defaults_and_paragraph_styles() {
    let s = sheet(
        r#"<w:docDefaults><w:pPrDefault><w:pPr><w:spacing w:before="40" w:after="160"/></w:pPr></w:pPrDefault></w:docDefaults>
           <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:pPr><w:spacing w:after="240"/></w:pPr></w:style>"#,
    );
    let table = PPr {
        before: Some(0.0),
        after: Some(0.0),
        ..Default::default()
    };
    let cell = s.resolve_ppr(&PPr::default(), Some(&table));
    // Normal's own space after wins; the table style still beats the defaults.
    assert_eq!(cell.after, Some(12.0));
    assert_eq!(cell.before, Some(0.0));
}

#[test]
fn legacy_modes_let_the_table_style_size_replace_normals_twelve_points() {
    let mut s = sheet(
        r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:rPr><w:sz w:val="24"/></w:rPr></w:style>
           <w:style w:type="paragraph" w:styleId="Body"><w:basedOn w:val="Normal"/></w:style>
           <w:style w:type="paragraph" w:styleId="Own"><w:rPr><w:sz w:val="24"/></w:rPr></w:style>"#,
    );
    let table = RPr {
        sz: Some(8.0),
        ..Default::default()
    };
    let size = |s: &StyleSheet, style: &str| {
        s.resolve_rpr(Some(style), &RPr::default(), Some(&table), None)
            .sz
    };
    s.compat_mode = 14;
    assert_eq!(size(&s, "Normal"), Some(8.0));
    assert_eq!(size(&s, "Body"), Some(8.0));
    assert_eq!(size(&s, "Own"), Some(12.0));
    s.compat_mode = 15;
    assert_eq!(size(&s, "Normal"), Some(12.0));
}

#[test]
fn widow_control_defaults_on() {
    let p = ParaStyle::from_ppr(&PPr::default(), 12.0);
    assert!(p.widow);
    let off = PPr {
        widow: Some(false),
        ..Default::default()
    };
    assert!(!ParaStyle::from_ppr(&off, 12.0).widow);
}

#[test]
fn character_grid_exemption_inherits_and_direct_values_override() {
    let s = sheet(
        r#"<w:docDefaults><w:rPrDefault><w:rPr><w:snapToGrid w:val="0"/></w:rPr></w:rPrDefault></w:docDefaults><w:style w:type="paragraph" w:styleId="Grid"><w:pPr><w:snapToGrid w:val="0"/></w:pPr><w:rPr><w:snapToGrid/></w:rPr></w:style>"#,
    );
    let inherited = s.resolve_rpr(None, &RPr::default(), None, None);
    assert_eq!(inherited.snap_to_grid, Some(false));
    assert!(!s.run_style(&inherited).snap_to_grid);
    assert!(
        s.run_style(&s.resolve_rpr(Some("Grid"), &RPr::default(), None, None))
            .snap_to_grid
    );
    let direct = RPr {
        snap_to_grid: Some(false),
        ..Default::default()
    };
    assert!(
        !s.run_style(&s.resolve_rpr(Some("Grid"), &direct, None, None))
            .snap_to_grid
    );
    assert!(s.run_style(&RPr::default()).snap_to_grid);
}

#[test]
fn bidi_run_override_preserves_explicit_false_and_selects_complex_formatting() {
    let sheet = super::StyleSheet::parse(None, Default::default());
    let mut p = crate::office::docx::props::RPr {
        rtl: Some(true),
        sz: Some(10.0),
        sz_cs: Some(17.0),
        b: Some(false),
        b_cs: Some(true),
        i: Some(false),
        i_cs: Some(true),
        ..Default::default()
    };
    let r = sheet.run_style(&p);
    {
        assert_eq!(r.rtl, Some(true));
        assert_eq!(r.size, 17.0);
        assert!(r.bold && r.italic);
    }
    p.rtl = Some(false);
    let r = sheet.run_style(&p);
    assert_eq!(r.rtl, { Some(false) });
    assert_eq!(r.size, 10.0);
    assert!(!r.bold && !r.italic);
}

#[test]
fn logical_side_indents_resolve_after_paragraph_direction() {
    for (start, end) in [("left", "right"), ("start", "end")] {
        for rtl in [false, true] {
            for value in [-851, 0, 720] {
                let xml = format!(
                    r#"<w:pPr xmlns:w="{}"><w:bidi w:val="{}"/><w:ind w:{start}="400" w:{end}="{value}"/></w:pPr>"#,
                    crate::office::docx::testutil::W,
                    u8::from(rtl)
                );
                let parsed = PPr::parse(&crate::office::pptx::xml::parse_word(&xml).unwrap());
                let style = ParaStyle::from_ppr(&parsed, 12.0);
                let expected = if rtl {
                    (value as f64 / 20.0, 20.0)
                } else {
                    (20.0, value as f64 / 20.0)
                };
                assert_eq!(
                    (style.left, style.right),
                    expected,
                    "{start}/{end}, rtl={rtl}"
                );
            }
        }
    }
}

#[test]
fn inherited_logical_indent_uses_final_direction_and_direct_zero() {
    let s = sheet(
        r#"<w:style w:type="paragraph" w:styleId="Base"><w:pPr><w:bidi/><w:ind w:start="720" w:end="-851"/></w:pPr></w:style><w:style w:type="paragraph" w:styleId="Child"><w:basedOn w:val="Base"/><w:pPr><w:ind w:left="400"/></w:pPr></w:style>"#,
    );
    let mut p = s.style_ppr("Child");
    let rtl = ParaStyle::from_ppr(&p, 12.0);
    assert_eq!((rtl.left, rtl.right), (-42.55, 20.0));
    p.apply(&PPr {
        bidi: Some(false),
        ind_right: Some(0.0),
        ..Default::default()
    });
    let ltr = ParaStyle::from_ppr(&p, 12.0);
    assert_eq!((ltr.left, ltr.right), (20.0, 0.0));
}

#[test]
fn table_cell_side_indent_moves_toward_the_logical_edge() {
    let bounds = |rtl: bool, start: bool, indent: i32, modern: bool| {
        let attr = match (modern, start) {
            (false, true) => "left",
            (false, false) => "right",
            (true, true) => "start",
            (true, false) => "end",
        };
        let jc = if start { "left" } else { "right" };
        let body = format!(
            r#"<w:tbl><w:tblPr><w:tblW w:w="6000" w:type="dxa"/><w:tblLayout w:type="fixed"/></w:tblPr><w:tblGrid><w:gridCol w:w="6000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="6000" w:type="dxa"/></w:tcPr><w:p><w:pPr><w:bidi w:val="{}"/><w:jc w:val="{jc}"/><w:ind w:{attr}="{indent}"/></w:pPr><w:r><w:t>abc אבג 123</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
            u8::from(rtl)
        );
        let out = crate::office::docx::testutil::pages(crate::office::docx::testutil::docx(
            &body, None, None,
        ));
        let runs: Vec<_> = out[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) => Some(r),
                _ => None,
            })
            .collect();
        assert_eq!(
            runs.iter()
                .flat_map(|r| r.text.chars())
                .filter(|c| !c.is_whitespace())
                .collect::<String>(),
            "abcאבג123"
        );
        (
            runs.iter()
                .map(|r| r.transform.tx as f64)
                .fold(f64::INFINITY, f64::min),
            runs.iter()
                .map(|r| r.transform.tx as f64 + r.width)
                .fold(f64::NEG_INFINITY, f64::max),
        )
    };
    for rtl in [false, true] {
        for start in [false, true] {
            for modern in [false, true] {
                let zero = bounds(rtl, start, 0, modern);
                for indent in [-851, 720] {
                    let current = bounds(rtl, start, indent, modern);
                    let sign = if start != rtl { 1.0 } else { -1.0 };
                    let expected = sign * indent as f64 / 20.0;
                    assert!(
                        (current.0 - zero.0 - expected).abs() < 0.02,
                        "rtl={rtl},start={start},modern={modern}: {current:?} vs {zero:?}"
                    );
                    assert!((current.1 - zero.1 - expected).abs() < 0.02);
                }
            }
        }
    }
}

#[test]
fn a_missing_paragraph_default_takes_words_8_points_after_and_1_15_lines() {
    // Word for Mac probes on an 18 pt grid: lines step 20.9 pt inside a paragraph
    // and 28.8 pt between paragraphs when the document has no w:pPrDefault.
    let missing = sheet(r#"<w:docDefaults><w:rPrDefault><w:rPr/></w:rPrDefault></w:docDefaults>"#);
    assert_eq!(missing.default_ppr.after, Some(8.0));
    assert_eq!(missing.default_ppr.line, Some(276.0));
    // An empty one (or one that says otherwise) is taken as written.
    let empty = sheet(r#"<w:docDefaults><w:pPrDefault/></w:docDefaults>"#);
    assert_eq!(empty.default_ppr.after, None);
    let zero = sheet(
        r#"<w:docDefaults><w:pPrDefault><w:pPr><w:spacing w:after="0"/></w:pPr></w:pPrDefault></w:docDefaults>"#,
    );
    assert_eq!(zero.default_ppr.after, Some(0.0));
}

#[test]
fn a_style_without_an_id_is_found_by_its_name() {
    // A report generator's empty-cell style has a name but no styleId or type; Word
    // for Mac applies its 1 pt size to the paragraphs that reference that name.
    let s = sheet(
        r#"<w:style><w:name w:val="EmptyCellLayoutStyle"/><w:rPr><w:sz w:val="2"/></w:rPr></w:style>"#,
    );
    let r = s.resolve_rpr(Some("EmptyCellLayoutStyle"), &RPr::default(), None, None);
    assert_eq!(r.sz, Some(1.0));
}

#[test]
fn an_empty_complex_script_theme_font_takes_the_bidi_language_script_font() {
    // Hebrew runs under a theme with an empty minor `a:cs` draw in its Hebr font
    // (Arial in Word's PDF), Arabic ones in its Arab font.
    let theme = crate::office::pptx::xml::parse_word(
        r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:fontScheme><a:majorFont><a:latin typeface="Aptos Display"/><a:ea typeface=""/><a:cs typeface=""/><a:font script="Hebr" typeface="Times New Roman"/></a:majorFont><a:minorFont><a:latin typeface="Aptos"/><a:ea typeface=""/><a:cs typeface=""/><a:font script="Hebr" typeface="Arial"/><a:font script="Arab" typeface="Tahoma"/></a:minorFont></a:fontScheme></a:themeElements></a:theme>"#,
    )
    .unwrap();
    let cs = |lang: Option<&str>| theme_fonts(Some(&theme), None, lang).minor.2;
    assert_eq!(cs(Some("he-IL")), "Arial");
    assert_eq!(cs(Some("ar-SA")), "Tahoma");
    assert_eq!(cs(None), "");
    assert_eq!(
        theme_fonts(Some(&theme), None, Some("he")).major.2,
        "Times New Roman"
    );
}
