use super::*;
use crate::office::doc::lists::{LevelOverride, ListDefinition, ListOverride};
fn level(size: u8) -> ListLevel {
    ListLevel {
        start_at: 1,
        number_format: 0,
        flags: 0,
        placeholder_offsets: [0; 9],
        following_character: 0,
        saved_indent_twips: 0,
        restart_limit: 0,
        html_flags: 0,
        raw_papx: vec![size],
        raw_chpx: vec![size + 1],
        number_text_utf16: vec![0, 46],
    }
}
fn fixture() -> (ListDefinitions, LabelSource) {
    (
        ListDefinitions {
            schema_version: 1,
            status: "test",
            lists: vec![ListDefinition {
                lsid: 9,
                template_code: 0,
                style_links: [4095; 9],
                flags: 1,
                html_flags: 0,
                levels: vec![level(1)],
            }],
            overrides: vec![ListOverride {
                lsid: 9,
                list_index: 0,
                field_type: 0,
                html_flags: 0,
                first_paragraph_cp: None,
                levels: vec![],
            }],
            appended_level_bytes: 0,
            unresolved: [""; 4],
        },
        LabelSource {
            start_cp: 5,
            list_id: 9,
            override_index: 1,
            level: 0,
        },
    )
}
#[test]
fn complete_format_override_replaces_both_paragraph_and_character_groups() {
    let (mut d, s) = fixture();
    assert_eq!(selected_level(&d, &s).unwrap().raw_chpx, vec![2]);
    d.overrides[0].levels.push(LevelOverride {
        level: 0,
        raw_flags: 32,
        raw_start_at: 99,
        start_override: None,
        format_override: Some(level(7)),
    });
    let v = selected_level(&d, &s).unwrap();
    assert_eq!(v.raw_papx, vec![7]);
    assert_eq!(v.raw_chpx, vec![8]);
}
#[test]
fn counter_only_override_keeps_base_presentation_and_duplicate_identity_rejects() {
    let (mut d, mut s) = fixture();
    d.overrides[0].levels.push(LevelOverride {
        level: 0,
        raw_flags: 16,
        raw_start_at: 99,
        start_override: Some(99),
        format_override: None,
    });
    assert_eq!(selected_level(&d, &s).unwrap().raw_papx, vec![1]);
    s.list_id = 10;
    assert!(selected_level(&d, &s).is_err());
    s.list_id = 9;
    s.override_index = 0;
    assert!(selected_level(&d, &s).is_err());
    s.override_index = 1;
    d.overrides[0].levels.push(LevelOverride {
        level: 0,
        raw_flags: 0,
        raw_start_at: 0,
        start_override: None,
        format_override: None,
    });
    assert!(selected_level(&d, &s).is_err());
}
fn model() -> (Vec<super::super::PlainParagraph>, Vec<PaintLabel>) {
    use crate::office::doc::render_model::{
        paragraphs::{ParagraphProperties, TabStop},
        plain::{PlainParagraph, PlainRun},
        runs::RunProperties,
    };
    let value = RunProperties {
        source_symbol: None,
        fonts: [0; 3],
        size_half_points: 24,
        bold: false,
        italic: false,
        small_caps: false,
        all_caps: false,
        underline: false,
        hidden: false,
        language: 0x409,
        kerning_threshold_half_points: 0,
        color_rgb: None,
        shading_rgb: None,
        highlight_rgb: None,
        scale_percent: 100,
        character_spacing_twips: 0,
        complex: Default::default(),
        hint: 0,
        bidi: false,
        vertical_alignment: 0,
        complex_property_evidence: vec![],
        applied: vec![],
    };
    let p = PlainParagraph {
        cached_fields: Vec::new(),
        source_cp: [0, 2],
        source_style: 0,
        properties: ParagraphProperties {
            justification: 0,
            font_alignment: None,
            left_twips: 720,
            right_twips: 0,
            first_twips: -360,
            before_twips: 0,
            after_twips: 0,
            keep_next: false,
            keep_lines: false,
            page_break_before: false,
            widow_control: true,
            outline_level: 9,
            line_multiple_240ths: 240,
            line_spacing: None,
            tabs: vec![TabStop {
                position_twips: 720,
                alignment: 6,
            }],
            applied: vec![],
        },
        runs: vec![PlainRun {
            source_cp: [0, 2],
            text: "A\r".into(),
            properties: value.clone(),
        }],
    };
    let l = PaintLabel {
        source: Label {
            anchor_cp: 0,
            list_id: 1,
            override_index: 1,
            level: 0,
            text: "1.".into(),
            following_character: 0,
            justification: 0,
            symbol_code: None,
            number_format: 0,
            raw_papx: vec![],
            raw_chpx: vec![],
        },
        source_paragraph_papx: vec![],
        presentation: Presentation {
            candidate_evidence: vec![value.clone(); 3],
            value,
            basis: "paragraph_style_first_text_and_mark_agree_after_level",
        },
    };
    (vec![p], vec![l])
}
#[test]
fn list_tab_requires_owned_generated_suffix_and_rejects_authored_tabs() {
    let (mut p, mut l) = model();
    validate(&l, &p, &[0], &mut 0).unwrap();
    assert!(validate(&[], &p, &[0], &mut 0).is_err());
    l[0].source.following_character = 1;
    assert!(validate(&l, &p, &[0], &mut 0).is_err());
    l[0].source.following_character = 0;
    p[0].runs[0].text = "A\t\r".into();
    assert!(validate(&l, &p, &[0], &mut 0).is_err());
    p[0].runs[0].text = "A\r".into();
    p[0].properties.tabs[0].position_twips = 1440;
    validate(&l, &p, &[0], &mut 0).unwrap();
    p[0].properties.first_twips = 0;
    assert!(validate(&l, &p, &[0], &mut 0).is_err());
    p[0].properties.first_twips = -360;
    l[0].source.justification = 1;
    assert!(validate(&l, &p, &[0], &mut 0).is_err());
    l[0].source.justification = 0;
    p[0].properties.tabs[0].position_twips = 719;
    assert!(validate(&l, &p, &[0], &mut 0).is_err());
}
#[test]
fn baseline_evidence_and_document_budget_are_enforced() {
    let (p, mut l) = model();
    let mut work = 8 * 1024 * 1024 - 1;
    assert!(matches!(
        validate(&l, &p, &[0], &mut work),
        Err(Error::Limit(_))
    ));
    l[0].presentation.candidate_evidence[2].size_half_points = 22;
    assert!(validate(&l, &p, &[0], &mut 0).is_err());
}
#[test]
fn paragraph_selection_grammar_and_clone_budget_precede_expansion() {
    let (_, l) = model();
    let p = crate::office::doc::formatting::PropertyModifier {
        sprm: 0x460b,
        operand: vec![1, 0],
    };
    assert!(
        paragraph_modifiers(&l[0].source, std::slice::from_ref(&p), &mut 0)
            .unwrap()
            .is_empty()
    );
    let malformed = crate::office::doc::formatting::PropertyModifier {
        sprm: 0x460b,
        operand: vec![1],
    };
    assert!(paragraph_modifiers(&l[0].source, &[malformed], &mut 0).is_err());
    assert!(matches!(
        paragraph_modifiers(&l[0].source, &[p], &mut (8 * 1024 * 1024)),
        Err(Error::Limit(_))
    ));
}

