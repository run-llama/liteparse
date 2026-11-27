use super::*;
use crate::office::doc::{
    paragraph_formatting::ParagraphSpan,
    structure::{ParagraphEndpoint, ParagraphOwnership, StoryStructure},
    styles::{Style, StyleProperties},
};

fn style(index: usize, base: Option<usize>) -> Style {
    Style {
        index,
        builtin_id: 105,
        kind: 3,
        base_style: base,
        next_style: index,
        linked_style: None,
        revision_marked: false,
        name: "Synthetic table".into(),
        raw_flags: 0,
        properties: vec![
            StyleProperties {
                kind: "table",
                bytes: vec![],
            },
            StyleProperties {
                kind: "paragraph",
                bytes: (index as u16).to_le_bytes().to_vec(),
            },
            StyleProperties {
                kind: "character",
                bytes: vec![],
            },
        ],
        raw_revision: vec![],
    }
}
struct Fixture {
    source: Inspection,
    formatting: FormattingInspection,
    paragraphs: ParagraphInspection,
    styles: Stylesheet,
    structure: StructurePlan,
    revisions: RevisionEvidence,
}
impl Fixture {
    fn new() -> Self {
        let (mut source, formatting, mut paragraphs) = super::super::tests::contexts();
        source.stories[0].text = "A\u{7}\u{7}\r".into();
        paragraphs.status = "reference_expanded_endpoint_selected_paragraphs";
        let inside = vec![0x16, 0x24, 1, 0x49, 0x66, 1, 0, 0, 0];
        paragraphs.spans[0].raw_modifiers = inside.clone();
        paragraphs.spans[1].end_cp = 3;
        paragraphs.spans[1].style_index = Some(0);
        paragraphs.spans[1].raw_modifiers = [
            inside,
            vec![0x17, 0x24, 1, 0x21, 0x76, 0, 1, 10, 0, 0x3a, 0x56, 11, 0],
        ]
        .concat();
        paragraphs.spans.push(ParagraphSpan {
            start_cp: 3,
            end_cp: 4,
            style_index: Some(0),
            raw_modifiers: vec![],
        });
        let mut styles = super::super::tests::sheet();
        styles.styles[11] = Some(style(11, None));
        let structure = StructurePlan {
            status: "synthetic",
            unresolved: [""; 3],
            stories: vec![StoryStructure {
                kind: "body",
                paragraphs: [(0, 2, 7), (2, 3, 7), (3, 4, 13)]
                    .into_iter()
                    .map(|(a, b, u)| ParagraphOwnership {
                        source_range: [a, b],
                        projected_bytes: Some([a, b]),
                        terminal_unit: Some(u),
                        endpoint: Some(ParagraphEndpoint {
                            end_cp: b,
                            piece_index: 0,
                            fkp_offset: 0,
                            fkp_run: 0,
                            end_fc: b,
                        }),
                        terminal_projection: "synthetic",
                        unresolved: vec![],
                    })
                    .collect(),
            }],
        };
        let revisions =
            crate::office::doc::visibility::resolve_revision_evidence(&formatting).unwrap();
        Self {
            source,
            formatting,
            paragraphs,
            styles,
            structure,
            revisions,
        }
    }
    fn evaluate(&self) -> Result<(super::super::HiddenEvidence, TableHiddenEvidence)> {
        super::super::resolve_hidden_evidence_with_tables(
            &self.source,
            &self.formatting,
            &self.paragraphs,
            &self.styles,
            &self.structure,
            &self.revisions,
        )
    }
}
#[test]
fn selected_row_empty_text_style_establishes_baseline_without_changing_source() {
    let f = Fixture::new();
    let (hidden, table) = f.evaluate().unwrap();
    assert_eq!(table.rows.len(), 1);
    assert!(table.rows[0].empty_text_contribution);
    assert_eq!(table.rows[0].selected_style, Some(11));
    assert_eq!(table.rows[0].source_range, [0, 3]);
    assert!(
        hidden
            .spans
            .iter()
            .all(|p| p.property.hidden == Some(false))
    );
    assert_eq!(
        hidden.spans[0].table_baseline_basis,
        "selected_row_style_has_empty_text_properties"
    );
    assert_eq!(f.source.stories[0].text, "A\u{7}\u{7}\r");
}
#[test]
fn end_piece_override_selects_style_and_unselected_hidden_style_has_no_effect() {
    let mut f = Fixture::new();
    let mut hidden = style(12, None);
    hidden.properties[2].bytes = vec![0x3c, 0x08, 1];
    f.styles.styles.push(Some(hidden));
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x3a, 0x56, 12, 0]);
    assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
    f.formatting
        .raw_property_groups
        .push(vec![0x3a, 0x56, 11, 0]);
    f.formatting.pieces[0].raw_prm = 1;
    assert!(f.evaluate().unwrap().1.rows[0].empty_text_contribution);
}
#[test]
fn empty_ancestor_chain_is_supported_but_conditional_or_history_groups_are_unknown() {
    let mut f = Fixture::new();
    f.styles.styles.push(Some(style(12, Some(11))));
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x3a, 0x56, 12, 0]);
    assert_eq!(
        f.evaluate().unwrap().1.rows[0].style_ancestors,
        vec![12, 11]
    );
    f.styles.styles[11].as_mut().unwrap().properties[2].bytes =
        vec![0x85, 0xca, 5, 1, 0, 0x3c, 0x08, 1];
    assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
    f.styles.styles[11].as_mut().unwrap().properties[2]
        .bytes
        .clear();
    f.styles.styles[11].as_mut().unwrap().revision_marked = true;
    assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
}
#[test]
fn row_revision_unknown_and_deleted_content_prevent_proof() {
    let mut f = Fixture::new();
    for deleted in [None, Some(true)] {
        f.revisions.spans[0].properties.deleted = deleted;
        let (_, t) = f.evaluate().unwrap();
        assert!(!t.rows[0].empty_text_contribution);
        assert!(
            t.rows[0]
                .unresolved
                .contains(&"table_revision_state_unresolved")
        );
    }
    f.revisions.spans[0].properties.deleted = Some(false);
    f.revisions.spans[0].properties.inserted = Some(true);
    assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
}
#[test]
fn physical_endpoint_and_revision_coverage_are_required() {
    let mut f = Fixture::new();
    f.structure.stories[0].paragraphs[1]
        .endpoint
        .as_mut()
        .unwrap()
        .piece_index = 2;
    assert!(f.evaluate().is_err());
    let mut f = Fixture::new();
    f.revisions.spans[0].end_cp = 2;
    assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
}
#[test]
fn missing_cell_definition_and_unknown_row_style_modifiers_remain_unresolved() {
    let mut f = Fixture::new();
    f.paragraphs.spans[1].raw_modifiers.truncate(12);
    let (_, t) = f.evaluate().unwrap();
    assert!(!t.rows[0].empty_text_contribution);
    let mut f = Fixture::new();
    f.paragraphs.spans[1].raw_modifiers.extend([0x68, 0x36, 1]);
    assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
}
#[test]
fn styles_validate_framing_and_bound_empty_ancestor_work() {
    let mut f = Fixture::new();
    let mut cache = BTreeMap::new();
    assert!(empty_style(11, &f.styles, &mut cache, &mut 0).is_err());
    f.styles.styles[11].as_mut().unwrap().base_style = Some(11);
    assert!(empty_style(11, &f.styles, &mut cache, &mut 4096).is_err());
    f.styles.styles[11].as_mut().unwrap().base_style = None;
    f.styles.styles[11].as_mut().unwrap().properties[0].bytes = vec![0x34, 0xd6, 6, 0];
    assert!(empty_style(11, &f.styles, &mut cache, &mut 4096).is_err());
}
#[test]
fn conditional_paragraph_group_cannot_change_hidden_baseline_through_a_style() {
    let mut f = Fixture::new();
    f.styles.styles[11].as_mut().unwrap().properties[1]
        .bytes
        .extend([0x66, 0xc6, 2, 1, 0]);
    assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
}

