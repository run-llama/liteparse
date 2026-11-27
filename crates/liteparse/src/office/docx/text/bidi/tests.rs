use super::*;

fn analysis(text: &str) -> Analysis {
    let mut a = Analysis {
        text: String::new(),
        original: String::new(),
        units: Vec::new(),
        source_to_analysis: Vec::new(),
    };
    a.push(text, None);
    a.finish();
    a
}

fn line(text: &str, byte_start: usize) -> LineBuild {
    let style = Rc::new(
        crate::office::docx::styles::StyleSheet::parse(None, Default::default())
            .run_style(&Default::default()),
    );
    let mut b = LineBuild::new(0.0);
    for (off, c) in text.char_indices() {
        let mut atom = blank(
            if c.is_whitespace() {
                Kind::Space
            } else {
                Kind::Text
            },
            &style,
        );
        atom.text = AtomText::new(&c.to_string());
        atom.width = 1.0;
        atom.bidi_range = Some(byte_start + off..byte_start + off + c.len_utf8());
        b.push(atom);
    }
    b
}

fn visual(b: &LineBuild) -> String {
    let mut ids: Vec<_> = (0..b.atoms.len()).collect();
    ids.sort_by(|a, c| b.xs[*a].total_cmp(&b.xs[*c]));
    ids.into_iter().map(|i| &*b.atoms[i].text).collect()
}

#[test]
fn rtl_clusters_move_without_reversing_logical_text_or_numbers() {
    for (s, want) in [
        ("אבג 12", "12 גבא"),
        ("مرحبا 123", "123 ابحرم"),
        ("abc אבג 12", "12 גבא abc"),
    ] {
        let a = analysis(s);
        let ctx = a.resolve(true);
        let mut b = line(s, 0);
        ctx.place(&mut b);
        assert_eq!(visual(&b), want);
        assert_eq!(b.atoms.iter().map(|a| &*a.text).collect::<String>(), s);
    }
}

#[test]
fn soft_break_keeps_paragraph_direction_and_l1_resets_trailing_space() {
    let a = analysis("אבג abc \n123 דה");
    let ctx = a.resolve(true);
    assert_eq!(ctx.info.paragraphs.len(), 1);
    let mut first = line("אבג abc ", 0);
    ctx.place(&mut first);
    assert_eq!(visual(&first), " abc גבא");
    assert_eq!(first.atoms.last().unwrap().bidi_level, 1);
    let mut second = line("123 דה", "אבג abc \n".len());
    ctx.place(&mut second);
    assert_eq!(visual(&second), "הד 123");
}

#[test]
fn latin_paragraph_keeps_its_base_with_embedded_hebrew_and_punctuation() {
    let a = analysis("abc (אבג) 12");
    let ctx = a.resolve(false);
    let mut b = line("abc (אבג) 12", 0);
    ctx.place(&mut b);
    assert_eq!(visual(&b), "abc (גבא) 12");
    let a = analysis("abc (12) xyz");
    let ctx = a.resolve(false);
    let mut b = line("abc (12) xyz", 0);
    let before = b.xs.clone();
    ctx.place(&mut b);
    assert_eq!(b.xs, before);
}

#[test]
fn overrides_preserve_source_byte_offsets_and_numeric_classes() {
    let mut st = crate::office::docx::styles::StyleSheet::parse(None, Default::default())
        .run_style(&Default::default());
    st.rtl = Some(true);
    st.caps = true;
    let st = Rc::new(st);
    let mut a = Analysis {
        text: String::new(),
        original: String::new(),
        units: Vec::new(),
        source_to_analysis: Vec::new(),
    };
    a.push("ß 12.3 אב", Some(&st));
    a.finish();
    assert_eq!(a.source_to_analysis.len(), "SS 12.3 אב".len() + 1);
    let ctx = a.resolve(true);
    assert!(ctx.level(3).is_ltr());
    assert!(ctx.level("SS 12.3 ".len()).is_rtl());
    assert!(a.text.contains("12.3"));
}

fn layout(body: &str, width: f64) -> Vec<Line> {
    let opened = crate::office::docx::extract::open(
        crate::office::docx::testutil::docx(body, None, None),
        &Default::default(),
    )
    .unwrap();
    let crate::office::docx::model::Block::Para(p) = &opened.sections[0].blocks[0] else {
        panic!("paragraph")
    };
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
        in_cell: Default::default(),
        in_header: Default::default(),
        grow_autofit: false,
        grid_in_table: false,
        no_leading: false,
        compat_mode: 15,
    };
    layout_paragraph(p, width, &env)
}

