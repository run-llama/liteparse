use crate::office::docx::testutil::{docx, pages};

#[test]
fn automatic_spacing_on_a_tall_picture_only_line() {
    let y = |spacing: u32| {
        let body = format!(
            r#"<w:p><w:pPr><w:spacing w:after="0" w:line="{spacing}" w:lineRule="auto"/></w:pPr><w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="914400" cy="914400"/></wp:inline></w:drawing></w:r></w:p><w:p><w:r><w:t>after</w:t></w:r></w:p>"#
        );
        pages(docx(&body, None, None))[0]
            .items
            .iter()
            .find_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text == "after" => {
                    Some(r.transform.ty)
                }
                _ => None,
            })
            .unwrap()
    };
    let delta = y(480) - y(240);
    assert!(delta > 5.0, "extra height {delta}");
}

#[test]
fn ruby_keeps_two_tiers_on_one_line_with_the_saved_raise() {
    use super::*;
    let cfg = crate::office::pptx::config::Config::default();
    let mut opened = crate::office::docx::extract::open(
        docx(&crate::office::docx::testutil::para("base"), None, None),
        &cfg,
    )
    .unwrap();
    let crate::office::docx::model::Block::Para(p) = &mut opened.sections[0].blocks[0] else {
        panic!("paragraph")
    };
    let base = p.mark.clone();
    let mut guide = (*base).clone();
    guide.size = 6.0;
    p.inlines = vec![
        Inline::Ruby(Box::new(crate::office::docx::model::Ruby {
            base: vec![("base".into(), base)],
            guide: vec![("guide".into(), Rc::new(guide))],
            align: "center".into(),
            raise: 12.0,
        })),
        Inline::Text("after".into(), p.mark.clone()),
    ];
    let fonts = FontSystem::new(&[], true, &Default::default());
    let out = crate::office::docx::extract::layout(&opened, &fonts);
    let text: Vec<_> = out.pages[0]
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Text(r) => Some(r),
            _ => None,
        })
        .collect();
    let find = |t: &str| text.iter().find(|r| r.text == t).unwrap();
    assert!((find("base").transform.ty - find("guide").transform.ty - 12.0).abs() < 0.01);
    assert!((find("base").transform.ty - find("after").transform.ty).abs() < 0.01);
    assert!(find("after").transform.tx > find("base").transform.tx);
}

#[test]
fn ruby_distribution_accounts_for_outer_half_spaces() {
    use super::*;
    let s = crate::office::docx::styles::StyleSheet::parse(None, Default::default())
        .run_style(&Default::default());
    let s = Rc::new(s);
    let atoms = || {
        (0..3)
            .map(|_| Atom {
                width: 10.0,
                ..blank(Kind::Text, &s)
            })
            .collect()
    };
    assert_eq!(
        ruby_tier(atoms(), 60.0, "distributeLetter").xs,
        vec![0.0, 25.0, 50.0]
    );
    assert_eq!(
        ruby_tier(atoms(), 60.0, "distributeSpace").xs,
        vec![5.0, 25.0, 45.0]
    );
    assert_eq!(ruby_tier(atoms(), 60.0, "right").xs, vec![30.0, 40.0, 50.0]);
}

#[test]
fn long_paragraph_wraps_and_breaks_split_lines() {
    let words = "word ".repeat(200);
    let body = format!(
        r#"<w:p><w:r><w:t xml:space="preserve">{words}</w:t><w:br/><w:t>after</w:t></w:r></w:p>"#
    );
    let p = pages(docx(&body, None, None));
    let runs: Vec<f32> = p[0]
        .items
        .iter()
        .filter_map(|i| match i {
            crate::office::pptx::scene::Item::Text(r) => Some(r.transform.ty),
            _ => None,
        })
        .collect();
    let mut ys = runs;
    ys.dedup();
    assert!(ys.len() > 5, "expected several lines, got {}", ys.len());
}

#[test]
fn very_long_paragraph_lays_out_in_linear_time() {
    // One run of 20,000 words: shaping must not scan the run once per cluster.
    let words = "word ".repeat(20_000);
    let body = format!(r#"<w:p><w:r><w:t xml:space="preserve">{words}</w:t></w:r></w:p>"#);
    let started = std::time::Instant::now();
    let p = pages(docx(&body, None, None));
    assert!(p.len() > 5);
    assert!(
        started.elapsed().as_secs() < 30,
        "took {:?}",
        started.elapsed()
    );
}

/// A list paragraph that opens with a page break shows its label on the next
/// page with its text, and no label when it holds only the break.
#[test]
fn list_label_follows_an_opening_page_break() {
    let numbering = r#"<w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="upperRoman"/><w:lvlText w:val="%1."/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#;
    let body = r#"<w:p><w:r><w:t>a</w:t></w:r></w:p><w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:br w:type="page"/><w:t>b</w:t></w:r></w:p><w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:br w:type="page"/></w:r></w:p>"#;
    let p = pages(docx(body, None, Some(numbering)));
    let text = |i: usize| -> String {
        p[i].items
            .iter()
            .filter_map(|it| match it {
                crate::office::pptx::scene::Item::Text(r) => Some(r.text.clone()),
                _ => None,
            })
            .collect()
    };
    assert_eq!(p.len(), 3);
    assert!(!text(0).contains("I."), "{}", text(0));
    assert!(
        text(1).contains("I.") && text(1).contains('b'),
        "{}",
        text(1)
    );
    assert!(!text(1).contains("II."), "{}", text(1));
    assert!(!text(2).contains("II."), "{}", text(2));
}

/// The line after a paragraph's opening page break takes the first-line indent.
#[test]
fn text_after_an_opening_page_break_starts_at_the_first_line_indent() {
    let body = r#"<w:p><w:r><w:t>a</w:t></w:r></w:p><w:p><w:pPr><w:ind w:left="720" w:hanging="720"/></w:pPr><w:r><w:br w:type="page"/><w:t>b</w:t><w:br/><w:t>c</w:t></w:r></w:p>"#;
    let p = pages(docx(body, None, None));
    let x = |t: &str| {
        p[1].items.iter().find_map(|it| match it {
            crate::office::pptx::scene::Item::Text(r) if r.text == t => Some(r.transform.tx),
            _ => None,
        })
    };
    // "b" sits 36 pt left of the line after it.
    let (b, c) = (x("b").unwrap(), x("c").unwrap());
    assert!((c - b - 36.0).abs() < 0.5, "{b} {c}");
}

/// Baselines of the text runs on page 1, one per line.
fn line_ys(bytes: Vec<u8>) -> Vec<f32> {
    let mut ys: Vec<f32> = pages(bytes)[0]
        .items
        .iter()
        .filter_map(|i| match i {
            crate::office::pptx::scene::Item::Text(r) => Some(r.transform.ty),
            _ => None,
        })
        .collect();
    ys.dedup_by(|a, b| (*a - *b).abs() < 0.5);
    ys
}

#[test]
fn deeper_label_descent_keeps_the_text_line_height() {
    let run = r#"<w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman"/><w:sz w:val="24"/></w:rPr>"#;
    let item = |num: &str| {
        format!(
            r#"<w:p><w:pPr>{num}<w:spacing w:after="0" w:line="240" w:lineRule="auto"/>{run}</w:pPr><w:r>{run}<w:t>item</w:t></w:r></w:p>"#
        )
    };
    let numbering = r#"<w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="o"/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr><w:rPr><w:rFonts w:ascii="Courier New" w:hAnsi="Courier New"/></w:rPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#;
    let num = r#"<w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr>"#;
    let listed = line_ys(docx(&item(num).repeat(3), None, Some(numbering)));
    let plain = line_ys(docx(&item("").repeat(3), None, None));
    assert_eq!(listed.len(), 3, "{listed:?}");
    let pitch = |ys: &[f32]| ys[2] - ys[1];
    assert!(
        (pitch(&listed) - pitch(&plain)).abs() < 0.05,
        "listed {listed:?} plain {plain:?}"
    );
}

/// An 18 pt tab after the last 11 pt word of a line does not make it taller.
#[test]
fn a_trailing_tab_does_not_size_its_line() {
    let line = |tab: &str| {
        format!(
            r#"<w:p><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr><w:r><w:rPr><w:sz w:val="22"/></w:rPr><w:t>one</w:t></w:r>{tab}</w:p>"#
        )
    };
    let tab = r#"<w:r><w:rPr><w:sz w:val="36"/></w:rPr><w:tab/></w:r>"#;
    let with = line_ys(docx(&format!("{}{}", line(tab), line("")), None, None));
    let without = line_ys(docx(&format!("{}{}", line(""), line("")), None, None));
    assert!((with[0] - without[0]).abs() < 0.05, "{with:?} {without:?}");
}

/// A struck space inside a line carries the strike; one ending the line does not.
#[test]
fn struck_spaces_inside_a_line_are_struck() {
    let strikes = |body: &str| {
        pages(docx(body, None, None))[0]
            .items
            .iter()
            .filter(|i| matches!(i, crate::office::pptx::scene::Item::Path(_)))
            .count()
    };
    let space = r#"<w:r><w:rPr><w:strike/></w:rPr><w:t xml:space="preserve"> </w:t></w:r>"#;
    let inside = format!(r#"<w:p><w:r><w:t>a</w:t></w:r>{space}<w:r><w:t>b</w:t></w:r></w:p>"#);
    let trailing = format!(r#"<w:p><w:r><w:t>a</w:t></w:r>{space}</w:p>"#);
    assert_eq!(strikes(&inside), 1);
    assert_eq!(strikes(&trailing), 0);
}

#[test]
fn character_grid_context_follows_each_section() {
    let first = r#"<w:p><w:pPr><w:sectPr><w:docGrid w:type="linesAndChars" w:charSpace="4096"/></w:sectPr></w:pPr><w:r><w:t>日本語</w:t></w:r></w:p>"#;
    let second = r#"<w:p><w:r><w:t>日本語</w:t></w:r></w:p><w:sectPr><w:docGrid w:type="linesAndChars" w:charSpace="-2048"/></w:sectPr>"#;
    let out = pages(docx(&format!("{first}{second}"), None, None));
    assert_eq!(out.len(), 2);
    let width = |p: &crate::office::docx::paginate::OutPage| {
        p.items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) => Some(r.width),
                _ => None,
            })
            .sum::<f64>()
    };
    let delta = width(&out[0]) - width(&out[1]);
    let expected = 4.5;
    assert!(
        (delta - expected).abs() < 0.001,
        "section advance difference {delta}"
    );
}

