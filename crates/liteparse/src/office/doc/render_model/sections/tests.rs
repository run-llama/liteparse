use super::*;
use crate::office::doc::{
    formatting::PropertyModifier,
    headers::{HeaderSection, SourceRange},
};

fn fixture(properties: Vec<PropertyModifier>) -> HeaderInspection {
    HeaderInspection {
        status: "synthetic",
        binary_version: 0x10c,
        source: SourceRange {
            start_cp: 10,
            end_cp: 10,
        },
        different_even_and_odd_pages: Some(false),
        dop_first_byte: Some(0),
        stories: vec![],
        sections: vec![HeaderSection {
            index: 0,
            plc_end_cp: 11,
            source: SourceRange {
                start_cp: 0,
                end_cp: 10,
            },
            sepx_offset: Some(512),
            properties,
            different_first_page: Some(false),
            legacy_presence_flags: None,
            references: vec![],
            unresolved: vec![],
        }],
        terminal_sentinel: None,
        ignored_final_plc_cp: None,
        diagnostics: vec![],
        unresolved: ["page_instances", "placement", "layout"],
    }
}
fn word(sprm: u16, value: u16) -> PropertyModifier {
    PropertyModifier {
        sprm,
        operand: value.to_le_bytes().to_vec(),
    }
}
fn margins() -> Vec<PropertyModifier> {
    vec![
        word(0xb021, 1800),
        word(0xb022, 1800),
        word(0x9023, 1440),
        word(0x9024, 1440),
    ]
}

#[test]
fn modern_missing_margins_do_not_inherit_historical_or_shared_defaults() {
    let result = resolve(&fixture(vec![]), 10).unwrap();
    let g = &result[0];
    assert_eq!(g.page_width_twips, Value::defaulted(12240));
    assert_eq!(g.page_height_twips, Value::defaulted(15840));
    assert_eq!(g.left_margin_twips, None);
    assert_eq!(g.header_distance_twips, None);
    assert_eq!(
        g.issues.iter().map(|i| i.reason).collect::<Vec<_>>(),
        [
            "left_margin_missing",
            "right_margin_missing",
            "top_margin_missing",
            "bottom_margin_missing"
        ]
    );
}

#[test]
fn ordered_dimensions_signed_margins_and_orientation_preserve_source_units() {
    let mut p = margins();
    p.extend([
        word(0xb01f, 10000),
        word(0xb01f, 15840),
        word(0xb020, 12240),
        word(0x9023, (-720i16) as u16),
        PropertyModifier {
            sprm: 0x301d,
            operand: vec![2],
        },
    ]);
    let g = resolve(&fixture(p), 10).unwrap().remove(0);
    assert!(g.issues.is_empty());
    assert_eq!(g.page_width_twips, Value::explicit(15840, 5));
    assert_eq!(g.page_height_twips, Value::explicit(12240, 6));
    assert_eq!(g.top_margin_twips, Some(Value::explicit(-720, 7)));
    assert_eq!(g.landscape, Value::explicit(true, 8));
}

#[test]
fn malformed_operands_and_out_of_range_values_fail() {
    for p in [
        word(0xb01f, 143),
        word(0xb020, 31681),
        word(0xb021, 65535),
        word(0x9023, (-32768i16) as u16),
        word(0x500b, 44),
        PropertyModifier {
            sprm: 0x301d,
            operand: vec![3],
        },
        PropertyModifier {
            sprm: 0x3009,
            operand: vec![5],
        },
        PropertyModifier {
            sprm: 0xb01f,
            operand: vec![1],
        },
    ] {
        assert!(resolve(&fixture(vec![p]), 10).is_err());
    }
}

#[test]
fn unsupported_source_operators_keep_order_and_geometry_is_provisional() {
    let mut p = margins();
    p.extend([
        PropertyModifier {
            sprm: 0x3239,
            operand: vec![1],
        },
        word(0xb01f, 12000),
        PropertyModifier {
            sprm: 0xd1ff,
            operand: vec![0],
        },
    ]);
    let g = resolve(&fixture(p), 10).unwrap().remove(0);
    assert_eq!(g.page_width_twips.value, 12000);
    assert_eq!(
        g.issues,
        [
            PropertyIssue {
                modifier_index: Some(4),
                sprm: Some(0x3239),
                reason: "section_revision_history_unresolved"
            },
            PropertyIssue {
                modifier_index: Some(6),
                sprm: Some(0xd1ff),
                reason: "section_property_unresolved"
            }
        ]
    );
}

