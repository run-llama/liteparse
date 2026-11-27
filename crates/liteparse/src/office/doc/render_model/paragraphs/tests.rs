use super::*;

pub(super) fn paragraph() -> ParagraphProperties {
    ParagraphProperties {
        justification: 0,
        font_alignment: None,
        left_twips: 0,
        right_twips: 0,
        first_twips: 0,
        before_twips: 0,
        after_twips: 0,
        keep_next: false,
        keep_lines: false,
        page_break_before: false,
        widow_control: true,
        outline_level: 9,
        line_multiple_240ths: 240,
        line_spacing: None,
        tabs: vec![],
        applied: Vec::new(),
    }
}

fn property(sprm: u16, operand: &[u8]) -> PropertyModifier {
    PropertyModifier {
        sprm,
        operand: operand.to_vec(),
    }
}

#[test]
fn save_session_identifier_preserves_layout_and_raw_evidence() {
    let mut result = paragraph();
    let original = result.clone();
    let id = property(0x6467, &[0xce, 0x74, 0x56, 0]);
    apply(&mut result, &id, 0).unwrap();
    assert_eq!(result.applied, vec![id]);
    result.applied.clear();
    assert_eq!(result, original);
    for bytes in [&[][..], &[0, 1, 2][..], &[0; 5][..]] {
        assert!(apply(&mut result, &property(0x6467, bytes), 0).is_err());
    }
}

#[test]
fn decoded_distances_preserve_twips_and_ordered_alias_overrides() {
    let mut bytes = Vec::new();
    for (sprm, value) in [
        (0x840f_u16, 720_i16),
        (0x845e, 1440),
        (0x840e, 360),
        (0x8411, -360),
        (0xa413, 120),
        (0xa414, 240),
    ] {
        bytes.extend(sprm.to_le_bytes());
        bytes.extend(value.to_le_bytes());
    }
    let properties = decode(&bytes).unwrap();
    let mut result = paragraph();
    for p in &properties {
        apply(&mut result, p, 0).unwrap();
    }
    assert_eq!(result.left_twips, 1440);
    assert_eq!(result.right_twips, 360);
    assert_eq!(result.first_twips, -360);
    assert_eq!((result.before_twips, result.after_twips), (120, 240));
    assert_eq!(result.applied, properties);
}

#[test]
fn invalid_bool8_does_not_change_paragraph_or_record_applied_property() {
    for sprm in [0x2405, 0x2406, 0x2407, 0x2431, 0x2437, 0x2448] {
        let mut result = paragraph();
        for operand in [&[][..], &[2], &[128], &[1, 0]] {
            let before = result.clone();
            assert!(matches!(
                apply(&mut result, &property(sprm, operand), 0),
                Err(Error::Invalid(_))
            ));
            assert_eq!(result, before);
        }
        apply(&mut result, &property(sprm, &[1]), 0).unwrap();
        apply(&mut result, &property(sprm, &[0]), 0).unwrap();
    }
}

#[test]
fn table_membership_requires_supported_layout() {
    for p in [
        property(0x2416, &[1]),
        property(0x2417, &[1]),
        property(0x244b, &[1]),
        property(0x244c, &[1]),
        property(0x6649, &1_u32.to_le_bytes()),
    ] {
        let mut result = paragraph();
        assert!(matches!(
            apply(&mut result, &p, 0),
            Err(Error::Unsupported(_))
        ));
        assert_eq!(result, paragraph());
    }
    let mut result = paragraph();
    for p in [
        property(0x2416, &[0]),
        property(0x6649, &[0; 4]),
        property(0x6412, &[240, 0, 1, 0]),
    ] {
        apply(&mut result, &p, 0).unwrap();
    }
}

#[test]
fn truncated_modifiers_and_nonparagraph_properties_fail_decoding() {
    for bytes in [
        &[0x0f][..],
        &[0x0f, 0x84, 0],
        &[0x35, 0x08, 1],
        &[0x12, 0x64, 240, 0, 1],
    ] {
        assert!(decode(bytes).is_err());
    }
}

#[test]
fn distance_limits_and_heading_outline_ownership_are_explicit() {
    for (sprm, value) in [
        (0x845e, 31681_u16),
        (0x845e, (-31681_i16) as u16),
        (0xa413, 31681),
    ] {
        assert!(apply(&mut paragraph(), &property(sprm, &value.to_le_bytes()), 0).is_err());
    }
    let mut heading = paragraph();
    heading.outline_level = 1;
    apply(&mut heading, &property(0x2640, &[7]), 2).unwrap();
    assert_eq!(heading.outline_level, 1);
    let mut ordinary = paragraph();
    apply(&mut ordinary, &property(0x2640, &[7]), 0).unwrap();
    assert_eq!(ordinary.outline_level, 7);
    assert!(apply(&mut ordinary, &property(0x2640, &[10]), 0).is_err());
}

