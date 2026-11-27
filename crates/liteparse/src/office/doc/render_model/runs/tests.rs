use super::*;
use crate::office::doc::styles::{Style, StyleProperties};
use crate::office::doc::symbols::Font;

fn p(sprm: u16, operand: &[u8]) -> PropertyModifier {
    PropertyModifier {
        sprm,
        operand: operand.to_vec(),
    }
}
fn encoded(properties: &[PropertyModifier]) -> Vec<u8> {
    properties
        .iter()
        .flat_map(|p| {
            p.sprm
                .to_le_bytes()
                .into_iter()
                .chain(p.operand.iter().copied())
        })
        .collect()
}
fn style(
    index: usize,
    kind: u8,
    base_style: Option<usize>,
    properties: Vec<PropertyModifier>,
) -> Style {
    Style {
        index,
        builtin_id: if index == 10 { 65 } else { index as u16 },
        kind,
        base_style,
        next_style: 0,
        linked_style: None,
        revision_marked: false,
        name: "fixture".into(),
        raw_flags: 0,
        properties: vec![StyleProperties {
            kind: "character",
            bytes: encoded(&properties),
        }],
        raw_revision: vec![],
    }
}
fn fixture() -> (Stylesheet, FontTable) {
    let mut slots: Vec<_> = (0..15).map(|_| None).collect();
    slots[0] = Some(style(
        0,
        1,
        None,
        vec![p(0x4873, &[9, 4]), p(0x4a43, &[24, 0])],
    ));
    slots[1] = Some(style(
        1,
        1,
        Some(0),
        vec![p(0x0835, &[129]), p(0x4a4f, &[1, 0])],
    ));
    slots[10] = Some(style(10, 2, None, vec![]));
    let stylesheet = Stylesheet {
        schema_version: 1,
        status: "fixture",
        std_base_bytes: 10,
        raw_header: vec![0; 20],
        styles: slots,
        unresolved: ["fixture"; 3],
    };
    let fonts = FontTable {
        status: "fixture",
        fonts: (0..2)
            .map(|index| Font {
                index,
                name: format!("font{index}"),
                alternate_name: None,
                charset: 0,
                family: 0,
                weight: 400,
                panose: vec![],
                signature: vec![],
            })
            .collect(),
    };
    (stylesheet, fonts)
}

#[test]
fn disjoint_style_properties_resolve_and_direct_toggles_use_style_baseline() {
    let (styles, fonts) = fixture();
    let run = resolve(
        &styles,
        &fonts,
        1,
        &[p(0x0835, &[129]), p(0x4a43, &[22, 0])],
    )
    .unwrap();
    assert!(!run.bold);
    assert_eq!(run.size_half_points, 22);
    assert_eq!(run.fonts, [1, 0, 0]);
}

#[test]
fn derived_size_overrides_ancestor_and_inactive_complex_size_is_retained() {
    let (mut styles, fonts) = fixture();
    styles.styles[1].as_mut().unwrap().properties[0]
        .bytes
        .extend(encoded(&[p(0x4a43, &[28, 0])]));
    assert_eq!(
        resolve(&styles, &fonts, 1, &[]).unwrap().size_half_points,
        28
    );
    let (mut styles, fonts) = fixture();
    for (index, size) in [(0, 24), (1, 13)] {
        styles.styles[index].as_mut().unwrap().properties[0]
            .bytes
            .extend(encoded(&[p(0x4a61, &[size, 0])]));
    }
    let run = resolve(&styles, &fonts, 1, &[]).unwrap();
    assert_eq!(run.complex_property_evidence.len(), 2);
    assert_eq!(run.size_half_points, 24);
}

#[test]
fn aliases_remain_ordered_and_resets_cannot_be_overwritten_silently() {
    let (styles, fonts) = fixture();
    let run = resolve(
        &styles,
        &fonts,
        0,
        &[p(0x4873, &[12, 4]), p(0x486d, &[22, 4])],
    )
    .unwrap();
    assert_eq!(run.language, 0x416);
    for reset in [p(0x2a33, &[0]), p(0x286f, &[3]), p(0x0882, &[1])] {
        assert!(resolve(&styles, &fonts, 0, &[reset, p(0x4a43, &[24, 0])]).is_err());
    }
}

#[test]
fn invalid_font_size_and_nonempty_default_character_style_fail() {
    let (mut styles, fonts) = fixture();
    for invalid in [
        p(0x4a43, &[1, 0]),
        p(0x4a43, &[24]),
        p(0x4a4f, &[2, 0]),
        p(0x0835, &[2]),
    ] {
        assert!(resolve(&styles, &fonts, 0, &[invalid]).is_err());
    }
    styles.styles[10].as_mut().unwrap().properties[0].bytes = encoded(&[p(0x0835, &[1])]);
    assert!(resolve(&styles, &fonts, 0, &[]).is_err());
}

