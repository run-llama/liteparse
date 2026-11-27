use super::*;
use crate::office::doc::paragraph_formatting::ParagraphSpan;

fn papx(bytes: Vec<u8>) -> ParagraphInspection {
    ParagraphInspection {
        schema_version: 1,
        status: "synthetic",
        binary_version: 0xc1,
        spans: vec![ParagraphSpan {
            start_cp: 4,
            end_cp: 8,
            style_index: Some(2),
            raw_modifiers: bytes,
        }],
        excluded_nonstory_cp_range: None,
        unresolved: [""; 4],
    }
}

#[test]
fn source_membership_is_ordered_and_requires_exact_physical_endpoint() {
    let p = papx(vec![0x16, 0x24, 1, 0x17, 0x24, 0, 0x03, 0x24, 2]);
    let (style, normal, table) = properties(&p, [4, 8], false, &mut 100).unwrap();
    assert_eq!(style, 2);
    assert_eq!(
        normal,
        vec![PropertyModifier {
            sprm: 0x2403,
            operand: vec![2]
        }]
    );
    assert!(table.is_empty());
    assert!(properties(&p, [5, 8], false, &mut 100).is_err());
    assert!(properties(&p, [4, 7], false, &mut 100).is_err());
    let p = papx(vec![0x16, 0x24, 1, 0x16, 0x24, 0]);
    assert!(properties(&p, [4, 8], false, &mut 100).is_err());
}

#[test]
fn table_properties_only_belong_to_complete_row_mark() {
    let p = papx(vec![0x16, 0x24, 1, 0x17, 0x24, 1, 0x03, 0x34, 1]);
    let (_, normal, table) = properties(&p, [4, 8], true, &mut 100).unwrap();
    assert!(normal.is_empty());
    assert_eq!(
        table,
        vec![PropertyModifier {
            sprm: 0x3403,
            operand: vec![1]
        }]
    );
    assert!(properties(&p, [4, 8], false, &mut 100).is_err());
    let p = papx(vec![0x16, 0x24, 1, 0x03, 0x34, 1]);
    assert!(properties(&p, [4, 8], false, &mut 100).is_err());
}

#[test]
fn malformed_membership_and_properties_fail_with_bounded_work() {
    for raw in [
        vec![0x16],
        vec![0x16, 0x24],
        vec![0x16, 0x24, 2],
        vec![0x08, 0xd6, 255, 255],
    ] {
        assert!(properties(&papx(raw), [4, 8], false, &mut 100).is_err());
    }
    assert!(properties(&papx(vec![0x16, 0x24, 1]), [4, 8], false, &mut 3).is_err());
}

#[test]
fn protection_payload_requires_explicit_table_boundary_semantics() {
    let mut word = vec![0; 154 + 144 * 8];
    word[152..154].copy_from_slice(&144u16.to_le_bytes());
    protection_absent(&word).unwrap();
    for index in 141..=143 {
        word[158 + 8 * index] = 1;
        assert!(protection_absent(&word).is_err());
        word[158 + 8 * index] = 0;
    }
    word[152..154].copy_from_slice(&93u16.to_le_bytes());
    protection_absent(&word[..154 + 93 * 8]).unwrap();
}

#[test]
fn repeated_row_style_evidence_consumes_one_aggregate_work_budget() {
    use crate::office::doc::render_model::tables::{CellLayout, style_defaults::CellDefaults};
    let row = RowLayout {
        style_defaults: vec![CellDefaults {
            selected_style: 11,
            base_to_derived: vec![11],
            cell: CellLayout {
                edges_twips: [0, 0],
                borders: Default::default(),
                preferred_width: None,
                margins_twips: [0, 108, 0, 108],
                vertical_alignment: 0,
                raw_flags: 0,
                no_wrap: false,
                shading_rgb: None,
            },
            row_borders: Default::default(),
            applied: vec![(
                11,
                PropertyModifier {
                    sprm: 0xf617,
                    operand: vec![3, 0, 0],
                },
            )],
        }],
        ..RowLayout::default()
    };
    let mut once = 65536;
    charge_row_evidence(&row, &mut once).unwrap();
    let cost = 65536 - once;
    assert!(cost > row.style_defaults[0].applied[0].1.operand.len());
    let mut budget = 2 * cost - 1;
    charge_row_evidence(&row, &mut budget).unwrap();
    assert!(matches!(
        charge_row_evidence(&row, &mut budget),
        Err(Error::Limit(_))
    ));
}

