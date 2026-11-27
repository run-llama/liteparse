use super::*;
use crate::office::doc::{formatting::CharacterSpan, formatting_inspection::PieceProperties};

#[test]
fn symbol_and_picture_assign_special_in_source_order() {
    for opcode in [0x6a03, 0x6a09] {
        let result =
            resolve_revision_evidence(&fixture(&[(0x0855, &[0]), (opcode, &[1, 0, 2, 0])]))
                .unwrap();
        assert_eq!(result.spans[0].properties.special, Some(true));
        assert_eq!(result.spans[0].properties.deleted, Some(false));
        assert_eq!(result.spans[0].evidence.last().unwrap().sprm, opcode);
        let result =
            resolve_revision_evidence(&fixture(&[(opcode, &[1, 0, 2, 0]), (0x0855, &[0])]))
                .unwrap();
        assert_eq!(result.spans[0].properties.special, Some(false));
        assert!(resolve_revision_evidence(&fixture(&[(opcode, &[1, 0, 2])])).is_err());
    }
}

fn fixture(modifiers: &[(u16, &[u8])]) -> FormattingInspection {
    FormattingInspection {
        schema_version: 1,
        status: "unresolved_character_formatting",
        binary_version: 0x112,
        authored_story_end_cp: 4,
        trailing_nonstory_cp_range: None,
        pieces: vec![PieceProperties {
            start_cp: 0,
            end_cp: 4,
            file_offset: 100,
            compressed: true,
            raw_flags: 0,
            raw_prm: 0,
        }],
        raw_property_groups: Vec::new(),
        character_spans: vec![CharacterSpan {
            start_cp: 0,
            end_cp: 4,
            properties: modifiers
                .iter()
                .map(|(sprm, operand)| PropertyModifier {
                    sprm: *sprm,
                    operand: operand.to_vec(),
                })
                .collect(),
        }],
        unresolved: ["styles", "piece semantics", "visibility"],
    }
}

#[test]
fn resolves_only_style_forbidden_properties_from_false_baselines() {
    let input = fixture(&[
        (0x0800, &[0x81]),
        (0x0801, &[0x80]),
        (0x0802, &[1]),
        (0x0855, &[0]),
        (0x083c, &[0x81]),
    ]);
    let result = resolve_revision_evidence(&input).unwrap();
    assert_eq!(result.hidden_text, "unresolved_style_dependent");
    assert_eq!(
        result.spans[0].properties,
        RevisionProperties {
            deleted: Some(true),
            inserted: Some(false),
            field_hidden: Some(true),
            special: Some(false)
        }
    );
    assert_eq!(result.spans[0].evidence.len(), 4);
    assert_eq!(result.spans[0].source_span_index, 0);
}

#[test]
fn repeated_style_toggle_uses_style_baseline_not_previous_direct_state() {
    let input = fixture(&[
        (0x0800, &[0x81]),
        (0x0800, &[0x81]),
        (0x0855, &[1]),
        (0x0855, &[0x80]),
    ]);
    let result = resolve_revision_evidence(&input).unwrap();
    assert_eq!(result.spans[0].properties.deleted, Some(true));
    assert_eq!(result.spans[0].properties.special, Some(false));
}

#[test]
fn character_style_and_reset_operators_preserve_tracked_properties() {
    let input = fixture(&[
        (0x0800, &[1]),
        (0x0801, &[1]),
        (0x0802, &[1]),
        (0x0855, &[1]),
        (0x4a30, &[10, 0]),
        (0xca31, &[0]),
        (0x2a33, &[0]),
        (0xca47, &[0]),
    ]);
    let result = resolve_revision_evidence(&input).unwrap();
    assert_eq!(
        result.spans[0].properties,
        RevisionProperties {
            deleted: Some(true),
            inserted: Some(true),
            field_hidden: Some(true),
            special: Some(true)
        }
    );
}

#[test]
fn simple_piece_prm_runs_after_chpx_and_retains_provenance() {
    let mut input = fixture(&[(0x0800, &[1])]);
    input.pieces[0].raw_prm = 0x41 << 1;
    let result = resolve_revision_evidence(&input).unwrap();
    assert_eq!(result.spans[0].properties.deleted, Some(false));
    assert_eq!(result.spans[0].evidence[1].source, "pcd_prm");
    assert_eq!(result.spans[0].evidence[1].sprm, 0x0800);
}

#[test]
fn complex_piece_prm_runs_after_chpx_and_mixed_groups_remain_unresolved() {
    let mut input = fixture(&[(0x0855, &[0])]);
    input.pieces[0].raw_prm = 1;
    input.raw_property_groups = vec![vec![0x55, 0x08, 1]];
    assert_eq!(
        resolve_revision_evidence(&input).unwrap().spans[0]
            .properties
            .special,
        Some(true)
    );
    input.raw_property_groups[0].extend_from_slice(&[0x07, 0x24, 1]);
    let result = resolve_revision_evidence(&input).unwrap();
    assert_eq!(result.spans[0].properties, RevisionProperties::unknown());
    assert!(!result.spans[0].unresolved.is_empty());
}