#[test]
fn section_ownership_must_partition_body_exactly() {
    let mut input = fixture(margins());
    input.sections[0].source.start_cp = 1;
    assert!(resolve(&input, 10).is_err());
    input.sections[0].source.start_cp = 0;
    assert!(resolve(&input, 11).is_err());
    input.sections[0].index = 1;
    assert!(resolve(&input, 10).is_err());
}

#[test]
fn nonpositive_text_area_and_installation_dependent_columns_remain_unresolved() {
    let mut p = margins();
    p.extend([word(0xb01f, 3000), word(0x500b, 1)]);
    let g = resolve(&fixture(p), 10).unwrap().remove(0);
    assert_eq!(g.column_count.value, 2);
    assert_eq!(
        g.issues.iter().map(|i| i.reason).collect::<Vec<_>>(),
        [
            "nonpositive_section_text_width",
            "column_spacing_installation_default_unresolved"
        ]
    );
}

#[test]
fn grid_properties_are_validated_even_when_final_mode_disables_grid() {
    let mut p = margins();
    p.extend([word(0x5032, 1), word(0x9031, 360), word(0x5032, 0)]);
    let g = resolve(&fixture(p), 10).unwrap().remove(0);
    assert!(g.issues.is_empty());
    assert_eq!(g.grid_mode, Value::explicit(0, 6));
    assert_eq!(g.grid_line_pitch_twips, Some(Value::explicit(360, 5)));
    for property in [
        word(0x9031, 0),
        word(0x5032, 4),
        PropertyModifier {
            sprm: 0x7030,
            operand: 6488065i32.to_le_bytes().to_vec(),
        },
    ] {
        assert!(resolve(&fixture(vec![property]), 10).is_err());
    }
}

#[test]
fn active_grid_preserves_exact_mode_but_requires_layout_support_and_pitch() {
    let mut p = margins();
    p.push(word(0x5032, 2));
    let g = resolve(&fixture(p), 10).unwrap().remove(0);
    assert_eq!(g.grid_mode.value, 2);
    assert_eq!(
        g.issues.iter().map(|i| i.reason).collect::<Vec<_>>(),
        [
            "active_document_grid_layout_unresolved",
            "active_document_grid_line_pitch_missing"
        ]
    );
}

#[test]
fn even_column_gaps_must_leave_positive_text_width() {
    let mut p = margins();
    p.extend([word(0x500b, 2), word(0x900c, 4320)]);
    let g = resolve(&fixture(p.clone()), 10).unwrap().remove(0);
    assert_eq!(
        g.issues.iter().map(|i| i.reason).collect::<Vec<_>>(),
        ["nonpositive_section_column_width"]
    );
    p.push(word(0x900c, 4319));
    assert!(resolve(&fixture(p), 10).unwrap()[0].issues.is_empty());
}

#[test]
fn orientation_is_a_one_based_enum_with_independent_page_dimensions() {
    let baseline = resolve(&fixture(margins()), 10).unwrap();
    assert_eq!(baseline[0].landscape, Value::defaulted(false));
    for (operand, expected) in [(1, false), (2, true)] {
        let mut properties = margins();
        properties.push(PropertyModifier {
            sprm: 0x301d,
            operand: vec![operand],
        });
        properties.push(word(0xb01f, 15840));
        properties.push(word(0xb020, 12240));
        let result = resolve(&fixture(properties), 10).unwrap();
        assert_eq!(result[0].landscape, Value::explicit(expected, 4));
        assert_eq!(result[0].page_width_twips.value, 15840);
        assert_eq!(result[0].page_height_twips.value, 12240);
    }
    for operand in [vec![], vec![0], vec![3], vec![255], vec![1, 0]] {
        let mut properties = margins();
        properties.push(PropertyModifier {
            sprm: 0x301d,
            operand,
        });
        assert!(resolve(&fixture(properties), 10).is_err());
    }
}