#[test]
fn character_tracking_is_cluster_based_and_respects_run_exemptions() {
    use super::*;
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], true, &Default::default());
    let env = TextEnv {
        fonts: &fonts,
        default_tab: 36.0,
        fields: Default::default(),
        grid: None,
        char_grid: std::cell::Cell::new(Some(0.5)),
        expand_shift_return: false,
        mark_in_last_line: true,
        html_spacing: true,
        grow_autofit: false,
        in_cell: Default::default(),
        in_header: Default::default(),
        grid_in_table: false,
        no_leading: false,
        compat_mode: 15,
    };
    let opened = crate::office::docx::extract::open(
        docx(
            "<w:p><w:r><w:rPr><w:rFonts w:hint=\"eastAsia\"/></w:rPr><w:t>字</w:t></w:r></w:p>",
            None,
            None,
        ),
        &Default::default(),
    )
    .unwrap();
    let crate::office::docx::model::Block::Para(p) = &opened.sections[0].blocks[0] else {
        panic!("paragraph")
    };
    let crate::office::docx::model::Inline::Text(_, style) = &p.inlines[0] else {
        panic!("text")
    };
    for text in ["字", "か\u{3099}", "※", "□", "A", "1"] {
        assert_eq!(character_tracking(text, style, &env, 9.0), 0.5);
    }
    for text in ["ｶ", "\u{0301}", "\u{3099}", "\u{200B}"] {
        assert_eq!(character_tracking(text, style, &env, 9.0), 0.0);
    }
    assert_eq!(character_tracking("字", style, &env, 0.0), 0.0);
    env.char_grid.set(Some(-0.5));
    assert_eq!(character_tracking("字", style, &env, 9.0), -0.5);
    let mut exempt = (**style).clone();
    exempt.snap_to_grid = false;
    assert_eq!(character_tracking("字", &exempt, &env, 9.0), 0.0);
    assert!(
        same_format(&exempt, style),
        "grid exemption is already in atom positions"
    );
}

