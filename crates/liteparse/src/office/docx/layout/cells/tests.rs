use super::*;
use crate::office::docx::model::{Block, Inline};

fn with_cell(wrap: Wrap, behind: bool, offset: f64, test: impl FnOnce(Vec<Slice>, f64)) {
    with_cell_modified(wrap, behind, offset, |_| {}, test)
}

fn with_cell_modified(
    wrap: Wrap,
    behind: bool,
    offset: f64,
    configure: impl FnOnce(&mut Anchored),
    test: impl FnOnce(Vec<Slice>, f64),
) {
    let body = r#"<w:p><w:pPr><w:widowControl w:val="0"/><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="254000" cy="203200"/><wp:wrapNone/></wp:anchor></w:drawing><w:t>OWNER</w:t><w:br/><w:t>AFTER</w:t><w:br/><w:t>TAIL</w:t></w:r></w:p>"#;
    let mut opened = crate::office::docx::extract::open(
        crate::office::docx::testutil::docx(body, None, None),
        &Default::default(),
    )
    .unwrap();
    let Block::Para(p) = &mut opened.sections[0].blocks[0] else {
        panic!()
    };
    let a = p
        .inlines
        .iter_mut()
        .find_map(|i| match i {
            Inline::Anchor(a) => Some(a),
            _ => None,
        })
        .unwrap();
    a.pic.part = Some("authored-test-image".into());
    a.wrap = wrap;
    a.behind = behind;
    a.y = Ok(offset);
    a.dist = [0.0, 3.0, 0.0, 2.0];
    configure(a);
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
        in_cell: std::cell::Cell::new(true),
        in_header: Default::default(),
        grid_in_table: false,
        no_leading: false,
        compat_mode: 15,
    };
    let units = Flow::new(&env, 15).units(&opened.sections[0].blocks, 150.0, true);
    let (slices, height) = stack(&units, 150.0, &env);
    test(slices, height);
}

#[test]
fn cell_top_bottom_wrap_keeps_picture_origin_and_reserves_line_clearance() {
    for behind in [false, true] {
        for offset in [0.0, 4.0] {
            with_cell(Wrap::TopAndBottom, behind, offset, |slices, height| {
                let anchors = slices.iter().flat_map(|s| &s.anchors).collect::<Vec<_>>();
                assert_eq!(anchors.len(), 1);
                let (a, _, ay) = anchors[0];
                assert_eq!(a.v_rel, VRel::Line);
                assert!((ay + a.y.as_ref().unwrap() - offset).abs() < 0.01);
                assert!(slices[0].h >= 12.0 + offset + 19.0);
                assert!(height >= 36.0 + offset + 19.0);
                let text = slices
                    .iter()
                    .flat_map(|s| &s.items)
                    .filter_map(|i| match i {
                        Item::Text(t) => Some(t.text.as_str()),
                        _ => None,
                    })
                    .collect::<String>();
                assert_eq!(text, "OWNERAFTERTAIL");
            });
        }
    }
}

#[test]
fn cell_square_uses_side_space_and_none_keeps_source_overlay() {
    for behind in [false, true] {
        for wrap in [Wrap::Square, Wrap::None] {
            with_cell(wrap, behind, 0.0, |slices, height| {
                assert!((height - 36.0).abs() < 0.01);
                let owner = slices
                    .iter()
                    .flat_map(|s| &s.items)
                    .find_map(|i| match i {
                        Item::Text(t) if t.text == "OWNER" => Some(t),
                        _ => None,
                    })
                    .unwrap();
                if wrap == Wrap::Square {
                    assert!(owner.transform.tx >= 22.0);
                } else {
                    assert!(owner.transform.tx < 1.0);
                }
                assert_eq!(slices.iter().map(|s| s.anchors.len()).sum::<usize>(), 1);
            });
        }
    }
}

#[test]
fn later_line_relative_anchor_follows_reflow_from_earlier_exclusion() {
    for wrap in [Wrap::Square, Wrap::TopAndBottom] {
        let anchor = r#"<w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="line"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="762000" cy="457200"/><wp:wrapNone/></wp:anchor></w:drawing>"#;
        let body = format!(
            r#"<w:p><w:pPr><w:widowControl w:val="0"/><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="exact"/></w:pPr><w:r>{anchor}<w:t>FIRST several words that require extra lines beside a picture and keep their source order</w:t><w:br/>{anchor}<w:t>SECOND</w:t></w:r></w:p>"#
        );
        let mut opened = crate::office::docx::extract::open(
            crate::office::docx::testutil::docx(&body, None, None),
            &Default::default(),
        )
        .unwrap();
        let Block::Para(p) = &mut opened.sections[0].blocks[0] else {
            panic!()
        };
        for (i, a) in p
            .inlines
            .iter_mut()
            .filter_map(|i| match i {
                Inline::Anchor(a) => Some(a),
                _ => None,
            })
            .enumerate()
        {
            a.pic.part = Some(format!("authored-image-{i}"));
            a.wrap = if i == 0 { wrap } else { Wrap::None };
            a.v_rel = VRel::Line;
        }
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
            in_cell: std::cell::Cell::new(true),
            in_header: Default::default(),
            grid_in_table: false,
            no_leading: false,
            compat_mode: 15,
        };
        let units = Flow::new(&env, 15).units(&opened.sections[0].blocks, 150.0, true);
        let (slices, _) = stack(&units, 150.0, &env);
        let mut count = 0;
        for slice in &slices {
            for (a, _, ay) in &slice.anchors {
                if a.pic.part.as_deref() == Some("authored-image-1") {
                    count += 1;
                    assert!(
                        (ay + a.y.as_ref().unwrap() - slice.y).abs() < 0.01,
                        "anchor={} owner={}",
                        ay + a.y.as_ref().unwrap(),
                        slice.y
                    );
                }
            }
        }
        assert_eq!(count, 1);
    }
}