#[test]
fn rtl_glyphs_keep_logical_text_and_positive_run_and_decoration_bounds() {
    let lines = layout(
        r#"<w:p><w:pPr><w:bidi/></w:pPr><w:r><w:rPr><w:u w:val="single"/><w:highlight w:val="yellow"/><w:strike/></w:rPr><w:t>אבגדה</w:t></w:r></w:p>"#,
        250.0,
    );
    assert_eq!(lines[0].text, "אבגדה");
    let runs: Vec<_> = lines[0]
        .items
        .iter()
        .filter_map(|i| if let Item::Text(r) = i { Some(r) } else { None })
        .collect();
    assert!(!runs.is_empty());
    for r in &runs {
        assert!(r.width > 0.0);
        assert!(r.glyphs.iter().all(|g| g.x >= -0.01 && g.x <= r.width));
    }
    let r = runs.iter().find(|r| r.glyphs.len() >= 3).unwrap();
    assert!(r.glyphs[0].x > r.glyphs.last().unwrap().x);
    let paths = lines[0]
        .items
        .iter()
        .filter(|i| matches!(i, Item::Path(_)))
        .count();
    assert!(paths >= 3, "highlight, underline and strike");
}

#[test]
fn mixed_styles_numbers_caps_and_wrapping_preserve_logical_stream() {
    let body = r#"<w:p><w:pPr><w:bidi/></w:pPr><w:r><w:rPr><w:caps/></w:rPr><w:t xml:space="preserve">straße </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">مرحبا (123) </w:t></w:r><w:r><w:rPr><w:i/></w:rPr><w:t>אבג xyz</w:t></w:r></w:p>"#;
    let lines = layout(body, 105.0);
    assert!(lines.len() > 1);
    let text: String = lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(text, "STRASSE مرحبا (123) אבג xyz");
    for r in lines
        .iter()
        .flat_map(|l| &l.items)
        .filter_map(|i| if let Item::Text(r) = i { Some(r) } else { None })
    {
        assert!(r.width >= 0.0 && r.width.is_finite());
        assert!(r.glyphs.iter().all(|g| g.x >= -1.0 && g.x <= r.width + 1.0));
    }
}

#[test]
fn unicode_isolates_keep_logical_attribution_without_visible_control_width() {
    let text = "אב \u{2066}abc 12\u{2069} גד";
    let body = format!("<w:p><w:pPr><w:bidi/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>");
    let lines = layout(&body, 250.0);
    assert_eq!(lines[0].text, text);
    let runs: String = lines[0]
        .items
        .iter()
        .filter_map(|i| {
            if let Item::Text(r) = i {
                Some(r.text.as_str())
            } else {
                None
            }
        })
        .collect();
    assert!(runs.contains('\u{2066}') && runs.contains('\u{2069}'));
}

#[test]
fn mirrored_brackets_follow_resolved_direction_during_shaping() {
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
        in_cell: Default::default(),
        in_header: Default::default(),
        grow_autofit: false,
        grid_in_table: false,
        no_leading: false,
        compat_mode: 15,
    };
    let style = Rc::new(
        crate::office::docx::styles::StyleSheet::parse(None, Default::default())
            .run_style(&Default::default()),
    );
    let a = analysis("(אב)");
    let ctx = a.resolve(true);
    let (mut atoms, mut full, mut offsets) = (Vec::new(), String::new(), Vec::new());
    shape(
        "(אב)",
        &style,
        &env,
        &mut atoms,
        &mut full,
        &mut offsets,
        Some(&ctx),
    );
    let mut expected = Vec::new();
    shape_direction(
        ")",
        &style,
        &env,
        &mut expected,
        &mut String::new(),
        &mut Vec::new(),
        ShapeContext {
            direction: Some(rustybuzz::Direction::LeftToRight),
            context: None,
        },
    );
    assert!(!atoms[0].glyphs.as_slice().is_empty());
    assert_eq!(
        atoms[0].glyphs.as_slice()[0].0,
        expected[0].glyphs.as_slice()[0].0
    );
    assert_eq!(&*atoms[0].text, "(");
}

#[test]
fn cluster_internal_glyph_order_survives_visual_line_placement() {
    let a = analysis("א\u{05b0}ב 12");
    let ctx = a.resolve(true);
    let style = Rc::new(
        crate::office::docx::styles::StyleSheet::parse(None, Default::default())
            .run_style(&Default::default()),
    );
    let mut b = LineBuild::new(0.0);
    let mut off = 0;
    for text in ["א\u{05b0}", "ב", " ", "1", "2"] {
        let mut atom = blank(if text == " " { Kind::Space } else { Kind::Text }, &style);
        atom.text = AtomText::new(text);
        atom.width = 1.0;
        atom.glyphs = Glyphs::Many(vec![(31, 1.0, 0.0, 0.0), (32, 0.0, -0.3, 0.2)]);
        atom.bidi_range = Some(off..off + text.len());
        off += text.len();
        b.push(atom);
    }
    ctx.place(&mut b);
    assert!(b.xs[0] > b.xs[1]);
    assert!(b.xs[3] < b.xs[4]);
    assert_eq!(
        b.atoms[0].glyphs.as_slice(),
        &[(31, 1.0, 0.0, 0.0), (32, 0.0, -0.3, 0.2)]
    );
}

