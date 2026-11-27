use super::*;

fn p(sprm: u16, operand: &[u8]) -> PropertyModifier {
    PropertyModifier {
        sprm,
        operand: operand.to_vec(),
    }
}

#[test]
fn defaults_and_later_resets_preserve_explicit_spacing() {
    assert_eq!(resolve(&[]).unwrap(), AutoSpacing::default());
    let properties = [
        p(0x245b, &[1]),
        p(0x245c, &[1]),
        p(0xa413, &[100, 0]),
        p(0x245b, &[0]),
        p(0x245c, &[0]),
    ];
    assert_eq!(resolve(&properties).unwrap(), AutoSpacing::default());
    assert!(resolve(&properties[..4]).unwrap().after);
    assert!(resolve(&[p(0x245b, &[0]), p(0x245b, &[1])]).unwrap().before);
}

#[test]
fn malformed_bool_is_rejected_even_before_reset() {
    for operand in [vec![], vec![2], vec![128], vec![129], vec![255], vec![0, 0]] {
        assert!(resolve(&[p(0x245b, &operand), p(0x245b, &[0])]).is_err());
        assert!(resolve(&[p(0x245c, &operand), p(0x245c, &[0])]).is_err());
    }
}

#[test]
fn paragraph_style_flags_are_overridden_by_direct_source_order() {
    use crate::office::doc::styles::{Style, StyleProperties, Stylesheet};
    let styles = Stylesheet {
        schema_version: 1,
        status: "fixture",
        std_base_bytes: 10,
        raw_header: vec![0; 20],
        unresolved: ["fixture"; 3],
        styles: vec![Some(Style {
            index: 0,
            builtin_id: 0,
            kind: 1,
            base_style: None,
            next_style: 0,
            linked_style: None,
            revision_marked: false,
            name: "fixture".into(),
            raw_flags: 0,
            raw_revision: vec![],
            properties: vec![StyleProperties {
                kind: "paragraph",
                bytes: vec![
                    0, 0, 0x5b, 0x24, 1, 0x5c, 0x24, 1, 0x13, 0xa4, 100, 0, 0x14, 0xa4, 200, 0,
                ],
            }],
        })],
    };
    let active = super::super::resolve(&styles, 0, &[]).unwrap();
    assert_eq!((active.before_twips, active.after_twips), (100, 200));
    assert_eq!(
        resolve(&active.applied).unwrap(),
        AutoSpacing {
            before: true,
            after: true
        }
    );
    let reset = [p(0x245b, &[0]), p(0x245c, &[0])];
    let result = super::super::resolve(&styles, 0, &reset).unwrap();
    assert_eq!((result.before_twips, result.after_twips), (100, 200));
    assert_eq!(&result.applied[result.applied.len() - 2..], &reset);
    let reactivated = [p(0x245b, &[0]), p(0x245c, &[0]), p(0x245c, &[1])];
    assert!(
        resolve(
            &super::super::resolve(&styles, 0, &reactivated)
                .unwrap()
                .applied
        )
        .unwrap()
        .after
    );
    let mut derived_styles = styles;
    let derived = Style {
        index: 1,
        builtin_id: 0,
        kind: 1,
        base_style: Some(0),
        next_style: 1,
        linked_style: None,
        revision_marked: false,
        name: "derived".into(),
        raw_flags: 0,
        raw_revision: vec![],
        properties: vec![StyleProperties {
            kind: "paragraph",
            bytes: vec![1, 0, 0x5b, 0x24, 0, 0x5c, 0x24, 0],
        }],
    };
    derived_styles.styles.push(Some(derived));
    let result = super::super::resolve(&derived_styles, 1, &[]).unwrap();
    assert_eq!((result.before_twips, result.after_twips), (100, 200));
    assert_eq!(&result.applied[result.applied.len() - 2..], &reset);
    assert!(
        resolve(
            &super::super::resolve(&derived_styles, 1, &[p(0x245b, &[1])])
                .unwrap()
                .applied
        )
        .unwrap()
        .before
    );
}
