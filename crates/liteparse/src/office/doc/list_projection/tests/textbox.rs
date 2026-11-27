use super::*;
use crate::office::doc::textbox_anchors::{
    TextboxInspection, TextboxPart, TextboxRange, TextboxRecord,
};

fn metadata(
    start: usize,
    active_end: usize,
    total: usize,
    kind: &'static str,
) -> TextboxInspection {
    TextboxInspection {
        status: "source_textbox_anchor_metadata",
        parts: vec![TextboxPart {
            text_story: kind,
            anchor_story: if kind == "textboxes" {
                "body"
            } else {
                "headers"
            },
            shapes: vec![],
            records: vec![
                TextboxRecord {
                    record_index: 0,
                    raw_cp_range: [0, active_end as u32],
                    reusable: false,
                    source: Some(TextboxRange {
                        start_cp: start,
                        end_cp: start + active_end,
                    }),
                    shape_id: Some(1026),
                    chain_shape_count: Some(1),
                    anchor_index: None,
                    anchor_status: "shape_hierarchy_unresolved",
                },
                TextboxRecord {
                    record_index: 1,
                    raw_cp_range: [active_end as u32, total as u32],
                    reusable: true,
                    source: None,
                    shape_id: None,
                    chain_shape_count: None,
                    anchor_index: None,
                    anchor_status: "reusable_slot",
                },
            ],
        }],
        unresolved: [""; 5],
    }
}
fn secondary(selections: &[u16], kind: &'static str) -> Fixture {
    let mut f = Fixture::new(selections);
    f.source.stories[0].kind = kind;
    f.fields.stories[0].kind = kind;
    f
}
fn projected(f: &Fixture, m: &TextboxInspection) -> ListProjectionReport {
    project_lists_with_textboxes(
        &transcribe(&f.source),
        &f.context(),
        Some(false),
        None,
        Some(m),
    )
    .unwrap()
}

#[test]
fn active_textbox_bullets_preserve_source_and_exclude_terminal_storage() {
    for kind in ["textboxes", "header_textboxes"] {
        let f = secondary(&[2, 2, 2], kind);
        let m = metadata(0, 10, 15, kind);
        let before = f.run().unwrap();
        assert!(
            !before.stories[0]
                .segments
                .iter()
                .any(|s| s.kind == "generated_list_label")
        );
        let report = projected(&f, &m);
        assert_eq!(
            report
                .label_sources
                .iter()
                .map(|l| l.start_cp)
                .collect::<Vec<_>>(),
            vec![0, 5]
        );
        partition(&report.transcription.stories[0], 15);
        let retained = report.transcription.stories[0]
            .segments
            .iter()
            .filter(|s| s.kind != "generated_list_label")
            .map(|s| &report.transcription.stories[0].text[s.output_start..s.output_end])
            .collect::<String>();
        assert_eq!(retained, before.stories[0].text);
        assert_eq!(f.source.stories[0].text, "Item\rItem\rItem\r");
    }
}

#[test]
fn textbox_numeric_sequences_stay_unresolved_without_blocking_separate_bullets() {
    let f = secondary(&[1, 2, 2], "textboxes");
    let report = projected(&f, &metadata(0, 10, 15, "textboxes"));
    assert_eq!(
        report
            .label_sources
            .iter()
            .map(|l| l.start_cp)
            .collect::<Vec<_>>(),
        vec![5]
    );
    assert!(
        report.transcription.stories[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "textbox_numeric_list_sequence_unresolved" && d.cp == 0)
    );
}

#[test]
fn textbox_range_and_field_mismatches_retain_unprojected_source() {
    let mut f = secondary(&[2, 2], "textboxes");
    let mut m = metadata(0, 5, 10, "textboxes");
    m.parts[0].records[0].source.as_mut().unwrap().start_cp = 1;
    let report = projected(&f, &m);
    assert!(report.label_sources.is_empty());
    assert!(
        report.transcription.stories[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "textbox_list_source_ranges_unresolved")
    );
    m.parts[0].records[0].source.as_mut().unwrap().start_cp = 0;
    f.fields.stories[0].kind = "body";
    let report = projected(&f, &m);
    assert!(report.label_sources.is_empty());
    assert!(
        report.transcription.stories[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "textbox_list_context_unresolved")
    );
}