#[test]
fn public_ledger_bounds_repeated_valid_style_replays_across_rows() {
    use crate::office::doc::render_model::{
        plain::PlainParagraph,
        tables::{CellLayout, style_defaults::CellDefaults},
    };
    let styles = vec![CellDefaults {
        selected_style: 11,
        base_to_derived: vec![11],
        cell: CellLayout {
            edges_twips: [0, 0],
            borders: Default::default(),
            preferred_width: None,
            margins_twips: [0, 108, 0, 108],
            vertical_alignment: 0,
            raw_flags: 0,
            no_wrap: false,
            shading_rgb: None,
        },
        row_borders: Default::default(),
        applied: vec![
            (
                11,
                PropertyModifier {
                    sprm: 0xf617,
                    operand: vec![3, 0, 0]
                }
            );
            100_000
        ],
    }];
    let layout = super::super::modern::decode(
        0x112,
        &[PropertyModifier {
            sprm: 0x7621,
            operand: vec![0, 1, 232, 3],
        }],
        &styles,
    )
    .unwrap();
    let properties = paragraphs::ParagraphProperties {
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
        applied: vec![],
    };
    let paras: Vec<_> = (0..4)
        .map(|cp| PlainParagraph {
            cached_fields: Vec::new(),
            source_cp: [cp, cp + 1],
            source_style: 0,
            properties: properties.clone(),
            runs: vec![],
        })
        .collect();
    let rows: Vec<_> = (0..2)
        .map(|index| {
            let cp = 2 * index;
            SourceRow {
                table_membership: Vec::new(),
                depth: 1,
                source_cp: [cp, cp + 2],
                row_mark_paragraph: cp + 1,
                row_mark_style: 0,
                row_mark_properties: properties.clone(),
                layout: layout.clone(),
                cells: vec![SourceCell {
                    nested_tables: Vec::new(),
                    source_cp: [cp, cp + 1],
                    paragraphs: vec![CellParagraph {
                        table_membership: Vec::new(),
                        physical_index: cp,
                        source_cp: [cp, cp + 1],
                        source_style: 0,
                        properties: properties.clone(),
                    }],
                }],
            }
        })
        .collect();
    let mut table = SourceTable {
        source_cp: [0, 2],
        grid: super::super::common_grid(std::slice::from_ref(&layout)).unwrap(),
        grouping_basis: "synthetic",
        rows,
    };
    let second = table.rows.pop().unwrap();
    assert_eq!(
        validate_ledger(std::slice::from_ref(&table), &paras, 0x112).unwrap(),
        [0, 1].into()
    );
    table.rows.push(second);
    table.source_cp[1] = 4;
    table.grid = super::super::common_grid(&[layout.clone(), layout]).unwrap();
    assert!(matches!(
        validate_ledger(&[table], &paras, 0x112),
        Err(Error::Limit("render table ledger work"))
    ));
}

#[test]
fn deferred_cell_list_selection_preserves_other_properties_and_rejects_bad_owners() {
    let styles = Stylesheet {
        schema_version: 1,
        status: "fixture",
        std_base_bytes: 10,
        raw_header: vec![0; 20],
        styles: vec![Some(crate::office::doc::styles::Style {
            index: 0,
            builtin_id: 0,
            kind: 1,
            base_style: None,
            next_style: 0,
            linked_style: None,
            revision_marked: false,
            name: "fixture".into(),
            raw_flags: 0,
            properties: vec![],
            raw_revision: vec![],
        })],
        unresolved: ["fixture"; 3],
    };
    let prop = |sprm, operand: &[u8]| PropertyModifier {
        sprm,
        operand: operand.to_vec(),
    };
    let direct = vec![
        prop(0x260a, &[1]),
        prop(0x460b, &[1, 0]),
        prop(0x840f, &[208, 2]),
    ];
    assert!(resolve_cell_paragraph(&styles, 0, &direct, [4, 8], None).is_err());
    let mut pending = std::collections::BTreeMap::new();
    let result = resolve_cell_paragraph(&styles, 0, &direct, [4, 8], Some(&mut pending)).unwrap();
    assert_eq!(result.left_twips, 720);
    assert_eq!(result.applied, vec![direct[2].clone()]);
    assert_eq!(pending[&[4, 8]], direct);
    assert!(resolve_cell_paragraph(&styles, 0, &direct, [4, 8], Some(&mut pending)).is_err());
    for invalid in [
        prop(0x260a, &[]),
        prop(0x260a, &[9]),
        prop(0x460b, &[1]),
        prop(0x2403, &[255]),
    ] {
        let mut attempted = direct.clone();
        attempted.push(invalid);
        assert!(
            resolve_cell_paragraph(&styles, 0, &attempted, [8, 12], Some(&mut pending)).is_err()
        );
        assert_eq!(pending.len(), 1);
    }
}

#[test]
fn nested_cr_membership_requires_raw_inner_flags_and_exact_depth() {
    let raw = vec![
        0x16, 0x24, 1, 0x49, 0x66, 2, 0, 0, 0, 0x4b, 0x24, 1, 0x4c, 0x24, 1,
    ];
    let source = papx(raw.clone());
    properties_at_depth(&source, [4, 8], true, 2, true, &mut 1000).unwrap();
    let retained = nested::record_membership(&source, [4, 8], 2).unwrap();
    nested::validate_membership(&retained, 2, true, true).unwrap();
    for sprm in [0x6649, 0x244b, 0x244c] {
        let mut missing = retained.clone();
        missing.retain(|p| p.sprm != sprm);
        assert!(nested::validate_membership(&missing, 2, true, true).is_err());
    }
    assert!(nested::validate_membership(&retained, 3, true, true).is_err());
    let mut ordinary = raw;
    ordinary[14] = 0;
    assert!(properties_at_depth(&papx(ordinary), [4, 8], true, 2, true, &mut 1000).is_err());
}