#[test]
fn nested_rows_stay_unknown_while_merge_only_rows_keep_empty_text_baseline() {
    let f = Fixture::new();
    let mut tables = crate::office::doc::table_structure::plan_tables(
        &f.source,
        &f.structure,
        &f.paragraphs,
        &f.formatting,
    )
    .unwrap();
    tables.stories[0].rows[0].depth = 2;
    let evidence = resolve(
        &f.source,
        &f.formatting,
        &f.paragraphs,
        &f.styles,
        &f.structure,
        &tables,
        &f.revisions,
    )
    .unwrap();
    assert!(!evidence.rows[0].empty_text_contribution);
    assert!(!evidence.permits_list_paragraph([0, 2]));
    tables.stories[0].rows[0].depth = 1;
    tables.stories[0].rows[0].cells[0]
        .definition
        .as_mut()
        .unwrap()
        .vertical_merge = 3;
    tables.stories[0].rows[0]
        .unresolved
        .push("merged_cell_visible_ownership_unresolved");
    let evidence = resolve(
        &f.source,
        &f.formatting,
        &f.paragraphs,
        &f.styles,
        &f.structure,
        &tables,
        &f.revisions,
    )
    .unwrap();
    assert!(evidence.rows[0].empty_text_contribution);
    assert!(evidence.rows[0].unresolved.is_empty());
    assert_eq!(
        evidence.rows[0].physical_ownership_unresolved,
        ["merged_cell_visible_ownership_unresolved"]
    );
    assert!(!evidence.permits_list_paragraph([0, 2]));
}

#[test]
fn proven_table_baseline_still_applies_paragraph_hidden_properties() {
    let mut f = Fixture::new();
    f.styles.styles[0].as_mut().unwrap().properties[0].bytes = vec![0x3c, 0x08, 1];
    let (hidden, table) = f.evaluate().unwrap();
    assert!(table.rows[0].empty_text_contribution);
    assert!(
        hidden
            .spans
            .iter()
            .all(|span| span.property.hidden == Some(true))
    );
    let accepted = crate::office::doc::transcribe(&f.source);
    let visible = super::super::apply_hidden(&accepted, &hidden).unwrap();
    assert!(!visible.stories[0].text.contains('A'));
    assert!(accepted.stories[0].text.contains('A'));
}

#[test]
fn valid_legacy_cell_definitions_preserve_row_style_selection() {
    let mut f = Fixture::new();
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x08, 0xd6, 6, 0, 1, 0, 0, 10, 0]);
    let tables = crate::office::doc::table_structure::plan_tables(
        &f.source,
        &f.structure,
        &f.paragraphs,
        &f.formatting,
    )
    .unwrap();
    assert_eq!(tables.stories[0].rows[0].definition_cell_count, Some(1));
    assert!(tables.stories[0].rows[0].unresolved.is_empty());
    let (_, evidence) = f.evaluate().unwrap();
    assert!(evidence.rows[0].unresolved.is_empty());
    assert!(evidence.rows[0].empty_text_contribution);
}

#[test]
fn autofit_boolean_changes_geometry_without_changing_hidden_baseline() {
    for value in [0, 1] {
        let mut f = Fixture::new();
        f.paragraphs.spans[1]
            .raw_modifiers
            .extend([0x15, 0x36, value]);
        let (hidden, table) = f.evaluate().unwrap();
        assert!(table.rows[0].empty_text_contribution);
        assert!(
            hidden
                .spans
                .iter()
                .all(|s| s.property.hidden == Some(false))
        );
    }
    for value in [2, 0x80, 0xff] {
        let mut f = Fixture::new();
        f.paragraphs.spans[1]
            .raw_modifiers
            .extend([0x15, 0x36, value]);
        let (hidden, table) = f.evaluate().unwrap();
        assert!(!table.rows[0].empty_text_contribution);
        assert_eq!(
            table.rows[0].unresolved,
            ["table_style_selection_unresolved"]
        );
        assert_eq!(hidden.spans[0].property.hidden, None);
    }
}

#[test]
fn truncated_autofit_operand_cannot_establish_table_visibility() {
    let mut f = Fixture::new();
    f.paragraphs.spans[1].raw_modifiers.extend([0x15, 0x36]);
    match f.evaluate() {
        Err(_) => {}
        Ok((hidden, table)) => {
            assert!(table.rows.iter().all(|r| !r.empty_text_contribution));
            assert_eq!(hidden.spans[0].property.hidden, None);
        }
    }
}

#[test]
fn autofit_keeps_conditional_style_and_revision_guards() {
    let mut f = Fixture::new();
    f.paragraphs.spans[1].raw_modifiers.extend([0x15, 0x36, 1]);
    f.styles.styles[11].as_mut().unwrap().properties[2].bytes =
        vec![0x85, 0xca, 5, 1, 0, 0x3c, 0x08, 1];
    assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
    f.styles.styles[11].as_mut().unwrap().properties[2]
        .bytes
        .clear();
    f.revisions.spans[0].properties.deleted = Some(true);
    assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
    f.revisions.spans[0].properties.deleted = Some(false);
    f.paragraphs.spans[1].raw_modifiers.extend([0x68, 0x36, 1]);
    assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
}

#[test]
fn malformed_legacy_definitions_remain_unresolved() {
    for data in [
        vec![0x08, 0xd6, 6, 0, 1, 10, 0, 0, 0],
        vec![0x08, 0xd6, 6, 0, 1, 0, 0],
    ] {
        let mut f = Fixture::new();
        f.paragraphs.spans[1].raw_modifiers.extend(data);
        match f.evaluate() {
            Err(_) => {}
            Ok((hidden, table)) => {
                assert!(table.rows.iter().all(|r| !r.empty_text_contribution));
                assert_eq!(hidden.spans[0].property.hidden, None);
            }
        }
    }
}

#[test]
fn borders_and_padding_preserve_hidden_baseline_with_strict_operands() {
    let mut f = Fixture::new();
    f.paragraphs.spans[1].raw_modifiers.extend([
        0x2f, 0xd6, 11, 0, 1, 15, 0, 0, 0, 255, 18, 1, 0, 0, 0x34, 0xd6, 6, 0, 1, 5, 0, 0, 0,
    ]);
    assert!(f.evaluate().unwrap().1.rows[0].empty_text_contribution);
    f.styles.styles[11].as_mut().unwrap().properties[2].bytes = vec![0x3c, 0x08, 1];
    assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
}

#[test]
fn border_range_must_exist_at_application_time() {
    let border = [
        0x2f, 0xd6, 11, 0, 1, 15, 255, 255, 255, 255, 255, 255, 255, 255,
    ];
    let mut f = Fixture::new();
    f.paragraphs.spans[1].raw_modifiers.splice(0..0, border);
    let result = f.evaluate().unwrap();
    assert_eq!(
        result.1.rows[0].unresolved,
        ["table_style_selection_unresolved"]
    );
    let mut f = Fixture::new();
    f.formatting.raw_property_groups.push(border.to_vec());
    f.formatting.pieces[0].raw_prm = 1;
    assert!(f.evaluate().unwrap().1.rows[0].empty_text_contribution);
}

#[test]
fn border_validation_accepts_nil_and_ignored_reserved_bits_but_rejects_bad_ranges() {
    let border = [11, 0, 1, 63, 0, 0, 0, 255, 18, 1, 255, 255];
    assert!(valid_cell_border(&border, 1));
    let nil = [11, 0, 1, 15, 1, 2, 3, 4, 255, 255, 255, 255];
    assert!(valid_cell_border(&nil, 1));
    let mut empty = border;
    empty[2] = 0;
    assert!(valid_cell_border(&empty, 1));
    for (index, value) in [(0, 10), (1, 1), (2, 2), (3, 64), (7, 1), (9, 2)] {
        let mut bad = border;
        bad[index] = value;
        assert!(!valid_cell_border(&bad, 1), "index {index}");
    }
    assert!(!valid_cell_border(&border[..11], 1));
    assert!(!valid_cell_border(&border, 0));
}

