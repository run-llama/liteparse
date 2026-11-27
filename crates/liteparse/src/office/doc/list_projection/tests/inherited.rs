use super::*;

pub(super) fn hierarchy(levels: &[u8]) -> Fixture {
    let mut f = multilevel(levels);
    for (i, level) in f.definitions.lists[0].levels.iter_mut().enumerate() {
        level.number_format = 0;
        level.number_text_utf16.clear();
        level.placeholder_offsets = [0; 9];
        for j in 0..=i {
            level.placeholder_offsets[j] = level.number_text_utf16.len() as u8 + 1;
            level.number_text_utf16.push(j as u16);
            level.number_text_utf16.push(b'.' as u16);
        }
    }
    f
}

pub(super) fn labels(f: &Fixture) -> Vec<String> {
    let out = f.run().unwrap();
    partition(&out.stories[0], f.source.stories[0].utf16_units);
    out.stories[0]
        .segments
        .iter()
        .filter(|s| s.kind == "generated_list_label")
        .map(|s| out.stories[0].text[s.output_start..s.output_end].to_string())
        .collect()
}

#[test]
fn ancestors_use_last_paragraph_and_current_level_uses_next_counter() {
    let f = hierarchy(&[0, 1, 1, 0, 1, 2, 2]);
    assert_eq!(
        labels(&f),
        [
            "1.\t", "1.1.\t", "1.2.\t", "2.\t", "2.1.\t", "2.1.1.\t", "2.1.2.\t"
        ]
    );
}

#[test]
fn nonunit_starts_and_custom_restart_keep_ancestor_history_separate() {
    let mut f = hierarchy(&[0, 1, 2, 1, 2, 0, 2]);
    f.definitions.lists[0].levels[0].start_at = 3;
    f.definitions.lists[0].levels[1].start_at = 7;
    f.definitions.lists[0].levels[2].start_at = 9;
    f.definitions.lists[0].levels[2].flags = 8;
    f.definitions.lists[0].levels[2].restart_limit = 1;
    assert_eq!(
        labels(&f),
        [
            "3.\t",
            "3.7.\t",
            "3.7.9.\t",
            "3.8.\t",
            "3.8.10.\t",
            "4.\t",
            "4.8.9.\t"
        ]
    );
}

#[test]
fn same_lsid_shares_counters_but_different_ilfo_keeps_ancestor_ownership() {
    let mut f = hierarchy(&[0, 0, 1, 1]);
    for i in [1, 3] {
        f.paragraphs.spans[i].raw_modifiers[2] = 3;
    }
    assert_eq!(labels(&f), ["1.\t", "2.\t", "1.1.\t", "2.2.\t"]);
}

#[test]
fn absent_ancestor_in_other_ilfo_retains_independent_labels() {
    let mut f = hierarchy(&[0, 1, 0]);
    f.paragraphs.spans[1].raw_modifiers[2] = 3;
    f.paragraphs.spans[2].raw_modifiers[2] = 2;
    assert_eq!(labels(&f), ["1.\t", "•\t"]);
    let out = f.run().unwrap();
    assert!(
        out.stories[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "list_ancestor_number_unresolved" && d.cp == 5)
    );
}

#[test]
fn mixed_ancestor_formats_use_ancestor_language_and_legal_decimal_override() {
    let mut f = hierarchy(&[0, 1, 2]);
    f.definitions.lists[0].levels[0].number_format = 1;
    f.definitions.lists[0].levels[1].number_format = 3;
    f.definitions.lists[0].levels[1].raw_chpx = vec![0x6d, 0x48, 0x09, 0x04];
    assert_eq!(labels(&f), ["I.\t", "I.A.\t", "I.A.1.\t"]);
    f.definitions.lists[0].levels[2].flags = 4;
    assert_eq!(labels(&f), ["I.\t", "I.A.\t", "1.1.1.\t"]);
}

#[test]
fn invalid_or_missing_ancestor_and_templates_preserve_plain_source() {
    for text in [vec![0, 46, 2], vec![0, 46, 1]] {
        let mut f = hierarchy(&[1]);
        f.definitions.lists[0].levels[1].number_text_utf16 = text;
        assert_eq!(labels(&f), Vec::<String>::new());
    }
    let mut f = hierarchy(&[0, 1]);
    f.definitions.lists[0].levels[1].placeholder_offsets[1] = 1;
    assert!(labels(&f).is_empty());
    f.definitions.lists[0].levels[1].placeholder_offsets = [1, 0, 3, 0, 0, 0, 0, 0, 0];
    assert!(labels(&f).is_empty());
}

