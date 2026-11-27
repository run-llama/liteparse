use super::*;

#[test]
fn source_level_tracks_nested_counter_selection() {
    let f = multilevel(&[0, 2, 1, 2]);
    let report =
        project_lists_with_font_source(&transcribe(&f.source), &f.context(), Some(false)).unwrap();
    let levels: Vec<_> = report
        .label_sources
        .iter()
        .map(|s| (s.start_cp, s.override_index, s.level))
        .collect();
    assert_eq!(levels, [(0, 1, 0), (5, 1, 2), (10, 1, 1), (15, 1, 2)]);
    assert_eq!(
        report.transcription.stories[0].text,
        "(1)\tItem\n(i)\tItem\n(1)\tItem\n(i)\tItem\n"
    );
}

#[test]
fn emitted_labels_retain_override_and_family_identity() {
    let f = Fixture::new(&[1, 2, 3]);
    let report =
        project_lists_with_font_source(&transcribe(&f.source), &f.context(), Some(false)).unwrap();
    let sources: Vec<_> = report
        .label_sources
        .iter()
        .map(|s| (s.start_cp, s.list_id, s.override_index, s.level))
        .collect();
    assert_eq!(sources, [(0, 7, 1, 0), (5, 8, 2, 0), (10, 7, 3, 0)]);
    let anchors: Vec<_> = report.transcription.stories[0]
        .segments
        .iter()
        .filter(|s| s.kind == "generated_list_label")
        .map(|s| {
            assert_eq!(s.start_cp, s.end_cp);
            s.start_cp
        })
        .collect();
    assert_eq!(anchors, sources.iter().map(|s| s.0).collect::<Vec<_>>());
}

#[test]
fn blocked_family_emits_neither_labels_nor_layout_selection() {
    let mut f = Fixture::new(&[1, 2, 1]);
    f.definitions.lists[0].levels[0].raw_chpx = vec![0x5c, 0x08, 2];
    let report =
        project_lists_with_font_source(&transcribe(&f.source), &f.context(), Some(false)).unwrap();
    assert_eq!(
        report.label_sources,
        [LabelSource {
            start_cp: 5,
            list_id: 8,
            override_index: 2,
            level: 0,
        }]
    );
    partition(&report.transcription.stories[0], 15);
}