#[test]
fn custom_tabs_apply_ordered_addition_and_twenty_five_twip_deletion() {
    let added = tabs(&[], &[8, 0, 2, 208, 2, 160, 5, 0, 2]).unwrap();
    assert_eq!(
        added,
        vec![
            TabStop {
                position_twips: 720,
                alignment: 0
            },
            TabStop {
                position_twips: 1440,
                alignment: 2
            }
        ]
    );
    let removed = tabs(&added, &[4, 1, 233, 2, 0]).unwrap();
    assert_eq!(
        removed,
        vec![TabStop {
            position_twips: 1440,
            alignment: 2
        }]
    );
    assert_eq!(tabs(&added, &[4, 1, 234, 2, 0]).unwrap(), added);
    assert!(tabs(&[], &[5, 0, 1, 208, 2, 8]).is_err());
    assert!(tabs(&[], &[5, 0, 1, 208, 2, 4]).is_err());
    assert!(tabs(&[], &[5, 0, 2, 208, 2, 0]).is_err());
    assert!(tabs(&[], &[8, 0, 2, 160, 5, 208, 2, 0, 0]).is_err());
}

#[test]
fn line_spacing_preserves_ordered_modes_and_validates_source_bounds() {
    let mut result = paragraph();
    for value in [15_u16, 120, 240, 360, 480, 31680] {
        let mut bytes = value.to_le_bytes().to_vec();
        bytes.extend([1, 0]);
        apply(&mut result, &property(0x6412, &bytes), 0).unwrap();
        assert_eq!(result.line_multiple_240ths, value);
    }
    for bytes in [[0, 0, 1, 0], [240, 0, 2, 0], [0xff, 0x7f, 1, 0]] {
        assert!(apply(&mut result, &property(0x6412, &bytes), 0).is_err());
    }
    for flag in [0_u8, 1] {
        apply(&mut result, &property(0x6412, &[0xac, 0xfe, flag, 0]), 0).unwrap();
        assert_eq!(result.line_spacing, Some(LineSpacing::ExactTwips(340)));
    }
    apply(&mut result, &property(0x6412, &[240, 0, 0, 0]), 0).unwrap();
    assert_eq!(result.line_spacing, Some(LineSpacing::MinimumTwips(240)));
    apply(&mut result, &property(0x6412, &[0, 0, 0, 0]), 0).unwrap();
    assert_eq!(result.line_spacing, Some(LineSpacing::MinimumTwips(0)));
    apply(&mut result, &property(0x6412, &[240, 0, 1, 0]), 0).unwrap();
    assert_eq!(result.line_spacing, None);
    assert!(apply(&mut result, &property(0x6412, &[240, 0, 1]), 0).is_err());
}

#[test]
fn cumulative_tab_operations_retain_source_order_and_decimal_stays_unresolved() {
    let mut result = paragraph();
    let add = property(0xc615, &[5, 0, 1, 0xd0, 2, 2]);
    let remove = property(0xc60d, &[4, 1, 0xd0, 2, 0]);
    apply(&mut result, &add, 0).unwrap();
    apply(&mut result, &remove, 0).unwrap();
    assert!(result.tabs.is_empty());
    apply(&mut result, &remove, 0).unwrap();
    apply(&mut result, &add, 0).unwrap();
    assert_eq!(
        result.tabs,
        vec![TabStop {
            position_twips: 720,
            alignment: 2
        }]
    );
    apply(&mut result, &property(0xc60d, &[5, 0, 1, 0xa0, 5, 0]), 0).unwrap();
    assert_eq!(result.tabs.len(), 2);
    assert!(apply(&mut result, &property(0xc615, &[5, 0, 1, 0xd0, 2, 3]), 0).is_err());
    assert!(apply(&mut result, &property(0xc615, &[6, 1, 0, 0, 25, 0, 0]), 0).is_err());
}