#[test]
fn unknown_sprms_block_property_claims() {
    for modifier in [0x088f, 0xcaff] {
        let input = fixture(&[(modifier, &[1]), (0x0800, &[1])]);
        let result = resolve_revision_evidence(&input).unwrap();
        assert_eq!(result.spans[0].properties, RevisionProperties::unknown());
        assert!(!result.spans[0].unresolved.is_empty());
        assert_eq!(result.spans[0].evidence.len(), 1);
    }
}

#[test]
fn rejects_invalid_toggle_and_source_coverage() {
    assert!(resolve_revision_evidence(&fixture(&[(0x0800, &[2])])).is_err());
    let mut input = fixture(&[]);
    input.character_spans[0].end_cp = 5;
    assert!(resolve_revision_evidence(&input).is_err());
    input.character_spans[0].end_cp = 4;
    input.character_spans[0].start_cp = 1;
    assert!(resolve_revision_evidence(&input).is_err());
}

#[test]
fn unknown_simple_piece_opcode_remains_explicit() {
    let mut input = fixture(&[]);
    input.pieces[0].raw_prm = 2;
    let result = resolve_revision_evidence(&input).unwrap();
    assert_eq!(result.spans[0].properties, RevisionProperties::unknown());
    assert!(result.spans[0].unresolved[0].contains("PRM0"));
}

#[test]
fn evidence_budget_stops_repeated_modifier_copies() {
    let mut state = RevisionSpan {
        start_cp: 0,
        end_cp: 1,
        source_span_index: 0,
        piece_index: 0,
        properties: RevisionProperties::default(),
        evidence: Vec::new(),
        unresolved: Vec::new(),
    };
    let property = PropertyModifier {
        sprm: 0x0800,
        operand: vec![1],
    };
    let mut budget = std::mem::size_of::<RevisionModifier>() + 1;
    apply(&mut state, &property, "chpx", &mut budget).unwrap();
    assert!(matches!(
        apply(&mut state, &property, "chpx", &mut budget),
        Err(Error::Limit(_))
    ));
}

#[test]
fn final_view_applies_properties_after_revision_wall_and_preserves_history_marker() {
    for (before, after) in [(1, 0), (0, 1)] {
        let input = fixture(&[
            (0x0800, &[before]),
            (0x2a83, &[1]),
            (0x0800, &[after]),
            (0x2a83, &[0]),
        ]);
        let result = resolve_revision_evidence(&input).unwrap();
        assert_eq!(result.spans[0].properties.deleted, Some(after == 1));
        assert_eq!(result.spans[0].evidence.len(), 4);
        assert_eq!(result.spans[0].evidence[1].sprm, 0x2a83);
        assert!(result.spans[0].unresolved.is_empty());
    }
    assert!(resolve_revision_evidence(&fixture(&[(0x2a83, &[2])])).is_err());
}

#[test]
fn authored_coverage_requires_exact_explicit_nonstory_range() {
    let mut input = fixture(&[]);
    input.pieces[0].end_cp = 5;
    assert!(resolve_revision_evidence(&input).is_err());
    input.trailing_nonstory_cp_range = Some([4, 5]);
    assert!(resolve_revision_evidence(&input).is_ok());
    input.trailing_nonstory_cp_range = Some([3, 5]);
    assert!(resolve_revision_evidence(&input).is_err());
    input.trailing_nonstory_cp_range = Some([4, 5]);
    input.character_spans[0].end_cp = 3;
    assert!(resolve_revision_evidence(&input).is_err());
}

#[test]
fn border_decoration_preserves_revision_field_and_special_evidence() {
    let input = fixture(&[
        (0x0800, &[1]),
        (0x0802, &[1]),
        (0x0855, &[1]),
        (0x6865, &[255, 25, 16, 255]),
        (0xca72, &[8, 1, 2, 3, 0, 1, 27, 255, 255]),
    ]);
    let result = resolve_revision_evidence(&input).unwrap();
    assert_eq!(result.spans[0].properties.deleted, Some(true));
    assert_eq!(result.spans[0].properties.field_hidden, Some(true));
    assert_eq!(result.spans[0].properties.special, Some(true));
    assert!(result.spans[0].unresolved.is_empty());
    for invalid in [
        vec![8, 1, 2, 3, 1, 1, 1, 0, 0],
        vec![8, 0, 0, 0, 0, 1, 64, 0, 0],
    ] {
        let result = resolve_revision_evidence(&fixture(&[(0xca72, &invalid)])).unwrap();
        assert!(result.spans[0].properties.deleted.is_none());
        assert!(!result.spans[0].unresolved.is_empty());
    }
}