#[test]
fn grid_exemptions_do_not_fragment_drawn_text_runs() {
    let body = r#"<w:p><w:r><w:t xml:space="preserve">alpha </w:t></w:r><w:r><w:rPr><w:snapToGrid w:val="0"/></w:rPr><w:t>beta</w:t></w:r><w:r><w:t xml:space="preserve"> gamma</w:t></w:r></w:p>"#;
    let out = pages(docx(body, None, None));
    let runs: Vec<_> = out[0]
        .items
        .iter()
        .filter_map(|i| match i {
            crate::office::pptx::scene::Item::Text(r) => Some(r.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(runs, vec!["alpha beta gamma"]);
}

#[test]
fn continuation_source_boundaries_include_nontext_and_empty_break_lines() {
    use super::*;
    let opened = crate::office::docx::extract::open(
        docx("<w:p><w:r><w:t>same</w:t></w:r></w:p>", None, None),
        &Default::default(),
    )
    .unwrap();
    let crate::office::docx::model::Block::Para(mut p) = opened.sections[0].blocks[0].clone()
    else {
        panic!("paragraph")
    };
    p.inlines = vec![
        Inline::Text("same".into(), p.mark.clone()),
        Inline::Note(7),
        Inline::Break(BreakKind::Column, p.mark.clone()),
        Inline::Text("same".into(), p.mark.clone()),
        Inline::Break(BreakKind::Line, p.mark.clone()),
        Inline::Break(BreakKind::Line, p.mark.clone()),
    ];
    let fonts = FontSystem::new(&[], true, &Default::default());
    let env = TextEnv {
        fonts: &fonts,
        default_tab: 36.0,
        fields: Default::default(),
        grid: None,
        char_grid: Default::default(),
        expand_shift_return: false,
        mark_in_last_line: true,
        html_spacing: true,
        grow_autofit: false,
        in_cell: Default::default(),
        in_header: Default::default(),
        grid_in_table: false,
        no_leading: false,
        compat_mode: 15,
    };
    let old = layout_paragraph(&p, 80.0, &env);
    assert_eq!(old[0].replay.unwrap().source_end, 8);
    assert_eq!(old[0].notes, vec![7]);
    let next = layout_paragraph_resuming(&p, 140.0, &env, &old[..1], &WrapIn::default());
    let ends = next
        .iter()
        .map(|l| l.replay.unwrap().source_end)
        .collect::<Vec<_>>();
    assert!(ends.windows(2).all(|p| p[0] <= p[1]));
    assert_eq!(ends.last(), Some(&14));
    assert_eq!(
        next[0].replay.unwrap().source_end,
        old[0].replay.unwrap().source_end
    );
    assert_eq!(
        next.iter()
            .flat_map(|l| &l.notes)
            .copied()
            .collect::<Vec<_>>(),
        vec![7]
    );
}

#[test]
fn hyphen_provenance_maps_transformed_glyphs_and_synthetic_suffix() {
    use super::hyphen_source_range;
    // Dotless-i becomes one drawn ASCII byte while its source occupies two bytes.
    let ranges = vec![(0..1, 0..2), (1..2, 2..3), (2..3, 3..4)];
    assert_eq!(hyphen_source_range(&ranges, 0, 1, 2, 4), 0..2);
    assert_eq!(hyphen_source_range(&ranges, 1, 2, 2, 4), 2..3);
    assert_eq!(hyphen_source_range(&ranges, 2, 3, 2, 4), 3..3);
    assert_eq!(hyphen_source_range(&ranges, 2, 3, 3, 4), 3..4);
    let expanded = vec![(0..2, 0..1), (2..3, 1..2)];
    assert_eq!(hyphen_source_range(&expanded, 0, 2, 2, 2), 0..1);
    assert_eq!(hyphen_source_range(&expanded, 2, 3, 2, 2), 1..1);
}

fn tab_metric_after_y(
    mode: &str,
    tab_size: u32,
    mark_size: u32,
    text_size: u32,
    decoration: &str,
    leader: bool,
) -> f32 {
    let ppr = format!(
        r#"<w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/>{}<w:rPr><w:sz w:val="{mark_size}"/></w:rPr></w:pPr>"#,
        if leader {
            r#"<w:tabs><w:tab w:val="left" w:leader="dot" w:pos="720"/></w:tabs>"#
        } else {
            ""
        }
    );
    let tab =
        format!(r#"<w:r><w:rPr><w:sz w:val="{tab_size}"/>{decoration}</w:rPr><w:tab/></w:r>"#);
    let word =
        format!(r#"<w:r><w:rPr><w:sz w:val="{text_size}"/></w:rPr><w:t>VISIBLE</w:t></w:r>"#);
    let contents = match mode {
        "leading" => format!("{tab}{word}"),
        "trailing" => format!("{word}{tab}"),
        _ => tab,
    };
    let preceding = if mode == "middle" {
        "<w:p><w:r><w:t>PRECEDING</w:t></w:r></w:p>"
    } else {
        ""
    };
    let body =
        format!(r#"{preceding}<w:p>{ppr}{contents}</w:p><w:p><w:r><w:t>AFTER</w:t></w:r></w:p>"#);
    pages(docx(&body, None, None))[0]
        .items
        .iter()
        .find_map(|item| match item {
            crate::office::pptx::scene::Item::Text(run) if run.text == "AFTER" => {
                Some(run.transform.ty)
            }
            _ => None,
        })
        .unwrap()
}

#[test]
fn plain_tabs_use_surrounding_text_or_paragraph_mark_height() {
    for mode in ["blank", "middle", "leading", "trailing"] {
        let small = tab_metric_after_y(mode, 22, 22, 22, "", false);
        let large = tab_metric_after_y(mode, 32, 22, 22, "", false);
        assert!((small - large).abs() < 0.01, "{mode}: {small} vs {large}");
    }
}

#[test]
fn plain_tabs_preserve_larger_marks_and_visible_text() {
    let small = tab_metric_after_y("leading", 22, 22, 22, "", false);
    for (mode, mark, text) in [("blank", 40, 22), ("leading", 22, 40)] {
        let height = tab_metric_after_y(mode, 22, mark, text, "", false);
        assert!(height > small + 3.0, "{mode}: {height} vs {small}");
        let larger_tab = tab_metric_after_y(mode, 32, mark, text, "", false);
        assert!((height - larger_tab).abs() < 0.01);
    }
}

#[test]
fn leading_decorated_tabs_retain_run_metrics() {
    for (decoration, leader) in [
        (r#"<w:u w:val="single"/>"#, false),
        (r#"<w:highlight w:val="yellow"/>"#, false),
        (r#"<w:shd w:fill="FFFF00"/>"#, false),
        ("", true),
    ] {
        let small = tab_metric_after_y("leading", 22, 22, 22, decoration, leader);
        let large = tab_metric_after_y("leading", 32, 22, 22, decoration, leader);
        assert!(
            large > small + 3.0,
            "{decoration}/{leader}: {small} vs {large}"
        );
    }
}

#[test]
fn numbering_tab_suffix_retains_its_font_metrics() {
    let after = |size| {
        let numbering = format!(
            r#"<w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val=" "/><w:suff w:val="tab"/><w:rPr><w:sz w:val="{size}"/></w:rPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#
        );
        let body = r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr><w:spacing w:after="0"/><w:rPr><w:sz w:val="22"/></w:rPr></w:pPr><w:r><w:rPr><w:sz w:val="22"/></w:rPr><w:t>BODY</w:t></w:r></w:p><w:p><w:r><w:t>AFTER</w:t></w:r></w:p>"#;
        pages(docx(body, None, Some(&numbering)))[0]
            .items
            .iter()
            .find_map(|item| match item {
                crate::office::pptx::scene::Item::Text(run) if run.text == "AFTER" => {
                    Some(run.transform.ty)
                }
                _ => None,
            })
            .unwrap()
    };
    assert!(after(40) > after(22) + 5.0);
}

fn symbol_test_font(symbol_cmap: bool, code: u16, glyph: u16) -> Vec<u8> {
    symbol_metric_font(symbol_cmap, code, glyph, None)
}

fn symbol_metric_font(symbol_cmap: bool, code: u16, glyph: u16, typo: Option<bool>) -> Vec<u8> {
    let mut head = vec![0; 54];
    head[..4].copy_from_slice(&0x10000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5f0f3cf5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    let mut hhea = vec![0; 36];
    hhea[..4].copy_from_slice(&0x10000u32.to_be_bytes());
    hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
    hhea[34..36].copy_from_slice(&2u16.to_be_bytes());
    let maxp = vec![0, 0, 0x50, 0, 0, 2];
    let hmtx = vec![1, 244, 0, 0, 2, 238, 0, 0];
    let mut cmap = Vec::new();
    for x in [0u16, 1, 3, u16::from(!symbol_cmap)] {
        cmap.extend(x.to_be_bytes());
    }
    cmap.extend(12u32.to_be_bytes());
    for x in [
        4u16,
        32,
        0,
        4,
        4,
        1,
        0,
        code,
        0xffff,
        0,
        code,
        0xffff,
        glyph.wrapping_sub(code),
        1,
        0,
        0,
    ] {
        cmap.extend(x.to_be_bytes());
    }
    if typo.is_some() {
        hhea[8..10].copy_from_slice(&500i16.to_be_bytes());
    }
    let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"head", head),
        (b"hhea", hhea),
        (b"maxp", maxp),
        (b"hmtx", hmtx),
        (b"cmap", cmap),
    ];
    if let Some(typo) = typo {
        let mut os2 = vec![0; 96];
        for (off, value) in [
            (0, 4u16),
            (4, 400),
            (6, 5),
            (62, if typo { 128 } else { 0 }),
            (68, 600),
            (70, (-100i16) as u16),
            (72, 50),
            (74, 900),
            (76, 300),
        ] {
            os2[off..off + 2].copy_from_slice(&value.to_be_bytes());
        }
        tables.push((b"OS/2", os2));
    }
    let mut out = vec![0, 1, 0, 0, 0, tables.len() as u8, 0, 64, 0, 2, 0, 16];
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in &tables {
        out.extend(*tag);
        out.extend([0; 4]);
        out.extend((offset as u32).to_be_bytes());
        out.extend((data.len() as u32).to_be_bytes());
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in tables {
        out.extend(data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    out
}

#[test]
fn explicit_symbol_uses_source_cmap_and_retains_accessible_text() {
    use super::*;
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], false, &Default::default());
    assert!(fonts.add_embedded("Webdings", false, false, symbol_test_font(true, 0xf063, 1)));
    let env = TextEnv {
        fonts: &fonts,
        default_tab: 36.0,
        fields: Default::default(),
        grid: None,
        char_grid: std::cell::Cell::new(None),
        expand_shift_return: false,
        mark_in_last_line: true,
        html_spacing: true,
        grow_autofit: false,
        in_cell: Default::default(),
        in_header: Default::default(),
        grid_in_table: false,
        no_leading: false,
        compat_mode: 15,
    };
    let opened = crate::office::docx::extract::open(docx(
        r#"<w:p><w:r><w:rPr><w:rFonts w:ascii="Conflicting Font"/><w:sz w:val="20"/></w:rPr><w:sym w:font="Webdings" w:char="F063"/></w:r></w:p>"#,
        None, None), &Default::default()).unwrap();
    let crate::office::docx::model::Block::Para(p) = &opened.sections[0].blocks[0] else {
        panic!("paragraph")
    };
    let Inline::Text(text, style) = &p.inlines[0] else {
        panic!("symbol")
    };
    let (mut atoms, mut full, mut offsets) = (Vec::new(), String::new(), Vec::new());
    shape_text(text, style, &env, &mut atoms, &mut full, &mut offsets);
    assert_eq!(full, "⬜");
    assert_eq!(atoms.len(), 1);
    assert_eq!(atoms[0].text, "⬜");
    assert_eq!(atoms[0].face(), fonts.resolve("Webdings", false, false));
    assert!((atoms[0].width - 7.5).abs() < 0.001);
    assert_eq!(offsets, [0]);
    let mut low = (**style).clone();
    low.symbol = Some(("Webdings".into(), 'c'));
    assert!(
        shape_source_symbol(
            text,
            &Rc::new(low),
            &env,
            &mut Vec::new(),
            &mut String::new(),
            &mut Vec::new()
        )
        .is_some()
    );
}

#[test]
fn explicit_symbol_rejects_missing_glyph_and_unrelated_unicode_face() {
    use super::*;
    let opened = crate::office::docx::extract::open(
        docx(&crate::office::docx::testutil::para("x"), None, None),
        &Default::default(),
    )
    .unwrap();
    let crate::office::docx::model::Block::Para(p) = &opened.sections[0].blocks[0] else {
        panic!("paragraph")
    };
    for (family, symbol_cmap, code, glyph, source_code) in [
        ("Webdings", true, 0xf064, 1, '\u{f063}'),
        ("Webdings", true, 0xf063, 0, '\u{f063}'),
        ("Arial", false, 0x63, 1, 'c'),
        ("Webdings", false, 0x63, 1, '\u{f063}'),
    ] {
        let fonts = crate::office::pptx::fonts::FontSystem::new(&[], false, &Default::default());
        assert!(fonts.add_embedded(
            family,
            false,
            false,
            symbol_test_font(symbol_cmap, code, glyph)
        ));
        let env = TextEnv {
            fonts: &fonts,
            default_tab: 36.0,
            fields: Default::default(),
            grid: None,
            char_grid: std::cell::Cell::new(None),
            expand_shift_return: false,
            mark_in_last_line: true,
            html_spacing: true,
            grow_autofit: false,
            in_cell: Default::default(),
            in_header: Default::default(),
            grid_in_table: false,
            no_leading: false,
            compat_mode: 15,
        };
        let mut style = (*p.mark).clone();
        style.symbol = Some(("Webdings".into(), source_code));
        let (mut atoms, mut full, mut offsets) = (Vec::new(), String::new(), Vec::new());
        assert!(
            shape_source_symbol(
                "⬜",
                &Rc::new(style),
                &env,
                &mut atoms,
                &mut full,
                &mut offsets
            )
            .is_none()
        );
        assert!(atoms.is_empty() && full.is_empty() && offsets.is_empty());
    }
}

#[test]
fn explicit_symbols_keep_raw_codes_and_independent_occurrences_under_caps_and_rtl() {
    use super::*;
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], false, &Default::default());
    assert!(fonts.add_embedded("Symbol", false, false, symbol_test_font(true, 0xf061, 1)));
    let env = TextEnv {
        fonts: &fonts,
        default_tab: 36.0,
        fields: Default::default(),
        grid: None,
        char_grid: std::cell::Cell::new(None),
        expand_shift_return: false,
        mark_in_last_line: true,
        html_spacing: true,
        grow_autofit: false,
        in_cell: Default::default(),
        in_header: Default::default(),
        grid_in_table: false,
        no_leading: false,
        compat_mode: 15,
    };
    let opened = crate::office::docx::extract::open(docx(
        r#"<w:p><w:pPr><w:bidi/></w:pPr><w:r><w:rPr><w:caps/><w:smallCaps/><w:rtl/><w:sz w:val="20"/></w:rPr><w:t>left</w:t><w:sym w:font="Symbol" w:char="F061"/><w:sym w:font="Symbol" w:char="F061"/><w:t>right</w:t></w:r></w:p>"#,
        None, None), &Default::default()).unwrap();
    let crate::office::docx::model::Block::Para(p) = &opened.sections[0].blocks[0] else {
        panic!("paragraph")
    };
    let symbols: Vec<_> = p
        .inlines
        .iter()
        .filter_map(|i| match i {
            Inline::Text(t, s) if s.symbol.is_some() => Some((t, s)),
            _ => None,
        })
        .collect();
    assert_eq!(symbols.len(), 2);
    assert!(
        symbols
            .iter()
            .all(|(t, s)| *t == "α" && s.caps && s.small_caps)
    );
    let lines = layout_paragraph(p, 25.0, &env);
    let text: String = lines
        .iter()
        .flat_map(|l| l.items.iter())
        .filter_map(|i| match i {
            Item::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text.matches('α').count(), 2);
    assert!(
        lines
            .windows(2)
            .all(|p| p[0].replay.unwrap().source_end <= p[1].replay.unwrap().source_end)
    );
    let (_, s) = symbols[0];
    assert!(
        shape_source_symbol(
            "αα",
            s,
            &env,
            &mut Vec::new(),
            &mut String::new(),
            &mut Vec::new()
        )
        .is_none()
    );
}

#[test]
fn explicit_symbol_source_metrics_honor_font_flags_and_vertical_styles() {
    use super::*;
    let opened = crate::office::docx::extract::open(
        docx(&crate::office::docx::testutil::para("x"), None, None),
        &Default::default(),
    )
    .unwrap();
    let crate::office::docx::model::Block::Para(p) = &opened.sections[0].blocks[0] else {
        panic!("paragraph")
    };
    for (typo, expected) in [(true, (6.5, 1.0)), (false, (12.0, 3.0))] {
        let fonts = crate::office::pptx::fonts::FontSystem::new(&[], false, &Default::default());
        assert!(fonts.add_embedded(
            "Webdings",
            false,
            false,
            symbol_metric_font(true, 0xf063, 1, Some(typo))
        ));
        let env = TextEnv {
            fonts: &fonts,
            default_tab: 36.0,
            fields: Default::default(),
            grid: None,
            char_grid: std::cell::Cell::new(None),
            expand_shift_return: false,
            mark_in_last_line: true,
            html_spacing: true,
            grow_autofit: false,
            in_cell: Default::default(),
            in_header: Default::default(),
            grid_in_table: false,
            no_leading: false,
            compat_mode: 15,
        };
        for (align, position) in [
            (VertAlign::Baseline, 0.0),
            (VertAlign::Super, 0.0),
            (VertAlign::Sub, 0.0),
            (VertAlign::Baseline, 3.0),
            (VertAlign::Baseline, -3.0),
        ] {
            let mut style = (*p.mark).clone();
            style.symbol = Some(("Webdings".into(), '\u{f063}'));
            style.size = 10.0;
            style.vert_align = align;
            style.position = position;
            let (size, rise) = draw_size(&style);
            let scaled = (expected.0 * size / 10.0, expected.1 * size / 10.0);
            let mut want = ((scaled.0 + rise).max(0.0), (scaled.1 - rise).max(0.0));
            if align == VertAlign::Baseline {
                want = (want.0.max(expected.0), want.1.max(expected.1));
            }
            let mut atoms = Vec::new();
            shape_source_symbol(
                "⬜",
                &Rc::new(style),
                &env,
                &mut atoms,
                &mut String::new(),
                &mut Vec::new(),
            )
            .unwrap();
            assert!(
                (atoms[0].ascent - want.0).abs() < 0.001
                    && (atoms[0].descent - want.1).abs() < 0.001,
                "typo={typo}, {align:?}, position={position}: actual {:?}, expected {want:?}",
                (atoms[0].ascent, atoms[0].descent)
            );
        }
    }
}

#[test]
fn explicit_symbol_zero_direct_mapping_uses_valid_symbol_alias() {
    use super::*;
    let mut data = symbol_test_font(true, 0xf063, 1);
    let entry = (0..5)
        .map(|i| 12 + 16 * i)
        .find(|&i| &data[i..i + 4] == b"cmap")
        .unwrap();
    let offset = u32::from_be_bytes(data[entry + 8..entry + 12].try_into().unwrap()) as usize;
    let mut cmap = vec![0, 0, 0, 1, 0, 3, 0, 0, 0, 0, 0, 12];
    for x in [
        4u16,
        40,
        0,
        6,
        4,
        1,
        2,
        0x63,
        0xf063,
        0xffff,
        0,
        0x63,
        0xf063,
        0xffff,
        0u16.wrapping_sub(0x63),
        1u16.wrapping_sub(0xf063),
        1,
        0,
        0,
        0,
    ] {
        cmap.extend(x.to_be_bytes());
    }
    data.truncate(offset);
    data.extend(&cmap);
    data[entry + 12..entry + 16].copy_from_slice(&(cmap.len() as u32).to_be_bytes());
    let parsed = rustybuzz::ttf_parser::Face::parse(&data, 0).unwrap();
    let sub = parsed
        .tables()
        .cmap
        .unwrap()
        .subtables
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(
        sub.glyph_index(0x63),
        Some(rustybuzz::ttf_parser::GlyphId(0))
    );
    assert_eq!(
        sub.glyph_index(0xf063),
        Some(rustybuzz::ttf_parser::GlyphId(1))
    );
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], false, &Default::default());
    assert!(fonts.add_embedded("Webdings", false, false, data));
    let env = TextEnv {
        fonts: &fonts,
        default_tab: 36.0,
        fields: Default::default(),
        grid: None,
        char_grid: std::cell::Cell::new(None),
        expand_shift_return: false,
        mark_in_last_line: true,
        html_spacing: true,
        grow_autofit: false,
        in_cell: Default::default(),
        in_header: Default::default(),
        grid_in_table: false,
        no_leading: false,
        compat_mode: 15,
    };
    let opened = crate::office::docx::extract::open(
        docx(&crate::office::docx::testutil::para("x"), None, None),
        &Default::default(),
    )
    .unwrap();
    let crate::office::docx::model::Block::Para(p) = &opened.sections[0].blocks[0] else {
        panic!("paragraph")
    };
    let mut style = (*p.mark).clone();
    style.symbol = Some(("Webdings".into(), 'c'));
    assert!(
        shape_source_symbol(
            "⬜",
            &Rc::new(style),
            &env,
            &mut Vec::new(),
            &mut String::new(),
            &mut Vec::new()
        )
        .is_some()
    );
}

#[test]
fn explicit_symbol_provenance_preserves_space_and_nbsp_in_raw_export() {
    check_symbol_raw_export(false);
}

#[test]
fn explicit_symbol_different_source_face_preserves_raw_whitespace() {
    check_symbol_raw_export(true);
}

fn check_symbol_raw_export(source_symbol: bool) {
    use super::*;
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], false, &Default::default());
    assert!(fonts.add_embedded(
        "PrivateSymbol",
        false,
        false,
        symbol_test_font(false, 0x03b1, 1)
    ));
    if source_symbol {
        assert!(fonts.add_embedded("Symbol", false, false, symbol_test_font(true, 0xf061, 1)));
    }
    let env = TextEnv {
        fonts: &fonts,
        default_tab: 36.0,
        fields: Default::default(),
        grid: None,
        char_grid: std::cell::Cell::new(None),
        expand_shift_return: false,
        mark_in_last_line: true,
        html_spacing: true,
        grow_autofit: false,
        in_cell: Default::default(),
        in_header: Default::default(),
        grid_in_table: false,
        no_leading: false,
        compat_mode: 15,
    };
    let build = |content: &str, caps: bool| {
        let cap = if caps { "<w:caps/>" } else { "" };
        let body = format!(
            r#"<w:p><w:r><w:rPr><w:rFonts w:ascii="PrivateSymbol" w:hAnsi="PrivateSymbol"/>{cap}</w:rPr>{content}</w:r></w:p>"#
        );
        crate::office::docx::extract::open(docx(&body, None, None), &Default::default()).unwrap()
    };
    let export = |p: &Paragraph| {
        let lines = layout_paragraph(p, 400.0, &env);
        let scene = crate::office::pptx::scene::Scene {
            width: 400.0,
            height: 100.0,
            items: lines.into_iter().flat_map(|l| l.items).collect(),
            ..Default::default()
        };
        crate::office::pptx::output::text_json(&fonts, &scene, true, (0.0, 0.0), None)
    };
    for caps in [false, true] {
        let expected = if caps {
            "Α α \u{a0} Α"
        } else {
            "α α \u{a0} α"
        };
        let ordinary = build(
            &format!(r#"<w:t xml:space="preserve">{expected}</w:t>"#),
            false,
        );
        let crate::office::docx::model::Block::Para(reference) = &ordinary.sections[0].blocks[0]
        else {
            panic!("paragraph")
        };
        let reference = export(reference);
        for (family, code) in [("PrivateSymbol", "03B1"), ("Symbol", "F061")] {
            let opened = build(
                &format!(
                    r#"<w:t xml:space="preserve">α </w:t><w:sym w:font="{family}" w:char="{code}"/><w:t xml:space="preserve"> &#xA0; α</w:t>"#
                ),
                caps,
            );
            let crate::office::docx::model::Block::Para(p) = &opened.sections[0].blocks[0] else {
                panic!("paragraph")
            };
            let actual = export(p);
            let raw: String = actual["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| i["unicode"].as_str().unwrap())
                .collect();
            assert_eq!(raw, expected, "{family}");
            if !source_symbol || family == "PrivateSymbol" {
                assert_eq!(actual, reference, "source glyph/fallback geometry {family}");
            }
            assert_eq!(
                export(p),
                actual,
                "repeated shaping retains parsed provenance"
            );
            assert!(
                p.inlines
                    .iter()
                    .any(|i| matches!(i, Inline::Text(_, s) if s.symbol.is_some()))
            );
        }
    }
}

#[test]
fn interior_whitespace_between_anchor_slots_is_exported_once() {
    use super::*;
    let font = FontSystem::new(&[], true, &Default::default());
    let box_xml = |id| {
        format!(
            r#"<w:r><w:pict><v:shape xmlns:v="urn:schemas-microsoft-com:vml" id="b{id}" style="position:absolute;left:40pt;top:40pt;width:70pt;height:20pt"><v:textbox><w:txbxContent><w:p><w:r><w:t>BOX{id}</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></w:r>"#
        )
    };
    for (gap, outer, empty_box) in [
        ("  ", true, 0),
        ("\u{a0}\u{a0}", true, 0),
        (" \u{a0} ", true, 0),
        (" \u{a0} ", false, 0),
        ("  ", false, 1),
        ("  ", false, 2),
    ] {
        let (before, after) = if outer { ("BEFORE", "AFTER") } else { ("", "") };
        let body = format!(
            r#"<w:p><w:r><w:t>{before}</w:t></w:r>{}<w:r><w:t xml:space="preserve">{gap}</w:t></w:r>{}<w:r><w:t>{after}</w:t></w:r></w:p>"#,
            box_xml(0),
            box_xml(1)
        );
        let body = if empty_box == 2 {
            let picture = r#"<w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="127000" cy="127000"/></wp:inline></w:drawing></w:r>"#;
            body.replace(&box_xml(0), picture)
                .replace(&box_xml(1), picture)
        } else if empty_box == 1 {
            body.replace("BOX0", "").replace("BOX1", "")
        } else {
            body
        };
        let doc = crate::office::docx::extract::open(docx(&body, None, None), &Default::default())
            .unwrap();
        let laid = crate::office::docx::extract::layout(&doc, &font);
        let scene = crate::office::pptx::scene::Scene {
            width: 612.0,
            height: 792.0,
            items: laid.pages[0].items.clone(),
            ..Default::default()
        };
        let text = crate::office::pptx::output::text_json(&font, &scene, true, (0.0, 0.0), None);
        let spaces: Vec<_> = text["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["unicode"].as_str())
            .filter(|text| !text.is_empty() && text.trim().is_empty())
            .collect();
        assert_eq!(spaces, [gap]);
        let retained: Vec<_> = scene
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Text(run) if run.preserve_whitespace => Some(run),
                _ => None,
            })
            .collect();
        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].text, gap);
        for (value, preserve) in [(gap, false), ("", true)] {
            let mut run = retained[0].clone();
            run.text = value.into();
            run.preserve_whitespace = preserve;
            let isolated = crate::office::pptx::scene::Scene {
                width: scene.width,
                height: scene.height,
                items: vec![Item::Text(run)],
                ..Default::default()
            };
            let output =
                crate::office::pptx::output::text_json(&font, &isolated, true, (0.0, 0.0), None);
            assert!(output["items"].as_array().unwrap().is_empty());
        }
    }
}

#[test]
fn east_asian_lines_take_1_3_em_off_grid_and_snap_that_height_on_grid() {
    let pitch = |size: u32, grid: &str| {
        let para = |t: &str| {
            format!(
                r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/><w:rPr><w:rFonts w:ascii="SimSun" w:hAnsi="SimSun" w:eastAsia="SimSun"/><w:sz w:val="{size}"/></w:rPr></w:pPr><w:r><w:rPr><w:rFonts w:ascii="SimSun" w:hAnsi="SimSun" w:eastAsia="SimSun"/><w:sz w:val="{size}"/></w:rPr><w:t>{t}</w:t></w:r></w:p>"#
            )
        };
        let body = format!(
            r#"{}{}<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>{grid}</w:sectPr>"#,
            para("一"),
            para("二")
        );
        let ys: Vec<f32> = pages(docx(&body, None, None))[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) => Some(r.transform.ty),
                _ => None,
            })
            .collect();
        ys[1] - ys[0]
    };
    let grid = r#"<w:docGrid w:type="lines" w:linePitch="312"/>"#;
    // 13 pt SimSun: 1.3 em = 16.9 pt without a grid.
    assert!((pitch(26, "") - 16.9).abs() < 0.05, "{}", pitch(26, ""));
    // Word for Mac on a 15.6 pt grid: 16.9 pt needs two pitches, 12 pt (15.6) one.
    assert!((pitch(26, grid) - 31.2).abs() < 0.05, "{}", pitch(26, grid));
    assert!((pitch(24, grid) - 15.6).abs() < 0.05, "{}", pitch(24, grid));
}

#[test]
fn at_least_zero_spacing_keeps_the_natural_line_off_the_grid() {
    let para = |t: &str| {
        format!(
            r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="0" w:lineRule="atLeast"/><w:rPr><w:rFonts w:ascii="SimSun" w:hAnsi="SimSun" w:eastAsia="SimSun"/><w:sz w:val="21"/></w:rPr></w:pPr><w:r><w:rPr><w:rFonts w:ascii="SimSun" w:hAnsi="SimSun" w:eastAsia="SimSun"/><w:sz w:val="21"/></w:rPr><w:t>{t}</w:t></w:r></w:p>"#
        )
    };
    let body = format!(
        r#"{}{}<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/><w:docGrid w:type="lines" w:linePitch="360"/></w:sectPr>"#,
        para("一"),
        para("二")
    );
    let ys: Vec<f32> = pages(docx(&body, None, None))[0]
        .items
        .iter()
        .filter_map(|i| match i {
            crate::office::pptx::scene::Item::Text(r) => Some(r.transform.ty),
            _ => None,
        })
        .collect();
    // 10.5 pt at 1.3 em is 13.65 pt; an 18 pt grid pitch does not apply.
    assert!((ys[1] - ys[0] - 13.65).abs() < 0.05, "{ys:?}");
}

#[test]
fn a_line_multiple_on_a_grid_is_a_floor_in_pitches_not_a_factor() {
    let pitch = |size: u32, line: u32| {
        let rpr = format!(
            r#"<w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman"/><w:sz w:val="{size}"/></w:rPr>"#
        );
        let para = |t: &str| {
            format!(
                r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="{line}" w:lineRule="auto"/>{rpr}</w:pPr><w:r>{rpr}<w:t>{t}</w:t></w:r></w:p>"#
            )
        };
        let body = format!(
            r#"{}{}<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/><w:docGrid w:type="lines" w:linePitch="312"/></w:sectPr>"#,
            para("one"),
            para("two")
        );
        let ys: Vec<f32> = pages(docx(&body, None, None))[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) => Some(r.transform.ty),
                _ => None,
            })
            .collect();
        ys[1] - ys[0]
    };
    // 10.5 pt needs one pitch: 1.5 lines makes it 1.5 pitches.
    assert!((pitch(21, 360) - 23.4).abs() < 0.05, "{}", pitch(21, 360));
    // 14 pt already needs two pitches: 1.5 lines does not add a third.
    assert!((pitch(28, 360) - 31.2).abs() < 0.05, "{}", pitch(28, 360));
}

