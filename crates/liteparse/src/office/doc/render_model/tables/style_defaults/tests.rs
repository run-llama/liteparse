use super::*;
use crate::office::doc::styles::{Style, StyleProperties};
fn style(index: usize, base: Option<usize>, extra: Vec<u8>) -> Style {
    let mut table = vec![0x3a, 0x56, index as u8, 0];
    if index == 11 {
        table.extend_from_slice(&[0x17, 0xf6, 3, 0, 0]);
    }
    table.extend(extra);
    Style {
        index,
        builtin_id: 105,
        kind: 3,
        base_style: base,
        next_style: index,
        linked_style: None,
        revision_marked: false,
        name: String::new(),
        raw_flags: 0,
        properties: vec![
            StyleProperties {
                kind: "table",
                bytes: table,
            },
            StyleProperties {
                kind: "paragraph",
                bytes: (index as u16).to_le_bytes().to_vec(),
            },
            StyleProperties {
                kind: "character",
                bytes: vec![],
            },
        ],
        raw_revision: vec![],
    }
}
fn sheet() -> Stylesheet {
    let mut slots: Vec<Option<Style>> = (0..13).map(|_| None).collect();
    slots[11] = Some(style(11, None, vec![]));
    Stylesheet {
        schema_version: 1,
        status: "synthetic",
        std_base_bytes: 18,
        raw_header: vec![],
        styles: slots,
        unresolved: ["a", "b", "c"],
    }
}
#[test]
fn explicit_padding_inherits_base_then_derived_and_retains_identity_record() {
    let mut s = sheet();
    s.styles[11] = Some(style(11, None, vec![0x34, 0xd6, 6, 0, 1, 10, 3, 54, 0]));
    s.styles[12] = Some(style(12, Some(11), vec![0x34, 0xd6, 6, 0, 1, 2, 3, 72, 0]));
    let value = resolve(&s, 12).unwrap();
    assert_eq!(value.base_to_derived, [11, 12]);
    assert_eq!(value.cell.margins_twips, [0, 72, 0, 54]);
    assert_eq!(value.applied.len(), 5);
}
#[test]
fn unresolved_style_text_conditional_or_revision_does_not_create_defaults() {
    let mut s = sheet();
    assert!(resolve(&s, 12).is_err());
    s.styles[12] = Some(style(12, Some(11), vec![0x6a, 0xd6, 0]));
    assert!(resolve(&s, 12).is_err());
    s.styles[12] = Some(style(12, Some(11), vec![]));
    s.styles[12].as_mut().unwrap().properties[2].bytes = vec![0x35, 8, 1];
    assert!(resolve(&s, 12).is_err());
    s.styles[12] = Some(style(12, Some(11), vec![]));
    s.styles[12].as_mut().unwrap().revision_marked = true;
    assert!(resolve(&s, 12).is_err());
    s.styles[12] = Some(style(12, Some(12), vec![]));
    assert!(resolve(&s, 12).is_err());
}
#[test]
fn malformed_padding_and_required_normal_style_width_are_rejected() {
    for operand in [
        vec![6, 0, 2, 10, 3, 108, 0],
        vec![6, 0, 1, 16, 3, 108, 0],
        vec![6, 0, 1, 10, 3, 255, 255],
        vec![6, 0, 1, 10, 2, 108, 0],
        vec![6, 0, 1, 10, 3, 0],
    ] {
        let mut s = sheet();
        let mut extra = vec![0x34, 0xd6];
        extra.extend(operand);
        s.styles[12] = Some(style(12, Some(11), extra));
        assert!(resolve(&s, 12).is_err());
    }
    let mut s = sheet();
    s.styles[11].as_mut().unwrap().properties[0]
        .bytes
        .truncate(4);
    assert!(resolve(&s, 11).is_err());
}

#[test]
fn row_border_style_is_applied_and_recorded_evidence_detects_mutation() {
    let mut s = sheet();
    let mut extra = vec![0x13, 0xd6, 48];
    extra.extend([0, 0, 0, 255, 4, 1, 0, 0].repeat(6));
    s.styles[12] = Some(style(12, Some(11), extra));
    let value = resolve(&s, 12).unwrap();
    assert!(value.row_borders.iter().all(|b| {
        b.as_ref()
            .is_some_and(|b| b.kind == super::super::BorderKind::Single && b.eighth_points == 4)
    }));
    validate(&value).unwrap();
    let mut changed = value.clone();
    changed.row_borders[0].as_mut().unwrap().eighth_points = 8;
    assert!(validate(&changed).is_err());
    let mut changed = value;
    changed.applied.swap(0, 3);
    assert!(validate(&changed).is_err());
}
