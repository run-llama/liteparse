use super::*;
fn record(kind: u16, version: u16, instance: u16, body: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&((instance << 4) | version).to_le_bytes());
    b.extend_from_slice(&kind.to_le_bytes());
    b.extend_from_slice(&(body.len() as u32).to_le_bytes());
    b.extend_from_slice(body);
    b
}
fn png() -> Vec<u8> {
    let mut b = vec![0; 17];
    b.extend_from_slice(b"\x89PNG\r\n\x1a\nfixture");
    record(0xf01e, 0, 0x6e0, &b)
}
fn shape(id: u32, property: Option<u32>) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&id.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    let mut body = record(0xf00a, 2, 0, &b);
    if let Some(value) = property {
        let mut p = 0x4104u16.to_le_bytes().to_vec();
        p.extend_from_slice(&value.to_le_bytes());
        body.extend(record(0xf00b, 3, 1, &p));
    }
    record(0xf004, 15, 0, &body)
}
fn decoder(word: &[u8]) -> Decoder<'_> {
    Decoder {
        word,
        budget: Budget::default(),
        cache: BTreeMap::new(),
        output: PictureInspection {
            status: "test",
            assets: vec![],
            references: vec![],
            diagnostics: vec![],
            source_picture_markers: 0,
            source_drawing_markers: 0,
        },
    }
}
#[test]
fn floating_delayed_asset_joins_group_anchor_without_claiming_direct_anchor() {
    let mut fbse = vec![0; 36];
    fbse[0] = 6;
    fbse[1] = 6;
    fbse[20..24].copy_from_slice(&(png().len() as u32).to_le_bytes());
    fbse[24..28].copy_from_slice(&1u32.to_le_bytes());
    let store = record(0xf001, 15, 1, &record(0xf007, 2, 6, &fbse));
    let mut drawing = record(0xf000, 15, 0, &store);
    drawing.push(0);
    let mut group = shape(42, None);
    group.extend(shape(43, Some(1)));
    drawing.extend(record(0xf002, 15, 0, &record(0xf003, 15, 0, &group)));
    let anchors = crate::office::doc::textbox_anchors::TextboxInspection {
        status: "test",
        unresolved: [""; 5],
        parts: vec![crate::office::doc::textbox_anchors::TextboxPart {
            text_story: "textboxes",
            anchor_story: "body",
            records: vec![],
            shapes: vec![crate::office::doc::textbox_anchors::ShapeAnchor {
                shape_id: 42,
                source_cp: 17,
                rectangle_twips: [0; 4],
                raw_flags: 0,
            }],
        }],
    };
    let word = png();
    let mut d = decoder(&word);
    floating(
        &mut d,
        &drawing,
        0,
        drawing.len(),
        Some(&anchor_map(&anchors)),
    )
    .unwrap();
    assert_eq!(d.output.assets.len(), 1);
    let r = &d.output.references[0];
    assert_eq!(r.source_cp, None);
    assert_eq!(r.group_anchor_cp, Some(17));
    assert_eq!(r.shape_id, Some(43));
    assert_eq!(d.output.assets[0].source.stream, "WordDocument");
    assert_eq!(d.output.assets[0].bytes(), b"\x89PNG\r\n\x1a\nfixture");
}
#[test]
fn inline_shape_uses_payload_order_not_property_value() {
    let mut b = vec![0; 68];
    b[4..6].copy_from_slice(&68u16.to_le_bytes());
    b[6..8].copy_from_slice(&100u16.to_le_bytes());
    b.extend(shape(99, Some(12345)));
    b.extend(png());
    let n = b.len() as u32;
    b[..4].copy_from_slice(&n.to_le_bytes());
    let mut d = decoder(&[]);
    inline_record(&mut d, &b, 0, 8, "body").unwrap();
    assert_eq!(d.output.references[0].source_cp, Some(8));
    assert_eq!(d.output.references[0].asset_index, Some(0));
}
#[test]
fn unsupported_store_slot_does_not_shift_following_image_id() {
    let empty = record(0xf01c, 0, 0, &[]);
    let mut store = empty;
    store.extend(png());
    let mut b = record(0xf000, 15, 0, &record(0xf001, 15, 2, &store));
    b.push(0);
    b.extend(record(0xf002, 15, 0, &shape(3, Some(2))));
    let mut d = decoder(&[]);
    floating(&mut d, &b, 0, b.len(), None).unwrap();
    assert_eq!(d.output.assets.len(), 1);
    assert_eq!(d.output.references[0].asset_index, Some(0));
}