#[test]
fn style_tab_sequence_survives_scalar_deduplication() {
    use crate::office::doc::styles::{Style, StyleProperties};
    let add = [0x0d, 0xc6, 5, 0, 1, 0xd0, 2, 0];
    let add_second = [0x0d, 0xc6, 5, 0, 1, 0xa0, 5, 2];
    let make = |operations: &[u8]| Stylesheet {
        schema_version: 1,
        status: "fixture",
        std_base_bytes: 10,
        raw_header: vec![0; 20],
        unresolved: ["fixture"; 3],
        styles: vec![Some(Style {
            index: 0,
            builtin_id: 0,
            kind: 1,
            base_style: None,
            next_style: 0,
            linked_style: None,
            revision_marked: false,
            name: "fixture".into(),
            raw_flags: 0,
            raw_revision: vec![],
            properties: vec![StyleProperties {
                kind: "paragraph",
                bytes: [&[0, 0][..], operations].concat(),
            }],
        })],
    };
    let styles = make(&[&add[..], &add_second[..]].concat());
    let result = resolve(&styles, 0, &[]).unwrap();
    assert_eq!(
        result
            .tabs
            .iter()
            .map(|t| t.position_twips)
            .collect::<Vec<_>>(),
        vec![720, 1440]
    );
    let add_close = [0x15, 0xc6, 5, 0, 1, 0xd0, 2, 0];
    let delete = [0x0d, 0xc6, 4, 1, 0xd0, 2, 0];
    assert!(
        resolve(&make(&[&add_close[..], &delete[..]].concat()), 0, &[])
            .unwrap()
            .tabs
            .is_empty()
    );
    assert_eq!(
        resolve(&make(&[&delete[..], &add_close[..]].concat()), 0, &[])
            .unwrap()
            .tabs
            .len(),
        1
    );
}

#[test]
fn typed_list_tab_retains_kind_and_rejects_unsupported_leader() {
    let result = tabs(&[], &[5, 0, 1, 0xd0, 2, 6]).unwrap();
    assert_eq!(
        result,
        vec![TabStop {
            position_twips: 720,
            alignment: 6
        }]
    );
    assert!(tabs(&[], &[5, 0, 1, 0xd0, 2, 14]).is_err());
    assert!(tabs(&[], &[5, 0, 1, 0xd0, 2, 7]).is_err());
}

#[test]
fn numbering_history_is_retained_without_changing_current_layout() {
    let mut operand = vec![0; 129];
    operand[0] = 128;
    let p = property(0xc645, &operand);
    let mut actual = paragraph();
    let expected = actual.clone();
    apply(&mut actual, &p, 0).unwrap();
    assert_eq!(actual.applied, vec![p]);
    actual.applied.clear();
    assert_eq!(actual, expected);
    assert!(apply(&mut actual, &property(0xc645, &operand[..128]), 0).is_err());
}

#[test]
fn retained_partial_borders_reject_malformed_framing_and_unknown_styles() {
    let mut result = paragraph();
    let old = property(0x6424, &[24, 11, 0, 1]);
    let modern = property(0xc64e, &[8, 0, 0, 0, 255, 24, 12, 1, 0]);
    apply(&mut result, &old, 0).unwrap();
    apply(&mut result, &modern, 1).unwrap();
    assert_eq!(result.applied, vec![old, modern]);
    for (op, bytes) in [
        (0x6424, vec![24, 11, 0]),
        (0xc64e, vec![7, 0, 0, 0, 255, 24, 12, 1, 0]),
        (0xc64e, vec![8, 0, 0, 0, 15, 24, 12, 1, 0]),
        (0x6424, vec![24, 2, 0, 1]),
    ] {
        assert!(apply(&mut paragraph(), &property(op, &bytes), 0).is_err());
    }
}

#[test]
fn modern_border_unused_bits_are_retained_without_visual_interpretation() {
    let mut result = paragraph();
    let raw = property(0xc64e, &[8, 0, 0, 0, 255, 24, 11, 128, 255]);
    apply(&mut result, &raw, 0).unwrap();
    assert_eq!(result.applied, vec![raw]);
}

#[test]
fn partial_frames_preserve_raw_operands_and_reject_malformed_framing() {
    let mut result = paragraph();
    for p in [
        property(0x8418, &[248, 255]),
        property(0x8419, &[1, 0]),
        property(0x261b, &[96]),
        property(0x2423, &[2]),
    ] {
        apply(&mut result, &p, 0).unwrap();
        assert_eq!(result.applied.last(), Some(&p));
    }
    for p in [
        property(0x8418, &[255, 127]),
        property(0x8419, &[1]),
        property(0x261b, &[96, 0]),
        property(0x2423, &[255]),
    ] {
        assert!(apply(&mut paragraph(), &p, 0).is_err());
    }
}

#[test]
fn line_preferences_preserve_source_but_malformed_operands_reject() {
    for opcode in [0x2433, 0x2435] {
        let raw = property(opcode, &[0]);
        let mut result = paragraph();
        apply(&mut result, &raw, 0).unwrap();
        assert_eq!(result.applied, vec![raw]);
        for operand in [&[2][..], &[][..], &[0, 0][..]] {
            assert!(apply(&mut paragraph(), &property(opcode, operand), 0).is_err());
        }
    }
}

