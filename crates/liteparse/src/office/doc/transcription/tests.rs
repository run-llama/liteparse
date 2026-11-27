use super::*;

fn story(text: &str) -> Story {
    Story {
        kind: "body",
        start_cp: 17,
        utf16_units: text.encode_utf16().count(),
        text: text.into(),
    }
}

fn check_partition(source: &Story, output: &TranscribedStory) {
    let mut cp = source.start_cp;
    let mut byte = 0;
    for span in &output.segments {
        assert_eq!(span.start_cp, cp);
        assert_eq!(span.output_start, byte);
        assert!(span.end_cp > cp);
        assert!(
            output
                .text
                .get(span.output_start..span.output_end)
                .is_some()
        );
        cp = span.end_cp;
        byte = span.output_end;
    }
    assert_eq!(cp, source.start_cp + source.utf16_units);
    assert_eq!(byte, output.text.len());
}

#[test]
fn nested_results_respect_all_enclosing_instruction_ranges() {
    let source =
        story("A\u{13}IF \u{13}PAGE\u{14}2\u{15} > 1\u{14}yes \u{13}DATE\u{14}today\u{15}\u{15}Z");
    let output = project_story(&source);
    assert_eq!(output.text, "Ayes todayZ");
    assert!(output.fields_balanced);
    assert!(output.diagnostics.is_empty());
    check_partition(&source, &output);
}

#[test]
fn fields_without_cached_results_are_retained_as_source_spans() {
    let source = story("a\u{13}SET x y\u{15}b");
    let output = project_story(&source);
    assert_eq!(output.text, "ab");
    let instruction = output
        .segments
        .iter()
        .find(|s| s.kind == "field_instruction")
        .unwrap();
    assert_eq!(instruction.end_cp - instruction.start_cp, 7);
    assert_eq!(instruction.output_start, instruction.output_end);
    check_partition(&source, &output);
}

#[test]
fn malformed_fields_preserve_ambiguous_instructions() {
    for (text, code) in [
        ("a\u{13}CODE", "unclosed_field"),
        ("a\u{14}b", "orphan_field_separator"),
        ("a\u{15}b", "orphan_field_end"),
        (
            "\u{13}CODE\u{14}a\u{14}b\u{15}",
            "duplicate_field_separator",
        ),
    ] {
        let source = story(text);
        let output = project_story(&source);
        assert!(!output.fields_balanced);
        assert_eq!(output.diagnostics[0].code, code);
        assert!(
            output
                .segments
                .iter()
                .all(|s| s.kind != "field_instruction")
        );
        assert!(output.unresolved_control_units > 0);
        if text.contains("CODE") {
            assert!(output.text.contains("CODE"));
        }
        check_partition(&source, &output);
    }
}

#[test]
fn structural_controls_keep_distinct_source_ownership() {
    let source = story("a\tb\rc\u{7}d\u{b}e\u{c}f\u{e}g\u{1e}h\u{1f}i");
    let output = project_story(&source);
    assert_eq!(output.text, "a\tb\nc\td\ne\u{c}f\ng\u{2011}h\u{ad}i");
    assert!(output.segments.iter().any(|s| s.kind == "cell_or_row_mark"));
    assert!(output.segments.iter().any(|s| s.kind == "paragraph"));
    check_partition(&source, &output);
}

#[test]
fn unresolved_notes_symbols_objects_keep_placeholders() {
    let source = story("a\u{1}\u{2}\u{5}\u{8}\u{28}\u{0}z");
    let output = project_story(&source);
    assert_eq!(output.unresolved_control_units, 5);
    // A symbol character can be printable; its font-dependent interpretation remains unresolved.
    assert!(output.text.contains('('));
    assert_eq!(output.text.matches('\u{fffc}').count(), 5);
    check_partition(&source, &output);
}

#[test]
fn supplementary_characters_use_two_source_units_and_utf8_output_ranges() {
    let source = story("😀\u{13}X\u{14}𐐀\u{15}終");
    let output = project_story(&source);
    assert_eq!(output.text, "😀𐐀終");
    assert_eq!(output.segments[0].end_cp, 19);
    assert_eq!(output.segments[0].output_end, 4);
    check_partition(&source, &output);
}

