use super::*;
fn piece(start_cp: usize, end_cp: usize, file_offset: usize, compressed: bool) -> PieceProperties {
    PieceProperties {
        start_cp,
        end_cp,
        file_offset,
        compressed,
        raw_flags: 0,
        raw_prm: 0,
    }
}
fn physical(limits: &[u32]) -> (Vec<u8>, Vec<u8>) {
    let mut word = vec![0; 2048];
    let n = limits.len() - 1;
    for (i, x) in limits.iter().enumerate() {
        word[512 + i * 4..516 + i * 4].copy_from_slice(&x.to_le_bytes());
    }
    word[1023] = n as u8;
    let mut plc = Vec::new();
    plc.extend(limits[0].to_le_bytes());
    plc.extend(limits[n].to_le_bytes());
    plc.extend(1u32.to_le_bytes());
    (word, plc)
}
fn source(text: &str) -> Inspection {
    Inspection {
        schema_version: 1,
        status: "synthetic",
        binary_version: 0x112,
        trailing_container_bytes: 0,
        piece_count: 1,
        trailing_utf16_units: vec![],
        stories: vec![crate::office::doc::Story {
            kind: "body",
            start_cp: 0,
            utf16_units: text.encode_utf16().count(),
            text: text.into(),
        }],
    }
}
fn endpoint(end_cp: usize) -> ParagraphEndpoint {
    ParagraphEndpoint {
        end_cp,
        piece_index: 0,
        fkp_offset: 512,
        fkp_run: 0,
        end_fc: end_cp,
    }
}
#[test]
fn split_and_reordered_pieces_use_physical_endpoints() {
    let (word, plc) = physical(&[0, 6, 12, 20]);
    let pieces = [
        piece(0, 2, 0, true),
        piece(2, 4, 10, true),
        piece(4, 6, 4, true),
    ];
    let ends = physical_endpoints(&word, &plc, &pieces).unwrap();
    assert_eq!(
        ends.iter().map(|e| e.end_cp).collect::<Vec<_>>(),
        vec![4, 6]
    );
    assert_eq!((ends[0].piece_index, ends[0].end_fc), (1, 12));
    assert_eq!((ends[1].piece_index, ends[1].end_fc), (2, 6));
}
#[test]
fn uncompressed_endpoint_counts_utf16_and_rejects_half_units() {
    let (word, plc) = physical(&[0, 6]);
    assert_eq!(
        physical_endpoints(&word, &plc, &[piece(0, 3, 0, false)]).unwrap()[0].end_cp,
        3
    );
    let (word, plc) = physical(&[0, 3]);
    assert!(physical_endpoints(&word, &plc, &[piece(0, 3, 0, false)]).is_err());
}
#[test]
fn interior_controls_do_not_create_paragraphs() {
    let raw = source("A\rB\u{c}C\r");
    let visible = crate::office::doc::transcribe(&raw);
    let plan = plan(&raw, &visible, &[endpoint(6)]).unwrap();
    assert_eq!(plan.stories[0].paragraphs.len(), 1);
    assert_eq!(plan.stories[0].paragraphs[0].source_range, [0, 6]);
    assert_eq!(plan.stories[0].paragraphs[0].projected_bytes, Some([0, 6]));
}
#[test]
fn missing_and_unexpected_endpoints_remain_diagnosed() {
    let raw = source("AB\rC");
    let visible = crate::office::doc::transcribe(&raw);
    let plan = plan(&raw, &visible, &[endpoint(2)]).unwrap();
    let p = &plan.stories[0].paragraphs;
    assert!(
        p[0].unresolved
            .contains(&"source_paragraph_terminal_unexpected")
    );
    assert!(
        p[1].unresolved
            .contains(&"source_paragraph_endpoint_missing")
    );
    assert_eq!(p[1].source_range, [2, 4]);
}
#[test]
fn generated_labels_belong_to_following_paragraph_and_suppressed_marks_stay_source_owned() {
    let raw = source("A\rB\r");
    let mut visible = crate::office::doc::transcribe(&raw);
    let story = &mut visible.stories[0];
    story.text = "A1.B\n".into();
    story.segments = vec![
        crate::office::doc::SourceSegment {
            kind: "text",
            start_cp: 0,
            end_cp: 1,
            output_start: 0,
            output_end: 1,
        },
        crate::office::doc::SourceSegment {
            kind: "revision_deleted",
            start_cp: 1,
            end_cp: 2,
            output_start: 1,
            output_end: 1,
        },
        crate::office::doc::SourceSegment {
            kind: "generated_list_label",
            start_cp: 2,
            end_cp: 2,
            output_start: 1,
            output_end: 3,
        },
        crate::office::doc::SourceSegment {
            kind: "text",
            start_cp: 2,
            end_cp: 4,
            output_start: 3,
            output_end: 5,
        },
    ];
    let plan = plan(&raw, &visible, &[endpoint(2), endpoint(4)]).unwrap();
    let p = &plan.stories[0].paragraphs;
    assert_eq!(p[0].projected_bytes, Some([0, 1]));
    assert_eq!(p[1].projected_bytes, Some([1, 5]));
    assert_eq!(p[0].terminal_projection, "revision_deleted");
    assert!(
        p[0].unresolved
            .contains(&"suppressed_terminal_visible_topology_unresolved")
    );
}
#[test]
fn nonuniform_and_surrogate_cuts_do_not_fabricate_output_boundaries() {
    let raw = source("😀\r");
    let mut visible = crate::office::doc::transcribe(&raw);
    let p = plan(&raw, &visible, &[endpoint(1), endpoint(3)]).unwrap();
    assert!(
        p.stories[0]
            .paragraphs
            .iter()
            .all(|p| p.projected_bytes.is_none())
    );
    visible.stories[0].text = "label".into();
    visible.stories[0].segments = vec![crate::office::doc::SourceSegment {
        kind: "generated_number",
        start_cp: 0,
        end_cp: 3,
        output_start: 0,
        output_end: 5,
    }];
    let p = plan(&raw, &visible, &[endpoint(2), endpoint(3)]).unwrap();
    assert!(
        p.stories[0]
            .paragraphs
            .iter()
            .all(|p| p.projected_bytes.is_none())
    );
}
#[test]
fn invalid_source_and_projection_orders_return_errors() {
    let raw = source("A\r");
    let mut visible = crate::office::doc::transcribe(&raw);
    assert!(plan(&raw, &visible, &[endpoint(2), endpoint(1)]).is_err());
    visible.stories[0].segments[0].output_end = usize::MAX;
    assert!(plan(&raw, &visible, &[endpoint(2)]).is_err());
    let (word, mut plc) = physical(&[0, 2]);
    plc[4..8].copy_from_slice(&0u32.to_le_bytes());
    assert!(physical_endpoints(&word, &plc, &[piece(0, 2, 0, true)]).is_err());
}

