use super::*;

fn linked() -> Fixture {
    let mut f = inherited::hierarchy(&[0, 1, 1, 0, 1]);
    f.fonts.fonts[0].charset = 0;
    f.fonts.fonts[0].name = "Latin font".into();
    f.styles.styles.clear();
    for index in 0..3 {
        f.definitions.lists[0].style_links[index] = index as u16;
        f.styles.styles.push(Some(Style {
            index,
            builtin_id: index as u16,
            kind: 1,
            base_style: None,
            next_style: index,
            linked_style: None,
            revision_marked: false,
            name: "List style".into(),
            raw_flags: 0,
            properties: vec![
                StyleProperties {
                    kind: "paragraph",
                    bytes: vec![index as u8, 0, 0x0b, 0x46, 1, 0, 0x0a, 0x26, index as u8],
                },
                StyleProperties {
                    kind: "character",
                    bytes: vec![0x4f, 0x4a, 0, 0, 0x6d, 0x48, 9, 4],
                },
            ],
            raw_revision: vec![],
        }));
    }
    for p in &mut f.paragraphs.spans {
        p.style_index = Some(*p.raw_modifiers.last().unwrap() as u16);
    }
    f
}

#[test]
fn same_resolved_paragraph_style_supports_multilevel_numeric_labels() {
    let f = linked();
    assert_eq!(
        inherited::labels(&f),
        ["1.\t", "1.1.\t", "1.2.\t", "2.\t", "2.1.\t"]
    );
}

#[test]
fn final_level_font_overrides_direct_mark_but_unresolved_conflict_is_rejected() {
    let mut f = linked();
    f.formatting.character_spans[0].properties =
        decode_character_modifiers(&[0x4f, 0x4a, 1, 0]).unwrap();
    assert!(inherited::labels(&f).is_empty());
    for level in &mut f.definitions.lists[0].levels {
        level.raw_chpx = vec![0x4f, 0x4a, 0, 0];
    }
    assert_eq!(inherited::labels(&f).len(), 5);
}

#[test]
fn linked_style_resets_case_symbolic_fonts_and_unknown_groups_stay_guarded() {
    for bytes in [
        vec![0x33, 0x2a, 0],
        vec![0x30, 0x4a, 0, 0],
        vec![0x3b, 0x08, 1],
        vec![0x3c, 0x08, 1],
    ] {
        let mut f = linked();
        f.definitions.lists[0].levels[1].raw_chpx = bytes;
        assert!(inherited::labels(&f).is_empty());
    }
    let mut f = linked();
    f.fonts.fonts[0].charset = 2;
    assert!(inherited::labels(&f).is_empty());
    let mut f = linked();
    f.styles.styles[1]
        .as_mut()
        .unwrap()
        .properties
        .push(StyleProperties {
            kind: "unexpected",
            bytes: vec![],
        });
    assert!(inherited::labels(&f).is_empty());
}

#[test]
fn conflicting_language_different_style_and_changed_membership_are_guarded() {
    let mut f = linked();
    f.formatting.character_spans[0].properties =
        decode_character_modifiers(&[0x6d, 0x48, 0x11, 0x04]).unwrap();
    assert!(inherited::labels(&f).is_empty());
    let mut f = linked();
    f.definitions.lists[0].style_links[1] = 0;
    assert!(inherited::labels(&f).is_empty());
    let mut f = linked();
    f.styles.styles[1].as_mut().unwrap().properties[0].bytes[4] = 3;
    assert!(inherited::labels(&f).is_empty());
}

#[test]
fn linked_style_inheritance_is_resolved_once_and_revision_groups_are_rejected() {
    let mut f = linked();
    f.styles.styles[1].as_mut().unwrap().base_style = Some(0);
    f.styles.styles[1].as_mut().unwrap().properties[1].bytes = vec![0x35, 0x08, 129];
    assert_eq!(inherited::labels(&f).len(), 5);
    f.styles.styles[1].as_mut().unwrap().raw_revision = vec![0];
    assert!(f.run().is_err());
}

#[test]
fn active_default_character_case_is_not_lost_under_paragraph_style() {
    let mut f = linked();
    f.styles.styles.resize_with(11, || None);
    f.styles.styles[10] = Some(Style {
        index: 10,
        builtin_id: 65,
        kind: 2,
        base_style: None,
        next_style: 10,
        linked_style: None,
        revision_marked: false,
        name: "Default font".into(),
        raw_flags: 0,
        properties: vec![StyleProperties {
            kind: "character",
            bytes: vec![0x3b, 0x08, 1],
        }],
        raw_revision: vec![],
    });
    assert!(inherited::labels(&f).is_empty());
}