#[test]
fn default_padding_validates_unit_width_side_and_fixed_range() {
    let padding = [6, 0, 1, 15, 3, 192, 123]; // 31680 twips.
    assert!(valid_default_padding(&padding));
    assert!(valid_default_padding(&[6, 0, 1, 0, 0, 0, 0]));
    for (index, value) in [(0, 5), (1, 1), (2, 2), (3, 16), (4, 1), (5, 193)] {
        let mut bad = padding;
        bad[index] = value;
        assert!(!valid_default_padding(&bad), "index {index}");
    }
    assert!(!valid_default_padding(&[6, 0, 1, 5, 0, 1, 0]));
    assert!(!valid_default_padding(&padding[..6]));
}

#[test]
fn malformed_cell_definitions_fail_before_border_count_replay() {
    for malformed in [vec![0x08, 0xd6, 1, 0], vec![0x21, 0x76, 0, 1]] {
        let mut f = Fixture::new();
        f.paragraphs.spans[1].raw_modifiers.extend(malformed);
        assert!(f.evaluate().is_err());
    }
}

#[test]
fn cant_split_variants_validate_boolean_without_claiming_pagination_equivalence() {
    for opcode in [0x3403u16, 0x3466] {
        for value in [0, 1] {
            let mut f = Fixture::new();
            f.paragraphs.spans[1]
                .raw_modifiers
                .extend(opcode.to_le_bytes());
            f.paragraphs.spans[1].raw_modifiers.push(value);
            let (hidden, table) = f.evaluate().unwrap();
            assert!(table.rows[0].empty_text_contribution);
            assert!(
                hidden
                    .spans
                    .iter()
                    .all(|s| s.property.hidden == Some(false))
            );
        }
        for value in [2, 0x80, 0x81, 0xff] {
            let mut f = Fixture::new();
            f.paragraphs.spans[1]
                .raw_modifiers
                .extend(opcode.to_le_bytes());
            f.paragraphs.spans[1].raw_modifiers.push(value);
            let (hidden, table) = f.evaluate().unwrap();
            assert_eq!(
                table.rows[0].unresolved,
                ["table_style_selection_unresolved"]
            );
            assert_eq!(hidden.spans[0].property.hidden, None);
        }
        let mut f = Fixture::new();
        f.paragraphs.spans[1]
            .raw_modifiers
            .extend(opcode.to_le_bytes());
        assert!(f.evaluate().is_err());
    }
}

#[test]
fn ordered_cant_split_values_and_piece_override_preserve_only_visibility_proof() {
    let mut f = Fixture::new();
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x03, 0x34, 1, 0x66, 0x34, 0]);
    f.formatting
        .raw_property_groups
        .push(vec![0x66, 0x34, 1, 0x03, 0x34, 0]);
    f.formatting.pieces[0].raw_prm = 1;
    assert!(f.evaluate().unwrap().1.rows[0].empty_text_contribution);
    f.paragraphs.spans[1].raw_modifiers.extend([0x66, 0x34, 2]);
    assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
}

#[test]
fn cant_split_keeps_merge_conditional_and_revision_guards() {
    for opcode in [0x3403u16, 0x3466] {
        let mut f = Fixture::new();
        f.paragraphs.spans[1]
            .raw_modifiers
            .extend(opcode.to_le_bytes());
        f.paragraphs.spans[1].raw_modifiers.push(1);
        f.styles.styles[11].as_mut().unwrap().properties[2].bytes =
            vec![0x85, 0xca, 5, 1, 0, 0x3c, 0x08, 1];
        assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
        f.styles.styles[11].as_mut().unwrap().properties[2]
            .bytes
            .clear();
        f.revisions.spans[0].properties.inserted = Some(true);
        assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
        f.revisions.spans[0].properties.inserted = Some(false);
        f.paragraphs.spans[1]
            .raw_modifiers
            .extend([0x2b, 0xd6, 2, 0, 1]);
        assert_merge_baseline_only(&f);
    }
}

#[test]
fn cell_padding_validates_existing_ranges_and_cssa_dimensions() {
    let padding = [6, 2, 5, 4, 3, 14, 0];
    assert!(valid_cell_padding(&padding, 5));
    assert!(valid_cell_padding(&[6, 2, 2, 0, 0, 0, 0], 3));
    assert!(valid_cell_padding(&[6, 0, 63, 15, 3, 192, 123], 63));
    for (index, value) in [(0, 5), (1, 5), (2, 1), (2, 6), (3, 16), (4, 1), (4, 19)] {
        let mut bad = padding;
        bad[index] = value;
        assert!(!valid_cell_padding(&bad, 5), "index {index}");
    }
    assert!(!valid_cell_padding(&[6, 0, 1, 5, 0, 1, 0], 1));
    assert!(!valid_cell_padding(&[6, 0, 1, 5, 3, 193, 123], 1));
    assert!(!valid_cell_padding(&[6, 1, 1, 5, 3, 0, 0], 1));
    assert!(!valid_cell_padding(&padding[..6], 5));
    assert!(!valid_cell_padding(&padding, 0));
}

#[test]
fn cell_padding_uses_ordered_definitions_and_ending_piece_properties() {
    let padding = [0x32, 0xd6, 6, 0, 1, 5, 3, 29, 0];
    let mut f = Fixture::new();
    f.paragraphs.spans[1].raw_modifiers.extend(padding);
    assert!(f.evaluate().unwrap().1.rows[0].empty_text_contribution);
    let mut f = Fixture::new();
    f.paragraphs.spans[1].raw_modifiers.splice(0..0, padding);
    assert_eq!(
        f.evaluate().unwrap().1.rows[0].unresolved,
        ["table_style_selection_unresolved"]
    );
    let mut f = Fixture::new();
    f.formatting.raw_property_groups.push(padding.to_vec());
    f.formatting.pieces[0].raw_prm = 1;
    assert!(f.evaluate().unwrap().1.rows[0].empty_text_contribution);
    for malformed in [vec![0x08, 0xd6, 1, 0], vec![0x21, 0x76, 0, 1]] {
        let mut f = Fixture::new();
        f.paragraphs.spans[1].raw_modifiers.extend(padding);
        f.paragraphs.spans[1].raw_modifiers.extend(malformed);
        assert!(f.evaluate().is_err());
    }
}

#[test]
fn cell_padding_keeps_conditional_revision_and_merge_uncertainty() {
    let mut f = Fixture::new();
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x32, 0xd6, 6, 0, 1, 15, 0, 0, 0]);
    f.styles.styles[11].as_mut().unwrap().properties[2].bytes =
        vec![0x85, 0xca, 5, 1, 0, 0x3c, 0x08, 1];
    assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
    f.styles.styles[11].as_mut().unwrap().properties[2]
        .bytes
        .clear();
    f.revisions.spans[0].properties.deleted = Some(true);
    assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
    f.revisions.spans[0].properties.deleted = Some(false);
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x2b, 0xd6, 2, 0, 1]);
    assert_merge_baseline_only(&f);
}