#[test]
fn character_first_indent_rejects_malformed_or_hanging_operands_without_partial_state() {
    let mut result = paragraph();
    for bytes in [
        vec![],
        vec![1],
        vec![1, 0, 0],
        (-1i16).to_le_bytes().to_vec(),
        31681i16.to_le_bytes().to_vec(),
    ] {
        let before = result.clone();
        assert!(apply(&mut result, &property(0x4457, &bytes), 0).is_err());
        assert_eq!(result, before);
    }
    apply(&mut result, &property(0x4457, &200i16.to_le_bytes()), 0).unwrap();
    apply(&mut result, &property(0x8411, &80i16.to_le_bytes()), 0).unwrap();
    assert_eq!(result.first_twips, 80);
    assert_eq!(result.applied[0], property(0x4457, &200i16.to_le_bytes()));
}

#[test]
fn numbering_insert_history_retains_raw_flag_and_rejects_non_boolean_values() {
    let mut current = paragraph();
    current.left_twips = 720;
    current.first_twips = -360;
    for flag in [1, 0] {
        apply(&mut current, &property(0x2443, &[flag]), 0).unwrap();
        assert_eq!((current.left_twips, current.first_twips), (720, -360));
    }
    assert_eq!(
        current
            .applied
            .iter()
            .map(|p| p.operand.clone())
            .collect::<Vec<_>>(),
        vec![vec![1], vec![0]]
    );
    for operand in [&[][..], &[2][..], &[1, 0][..]] {
        assert!(apply(&mut current, &property(0x2443, operand), 0).is_err());
    }
}

fn alignment_styles(base: &[u8], derived: &[u8]) -> Stylesheet {
    use crate::office::doc::styles::{Style, StyleProperties};
    Stylesheet {
        schema_version: 1,
        status: "fixture",
        std_base_bytes: 10,
        raw_header: vec![0; 20],
        unresolved: ["fixture"; 3],
        styles: [base, derived]
            .iter()
            .enumerate()
            .map(|(i, ops)| {
                Some(Style {
                    index: i,
                    builtin_id: 0,
                    kind: 1,
                    base_style: if i == 0 { None } else { Some(0) },
                    next_style: i,
                    linked_style: None,
                    revision_marked: false,
                    name: "fixture".into(),
                    raw_flags: 0,
                    raw_revision: vec![],
                    properties: vec![StyleProperties {
                        kind: "paragraph",
                        bytes: [&(i as u16).to_le_bytes()[..], ops].concat(),
                    }],
                })
            })
            .collect(),
    }
}

#[test]
fn font_alignment_inheritance_and_direct_reset_preserve_source_order() {
    let styles = alignment_styles(&[0x39, 0x44, 4, 0], &[0x39, 0x44, 2, 0]);
    let inherited = resolve(&styles, 1, &[]).unwrap();
    assert_eq!(inherited.font_alignment, Some(2));
    assert_eq!(
        inherited
            .applied
            .iter()
            .filter(|p| p.sprm == 0x4439)
            .map(|p| p.operand.clone())
            .collect::<Vec<_>>(),
        vec![vec![4, 0], vec![2, 0]]
    );
    assert_eq!(
        resolve(&styles, 1, &[property(0x4439, &[4, 0])])
            .unwrap()
            .font_alignment,
        Some(4)
    );
    let invalid = alignment_styles(&[0x39, 0x44, 5, 0], &[0x39, 0x44, 2, 0]);
    assert!(resolve(&invalid, 1, &[property(0x4439, &[2, 0])]).is_err());
}

#[test]
fn font_alignment_rejects_unknown_lengths_and_unimplemented_active_modes() {
    for bytes in [
        &[][..],
        &[2][..],
        &[2, 0, 0][..],
        &[5, 0][..],
        &[255, 255][..],
    ] {
        assert!(matches!(
            apply(&mut paragraph(), &property(0x4439, bytes), 0),
            Err(Error::Invalid(_))
        ));
    }
    for value in [0u16, 1, 3] {
        assert!(matches!(
            apply(&mut paragraph(), &property(0x4439, &value.to_le_bytes()), 0),
            Err(Error::Unsupported(_))
        ));
    }
}

#[test]
fn ignored_style_info_reserved_payload_does_not_change_layout() {
    for fill in [0, 0xff, 0x5a] {
        let mut operand = vec![fill; 17];
        operand[0] = 16;
        let mut result = paragraph();
        apply(&mut result, &property(0xc66c, &operand), 0).unwrap();
        assert_eq!(result.applied.len(), 1);
        result.applied.clear();
        assert_eq!(result, paragraph());
    }
    for operand in [vec![], vec![16; 16], vec![16; 18], vec![15; 17]] {
        assert!(apply(&mut paragraph(), &property(0xc66c, &operand), 0).is_err());
    }
    let styles = alignment_styles(&[0x6c, 0xc6, 0], &[]);
    assert!(resolve(&styles, 1, &[property(0xc66c, &[16; 17])]).is_err());
}