#[test]
fn cell_picture_alignment_and_wrap_side_share_the_painted_origin() {
    for (alignment, expected) in [("center", 65.0), ("right", 130.0)] {
        with_cell_modified(
            Wrap::Square,
            false,
            0.0,
            |a| {
                a.x = Err(alignment.into());
            },
            |slices, _| {
                let a = &slices.iter().flat_map(|s| &s.anchors).next().unwrap().0;
                assert_eq!(a.x, Ok(expected));
                for run in slices[0].items.iter().filter_map(|i| match i {
                    Item::Text(t) => Some(t),
                    _ => None,
                }) {
                    let left = run.transform.tx as f64;
                    assert!(left + run.width <= expected + 0.01 || left >= expected + 22.0 - 0.01);
                }
            },
        );
    }
    for side in ["left", "right"] {
        with_cell_modified(
            Wrap::Square,
            false,
            0.0,
            |a| {
                a.x = Ok(65.0);
                a.wrap_side = side.into();
            },
            |slices, _| {
                let run = slices[0]
                    .items
                    .iter()
                    .find_map(|i| match i {
                        Item::Text(t) => Some(t),
                        _ => None,
                    })
                    .unwrap();
                if side == "right" {
                    assert!(run.transform.tx >= 87.0);
                } else {
                    assert!(run.transform.tx as f64 + run.width <= 65.01);
                }
            },
        );
    }
}

#[test]
fn inline_image_clip_preserves_source_transform_and_moves_with_slice() {
    let transform = tiny_skia::Transform::from_row(360.0, 0.0, 0.0, 20.0, 4.0, 12.0);
    let mut slices = vec![Slice {
        y: 0.0,
        h: 32.0,
        items: vec![Item::Image(crate::office::pptx::scene::ImageItem {
            image: crate::office::pptx::scene::ImageRef {
                part: "authored.png".into(),
                effects: vec![],
            },
            transform,
            crop: (0.0, 0.0, 0.0, 0.0),
            alpha: 1.0,
            clip: None,
            shape: None,
            alt_text: None,
        })],
        anchors: vec![],
        paragraph_offset: 0.0,
        widow: None,
        notes: vec![],
        nested_row: None,
        nested_table: None,
        paragraph_source: None,
    }];
    clip_inline_images(&mut slices, 80.0);
    assert_eq!(slices[0].items.len(), 3);
    let Item::Image(image) = &slices[0].items[1] else {
        panic!("source image")
    };
    assert_eq!(image.transform, transform);
    for item in &mut slices[0].items {
        translate_item(item, 30.0, 50.0);
    }
    let Item::PushClip(path) = &slices[0].items[0] else {
        panic!("clip")
    };
    assert_eq!(path.bounds().left(), 30.0);
    assert_eq!(path.bounds().right(), 110.0);
    let Item::Image(image) = &slices[0].items[1] else {
        panic!("source image")
    };
    assert_eq!(image.transform.sx, 360.0);
    assert_eq!(image.transform.tx, 34.0);
    assert!(matches!(slices[0].items[2], Item::PopClip));
}

#[test]
fn cell_image_clip_policy_survives_merged_source_remeasurement() {
    let xml = r#"<w:p><w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="4572000" cy="254000"/></wp:inline></w:drawing><w:t>OWNER</w:t></w:r></w:p>"#;
    let mut opened = crate::office::docx::extract::open(
        crate::office::docx::testutil::docx(xml, None, None),
        &Default::default(),
    )
    .unwrap();
    let Block::Para(p) = &mut opened.sections[0].blocks[0] else {
        panic!("paragraph")
    };
    let Inline::Picture(pic, _) = &mut p.inlines[0] else {
        panic!("picture")
    };
    pic.part = Some("authored.png".into());
    let fonts = crate::office::pptx::fonts::FontSystem::new(&[], true, &Default::default());
    let env = crate::office::docx::text::TextEnv {
        fonts: &fonts,
        default_tab: 36.0,
        fields: Default::default(),
        grid: None,
        char_grid: Default::default(),
        expand_shift_return: false,
        mark_in_last_line: true,
        html_spacing: true,
        grow_autofit: false,
        in_cell: std::cell::Cell::new(true),
        in_header: Default::default(),
        grid_in_table: false,
        no_leading: false,
        compat_mode: 15,
    };
    for clip in [false, true] {
        let flow = Flow::new(&env, 15);
        let mut units = flow.units(&opened.sections[0].blocks, 120.0, true);
        let Unit::Para(p) = &mut units[0] else {
            panic!("unit")
        };
        p.clip_inline_images = clip;
        let (mut slices, _) = note_stack(&units, 120.0, &env);
        for width in [80.0, 140.0, 80.0] {
            let (rebuilt, _) =
                super::super::note_source::remeasure_slices(&slices, width, &env).unwrap();
            let clips: Vec<_> = rebuilt
                .iter()
                .flat_map(|s| &s.items)
                .filter_map(|i| match i {
                    Item::PushClip(path) => Some(path.bounds()),
                    _ => None,
                })
                .collect();
            assert_eq!(clips.len(), usize::from(clip));
            if clip {
                assert_eq!(clips[0].right(), width as f32);
            }
            let images: Vec<_> = rebuilt
                .iter()
                .flat_map(|s| &s.items)
                .filter_map(|i| match i {
                    Item::Image(image) => Some(image),
                    _ => None,
                })
                .collect();
            assert_eq!(images.len(), 1);
            assert_eq!(images[0].transform.sx, 360.0);
            slices = rebuilt;
        }
    }
}