#[test]
fn cell_shading_validates_sizes_counts_palette_colors_and_patterns() {
    assert!(valid_cell_shading(0xd609, &[0], 0));
    assert!(valid_cell_shading(0xd612, &[0], 0));
    assert!(valid_cell_shading(0xd609, &[2, 255, 255], 1));
    assert!(valid_cell_shading(0xd609, &[2, 9, 154], 1)); // Palette 9, background 16, pattern 38.
    for value in [17u16, 17 << 5, 26 << 10, 62 << 10, 63 << 10] {
        let mut operand = vec![2];
        operand.extend(value.to_le_bytes());
        assert!(!valid_cell_shading(0xd609, &operand, 1));
    }
    for malformed in [vec![], vec![1, 0], vec![2, 0], vec![4, 0, 0, 0, 0]] {
        assert!(!valid_cell_shading(0xd609, &malformed, 1));
    }
    let shading = [10, 0, 0, 0, 255, 230, 230, 230, 0, 0, 0];
    assert!(valid_cell_shading(0xd612, &shading, 1));
    for (index, value) in [(0, 9), (4, 1), (8, 1), (9, 26), (9, 62)] {
        let mut bad = shading;
        bad[index] = value;
        assert!(!valid_cell_shading(0xd612, &bad, 1));
    }
    assert!(!valid_cell_shading(0xd612, &shading[..10], 1));
    assert!(!valid_cell_shading(0xd612, &shading, 0));
    let mut full = vec![220];
    full.extend(vec![0; 220]);
    assert!(valid_cell_shading(0xd612, &full, 22));
    assert!(!valid_cell_shading(0xd612, &full, 21));
    full[0] = 230;
    full.extend([0; 10]);
    assert!(!valid_cell_shading(0xd612, &full, 23));
}

#[test]
fn cell_shading_auto_nil_and_legacy_backgrounds_preserve_hidden_properties() {
    for version in [0xc1, 0xd9, 0x101, 0x112] {
        for shading in [
            [10, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0],
            [10, 255, 255, 255, 255, 255, 255, 255, 255, 0, 0],
            [10, 0, 0, 0, 0, 0, 0, 0, 0, 255, 255],
        ] {
            let mut f = Fixture::new();
            f.source.binary_version = version;
            f.paragraphs.binary_version = version;
            f.formatting.binary_version = version;
            f.paragraphs.spans[1]
                .raw_modifiers
                .extend([0x09, 0xd6, 2, 255, 255, 0x12, 0xd6]);
            f.paragraphs.spans[1].raw_modifiers.extend(shading);
            let (hidden, table) = f.evaluate().unwrap();
            assert!(table.rows[0].empty_text_contribution);
            assert!(
                hidden
                    .spans
                    .iter()
                    .all(|s| s.property.hidden == Some(false))
            );
        }
    }
}

#[test]
fn cell_shading_requires_source_ordered_definitions_and_valid_earlier_operands() {
    let shading = [
        0x09, 0xd6, 2, 0, 0, 0x12, 0xd6, 10, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0,
    ];
    let mut f = Fixture::new();
    f.paragraphs.spans[1].raw_modifiers.splice(0..0, shading);
    assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
    let mut f = Fixture::new();
    f.formatting.raw_property_groups.push(shading.to_vec());
    f.formatting.pieces[0].raw_prm = 1;
    assert!(f.evaluate().unwrap().1.rows[0].empty_text_contribution);
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x09, 0xd6, 2, 17, 0]);
    assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
}

#[test]
fn cell_shading_retains_style_revision_and_merge_guards() {
    let mut f = Fixture::new();
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x09, 0xd6, 2, 0, 0]);
    f.styles.styles[11].as_mut().unwrap().properties[2].bytes =
        vec![0x85, 0xca, 5, 1, 0, 0x3c, 0x08, 1];
    assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
    f.styles.styles[11].as_mut().unwrap().properties[2]
        .bytes
        .clear();
    f.revisions.spans[0].properties.inserted = Some(true);
    assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
    f.revisions.spans[0].properties.inserted = Some(false);
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x2b, 0xd6, 2, 0, 1]);
    assert_merge_baseline_only(&f);
}

#[test]
fn empty_text_proof_requires_exact_physical_paragraph_membership() {
    let (_, mut table) = Fixture::new().evaluate().unwrap();
    assert!(table.has_empty_text_contribution([0, 2], 1));
    assert!(table.has_empty_text_contribution([2, 3], 2));
    for (range, mark) in [([0, 1], 2), ([1, 3], 1), ([3, 4], 3), ([0, 0], 1)] {
        assert!(!table.has_empty_text_contribution(range, mark));
    }
    assert!(!table.has_empty_text_contribution([0, 1], usize::MAX));
    table.rows[0].unresolved.push("synthetic_unresolved");
    assert!(!table.has_empty_text_contribution([0, 2], 1));
    table.rows[0].unresolved.clear();
    table.rows[0].empty_text_contribution = false;
    assert!(!table.has_empty_text_contribution([0, 2], 1));
}

#[test]
fn capitalization_uses_empty_table_proof_then_applies_direct_toggles() {
    let mut f = Fixture::new();
    f.formatting.character_spans[0].properties.push(
        crate::office::doc::formatting::PropertyModifier {
            sprm: 0x083b,
            operand: vec![129],
        },
    );
    let (hidden, table) = f.evaluate().unwrap();
    let text = crate::office::doc::transcribe(&f.source);
    let mut context = crate::office::doc::capitalization::Context {
        source: &f.source,
        formatting: &f.formatting,
        styles: &f.styles,
        hidden: &hidden,
        revisions: &f.revisions,
        table_properties: None,
    };
    let before = crate::office::doc::capitalization::inspect(&text, &context).unwrap();
    assert_eq!(before.spans[0].all_caps, None);
    context.table_properties = Some(&table);
    let after = crate::office::doc::capitalization::inspect(&text, &context).unwrap();
    assert_eq!(after.spans[0].all_caps, Some(true));
    assert_eq!(after.spans[0].small_caps, Some(false));
    assert_eq!(
        serde_json::to_value(&before.spans[0].output).unwrap(),
        serde_json::to_value(&after.spans[0].output).unwrap()
    );
}

#[test]
fn table_caps_and_small_caps_properties_prevent_empty_text_proof() {
    for sprm in [0x083au16, 0x083b] {
        let mut f = Fixture::new();
        let style = f.styles.styles[11].as_mut().unwrap();
        style
            .properties
            .iter_mut()
            .find(|g| g.kind == "character")
            .unwrap()
            .bytes = [sprm.to_le_bytes().to_vec(), vec![0]].concat();
        let (_, table) = f.evaluate().unwrap();
        assert!(!table.has_empty_text_contribution([0, 2], 1));
    }
}

#[test]
fn capitalization_table_baseline_keeps_paragraph_and_direct_precedence() {
    for operand in [0, 1, 128, 129] {
        let mut f = Fixture::new();
        f.styles.styles[0].as_mut().unwrap().properties[0].bytes = vec![0x3b, 0x08, 1];
        f.formatting.character_spans[0].properties.push(
            crate::office::doc::formatting::PropertyModifier {
                sprm: 0x083b,
                operand: vec![operand],
            },
        );
        let (hidden, table) = f.evaluate().unwrap();
        let text = crate::office::doc::transcribe(&f.source);
        let output = crate::office::doc::capitalization::inspect(
            &text,
            &crate::office::doc::capitalization::Context {
                source: &f.source,
                formatting: &f.formatting,
                styles: &f.styles,
                hidden: &hidden,
                revisions: &f.revisions,
                table_properties: Some(&table),
            },
        )
        .unwrap();
        assert_eq!(output.spans[0].all_caps, Some(matches!(operand, 1 | 128)));
    }
}