#[test]
fn hebrew_number_separator_exception_crosses_style_boundaries() {
    let mut st = crate::office::docx::styles::StyleSheet::parse(None, Default::default())
        .run_style(&Default::default());
    st.rtl = Some(true);
    let st = Rc::new(st);
    let mut a = Analysis {
        text: String::new(),
        original: String::new(),
        units: Vec::new(),
        source_to_analysis: Vec::new(),
    };
    for t in ["אב 12", "-", "34"] {
        a.push(t, Some(&st));
    }
    a.finish();
    let ctx = a.resolve(true);
    assert!(ctx.level("אב 12".len()).is_ltr());
    assert!(a.text.contains("12-34"));
}

#[test]
fn arabic_style_boundary_retains_neighbor_context_for_joining() {
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
        in_cell: Default::default(),
        in_header: Default::default(),
        grow_autofit: false,
        grid_in_table: false,
        no_leading: false,
        compat_mode: 15,
    };
    let mut s = crate::office::docx::styles::StyleSheet::parse(None, Default::default())
        .run_style(&Default::default());
    s.font_cs = "Arial".into();
    let s = Rc::new(s);
    let a = analysis("بب");
    let ctx = a.resolve(true);
    let (mut whole, mut split) = (Vec::new(), Vec::new());
    shape(
        "بب",
        &s,
        &env,
        &mut whole,
        &mut String::new(),
        &mut Vec::new(),
        Some(&ctx),
    );
    let (mut text, mut offsets) = (String::new(), Vec::new());
    shape(
        "ب",
        &s,
        &env,
        &mut split,
        &mut text,
        &mut offsets,
        Some(&ctx),
    );
    shape(
        "ب",
        &s,
        &env,
        &mut split,
        &mut text,
        &mut offsets,
        Some(&ctx),
    );
    assert_eq!(whole.len(), 2);
    assert_eq!(split.len(), 2);
    for (a, b) in whole.iter().zip(&split) {
        assert_eq!(a.glyphs.as_slice(), b.glyphs.as_slice());
    }
}

#[test]
fn rtl_symbol_fonts_preserve_unicode_bullets_and_source_offsets() {
    let mut st = crate::office::docx::styles::StyleSheet::parse(None, Default::default())
        .run_style(&Default::default());
    st.font = "Wingdings".into();
    st.font_cs = "Arial".into();
    st.rtl = Some(true);
    assert_eq!(family_for(&st, '\u{f076}'), "Wingdings");
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
        in_cell: Default::default(),
        in_header: Default::default(),
        grow_autofit: false,
        grid_in_table: false,
        no_leading: false,
        compat_mode: 15,
    };
    let mut atoms = Vec::new();
    let mut full = String::new();
    let mut offsets = Vec::new();
    shape_text(
        "\u{f076}\u{f0b7}\u{f0fc}",
        &Rc::new(st.clone()),
        &env,
        &mut atoms,
        &mut full,
        &mut offsets,
    );
    assert_eq!(atoms.iter().map(|a| &*a.text).collect::<String>(), "❖🕐✓");
    atoms.clear();
    full.clear();
    offsets.clear();
    shape_text(
        "vü",
        &Rc::new(st),
        &env,
        &mut atoms,
        &mut full,
        &mut offsets,
    );
    assert_eq!(offsets, [0, 1]);
    let start = full.len();
    let mut arabic = crate::office::docx::styles::StyleSheet::parse(None, Default::default())
        .run_style(&Default::default());
    arabic.rtl = Some(true);
    shape_text(
        "اب",
        &Rc::new(arabic),
        &env,
        &mut atoms,
        &mut full,
        &mut offsets,
    );
    assert_eq!(offsets[2], start);
    assert!(offsets.iter().all(|&n| full.is_char_boundary(n)));
}