#[test]
fn empty_stories_and_fields_are_supported() {
    for text in ["", "\u{13}\u{15}", "\u{13}\u{14}\u{15}"] {
        let source = story(text);
        let output = project_story(&source);
        assert!(output.text.is_empty());
        assert!(output.fields_balanced);
        check_partition(&source, &output);
    }
}

#[test]
fn output_always_discloses_unresolved_visibility_and_trailing_source() {
    let inspection = Inspection {
        schema_version: 1,
        status: "raw_stories_only",
        binary_version: 0x112,
        trailing_container_bytes: 0,
        piece_count: 1,
        stories: vec![story("ordinary text")],
        trailing_utf16_units: vec![13],
    };
    let output = transcribe(&inspection);
    assert_eq!(output.status, "partial_transcription");
    assert!(
        output
            .unresolved
            .contains(&"character_properties_and_revision_visibility")
    );
    assert!(
        output
            .unresolved
            .contains(&"trailing_source_units_retained_in_inspection")
    );
}

#[test]
fn deep_fields_and_excessive_diagnostics_are_bounded() {
    for text in ["\u{13}".repeat(MAX_FIELD_DEPTH + 1), "\u{14}".repeat(4096)] {
        let source = story(&text);
        let output = project_story(&source);
        assert!(!output.fields_balanced);
        assert!(output.diagnostics.len() <= MAX_DIAGNOSTICS + 1);
        assert!(output.diagnostics.iter().any(|d| d.code.ends_with("limit")));
        check_partition(&source, &output);
    }
}

#[test]
fn segment_limit_preserves_tail_source_and_reports_incomplete_projection() {
    let source = story(&"x\r".repeat(MAX_SEGMENTS));
    let output = project_story(&source);
    assert_eq!(output.segments.len(), MAX_SEGMENTS + 1);
    assert_eq!(output.segments.last().unwrap().kind, "unprojected_tail");
    assert!(output.diagnostics.iter().any(|d| d.code == "segment_limit"));
    check_partition(&source, &output);
}

#[test]
fn macrobutton_without_cached_result_retains_display_argument() {
    for (input, expected) in [
        (
            "\u{13} MACROBUTTON NoMacro [Enter value here]\u{15}",
            "[Enter value here]",
        ),
        (
            "\u{13}macrobutton \"Macro Name\" \"Click here 😀\" \u{15}",
            "Click here 😀",
        ),
        ("\u{13}MACROBUTTON Run display\u{14}cached\u{15}", "cached"),
    ] {
        let source = story(input);
        let output = project_story(&source);
        assert_eq!(output.text, expected);
        assert!(output.diagnostics.is_empty());
        check_partition(&source, &output);
    }
}

#[test]
fn macrobutton_respects_enclosing_field_instruction_visibility() {
    let source = story(
        "\u{13}IF \u{13}MACROBUTTON Run hidden\u{15}\u{14}show \u{13}MACROBUTTON Run visible\u{15}\u{15}",
    );
    let output = project_story(&source);
    assert_eq!(output.text, "show visible");
    check_partition(&source, &output);
}

#[test]
fn ambiguous_macrobutton_arguments_are_diagnosed() {
    for code in [
        "MACROBUTTON Run",
        "MACROBUTTON \"Run display",
        "MACROBUTTON Run display \\* MERGEFORMAT",
        "MACROBUTTON Run \u{13}REF value\u{14}nested\u{15}",
    ] {
        let source = story(&format!("\u{13}{code}\u{15}"));
        let output = project_story(&source);
        assert!(
            output
                .diagnostics
                .iter()
                .any(|d| d.code == "macrobutton_display_unresolved")
        );
        assert!(output.text.is_empty());
        check_partition(&source, &output);
    }
}