#[test]
fn list_membership_proof_requires_exact_cell_paragraph_and_empty_style() {
    let f = Fixture::new();
    let (_, proof) = f.evaluate().unwrap();
    assert!(proof.permits_list_paragraph([0, 2]));
    for range in [[0, 1], [1, 2], [2, 3], [0, 3], [3, 4]] {
        assert!(!proof.permits_list_paragraph(range));
    }
    let mut conditional = Fixture::new();
    conditional.styles.styles[11]
        .as_mut()
        .unwrap()
        .properties
        .iter_mut()
        .find(|g| g.kind == "character")
        .unwrap()
        .bytes = vec![0x3c, 0x08, 0];
    assert!(
        !conditional
            .evaluate()
            .unwrap()
            .1
            .permits_list_paragraph([0, 2])
    );
    let mut unknown = Fixture::new();
    unknown.revisions.spans[0].properties.deleted = None;
    assert!(!unknown.evaluate().unwrap().1.permits_list_paragraph([0, 2]));
}

#[test]
fn table_alignment_and_header_repeat_preserve_only_empty_style_visibility() {
    for operation in [
        vec![0x8a, 0x54, 0, 0],
        vec![0x8a, 0x54, 1, 0],
        vec![0x8a, 0x54, 2, 0],
        vec![0x04, 0x34, 0],
        vec![0x04, 0x34, 1],
    ] {
        let mut f = Fixture::new();
        f.paragraphs.spans[1].raw_modifiers.extend(operation);
        let (hidden, table) = f.evaluate().unwrap();
        assert!(table.rows[0].empty_text_contribution);
        assert!(table.permits_list_paragraph([0, 2]));
        assert!(
            hidden
                .spans
                .iter()
                .all(|s| s.property.hidden == Some(false))
        );
    }
}
#[test]
fn table_alignment_and_header_repeat_reject_invalid_operands_and_keep_outside_text() {
    for operation in [
        vec![0x8a, 0x54, 3, 0],
        vec![0x8a, 0x54, 0, 1],
        vec![0x8a, 0x54, 255, 255],
        vec![0x04, 0x34, 2],
        vec![0x04, 0x34, 128],
        vec![0x04, 0x34, 129],
    ] {
        let mut f = Fixture::new();
        f.paragraphs.spans[1].raw_modifiers.extend(operation);
        let (hidden, table) = f.evaluate().unwrap();
        assert!(!table.rows[0].empty_text_contribution);
        assert_eq!(hidden.spans[0].property.hidden, None);
        assert_eq!(hidden.spans.last().unwrap().property.hidden, Some(false));
    }
    for operation in [vec![0x8a, 0x54], vec![0x8a, 0x54, 1], vec![0x04, 0x34]] {
        let mut f = Fixture::new();
        f.paragraphs.spans[1].raw_modifiers.extend(operation);
        assert!(f.evaluate().is_err());
    }
}
#[test]
fn header_alignment_keep_conditional_character_revision_and_membership_guards() {
    for operation in [vec![0x8a, 0x54, 1, 0], vec![0x04, 0x34, 1]] {
        for case in 0..5 {
            let mut f = Fixture::new();
            f.paragraphs.spans[1]
                .raw_modifiers
                .extend(operation.clone());
            match case {
                0 => {
                    f.styles.styles[11].as_mut().unwrap().properties[2].bytes =
                        vec![0x85, 0xca, 5, 1, 0, 0x3c, 0x08, 1]
                }
                1 => {
                    f.styles.styles[11].as_mut().unwrap().properties[2].bytes = vec![0x3c, 0x08, 0]
                }
                2 => f.revisions.spans[0].properties.deleted = Some(true),
                3 => f.paragraphs.spans[1].raw_modifiers.extend([0x68, 0x36, 1]),
                4 => f.paragraphs.spans[1]
                    .raw_modifiers
                    .extend([0x2b, 0xd6, 2, 0, 1]),
                _ => unreachable!(),
            }
            let (_, table) = f.evaluate().unwrap();
            assert_eq!(table.rows[0].empty_text_contribution, case == 4);
            assert!(!table.permits_list_paragraph([0, 2]));
        }
    }
}
#[test]
fn header_alignment_piece_order_does_not_erase_malformed_earlier_properties() {
    let mut f = Fixture::new();
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x8a, 0x54, 1, 0, 0x04, 0x34, 1]);
    f.formatting
        .raw_property_groups
        .push(vec![0x8a, 0x54, 2, 0, 0x04, 0x34, 0]);
    f.formatting.pieces[0].raw_prm = 1;
    assert!(f.evaluate().unwrap().1.rows[0].empty_text_contribution);
    f.paragraphs.spans[1].raw_modifiers.extend([0x04, 0x34, 2]);
    assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
}

#[test]
fn leading_table_width_valid_units_preserve_empty_style_visibility() {
    for (unit, width) in [(0, 0u16), (0, 65535), (1, 0), (2, 0), (2, 5000), (3, 31680)] {
        let mut f = Fixture::new();
        f.paragraphs.spans[1]
            .raw_modifiers
            .extend([0x17, 0xf6, unit]);
        f.paragraphs.spans[1]
            .raw_modifiers
            .extend(width.to_le_bytes());
        let (hidden, table) = f.evaluate().unwrap();
        assert!(table.rows[0].empty_text_contribution);
        assert!(table.permits_list_paragraph([0, 2]));
        assert!(
            hidden
                .spans
                .iter()
                .all(|s| s.property.hidden == Some(false))
        );
    }
}

#[test]
fn leading_table_width_invalid_units_ranges_and_framing_stay_unresolved() {
    for (unit, width) in [
        (1, 1u16),
        (2, 5001),
        (2, 65535),
        (3, 31681),
        (3, 65535),
        (4, 0),
        (0x13, 0),
    ] {
        let mut f = Fixture::new();
        f.paragraphs.spans[1]
            .raw_modifiers
            .extend([0x17, 0xf6, unit]);
        f.paragraphs.spans[1]
            .raw_modifiers
            .extend(width.to_le_bytes());
        let (hidden, table) = f.evaluate().unwrap();
        assert!(!table.rows[0].empty_text_contribution);
        assert_eq!(hidden.spans[0].property.hidden, None);
        assert_eq!(hidden.spans.last().unwrap().property.hidden, Some(false));
    }
    for bytes in [vec![], vec![0], vec![0, 0], vec![0, 0, 0, 0]] {
        assert!(!valid_table_part_width(&bytes));
    }
    for bytes in [
        vec![0x17, 0xf6],
        vec![0x17, 0xf6, 0],
        vec![0x17, 0xf6, 0, 0],
    ] {
        let mut f = Fixture::new();
        f.paragraphs.spans[1].raw_modifiers.extend(bytes);
        assert!(f.evaluate().is_err());
    }
}

#[test]
fn leading_table_width_retains_style_revision_and_membership_guards() {
    for case in 0..5 {
        let mut f = Fixture::new();
        f.paragraphs.spans[1]
            .raw_modifiers
            .extend([0x17, 0xf6, 0, 0, 0]);
        match case {
            0 => {
                f.styles.styles[11].as_mut().unwrap().properties[2].bytes =
                    vec![0x85, 0xca, 5, 1, 0, 0x3c, 0x08, 1]
            }
            1 => f.styles.styles[11].as_mut().unwrap().properties[2].bytes = vec![0x5f, 0x48, 0, 4],
            2 => f.revisions.spans[0].properties.deleted = Some(true),
            3 => f.paragraphs.spans[1].raw_modifiers.extend([0x68, 0x36, 1]),
            4 => f.paragraphs.spans[1]
                .raw_modifiers
                .extend([0x2b, 0xd6, 2, 0, 1]),
            _ => unreachable!(),
        }
        let (_, table) = f.evaluate().unwrap();
        assert_eq!(table.rows[0].empty_text_contribution, case == 4);
        assert!(!table.permits_list_paragraph([0, 2]));
    }
}

#[test]
fn leading_table_width_later_piece_value_does_not_erase_invalid_earlier_operand() {
    let mut f = Fixture::new();
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x17, 0xf6, 3, 0, 0]);
    f.formatting
        .raw_property_groups
        .push(vec![0x17, 0xf6, 0, 255, 255]);
    f.formatting.pieces[0].raw_prm = 1;
    assert!(f.evaluate().unwrap().1.rows[0].empty_text_contribution);
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x17, 0xf6, 1, 1, 0]);
    assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
}

