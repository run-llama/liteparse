use super::*;

fn fixture() -> Fixture {
    let mut f = Fixture::new(&[2, 2]);
    f.styles.raw_header = vec![0; 18];
    f.font_defaults_preserved = Some(true);
    f.fonts.fonts[0].name = "Times New Roman".into();
    f.fonts.fonts[0].charset = 0;
    let level = &mut f.definitions.lists[1].levels[0];
    level.number_text_utf16 = vec![45];
    level.raw_chpx = vec![0x6f, 0x28, 0];
    f
}

#[test]
fn agreeing_source_fonts_restore_literal_ascii_bullets() {
    for scalar in [45, 43, 42] {
        let mut f = fixture();
        f.definitions.lists[1].levels[0].number_text_utf16 = vec![scalar];
        let report =
            project_lists_with_font_source(&transcribe(&f.source), &f.context(), Some(false))
                .unwrap();
        let label = char::from_u32(u32::from(scalar)).unwrap();
        assert_eq!(
            report.transcription.stories[0].text,
            format!("{label}\tItem\n{label}\tItem\n")
        );
        assert_eq!(report.label_sources.len(), 2);
        partition(&report.transcription.stories[0], 10);
    }
}

#[test]
fn body_only_font_conflict_blocks_the_whole_list_family() {
    let mut f = fixture();
    let mut body = f.formatting.character_spans[0].clone();
    body.end_cp = 6;
    let mut conflict = body.clone();
    conflict.start_cp = 6;
    conflict.end_cp = 7;
    conflict.properties = vec![PropertyModifier {
        sprm: 0x4a4f,
        operand: vec![1, 0],
    }];
    let mut tail = body.clone();
    tail.start_cp = 7;
    tail.end_cp = 10;
    f.formatting.character_spans = vec![body, conflict, tail];
    let report =
        project_lists_with_font_source(&transcribe(&f.source), &f.context(), Some(false)).unwrap();
    assert_eq!(report.transcription.stories[0].text, "Item\nItem\n");
    assert!(report.label_sources.is_empty());
}

#[test]
fn inherited_bullet_rejects_unknown_fonts_routing_resets_and_piece_properties() {
    for case in 0..13 {
        let mut f = fixture();
        match case {
            0 => f.styles.raw_header[14] = 1,
            1 => f.fonts.fonts[0].charset = 2,
            2 => f.fonts.fonts[0].alternate_name = Some("Symbol".into()),
            3 => f.formatting.character_spans[0]
                .properties
                .push(PropertyModifier {
                    sprm: 0x286f,
                    operand: vec![2],
                }),
            4 => f.formatting.character_spans[0]
                .properties
                .push(PropertyModifier {
                    sprm: 0x4a30,
                    operand: vec![0, 0],
                }),
            5 => f.formatting.pieces[0].raw_prm = 1,
            6 => f.styles.styles[0]
                .as_mut()
                .unwrap()
                .properties
                .push(StyleProperties {
                    kind: "character",
                    bytes: vec![0x4f, 0x4a, 1, 0],
                }),
            7 => f.definitions.lists[1].levels[0].number_text_utf16 = vec![0xf02d],
            8 => f.definitions.lists[1].style_links[0] = 0,
            9 => f.font_defaults_preserved = Some(false),
            10 => f.font_defaults_preserved = None,
            11 => {
                let mut other = fixture();
                let mut duplicate = other.fonts.fonts.remove(0);
                duplicate.index = 1;
                f.fonts.fonts.push(duplicate);
            }
            12 => {
                f.styles.raw_header.extend([1, 0]);
            }
            _ => unreachable!(),
        }
        let report =
            project_lists_with_font_source(&transcribe(&f.source), &f.context(), Some(false));
        if case == 5 {
            assert!(report.is_err());
        } else {
            assert!(report.unwrap().label_sources.is_empty(), "case {case}");
        }
    }
    for embedded in [None, Some(true)] {
        let f = fixture();
        let report =
            project_lists_with_font_source(&transcribe(&f.source), &f.context(), embedded).unwrap();
        assert!(report.label_sources.is_empty());
    }
}

#[test]
fn inherited_bullet_work_is_charged_across_source_layers() {
    let f = fixture();
    assert!(matches!(
        super::super::inherited_bullet::resolve(
            &f.context(),
            0,
            4,
            &f.definitions.lists[1].levels[0],
            &mut 0
        ),
        Err(Error::Limit(_))
    ));
}

#[test]
fn projection_limits_repeated_inherited_font_work_across_paragraphs() {
    let mut f = Fixture::new(&[2; 16]);
    let setup = fixture();
    f.styles = setup.styles;
    f.fonts = setup.fonts;
    f.definitions = setup.definitions;
    f.font_defaults_preserved = Some(true);
    f.styles.styles[0]
        .as_mut()
        .unwrap()
        .properties
        .push(StyleProperties {
            kind: "character",
            bytes: [0x35, 0x08, 0].repeat(200_000),
        });
    let result = project_lists_with_font_source(&transcribe(&f.source), &f.context(), Some(false));
    assert!(matches!(
        result,
        Err(Error::Limit("inherited bullet property work"))
    ));
}
