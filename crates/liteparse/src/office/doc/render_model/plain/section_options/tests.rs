use super::*;
use crate::office::doc::headers::{HeaderInspection, HeaderSection, SourceRange};

fn geometry(properties: Vec<PropertyModifier>) -> SectionGeometry {
    let source = HeaderInspection {
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
    };
    crate::office::doc::render_model::sections::resolve(&source, 10)
        .unwrap()
        .remove(0)
}
fn margins() -> Vec<PropertyModifier> {
    [0xb021, 0xb022, 0x9023, 0x9024]
        .into_iter()
        .map(|sprm| PropertyModifier {
            sprm,
            operand: 1440u16.to_le_bytes().to_vec(),
        })
        .collect()
}

#[test]
fn retained_section_options_cannot_hide_missing_geometry_or_change_issue_owners() {
    let mut properties = margins();
    properties.extend([
        PropertyModifier {
            sprm: 0x3011,
            operand: vec![1],
        },
        PropertyModifier {
            sprm: 0x7044,
            operand: 12u32.to_le_bytes().to_vec(),
        },
        PropertyModifier {
            sprm: 0x522f,
            operand: vec![0x2a, 0],
        },
    ]);
    let g = geometry(properties.clone());
    let retained = retain(&g, &properties).unwrap();
    assert_eq!(
        retained
            .iter()
            .map(|p| p.modifier_index)
            .collect::<Vec<_>>(),
        vec![4, 5, 6]
    );
    assert_eq!(g.issues.len(), 3);
    let mut changed = retained.clone();
    changed.swap(0, 1);
    assert!(validate(&g, &changed).is_err());
    assert!(validate(&g, &retained[..2]).is_err());
    changed = retained;
    changed[0].property.operand[0] = 2;
    assert!(validate(&g, &changed).is_err());
    let missing = geometry(properties[4..].to_vec());
    assert!(retain(&missing, &properties[4..]).is_err());
}

#[test]
fn known_operator_names_do_not_admit_malformed_operands_or_active_grid_issues() {
    for property in [
        PropertyModifier {
            sprm: 0x3011,
            operand: vec![2],
        },
        PropertyModifier {
            sprm: 0x3012,
            operand: vec![],
        },
        PropertyModifier {
            sprm: 0x3014,
            operand: vec![0x40],
        },
        PropertyModifier {
            sprm: 0x7044,
            operand: u32::MAX.to_le_bytes().to_vec(),
        },
        PropertyModifier {
            sprm: 0x9016,
            operand: 31681u16.to_le_bytes().to_vec(),
        },
        PropertyModifier {
            sprm: 0x522f,
            operand: vec![3, 0],
        },
        PropertyModifier {
            sprm: 0x522f,
            operand: vec![0x10, 0],
        },
        PropertyModifier {
            sprm: 0x522f,
            operand: vec![0x40, 0],
        },
        PropertyModifier {
            sprm: 0x522f,
            operand: vec![0, 1],
        },
    ] {
        assert!(name(&property).is_err(), "{:x}", property.sprm);
    }
    let mut properties = margins();
    properties.push(PropertyModifier {
        sprm: 0x5032,
        operand: vec![2, 0],
    });
    assert!(retain(&geometry(properties.clone()), &properties).is_err());
}

#[test]
fn grid_and_retained_metadata_preserve_independent_geometry_guards() {
    let mut properties = margins();
    properties.extend([
        PropertyModifier {
            sprm: 0x5032,
            operand: 2u16.to_le_bytes().to_vec(),
        },
        PropertyModifier {
            sprm: 0x9031,
            operand: 360u16.to_le_bytes().to_vec(),
        },
        PropertyModifier {
            sprm: 0x3011,
            operand: vec![1],
        },
    ]);
    let g = geometry(properties.clone());
    let retained = retain(&g, &properties).unwrap();
    assert_eq!(retained.len(), 1);
    validate(&g, &retained).unwrap();
    assert!(!super::super::qualified_line_grid(&g));
    let mut bad = g.clone();
    bad.grid_mode.value = 0;
    assert!(validate(&bad, &retained).is_err());
    for pitch in [0, 31681] {
        let mut bad = g.clone();
        bad.grid_line_pitch_twips.as_mut().unwrap().value = pitch;
        assert!(validate(&bad, &retained).is_err());
        assert!(retain(&bad, &properties).is_err());
    }
    let mut bad = g.clone();
    bad.issues[0].reason = "unqualified_geometry";
    assert!(validate(&bad, &retained).is_err());
    assert!(retain(&bad, &properties).is_err());
    assert!(validate(&g, &[]).is_err());
}

#[test]
fn retained_page_borders_bind_ordered_legacy_and_modern_operands() {
    let mut properties = margins();
    properties.push(PropertyModifier {
        sprm: 0x702b,
        operand: vec![4, 1, 1, 4],
    });
    properties.push(PropertyModifier {
        sprm: 0xd234,
        operand: vec![8, 0, 0, 0, 255, 0, 0, 0, 0],
    });
    let g = geometry(properties.clone());
    let retained = retain(&g, &properties).unwrap();
    assert_eq!(retained.len(), 2);
    assert_eq!(retained[0].property.sprm, 0x702b);
    assert_eq!(retained[1].property.sprm, 0xd234);
    let mut wrong = retained;
    wrong.swap(0, 1);
    assert!(validate(&g, &wrong).is_err());
    properties[4].operand.pop();
    assert!(retain(&geometry(properties.clone()), &properties).is_err());
}
