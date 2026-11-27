use super::*;
use crate::office::doc::headers::{HeaderInspection, HeaderSection, HeaderStory, SourceRange};
fn range(a: usize, b: usize) -> SourceRange {
    SourceRange {
        start_cp: a,
        end_cp: b,
    }
}
fn document() -> HeaderDocument {
    let mut stories = Vec::new();
    for i in 0..12 {
        let (a, b) = if i < 7 {
            (2, 2)
        } else if i == 7 {
            (2, 5)
        } else {
            (5, 5)
        };
        stories.push(HeaderStory {
            index: i,
            role: None,
            section_index: None,
            source: range(a, b),
            content: (i == 7).then_some(range(2, 4)),
            guard: (i == 7).then_some(range(4, 5)),
            explicit_blank: false,
            unresolved: vec![],
        });
    }
    let mut p = super::super::tests::paragraph("T\r");
    p.source_cp = [2, 4];
    p.runs[0].source_cp = [2, 4];
    HeaderDocument {
        inactive_unformatted_slots: vec![],
        source_text: "T\r\r\r".into(),
        slots: vec![HeaderSlot {
            index: 7,
            paragraphs: vec![p],
        }],
        ownership: HeaderInspection {
            status: "source_header_footer_ownership",
            binary_version: 0xd9,
            source: range(2, 6),
            different_even_and_odd_pages: Some(false),
            dop_first_byte: Some(0),
            stories,
            sections: vec![HeaderSection {
                index: 0,
                plc_end_cp: 2,
                source: range(0, 2),
                sepx_offset: None,
                properties: vec![],
                different_first_page: Some(false),
                legacy_presence_flags: None,
                references: [
                    "even_header",
                    "odd_header",
                    "even_footer",
                    "odd_footer",
                    "first_header",
                    "first_footer",
                ]
                .iter()
                .enumerate()
                .map(|(i, role)| crate::office::doc::headers::HeaderReference {
                    role,
                    status: if i == 1 { "local" } else { "absent" },
                    local_story_index: Some(6 + i),
                    origin_story_index: (i == 1).then_some(7),
                    origin_section_index: (i == 1).then_some(0),
                })
                .collect(),
                unresolved: vec![],
            }],
            terminal_sentinel: Some(range(5, 6)),
            ignored_final_plc_cp: None,
            diagnostics: vec![],
            unresolved: ["page instances", "placement", "view"],
        },
    }
}
#[test]
fn header_content_guard_and_terminal_partition_are_separate() {
    assert!(document().validate(2, 6, 1).is_ok());
    let mut d = document();
    d.ownership.stories[7].guard = Some(range(3, 5));
    assert!(d.validate(2, 6, 1).is_err());
    let mut d = document();
    d.source_text = "T\rX\r".into();
    assert!(d.validate(2, 6, 1).is_err());
}
#[test]
fn missing_duplicate_or_mutated_content_is_rejected() {
    let mut d = document();
    d.slots.clear();
    assert!(d.validate(2, 6, 1).is_err());
    let mut d = document();
    d.slots.push(HeaderSlot {
        index: 7,
        paragraphs: vec![],
    });
    assert!(d.validate(2, 6, 1).is_err());
    let mut d = document();
    d.slots[0].paragraphs[0].runs[0].text = "X\r".into();
    assert!(d.validate(2, 6, 1).is_err());
    let mut d = document();
    d.ownership.sections.clear();
    assert!(d.validate(2, 6, 1).is_err());
}