#[test]
fn an_installed_office_font_keeps_its_word_line_height() {
    let height = |installed: bool| {
        let fonts = crate::office::pptx::fonts::FontSystem::new(&[], false, &Default::default());
        // A face registered as "Garamond" (a Word-table family) has other metrics.
        if installed {
            assert!(fonts.add_embedded("Garamond", false, false, symbol_test_font(false, 0x41, 1)));
        }
        let face = fonts.resolve("Garamond", false, false);
        let (a, d) = crate::office::docx::metrics::line_metrics_styled(
            &fonts, "Garamond", face, 12.0, false, false,
        );
        a + d
    };
    // Without the font the Word table sets the height; installing it changes nothing.
    assert!(
        (height(true) - height(false)).abs() < 1e-6,
        "{} vs {}",
        height(true),
        height(false)
    );
}

#[test]
fn a_larger_paragraph_mark_does_not_size_a_line_with_text() {
    // Text baselines of three paragraphs: 10 pt text, then an empty paragraph.
    let ys = |mark: u32| {
        let para = |t: &str| {
            format!(
                r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/><w:rPr><w:sz w:val="{mark}"/></w:rPr></w:pPr><w:r><w:rPr><w:sz w:val="20"/></w:rPr><w:t>{t}</w:t></w:r></w:p>"#
            )
        };
        let body = format!("{}{}{}{}", para("one"), para("two"), para(""), para("end"));
        let ys: Vec<f32> = pages(docx(&body, None, None))[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) => Some(r.transform.ty),
                _ => None,
            })
            .collect();
        (ys[1] - ys[0], ys[2] - ys[1])
    };
    let (same_text, same_empty) = ys(20);
    let (big_text, big_empty) = ys(40);
    // Word for Mac: a 20 pt mark leaves 10 pt text lines alone (modes 11-15)...
    assert!(
        (big_text - same_text).abs() < 0.01,
        "{big_text} {same_text}"
    );
    // ...but still sizes an empty paragraph.
    assert!(big_empty > same_empty + 5.0, "{big_empty} {same_empty}");
}

