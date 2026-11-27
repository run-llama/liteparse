use super::*;
use crate::office::doc::textbox_anchors::{TextboxRange, TextboxRecord};
fn fixture() -> (Story, TextboxPart) {
    (
        Story {
            kind: "textboxes",
            start_cp: 10,
            utf16_units: 3,
            text: "\r\r\r".into(),
        },
        TextboxPart {
            text_story: "textboxes",
            anchor_story: "body",
            shapes: vec![],
            records: (0..3)
                .map(|i| TextboxRecord {
                    record_index: i,
                    raw_cp_range: [i as u32, if i == 2 { u32::MAX } else { i as u32 + 1 }],
                    reusable: true,
                    source: None,
                    shape_id: None,
                    chain_shape_count: None,
                    anchor_index: None,
                    anchor_status: "reusable_slot",
                })
                .collect(),
        },
    )
}
#[test]
fn retains_each_reusable_cp_and_ignored_terminal_bound() {
    let (story, part) = fixture();
    let value = qualify(&story, &part).unwrap();
    assert_eq!(value.source_text, "\r\r\r");
    assert_eq!(value.records[2].source_cp, [12, 13]);
    assert_eq!(value.records[2].raw_cp_range[1], u32::MAX);
    assert!(validate_drawings(&[value], None).is_ok());
}
#[test]
fn authored_box_and_missing_storage_ranges_reject() {
    let (story, mut part) = fixture();
    part.records[0].reusable = false;
    part.records[0].source = Some(TextboxRange {
        start_cp: 10,
        end_cp: 12,
    });
    assert!(qualify(&story, &part).is_err());
    let (story, mut part) = fixture();
    part.records[0].raw_cp_range[0] = 1;
    assert!(qualify(&story, &part).is_err());
    let (story, mut part) = fixture();
    part.records[1].raw_cp_range[1] = 3;
    assert!(qualify(&story, &part).is_err());
}
#[test]
fn terminal_ignored_text_is_preserved_without_lexical_reinterpretation() {
    let (mut story, mut part) = fixture();
    part.records.truncate(1);
    part.records[0].raw_cp_range = [0, u32::MAX];
    story.text = "old".into();
    let value = qualify(&story, &part).unwrap();
    assert_eq!(value.source_text, "old");
    assert_eq!(value.records[0].source_cp, [10, 13]);
}
#[test]
fn public_validation_rejects_duplicated_or_missing_records() {
    let (story, part) = fixture();
    let mut value = qualify(&story, &part).unwrap();
    value.records.remove(0);
    assert!(value.validate().is_err());
    let a = qualify(&story, &part).unwrap();
    let b = qualify(&story, &part).unwrap();
    assert!(validate_all(&[a, b]).is_err());
}

#[test]
fn active_box_preserves_text_and_joins_unique_anchor() {
    use crate::office::doc::textbox_anchors::ShapeAnchor;
    let (mut story, mut part) = fixture();
    story.text = "A\r\r".into();
    part.records.remove(1);
    part.records[0].raw_cp_range = [0, 2];
    part.records[0].reusable = false;
    part.records[0].source = Some(TextboxRange {
        start_cp: 10,
        end_cp: 12,
    });
    part.records[0].shape_id = Some(7);
    part.records[0].chain_shape_count = Some(1);
    part.records[0].anchor_index = Some(0);
    part.records[1].record_index = 1;
    part.shapes.push(ShapeAnchor {
        shape_id: 7,
        source_cp: 0,
        rectangle_twips: [0, 0, 100, 100],
        raw_flags: 116,
    });
    let value = qualify(&story, &part).unwrap();
    assert_eq!(value.textboxes[0].source_text, "A\r");
    assert_eq!(value.textboxes[0].source_cp, [10, 12]);
    assert_eq!(value.records[1].source_cp, [12, 13]);
    assert!(matches!(
        validate_drawings(&[value], None),
        Err(Error::Invalid("render textbox drawing cardinality"))
    ));
    part.records[0].shape_id = Some(8);
    assert!(qualify(&story, &part).is_err());
    part.records[0].shape_id = Some(7);
    part.records[0].chain_shape_count = Some(2);
    assert!(qualify(&story, &part).is_err());
}