#[test]
fn language_only_table_style_resolves_visibility_without_empty_text_claim() {
    let mut f = Fixture::new();
    f.styles.styles[11].as_mut().unwrap().properties[2].bytes = vec![0x5f, 0x48, 0, 4];
    let (hidden, proof) = f.evaluate().unwrap();
    let row = &proof.rows[0];
    assert!(!row.empty_text_contribution);
    assert!(row.unresolved.is_empty());
    assert!(
        serde_json::to_value(&proof)
            .unwrap()
            .get("language_paragraph_baselines")
            .is_none()
    );
    let language = row.language_contribution.as_ref().unwrap();
    assert_eq!(language.complex_script_language, 0x0400);
    assert_eq!(language.ordered_properties[0].style_index, 11);
    assert_eq!(hidden.spans[0].property.hidden, Some(false));
    assert_eq!(
        hidden.spans[0].table_baseline_basis,
        "selected_row_style_has_language_only_character_properties"
    );
    assert!(!proof.has_empty_text_contribution([0, 2], 1));
    assert!(!proof.permits_list_paragraph([0, 2]));
    assert!(proof.visibility_basis(1).is_none());
    assert!(proof.visibility_basis(4).is_none());
}

#[test]
fn language_table_inheritance_preserves_ancestor_then_child_source_order() {
    let mut f = Fixture::new();
    let mut parent = style(12, None);
    parent.properties[2].bytes = vec![0x5f, 0x48, 9, 4];
    f.styles.styles.push(Some(parent));
    let child = f.styles.styles[11].as_mut().unwrap();
    child.base_style = Some(12);
    child.properties[2].bytes = vec![0x5f, 0x48, 0, 4];
    let (_, proof) = f.evaluate().unwrap();
    let language = proof.rows[0].language_contribution.as_ref().unwrap();
    assert_eq!(proof.rows[0].style_ancestors, [11, 12]);
    assert_eq!(
        language
            .ordered_properties
            .iter()
            .map(|p| p.style_index)
            .collect::<Vec<_>>(),
        [12, 11]
    );
    assert_eq!(language.complex_script_language, 0x0400);
}

#[test]
fn language_table_baseline_retains_direct_hidden_and_capitalization_uncertainty() {
    let mut f = Fixture::new();
    f.styles.styles[11].as_mut().unwrap().properties[2].bytes = vec![0x5f, 0x48, 0, 4];
    f.formatting.character_spans[0].properties.push(
        crate::office::doc::formatting::PropertyModifier {
            sprm: 0x083c,
            operand: vec![1],
        },
    );
    let (hidden, proof) = f.evaluate().unwrap();
    assert_eq!(hidden.spans[0].property.hidden, Some(true));
    let text = crate::office::doc::transcribe(&f.source);
    let caps = crate::office::doc::capitalization::inspect(
        &text,
        &crate::office::doc::capitalization::Context {
            source: &f.source,
            formatting: &f.formatting,
            styles: &f.styles,
            hidden: &hidden,
            revisions: &f.revisions,
            table_properties: Some(&proof),
        },
    )
    .unwrap();
    assert_eq!(caps.spans[0].all_caps, None);
    assert_eq!(caps.spans[0].small_caps, None);
    assert_eq!(caps.spans[0].hidden, Some(true));
}

#[test]
fn conditional_table_text_resolves_visibility_but_preserves_history_and_identity_guards() {
    for case in 0..7 {
        let mut f = Fixture::new();
        f.styles.styles[11].as_mut().unwrap().properties[2].bytes = vec![0x5f, 0x48, 0, 4];
        match case {
            0 => f.styles.styles[11].as_mut().unwrap().properties[2]
                .bytes
                .extend([0x3c, 0x08, 0]),
            1 => f.styles.styles[11].as_mut().unwrap().properties[2]
                .bytes
                .extend([0x85, 0xca, 5, 1, 0, 0x3c, 0x08, 1]),
            2 => f.styles.styles[11].as_mut().unwrap().revision_marked = true,
            3 => f.styles.styles[11].as_mut().unwrap().kind = 1,
            4 => f.styles.styles[11].as_mut().unwrap().linked_style = Some(0),
            5 => f.revisions.spans[0].properties.deleted = None,
            6 => f.structure.stories[0].paragraphs[0]
                .unresolved
                .push("synthetic_unknown"),
            _ => unreachable!(),
        }
        let (hidden, proof) = f.evaluate().unwrap();
        assert!(proof.rows[0].language_contribution.is_none());
        assert!(proof.visibility_basis(2).is_none());
        if case < 2 {
            assert_eq!(hidden.spans[0].property.hidden, Some(false), "case {case}");
            assert!(!proof.rows[0].conditional_cell_text.is_empty());
        } else if case != 3 {
            assert_eq!(hidden.spans[0].property.hidden, None, "case {case}");
        }
        assert!(!proof.has_empty_text_contribution([0, 2], 1));
    }
}

#[test]
fn exhausted_language_budget_keeps_independent_empty_style_proof_available() {
    let mut f = Fixture::new();
    let mut language = style(12, None);
    language.properties[2].bytes = vec![0x5f, 0x48, 0, 4];
    f.styles.styles.push(Some(language));
    assert!(language_style(12, &f.styles, &mut 0).is_err());
    assert_eq!(
        empty_style(11, &f.styles, &mut BTreeMap::new(), &mut 4096).unwrap(),
        Some(vec![11])
    );
    let (_, proof) = f.evaluate().unwrap();
    let serialized = serde_json::to_value(&proof.rows[0]).unwrap();
    assert!(serialized.get("language_contribution").is_none());
    assert!(proof.rows[0].empty_text_contribution);
}

#[test]
fn table_placement_scalar_bounds_and_ignored_anchor_padding() {
    for value in 0..=255 {
        assert!(valid_table_placement(0x360d, &[value]));
    }
    for sprm in [0x940e, 0x940f] {
        for value in [-31679i16, -20, -16, -12, -8, -4, 0, 1, 31681] {
            assert!(valid_table_placement(sprm, &value.to_le_bytes()));
        }
        for value in [i16::MIN, -31680, 31682, i16::MAX] {
            assert!(!valid_table_placement(sprm, &value.to_le_bytes()));
        }
    }
    for sprm in [0x9410, 0x941e] {
        for value in [0u16, 181, 187, 31680] {
            assert!(valid_table_placement(sprm, &value.to_le_bytes()));
        }
        for value in [31681u16, 65535] {
            assert!(!valid_table_placement(sprm, &value.to_le_bytes()));
        }
    }
    for sprm in [0x360d, 0x940e, 0x940f, 0x9410, 0x941e] {
        assert!(!valid_table_placement(sprm, &[]));
        assert!(!valid_table_placement(sprm, &[0, 0, 0]));
    }
}

#[test]
fn positioned_table_preserves_only_source_qualified_visibility() {
    for case in 0..5 {
        let mut f = Fixture::new();
        f.paragraphs.spans[1].raw_modifiers.extend([
            0x0d, 0x36, 0x5f, 0x0e, 0x94, 0xfc, 0xff, 0x0f, 0x94, 150, 1, 0x10, 0x94, 181, 0, 0x1e,
            0x94, 181, 0,
        ]);
        match case {
            1 => {
                f.styles.styles[11].as_mut().unwrap().properties[2].bytes =
                    vec![0x85, 0xca, 5, 1, 0, 0x3c, 0x08, 1]
            }
            2 => f.revisions.spans[0].properties.deleted = Some(true),
            3 => f.paragraphs.spans[1].raw_modifiers.extend([0x68, 0x36, 1]),
            4 => f.styles.styles[11].as_mut().unwrap().revision_marked = true,
            _ => {}
        }
        let (_, table) = f.evaluate().unwrap();
        assert_eq!(table.rows[0].empty_text_contribution, case == 0);
        assert_eq!(table.permits_list_paragraph([0, 2]), case == 0);
    }
}

