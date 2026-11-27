use super::*;

fn story(text: &str) -> Story {
    Story {
        kind: "headers",
        start_cp: 100,
        utf16_units: text.encode_utf16().count(),
        text: text.into(),
    }
}
fn plc(events: &[(u32, u8)], terminal: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (cp, _) in events {
        bytes.extend(cp.to_le_bytes());
    }
    bytes.extend(terminal.to_le_bytes());
    for (_, kind) in events {
        bytes.extend([*kind, 0]);
    }
    bytes
}

#[test]
fn nested_field_ranges_crosscheck_utf16_and_absolute_story_positions() {
    let source = story("😀\u{13}IF\u{13}X\u{14}Y\u{15}\u{14}Z\u{15}");
    let table = decode(
        &source,
        &plc(
            &[(2, 19), (5, 19), (7, 20), (9, 21), (10, 20), (12, 21)],
            500,
        ),
    )
    .unwrap();
    assert!(table.topology_valid);
    assert_eq!(table.unindexed_marker_units, 0);
    assert_eq!(table.ranges.len(), 2);
    assert_eq!(table.ranges[0].parent_begin_cp, Some(102));
    assert_eq!(table.ranges[1].source.start, 102);
    assert_eq!(table.ranges[1].source.end, 113);
    assert_eq!(table.ranges[1].result.as_ref().unwrap().start, 111);
}

#[test]
fn absent_and_empty_tables_retain_unindexed_marker_count() {
    let source = story("\u{13}XE word\u{15}");
    for bytes in [vec![], plc(&[], u32::MAX)] {
        let table = decode(&source, &bytes).unwrap();
        assert!(table.topology_valid);
        assert_eq!(table.unindexed_marker_units, 2);
        assert!(table.ranges.is_empty());
    }
}

#[test]
fn reserved_fldch_bits_are_ignored_and_metadata_preserved() {
    let mut bytes = plc(&[(0, 19 | 0xe0), (2, 21 | 0xa0)], 3);
    bytes[13] = 42;
    bytes[15] = 0x80;
    let table = decode(&story("\u{13}X\u{15}"), &bytes).unwrap();
    assert_eq!(table.events[0].metadata, 42);
    assert_eq!(table.events[1].metadata, 128);
    assert!(table.ranges[0].result.is_none());
}

#[test]
fn malformed_topology_retains_events_without_usable_ranges() {
    for text in ["\u{13}X", "\u{14}", "\u{15}", "\u{13}\u{14}\u{14}\u{15}"] {
        let events: Vec<_> = text
            .chars()
            .enumerate()
            .filter(|(_, ch)| matches!(ch, '\u{13}'..='\u{15}'))
            .map(|(cp, ch)| (cp as u32, ch as u8))
            .collect();
        let table = decode(&story(text), &plc(&events, text.len() as u32 + 1)).unwrap();
        assert!(!table.topology_valid);
        assert_eq!(table.events.len(), events.len());
        assert!(table.ranges.is_empty());
        assert!(!table.diagnostics.is_empty());
    }
}

#[test]
fn rejects_bad_sizes_positions_marker_types_and_source_mismatch() {
    let source = story("😀\u{13}X\u{15}");
    for bytes in [
        vec![0],
        plc(&[(2, 19), (2, 21)], 5),
        plc(&[(2, 19), (4, 21)], 4),
        plc(&[(1, 19)], 5),
        plc(&[(2, 18)], 5),
        plc(&[(2, 21)], 5),
        plc(&[(5, 19)], 6),
    ] {
        assert!(decode(&source, &bytes).is_err());
    }
}

#[test]
fn field_resource_limits_are_explicit() {
    let text = "\u{13}".repeat(MAX_DEPTH + 1);
    let events: Vec<_> = (0..=MAX_DEPTH).map(|cp| (cp as u32, 19)).collect();
    assert_eq!(
        decode(&story(&text), &plc(&events, MAX_DEPTH as u32 + 2)).unwrap_err(),
        Error::Limit("field PLC nesting")
    );
    let oversized = vec![0; (MAX_EVENTS + 1) * 6 + 4];
    assert_eq!(
        decode(&story(""), &oversized).unwrap_err(),
        Error::Limit("field PLC events")
    );
}

#[test]
fn field_tables_use_verified_fib_pair_indices() {
    for (name, index) in [
        ("body", 16),
        ("headers", 17),
        ("footnotes", 18),
        ("comments", 19),
        ("endnotes", 48),
        ("textboxes", 57),
        ("header_textboxes", 59),
    ] {
        assert_eq!(fib_pair_index(name), Some(index));
    }
    assert_eq!(fib_pair_index("reserved_macro"), None);
}