#[test]
fn table_only_compatibility_stays_guarded_when_tables_are_present() {
    for name in ["word2002_table_style_rules", "grow_autofit"] {
        for grid in [0, 1, 2, 3] {
            assert!(super::super::unsupported_table_compatibility(name, grid));
        }
    }
}

#[test]
fn storage_order_does_not_impose_anchor_order() {
    let value = TextboxStorage {
        story: "textboxes",
        source_cp: [10, 15],
        source_text: "A\rB\r\r".into(),
        records: vec![
            StorageRecord {
                record_index: 0,
                raw_cp_range: [0, 2],
                source_cp: [10, 12],
                kind: "authored_textbox",
            },
            StorageRecord {
                record_index: 1,
                raw_cp_range: [2, 4],
                source_cp: [12, 14],
                kind: "authored_textbox",
            },
            StorageRecord {
                record_index: 2,
                raw_cp_range: [4, u32::MAX],
                source_cp: [14, 15],
                kind: "terminal_reusable_storage",
            },
        ],
        textboxes: vec![
            AuthoredTextbox {
                record_index: 0,
                shape_id: 7,
                anchor_cp: 100,
                raw_anchor_flags: 116,
                rectangle_twips: [0, 0, 100, 100],
                source_cp: [10, 12],
                source_text: "A\r".into(),
                paragraphs: vec![],
                group: None,
                group_rectangle_twips: None,
            },
            AuthoredTextbox {
                record_index: 1,
                shape_id: 8,
                anchor_cp: 20,
                raw_anchor_flags: 116,
                rectangle_twips: [0, 0, 100, 100],
                source_cp: [12, 14],
                source_text: "B\r".into(),
                paragraphs: vec![],
                group: None,
                group_rectangle_twips: None,
            },
        ],
    };
    value.validate().unwrap();
    assert_eq!(
        anchor_cps(std::slice::from_ref(&value))
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>(),
        vec![20, 100]
    );
    let mut duplicate = value;
    duplicate.textboxes[1].shape_id = 7;
    assert!(duplicate.validate().is_err());
}

#[test]
fn header_box_retains_story_and_rejects_wrong_shape_or_linked_chain() {
    use crate::office::doc::textbox_anchors::ShapeAnchor;
    let (mut story, mut part) = fixture();
    story.kind = "header_textboxes";
    part.text_story = "header_textboxes";
    part.anchor_story = "headers";
    story.text = "A\r\r".into();
    part.records.remove(1);
    part.records[0].raw_cp_range = [0, 2];
    part.records[0].reusable = false;
    part.records[0].source = Some(TextboxRange {
        start_cp: 10,
        end_cp: 12,
    });
    part.records[0].shape_id = Some(7);
    part.records[0].chain_shape_count = Some(1);
    part.records[0].anchor_index = Some(0);
    part.records[1].record_index = 1;
    part.shapes.push(ShapeAnchor {
        shape_id: 7,
        source_cp: 5,
        rectangle_twips: [0, 0, 100, 100],
        raw_flags: 116,
    });
    let value = qualify(&story, &part).unwrap();
    assert_eq!(value.textboxes[0].source_text, "A\r");
    assert_eq!(value.textboxes[0].source_cp, [10, 12]);
    assert_eq!(value.records[1].source_cp, [12, 13]);
    assert!(matches!(
        validate_drawings(&[value], None),
        Err(Error::Invalid("render textbox drawing cardinality"))
    ));
    part.records[0].shape_id = Some(8);
    assert!(qualify(&story, &part).is_err());
    part.records[0].shape_id = Some(7);
    part.records[0].chain_shape_count = Some(2);
    assert!(qualify(&story, &part).is_err());
}
