use super::*;

#[test]
fn active_header_condition_changes_only_cell_text_before_direct_override() {
    let mut f = Fixture::new();
    f.styles.styles[11].as_mut().unwrap().properties[2].bytes =
        vec![0x85, 0xca, 5, 1, 0, 0x3c, 0x08, 1];
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x0a, 0x74, 0, 0, 0x20, 0]);
    let (hidden, proof) = f.evaluate().unwrap();
    assert_eq!(hidden.spans[0].property.hidden, Some(true));
    assert_eq!(proof.rows[0].conditional_cell_text[0].conditions, [1]);
    assert!(proof.conditional_text(3).is_none());
    f.formatting.character_spans[0].properties.push(
        crate::office::doc::formatting::PropertyModifier {
            sprm: 0x083c,
            operand: vec![0],
        },
    );
    let (hidden, _) = f.evaluate().unwrap();
    assert_eq!(hidden.spans[0].property.hidden, Some(false));
}

#[test]
fn effective_run_budget_stops_before_retention_and_population_is_single_use() {
    let mut f = Fixture::new();
    f.styles.styles[11].as_mut().unwrap().properties[2].bytes =
        vec![0x85, 0xca, 5, 1, 0, 0x35, 0x08, 1];
    let (hidden, mut proof) = f.evaluate().unwrap();
    let fonts = crate::office::doc::symbols::FontTable {
        status: "fixture",
        fonts: vec![],
    };
    let error = conditional::populate_with_budget(
        &mut proof,
        &hidden,
        &f.formatting,
        &f.styles,
        &fonts,
        &mut (8 * 1024 * 1024),
    )
    .unwrap_err();
    assert!(matches!(error, Error::Limit(_)));
    assert!(
        proof.rows[0].conditional_cell_text[0]
            .effective_runs
            .is_empty()
    );
    proof
        .resolve_conditional_runs(&hidden, &f.formatting, &f.styles, &fonts)
        .unwrap();
    assert_eq!(
        proof.rows[0].conditional_cell_text[0].effective_runs.len(),
        1
    );
    assert!(
        proof
            .resolve_conditional_runs(&hidden, &f.formatting, &f.styles, &fonts)
            .is_err()
    );
    assert_eq!(
        proof.rows[0].conditional_cell_text[0].effective_runs.len(),
        1
    );
}
