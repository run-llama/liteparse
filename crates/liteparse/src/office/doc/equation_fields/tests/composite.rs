use super::*;

#[test]
fn absent_cache_overstrike_recovers_each_literal_with_source_owned_ranges() {
    let code = r"eq \o(\s\up 12(午前),\s\do 4(午後))";
    let f = Fixture::new(&[(code, None), (r"eq \o\ac(□,v)", None)]);
    let mut fields = f.equations();
    assert_eq!(fields.fields.len(), 2);
    assert!(
        fields
            .fields
            .iter()
            .all(|f| f.eligible && f.separator_cp.is_none())
    );
    let output = apply_fields(
        &crate::office::doc::transcribe(&f.source),
        &f.source,
        None,
        Some(&mut fields),
    )
    .unwrap();
    assert_eq!(output.stories[0].text, "😀Prefix 午前午後□v suffix\n");
    assert_eq!(
        output.stories[0]
            .segments
            .iter()
            .filter(|s| s.kind == "field_display_argument")
            .map(|s| s.end_cp - s.start_cp)
            .sum::<usize>(),
        6
    );
    for field in &fields.fields {
        assert!(field.diagnostic.as_deref().unwrap().contains("linearized"));
        for operand in &field.composite.as_ref().unwrap().operands {
            let [start, end] = operand.projected_utf8_range.unwrap();
            assert_eq!(&output.stories[0].text[start..end], operand.text);
        }
    }
}
#[test]
fn cached_output_and_unproven_visibility_take_precedence_over_composite_interpretation() {
    let code = r"EQ \o(□,v)";
    let f = Fixture::new(&[(code, Some("saved"))]);
    assert!(f.equations().fields.is_empty());
    assert!(
        crate::office::doc::transcribe(&f.source).stories[0]
            .text
            .contains("saved")
    );
    let mut f = Fixture::new(&[(code, None)]);
    let first = f.fields.stories[0].table.as_ref().unwrap().ranges[0]
        .source
        .start;
    f.revisions
        .spans
        .iter_mut()
        .find(|s| s.start_cp == first)
        .unwrap()
        .properties
        .special = None;
    let fields = f.equations();
    assert!(!fields.fields[0].eligible);
    assert!(fields.fields[0].composite.is_some());
}

#[test]
fn diagnostic_refinement_requires_actual_projection_and_retains_other_unknown_fields() {
    let f = Fixture::new(&[(RUBY, None), (r"EQ \f(a,b)", None), (r"EQ \o(□,v)", None)]);
    let mut ruby = f.ruby();
    let mut equations = f.equations();
    let input = crate::office::doc::transcribe(&f.source);
    assert_eq!(
        input.stories[0]
            .diagnostics
            .iter()
            .filter(|d| d.code == "equation_display_unresolved")
            .count(),
        3
    );
    let output = apply_fields(&input, &f.source, Some(&mut ruby), Some(&mut equations)).unwrap();
    assert_eq!(
        output.stories[0]
            .diagnostics
            .iter()
            .filter(|d| d.code == "equation_display_unresolved")
            .count(),
        1
    );
    assert_eq!(
        output.stories[0]
            .diagnostics
            .iter()
            .filter(|d| d.code == "equation_overstrike_linearized_layout_unresolved")
            .count(),
        1
    );
    assert_eq!(output.stories[0].text, "😀Prefix base□v suffix\n");
}

#[test]
fn one_deleted_operand_blocks_publication_of_the_entire_overstrike() {
    let mut f = Fixture::new(&[(r"EQ \o(first,second)", None)]);
    let inspected = f.equations();
    let cp = inspected.fields[0].composite.as_ref().unwrap().operands[1].source_range[0];
    f.revisions
        .spans
        .iter_mut()
        .find(|s| s.start_cp == cp)
        .unwrap()
        .properties
        .deleted = Some(true);
    let mut fields = f.equations();
    assert!(!fields.fields[0].eligible);
    let input = crate::office::doc::transcribe(&f.source);
    let output = apply_fields(&input, &f.source, None, Some(&mut fields)).unwrap();
    assert_eq!(output.stories[0].text, input.stories[0].text);
    assert!(fields.fields[0].projected_literal_utf8_range.is_none());
    assert!(
        output.stories[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "equation_display_unresolved")
    );
}

#[test]
fn an_already_published_operand_blocks_all_remaining_components() {
    let f = Fixture::new(&[(r"EQ \o(first,second)", None)]);
    let mut fields = f.equations();
    let field = &fields.fields[0];
    let first = &field.composite.as_ref().unwrap().operands[0];
    let input = source_literal_projection::project(
        &crate::office::doc::transcribe(&f.source),
        &f.source,
        &[Candidate {
            story: field.story,
            field_range: field.field_range,
            source_range: first.source_range,
            text: &first.text,
        }],
    )
    .unwrap()
    .transcription;
    let output = apply_fields(&input, &f.source, None, Some(&mut fields)).unwrap();
    assert_eq!(output.stories[0].text, input.stories[0].text);
    assert!(fields.fields[0].projected_literal_utf8_range.is_none());
    assert!(
        fields.fields[0]
            .composite
            .as_ref()
            .unwrap()
            .operands
            .iter()
            .all(|o| o.projected_utf8_range.is_none())
    );
    assert!(
        output.stories[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "equation_display_unresolved")
    );
    assert!(
        fields.fields[0]
            .diagnostic
            .as_deref()
            .unwrap()
            .contains("existing output")
    );
}
