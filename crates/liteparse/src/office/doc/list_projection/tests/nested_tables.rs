use super::*;

fn fixture(inner_list: u8) -> Fixture {
    let mut f = table_fixture();
    f.source.stories[0].text = "x\r\r\u{7}\u{7}Item\rItem\r".into();
    f.source.binary_version = 0x010c;
    f.paragraphs.binary_version = 0x010c;
    f.formatting.binary_version = 0x010c;
    f.fields.binary_version = 0x010c;
    f.paragraphs.status = "reference_expanded_endpoint_selected_paragraphs";
    f.paragraphs.spans = [
        (0, 2, 2, false, true, inner_list),
        (2, 3, 2, true, false, 0),
        (3, 4, 1, false, true, 0),
        (4, 5, 1, true, false, 0),
        (5, 10, 0, false, false, 1),
        (10, 15, 0, false, false, 1),
    ]
    .into_iter()
    .map(|(start, end, depth, row, cell, ilfo)| {
        let mut raw = vec![0x0b, 0x46, ilfo, 0];
        if depth != 0 {
            raw.extend([0x16, 0x24, 1, 0x49, 0x66, depth, 0, 0, 0]);
            if depth == 1 {
                raw.extend([0x17, 0x24, u8::from(row)]);
            } else {
                raw.extend([0x4b, 0x24, u8::from(cell), 0x4c, 0x24, u8::from(row)]);
            }
            if row {
                raw.extend([0x21, 0x76, 0, 1, 10, 0]);
            }
        }
        ParagraphSpan {
            start_cp: start,
            end_cp: end,
            style_index: Some(0),
            raw_modifiers: raw,
        }
    })
    .collect();
    f
}

#[test]
fn closed_nested_rows_recover_only_independent_outside_families() {
    for (inner, expected) in [(0, 2), (1, 0), (2, 2), (3, 0)] {
        let f = fixture(inner);
        let proof = table_list_proof(&f);
        assert!(proof.nested_list_context([0, 2]).is_some());
        assert!(proof.nested_list_context([4, 5]).is_some());
        let input = transcribe(&f.source);
        let out = project_lists_with_sources(&input, &f.context(), None, Some(&proof))
            .unwrap()
            .transcription;
        let story = &out.stories[0];
        let labels: Vec<_> = story
            .segments
            .iter()
            .filter(|s| s.kind == "generated_list_label")
            .collect();
        assert_eq!(labels.len(), expected);
        if expected != 0 {
            assert_eq!(
                labels.iter().map(|s| s.start_cp).collect::<Vec<_>>(),
                [5, 10]
            );
            assert!(story.text.ends_with("1.\tItem\n2.\tItem\n"));
        }
        partition(story, 15);
    }
}

#[test]
fn nested_ownership_requires_complete_rows_and_exact_source_flags() {
    let f = fixture(0);
    assert!(f.run().is_err());
    let proof = table_list_proof(&f);
    let mut changed = fixture(0);
    changed.paragraphs.spans[0]
        .raw_modifiers
        .extend([0x4b, 0x24, 0]);
    let input = transcribe(&changed.source);
    assert!(project_lists_with_sources(&input, &changed.context(), None, Some(&proof)).is_err());
    for bad in [vec![0x4b, 0x24, 2], vec![0x4c, 0x24, 255], vec![0x4b, 0x24]] {
        assert!(apply_paragraph(&mut Selection::default(), &bad).is_err());
    }
    let mut incomplete = fixture(0);
    incomplete.paragraphs.spans[1].raw_modifiers.truncate(19);
    let proof = table_list_proof(&incomplete);
    assert!(proof.nested_list_context([0, 2]).is_none());
}

#[test]
fn nested_row_flag_refines_cell_flag_but_style_membership_is_forbidden() {
    let mut f = fixture(0);
    f.paragraphs.spans[1].raw_modifiers.extend([0x4b, 0x24, 1]);
    let proof = table_list_proof(&f);
    let out = project_lists_with_sources(&transcribe(&f.source), &f.context(), None, Some(&proof))
        .unwrap();
    assert!(
        out.transcription.stories[0]
            .text
            .ends_with("1.\tItem\n2.\tItem\n")
    );
    f.styles.styles[0].as_mut().unwrap().properties[0]
        .bytes
        .extend([0x4b, 0x24, 0]);
    assert!(f.run().is_err());
}

#[test]
fn independent_list_counter_survives_nested_row_between_outside_paragraphs() {
    let mut f = fixture(0);
    f.source.stories[0].text = "Item\rx\r\r\u{7}\u{7}Item\r".into();
    let mut before = f.paragraphs.spans.remove(4);
    before.start_cp = 0;
    before.end_cp = 5;
    for p in &mut f.paragraphs.spans[..4] {
        p.start_cp += 5;
        p.end_cp += 5;
    }
    f.paragraphs.spans.insert(0, before);
    let proof = table_list_proof(&f);
    let out = project_lists_with_sources(&transcribe(&f.source), &f.context(), None, Some(&proof))
        .unwrap();
    let story = &out.transcription.stories[0];
    let labels: Vec<_> = story
        .segments
        .iter()
        .filter(|s| s.kind == "generated_list_label")
        .map(|s| (s.start_cp, &story.text[s.output_start..s.output_end]))
        .collect();
    assert_eq!(labels, [(0, "1.\t"), (10, "2.\t")]);
    partition(story, 15);
}

