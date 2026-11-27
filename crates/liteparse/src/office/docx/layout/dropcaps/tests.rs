use super::*;
use crate::office::docx::testutil::docx;

fn fixture(attrs: &str, following_props: &str) -> String {
    format!(
        r#"<w:p><w:pPr><w:framePr {attrs}/><w:rPr><w:sz w:val="58"/></w:rPr></w:pPr><w:r><w:rPr><w:sz w:val="58"/><w:position w:val="-5"/></w:rPr><w:t>T</w:t></w:r></w:p><w:p><w:pPr><w:spacing w:line="240" w:lineRule="exact"/>{following_props}</w:pPr><w:r><w:rPr><w:sz w:val="20"/></w:rPr><w:t>first</w:t><w:br/><w:t>second</w:t><w:br/><w:t>third</w:t><w:br/><w:t>fourth</w:t></w:r></w:p>"#
    )
}

fn with_flow(body: &str, test: impl FnOnce(&Flow<'_>, &[Block])) {
    let opened =
        crate::office::docx::extract::open(docx(body, None, None), &Default::default()).unwrap();
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], true, &Default::default());
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
        in_cell: std::cell::Cell::new(false),
        in_header: std::cell::Cell::new(false),
        grid_in_table: false,
        no_leading: false,
        compat_mode: 15,
    };
    test(&Flow::new(&env, 15), &opened.sections[0].blocks);
}

fn generated(p: &Paragraph) -> &Anchored {
    let Inline::Anchor(a) = &p.inlines[0] else {
        panic!("missing cap")
    };
    a
}