#[test]
fn public_model_shares_property_budget_across_body_and_header() {
    use crate::office::doc::render_model::{
        Value, plain::PlainDocument, sections::SectionGeometry, settings::DocumentSettings,
    };
    let mut model = PlainDocument {
        footnotes: None,
        endnotes: None,
        comments: None,
        image_assets: Vec::new(),
        inline_pictures: None,
        following_sections: Vec::new(),
        unmodeled_section_options: Vec::new(),
        list_labels: vec![],
        textbox_storage: vec![],
        source_utf16_units: 6,
        settings: DocumentSettings {
            binary_version: 0xd9,
            dop_schema: "synthetic",
            raw_dop: vec![],
            recorded_install_language: 0x409,
            override_normal_font_and_language: false,
            override_section_geometry: false,
            default_tab_twips: 720,
            mirror_margins: false,
            gutter_at_top: false,
            different_even_and_odd_headers: false,
            automatic_hyphenation: false,
            hyphenate_capitals: false,
            hyphenation_zone_twips: 0,
            consecutive_hyphen_limit: 0,
            update_styles_from_template: false,
            saved_word97_compatibility: false,
            saved_show_revision_markup: false,
            saved_print_revision_markup: false,
            two_pages_per_sheet: false,
            feature_restrictions: None,
            feature_restrictions_dop_byte: None,
            compatibility: vec![],
            issues: vec![],
        },
        section: SectionGeometry {
            section_index: 0,
            source_cp: [0, 2],
            page_width_twips: Value::defaulted(12240),
            page_height_twips: Value::defaulted(15840),
            landscape: Value::defaulted(false),
            left_margin_twips: Some(Value::defaulted(720)),
            right_margin_twips: Some(Value::defaulted(720)),
            top_margin_twips: Some(Value::defaulted(720)),
            bottom_margin_twips: Some(Value::defaulted(720)),
            gutter_twips: Value::defaulted(0),
            header_distance_twips: Some(Value::defaulted(720)),
            footer_distance_twips: Some(Value::defaulted(720)),
            section_break: Value::defaulted(2),
            column_count: Value::defaulted(1),
            evenly_spaced_columns: Value::defaulted(true),
            column_spacing_twips: None,
            column_rule: Value::defaulted(false),
            vertical_justification: Value::defaulted(0),
            different_first_page: Value::defaulted(false),
            grid_mode: Value::defaulted(0),
            grid_line_pitch_twips: None,
            grid_character_spacing: Value::defaulted(0),
            issues: Vec::new(),
        },
        paragraphs: vec![super::super::tests::paragraph("A\r")],
        tables: vec![],
        font_names: vec!["Arial".into()],
        font_charsets: vec![0],
        headers: Some(document()),
        drawing_proof: None,
    };
    model.validate().unwrap();
    let evidence = crate::office::doc::formatting::PropertyModifier {
        sprm: 0x6815,
        operand: vec![0; 4],
    };
    model.paragraphs[0].runs[0].properties.applied = vec![evidence.clone(); 600_000];
    model.validate().unwrap();
    model.headers.as_mut().unwrap().slots[0].paragraphs[0].runs[0]
        .properties
        .applied = vec![evidence; 600_000];
    assert!(matches!(
        model.validate(),
        Err(Error::Limit("render model source property evidence"))
    ));
}

#[test]
fn image_control_partition_requires_exact_anchor_and_one_cp_run() {
    let mut p = super::super::tests::paragraph("\u{8}\r");
    let mut mark = p.runs[0].clone();
    p.runs[0].text = "\u{8}".into();
    p.runs[0].source_cp = [0, 1];
    mark.text = "\r".into();
    mark.source_cp = [1, 2];
    p.runs.push(mark);
    let anchors = std::collections::BTreeSet::from([0]);
    let check = |p: &super::super::PlainParagraph, anchors: &std::collections::BTreeSet<usize>| {
        super::super::validate_partition_controls(
            std::slice::from_ref(p),
            [0, 2],
            1,
            &Default::default(),
            anchors,
            &Default::default(),
        )
    };
    assert!(check(&p, &anchors).is_ok());
    assert!(check(&p, &Default::default()).is_err());
    assert!(check(&p, &std::collections::BTreeSet::from([1])).is_err());
    p.runs[0].source_cp = [0, 2];
    assert!(check(&p, &anchors).is_err());
}

fn inactive_field_document(index: usize) -> HeaderDocument {
    let mut d = document();
    d.source_text = "\u{13}PAGE\u{15}\r\r\r".into();
    d.ownership.source = range(2, 11);
    d.ownership.terminal_sentinel = Some(range(10, 11));
    d.slots.clear();
    for s in &mut d.ownership.stories {
        let start = if s.index <= index { 2 } else { 10 };
        let end = if s.index < index { 2 } else { 10 };
        s.source = range(start, end);
        s.content = (s.index == index).then_some(range(2, 9));
        s.guard = (s.index == index).then_some(range(9, 10));
    }
    for reference in &mut d.ownership.sections[0].references {
        let active = reference.local_story_index == Some(index);
        reference.status = if active { "local" } else { "absent" };
        reference.origin_story_index = active.then_some(index);
        reference.origin_section_index = active.then_some(0);
    }
    d.inactive_unformatted_slots = vec![InactiveSlot {
        index,
        source_cp: [2, 9],
        source_text: "\u{13}PAGE\u{15}\r".into(),
        uncached_fields: vec![[2, 8]],
    }];
    d
}
#[test]
fn inactive_uncached_page_retains_raw_source_and_activation_mutations_reject() {
    let d = inactive_field_document(6);
    assert!(d.validate(2, 11, 1).is_ok());
    assert_eq!(
        d.inactive_unformatted_slots[0].source_text,
        "\u{13}PAGE\u{15}\r"
    );
    let mut d = inactive_field_document(6);
    d.ownership.different_even_and_odd_pages = Some(true);
    assert!(d.validate(2, 11, 1).is_err());
    let mut d = inactive_field_document(10);
    assert!(d.validate(2, 11, 1).is_ok());
    d.ownership.sections[0].different_first_page = Some(true);
    assert!(d.validate(2, 11, 1).is_err());
    let mut d = inactive_field_document(6);
    d.inactive_unformatted_slots[0].source_text = "other".into();
    assert!(d.validate(2, 11, 1).is_err());
    let mut d = inactive_field_document(6);
    d.inactive_unformatted_slots[0].uncached_fields[0] = [3, 8];
    assert!(d.validate(2, 11, 1).is_err());
}