#[test]
fn body_projection_survives_unresolved_secondary_story() {
    let mut f = Fixture::new(&[1, 2, 2]);
    f.source.stories[0].text = "Item\r".into();
    f.source.stories[0].utf16_units = 5;
    f.source.stories.push(crate::office::doc::Story {
        kind: "textboxes",
        start_cp: 5,
        utf16_units: 10,
        text: "Item\rItem\r".into(),
    });
    f.fields.stories[0].utf16_units = 5;
    f.fields.stories[0].table =
        Some(crate::office::doc::fields::decode(&f.source.stories[0], &[]).unwrap());
    let report = projected(&f, &metadata(5, 5, 10, "textboxes"));
    assert_eq!(report.label_sources.len(), 1);
    assert_eq!(report.label_sources[0].start_cp, 0);
    assert!(
        report.transcription.stories[1]
            .diagnostics
            .iter()
            .any(|d| d.code == "textbox_list_context_unresolved")
    );
    f.fields.stories.push(StoryFields {
        status: "absent",
        kind: "textboxes",
        start_cp: 5,
        utf16_units: 10,
        table_present: false,
        table: Some(crate::office::doc::fields::decode(&f.source.stories[1], &[]).unwrap()),
        error: None,
    });
    let report = projected(&f, &metadata(5, 5, 10, "textboxes"));
    assert_eq!(
        report
            .label_sources
            .iter()
            .map(|l| l.start_cp)
            .collect::<Vec<_>>(),
        vec![0, 5]
    );
}

#[test]
fn intermediate_spare_and_terminal_guard_do_not_generate_bullets() {
    let mut f = secondary(&[2, 2, 2, 2], "textboxes");
    f.source.stories[0].text = "Item\r\rItem\r\r".into();
    f.source.stories[0].utf16_units = 12;
    for (span, [start, end]) in
        f.paragraphs
            .spans
            .iter_mut()
            .zip([[0, 5], [5, 6], [6, 11], [11, 12]])
    {
        span.start_cp = start;
        span.end_cp = end;
    }
    f.formatting.authored_story_end_cp = 12;
    f.formatting.pieces[0].end_cp = 12;
    f.formatting.character_spans[0].end_cp = 12;
    f.revisions.spans[0].end_cp = 12;
    f.fields.stories[0].utf16_units = 12;
    f.fields.stories[0].table =
        Some(crate::office::doc::fields::decode(&f.source.stories[0], &[]).unwrap());
    let mut m = metadata(0, 5, 12, "textboxes");
    let mut tail = m.parts[0].records.pop().unwrap();
    tail.record_index = 3;
    tail.raw_cp_range = [11, 12];
    let spare = TextboxRecord {
        record_index: 1,
        raw_cp_range: [5, 6],
        reusable: true,
        source: None,
        shape_id: None,
        chain_shape_count: None,
        anchor_index: None,
        anchor_status: "reusable_slot",
    };
    let mut another = metadata(6, 5, 6, "textboxes")
        .parts
        .remove(0)
        .records
        .remove(0);
    another.record_index = 2;
    another.raw_cp_range = [6, 11];
    another.shape_id = Some(1027);
    m.parts[0].records.extend([spare, another, tail]);
    let report = projected(&f, &m);
    assert_eq!(
        report
            .label_sources
            .iter()
            .map(|l| l.start_cp)
            .collect::<Vec<_>>(),
        vec![0, 6]
    );
    partition(&report.transcription.stories[0], 12);
    m.parts[0].records[1].source = Some(TextboxRange {
        start_cp: 5,
        end_cp: 6,
    });
    assert!(projected(&f, &m).label_sources.is_empty());
    m.parts[0].records[1].source = None;
    m.parts[0].records[3].reusable = false;
    assert!(projected(&f, &m).label_sources.is_empty());
}

#[test]
fn numeric_textbox_diagnostics_are_bounded_without_losing_source() {
    let f = secondary(&vec![1; 301], "textboxes");
    let report = projected(&f, &metadata(0, 1500, 1505, "textboxes"));
    assert!(report.label_sources.is_empty());
    assert_eq!(
        report.transcription.stories[0]
            .diagnostics
            .iter()
            .filter(|d| d.code == "textbox_numeric_list_sequence_unresolved")
            .count(),
        256
    );
    assert_eq!(
        report.transcription.stories[0].text,
        transcribe(&f.source).stories[0].text
    );
}
