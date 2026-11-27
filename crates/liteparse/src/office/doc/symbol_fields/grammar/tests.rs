use super::*;

#[test]
fn numeric_explicit_font_grammar_preserves_code() {
    for s in [
        "SYMBOL 211 \\f \"Symbol\"",
        " symbol\t0xD3  \\f \"Symbol\" ",
    ] {
        let parsed = parse(s).unwrap();
        assert_eq!(parsed.code, 211);
        assert_eq!(parsed.font, "Symbol");
    }
    assert_eq!(parse("SYMBOL 227 \\f \"Symbol\"").unwrap().code, 227);
}

#[test]
fn unsupported_modes_and_ambiguous_instructions_stay_guarded() {
    for s in [
        "SYMBOL211 \\f \"Symbol\"",
        "SYMBOL -1 \\f \"Symbol\"",
        "SYMBOL 65536 \\f \"Symbol\"",
        "SYMBOL 0x \\f \"Symbol\"",
        "SYMBOL 211",
        "SYMBOL 211 \\f Symbol",
        "SYMBOL 211 \\f \"\"",
        "SYMBOL 211 \\u \\f \"Symbol\"",
        "SYMBOL 211 \\f \"Symbol\" \\s 12",
        "SYMBOL 211 \\f \"Symbol\" \\f \"Arial\"",
        "SYMBOL 31 \\f \"Symbol\"",
        "SYMBOL 0xf0d3 \\f \"Symbol\"",
        "SYMBOL © \\f \"Symbol\"",
    ] {
        assert!(parse(s).is_err(), "{s}");
    }
}