#[test]
fn flat_rows_on_both_sides_keep_separate_cell_counts_and_list_progression() {
    let mut f = fixture(0);
    f.source.stories[0].text = "abc\u{7}\u{7}x\r\r\u{7}\u{7}abc\u{7}\u{7}".into();
    let flat = table_fixture();
    let mut spans = Vec::new();
    for offset in [0, 10] {
        for p in &flat.paragraphs.spans[..2] {
            let mut raw = p.raw_modifiers.clone();
            if p.start_cp == 0 {
                raw[2] = 1;
            } else {
                raw.extend([0x21, 0x76, 0, 1, 10, 0]);
            }
            spans.push(ParagraphSpan {
                start_cp: p.start_cp + offset,
                end_cp: p.end_cp + offset,
                style_index: p.style_index,
                raw_modifiers: raw,
            });
        }
    }
    for p in &f.paragraphs.spans[..4] {
        spans.push(ParagraphSpan {
            start_cp: p.start_cp + 5,
            end_cp: p.end_cp + 5,
            style_index: p.style_index,
            raw_modifiers: p.raw_modifiers.clone(),
        });
    }
    spans.sort_by_key(|p| p.start_cp);
    f.paragraphs.spans = spans;
    let proof = table_list_proof(&f);
    let out = project_lists_with_sources(&transcribe(&f.source), &f.context(), None, Some(&proof))
        .unwrap();
    let story = &out.transcription.stories[0];
    let labels: Vec<_> = story
        .segments
        .iter()
        .filter(|s| s.kind == "generated_list_label")
        .map(|s| (s.start_cp, &story.text[s.output_start..s.output_end]))
        .collect();
    assert_eq!(labels, [(0, "1.\t"), (10, "2.\t")]);
    partition(story, 15);
}

#[test]
fn recovered_outside_family_uses_authored_start_value() {
    let mut f = fixture(0);
    f.definitions.lists[0].levels[0].start_at = 3;
    let proof = table_list_proof(&f);
    let out = project_lists_with_sources(&transcribe(&f.source), &f.context(), None, Some(&proof))
        .unwrap();
    assert!(
        out.transcription.stories[0]
            .text
            .ends_with("3.\tItem\n4.\tItem\n")
    );
}

#[test]
fn source_qualified_nested_empty_style_keeps_shared_counter_sequence() {
    let mut f = fixture(1);
    let raw = &mut f.paragraphs.spans[1].raw_modifiers;
    let inner_cell = raw.windows(2).position(|v| v == [0x4b, 0x24]).unwrap();
    raw[inner_cell + 2] = 1;
    let proof = table_list_proof(&f);
    assert!(proof.permits_list_paragraph([0, 2]));
    let input = transcribe(&f.source);
    let out = project_lists_with_sources(&input, &f.context(), None, Some(&proof))
        .unwrap()
        .transcription;
    let labels: Vec<_> = out.stories[0]
        .segments
        .iter()
        .filter(|s| s.kind == "generated_list_label")
        .map(|s| s.start_cp)
        .collect();
    assert_eq!(labels, vec![0, 5, 10]);
    assert!(out.stories[0].text.ends_with("2.\tItem\n3.\tItem\n"));
    partition(&out.stories[0], 15);
}

#[test]
fn uncertain_child_style_blocks_shared_counter_but_keeps_independent_outside_labels() {
    for (inner, expected) in [(1, 0), (2, 2)] {
        let mut f = fixture(inner);
        let raw = &mut f.paragraphs.spans[1].raw_modifiers;
        let flag = raw.windows(2).position(|v| v == [0x4b, 0x24]).unwrap();
        raw[flag + 2] = 1;
        raw.extend([0x3a, 0x56, 12, 0]);
        f.styles.styles.push(Some(Style {
            index: 12,
            builtin_id: 105,
            kind: 3,
            base_style: Some(11),
            next_style: 12,
            linked_style: None,
            revision_marked: false,
            name: "Synthetic child".into(),
            raw_flags: 0,
            properties: vec![],
            raw_revision: vec![],
        }));
        f.styles.styles[12].as_mut().unwrap().properties.push(
            crate::office::doc::styles::StyleProperties {
                kind: "character",
                bytes: vec![0x3c, 0x08, 1],
            },
        );
        let proof = table_list_proof(&f);
        assert!(!proof.permits_list_paragraph([0, 2]));
        let input = transcribe(&f.source);
        let out = project_lists_with_sources(&input, &f.context(), None, Some(&proof))
            .unwrap()
            .transcription;
        let labels: Vec<_> = out.stories[0]
            .segments
            .iter()
            .filter(|s| s.kind == "generated_list_label")
            .map(|s| s.start_cp)
            .collect();
        assert_eq!(labels.len(), expected);
        if expected == 2 {
            assert_eq!(labels, vec![5, 10]);
            assert!(out.stories[0].text.ends_with("1.\tItem\n2.\tItem\n"));
        }
        partition(&out.stories[0], 15);
        f.styles.styles[12].as_mut().unwrap().properties[0].bytes[2] = 2;
        assert!(project_lists_with_sources(&input, &f.context(), None, Some(&proof)).is_err());
    }
}