#[test]
fn a_larger_list_label_adds_only_its_extra_ascent_to_the_line() {
    // Baseline pitch of 10 pt Arial items at 1.5 lines under an Arial label of `label` half-points.
    let pitch = |label: u32| {
        let numbering = format!(
            r#"<w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial"/><w:sz w:val="{label}"/></w:rPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#
        );
        let rpr = r#"<w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial"/><w:sz w:val="20"/></w:rPr>"#;
        let item = format!(
            r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr><w:spacing w:before="0" w:after="0" w:line="360" w:lineRule="auto"/>{rpr}</w:pPr><w:r>{rpr}<w:t>item</w:t></w:r></w:p>"#
        );
        let ys: Vec<f32> = pages(docx(&item.repeat(2), None, Some(&numbering)))[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text == "item" => {
                    Some(r.transform.ty)
                }
                _ => None,
            })
            .collect();
        ys[1] - ys[0]
    };
    // Word for Mac: a 20 pt label adds 10 pt of Arial ascent (with line gap), 9.38 pt,
    // unscaled by the 1.5 multiple and without its larger descent.
    let extra = pitch(40) - pitch(20);
    assert!((extra - 9.38).abs() < 0.15, "{extra}");
}

#[test]
fn a_western_run_beside_east_asian_text_keeps_the_east_asian_line_height() {
    let pitch = |western: bool| {
        let rpr = |font: &str| {
            format!(
                r#"<w:rPr><w:rFonts w:ascii="{font}" w:hAnsi="{font}" w:eastAsia="SimSun"/><w:sz w:val="26"/></w:rPr>"#
            )
        };
        let tail = if western {
            format!("<w:r>{}<w:t>x</w:t></w:r>", rpr("Times New Roman"))
        } else {
            String::new()
        };
        let para = |t: &str| {
            format!(
                r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/>{}</w:pPr><w:r>{}<w:t>{t}</w:t></w:r>{tail}</w:p>"#,
                rpr("SimSun"),
                rpr("SimSun")
            )
        };
        let body = format!("{}{}", para("一"), para("二"));
        let ys: Vec<f32> = pages(docx(&body, None, None))[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if !r.text.contains('x') => {
                    Some(r.transform.ty)
                }
                _ => None,
            })
            .collect();
        ys[1] - ys[0]
    };
    // Word for Mac (Century beside MS Mincho, 10.5-14 pt): the deeper Western descent
    // fits inside the 1.3 em East Asian line, so 13 pt SimSun stays 16.9 pt.
    assert!((pitch(false) - 16.9).abs() < 0.05, "{}", pitch(false));
    assert!((pitch(true) - 16.9).abs() < 0.05, "{}", pitch(true));
}

#[test]
fn no_leading_drops_the_fonts_external_leading_from_line_height() {
    let pitch = |settings: &str| {
        let rpr = r#"<w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman"/><w:sz w:val="24"/></w:rPr>"#;
        let para = |t: &str| {
            format!(
                r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/>{rpr}</w:pPr><w:r>{rpr}<w:t>{t}</w:t></w:r></w:p>"#
            )
        };
        let body = format!("{}{}", para("one"), para("two"));
        let bytes = crate::office::docx::testutil::with_related_part(
            docx(&body, None, None),
            "settings",
            "settings.xml",
            &format!(
                r#"<w:settings xmlns:w="{}"><w:compat>{settings}<w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="11"/></w:compat></w:settings>"#,
                crate::office::docx::testutil::W
            ),
            false,
        );
        let ys: Vec<f32> = pages(bytes)[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) => Some(r.transform.ty),
                _ => None,
            })
            .collect();
        ys[1] - ys[0]
    };
    // Times New Roman 12 pt: 13.8 pt with its 87-unit external leading, 13.29 without
    // (Word for Mac on a Word 97 file with fNoLeading: 13.2-13.44 pt alternating).
    assert!((pitch("") - 13.8).abs() < 0.02, "{}", pitch(""));
    assert!(
        (pitch("<w:noLeading/>") - 13.29).abs() < 0.02,
        "{}",
        pitch("<w:noLeading/>")
    );
}

#[test]
fn a_line_of_spaces_takes_the_paragraph_marks_height() {
    let gap = |middle: &str| {
        let rpr = r#"<w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman"/><w:sz w:val="24"/></w:rPr>"#;
        let ppr = format!(
            r#"<w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/>{rpr}</w:pPr>"#
        );
        let body = format!(
            r#"<w:p>{ppr}<w:r>{rpr}<w:t>Top</w:t></w:r></w:p><w:p>{ppr}{middle}</w:p><w:p>{ppr}<w:r>{rpr}<w:t>Bottom</w:t></w:r></w:p>"#
        );
        let ys: Vec<f32> = pages(docx(&body, None, None))[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text.trim() != "" => {
                    Some(r.transform.ty)
                }
                _ => None,
            })
            .collect();
        ys[ys.len() - 1] - ys[0]
    };
    // Word for Mac: a 42 pt space leaves the line at its 12 pt mark's height.
    let big_space =
        r#"<w:r><w:rPr><w:sz w:val="84"/></w:rPr><w:t xml:space="preserve"> </w:t></w:r>"#;
    assert!(
        (gap(big_space) - gap("")).abs() < 0.01,
        "{} vs {}",
        gap(big_space),
        gap("")
    );
}