#[test]
fn table_placement_later_valid_value_keeps_invalid_earlier_value_unresolved() {
    let mut f = Fixture::new();
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x10, 0x94, 0xff, 0xff, 0x10, 0x94, 187, 0]);
    let (hidden, table) = f.evaluate().unwrap();
    assert!(!table.rows[0].empty_text_contribution);
    assert_eq!(hidden.spans[0].property.hidden, None);
    assert_eq!(hidden.spans.last().unwrap().property.hidden, Some(false));
    let mut truncated = Fixture::new();
    truncated.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x0f, 0x94, 1]);
    assert!(truncated.evaluate().is_err());
}

#[test]
fn legacy_border_colors_resolve_visibility_only_after_exact_cell_ownership() {
    let mut f = Fixture::new();
    let mut borders = vec![0x05, 0xd6, 24];
    borders.extend([4, 1, 0, 0].repeat(6));
    f.paragraphs.spans[1].raw_modifiers.extend(borders);
    for side in 0x1a..=0x1d {
        f.paragraphs.spans[1]
            .raw_modifiers
            .extend([side, 0xd6, 4, 1, 2, 3, 0]);
    }
    let (hidden, table) = f.evaluate().unwrap();
    assert!(table.rows[0].empty_text_contribution);
    assert!(
        hidden
            .spans
            .iter()
            .all(|span| span.property.hidden == Some(false))
    );
    f.styles.styles[11].as_mut().unwrap().properties[2].bytes = vec![0x3c, 0x08, 1];
    assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
}

#[test]
fn border_color_cardinality_and_order_cannot_create_visibility_evidence() {
    for operand in [vec![4, 1, 2, 3, 1], vec![8, 0, 0, 0, 0, 0, 0, 0, 0]] {
        let mut f = Fixture::new();
        f.paragraphs.spans[1].raw_modifiers.extend([0x1a, 0xd6]);
        f.paragraphs.spans[1].raw_modifiers.extend(operand);
        assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
    }
    let mut f = Fixture::new();
    let original = f.paragraphs.spans[1].raw_modifiers.clone();
    f.paragraphs.spans[1].raw_modifiers = [vec![0x1a, 0xd6, 4, 0, 0, 0, 255], original].concat();
    assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
    assert!(!borders::legacy_array(&[24; 25]));
    assert!(!borders::legacy_array(&[0; 24]));
    assert!(borders::legacy_array(&[vec![24], vec![255; 24]].concat()));
}

#[test]
fn logical_border_validation_preserves_visibility_for_all_table_decorations() {
    for kind in [
        0, 1, 3, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 255,
    ] {
        let mut f = Fixture::new();
        f.paragraphs.spans[1].raw_modifiers.extend([0x05, 0xd6, 24]);
        f.paragraphs.spans[1]
            .raw_modifiers
            .extend([255, kind, 16, 255].repeat(6));
        assert!(
            f.evaluate().unwrap().1.rows[0].empty_text_contribution,
            "kind {kind}"
        );
    }
    for border in [
        [4, 2, 0, 0],
        [4, 4, 0, 0],
        [4, 26, 0, 0],
        [4, 27, 0, 0],
        [4, 64, 0, 0],
        [4, 1, 17, 0],
    ] {
        let mut f = Fixture::new();
        f.paragraphs.spans[1].raw_modifiers.extend([0x05, 0xd6, 24]);
        f.paragraphs.spans[1].raw_modifiers.extend(border.repeat(6));
        assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
    }
}

#[test]
fn table_appearance_preserves_selected_style_but_not_hidden_style_content() {
    let mut f = Fixture::new();
    f.paragraphs.spans[1].raw_modifiers.extend([
        0x00, 0x54, 2, 0, 0x39, 0xd6, 3, 0, 1, 1, 0x42, 0xd6, 3, 0, 1, 1, 0x33, 0xd6, 6, 0, 1, 15,
        0x13, 0xe0, 0x3d, 0x60, 0xd6, 10, 1, 2, 3, 0, 4, 5, 6, 255, 0x3d, 0,
    ]);
    let (hidden, table) = f.evaluate().unwrap();
    assert!(table.permits_list_paragraph([0, 2]));
    assert!(
        hidden
            .spans
            .iter()
            .all(|span| span.property.hidden == Some(false))
    );
    f.styles.styles[11].as_mut().unwrap().properties[2].bytes = vec![0x3c, 0x08, 1];
    assert!(!f.evaluate().unwrap().1.permits_list_paragraph([0, 2]));
}

#[test]
fn table_appearance_invalid_operands_and_future_cells_stay_unresolved() {
    for raw in [
        vec![0x00, 0x54, 3, 0],
        vec![0x39, 0xd6, 3, 0, 2, 1],
        vec![0x42, 0xd6, 3, 0, 1, 2],
        vec![0x33, 0xd6, 6, 0, 1, 15, 3, 0xe1, 0x3d],
        vec![0x33, 0xd6, 6, 0, 1, 14, 3, 0, 0],
        vec![0x33, 0xd6, 6, 0, 1, 15, 0, 1, 0],
        vec![0x60, 0xd6, 10, 0, 0, 0, 0, 0, 0, 0, 0, 26, 0],
    ] {
        let mut f = Fixture::new();
        f.paragraphs.spans[1].raw_modifiers.extend(raw);
        assert!(!f.evaluate().unwrap().1.permits_list_paragraph([0, 2]));
    }
    for opcode in [0x39, 0x42] {
        let mut f = Fixture::new();
        let old = f.paragraphs.spans[1].raw_modifiers.clone();
        f.paragraphs.spans[1].raw_modifiers = [vec![opcode, 0xd6, 3, 0, 1, 1], old].concat();
        assert!(!f.evaluate().unwrap().1.permits_list_paragraph([0, 2]));
    }
}

#[test]
fn merge_baseline_retains_conditional_and_revision_guards() {
    for extra in [vec![], vec![0x66, 0xc6, 2, 1, 0]] {
        let mut f = Fixture::new();
        f.paragraphs.spans[1]
            .raw_modifiers
            .extend([0x2b, 0xd6, 2, 0, 3]);
        if !extra.is_empty() {
            f.styles.styles[11].as_mut().unwrap().properties[1]
                .bytes
                .extend(extra);
            assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
        } else {
            let (hidden, table) = f.evaluate().unwrap();
            assert_eq!(hidden.spans[0].property.hidden, Some(false));
            assert!(table.rows[0].empty_text_contribution);
            assert!(!table.permits_list_paragraph([0, 2]));
            f.revisions.spans[0].properties.inserted = Some(true);
            assert!(!f.evaluate().unwrap().1.rows[0].empty_text_contribution);
        }
    }
}

fn assert_merge_baseline_only(f: &Fixture) {
    let (hidden, table) = f.evaluate().unwrap();
    assert!(table.rows[0].empty_text_contribution);
    assert_eq!(hidden.spans[0].property.hidden, Some(false));
    assert_eq!(
        table.rows[0].physical_ownership_unresolved,
        ["merged_cell_visible_ownership_unresolved"]
    );
    assert!(!table.permits_list_paragraph([0, 2]));
}

