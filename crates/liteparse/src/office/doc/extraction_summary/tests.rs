use super::*;
fn source(text: &str) -> Inspection {
    Inspection {
        schema_version: 1,
        status: "decoded",
        binary_version: 0x112,
        trailing_container_bytes: 0,
        piece_count: 1,
        stories: vec![crate::office::doc::Story {
            kind: "textboxes",
            start_cp: 10,
            utf16_units: text.encode_utf16().count(),
            text: text.into(),
        }],
        trailing_utf16_units: vec![13],
    }
}
#[test]
fn partial_textbox_text_does_not_require_page_layout() {
    let raw = source("A\u{13}CODE\u{14}B\u{15}\r");
    let projected = crate::office::doc::transcribe(&raw);
    let summary = summarize(&raw, &projected).unwrap();
    assert_eq!(summary.fidelity, "partial_logical_extraction");
    assert_eq!(summary.layout, "not_established");
    assert_eq!(summary.stories[0].source_range, [10, 20]);
    assert!(
        summary.stories[0]
            .suppressed
            .iter()
            .any(|s| s.kind == "field_instruction")
    );
    assert_eq!(summary.trailing_source_units, 1);
    assert_eq!(projected.stories[0].text, "AB\n");
}
#[test]
fn unresolved_controls_retain_source_and_output_ranges() {
    let raw = source("A\u{8}😀\r");
    let projected = crate::office::doc::transcribe(&raw);
    let summary = summarize(&raw, &projected).unwrap();
    assert_eq!(summary.stories[0].raw_utf16_units, 5);
    assert_eq!(summary.stories[0].unresolved_source_units, 1);
    assert_eq!(
        summary.stories[0].unresolved_ranges[0].source_range,
        [11, 12]
    );
    assert_eq!(summary.stories[0].unresolved_ranges[0].output_bytes, [1, 4]);
}
#[test]
fn fabricated_or_missing_partition_is_not_reported_complete() {
    let raw = source("AB\r");
    let mut projected = crate::office::doc::transcribe(&raw);
    projected.stories[0].segments[0].start_cp = 11;
    assert!(summarize(&raw, &projected).is_err());
    projected = crate::office::doc::transcribe(&raw);
    projected.stories[0].segments.pop();
    assert!(summarize(&raw, &projected).is_err());
}
#[test]
fn generated_text_has_zero_source_width_and_is_not_a_source_loss() {
    let raw = source("A\r");
    let mut projected = crate::office::doc::transcribe(&raw);
    projected.stories[0].text.insert_str(0, "1. ");
    for segment in &mut projected.stories[0].segments {
        segment.output_start += 3;
        segment.output_end += 3;
    }
    projected.stories[0].segments.insert(
        0,
        crate::office::doc::SourceSegment {
            kind: "generated_list_label",
            start_cp: 10,
            end_cp: 10,
            output_start: 0,
            output_end: 3,
        },
    );
    let summary = summarize(&raw, &projected).unwrap();
    assert_eq!(summary.stories[0].generated[0].source_range, [10, 10]);
    assert_eq!(summary.stories[0].suppressed_source_units, 0);
}