#[test]
fn image_export_writes_decoder_ready_bytes_and_refuses_existing_directory() {
    let word = png();
    let mut d = decoder(&word);
    let r = d.budget.record(&word, 0, word.len()).unwrap();
    d.asset(&word, r, "WordDocument").unwrap();
    let path = std::env::temp_dir().join(format!(
        "doc-picture-export-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let names = export_assets(&d.output, &path).unwrap();
    assert_eq!(names, ["asset-00000.png"]);
    assert_eq!(
        std::fs::read(path.join(&names[0])).unwrap(),
        d.output.assets[0].bytes()
    );
    assert!(path.join("manifest.json").is_file());
    assert!(export_assets(&d.output, &path).is_err());
    std::fs::remove_dir_all(path).unwrap();
}
#[test]
fn invalid_complex_property_and_missing_inline_payload_are_explicit() {
    let mut property = 0xc104u16.to_le_bytes().to_vec();
    property.extend_from_slice(&100u32.to_le_bytes());
    let data = record(0xf00b, 3, 1, &property);
    let r = Budget::default().record(&data, 0, data.len()).unwrap();
    assert!(picture_properties(&data, r, false).is_err());
    let mut b = vec![0; 68];
    b[4..6].copy_from_slice(&68u16.to_le_bytes());
    b[6..8].copy_from_slice(&100u16.to_le_bytes());
    b.extend(shape(9, Some(1)));
    let n = b.len() as u32;
    b[..4].copy_from_slice(&n.to_le_bytes());
    assert!(inline_record(&mut decoder(&[]), &b, 0, 0, "body").is_err());
}
#[test]
fn embedded_fbse_precedes_invalid_delay_and_retains_payload_bounds() {
    let mut fbse = vec![0; 36];
    fbse[0] = 6;
    fbse[1] = 6;
    fbse[28..32].copy_from_slice(&12345u32.to_le_bytes());
    fbse.extend(png());
    let b = record(0xf007, 2, 6, &fbse);
    let mut d = decoder(&[]);
    let r = d.budget.record(&b, 0, b.len()).unwrap();
    let index = d.asset(&b, r, "Data").unwrap();
    assert_eq!(d.output.assets[index].bytes(), b"\x89PNG\r\n\x1a\nfixture");
    assert_eq!(d.output.assets[index].source.stream, "Data");
    assert_eq!(d.asset(&b, r, "Data").unwrap(), index);
    assert_eq!(d.output.assets.len(), 1);
}

#[test]
fn non_image_complex_properties_ignore_fbid_without_consuming_inline_blips() {
    for id in [0x105u16, 0x30c, 0x380, 0x381] {
        let mut properties = 0x4104u16.to_le_bytes().to_vec();
        properties.extend_from_slice(&0u32.to_le_bytes());
        properties.extend_from_slice(&(id | 0xc000).to_le_bytes());
        properties.extend_from_slice(&2u32.to_le_bytes());
        properties.extend_from_slice(&[0, 0]);
        let b = record(0xf00b, 3, 2, &properties);
        let r = Budget::default().record(&b, 0, b.len()).unwrap();
        let props = picture_properties(&b, r, true).unwrap();
        assert_eq!(props.len(), 1);
        assert_eq!(props[0].role, "picture");
    }
}

#[test]
fn duplicate_anchor_ids_do_not_claim_unique_source_cp() {
    let anchors = crate::office::doc::textbox_anchors::TextboxInspection {
        status: "test",
        unresolved: [""; 5],
        parts: vec![crate::office::doc::textbox_anchors::TextboxPart {
            text_story: "textboxes",
            anchor_story: "body",
            records: vec![],
            shapes: vec![
                crate::office::doc::textbox_anchors::ShapeAnchor {
                    shape_id: 7,
                    source_cp: 1,
                    rectangle_twips: [0; 4],
                    raw_flags: 0,
                },
                crate::office::doc::textbox_anchors::ShapeAnchor {
                    shape_id: 7,
                    source_cp: 3,
                    rectangle_twips: [0; 4],
                    raw_flags: 0,
                },
            ],
        }],
    };
    assert_eq!(anchor_map(&anchors).get(&("body", 7)), Some(&None));
    let mut b = record(0xf000, 15, 0, &record(0xf001, 15, 1, &png()));
    b.push(0);
    b.extend(record(0xf002, 15, 0, &shape(7, Some(1))));
    let mut d = decoder(&[]);
    floating(&mut d, &b, 0, b.len(), Some(&anchor_map(&anchors))).unwrap();
    assert_eq!(d.output.references[0].source_cp, None);
    assert_eq!(d.output.references[0].group_anchor_cp, None);
    assert_eq!(d.output.references[0].asset_index, Some(0));
}

#[test]
fn binary_data_distinction_preserves_reset_and_piece_override_order() {
    use crate::office::doc::formatting::{CharacterSpan, PropertyModifier};
    use crate::office::doc::formatting_inspection::{FormattingInspection, PieceProperties};
    let source = crate::office::doc::Inspection {
        schema_version: 1,
        status: "fixture",
        binary_version: 0xc1,
        trailing_container_bytes: 0,
        piece_count: 1,
        trailing_utf16_units: vec![],
        stories: vec![crate::office::doc::Story {
            kind: "body",
            start_cp: 0,
            utf16_units: 1,
            text: "\u{1}".into(),
        }],
    };
    let mut formatting = FormattingInspection {
        schema_version: 1,
        status: "fixture",
        binary_version: 0xc1,
        authored_story_end_cp: 1,
        trailing_nonstory_cp_range: None,
        raw_property_groups: vec![],
        unresolved: [""; 3],
        pieces: vec![PieceProperties {
            start_cp: 0,
            end_cp: 1,
            file_offset: 0,
            compressed: true,
            raw_flags: 0,
            raw_prm: 0,
        }],
        character_spans: vec![CharacterSpan {
            start_cp: 0,
            end_cp: 1,
            properties: vec![
                PropertyModifier {
                    sprm: 0x6a03,
                    operand: vec![0; 4],
                },
                PropertyModifier {
                    sprm: 0x0806,
                    operand: vec![1],
                },
                PropertyModifier {
                    sprm: 0x2a33,
                    operand: vec![0],
                },
            ],
        }],
    };
    let mut binary = decoder(&[]);
    inline(&mut binary, &[], &source, &formatting).unwrap();
    assert_eq!(binary.output.references[0].role, "binary_data");
    assert!(binary.output.assets.is_empty());
    formatting.pieces[0].raw_prm = 0x47 << 1;
    let mut picture = decoder(&[]);
    inline(&mut picture, &[], &source, &formatting).unwrap();
    assert_eq!(picture.output.references[0].role, "picture");
    assert!(picture.output.references[0].unresolved[0].contains("truncated"));
}

#[test]
fn later_duplicate_shape_container_invalidates_both_direct_anchors() {
    let mut b = record(0xf000, 15, 0, &record(0xf001, 15, 1, &png()));
    for _ in 0..2 {
        b.push(0);
        b.extend(record(0xf002, 15, 0, &shape(7, Some(1))));
    }
    let anchors = AnchorMap::from([(("body", 7), Some(11))]);
    let mut d = decoder(&[]);
    floating(&mut d, &b, 0, b.len(), Some(&anchors)).unwrap();
    assert_eq!(d.output.references.len(), 2);
    assert_eq!(d.output.assets.len(), 1);
    assert!(
        d.output.references.iter().all(|r| r.source_cp.is_none()
            && r.group_anchor_cp.is_none()
            && r.asset_index == Some(0))
    );
    assert!(d.output.diagnostics[0].contains("duplicate drawing shape IDs"));
}

#[test]
fn duplicate_nested_group_ids_invalidate_all_descendant_group_anchors() {
    let mut b = record(0xf000, 15, 0, &record(0xf001, 15, 1, &png()));
    b.push(0);
    let mut groups = Vec::new();
    for child_id in [8, 9] {
        let mut children = shape(7, None);
        children.extend(shape(child_id, Some(1)));
        groups.extend(record(0xf003, 15, 0, &children));
    }
    b.extend(record(0xf002, 15, 0, &groups));
    let anchors = AnchorMap::from([(("body", 7), Some(11))]);
    let mut d = decoder(&[]);
    floating(&mut d, &b, 0, b.len(), Some(&anchors)).unwrap();
    assert_eq!(d.output.references.len(), 2);
    assert!(
        d.output
            .references
            .iter()
            .all(|r| r.group_anchor_cp.is_none() && r.asset_index == Some(0))
    );
}

#[test]
fn shape_ids_in_different_stories_have_independent_ownership() {
    let mut b = record(0xf000, 15, 0, &record(0xf001, 15, 1, &png()));
    for story in [0, 1] {
        b.push(story);
        b.extend(record(0xf002, 15, 0, &shape(7, Some(1))));
    }
    let anchors = AnchorMap::from([(("body", 7), Some(11)), (("headers", 7), Some(91))]);
    let mut d = decoder(&[]);
    floating(&mut d, &b, 0, b.len(), Some(&anchors)).unwrap();
    assert_eq!(d.output.references[0].source_cp, Some(11));
    assert_eq!(d.output.references[1].source_cp, Some(91));
}