#[test]
fn a_line_break_sizes_only_a_line_without_text() {
    // The gap from "Top" to "Bottom" across one paragraph: Top, a break, then
    // `middle` and a 24 pt break.
    let gap = |middle: &str| {
        let rpr = r#"<w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman"/><w:sz w:val="20"/></w:rPr>"#;
        let big = r#"<w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman"/><w:sz w:val="48"/></w:rPr>"#;
        let body = format!(
            r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/>{rpr}</w:pPr><w:r>{rpr}<w:t>Top</w:t><w:br/></w:r><w:r>{rpr}<w:t xml:space="preserve">{middle}</w:t></w:r><w:r>{big}<w:br/></w:r><w:r>{rpr}<w:t>Bottom</w:t></w:r></w:p>"#
        );
        let ys: Vec<f32> = pages(docx(&body, None, None))[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r)
                    if ["Top", "Bottom"].contains(&r.text.trim()) =>
                {
                    Some(r.transform.ty)
                }
                _ => None,
            })
            .collect();
        ys[1] - ys[0]
    };
    // Word for Mac: after "A" the 24 pt break leaves the line 10 pt; after a lone
    // space it makes the line 24 pt.
    let (text, space) = (gap("A"), gap(" "));
    assert!(space - text > 10.0, "text {text} vs space {space}");
    assert!(text < 2.0 * 12.0, "text line {text}");
}

#[test]
fn trailing_tabs_past_the_right_edge_wrap() {
    // Word for Mac: twenty default tabs alone in a paragraph take two lines on an
    // A4 page with 1 inch margins, where no default stop falls on the 451 pt edge.
    const SECT: &str = r#"<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>"#;
    let gap = |middle: &str| {
        let ppr = r#"<w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/></w:pPr>"#;
        let body = format!(
            r#"<w:p>{ppr}<w:r><w:t>Top</w:t></w:r></w:p><w:p>{ppr}<w:r>{middle}</w:r></w:p><w:p>{ppr}<w:r><w:t>Bottom</w:t></w:r></w:p>{SECT}"#
        );
        let ys: Vec<f32> = pages(docx(&body, None, None))[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r)
                    if ["Top", "Bottom"].contains(&r.text.trim()) =>
                {
                    Some(r.transform.ty)
                }
                _ => None,
            })
            .collect();
        ys[1] - ys[0]
    };
    // "Top" plus a one-line middle paragraph is two lines; plus the tabs, three.
    let line = gap("<w:t>x</w:t>") / 2.0;
    let tabs = gap(&"<w:tab/>".repeat(20));
    assert!((tabs - 3.0 * line).abs() < 0.5, "{tabs} vs line {line}");
}

#[test]
fn trailing_en_spaces_past_the_right_edge_wrap() {
    // Word for Mac: a form field of en spaces that does not fit wraps to the next
    // line; ordinary spaces hang past the edge and keep the paragraph on one line.
    let gap = |spaces: &str| {
        let ppr = r#"<w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/></w:pPr>"#;
        let body = format!(
            r#"<w:p>{ppr}<w:r><w:t>Top</w:t></w:r></w:p><w:p>{ppr}<w:r><w:t xml:space="preserve">x{spaces}</w:t></w:r></w:p><w:p>{ppr}<w:r><w:t>Bottom</w:t></w:r></w:p>"#
        );
        let ys: Vec<f32> = pages(docx(&body, None, None))[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r)
                    if ["Top", "Bottom"].contains(&r.text.trim()) =>
                {
                    Some(r.transform.ty)
                }
                _ => None,
            })
            .collect();
        ys[1] - ys[0]
    };
    let plain = gap(&" ".repeat(200));
    let en = gap(&"\u{2002}".repeat(200));
    assert!(en > plain * 1.4, "en {en} vs plain {plain}");
}

#[test]
fn raised_text_takes_its_descent_up_with_it() {
    // A 36 pt run raised 9 pt above a line closed by a 12 pt paragraph mark: the
    // line below the baseline is the mark's, not the big run's full descent.
    let para = |position: &str| {
        format!(
            r#"<w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:rPr><w:sz w:val="72"/>{position}</w:rPr><w:t>Big</w:t></w:r></w:p><w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:t>next</w:t></w:r></w:p>"#
        )
    };
    let gap = |position: &str| {
        let ys = line_ys(docx(&para(position), None, None));
        ys[1] - ys[0]
    };
    let raised = gap(r#"<w:position w:val="18"/>"#);
    let plain = gap("");
    // Keeping the run's full descent would add the whole 9 pt raise to the gap below
    // the raised baseline; Word adds only what the mark's descent needs.
    assert!(
        raised - plain > 0.0 && raised - plain < 5.0,
        "raised {raised} plain {plain}"
    );
}

#[test]
fn a_multiple_on_the_line_grid_centres_its_text_in_the_snapped_height() {
    // Word for Mac probe: 14 pt SimSun on a 15.6 pt grid takes two pitches; at three
    // lines it takes three, and its text sits half a pitch below a single line's.
    let run = r#"<w:rPr><w:rFonts w:ascii="SimSun" w:hAnsi="SimSun" w:eastAsia="SimSun"/><w:sz w:val="28"/></w:rPr>"#;
    let para = |spacing: &str| {
        format!(
            r#"<w:p><w:pPr><w:spacing w:after="0" {spacing}/></w:pPr><w:r>{run}<w:t>字</w:t></w:r></w:p>"#
        )
    };
    let body = format!(
        r#"{}{}{}<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/><w:docGrid w:type="lines" w:linePitch="312"/></w:sectPr>"#,
        para(""),
        para(r#"w:line="720" w:lineRule="auto""#),
        para("")
    );
    let ys = line_ys(docx(&body, None, None));
    assert_eq!(ys.len(), 3, "{ys:?}");
    assert!(
        ((ys[1] - ys[0]) as f64 - (31.2 + 7.8)).abs() < 0.3,
        "{ys:?}"
    );
    assert!(
        ((ys[2] - ys[0]) as f64 - (31.2 + 46.8)).abs() < 0.3,
        "{ys:?}"
    );
}

#[test]
fn a_line_final_comma_takes_half_its_width_under_compress_punctuation() {
    // 21 SimSun characters (210 pt) and a 10 pt comma on a 216 pt line: with
    // compressPunctuation the comma counts half and the line holds it; without, the
    // last character wraps with it.
    let body = format!(
        r#"<w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="SimSun" w:hAnsi="SimSun" w:eastAsia="SimSun"/><w:sz w:val="20"/></w:rPr><w:t>{}，</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="7200" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>"#,
        "字".repeat(21)
    );
    let compressed = crate::office::docx::testutil::with_settings(
        docx(&body, None, None),
        r#"<w:characterSpacingControl w:val="compressPunctuation"/>"#,
    );
    assert_eq!(line_ys(compressed).len(), 1);
    assert_eq!(line_ys(docx(&body, None, None)).len(), 2);
}

#[test]
fn chinese_punctuation_compresses_on_left_aligned_lines_and_japanese_does_not() {
    // 20 SimSun characters with a bracket pair on a 226 pt line: the pair's squeeze
    // (up to half an em) lets the last character fit on a left-aligned Chinese line;
    // a Japanese one compresses only when justified.
    let body = |lang: &str| {
        format!(
            r#"<w:p><w:pPr><w:jc w:val="left"/><w:spacing w:after="0"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="SimSun" w:hAnsi="SimSun" w:eastAsia="SimSun"/><w:sz w:val="20"/><w:lang w:eastAsia="{lang}"/></w:rPr><w:t>{}（字）字</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="7400" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>"#,
            "字".repeat(19)
        )
    };
    let lines = |lang: &str| {
        line_ys(crate::office::docx::testutil::with_settings(
            docx(&body(lang), None, None),
            r#"<w:characterSpacingControl w:val="compressPunctuation"/>"#,
        ))
        .len()
    };
    assert_eq!(lines("zh-CN"), 1);
    assert_eq!(lines("ja-JP"), 2);
}

#[test]
fn synthetic_bold_simsun_is_wider_than_regular() {
    // 22 SimSun characters at 10 pt (220 pt) on a 221 pt line: regular fits; bold
    // SimSun has no bold face, and Word's emboldening widens each glyph past the line.
    let body = |bold: &str| {
        format!(
            r#"<w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="SimSun" w:hAnsi="SimSun" w:eastAsia="SimSun"/>{bold}<w:sz w:val="20"/></w:rPr><w:t>{}</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="7300" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>"#,
            "字".repeat(22)
        )
    };
    assert_eq!(line_ys(docx(&body(""), None, None)).len(), 1);
    assert_eq!(line_ys(docx(&body("<w:b/>"), None, None)).len(), 2);
}

#[test]
fn synthetic_bold_widens_a_latin_family_without_a_bold_face() {
    // Ten 10 pt Lucida Console words "0000" (49 glyphs, 295.2 pt) on a 300 pt line:
    // regular fits; the family has no bold face, so bold adds 0.0194 em a glyph.
    let body = |bold: &str| {
        format!(
            r#"<w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="Lucida Console" w:hAnsi="Lucida Console"/>{bold}<w:sz w:val="20"/></w:rPr><w:t>{}</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="8880" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>"#,
            ["0000"; 10].join(" ")
        )
    };
    assert_eq!(line_ys(docx(&body(""), None, None)).len(), 1);
    assert_eq!(line_ys(docx(&body("<w:b/>"), None, None)).len(), 2);
}

#[test]
fn each_section_snaps_lines_to_its_own_grid() {
    // A first section on an 18 pt grid, then a continuous section on an 8.05 pt grid:
    // the second section's 12 pt lines take two of its own pitches (16.1 pt).
    let sect = |pitch: u32| {
        format!(
            r#"<w:type w:val="continuous"/><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/><w:docGrid w:type="lines" w:linePitch="{pitch}"/>"#
        )
    };
    let run = r#"<w:r><w:rPr><w:sz w:val="24"/></w:rPr><w:t>Line</w:t></w:r>"#;
    let body = format!(
        r#"<w:p><w:pPr><w:sectPr>{}</w:sectPr></w:pPr>{run}</w:p><w:p>{run}</w:p><w:p>{run}</w:p><w:sectPr>{}</w:sectPr>"#,
        sect(360),
        sect(161)
    );
    let ys = line_ys(docx(&body, None, None));
    assert_eq!(ys.len(), 3, "{ys:?}");
    assert!((ys[2] - ys[1] - 16.1).abs() < 0.1, "{ys:?}");
}

#[test]
fn a_none_border_keeps_its_spacing() {
    // Two paragraphs, the first with `none` top and bottom borders 9 pt from the
    // text: nothing is drawn, but the second line sits 9 pt lower than without.
    let body = |bdr: &str| {
        format!(
            r#"<w:p><w:pPr>{bdr}<w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr><w:r><w:t>One</w:t></w:r></w:p><w:p><w:pPr><w:spacing w:before="0" w:line="240" w:lineRule="auto"/></w:pPr><w:r><w:t>Two</w:t></w:r></w:p>"#
        )
    };
    let gap = |bdr: &str| {
        let ys = line_ys(docx(&body(bdr), None, None));
        ys[1] - ys[0]
    };
    let none = r#"<w:pBdr><w:top w:val="none" w:sz="0" w:space="9" w:color="auto"/><w:bottom w:val="none" w:sz="0" w:space="9" w:color="auto"/></w:pBdr>"#;
    assert!(
        (gap(none) - gap("") - 9.0).abs() < 0.1,
        "{} vs {}",
        gap(none),
        gap("")
    );
}

#[test]
fn hidden_empty_paragraphs_beside_a_floating_table_take_no_room() {
    // A narrow floating table, then three empty paragraphs with hidden marks: the
    // next line still runs beside the table, as with no hidden paragraphs between.
    let tbl = r#"<w:tbl><w:tblPr><w:tblpPr w:leftFromText="142" w:rightFromText="142" w:vertAnchor="text" w:horzAnchor="page" w:tblpX="9571" w:tblpY="188"/><w:tblW w:w="1498" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="1498"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>X</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    let hidden = r#"<w:p><w:pPr><w:rPr><w:vanish/></w:rPr></w:pPr></w:p>"#.repeat(3);
    let ys = |between: &str| {
        line_ys(docx(
            &format!(
                "<w:p><w:r><w:t>A</w:t></w:r></w:p>{tbl}{between}<w:p><w:r><w:t>B</w:t></w:r></w:p>"
            ),
            None,
            None,
        ))
    };
    assert_eq!(ys(&hidden), ys(""));
}

#[test]
fn spaces_between_small_capitals_are_small() {
    // "ab ab" in small caps: the space after a small capital is set at the small
    // size too, so the run is narrower than with a full-size space.
    let width = |text: &str| {
        let body = format!(
            r#"<w:p><w:r><w:rPr><w:smallCaps/><w:sz w:val="40"/></w:rPr><w:t xml:space="preserve">{text}</w:t></w:r></w:p>"#
        );
        let mut xs: Vec<f32> = pages(docx(&body, None, None))[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) => Some(r.transform.tx),
                _ => None,
            })
            .collect();
        xs.sort_by(f32::total_cmp);
        xs.last().copied().unwrap_or(0.0) - xs.first().copied().unwrap_or(0.0)
    };
    // The second "ab" starts after "ab" plus one space; full size would add 20 * 0.25.
    let small_space = width("ab ab") - width("abab");
    assert!(small_space < 20.0 * 0.25 - 0.5, "{small_space}");
}