#[test]
fn one_literal_overstrike_operand_preserves_text_and_source_positions() {
    for alignment in ["", "\\al", "\\ac", "\\ar", "\\ad", "\\AD"] {
        let source = story(&format!(
            "😀A\u{13} EQ \\o{alignment}(\u{3000},Label 🦀,   ) \u{15}Z"
        ));
        let output = project_story(&source);
        assert_eq!(output.text, "😀ALabel 🦀Z");
        assert!(output.diagnostics.is_empty());
        let span = output
            .segments
            .iter()
            .find(|s| s.kind == "field_display_argument")
            .unwrap();
        let start = source.text.find("Label").unwrap();
        assert_eq!(
            span.start_cp,
            source.start_cp + source.text[..start].encode_utf16().count()
        );
        assert_eq!(
            span.end_cp - span.start_cp,
            "Label 🦀".encode_utf16().count()
        );
        assert_eq!(&output.text[span.output_start..span.output_end], "Label 🦀");
        check_partition(&source, &output);
    }
}

#[test]
fn overstrike_keeps_literal_spacing_and_accepts_only_one_visible_operand() {
    for code in [" eq \\o\\ad( A B ,　)", "EQ \\o (　, A B , )   "] {
        let source = story(&format!("\u{13}{code}\u{15}"));
        let output = project_story(&source);
        assert_eq!(output.text, " A B ");
        check_partition(&source, &output);
    }
    for code in [
        "EQ \\o(a,-)",
        "EQ \\o\\ad( , )",
        "EQ \\o\\unknown(Label, )",
        "EQ \\o\\ad(Label; )",
        "EQ \\o\\ad(Label, ) trailing",
        "EQ \\o\\ad(\\s\\up2(Label), )",
        "EQ \\o\\ad(\"Label\", )",
        "EQ \\o\\ad(Label, ) \\* MERGEFORMAT",
        "EQ \\o\\ad(Label\rOther, )",
        "EQ \\o\\ad(Label\\,Other, )",
    ] {
        let source = story(&format!("A\u{13}{code}\u{15}Z"));
        let output = project_story(&source);
        assert_eq!(output.text, "AZ");
        assert!(
            output
                .diagnostics
                .iter()
                .any(|d| d.code == "equation_display_unresolved")
        );
        check_partition(&source, &output);
    }
}

#[test]
fn overstrike_preserves_cached_results_and_refuses_nested_equations() {
    let source = story("A\u{13}EQ \\o(Label, )\u{14}Cached\u{15}Z");
    let output = project_story(&source);
    assert_eq!(output.text, "ACachedZ");
    check_partition(&source, &output);
    for text in [
        "\u{13}IF condition\u{14}A\u{13}EQ \\o(Label, )\u{15}Z\u{15}",
        "\u{13}EQ \\o(\u{13}REF target\u{14}Label\u{15}, )\u{15}",
        "\u{13}EQ \\o(\u{13}EQ \\o(Label, )\u{15}, )\u{15}",
    ] {
        let source = story(text);
        let output = project_story(&source);
        assert!(!output.text.contains("Label"));
        assert!(
            output
                .diagnostics
                .iter()
                .any(|d| d.code == "equation_display_unresolved")
        );
        check_partition(&source, &output);
    }
}

#[test]
fn nested_unknown_field_names_do_not_scan_enclosing_instruction_tails() {
    let text = format!(
        "{}{}{}",
        "\u{13}".repeat(1000),
        "x".repeat(200_000),
        "\u{15}".repeat(1000)
    );
    let source = story(&text);
    let output = project_story(&source);
    assert!(output.fields_balanced);
    assert!(output.text.is_empty());
    assert!(output.diagnostics.is_empty());
    check_partition(&source, &output);
}