#[test]
fn plc_interval_selects_page_without_inventing_paragraph_boundary() {
    let (word, _) = physical(&[0, 6, 10]);
    let mut plc = Vec::new();
    for x in [0u32, 4, 10, 1, 1] {
        plc.extend(x.to_le_bytes());
    }
    let ends = physical_endpoints(&word, &plc, &[piece(0, 10, 0, true)]).unwrap();
    assert_eq!(
        ends.iter().map(|e| e.end_cp).collect::<Vec<_>>(),
        vec![6, 10]
    );
    plc[8..12].copy_from_slice(&11u32.to_le_bytes());
    assert!(physical_endpoints(&word, &plc, &[piece(0, 10, 0, true)]).is_err());
}
#[test]
fn source_identity_checks_each_story_not_just_total() {
    let mut raw = source("A\r");
    let names = [
        "footnotes",
        "headers",
        "reserved_macro",
        "comments",
        "endnotes",
        "textboxes",
        "header_textboxes",
    ];
    for kind in names {
        raw.stories.push(crate::office::doc::Story {
            kind,
            start_cp: 2,
            utf16_units: 0,
            text: String::new(),
        });
    }
    let lengths = [2, 0, 0, 0, 0, 0, 0, 0];
    assert!(validate_source_identity(0x112, &lengths, &raw).is_ok());
    raw.stories[0].utf16_units = 1;
    raw.stories[1].utf16_units = 1;
    assert!(validate_source_identity(0x112, &lengths, &raw).is_err());
    raw.stories[0].utf16_units = 2;
    raw.stories[1].utf16_units = 0;
    raw.stories[1].kind = "headers";
    assert!(validate_source_identity(0x112, &lengths, &raw).is_err());
}