#[test]
fn trailing_ideographic_spaces_keep_their_room_in_right_alignment() {
    // A right-aligned "AB" followed by two ideographic spaces (10 pt each) ends
    // 20 pt left of where a bare right-aligned "AB" ends.
    let right_end = |tail: &str| {
        let body = format!(
            r#"<w:p><w:pPr><w:jc w:val="right"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="SimSun" w:hAnsi="SimSun" w:eastAsia="SimSun"/><w:sz w:val="20"/></w:rPr><w:t xml:space="preserve">AB{tail}</w:t></w:r></w:p>"#
        );
        pages(docx(&body, None, None))[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) => Some(r.transform.tx),
                _ => None,
            })
            .fold(f32::NEG_INFINITY, f32::max)
    };
    let shift = right_end("") - right_end("\u{3000}\u{3000}");
    assert!((shift - 20.0).abs() < 0.5, "{shift}");
}

#[test]
fn a_bracket_pair_at_the_margin_squeezes_the_lines_punctuation() {
    // compressPunctuation on a 29.7 em line of "测测（测测）测测，": the 30th
    // character opens a bracket that cannot end a line, so Word squeezes the line's
    // punctuation to keep "（测" on it (31 characters) rather than break at 29.
    let text = "测测（测测）测测，".repeat(4);
    let body = format!(
        r#"<w:p><w:pPr><w:jc w:val="left"/><w:spacing w:after="0"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="SimSun" w:hAnsi="SimSun" w:eastAsia="SimSun"/><w:sz w:val="28"/></w:rPr><w:t>{text}</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="11192" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>"#
    );
    let bytes = crate::office::docx::testutil::with_settings(
        docx(&body, None, None),
        r#"<w:characterSpacingControl w:val="compressPunctuation"/>"#,
    );
    let runs: Vec<(f32, String)> = pages(bytes)[0]
        .items
        .iter()
        .filter_map(|i| match i {
            crate::office::pptx::scene::Item::Text(r) => Some((r.transform.ty, r.text.clone())),
            _ => None,
        })
        .collect();
    let top = runs.iter().map(|r| r.0).fold(f32::INFINITY, f32::min);
    let first_line: String = runs
        .iter()
        .filter(|r| (r.0 - top).abs() < 1.0)
        .map(|r| r.1.as_str())
        .collect();
    assert_eq!(first_line.chars().count(), 31, "{first_line}");
}

#[test]
fn a_comma_pair_at_the_margin_squeezes_only_on_a_line_with_a_hyphenation_point() {
    // 29.2 em of text: a line of 29 characters with two commas, then "测，" across the
    // margin. Word for Mac wraps the pair on a Chinese line, digits or not, and
    // squeezes it in (31 characters) when the line also holds a "‧".
    let first_line_len = |text: &str| {
        let body = format!(
            r#"<w:p><w:pPr><w:jc w:val="left"/><w:spacing w:after="0"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="SimSun" w:hAnsi="SimSun" w:eastAsia="SimSun"/><w:sz w:val="28"/></w:rPr><w:t>{text}</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="11192" w:h="16838"/><w:pgMar w:top="1440" w:right="1508" w:bottom="1440" w:left="1508"/></w:sectPr>"#
        );
        let bytes = crate::office::docx::testutil::with_settings(
            docx(&body, None, None),
            r#"<w:characterSpacingControl w:val="compressPunctuation"/>"#,
        );
        let runs: Vec<(f32, String)> = pages(bytes)[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) => Some((r.transform.ty, r.text.clone())),
                _ => None,
            })
            .collect();
        let top = runs.iter().map(|r| r.0).fold(f32::INFINITY, f32::min);
        runs.iter()
            .filter(|r| (r.0 - top).abs() < 1.0)
            .map(|r| r.1.chars().count())
            .sum::<usize>()
    };
    let line =
        "测测测，测测测测，测测测测测测测测测测测测测测测测测测测测测，测测测测测测测测测测测测";
    assert_eq!(first_line_len(line), 29);
    assert_eq!(first_line_len(&line.replacen('测', "‧", 1)), 31);
    // A line-final colon takes half its width like a comma: "测：" fits after 28.
    let colon =
        "测测测，测测测测，测测测测测测测测测测测测测测测测测测测测：测测测测测测测测测测测测";
    assert_eq!(first_line_len(colon), 30);
}

#[test]
fn a_character_after_a_latin_one_squeezes_only_half_that_ones_width() {
    // 35 characters with a bracket and a comma, then "/" or "测", then "系" 0.3 em past
    // the margin. Word for Mac squeezes "系" in after "测" (half an em) and wraps it
    // after "/" (half the slash: 0.14 em).
    let last_on_first_line = |prev: &str, prev_em: f64| {
        let right = ((38.85 - (36.0 + prev_em - 0.3)) * 240.0).round();
        let text = format!(
            "测测测测测（测测测测测，{}{prev}系{}",
            "测".repeat(23),
            "测".repeat(12)
        );
        let body = format!(
            r#"<w:p><w:pPr><w:jc w:val="both"/><w:autoSpaceDE w:val="0"/><w:autoSpaceDN w:val="0"/><w:ind w:right="{right}"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:eastAsia="SimSun"/><w:sz w:val="24"/></w:rPr><w:t>{text}</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1000" w:right="1291" w:bottom="1000" w:left="1291"/></w:sectPr>"#
        );
        let bytes = crate::office::docx::testutil::with_settings(
            docx(&body, None, None),
            r#"<w:characterSpacingControl w:val="compressPunctuation"/>"#,
        );
        let runs: Vec<(f32, f32, String)> = pages(bytes)[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) => {
                    Some((r.transform.ty, r.transform.tx, r.text.clone()))
                }
                _ => None,
            })
            .collect();
        let top = runs.iter().map(|r| r.0).fold(f32::INFINITY, f32::min);
        runs.iter()
            .filter(|r| (r.0 - top).abs() < 4.0)
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .and_then(|r| r.2.chars().last())
            .unwrap()
    };
    assert_eq!(last_on_first_line("测", 1.0), '系');
    assert_eq!(last_on_first_line("/", 0.278), '/');
}

#[test]
fn without_a_hanging_indent_tab_stop_a_tab_goes_to_the_default_stop() {
    // A 5 inch hanging indent and "I.<tab>ENTRY": Word for Mac tabs to the indent
    // (360 pt), or to the first default stop (36 pt) under noTabHangInd.
    let x_of_entry = |compat: &str| {
        let body = r#"<w:p><w:pPr><w:ind w:left="7200" w:hanging="7200"/></w:pPr><w:r><w:t>I.</w:t></w:r><w:r><w:tab/></w:r><w:r><w:t>ENTRY</w:t></w:r></w:p>"#;
        let bytes = crate::office::docx::testutil::with_settings(docx(body, None, None), compat);
        pages(bytes)[0]
            .items
            .iter()
            .find_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text == "ENTRY" => {
                    Some(r.transform.tx)
                }
                _ => None,
            })
            .expect("ENTRY")
    };
    let left = 72.0;
    assert!((x_of_entry("") - left - 360.0).abs() < 0.1);
    assert!((x_of_entry("<w:compat><w:noTabHangInd/></w:compat>") - left - 36.0).abs() < 0.1);
}