#[test]
fn drop_cap_count_controls_first_lines_and_preserves_body() {
    for count in [1, 2, 3] {
        with_flow(
            &fixture(
                &format!(r#"w:dropCap="drop" w:lines="{count}" w:hSpace="80""#),
                "",
            ),
            |flow, blocks| {
                let p = flow.drop_cap_pair(&blocks[0], &blocks[1], 200.0).unwrap();
                let a = generated(&p);
                assert_eq!(a.pic.h, count as f64 * 12.0);
                assert_eq!(a.dist, [0.0, 0.0, 4.0, 4.0]);
                let wrap = crate::office::docx::text::WrapIn {
                    contour: Vec::new(),
                    rects: vec![(0.0, a.pic.h, 0.0, a.pic.w + 4.0)],
                    est_h: 12.0,
                    min_gap: 0.0,
                };
                let lines =
                    crate::office::docx::text::layout_paragraph_wrapped(&p, 200.0, flow.env, &wrap);
                let text: String = lines.iter().map(|l| l.text.as_str()).collect();
                assert_eq!(text.replace('\n', ""), "firstsecondthirdfourth");
                for (i, l) in lines.iter().enumerate().take(4) {
                    let x = l
                        .items
                        .iter()
                        .find_map(|it| match it {
                            Item::Text(t) => Some(t.transform.tx as f64),
                            _ => None,
                        })
                        .unwrap();
                    assert!((x - if i < count { a.pic.w + 4.0 } else { 0.0 }).abs() < 0.01);
                }
            },
        );
    }
}

#[test]
fn drop_cap_defaults_margin_mode_and_vertical_precedence() {
    with_flow(
        &fixture(r#"w:dropCap="margin" w:y="8000" w:yAlign="bottom""#, ""),
        |flow, blocks| {
            let p = flow.drop_cap_pair(&blocks[0], &blocks[1], 200.0).unwrap();
            let a = generated(&p);
            assert_eq!(a.pic.h, 12.0);
            assert_eq!(a.x, Ok(-a.pic.w));
            assert_eq!(a.y, Ok(0.0));
            assert_eq!(a.dist, [0.0; 4]);
            assert_eq!(a.wrap, Wrap::None);
            let lines = layout_paragraph(&p, 200.0, flow.env);
            assert_eq!(
                lines
                    .iter()
                    .map(|l| l.text.as_str())
                    .collect::<String>()
                    .replace('\n', ""),
                "firstsecondthirdfourth"
            );
        },
    );
    with_flow(
        &fixture(
            r#"w:dropCap="drop" w:lines="2" w:wrap="none" w:hSpace="400""#,
            "",
        ),
        |flow, blocks| {
            let p = flow.drop_cap_pair(&blocks[0], &blocks[1], 200.0).unwrap();
            assert_eq!(generated(&p).dist, [0.0; 4]);
            assert_eq!(generated(&p).wrap, Wrap::None);
        },
    );
}

#[test]
fn drop_cap_glyph_metrics_and_following_anchor_survive_pairing() {
    let body = fixture(
        r#"w:dropCap="drop" w:lines="2""#,
        r#"<w:spacing w:before="200" w:line="240" w:lineRule="exact"/>"#,
    );
    with_flow(&body, |flow, blocks| {
        let p = flow.drop_cap_pair(&blocks[0], &blocks[1], 200.0).unwrap();
        let a = generated(&p);
        let Item::Text(t) = &a.text_items[0] else {
            panic!("missing initial")
        };
        assert_eq!(t.size, 29.0);
        assert_eq!(a.v_rel, VRel::Line);
        assert_eq!(p.style.before, 10.0);
        let units = flow.units(blocks, 200.0, false);
        assert_eq!(units.len(), 1);
        {
            let Unit::Para(p) = &units[0] else {
                panic!("paragraph expected")
            };
            assert_eq!(p.drop_cap_anchor, Some(0));
            assert!(matches!(
                p.src.as_ref().unwrap().inlines[0],
                Inline::Anchor(_)
            ));
        }
    });
}

#[test]
fn drop_cap_unsupported_pairs_keep_original_flow() {
    with_flow(
        &fixture(
            r#"w:dropCap="drop" w:lines="2""#,
            r#"<w:framePr w:w="1000"/>"#,
        ),
        |flow, blocks| {
            assert!(flow.drop_cap_pair(&blocks[0], &blocks[1], 200.0).is_none());
            assert_eq!(flow.units(&blocks[..1], 200.0, false).len(), 1);
            assert_eq!(flow.units(blocks, 200.0, true).len(), 2);
        },
    );
}

#[test]
fn drop_cap_mixed_height_uses_the_wrapped_line_span() {
    let body = fixture(r#"w:dropCap="drop" w:lines="1" w:hSpace="100""#, "")
        .replace(r#"w:line="240" w:lineRule="exact""#, r#"w:line="240" w:lineRule="auto""#)
        .replace("first</w:t><w:br/><w:t>second</w:t><w:br/><w:t>third</w:t><w:br/><w:t>fourth", "small small small small </w:t></w:r><w:r><w:rPr><w:sz w:val=\"48\"/></w:rPr><w:t>LARGE</w:t></w:r><w:r><w:rPr><w:sz w:val=\"20\"/></w:rPr><w:t> next words next words next words");
    with_flow(&body, |flow, blocks| {
        let following = para_of(&blocks[1]).unwrap();
        let wide = layout_paragraph(following, 1000.0, flow.env);
        let large_end = wide[0]
            .items
            .iter()
            .find_map(|item| match item {
                Item::Text(t) if t.text.contains("LARGE") => Some(t.transform.tx as f64 + t.width),
                _ => None,
            })
            .expect("large run fits the measurement line");
        let wide_pair = flow.drop_cap_pair(&blocks[0], &blocks[1], 1000.0).unwrap();
        let exclusion = generated(&wide_pair).pic.w + 5.0;
        // Shape the boundary with the available font so the cap moves LARGE
        // from the first line to the second on each platform.
        let width = large_end + exclusion / 4.0;
        let p = flow.drop_cap_pair(&blocks[0], &blocks[1], width).unwrap();
        let a = generated(&p);
        let wrapped = crate::office::docx::text::layout_paragraph_wrapped(
            &p,
            width,
            flow.env,
            &crate::office::docx::text::WrapIn {
                contour: Vec::new(),
                rects: vec![(0.0, a.pic.h, -5.0, a.pic.w + 5.0)],
                est_h: 12.0,
                min_gap: 18.0,
            },
        );
        assert!(
            (a.pic.h
                - wrapped
                    .iter()
                    .take(1)
                    .map(|l| l.height + l.skip)
                    .sum::<f64>())
            .abs()
                < 0.01
        );
        let raw = layout_paragraph(following, width, flow.env);
        assert!(raw[0].text.contains("LARGE"));
        assert!(!wrapped[0].text.contains("LARGE"));
        assert!(wrapped[1].text.contains("LARGE"));
        assert!((raw[0].height + raw[0].skip - wrapped[0].height - wrapped[0].skip).abs() > 0.01);
        assert_ne!(
            raw.iter().take(2).map(|l| l.height).collect::<Vec<_>>(),
            wrapped.iter().take(2).map(|l| l.height).collect::<Vec<_>>()
        );
    });
}

#[test]
fn drop_cap_reads_before_body_and_follows_page_break() {
    let body = format!(
        "{}{}",
        crate::office::docx::testutil::para("preceding"),
        fixture(r#"w:dropCap="drop" w:lines="2""#, "")
    )
    .replace(
        "<w:pPr><w:framePr",
        r#"<w:pPr><w:pageBreakBefore/><w:spacing w:before="800" w:after="800"/><w:framePr"#,
    );
    let pages = crate::office::docx::testutil::pages(docx(&body, None, None));
    assert_eq!(pages.len(), 2);
    let mut runs: Vec<_> = pages[1]
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Text(t) => Some(t),
            _ => None,
        })
        .collect();
    runs.sort_by_key(|r| r.paragraph);
    assert_eq!(
        runs.iter().map(|r| r.text.as_str()).collect::<String>(),
        "Tfirstsecondthirdfourth"
    );
    {
        let cap = runs.iter().find(|r| r.text == "T").unwrap();
        let first = runs.iter().find(|r| r.text == "first").unwrap();
        let third = runs.iter().find(|r| r.text == "third").unwrap();
        assert!(first.transform.tx > third.transform.tx);
        assert_eq!(cap.size, 29.0);
        assert!(cap.transform.ty > first.transform.ty);
        assert!(cap.transform.ty - first.transform.ty < 20.0);
    }
}

#[test]
fn drop_cap_typed_defaults_and_style_merge_preserve_mode() {
    with_flow(
        &fixture(r#"w:dropCap="margin" w:lines="3" w:hSpace="120""#, ""),
        |_, blocks| {
            let p = para_of(&blocks[0]).unwrap();
            let base = p.style.frame.as_ref().unwrap();
            let override_node = crate::office::pptx::xml::XNode {
                name: "w:framePr".into(),
                attrs: vec![("lines".into(), "2".into())],
                children: Vec::new(),
                text: String::new(),
            };
            let merged = crate::office::docx::props::Frame::parse(&override_node).over(base);
            assert_eq!(merged.drop_cap, Some(DropCap::Margin));
            assert_eq!(merged.drop_lines, 2);
            assert_eq!(merged.h_space, 6.0);
            let mut plain = crate::office::docx::props::Frame::parse(&override_node);
            let mut other = plain.clone();
            other.drop_lines = 7;
            assert_eq!(plain, other);
            plain.drop_cap = Some(DropCap::Drop);
            other.drop_cap = Some(DropCap::Drop);
            assert_ne!(plain, other);
        },
    );
}

#[test]
fn drop_cap_spacing_and_border_padding_keep_two_excluded_lines() {
    let anchor = r#"<w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:positionH relativeFrom="column"><wp:posOffset>5080000</wp:posOffset></wp:positionH><wp:positionV relativeFrom="line"><wp:posOffset>63500</wp:posOffset></wp:positionV><wp:extent cx="127000" cy="127000"/><wp:wrapSquare wrapText="bothSides"/></wp:anchor></w:drawing>"#;
    let body = format!("{}{}", crate::office::docx::testutil::para("preceding"), fixture(r#"w:dropCap="drop" w:lines="2""#, r#"<w:spacing w:before="200" w:line="240" w:lineRule="exact"/><w:pBdr><w:top w:val="single" w:sz="8" w:space="2"/></w:pBdr>"#))
        .replace("<w:t>first</w:t>", &format!("{anchor}<w:t>first</w:t>"));
    with_flow(&body, |flow, blocks| {
        let p = flow.drop_cap_pair(&blocks[1], &blocks[2], 468.0).unwrap();
        let anchors: Vec<_> = p
            .inlines
            .iter()
            .filter_map(|i| match i {
                Inline::Anchor(a) => Some(a),
                _ => None,
            })
            .collect();
        assert_eq!(anchors.len(), 2);
        assert_eq!(anchors[1].x, Ok(400.0));
        assert_eq!(anchors[1].y, Ok(5.0));
        assert_eq!(anchors[1].v_rel, VRel::Line);
    });
    let pages = crate::office::docx::testutil::pages(docx(&body, None, None));
    let x = |text: &str| {
        pages[0]
            .items
            .iter()
            .find_map(|i| match i {
                Item::Text(t) if t.text == text => Some(t.transform.tx),
                _ => None,
            })
            .unwrap()
    };
    {
        assert_eq!(x("first"), x("second"));
        assert!(x("second") > x("third"));
    }
}

#[test]
fn drop_cap_rtl_and_header_contexts_keep_original_flow() {
    with_flow(
        &fixture(r#"w:dropCap="drop" w:lines="2""#, "<w:bidi/>"),
        |flow, blocks| {
            assert!(flow.drop_cap_pair(&blocks[0], &blocks[1], 200.0).is_none());
            assert_eq!(flow.units(blocks, 200.0, false).len(), 2);
        },
    );
    with_flow(
        &fixture(r#"w:dropCap="drop" w:lines="2""#, ""),
        |flow, blocks| {
            flow.env.in_header.set(true);
            assert_eq!(flow.units(blocks, 200.0, false).len(), 2);
        },
    );
}