#[test]
fn style_links_remain_guarded_and_valid_level_override_keeps_ancestor() {
    let mut f = hierarchy(&[0, 1]);
    f.definitions.lists[0].style_links[1] = 0;
    assert!(labels(&f).is_empty());
    f.definitions.lists[0].style_links[1] = 0xfff;
    f.definitions.overrides[0].levels.push(LevelOverride {
        level: 1,
        raw_flags: 0x11,
        raw_start_at: 7,
        start_override: Some(7),
        format_override: None,
    });
    assert_eq!(labels(&f), ["1.\t", "1.7.\t"]);
}

#[test]
fn bullet_ancestor_has_no_number_sequence_and_literal_unicode_is_preserved() {
    let mut f = hierarchy(&[0, 1]);
    f.definitions.lists[0].levels[0] = level(true);
    f.definitions.lists[0].levels[1]
        .number_text_utf16
        .extend("章".encode_utf16());
    assert_eq!(labels(&f), ["•\t", ".1.章\t"]);
}

#[test]
fn rejected_rtl_ancestor_clears_tentative_generated_evidence() {
    let mut f = rtl_fixture(0);
    f.definitions.lists[0].flags = 0;
    f.definitions.lists[0]
        .levels
        .push(hierarchy(&[1]).definitions.lists[0].levels.remove(1));
    f.paragraphs.spans[1]
        .raw_modifiers
        .extend([0x41, 0x24, 0, 0x0a, 0x26, 1]);
    let report = project_lists(&transcribe(&f.source), &f.context()).unwrap();
    assert_eq!(report.transcription.stories[0].text, "Item\nItem\n");
    assert!(
        report.transcription.stories[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "list_ancestor_number_unresolved")
    );
    assert_eq!(
        report.paragraph_direction_evidence[0].list_label_status,
        "unsupported_list_sequence"
    );
}

#[test]
fn decimal_zero_ancestor_survives_legal_conversion_and_restarts() {
    let mut f = hierarchy(&[0, 1, 1, 0, 1]);
    f.definitions.lists[0].levels[0].number_format = 0x16;
    f.definitions.lists[0].levels[0].start_at = 9;
    f.definitions.lists[0].levels[1].flags = 4;
    assert_eq!(
        labels(&f),
        ["09.\t", "09.1.\t", "09.2.\t", "10.\t", "10.1.\t"]
    );
    f.definitions.lists[0].levels[1].number_format = 0x16;
    assert_eq!(
        labels(&f),
        ["09.\t", "09.01.\t", "09.02.\t", "10.\t", "10.01.\t"]
    );
}

#[test]
fn decimal_zero_start_override_and_hidden_family_guard() {
    let mut f = hierarchy(&[0, 0]);
    f.definitions.lists[0].levels[0].number_format = 0x16;
    f.definitions.overrides[0].levels.push(LevelOverride {
        level: 0,
        raw_flags: 0x10,
        raw_start_at: 9,
        start_override: Some(9),
        format_override: None,
    });
    assert_eq!(labels(&f), ["09.\t", "10.\t"]);
    f.formatting.character_spans[0].properties =
        decode_character_modifiers(&[0x3c, 0x08, 1]).unwrap();
    assert!(labels(&f).is_empty());
}

#[test]
fn skipped_display_retains_own_counter_for_descendants_and_later_restart() {
    let mut f = hierarchy(&[1, 2, 0, 1, 2]);
    f.definitions.lists[0].levels[2].number_text_utf16 = vec![1, 46, 2, 46];
    f.definitions.lists[0].levels[2].placeholder_offsets = [1, 3, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(labels(&f), ["1.1.\t", "1.\t", "1.1.\t", "1.1.\t"]);
    let out = f.run().unwrap();
    assert!(
        out.stories[0]
            .diagnostics
            .iter()
            .any(|d| d.cp == 0 && d.code == "list_ancestor_number_unresolved")
    );
}

#[test]
fn missing_ancestor_does_not_hide_invalid_later_template_literal() {
    let mut f = hierarchy(&[1, 2]);
    f.definitions.lists[0].levels[1].number_text_utf16.push(11);
    assert!(labels(&f).is_empty());
}