#[test]
fn inherited_relative_toggles_apply_per_style_then_direct_uses_final_style() {
    let (mut styles, fonts) = fixture();
    styles.styles[0].as_mut().unwrap().properties[0]
        .bytes
        .extend(encoded(&[p(0x0835, &[129])]));
    assert!(!resolve(&styles, &fonts, 1, &[]).unwrap().bold);
    assert!(
        resolve(&styles, &fonts, 1, &[p(0x0835, &[129])])
            .unwrap()
            .bold
    );
}

#[test]
fn save_ids_preserve_layout_and_source_kerning_threshold_is_typed() {
    let (mut styles, fonts) = fixture();
    let baseline = resolve(&styles, &fonts, 0, &[]).unwrap();
    for opcode in [0x6815, 0x6816, 0x6817] {
        let mut result = resolve(&styles, &fonts, 0, &[p(opcode, &[1, 2, 3, 4])]).unwrap();
        result.applied.pop();
        assert_eq!(result, baseline);
        assert!(resolve(&styles, &fonts, 0, &[p(opcode, &[1, 2, 3])]).is_err());
    }
    assert_eq!(
        resolve(&styles, &fonts, 0, &[p(0x484b, &[24, 0])])
            .unwrap()
            .kerning_threshold_half_points,
        24
    );
    styles.styles[0].as_mut().unwrap().properties[0]
        .bytes
        .extend(encoded(&[p(0x484b, &[24, 0])]));
    assert_eq!(
        resolve(&styles, &fonts, 0, &[])
            .unwrap()
            .kerning_threshold_half_points,
        24
    );
    assert!(resolve(&styles, &fonts, 0, &[p(0x484b, &[0xff, 0xff])]).is_err());
}

#[test]
fn linked_character_partner_does_not_apply_without_character_style_selection() {
    let (mut styles, fonts) = fixture();
    let baseline = resolve(&styles, &fonts, 0, &[]).unwrap();
    styles.styles[0].as_mut().unwrap().linked_style = Some(12);
    styles.styles[12] = Some(style(12, 2, None, vec![p(0x4a43, &[50, 0])]));
    assert_eq!(resolve(&styles, &fonts, 0, &[]).unwrap(), baseline);
    styles.styles[12].as_mut().unwrap().kind = 1;
    assert!(resolve(&styles, &fonts, 0, &[]).is_err());
    styles.styles[12] = None;
    assert!(resolve(&styles, &fonts, 0, &[]).is_err());
}

#[test]
fn ordered_palette_rgb_and_automatic_colors_keep_source_meaning() {
    let (styles, fonts) = fixture();
    assert_eq!(
        resolve(
            &styles,
            &fonts,
            0,
            &[p(0x2a42, &[1]), p(0x6870, &[24, 23, 23, 0])]
        )
        .unwrap()
        .color_rgb,
        Some([24, 23, 23])
    );
    assert_eq!(
        resolve(
            &styles,
            &fonts,
            0,
            &[p(0x6870, &[24, 23, 23, 0]), p(0x2a42, &[1])]
        )
        .unwrap()
        .color_rgb,
        Some([0, 0, 0])
    );
    assert_eq!(
        resolve(&styles, &fonts, 0, &[p(0x6870, &[24, 23, 23, 255])])
            .unwrap()
            .color_rgb,
        None
    );
    assert!(resolve(&styles, &fonts, 0, &[p(0x6870, &[0, 0, 0, 1])]).is_err());
    assert!(resolve(&styles, &fonts, 0, &[p(0x6870, &[0, 0, 0])]).is_err());
}

#[test]
fn complex_route_retains_its_own_font_size_and_inherited_toggle_state() {
    let (mut styles, fonts) = fixture();
    styles.styles[0].as_mut().unwrap().properties[0]
        .bytes
        .extend(encoded(&[
            p(0x085c, &[129]),
            p(0x4a5e, &[0, 0]),
            p(0x4a61, &[24, 0]),
        ]));
    styles.styles[1].as_mut().unwrap().properties[0]
        .bytes
        .extend(encoded(&[p(0x085c, &[129])]));
    let run = resolve(
        &styles,
        &fonts,
        1,
        &[p(0x286f, &[2]), p(0x085a, &[129]), p(0x4a61, &[36, 0])],
    )
    .unwrap();
    assert_eq!(run.hint, 2);
    assert!(run.bidi);
    assert!(!run.complex.bold);
    assert_eq!(run.complex.font, Some(0));
    assert_eq!(run.complex.size_half_points, Some(36));
    assert_eq!(run.fonts[0], 1);
    assert_eq!(run.size_half_points, 24);
}

