use super::*;

#[test]
fn relative_colours_darken_the_shapes_fill() {
    // 0x10 flag, index 0xF0 (fill colour), operation 1 (darken) in green with
    // parameter 0x80 in blue.
    let v = 0x1080_01F0;
    assert_eq!(color(v, &Scheme::DEFAULT, 0xFF8040, 0), Some(0x804020));
    // A real master background (0x107601F0): PowerPoint draws CCFFFF darkened to
    // 0x76/255 of each channel.
    assert_eq!(
        color(0x1076_01F0, &Scheme::DEFAULT, 0xCCFFFF, 0),
        Some(0x5E7676)
    );
    // Operation 2 lightens towards white by the same proportion.
    assert_eq!(
        color(0x1080_02F0, &Scheme::DEFAULT, 0x000000, 0),
        Some(0x7F7F7F)
    );
}

#[test]
fn boolean_groups_merge_only_the_bits_a_table_sets() {
    let mut p = Props::default();
    p.add_flag(id::FILL_BOOLS, 4, true);
    p.add_flag(id::FILL_BOOLS, 1, true);
    // A later table that sets only bit 4 off keeps bit 1.
    let data = [0xBF, 0x01, 0x00, 0x00, 0x10, 0x00];
    p.add(
        &data,
        Rec {
            inst: 1,
            kind: r::OPT,
            start: 0,
            end: 6,
        },
    );
    assert_eq!(p.flag(id::FILL_BOOLS, 4), Some(false));
    assert_eq!(p.flag(id::FILL_BOOLS, 1), Some(true));
}