#[test]
fn note_number_formats_preserve_geometry_but_unknown_and_malformed_stay_guarded() {
    for sprm in [0x5040, 0x5042] {
        let mut properties = margins();
        properties.push(word(sprm, 0));
        let parsed = resolve(&fixture(properties.clone()), 10).unwrap();
        assert!(parsed[0].issues.is_empty());
        assert_eq!(parsed[0].left_margin_twips.unwrap().value, 1800);
        properties.last_mut().unwrap().operand = 65535u16.to_le_bytes().to_vec();
        let parsed = resolve(&fixture(properties.clone()), 10).unwrap();
        assert!(
            parsed[0]
                .issues
                .iter()
                .any(|i| i.reason == "note_number_format_unresolved")
        );
        properties.last_mut().unwrap().operand.pop();
        assert!(resolve(&fixture(properties), 10).is_err());
    }
}

#[test]
fn line_grid_qualification_requires_pitch_and_preserves_other_geometry_errors() {
    let mut props = margins();
    props.push(word(0x5032, 2));
    let missing = resolve(&fixture(props.clone()), 10).unwrap();
    assert!(!crate::office::doc::render_model::plain::qualified_line_grid(&missing[0]));
    props.push(word(0x9031, 360));
    let valid = resolve(&fixture(props.clone()), 10).unwrap();
    assert!(crate::office::doc::render_model::plain::qualified_line_grid(&valid[0]));
    for mode in [1, 3] {
        let mut wrong_mode = valid[0].clone();
        wrong_mode.grid_mode.value = mode;
        assert!(!crate::office::doc::render_model::plain::qualified_line_grid(&wrong_mode));
    }
    props.push(word(0xb021, 12000));
    let invalid = resolve(&fixture(props), 10).unwrap();
    assert!(!crate::office::doc::render_model::plain::qualified_line_grid(&invalid[0]));
}

#[test]
fn ignored_section_extension_preserves_later_geometry_and_guards_other_payloads() {
    let mut p = margins();
    p.extend([
        word(0xb01f, 10000),
        PropertyModifier {
            sprm: 0xd1ff,
            operand: vec![2, 3, 0],
        },
        word(0xb01f, 15840),
    ]);
    let source = fixture(p);
    let g = resolve(&source, 10).unwrap().remove(0);
    assert!(g.issues.is_empty());
    assert_eq!(g.page_width_twips, Value::explicit(15840, 6));
    assert_eq!(source.sections[0].properties[5].operand, [2, 3, 0]);
    for operand in [vec![2, 3], vec![1, 3], vec![2, 4, 0]] {
        let mut p = margins();
        p.push(PropertyModifier {
            sprm: 0xd1ff,
            operand,
        });
        let g = resolve(&fixture(p), 10).unwrap().remove(0);
        assert!(
            g.issues
                .iter()
                .any(|i| i.reason == "section_property_unresolved")
        );
    }
}

#[test]
fn equal_column_width_uses_ordered_source_gap_and_rejects_shared_clamping() {
    let mut properties = margins();
    properties.extend([word(0x500b, 1), word(0x900c, 360), word(0x900c, 720)]);
    let mut section = resolve(&fixture(properties), 10).unwrap().remove(0);
    assert_eq!(
        crate::office::doc::render_model::plain::columns::width_twips(&section).unwrap(),
        3960.0
    );
    section.page_width_twips.value = 3960;
    section.column_spacing_twips.as_mut().unwrap().value = 0;
    assert!(crate::office::doc::render_model::plain::columns::width_twips(&section).is_err());
    section.page_width_twips.value = 4320;
    section.column_spacing_twips.as_mut().unwrap().value = 360;
    assert_eq!(
        crate::office::doc::render_model::plain::columns::width_twips(&section).unwrap(),
        180.0
    );
    section.column_spacing_twips.as_mut().unwrap().value = 361;
    assert!(crate::office::doc::render_model::plain::columns::width_twips(&section).is_err());
    section.column_spacing_twips = None;
    assert!(crate::office::doc::render_model::plain::columns::width_twips(&section).is_err());
}