#[test]
fn known_hidden_baseline_does_not_project_continuation_form_values() {
    use crate::office::doc::form_fields::{FormData, FormField, FormInspection};
    for (horizontal, merge) in [(0u8, 0u8), (0, 1), (0, 3), (1, 0), (2, 0), (3, 0)] {
        let mut f = Fixture::new();
        f.source.stories[0].text = "\u{13}\u{1}\u{15}\u{7}\u{7}\r".into();
        f.source.stories[0].utf16_units = 6;
        f.formatting.authored_story_end_cp = 6;
        f.formatting.pieces.last_mut().unwrap().end_cp = 6;
        f.formatting.character_spans.last_mut().unwrap().end_cp = 6;
        f.revisions.spans.last_mut().unwrap().end_cp = 6;
        for (index, (start, end)) in [(0, 4), (4, 5), (5, 6)].into_iter().enumerate() {
            f.paragraphs.spans[index].start_cp = start;
            f.paragraphs.spans[index].end_cp = end;
            let p = &mut f.structure.stories[0].paragraphs[index];
            p.source_range = [start, end];
            p.projected_bytes = Some([start, end]);
            p.endpoint.as_mut().unwrap().end_cp = end;
            p.endpoint.as_mut().unwrap().end_fc = end;
        }
        f.paragraphs.spans[1]
            .raw_modifiers
            .extend([0x2b, 0xd6, 2, 0, 1, 0x2b, 0xd6, 2, 0, merge]);
        if horizontal != 0 {
            let mut definition = vec![26, 0, 1, 0, 0, 100, 0];
            definition.extend([horizontal, 0]);
            definition.extend([0; 18]);
            f.paragraphs.spans[1].raw_modifiers.extend([0x08, 0xd6]);
            f.paragraphs.spans[1].raw_modifiers.extend(definition);
        }
        let continuation = merge == 1 || horizontal == 1;
        let (hidden, table) = f.evaluate().unwrap();
        assert_eq!(hidden.spans[0].property.hidden, Some(false));
        let mut forms = FormInspection {
            status: "synthetic",
            unresolved: vec![],
            fields: vec![FormField {
                identity: None,
                story: "body",
                field_start_cp: 0,
                field_end_cp: 3,
                marker_cp: Some(1),
                data_offset: Some(0),
                data_bytes: Some(100),
                field_type: "checkbox",
                eligible: true,
                diagnostic: None,
                data: Some(FormData {
                    legacy_encoding: None,
                    kind: "checkbox",
                    raw_bits: 101,
                    raw_current_state: 25,
                    default_state: Some(0),
                    effective_checked: Some(false),
                    selected_index: None,
                    selected_text: None,
                    name: String::new(),
                    default_text: None,
                    text_format: String::new(),
                    help_text: String::new(),
                    status_text: String::new(),
                    entry_macro: String::new(),
                    exit_macro: String::new(),
                    items: vec![],
                }),
            }],
        };
        let raw_value = serde_json::to_value(&forms.fields[0].data).unwrap();
        table.retain_form_display_uncertainty(&mut forms);
        assert_eq!(forms.fields[0].eligible, !continuation);
        assert_eq!(
            serde_json::to_value(&forms.fields[0].data).unwrap(),
            raw_value
        );
        let input = crate::office::doc::transcribe(&f.source);
        let output = crate::office::doc::form_fields::apply_forms(&input, &forms).unwrap();
        assert_eq!(
            output.stories[0]
                .segments
                .iter()
                .filter(|s| s.kind == "generated_form_value")
                .count(),
            usize::from(!continuation)
        );
        if continuation {
            assert!(
                forms.fields[0]
                    .diagnostic
                    .as_ref()
                    .unwrap()
                    .contains("merged_continuation")
            );
            assert_eq!(output.stories[0].text, input.stories[0].text);
        }
    }
}

mod conditional_cells;

fn nested_fixture() -> Fixture {
    let mut f = Fixture::new();
    f.source.stories[0].text = "A\rB\r\rC\u{7}\u{7}\r".into();
    f.source.stories[0].utf16_units = 9;
    f.formatting.authored_story_end_cp = 9;
    f.formatting.pieces[0].end_cp = 9;
    f.formatting.character_spans[0].end_cp = 9;
    let definitions = [
        (0, 2, 13, 1, false, false),
        (2, 4, 13, 2, true, false),
        (4, 5, 13, 2, true, true),
        (5, 7, 7, 1, false, false),
        (7, 8, 7, 1, false, true),
        (8, 9, 13, 0, false, false),
    ];
    f.paragraphs.spans.clear();
    f.structure.stories[0].paragraphs.clear();
    for (a, b, terminal, depth, cell, row) in definitions {
        let mut raw = if depth == 0 {
            vec![]
        } else {
            vec![0x16, 0x24, 1, 0x49, 0x66, depth, 0, 0, 0]
        };
        if depth == 2 {
            raw.extend([0x4b, 0x24, u8::from(cell), 0x4c, 0x24, u8::from(row)]);
        }
        if row {
            if depth == 1 {
                raw.extend([0x17, 0x24, 1]);
            }
            raw.extend([
                0x21,
                0x76,
                0,
                1,
                10,
                0,
                0x3a,
                0x56,
                if depth == 1 { 11 } else { 12 },
                0,
            ]);
        }
        f.paragraphs.spans.push(ParagraphSpan {
            start_cp: a,
            end_cp: b,
            style_index: Some(0),
            raw_modifiers: raw,
        });
        f.structure.stories[0].paragraphs.push(ParagraphOwnership {
            source_range: [a, b],
            projected_bytes: Some([a, b]),
            terminal_unit: Some(terminal),
            endpoint: Some(ParagraphEndpoint {
                end_cp: b,
                piece_index: 0,
                fkp_offset: 0,
                fkp_run: 0,
                end_fc: b,
            }),
            terminal_projection: "synthetic",
            unresolved: vec![],
        });
    }
    if f.styles.styles.len() < 13 {
        f.styles.styles.resize_with(13, || None);
    }
    f.styles.styles[12] = Some(style(12, None));
    f.revisions = crate::office::doc::visibility::resolve_revision_evidence(&f.formatting).unwrap();
    f
}

#[test]
fn nested_empty_styles_bind_innermost_rows_without_inheriting_parent_visibility() {
    let mut f = nested_fixture();
    let (hidden, table) = f.evaluate().unwrap();
    assert!(
        hidden
            .spans
            .iter()
            .all(|s| s.property.hidden == Some(false))
    );
    assert_eq!(table.rows.len(), 2);
    assert_eq!(table.rows[0].selected_style, Some(12));
    assert_eq!(table.rows[1].selected_style, Some(11));
    assert_eq!(table.paragraph_baselines.get(&4), Some(&0));
    assert_eq!(table.paragraph_baselines.get(&2), Some(&1));
    assert_eq!(table.paragraph_baselines.get(&7), Some(&1));
    f.styles.styles[12]
        .as_mut()
        .unwrap()
        .properties
        .iter_mut()
        .find(|p| p.kind == "character")
        .unwrap()
        .bytes = vec![0x3c, 0x08, 1];
    let (hidden, table) = f.evaluate().unwrap();
    assert!(!table.rows[0].empty_text_contribution);
    assert!(table.rows[1].empty_text_contribution);
    assert!(
        hidden
            .spans
            .iter()
            .filter(|s| s.start_cp >= 2 && s.end_cp <= 5)
            .all(|s| s.property.hidden.is_none())
    );
    assert!(
        hidden
            .spans
            .iter()
            .filter(|s| s.end_cp <= 2 || (s.start_cp >= 5 && s.end_cp <= 8))
            .all(|s| s.property.hidden == Some(false))
    );
}

#[test]
fn nested_row_mark_requires_inner_flag_and_recorded_depth() {
    for mutation in [0, 1] {
        let mut f = nested_fixture();
        if mutation == 0 {
            f.paragraphs.spans[2].raw_modifiers[14] = 0;
        } else {
            f.paragraphs.spans[2].raw_modifiers[5] = 1;
        }
        let (_, table) = f.evaluate().unwrap();
        assert!(table.rows.iter().all(|r| !r.empty_text_contribution));
    }
}
