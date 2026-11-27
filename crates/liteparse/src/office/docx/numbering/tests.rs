use super::*;

#[test]
fn formats() {
    assert_eq!(format_number("lowerLetter", 1), "a");
    assert_eq!(format_number("upperLetter", 28), "BB");
    assert_eq!(format_number("lowerRoman", 4), "iv");
    assert_eq!(format_number("upperRoman", 9), "IX");
    assert_eq!(format_number("decimalZero", 7), "07");
    assert_eq!(format_number("decimal", 12), "12");
    assert_eq!(format_number("numberInDash", 3), "- 3 -");
    assert_eq!(format_number("decimalFullWidth", 12), "１２");
    assert_eq!(format_number("decimalEnclosedCircle", 2), "②");
    assert_eq!(format_number("chineseCounting", 10), "十");
    assert_eq!(format_number("chineseCounting", 23), "二十三");
    assert_eq!(format_number("chineseCounting", 105), "一百〇五");
    assert_eq!(format_number("ideographDigital", 20), "二〇");
    assert_eq!(format_number("cardinalText", 21), "Twenty-one");
    assert_eq!(format_number("ordinalTextLower", 22), "twenty-second");
    assert_eq!(format_number("ordinalTextLower", 100), "one hundredth");
    assert_eq!(
        format_number("cardinalTextLower", 1105),
        "one thousand one hundred five"
    );
}

#[test]
fn nested_levels_count_and_restart() {
    let xml = format!(
        r#"<w:numbering xmlns:w="{}">
        <w:abstractNum w:abstractNumId="0">
          <w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl>
          <w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="%1.%2)"/></w:lvl>
        </w:abstractNum>
        <w:num w:numId="5"><w:abstractNumId w:val="0"/></w:num>
        </w:numbering>"#,
        crate::office::docx::testutil::W
    );
    let root = crate::office::pptx::xml::parse_word(&xml).unwrap();
    let mut n = Numbering::parse(Some(&root));
    let none = |_: &str| None;
    let mut next = |l| n.next_label(5, l, &none).unwrap().0;
    assert_eq!(next(0), "1.");
    assert_eq!(next(1), "1.a)");
    assert_eq!(next(1), "1.b)");
    assert_eq!(next(0), "2.");
    assert_eq!(next(1), "2.a)");
}

#[test]
fn chicago_footnote_symbols() {
    assert_eq!(super::format_number("chicago", 1), "*");
    assert_eq!(super::format_number("chicago", 5), "**");
}

#[test]
fn overlapping_start_overrides_keep_the_deepest_start_in_any_xml_order() {
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let levels = (0..3).map(|i| format!(r#"<w:lvl w:ilvl="{i}"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%{}"/></w:lvl>"#, i + 1)).collect::<String>();
        let overrides = order
            .iter()
            .map(|i| {
                format!(
                    r#"<w:lvlOverride w:ilvl="{i}"><w:startOverride w:val="{}"/></w:lvlOverride>"#,
                    [3, 5, 17][*i]
                )
            })
            .collect::<String>();
        let xml = format!(
            r#"<w:numbering xmlns:w="{}"><w:abstractNum w:abstractNumId="0">{levels}</w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/>{overrides}</w:num></w:numbering>"#,
            crate::office::docx::testutil::W
        );
        let root = crate::office::pptx::xml::parse_word(&xml).unwrap();
        let mut numbering = Numbering::parse(Some(&root));
        let no_style = |_: &str| None;
        assert_eq!(
            numbering.next_label(1, 2, &no_style).unwrap().0,
            "17",
            "{order:?}"
        );
        assert_eq!(
            numbering.next_label(1, 2, &no_style).unwrap().0,
            "18",
            "{order:?}"
        );
    }
}

#[test]
fn letter_numbers_cycle_after_thirty_z() {
    // Word for Mac: 780 is thirty "z", 781 starts again at "a".
    assert_eq!(format_number("lowerLetter", 780), "z".repeat(30));
    assert_eq!(format_number("lowerLetter", 781), "a");
    assert_eq!(format_number("lowerLetter", 40_000), "l".repeat(9));
    assert_eq!(format_number("upperLetter", 134_954_608), "V".repeat(22));
}