#[test]
fn linked_style_requires_empty_default_and_zero_piece_modifiers() {
    let mut f = linked();
    f.formatting.pieces[0].raw_prm = 1;
    assert!(matches!(
        f.run(),
        Err(Error::Unsupported("list piece properties"))
    ));
    let mut f = linked();
    f.styles.styles.resize_with(11, || None);
    f.styles.styles[10] = Some(Style {
        index: 10,
        builtin_id: 65,
        kind: 2,
        base_style: None,
        next_style: 10,
        linked_style: None,
        revision_marked: false,
        name: "Default font".into(),
        raw_flags: 0,
        properties: vec![StyleProperties {
            kind: "character",
            bytes: vec![0x4f, 0x4a, 0, 0],
        }],
        raw_revision: vec![],
    });
    assert!(inherited::labels(&f).is_empty());
}

#[test]
fn valid_rgb_color_preserves_numbering_but_visibility_and_malformed_color_do_not() {
    for auto in [0, 255] {
        let mut f = linked();
        f.formatting.character_spans[0].properties =
            decode_character_modifiers(&[0x70, 0x68, 255, 0, 0, auto]).unwrap();
        assert_eq!(inherited::labels(&f).len(), 5);
        f.formatting.character_spans[0]
            .properties
            .extend(decode_character_modifiers(&[0x3c, 0x08, 1]).unwrap());
        assert!(inherited::labels(&f).is_empty());
    }
    for operand in [
        vec![],
        vec![255, 0, 0],
        vec![255, 0, 0, 1],
        vec![255, 0, 0, 0, 0],
    ] {
        assert!(!visible_modifier(&PropertyModifier {
            sprm: 0x6870,
            operand
        }));
    }
    let mut f = linked();
    f.definitions.lists[0].levels[1].raw_chpx = vec![0x70, 0x68, 255, 0, 0, 1];
    assert!(inherited::labels(&f).is_empty());
}

#[test]
fn linked_decimal_zero_keeps_font_and_case_precedence_guards() {
    let mut f = linked();
    f.definitions.lists[0].levels[1].number_format = 0x16;
    assert_eq!(
        inherited::labels(&f),
        ["1.\t", "1.01.\t", "1.02.\t", "2.\t", "2.01.\t"]
    );
    f.formatting.character_spans[0].properties =
        decode_character_modifiers(&[0x4f, 0x4a, 1, 0]).unwrap();
    assert!(inherited::labels(&f).is_empty());
    f.formatting.character_spans[0].properties.clear();
    f.styles.styles[1].as_mut().unwrap().properties[1]
        .bytes
        .extend([0x3b, 0x08, 1]);
    assert!(inherited::labels(&f).is_empty());
}

#[test]
fn differing_latin_faces_preserve_numbers_but_symbolic_or_missing_faces_block_family() {
    let mut f = linked();
    let first = &f.fonts.fonts[0];
    let second = crate::office::doc::symbols::Font {
        index: 1,
        name: "Other Latin font".into(),
        alternate_name: None,
        charset: 0,
        family: first.family,
        weight: first.weight,
        panose: first.panose.clone(),
        signature: first.signature.clone(),
    };
    f.fonts.fonts.push(second);
    f.formatting.character_spans[0].properties =
        decode_character_modifiers(&[0x4f, 0x4a, 1, 0]).unwrap();
    assert_eq!(
        inherited::labels(&f),
        ["1.\t", "1.1.\t", "1.2.\t", "2.\t", "2.1.\t"]
    );
    f.fonts.fonts[1].charset = 2;
    assert!(inherited::labels(&f).is_empty());
    f.fonts.fonts[1].charset = 0;
    f.fonts.fonts[1].index = 2;
    assert!(inherited::labels(&f).is_empty());
    f.fonts.fonts.pop();
    assert!(inherited::labels(&f).is_empty());
}

#[test]
fn linked_missing_face_uses_source_default_only_when_document_preserves_it() {
    let mut f = linked();
    f.font_defaults_preserved = Some(true);
    f.styles.raw_header = vec![0; 18];
    for style in f.styles.styles.iter_mut().flatten() {
        style.properties[1].bytes = vec![0x6d, 0x48, 9, 4];
    }
    assert_eq!(
        inherited::labels(&f),
        ["1.\t", "1.1.\t", "1.2.\t", "2.\t", "2.1.\t"]
    );
    f.font_defaults_preserved = Some(false);
    assert!(inherited::labels(&f).is_empty());
    f.font_defaults_preserved = None;
    assert!(inherited::labels(&f).is_empty());
    f.font_defaults_preserved = Some(true);
    f.styles.raw_header[12] = 1;
    assert!(inherited::labels(&f).is_empty());
    f.styles.raw_header[12] = 0;
    f.fonts.fonts[0].charset = 2;
    assert!(inherited::labels(&f).is_empty());
    f.fonts.fonts[0].charset = 0;
    f.styles.raw_header.truncate(13);
    assert!(inherited::labels(&f).is_empty());
}