#[test]
fn superscript_subscript_and_baseline_are_ordered_source_properties() {
    let (styles, fonts) = fixture();
    for value in 0..=2 {
        assert_eq!(
            resolve(&styles, &fonts, 0, &[p(0x2a48, &[value])])
                .unwrap()
                .vertical_alignment,
            value
        );
    }
    assert_eq!(
        resolve(&styles, &fonts, 0, &[p(0x2a48, &[1]), p(0x2a48, &[0])])
            .unwrap()
            .vertical_alignment,
        0
    );
    for operand in [&[][..], &[3][..], &[1, 0][..]] {
        assert!(resolve(&styles, &fonts, 0, &[p(0x2a48, operand)]).is_err());
    }
}

#[test]
fn plain_shading_has_real_background_and_nil_clears_it() {
    let (styles, fonts) = fixture();
    assert_eq!(
        resolve(&styles, &fonts, 0, &[p(0x4866, &[0, 1])])
            .unwrap()
            .shading_rgb,
        Some([255; 3])
    );
    assert_eq!(
        resolve(
            &styles,
            &fonts,
            0,
            &[p(0x4866, &[0, 1]), p(0x4866, &[255, 255])]
        )
        .unwrap()
        .shading_rgb,
        None
    );
    for bytes in [&[0][..], &[0, 4][..], &[31, 1][..]] {
        assert!(resolve(&styles, &fonts, 0, &[p(0x4866, bytes)]).is_err());
    }
}

#[test]
fn clear_shading_keeps_background_and_validates_both_colors() {
    let (styles, fonts) = fixture();
    let bytes = [10, 0, 0, 0, 255, 255, 255, 255, 0, 0, 0];
    assert_eq!(
        resolve(&styles, &fonts, 0, &[p(0xca71, &bytes)])
            .unwrap()
            .shading_rgb,
        Some([255; 3])
    );
    for position in [0, 4, 8, 9] {
        let mut invalid = bytes;
        invalid[position] = 2;
        assert!(resolve(&styles, &fonts, 0, &[p(0xca71, &invalid)]).is_err());
    }
}

#[test]
fn proofing_toggle_preserves_source_without_changing_visual_properties() {
    let (styles, fonts) = fixture();
    let baseline = resolve(&styles, &fonts, 0, &[]).unwrap();
    for value in [0, 1, 128, 129] {
        let property = p(0x0875, &[value]);
        let resolved = resolve(&styles, &fonts, 0, std::slice::from_ref(&property)).unwrap();
        assert_eq!(resolved.fonts, baseline.fonts);
        assert_eq!(resolved.size_half_points, baseline.size_half_points);
        assert_eq!(resolved.bold, baseline.bold);
        assert_eq!(resolved.italic, baseline.italic);
        assert_eq!(resolved.applied.last().unwrap().sprm, property.sprm);
        assert_eq!(resolved.applied.last().unwrap().operand, property.operand);
    }
    for operand in [&[][..], &[2][..], &[127][..], &[130][..], &[1, 0][..]] {
        assert!(resolve(&styles, &fonts, 0, &[p(0x0875, operand)]).is_err());
    }
}

#[test]
fn hidden_toggle_uses_ordered_style_baseline_and_strict_operands() {
    let (mut styles, fonts) = fixture();
    styles.styles[0].as_mut().unwrap().properties[0]
        .bytes
        .extend(encoded(&[p(0x083c, &[1])]));
    for (value, expected) in [(0, false), (1, true), (128, true), (129, false)] {
        let result = resolve(&styles, &fonts, 0, &[p(0x083c, &[value])]).unwrap();
        assert_eq!(result.hidden, expected);
        assert_eq!(result.applied.last().unwrap().operand, vec![value]);
    }
    for bytes in [vec![], vec![2], vec![1, 0]] {
        assert!(resolve(&styles, &fonts, 0, &[p(0x083c, &bytes)]).is_err());
    }
}

#[test]
fn manual_break_special_semantics_remain_unresolved() {
    let (styles, fonts) = fixture();
    for op in [0x0818, 0x2879] {
        for value in [0, 1, 2, 3, 128, 129] {
            assert!(resolve(&styles, &fonts, 0, &[p(op, &[value])]).is_err());
        }
    }
}

