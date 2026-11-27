use super::*;

#[test]
fn overstrike_literals_preserve_unicode_ranges_and_signed_source_shifts() {
    let code = r" EQ \o(\s\up 12(午😀),\s\do 4(後)) ";
    let value = parse(code, 700).unwrap();
    assert_eq!(value.operands.len(), 2);
    assert_eq!(value.operands[0].vertical_shift_points, 12);
    assert_eq!(value.operands[1].vertical_shift_points, -4);
    let units: Vec<_> = code.encode_utf16().collect();
    for operand in &value.operands {
        let [start, end] = operand.source_range;
        assert_eq!(
            String::from_utf16(&units[start - 700..end - 700]).unwrap(),
            operand.text
        );
    }
    assert_eq!(
        value.operands[0].source_range[1] - value.operands[0].source_range[0],
        3
    );
}
#[test]
fn unsupported_or_malformed_operators_do_not_become_literal_output() {
    for code in [
        r"EQ \f(a,b)",
        r"EQ \o(a,\r(b))",
        r"EQ \o(a)",
        r"EQ \o(a,\s\up128(b))",
        r"EQ \o(a,\s\up-1(b))",
        r"EQ \o(a,\s\up2(b)",
        r"EQ \o(a,)",
        r"EQ \o(a,\o(b,c))",
        "EQ \\o(a,\u{13}b)",
    ] {
        assert!(parse(code, 0).is_err(), "{code}");
    }
}