#[test]
fn recovered_equation_literal_still_obeys_accepted_deletions() {
    use crate::office::doc::visibility::{RevisionEvidence, RevisionProperties, RevisionSpan};
    let source = story("A\u{13}EQ \\o\\ad(Label,　)\u{15}Z");
    let projected = project_story(&source);
    let literal = projected
        .segments
        .iter()
        .find(|s| s.kind == "field_display_argument")
        .unwrap();
    let ranges = [
        (source.start_cp, literal.start_cp, false),
        (literal.start_cp, literal.end_cp, true),
        (literal.end_cp, source.start_cp + source.utf16_units, false),
    ];
    let evidence = RevisionEvidence {
        status: "revision_evidence_only",
        basis: "synthetic",
        hidden_text: "unresolved_style_dependent",
        spans: ranges
            .into_iter()
            .enumerate()
            .map(|(i, (start_cp, end_cp, deleted))| RevisionSpan {
                start_cp,
                end_cp,
                source_span_index: i,
                piece_index: 0,
                properties: RevisionProperties {
                    deleted: Some(deleted),
                    ..RevisionProperties::default()
                },
                evidence: vec![],
                unresolved: vec![],
            })
            .collect(),
    };
    let input = Transcription {
        schema_version: 1,
        status: "partial_transcription",
        unresolved: vec![],
        stories: vec![projected],
    };
    let output =
        crate::office::doc::revision_projection::apply_revisions(&input, &evidence).unwrap();
    assert_eq!(output.stories[0].text, "AZ");
    check_partition(&source, &output.stories[0]);
}

#[test]
fn listnum_empty_result_reports_source_anchor_without_changing_partition() {
    for field in [
        "\u{13}LISTNUM\u{15}",
        "\u{13} listnum \\l 3 \u{15}",
        "\u{13}LISTNUM \\l 3\u{14}\u{15}",
    ] {
        let source = story(&format!("😀{field}tail"));
        let output = project_story(&source);
        assert_eq!(output.text, "😀tail");
        assert!(output.fields_balanced);
        assert_eq!(output.diagnostics.len(), 1);
        assert_eq!(
            output.diagnostics[0].code,
            "listnum_empty_result_unresolved"
        );
        assert_eq!(output.diagnostics[0].cp, 19);
        check_partition(&source, &output);
    }
}

#[test]
fn listnum_cached_nested_lookalike_and_malformed_cases_keep_existing_semantics() {
    for (text, expected) in [
        ("\u{13}LISTNUM \\l 3\u{14}(a)\u{15}", "(a)"),
        ("\u{13}LISTNUM \\l 3\u{14} \u{15}", " "),
        ("\u{13}LISTNUMBER \\l 3\u{15}", ""),
        ("\u{13}IF \u{13}LISTNUM \\l 3\u{15}\u{14}yes\u{15}", "yes"),
    ] {
        let source = story(text);
        let output = project_story(&source);
        assert_eq!(output.text, expected);
        assert!(output.diagnostics.is_empty());
        check_partition(&source, &output);
    }
    let output = project_story(&story("\u{13}LISTNUM "));
    assert!(!output.fields_balanced);
    assert!(
        output
            .diagnostics
            .iter()
            .all(|d| d.code != "listnum_empty_result_unresolved")
    );
}

#[test]
fn repeated_empty_listnum_diagnostics_stay_bounded_and_keep_all_source() {
    let source = story(&"\u{13}LISTNUM\u{15}".repeat(MAX_DIAGNOSTICS + 1));
    let output = project_story(&source);
    assert_eq!(output.diagnostics.len(), MAX_DIAGNOSTICS);
    assert!(output.text.is_empty());
    assert!(output.fields_balanced);
    check_partition(&source, &output);
}

#[test]
fn top_level_listnum_with_child_argument_still_reports_its_empty_result() {
    for field in [
        "\u{13}LISTNUM \u{13}REF level\u{14}3\u{15}\u{15}",
        "\u{13}LISTNUM \u{13}REF level\u{14}3\u{15}\u{14}\u{15}",
    ] {
        let source = story(field);
        let output = project_story(&source);
        assert!(output.text.is_empty());
        assert_eq!(output.diagnostics.len(), 1);
        assert_eq!(
            output.diagnostics[0].code,
            "listnum_empty_result_unresolved"
        );
        assert_eq!(output.diagnostics[0].cp, source.start_cp);
        check_partition(&source, &output);
    }
}
