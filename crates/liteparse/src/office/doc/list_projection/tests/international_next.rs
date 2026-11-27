use super::*;

#[test]
fn thai_digits_and_iroha_wrap_use_source_counters() {
    for (format, start, expected) in [
        (54, 9, "๙.\tItem\n๑๐.\tItem\n๑๑.\tItem\n"),
        (21, 47, "ス.\tItem\nン.\tItem\nイ.\tItem\n"),
    ] {
        let mut f = Fixture::new(&[1, 1, 1]);
        let level = &mut f.definitions.lists[0].levels[0];
        level.number_format = format;
        level.start_at = start;
        let out = f.run().unwrap();
        assert_eq!(out.stories[0].text, expected);
        partition(&out.stories[0], 15);
    }
}

#[test]
fn east_asian_counting_handles_tens_and_keeps_unsupported_families_guarded() {
    for format in [11, 37] {
        for (start, expected) in [
            (9, "九.\tItem\n十.\tItem\n十一.\tItem\n"),
            (19, "十九.\tItem\n二十.\tItem\n二十一.\tItem\n"),
        ] {
            let mut f = Fixture::new(&[1, 1, 1]);
            let level = &mut f.definitions.lists[0].levels[0];
            level.number_format = format;
            level.start_at = start;
            level.raw_chpx = vec![0x6f, 0x28, 1];
            assert_eq!(f.run().unwrap().stories[0].text, expected);
        }
        for start in [0, 100] {
            let mut f = Fixture::new(&[1, 2, 1]);
            f.definitions.lists[0].levels[0].number_format = format;
            f.definitions.lists[0].levels[0].start_at = start;
            assert_eq!(f.run().unwrap().stories[0].text, "Item\n•\tItem\nItem\n");
        }
    }
}
