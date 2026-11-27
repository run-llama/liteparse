use super::*;
fn record(kind: u16, version: u16, instance: u16, body: &[u8]) -> Vec<u8> {
    let mut out = ((instance << 4) | version).to_le_bytes().to_vec();
    out.extend(kind.to_le_bytes());
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    out
}
fn property(kind: u16, text: &str, prefix: bool) -> Vec<u8> {
    let mut payload: Vec<u8> = text
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    let mut fixed = Vec::new();
    if prefix {
        fixed.extend(0x8080u16.to_le_bytes());
        fixed.extend(10u32.to_le_bytes());
    }
    fixed.extend(0xc0c0u16.to_le_bytes());
    fixed.extend((payload.len() as u32).to_le_bytes());
    if prefix {
        fixed.extend([1, 0, 1, 0, 0xf0, 0xff, 4, 3, 2, 1]);
    }
    fixed.append(&mut payload);
    record(kind, 3, if prefix { 2 } else { 1 }, &fixed)
}
fn shape(properties: &[Vec<u8>]) -> Vec<u8> {
    let mut identity = 7u32.to_le_bytes().to_vec();
    identity.extend(0xa00u32.to_le_bytes());
    let mut body = record(0xf00a, 2, 136, &identity);
    for p in properties {
        body.extend(p);
    }
    record(0xf004, 15, 0, &body)
}
fn drawing(shape: &[u8], story: u8) -> Vec<u8> {
    let mut data = record(0xf000, 15, 0, &[]);
    data.push(story);
    data.extend(record(0xf002, 15, 0, &record(0xf003, 15, 0, shape)));
    data
}
#[test]
fn complex_arrays_and_supplementary_text_keep_stream_ownership() {
    let data = drawing(&shape(&[property(0xf00b, "A🗸B", true)]), 1);
    let decoded = decode(&data, 400, "1Table").unwrap();
    let s = &decoded.shapes[0];
    assert_eq!(s.source_text.as_deref(), Some("A🗸B"));
    assert_eq!(s.story, "headers");
    assert_eq!(s.shape_id, 7);
    let p = &s.candidates[0];
    assert_eq!(p.utf16_units, 4);
    let [a, b] = p.payload_table_range.unwrap();
    let expected: Vec<u8> = "A🗸B"
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    assert_eq!(&data[a - 400..b - 400], expected);
    assert!(s.unresolved.contains(&"visibility_not_resolved"));
}
#[test]
fn repeated_property_tables_preserve_candidates_without_inventing_precedence() {
    let data = drawing(
        &shape(&[
            property(0xf00b, "first", false),
            property(0xf122, "second", false),
        ]),
        0,
    );
    let out = decode(&data, 0, "0Table").unwrap();
    assert!(out.shapes[0].source_text.is_none());
    assert_eq!(out.shapes[0].candidates.len(), 2);
    assert_eq!(out.shapes[0].candidates[1].text, "second");
    for kind in [0xf00b, 0xf121, 0xf122] {
        let out = decode(
            &drawing(&shape(&[property(kind, "text", false)]), 0),
            0,
            "0Table",
        )
        .unwrap();
        assert_eq!(out.shapes[0].source_text.as_deref(), Some("text"));
    }
}
#[test]
fn malformed_payloads_do_not_shift_text_ownership_or_escape_budget() {
    let mut p = property(0xf00b, "text", true);
    p[10..14].copy_from_slice(&9u32.to_le_bytes());
    assert!(decode(&drawing(&shape(&[p]), 0), 0, "0Table").is_err());
    let mut p = property(0xf00b, "text", false);
    let n = p.len();
    p[n - 2] = 1;
    assert!(decode(&drawing(&shape(&[p]), 0), 0, "0Table").is_err());
    let p = property(0xf00b, "text", false);
    let mut budget = 1;
    assert!(matches!(
        properties::decode(&p[8..], 1, 0, 0xf00b, &mut budget),
        Err(Error::Limit(_))
    ));
    let no_identity = record(0xf004, 15, 0, &property(0xf00b, "text", false));
    assert!(decode(&drawing(&no_identity, 0), 0, "0Table").is_err());
    let data = drawing(&shape(&[property(0xf00b, "text", false)]), 0);
    let mut reader = Reader {
        data: &data,
        base: 0,
        records_left: 0,
        work_left: 1000,
        text_left: 1000,
        shapes: vec![],
    };
    assert!(matches!(reader.record(0, data.len()), Err(Error::Limit(_))));
}

#[test]
fn opaque_nontext_shape_properties_do_not_block_separate_geometry_strings() {
    let mut body = 0x8145u16.to_le_bytes().to_vec();
    body.extend(4u32.to_le_bytes());
    body.extend([1, 0, 1, 0, 0xf0, 0xff, 1, 2, 3, 4]);
    let unrelated = shape(&[record(0xf00b, 3, 1, &body)]);
    let mut shapes = unrelated;
    shapes.extend(shape(&[property(0xf00b, "usable", false)]));
    let out = decode(&drawing(&shapes, 0), 0, "1Table").unwrap();
    assert_eq!(out.shapes.len(), 1);
    assert_eq!(out.shapes[0].source_text.as_deref(), Some("usable"));
}

#[test]
fn duplicate_text_entries_preserve_both_strings_without_guessing_effective_value() {
    let mut table = Vec::new();
    for _ in 0..2 {
        table.extend(0x80c0u16.to_le_bytes());
        table.extend(4u32.to_le_bytes());
    }
    table.extend([65, 0, 0, 0, 66, 0, 0, 0]);
    let out = decode(
        &drawing(&shape(&[record(0xf00b, 3, 2, &table)]), 0),
        0,
        "1Table",
    )
    .unwrap();
    let s = &out.shapes[0];
    assert!(s.source_text.is_none());
    assert_eq!(
        s.candidates
            .iter()
            .map(|p| p.text.as_str())
            .collect::<Vec<_>>(),
        vec!["A", "B"]
    );
}

#[test]
fn supplementary_text_summary_keeps_story_partition_and_visibility_uncertainty_separate() {
    let raw = crate::office::doc::Inspection {
        schema_version: 1,
        status: "decoded",
        binary_version: 0x112,
        trailing_container_bytes: 0,
        piece_count: 1,
        stories: vec![crate::office::doc::Story {
            kind: "body",
            start_cp: 0,
            utf16_units: 2,
            text: "A\r".into(),
        }],
        trailing_utf16_units: vec![],
    };
    let projection = crate::office::doc::transcribe(&raw);
    let mut summary = crate::office::doc::extraction_summary::summarize(&raw, &projection).unwrap();
    let stories = serde_json::to_value(&summary.stories).unwrap();
    let geometry = decode(
        &drawing(&shape(&[property(0xf00b, "A😀", false)]), 0),
        0,
        "1Table",
    )
    .unwrap();
    summary.attach_geometry_text(&geometry);
    summary.attach_geometry_text(&geometry);
    assert_eq!(serde_json::to_value(&summary.stories).unwrap(), stories);
    assert_eq!(
        summary
            .supplementary_geometry_text
            .as_ref()
            .unwrap()
            .stored_utf16_units,
        3
    );
    assert_eq!(
        summary
            .unresolved
            .iter()
            .filter(|c| **c == "geometry_text_visibility_and_reading_order_unresolved")
            .count(),
        1
    );
    assert_eq!(summary.stories[0].unresolved_source_units, 0);
}