#[test]
fn shared_run_borders_join_visual_edges_across_rtl_style_boundaries() {
    for second in ["דהו", "abc"] {
        let body = format!(
            r#"<w:p><w:pPr><w:bidi/></w:pPr><w:r><w:rPr><w:bdr w:val="single" w:sz="8" w:color="FF0000"/></w:rPr><w:t>אבג</w:t></w:r><w:r><w:rPr><w:b/><w:bdr w:val="single" w:sz="8" w:color="FF0000"/></w:rPr><w:t>{second}</w:t></w:r></w:p>"#
        );
        let lines = layout(&body, 250.0);
        let runs: Vec<_> = lines[0]
            .items
            .iter()
            .filter_map(|i| if let Item::Text(r) = i { Some(r) } else { None })
            .collect();
        let left = runs
            .iter()
            .map(|r| r.transform.tx as f64)
            .fold(f64::INFINITY, f64::min);
        let right = runs
            .iter()
            .map(|r| r.transform.tx as f64 + r.width)
            .fold(f64::NEG_INFINITY, f64::max);
        let sides: Vec<_> = lines[0]
            .items
            .iter()
            .filter_map(|i| {
                if let Item::Path(p) = i {
                    let b = p.path.bounds();
                    (b.height() > 5.0 && b.width() < 2.0).then_some(b)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(sides.len(), 2);
        assert!(sides.iter().any(|b| (b.right() as f64 - left).abs() < 0.01));
        assert!(sides.iter().any(|b| (b.left() as f64 - right).abs() < 0.01));
    }
}

#[test]
fn rtl_trailing_tabs_keep_the_visible_cluster_origins() {
    let plain = analysis("אבג");
    let with_tabs = analysis("אבג\t\t\t");
    let mut expected = line("אבג", 0);
    plain.resolve(true).place(&mut expected);
    let mut actual = line("אבג\t\t\t", 0);
    for atom in &mut actual.atoms[3..] {
        atom.kind = Kind::Tab;
        atom.width = 12.0;
    }
    actual.xs = vec![0.0, 1.0, 2.0, 3.0, 15.0, 27.0];
    with_tabs.resolve(true).place(&mut actual);
    assert_eq!(&actual.xs[..3], &expected.xs[..3]);
    assert_eq!(
        actual.atoms.iter().map(|a| &*a.text).collect::<String>(),
        "אבג\t\t\t"
    );
}

#[test]
fn rtl_trailing_tabs_preserve_wrapped_body_ink_bounds() {
    let text = "مرحبا بالعالم ".repeat(24) + "نهاية";
    let paragraph = |tabs: usize| {
        format!(
            r#"<w:p><w:pPr><w:bidi/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="Traditional Arabic" w:cs="Traditional Arabic"/><w:sz w:val="32"/></w:rPr><w:t>{text}</w:t></w:r><w:r><w:footnoteReference w:id="1"/></w:r>{}</w:p>"#,
            "<w:r><w:tab/></w:r>".repeat(tabs)
        )
    };
    let plain = layout(&paragraph(0), 439.35);
    let tabs = layout(&paragraph(11), 439.35);
    let runs = |lines: &[Line]| -> Vec<(String, f64, f64)> {
        lines
            .iter()
            .flat_map(|l| &l.items)
            .filter_map(|i| {
                if let Item::Text(r) = i {
                    Some((r.text.clone(), r.transform.tx as f64, r.width))
                } else {
                    None
                }
            })
            .collect()
    };
    let plain = runs(&plain);
    let tabs = runs(&tabs);
    assert_eq!(plain.len(), tabs.len());
    assert_eq!(
        plain.iter().map(|r| r.0.as_str()).collect::<String>(),
        tabs.iter().map(|r| r.0.as_str()).collect::<String>()
    );
    assert!(tabs.iter().all(|(_, x, w)| *x >= -0.01 && x + w <= 439.36));
    for (a, b) in plain.iter().zip(&tabs) {
        assert!(
            (a.1 - b.1).abs() < 0.01,
            "trailing tabs shifted body ink: {a:?} vs {b:?}"
        );
    }
}

#[test]
fn continuation_replays_wrapped_prefix_and_keeps_full_bidi_context() {
    for (rtl, leading_control) in [(false, false), (true, false), (false, true), (true, true)] {
        let content = "abc אבג 123 def مرحبا 456 ".repeat(30);
        let control = if leading_control {
            r#"<w:r><w:br w:type="column"/></w:r>"#
        } else {
            ""
        };
        let body = format!(
            r#"<w:p><w:pPr><w:bidi w:val="{}"/></w:pPr>{control}<w:r><w:t>{content}</w:t></w:r></w:p>"#,
            u8::from(rtl)
        );
        let mut opened = crate::office::docx::extract::open(
            crate::office::docx::testutil::docx(&body, None, None),
            &Default::default(),
        )
        .unwrap();
        let crate::office::docx::model::Block::Para(p) = &mut opened.sections[0].blocks[0] else {
            panic!("paragraph")
        };
        if leading_control {
            p.num = Some(crate::office::docx::model::NumLabel {
                picture: None,
                picture_count: 0,
                picture_table: None,
                text: "LABEL".into(),
                style: p.mark.clone(),
                suffix: Suffix::Space,
                jc: "left".into(),
            });
        }
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
            in_cell: Default::default(),
            in_header: Default::default(),
            grow_autofit: false,
            grid_in_table: false,
            no_leading: false,
            compat_mode: 15,
        };
        let initial_wrap = WrapIn {
            contour: Vec::new(),
            rects: vec![(0.0, 10.0, 0.0, 260.0), (10.0, 70.0, 0.0, 60.0)],
            est_h: 12.0,
            min_gap: 20.0,
        };
        let original = layout_paragraph_wrapped(p, 260.0, &env, &initial_wrap);
        assert!(original.len() > 4);
        let expected = original.iter().map(|l| l.text.as_str()).collect::<String>();
        assert_eq!(
            expected.matches("LABEL").count(),
            usize::from(leading_control)
        );
        assert!(original[0].skip > 0.0);
        let tail_wrap = WrapIn {
            contour: Vec::new(),
            rects: vec![(0.0, 30.0, 100.0, 130.0)],
            est_h: 12.0,
            min_gap: 20.0,
        };
        let resumed = layout_paragraph_resuming(p, 130.0, &env, &original[..2], &tail_wrap);
        let signature = |l: &Line| {
            let runs: Vec<_> = l
                .items
                .iter()
                .filter_map(|i| match i {
                    Item::Text(r) => {
                        Some((r.text.clone(), r.transform.tx, r.transform.ty, r.width))
                    }
                    _ => None,
                })
                .collect();
            (l.text.clone(), l.height, l.skip, runs)
        };
        for (a, b) in original[..2].iter().zip(&resumed[..2]) {
            assert_eq!(signature(a), signature(b));
        }
        assert_eq!(
            resumed.iter().map(|l| l.text.as_str()).collect::<String>(),
            expected
        );
        assert_eq!(resumed[2].replay.unwrap().right, 100.0);
        assert!(
            resumed[2..]
                .iter()
                .all(|l| l.replay.unwrap().right <= 130.0)
        );
        // A second transition replays the complete history, including the narrower column.
        let again = layout_paragraph_resuming(p, 200.0, &env, &resumed[..4], &WrapIn::default());
        for (a, b) in resumed[..4].iter().zip(&again[..4]) {
            assert_eq!(signature(a), signature(b));
        }
        assert_eq!(
            again.iter().map(|l| l.text.as_str()).collect::<String>(),
            expected
        );
        assert_eq!(again[4].replay.unwrap().right, 200.0);
    }
}

#[test]
fn original_whitespace_types_survive_rtl_run_overrides() {
    let mut style = crate::office::docx::styles::StyleSheet::parse(None, Default::default())
        .run_style(&Default::default());
    style.rtl = Some(true);
    let style = Rc::new(style);
    for (source, expected) in [("אבג   ", "גבא   "), ("אבג  \tabc", "גבא  \tabc")] {
        let mut a = Analysis {
            text: String::new(),
            original: String::new(),
            units: Vec::new(),
            source_to_analysis: Vec::new(),
        };
        a.push(source, Some(&style));
        a.finish();
        let mut b = line(source, 0);
        a.resolve(false).place(&mut b);
        assert_eq!(visual(&b), expected);
        assert_eq!(b.atoms.iter().map(|a| &*a.text).collect::<String>(), source);
    }
}

#[test]
fn original_whitespace_reset_keeps_ltr_right_aligned_ink_inside_line() {
    let content = "مرحبا بالعالم هذا مثال عربي لاختبار محاذاة النص";
    let body = format!(
        r#"<w:p><w:pPr><w:bidi w:val="0"/><w:jc w:val="end"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="Traditional Arabic" w:cs="Traditional Arabic"/><w:sz w:val="32"/><w:rtl/></w:rPr><w:t xml:space="preserve">{content}{}</w:t></w:r><w:r><w:t>120</w:t></w:r></w:p>"#,
        " ".repeat(47)
    );
    let lines = layout(&body, 400.0);
    assert!(lines.len() >= 2);
    assert_eq!(
        lines.iter().map(|l| l.text.as_str()).collect::<String>(),
        format!("{content}{}120", " ".repeat(47))
    );
    for run in lines.iter().flat_map(|l| &l.items).filter_map(|i| match i {
        Item::Text(r) => Some(r),
        _ => None,
    }) {
        let ink_right = run.glyphs.last().map_or(0.0, |g| g.x).max(run.width);
        assert!(
            run.transform.tx >= -0.01 && run.transform.tx as f64 + ink_right <= 400.01,
            "ink outside line: {}..{}",
            run.transform.tx,
            run.transform.tx as f64 + ink_right
        );
    }
}

#[test]
fn first_line_indent_uses_the_paragraph_start_edge() {
    let bounds = |l: &Line| {
        l.items
            .iter()
            .filter_map(|i| match i {
                Item::Text(r) => Some((r.transform.tx as f64, r.transform.tx as f64 + r.width)),
                _ => None,
            })
            .fold((f64::INFINITY, f64::NEG_INFINITY), |a, b| {
                (a.0.min(b.0), a.1.max(b.1))
            })
    };
    for rtl in [false, true] {
        for first in [-36.0_f64, 0.0, 36.0] {
            let attr = if first < 0.0 { "hanging" } else { "firstLine" };
            let body = format!(
                r#"<w:p><w:pPr><w:bidi w:val="{}"/><w:jc w:val="start"/><w:ind w:start="720" w:end="720" w:{attr}="{}"/></w:pPr><w:r><w:t>abc אבג 123</w:t><w:br/><w:t>abc אבג 123</w:t></w:r></w:p>"#,
                u8::from(rtl),
                first.abs() * 20.0
            );
            let lines = layout(&body, 300.0);
            assert_eq!(lines.len(), 2);
            let (a, b) = (bounds(&lines[0]), bounds(&lines[1]));
            let expected = if rtl { -first } else { first };
            assert!(
                (a.0 - b.0 - expected).abs() < 0.01,
                "rtl={rtl}, first={first}: {a:?} {b:?}"
            );
            assert!((a.1 - b.1 - expected).abs() < 0.01);
            let measure = lines[0].replay.unwrap();
            assert!((measure.left - if rtl { 36.0 } else { 36.0 + first }).abs() < 0.01);
            assert!((measure.right - if rtl { 264.0 - first } else { 264.0 }).abs() < 0.01);
        }
    }
}

#[test]
fn rtl_first_line_indent_changes_wrapping_and_preserves_continuation() {
    let content = "abc אבג 123 ".repeat(35);
    for first in [-24.0_f64, 24.0] {
        let attr = if first < 0.0 { "hanging" } else { "firstLine" };
        let body = format!(
            r#"<w:p><w:pPr><w:bidi/><w:jc w:val="start"/><w:ind w:start="480" w:end="240" w:{attr}="{}"/></w:pPr><w:r><w:t>{content}</w:t></w:r></w:p>"#,
            first.abs() * 20.0
        );
        let opened = crate::office::docx::extract::open(
            crate::office::docx::testutil::docx(&body, None, None),
            &Default::default(),
        )
        .unwrap();
        let crate::office::docx::model::Block::Para(p) = &opened.sections[0].blocks[0] else {
            panic!("paragraph")
        };
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
        let lines = layout_paragraph_wrapped(p, 180.0, &env, &WrapIn::default());
        assert!(lines.len() > 4);
        assert_eq!(lines[0].replay.unwrap().right, 156.0 - first);
        assert_eq!(lines[1].replay.unwrap().right, 156.0);
        let resumed = layout_paragraph_resuming(p, 110.0, &env, &lines[..2], &WrapIn::default());
        for (a, b) in lines[..2].iter().zip(&resumed[..2]) {
            assert_eq!(a.text, b.text);
            assert_eq!(format!("{:?}", a.items), format!("{:?}", b.items));
        }
        assert_eq!(resumed[2].replay.unwrap().right, 86.0);
        let normalized = |ls: &[Line]| {
            ls.iter()
                .flat_map(|l| l.text.chars())
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
        };
        assert_eq!(normalized(&lines), normalized(&resumed));
        assert_eq!(
            normalized(&lines),
            content
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
        );
    }
}

#[test]
fn rtl_hanging_list_tab_aligns_body_after_deferred_label() {
    for leading_break in [false, true] {
        let control = if leading_break {
            r#"<w:r><w:br w:type="column"/></w:r>"#
        } else {
            ""
        };
        let body = format!(
            r#"<w:p><w:pPr><w:bidi/><w:jc w:val="start"/><w:ind w:start="720" w:end="720" w:hanging="720"/></w:pPr>{control}<w:r><w:t>אבג</w:t><w:br/><w:t>אבג</w:t></w:r></w:p>"#
        );
        let mut opened = crate::office::docx::extract::open(
            crate::office::docx::testutil::docx(&body, None, None),
            &Default::default(),
        )
        .unwrap();
        let crate::office::docx::model::Block::Para(p) = &mut opened.sections[0].blocks[0] else {
            panic!("paragraph")
        };
        p.num = Some(crate::office::docx::model::NumLabel {
            picture: None,
            picture_count: 0,
            picture_table: None,
            text: "1.".into(),
            style: p.mark.clone(),
            suffix: Suffix::Tab,
            jc: "left".into(),
        });
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
        let lines = layout_paragraph(p, 300.0, &env);
        let n = usize::from(leading_break);
        assert_eq!(lines.len(), n + 2);
        assert_eq!(
            lines
                .iter()
                .map(|l| l.text.as_str())
                .collect::<String>()
                .matches("1.")
                .count(),
            1
        );
        let body_right = |l: &Line| {
            l.items
                .iter()
                .find_map(|i| match i {
                    Item::Text(r) if r.text == "אבג" => Some(r.transform.tx as f64 + r.width),
                    _ => None,
                })
                .unwrap()
        };
        assert!(
            (body_right(&lines[n]) - body_right(&lines[n + 1])).abs() < 0.01,
            "first={} next={}",
            body_right(&lines[n]),
            body_right(&lines[n + 1])
        );
        assert_eq!(lines[n].replay.unwrap().right, 300.0);
        assert_eq!(lines[n + 1].replay.unwrap().right, 264.0);
    }
}

#[test]
fn rtl_list_justification_keeps_label_anchor_and_body_separate() {
    for jc in ["left", "center", "right"] {
        for number in ["1", "10", "100"] {
            for paragraph_jc in ["start", "both", "distribute"] {
                for suffix in [Suffix::Tab, Suffix::Space, Suffix::Nothing] {
                    for content in [
                        "אבג דהו זחט ".repeat(20),
                        "abc def ghi ".repeat(20),
                        String::new(),
                        "   ".into(),
                    ] {
                        let body = format!(
                            r#"<w:p><w:pPr><w:bidi/><w:jc w:val="{paragraph_jc}"/><w:ind w:start="720" w:end="720" w:hanging="720"/></w:pPr><w:r><w:t>{content}</w:t></w:r></w:p>"#
                        );
                        let mut opened = crate::office::docx::extract::open(
                            crate::office::docx::testutil::docx(&body, None, None),
                            &Default::default(),
                        )
                        .unwrap();
                        let crate::office::docx::model::Block::Para(p) =
                            &mut opened.sections[0].blocks[0]
                        else {
                            panic!("paragraph")
                        };
                        p.num = Some(crate::office::docx::model::NumLabel {
                            picture: None,
                            picture_count: 0,
                            picture_table: None,
                            text: number.into(),
                            style: p.mark.clone(),
                            suffix,
                            jc: jc.into(),
                        });
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
                        let lines = layout_paragraph(p, 300.0, &env);
                        let plain =
                            layout(&format!("<w:p><w:r><w:t>{number}</w:t></w:r></w:p>"), 300.0);
                        let label_width = plain[0]
                            .items
                            .iter()
                            .filter_map(|i| match i {
                                Item::Text(r) => Some(r.width),
                                _ => None,
                            })
                            .sum::<f64>();
                        let (mut space_atoms, mut scratch, mut offsets) =
                            (Vec::new(), String::new(), Vec::new());
                        shape_text(
                            " ",
                            &p.mark,
                            &env,
                            &mut space_atoms,
                            &mut scratch,
                            &mut offsets,
                        );
                        let space_width: f64 = space_atoms.iter().map(|a| a.width).sum();
                        let mut label = None;
                        let mut body_right = f64::NEG_INFINITY;
                        for item in &lines[0].items {
                            if let Item::Text(r) = item {
                                assert!(r.width >= 0.0);
                                assert!(
                                    r.glyphs
                                        .iter()
                                        .all(|g| g.x >= -0.02 && g.x <= r.width + 0.02)
                                );
                                if r.text.trim() == number {
                                    let x =
                                        r.glyphs.iter().map(|g| g.x).fold(f64::INFINITY, f64::min);
                                    label = Some((r.transform.tx as f64 + x, label_width));
                                } else {
                                    let trailing = if r.text.is_ascii() {
                                        r.text.chars().rev().take_while(|c| *c == ' ').count()
                                            as f64
                                            * space_width
                                    } else {
                                        0.0
                                    };
                                    body_right =
                                        body_right.max(r.transform.tx as f64 + r.width - trailing);
                                }
                            }
                        }
                        let (x, width) = label.unwrap_or_else(|| {
                            panic!(
                                "missing label {jc}/{number}/{paragraph_jc}/{suffix:?}/{}: {:?}",
                                content.len(),
                                lines[0].items
                            )
                        });
                        let anchor = match jc {
                            "left" => x + width,
                            "center" => x + width / 2.0,
                            _ => x,
                        };
                        assert!(
                            (anchor - 300.0).abs() < 0.02,
                            "{jc}/{number}/{paragraph_jc}: anchor={anchor}"
                        );
                        if suffix == Suffix::Tab {
                            assert!(
                                body_right <= 264.01,
                                "{jc}/{number}/{paragraph_jc}: body={body_right}"
                            );
                        }
                        assert!(
                            x + 0.02 >= body_right,
                            "label overlaps body {jc}/{number}/{paragraph_jc}/{suffix:?}/{}: x={x}, body={body_right}",
                            content.len()
                        );
                        assert_eq!(
                            lines
                                .iter()
                                .map(|l| l.text.as_str())
                                .collect::<String>()
                                .matches(number)
                                .count(),
                            1
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn rtl_numbering_punctuation_and_decorations_keep_logical_text() {
    for suffix in [Suffix::Nothing, Suffix::Space, Suffix::Tab] {
        let style =
            r#"<w:rPr><w:u w:val="single"/><w:highlight w:val="yellow"/><w:strike/></w:rPr>"#;
        let body = format!(
            r#"<w:p><w:pPr><w:bidi/><w:jc w:val="start"/><w:ind w:start="720" w:hanging="720"/>{style}</w:pPr><w:r>{style}<w:t>Latin body</w:t></w:r></w:p>"#
        );
        let mut opened = crate::office::docx::extract::open(
            crate::office::docx::testutil::docx(&body, None, None),
            &Default::default(),
        )
        .unwrap();
        let crate::office::docx::model::Block::Para(p) = &mut opened.sections[0].blocks[0] else {
            panic!("paragraph")
        };
        p.num = Some(crate::office::docx::model::NumLabel {
            picture: None,
            picture_count: 0,
            picture_table: None,
            text: "12.".into(),
            style: p.mark.clone(),
            suffix,
            jc: "left".into(),
        });
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
        let lines = layout_paragraph(p, 300.0, &env);
        let runs: Vec<_> = lines[0]
            .items
            .iter()
            .filter_map(|i| if let Item::Text(r) = i { Some(r) } else { None })
            .collect();
        assert_eq!(
            runs.iter()
                .flat_map(|r| r.text.chars())
                .filter(|c| !c.is_whitespace())
                .collect::<String>(),
            "12.Latinbody"
        );
        for r in &runs {
            assert!(r.width >= 0.0);
            assert!(
                r.glyphs
                    .iter()
                    .all(|g| g.x >= -0.02 && g.x <= r.width + 0.02)
            );
        }
        let digits = runs.iter().find(|r| r.text.trim() == "12").unwrap();
        let dot = runs.iter().find(|r| r.text.trim() == ".").unwrap();
        assert!(dot.transform.tx < digits.transform.tx);
        assert!(
            lines[0]
                .items
                .iter()
                .filter(|i| matches!(i, Item::Path(_)))
                .count()
                >= 3
        );
    }
}

#[test]
fn rtl_hanging_indent_does_not_add_a_tab_stop_to_later_lines() {
    let make = |hanging| {
        format!(
            r#"<w:p><w:pPr><w:bidi/><w:jc w:val="start"/><w:ind w:start="720" w:end="720" w:hanging="{hanging}"/></w:pPr><w:r><w:t>אבג</w:t><w:br/><w:tab/><w:t>דהו</w:t></w:r></w:p>"#
        )
    };
    let zero = layout(&make(0), 300.0);
    let hanging = layout(&make(360), 300.0);
    assert_eq!(
        format!("{:?}", zero[1].items),
        format!("{:?}", hanging[1].items)
    );
}

#[test]
fn rtl_numbering_brackets_are_shaped_in_the_label_context() {
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
        compat_mode: 12,
    };
    for (left, right) in [("(", ")"), ("[", "]"), ("<", ">")] {
        let body = r#"<w:p><w:pPr><w:bidi/><w:jc w:val="start"/><w:ind w:start="720" w:hanging="720"/></w:pPr><w:r><w:t>Body</w:t></w:r></w:p>"#;
        let mut opened = crate::office::docx::extract::open(
            crate::office::docx::testutil::docx(body, None, None),
            &Default::default(),
        )
        .unwrap();
        let crate::office::docx::model::Block::Para(p) = &mut opened.sections[0].blocks[0] else {
            panic!("paragraph")
        };
        let label = format!("{left}12{right}");
        p.num = Some(crate::office::docx::model::NumLabel {
            picture: None,
            picture_count: 0,
            picture_table: None,
            text: label.clone(),
            style: p.mark.clone(),
            suffix: Suffix::Tab,
            jc: "left".into(),
        });
        let lines = layout_paragraph(p, 300.0, &env);
        assert_eq!(
            lines[0]
                .text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>(),
            format!("{label}Body")
        );
        let runs: Vec<_> = lines[0]
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Text(r) => Some(r),
                _ => None,
            })
            .collect();
        let first = runs.iter().find(|r| r.text == left).unwrap();
        let last = runs.iter().find(|r| r.text == right).unwrap();
        assert!(first.transform.tx > last.transform.tx);
        for (run, mirrored) in [(first, right), (last, left)] {
            let (mut expected, mut full, mut offsets) = (Vec::new(), String::new(), Vec::new());
            shape_direction(
                mirrored,
                &p.mark,
                &env,
                &mut expected,
                &mut full,
                &mut offsets,
                ShapeContext {
                    direction: Some(rustybuzz::Direction::LeftToRight),
                    context: None,
                },
            );
            assert_eq!(run.glyphs[0].id, expected[0].glyphs.as_slice()[0].0);
        }
    }
}

#[test]
fn slashes_in_right_to_left_runs_reverse_a_date() {
    // "02" "/"(rtl) "03" "/"(rtl) "2025" in a right-to-left paragraph reads
    // 2025/03/02 from the left, as in Word; plain slashes keep 02/03/2025.
    let plain = Rc::new(
        crate::office::docx::styles::StyleSheet::parse(None, Default::default())
            .run_style(&Default::default()),
    );
    let mut rtl = (*plain).clone();
    rtl.rtl = Some(true);
    let rtl = Rc::new(rtl);
    let order = |slash: &Rc<RunStyle>| {
        let mut a = Analysis {
            text: String::new(),
            original: String::new(),
            units: Vec::new(),
            source_to_analysis: Vec::new(),
        };
        for (t, s) in [
            ("02", &plain),
            ("/", slash),
            ("03", &plain),
            ("/", slash),
            ("2025", &plain),
        ] {
            a.push(t, Some(s));
        }
        a.finish();
        let ctx = a.resolve(true);
        let mut b = line("02/03/2025", 0);
        ctx.place(&mut b);
        visual(&b)
    };
    assert_eq!(order(&rtl), "2025/03/02");
    assert_eq!(order(&plain), "02/03/2025");
}