#[test]
fn balanced_widths_give_a_space_beside_chinese_half_an_em() {
    // "iiii WWWW 测测 ii" at 12 pt: Word for Mac draws the spaces beside "测" 6 pt wide
    // with balanceSingleByteDoubleByteWidth (3 pt without); the one between Latin
    // words stays 3 pt.
    let xs = |compat: &str| {
        let body = r#"<w:p><w:r><w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:eastAsia="SimSun"/><w:sz w:val="24"/></w:rPr><w:t xml:space="preserve">iiii WWWW 测测 ii</w:t></w:r></w:p>"#;
        let bytes = crate::office::docx::testutil::with_settings(docx(body, None, None), compat);
        let mut runs: Vec<(f32, String)> = pages(bytes)[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) => Some((r.transform.tx, r.text.clone())),
                _ => None,
            })
            .collect();
        runs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let at = |t: &str| runs.iter().find(|r| r.1.starts_with(t)).unwrap().0;
        let last = runs.last().unwrap().0;
        (at("测") - at("iiii"), last - at("测"))
    };
    let (plain_cjk, plain_ii) = xs("");
    let (bal_cjk, bal_ii) = xs("<w:compat><w:balanceSingleByteDoubleByteWidth/></w:compat>");
    assert!(
        (bal_cjk - plain_cjk - 3.0).abs() < 0.1,
        "{plain_cjk} {bal_cjk}"
    );
    assert!((bal_ii - plain_ii - 3.0).abs() < 0.1, "{plain_ii} {bal_ii}");
}

#[test]
fn balanced_widths_double_letter_spacing_after_chinese_characters() {
    // Four 12 pt SimSun characters spaced -20 twips, then "x": Word for Mac moves
    // each character 1 pt closer, or 2 pt with balanceSingleByteDoubleByteWidth.
    let x_at = |compat: &str| {
        let body = r#"<w:p><w:r><w:rPr><w:rFonts w:ascii="SimSun" w:hAnsi="SimSun" w:eastAsia="SimSun"/><w:spacing w:val="-20"/><w:sz w:val="24"/></w:rPr><w:t>测测测测</w:t></w:r><w:r><w:rPr><w:rFonts w:ascii="SimSun" w:hAnsi="SimSun" w:eastAsia="SimSun"/><w:sz w:val="24"/></w:rPr><w:t>x</w:t></w:r></w:p>"#;
        let bytes = crate::office::docx::testutil::with_settings(docx(body, None, None), compat);
        pages(bytes)[0]
            .items
            .iter()
            .find_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) if r.text == "x" => Some(r.transform.tx),
                _ => None,
            })
            .expect("x")
    };
    let plain = x_at("");
    let balanced = x_at("<w:compat><w:balanceSingleByteDoubleByteWidth/></w:compat>");
    assert!((plain - balanced - 4.0).abs() < 0.1, "{plain} {balanced}");
}

#[test]
fn a_character_grid_tracks_digits_like_chinese_characters() {
    // "测2020测" on a linesAndChars grid that narrows each character: Word for Mac
    // draws the digits narrower by the same tracking as the Chinese characters.
    let span = |grid: &str| {
        let body = format!(
            r#"<w:p><w:pPr><w:autoSpaceDE w:val="0"/><w:autoSpaceDN w:val="0"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:eastAsia="SimSun"/><w:sz w:val="28"/></w:rPr><w:t>测2020测</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/>{grid}</w:sectPr>"#
        );
        let runs: Vec<(f32, String)> = pages(docx(&body, None, None))[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) => Some((r.transform.tx, r.text.clone())),
                _ => None,
            })
            .collect();
        let last = runs.iter().map(|r| r.0).fold(f32::MIN, f32::max);
        let first = runs.iter().map(|r| r.0).fold(f32::MAX, f32::min);
        (last - first) as f64
    };
    let plain = span("");
    let tracked =
        span(r#"<w:docGrid w:type="linesAndChars" w:linePitch="579" w:charSpace="-849"/>"#);
    // One Chinese character and four digits before the last "测", each 0.21 pt narrower.
    assert!(
        (plain - tracked - 5.0 * 0.21).abs() < 0.05,
        "{plain} {tracked}"
    );
}

#[test]
fn a_character_after_spaces_takes_the_lines_whole_punctuation_squeeze() {
    // 34 characters with two colons, two balanced spaces (1 em), then "系": Word for
    // Mac squeezes "系" in 0.9 em past the margin (two half-em colons) and wraps it
    // at 1.1 em.
    let last_on_first_line = |over: f64| {
        let right = ((38.85 - (36.0 - over)) * 240.0).round();
        let mut line: Vec<char> = "测".repeat(34).chars().collect();
        line[3] = '：';
        line[10] = '：';
        let text = format!("{}  系{}", line.iter().collect::<String>(), "测".repeat(12));
        let body = format!(
            r#"<w:p><w:pPr><w:jc w:val="both"/><w:ind w:right="{right}"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:eastAsia="SimSun"/><w:sz w:val="24"/></w:rPr><w:t xml:space="preserve">{text}</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1000" w:right="1291" w:bottom="1000" w:left="1291"/></w:sectPr>"#
        );
        let bytes = crate::office::docx::testutil::with_settings(
            docx(&body, None, None),
            r#"<w:characterSpacingControl w:val="compressPunctuation"/><w:compat><w:balanceSingleByteDoubleByteWidth/></w:compat>"#,
        );
        let runs: Vec<(f32, f32, String)> = pages(bytes)[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) => {
                    Some((r.transform.ty, r.transform.tx, r.text.clone()))
                }
                _ => None,
            })
            .collect();
        let top = runs.iter().map(|r| r.0).fold(f32::INFINITY, f32::min);
        runs.iter()
            .filter(|r| (r.0 - top).abs() < 4.0)
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .and_then(|r| r.2.trim_end().chars().last())
            .unwrap()
    };
    assert_eq!(last_on_first_line(0.9), '系');
    assert_eq!(last_on_first_line(1.1), '测');
}

#[test]
fn a_justified_line_squeezes_its_spaces_to_three_quarters() {
    // The Word for Mac probe line: 25 underscores fit on the first line of a 439.4 pt
    // justified line by narrowing its eight spaces to about 0.754 of their width.
    let lines = |n: usize| {
        let body = format!(
            r#"<w:p><w:pPr><w:ind w:firstLine="567"/><w:jc w:val="both"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman"/><w:sz w:val="24"/></w:rPr><w:t xml:space="preserve">3. Контактні дані (електронна адреса та номер телефону) {}</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1134" w:right="850" w:bottom="1134" w:left="1701"/></w:sectPr>"#,
            "_".repeat(n)
        );
        let bytes = crate::office::docx::testutil::with_compat_mode(docx(&body, None, None), 15);
        let mut ys: Vec<i32> = pages(bytes)[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) => Some(r.transform.ty.round() as i32),
                _ => None,
            })
            .collect();
        ys.sort();
        ys.dedup();
        ys.len()
    };
    assert_eq!(lines(25), 1);
    assert_eq!(lines(26), 2);
}

#[test]
fn a_large_east_asian_line_takes_whole_grid_pitches_with_its_extra() {
    // 14 pt MS Mincho on an 18 pt line grid with default spacing: Word for Mac gives
    // the line two pitches (1.3 x 14 pt > 18 pt); 10.5 pt keeps one.
    let gap = |size: u32| {
        let p = |text: &str, sz: u32| {
            format!(
                r#"<w:p><w:r><w:rPr><w:rFonts w:ascii="MS Mincho" w:eastAsia="MS Mincho" w:hAnsi="MS Mincho"/><w:sz w:val="{sz}"/></w:rPr><w:t>{text}</w:t></w:r></w:p>"#
            )
        };
        let body = format!(
            r#"{}{}{}<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/><w:docGrid w:type="lines" w:linePitch="360"/></w:sectPr>"#,
            p("上", 21),
            p("中", size),
            p("下", 21)
        );
        let ys: Vec<f32> = pages(docx(&body, None, None))[0]
            .items
            .iter()
            .filter_map(|i| match i {
                crate::office::pptx::scene::Item::Text(r) => Some(r.transform.ty),
                _ => None,
            })
            .collect();
        (ys[2] - ys[0]) as f64
    };
    assert!((gap(21) - 36.0).abs() < 0.5, "{}", gap(21));
    assert!((gap(28) - 54.0).abs() < 0.5, "{}", gap(28));
}

#[test]
fn a_line_held_by_compressed_punctuation_stays_inside_the_margin() {
    // With compressPunctuation a line may take one more character by compressing its
    // CJK punctuation; the drawn line then gives that room back from the marks
    // instead of running past the right margin.
    let text = "测测（测测）测测，测测、测测。测测《测测》测测：测测；".repeat(4);
    for width in (5000..7000).step_by(53) {
        let body = format!(
            r#"<w:p><w:pPr><w:spacing w:after="0"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="SimSun" w:hAnsi="SimSun" w:eastAsia="SimSun"/><w:sz w:val="24"/></w:rPr><w:t>{text}</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="{}" w:h="16838"/><w:pgMar w:top="1440" w:right="1000" w:bottom="1440" w:left="1000"/></w:sectPr>"#,
            width + 2000
        );
        let bytes = crate::office::docx::testutil::with_settings(
            docx(&body, None, None),
            r#"<w:characterSpacingControl w:val="compressPunctuation"/>"#,
        );
        let out = crate::office::docx::testutil::pages(bytes);
        let right = (width + 1000) as f32 / 20.0;
        for item in &out[0].items {
            if let crate::office::pptx::scene::Item::Text(r) = item {
                let end = r.transform.tx + r.width as f32;
                assert!(
                    end <= right + 0.5,
                    "width {width}: {:?} ends at {end}",
                    r.text
                );
            }
        }
    }
}

#[test]
fn a_line_ended_by_a_page_break_is_not_justified() {
    // A justified paragraph whose last line ends with a page break: under
    // expandShiftReturn only a line break stretches its line, so this line stays
    // left-aligned and ends well short of the margin.
    let words = "word ".repeat(30);
    let body = format!(
        r#"<w:p><w:pPr><w:jc w:val="both"/></w:pPr><w:r><w:t xml:space="preserve">{words}end.</w:t></w:r><w:r><w:br w:type="page"/></w:r></w:p>{}"#,
        crate::office::docx::testutil::para("after")
    );
    let out = crate::office::docx::testutil::pages(docx(&body, None, None));
    let end = out[0]
        .items
        .iter()
        .filter_map(|i| match i {
            crate::office::pptx::scene::Item::Text(r) if r.text.contains("end.") => {
                Some(r.transform.tx + r.width as f32)
            }
            _ => None,
        })
        .fold(0.0f32, f32::max);
    assert!(end < 500.0, "{end}");
}