mod palette;

#[test]
fn single_underline_replays_style_selection_and_explicit_clearing() {
    let (mut styles, fonts) = fixture();
    styles.styles[2] = Some(style(2, 1, Some(0), vec![p(0x2a3e, &[1])]));
    styles.styles[11] = Some(style(11, 2, Some(10), vec![p(0x2a3e, &[0])]));
    assert!(resolve(&styles, &fonts, 2, &[]).unwrap().underline);
    assert!(
        !resolve(&styles, &fonts, 2, &[p(0x2a3e, &[0])])
            .unwrap()
            .underline
    );
    assert!(
        !resolve(&styles, &fonts, 2, &[p(0x4a30, &[11, 0])])
            .unwrap()
            .underline
    );
    assert!(
        resolve(&styles, &fonts, 2, &[p(0x4a30, &[11, 0]), p(0x2a3e, &[1])])
            .unwrap()
            .underline
    );
    for operand in [vec![], vec![0, 0], vec![2], vec![3], vec![255]] {
        assert!(resolve(&styles, &fonts, 0, &[p(0x2a3e, &operand)]).is_err());
    }
}

#[test]
fn selected_character_style_preserves_highlight_but_resets_scale() {
    let (styles, fonts) = fixture();
    let run = resolve(
        &styles,
        &fonts,
        0,
        &[
            p(0x2a0c, &[7]),
            p(0x4852, &99u16.to_le_bytes()),
            p(0x4a30, &10u16.to_le_bytes()),
        ],
    )
    .unwrap();
    assert_eq!(run.highlight_rgb, Some([255, 255, 0]));
    assert_eq!(run.scale_percent, 100);
    let reset = resolve(
        &styles,
        &fonts,
        0,
        &[
            p(0x2a0c, &[7]),
            p(0x2a0c, &[0]),
            p(0x4852, &600u16.to_le_bytes()),
        ],
    )
    .unwrap();
    assert_eq!(reset.highlight_rgb, None);
    assert_eq!(reset.scale_percent, 600);
    for property in [
        p(0x2a0c, &[17]),
        p(0x2a0c, &[]),
        p(0x4852, &[100]),
        p(0x4852, &0u16.to_le_bytes()),
        p(0x4852, &601u16.to_le_bytes()),
    ] {
        assert!(resolve(&styles, &fonts, 0, &[property]).is_err());
    }
}

#[test]
fn signed_character_spacing_rejects_bounds_and_resets_with_character_style() {
    let (styles, fonts) = fixture();
    let value = |n: i16| p(0x8840, &n.to_le_bytes());
    let run = resolve(&styles, &fonts, 0, &[value(-3)]).unwrap();
    assert_eq!(run.character_spacing_twips, -3);
    let reset = resolve(
        &styles,
        &fonts,
        0,
        &[value(-3), p(0x4a30, &10u16.to_le_bytes())],
    )
    .unwrap();
    assert_eq!(reset.character_spacing_twips, 0);
    for n in [-31680, 31680] {
        assert_eq!(
            resolve(&styles, &fonts, 0, &[value(n)])
                .unwrap()
                .character_spacing_twips,
            n
        );
    }
    for property in [value(-31681), value(31681), p(0x8840, &[0])] {
        assert!(resolve(&styles, &fonts, 0, &[property]).is_err());
    }
}

#[test]
fn east_asian_hint_changes_source_slot_without_resetting_run_metrics() {
    let (styles, fonts) = fixture();
    let run = resolve(&styles, &fonts, 0, &[p(0x286f, &[1]), p(0x4a43, &[30, 0])]).unwrap();
    assert_eq!(run.hint, 1);
    assert_eq!(run.size_half_points, 30);
    assert_eq!(font_slot('漢', run.hint), 1);
    assert_eq!(font_slot('é', run.hint), 1);
    assert_eq!(font_slot('A', run.hint), 0);
    assert_eq!(font_slot('\u{b}', run.hint), 2);
}

#[test]
fn east_asian_language_ids_preserve_other_script_qualification() {
    let (styles, fonts) = fixture();
    for language in [0x0804u16, 0x0404, 0x0411, 0x0412] {
        let run = resolve(&styles, &fonts, 0, &[p(0x4873, &language.to_le_bytes())]).unwrap();
        assert_eq!(run.language, language);
    }
    assert!(resolve(&styles, &fonts, 0, &[p(0x4873, &0x0401u16.to_le_bytes())]).is_err());
}

