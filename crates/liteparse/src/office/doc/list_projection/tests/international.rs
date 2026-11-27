use super::*;

#[test]
fn international_counters_cross_digit_and_circle_boundaries() {
    for (format, start, expected) in [
        (14, 9, "９.\tItem\n１０.\tItem\n１１.\tItem\n"),
        (18, 19, "⑲.\tItem\n⑳.\tItem\n21.\tItem\n"),
    ] {
        let mut f = Fixture::new(&[1, 1, 1]);
        let level = &mut f.definitions.lists[0].levels[0];
        level.number_format = format;
        level.start_at = start;
        level.raw_chpx = vec![0x6f, 0x28, 1];
        let out = f.run().unwrap();
        assert_eq!(out.stories[0].text, expected);
        partition(&out.stories[0], 15);
    }
}

#[test]
fn unsupported_circle_zero_blocks_family_without_losing_source() {
    let mut f = Fixture::new(&[1, 2, 1]);
    let level = &mut f.definitions.lists[0].levels[0];
    level.number_format = 18;
    level.start_at = 0;
    let out = f.run().unwrap();
    assert_eq!(out.stories[0].text, "Item\n•\tItem\nItem\n");
    partition(&out.stories[0], 15);
}

#[test]
fn international_parent_template_retains_ancestor_value_and_literal_punctuation() {
    let mut f = multilevel(&[0, 1, 1]);
    let parent = &mut f.definitions.lists[0].levels[0];
    parent.number_format = 14;
    parent.start_at = 9;
    let child = &mut f.definitions.lists[0].levels[1];
    child.number_format = 18;
    child.number_text_utf16 = vec![0, 0xff0e, 1];
    child.placeholder_offsets = [1, 3, 0, 0, 0, 0, 0, 0, 0];
    let out = f.run().unwrap();
    assert!(out.stories[0].text.contains("９．①"));
    assert!(out.stories[0].text.contains("９．②"));
    partition(&out.stories[0], 15);
}

#[test]
fn legal_child_converts_unicode_ancestor_but_preserves_decimal_zero() {
    for (format, expected) in [(14, "9.1"), (18, "9.1"), (22, "09.1")] {
        let mut f = multilevel(&[0, 1]);
        let parent = &mut f.definitions.lists[0].levels[0];
        parent.number_format = format;
        parent.start_at = 9;
        let child = &mut f.definitions.lists[0].levels[1];
        child.flags |= 4;
        child.number_text_utf16 = vec![0, 46, 1];
        child.placeholder_offsets = [1, 3, 0, 0, 0, 0, 0, 0, 0];
        let out = f.run().unwrap();
        assert!(
            out.stories[0].text.contains(expected),
            "{}",
            out.stories[0].text
        );
        partition(&out.stories[0], 10);
    }
}

#[test]
fn unicode_numbering_does_not_admit_unknown_or_complex_font_hints() {
    for hint in [2, 3, 255] {
        let mut f = Fixture::new(&[1, 2, 1]);
        let level = &mut f.definitions.lists[0].levels[0];
        level.number_format = 14;
        level.raw_chpx = vec![0x6f, 0x28, hint];
        assert_eq!(f.run().unwrap().stories[0].text, "Item\n•\tItem\nItem\n");
    }
}