#[test]
fn symbol_label_model_retains_raw_code_and_rejects_wrong_route() {
    let (p, mut l) = model();
    l[0].source.number_format = 23;
    l[0].source.symbol_code = Some(0xf071);
    l[0].source.text = "❑".into();
    validate(&l, &p, &[2], &mut 0).unwrap();
    assert!(validate(&l, &p, &[0], &mut 0).is_err());
    l[0].source.text.push('A');
    assert!(validate(&l, &p, &[2], &mut 0).is_err());
    l[0].source.text = "❑".into();
    l[0].source.number_format = 0;
    assert!(validate(&l, &p, &[2], &mut 0).is_err());
}

#[test]
fn inferred_list_baseline_requires_first_and_mark_agreement_and_retains_style() {
    let (p, mut labels) = model();
    labels[0].presentation.candidate_evidence[0].size_half_points = 40;
    assert!(validate(&labels, &p, &[0], &mut 0).is_err());
    labels[0].presentation.basis = "approximate_first_text_and_mark_agree_after_level";
    validate(&labels, &p, &[0], &mut 0).unwrap();
    labels[0].presentation.candidate_evidence[2].bold = true;
    assert!(validate(&labels, &p, &[0], &mut 0).is_err());
}

#[test]
fn western_label_baseline_ignores_inactive_complex_bold_only() {
    let (paragraphs, mut labels) = model();
    labels[0].presentation.candidate_evidence[2].complex.bold = true;
    validate(&labels, &paragraphs, &[0], &mut 0).unwrap();
    labels[0].presentation.candidate_evidence[2].bold = true;
    assert!(validate(&labels, &paragraphs, &[0], &mut 0).is_err());
    labels[0].presentation.candidate_evidence[2].bold = false;
    labels[0].presentation.candidate_evidence[2].bidi = true;
    assert!(validate(&labels, &paragraphs, &[0], &mut 0).is_err());
}