#[test]
fn greek_cyrillic_languages_retain_complex_script_guards() {
    let (styles, fonts) = fixture();
    for language in [0x0408u16, 0x0419, 0x042f] {
        let run = resolve(&styles, &fonts, 0, &[p(0x4873, &language.to_le_bytes())]).unwrap();
        assert_eq!(run.language, language);
    }
    for language in [0x0401u16, 0x041e, 0x0439] {
        assert!(resolve(&styles, &fonts, 0, &[p(0x4873, &language.to_le_bytes())]).is_err());
    }
}

#[test]
fn animation_preserves_text_metrics_and_rejects_unknown_or_malformed_effects() {
    let (styles, fonts) = fixture();
    let baseline = resolve(&styles, &fonts, 0, &[]).unwrap();
    for effect in 0..=6 {
        let result = resolve(&styles, &fonts, 0, &[p(0x2859, &[effect])]).unwrap();
        assert_eq!(result.fonts, baseline.fonts);
        assert_eq!(result.size_half_points, baseline.size_half_points);
        assert_eq!(result.applied.last(), Some(&p(0x2859, &[effect])));
    }
    for operand in [vec![], vec![7], vec![255], vec![5, 0]] {
        assert!(resolve(&styles, &fonts, 0, &[p(0x2859, &operand)]).is_err());
    }
}

#[test]
fn font_fallback_hints_retain_selected_face_and_reject_reserved_values() {
    let (styles, fonts) = fixture();
    let baseline = resolve(&styles, &fonts, 0, &[]).unwrap();
    for value in [0, 1, 2, 4] {
        let run = resolve(&styles, &fonts, 0, &[p(0x2a86, &[value])]).unwrap();
        assert_eq!(run.fonts, baseline.fonts);
        assert_eq!(run.size_half_points, baseline.size_half_points);
        assert_eq!(run.applied.last(), Some(&p(0x2a86, &[value])));
    }
    for value in [vec![], vec![3], vec![5], vec![255], vec![1, 0]] {
        assert!(resolve(&styles, &fonts, 0, &[p(0x2a86, &value)]).is_err());
    }
}

mod table_text;

mod symbols;

#[test]
fn small_caps_relative_toggle_replays_style_and_character_style_reset() {
    let (mut styles, fonts) = fixture();
    styles.styles[2] = Some(style(2, 1, Some(0), vec![p(0x083a, &[1])]));
    styles.styles[11] = Some(style(11, 2, Some(10), vec![p(0x083a, &[0])]));
    assert!(resolve(&styles, &fonts, 2, &[]).unwrap().small_caps);
    assert!(
        !resolve(&styles, &fonts, 2, &[p(0x083a, &[129])])
            .unwrap()
            .small_caps
    );
    assert!(
        resolve(&styles, &fonts, 2, &[p(0x083a, &[0]), p(0x083a, &[128])])
            .unwrap()
            .small_caps
    );
    assert!(
        !resolve(&styles, &fonts, 2, &[p(0x4a30, &[11, 0])])
            .unwrap()
            .small_caps
    );
    for bytes in [vec![], vec![2], vec![1, 0]] {
        assert!(resolve(&styles, &fonts, 0, &[p(0x083a, &bytes)]).is_err());
    }
}

#[test]
fn all_caps_resolves_relative_style_toggles_and_resets_independently() {
    let (mut styles, fonts) = fixture();
    styles.styles[2] = Some(style(2, 1, Some(0), vec![p(0x083b, &[1])]));
    styles.styles[11] = Some(style(11, 2, Some(10), vec![p(0x083b, &[0])]));
    assert!(resolve(&styles, &fonts, 2, &[]).unwrap().all_caps);
    assert!(
        !resolve(&styles, &fonts, 2, &[p(0x083b, &[129])])
            .unwrap()
            .all_caps
    );
    assert!(
        resolve(&styles, &fonts, 2, &[p(0x083b, &[0]), p(0x083b, &[128])])
            .unwrap()
            .all_caps
    );
    assert!(
        !resolve(&styles, &fonts, 2, &[p(0x4a30, &[11, 0])])
            .unwrap()
            .all_caps
    );
    assert_eq!(
        resolve(&styles, &fonts, 0, &[p(0x083b, &[1]), p(0x2a33, &[0])]).unwrap_err(),
        Error::Unsupported("render character property")
    );
    let independent = resolve(&styles, &fonts, 0, &[p(0x083a, &[1]), p(0x083b, &[0])]).unwrap();
    assert!(independent.small_caps && !independent.all_caps);
    for bytes in [vec![], vec![2], vec![1, 0]] {
        assert!(resolve(&styles, &fonts, 0, &[p(0x083b, &bytes)]).is_err());
    }
}