#[test]
fn inactive_header_activation_binds_raw_dop_and_document_settings() {
    let d = inactive_field_document(8);
    d.validate_activation(false, false).unwrap();
    assert!(d.validate_activation(true, false).is_err());
    assert!(d.validate_activation(false, true).is_err());
    let mut raw = inactive_field_document(8);
    raw.ownership.dop_first_byte = Some(1);
    assert!(raw.validate(2, raw.ownership.source.end_cp, 1).is_err());
    assert!(raw.validate_activation(false, false).is_err());
    let mut missing = d;
    missing.ownership.dop_first_byte = None;
    assert!(
        missing
            .validate(2, missing.ownership.source.end_cp, 1)
            .is_err()
    );
}

#[test]
fn header_inline_control_requires_exact_formatted_slot_ownership() {
    let mut header = document();
    header.source_text = "\u{1}\r\r\r".into();
    let mut mark = header.slots[0].paragraphs[0].runs[0].clone();
    mark.source_cp = [3, 4];
    mark.text = "\r".into();
    header.slots[0].paragraphs[0].runs[0].source_cp = [2, 3];
    header.slots[0].paragraphs[0].runs[0].text = "\u{1}".into();
    header.slots[0].paragraphs[0].runs[0].properties.language = 0x0400;
    header.slots[0].paragraphs[0].runs.push(mark);
    let pictures = std::collections::BTreeSet::from([2]);
    assert!(
        header
            .validate_after_notes_with_pictures(2, 2, 6, 1, &Default::default(), &pictures)
            .is_ok()
    );
    assert!(
        header
            .validate_after_notes_with_pictures(
                2,
                2,
                6,
                1,
                &Default::default(),
                &Default::default()
            )
            .is_err()
    );
    assert!(
        header
            .validate_after_notes_with_pictures(
                2,
                2,
                6,
                1,
                &Default::default(),
                &std::collections::BTreeSet::from([3])
            )
            .is_err()
    );
    let mut header = document();
    header.source_text = "T\r\u{1}\r".into();
    header.slots[0].paragraphs[0].runs[0].text = "T\r".into();
    assert!(
        header
            .validate_after_notes_with_pictures(
                2,
                2,
                6,
                1,
                &Default::default(),
                &std::collections::BTreeSet::from([4])
            )
            .is_err()
    );
}

#[test]
fn later_section_inherits_exact_role_owner_and_rejects_forged_origin() {
    let mut d = document();
    d.ownership.sections[0].source.end_cp = 1;
    d.ownership.sections[0].plc_end_cp = 1;
    for i in 12..18 {
        d.ownership.stories.push(HeaderStory {
            index: i,
            role: None,
            section_index: Some(1),
            source: range(5, 5),
            content: None,
            guard: None,
            explicit_blank: false,
            unresolved: vec![],
        });
    }
    let references = d.ownership.sections[0]
        .references
        .iter()
        .enumerate()
        .map(|(i, r)| crate::office::doc::headers::HeaderReference {
            role: r.role,
            status: if i == 1 { "inherited" } else { "absent" },
            local_story_index: Some(12 + i),
            origin_story_index: r.origin_story_index,
            origin_section_index: r.origin_section_index,
        })
        .collect();
    d.ownership.sections.push(HeaderSection {
        index: 1,
        plc_end_cp: 2,
        source: range(1, 2),
        sepx_offset: None,
        properties: vec![],
        different_first_page: Some(false),
        legacy_presence_flags: None,
        references,
        unresolved: vec![],
    });
    d.validate(2, 6, 1).unwrap();
    d.ownership.sections[1].references[1].origin_section_index = Some(1);
    assert!(d.validate(2, 6, 1).is_err());
    d.ownership.sections[1].references[1].origin_section_index = Some(0);
    d.ownership.sections[1].references[1].status = "local";
    assert!(d.validate(2, 6, 1).is_err());
}
