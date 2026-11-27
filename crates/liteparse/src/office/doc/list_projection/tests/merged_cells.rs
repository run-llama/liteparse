use super::*;

#[test]
fn unmerged_cell_lists_recover_without_skipping_merged_family_members() {
    for second_list in [0, 1, 2, 3] {
        let mut f = table_fixture();
        f.paragraphs.status = "reference_expanded_endpoint_selected_paragraphs";
        f.source.stories[0].text = "A\u{7}B\u{7}\u{7}Item\rItem\r".into();
        f.paragraphs.spans[0].end_cp = 2;
        f.paragraphs.spans[0].raw_modifiers[2] = 1;
        let mut other = f.paragraphs.spans[0].raw_modifiers.clone();
        other[2] = second_list;
        f.paragraphs.spans.insert(
            1,
            ParagraphSpan {
                start_cp: 2,
                end_cp: 4,
                style_index: Some(0),
                raw_modifiers: other,
            },
        );
        f.paragraphs.spans[2]
            .raw_modifiers
            .extend([0x21, 0x76, 0, 2, 10, 0, 0x2b, 0xd6, 2, 1, 3]);
        let proof = table_list_proof(&f);
        assert!(proof.permits_list_paragraph([0, 2]));
        assert!(!proof.permits_list_paragraph([2, 4]));
        let out =
            project_lists_with_sources(&transcribe(&f.source), &f.context(), None, Some(&proof))
                .unwrap();
        let story = &out.transcription.stories[0];
        let labels: Vec<_> = story
            .segments
            .iter()
            .filter(|s| s.kind == "generated_list_label")
            .map(|s| (s.start_cp, &story.text[s.output_start..s.output_end]))
            .collect();
        if matches!(second_list, 1 | 3) {
            assert!(labels.is_empty());
        } else {
            assert_eq!(labels, [(0, "1.\t"), (5, "2.\t"), (10, "3.\t")]);
        }
        assert!(
            out.transcription
                .unresolved
                .contains(&"merged_cell_visible_ownership_unresolved")
        );
        partition(story, 15);
    }
}
