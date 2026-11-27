use super::*;
use crate::office::doc::tests::{fixture, put16, put32};

fn field_fixture() -> Vec<u8> {
    let mut data = fixture();
    data[1536 + 2048..1536 + 2058].copy_from_slice(b"\x13X\x14Y\x15\x13Z\x15!\r");
    for (pair, offset, positions, markers) in [
        (16, 64, vec![0, 2, 4, 5], vec![19, 20, 21]),
        (18, 128, vec![0, 2, 5], vec![19, 21]),
    ] {
        put32(&mut data, 1536 + 154 + pair * 8, offset as u32);
        put32(
            &mut data,
            1536 + 158 + pair * 8,
            (positions.len() * 4 + markers.len() * 2) as u32,
        );
        for (index, cp) in positions.iter().enumerate() {
            put32(&mut data, 5632 + offset + index * 4, *cp);
        }
        for (index, marker) in markers.iter().enumerate() {
            data[5632 + offset + positions.len() * 4 + index * 2] = *marker;
        }
    }
    data
}

#[test]
fn inspects_source_fields_by_story_with_absolute_positions() {
    let data = field_fixture();
    let result = inspect_fields(&data).unwrap();
    assert_eq!(result.status, "source_field_metadata_only");
    assert_eq!(result.stories.len(), 7);
    let body = &result.stories[0];
    assert_eq!(body.status, "indexed_cf_spec_unresolved");
    assert_eq!(
        body.table.as_ref().unwrap().ranges[0]
            .result
            .as_ref()
            .unwrap()
            .start,
        3
    );
    let note = result
        .stories
        .iter()
        .find(|s| s.kind == "footnotes")
        .unwrap();
    assert_eq!(note.table.as_ref().unwrap().events[0].cp, 5);
    assert_eq!(note.table.as_ref().unwrap().ranges[0].source.end, 8);
    assert_eq!(
        inspect(&data).unwrap().stories[0].text,
        "\u{13}X\u{14}Y\u{15}"
    );
}

#[test]
fn one_invalid_table_keeps_other_stories_and_raw_text() {
    let mut data = field_fixture();
    put32(&mut data, 1536 + 154 + 16 * 8, 4095);
    let result = inspect_fields(&data).unwrap();
    assert_eq!(result.stories[0].status, "unavailable");
    assert!(result.stories[0].error.is_some());
    assert!(result.stories[0].table.is_none());
    let note = result
        .stories
        .iter()
        .find(|s| s.kind == "footnotes")
        .unwrap();
    assert_eq!(note.status, "indexed_cf_spec_unresolved");
    assert!(inspect(&data).is_ok());
}

#[test]
fn absent_table_ignores_undefined_offset_and_retains_unindexed_marker_count() {
    let mut data = field_fixture();
    put32(&mut data, 1536 + 154 + 16 * 8, u32::MAX);
    put32(&mut data, 1536 + 158 + 16 * 8, 0);
    let result = inspect_fields(&data).unwrap();
    let body = &result.stories[0];
    assert_eq!(body.status, "absent");
    assert!(!body.table_present);
    assert_eq!(body.table.as_ref().unwrap().unindexed_marker_units, 3);
    assert!(body.error.is_none());
}

#[test]
fn malformed_topology_is_distinct_from_absence_and_unavailable_bytes() {
    let mut data = field_fixture();
    data[1536 + 2048] = 21;
    data[5632 + 64 + 16] = 21;
    let result = inspect_fields(&data).unwrap();
    assert_eq!(result.stories[0].status, "invalid_topology");
    assert!(
        !result.stories[0]
            .table
            .as_ref()
            .unwrap()
            .diagnostics
            .is_empty()
    );
    assert!(result.stories[0].error.is_none());
}

#[test]
fn shared_fib_validation_rejects_encryption_and_reads_unlisted_revisions() {
    let mut data = field_fixture();
    put16(&mut data, 1536 + 10, 0x300);
    assert!(matches!(
        inspect_fields(&data),
        Err(Error::Unsupported("encrypted binary Word"))
    ));
    put16(&mut data, 1536 + 10, 0x200);
    put16(&mut data, 1536 + 156 + 183 * 8, 0xffff);
    // An unlisted extended revision reads by its offset table's size.
    assert!(inspect_fields(&data).is_ok());
}

#[test]
fn aggregate_event_budget_includes_malformed_table_work() {
    let source = crate::office::doc::Story {
        kind: "body",
        start_cp: 0,
        utf16_units: 1,
        text: "\u{13}".into(),
    };
    let mut plc = vec![0; 10];
    put32(&mut plc, 4, 1);
    plc[8] = 19;
    let mut budget = 1;
    let table = decode_table(&source, &plc, 0, 10, &mut budget).unwrap();
    assert!(!table.topology_valid);
    assert_eq!(budget, 0);
    assert!(matches!(
        decode_table(&source, &plc, 0, 10, &mut budget),
        Err(Error::Limit("document field events"))
    ));
    assert!(decode_table(&source, &[], usize::MAX, 0, &mut budget).is_ok());
}

#[test]
fn all_story_tables_use_selected_stream_and_story_relative_positions() {
    let mut data = fixture();
    put16(&mut data, 1536 + 10, 0);
    put16(&mut data, 1024 + 2 * 128, b'0' as u16);
    let pair_indices = [16, 18, 17, 19, 48, 57, 59];
    let story_indices = [0, 1, 2, 4, 5, 6, 7];
    for (index, pair) in story_indices.into_iter().zip(pair_indices) {
        put32(&mut data, 1536 + 76 + index * 4, 3);
        put32(&mut data, 1536 + 154 + pair * 8, 64);
        put32(&mut data, 1536 + 158 + pair * 8, 16);
    }
    data[1536 + 2048..1536 + 2069].copy_from_slice(&b"\x13X\x15".repeat(7));
    put32(&mut data, 5632 + 9, 21);
    for (offset, value) in [(64, 0), (68, 2), (72, 3)] {
        put32(&mut data, 5632 + offset, value);
    }
    data[5632 + 76] = 19;
    data[5632 + 78] = 21;
    let result = inspect_fields(&data).unwrap();
    for (index, story) in result.stories.iter().enumerate() {
        assert_eq!(story.status, "indexed_cf_spec_unresolved");
        assert_eq!(story.start_cp, index * 3);
        assert_eq!(story.table.as_ref().unwrap().events[0].cp, index * 3);
    }
}
