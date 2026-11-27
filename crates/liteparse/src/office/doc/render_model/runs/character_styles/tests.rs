use super::*;
use crate::office::doc::styles::{Style, StyleProperties};
fn p(sprm: u16, operand: &[u8]) -> PropertyModifier {
    PropertyModifier {
        sprm,
        operand: operand.into(),
    }
}
fn style(index: usize, kind: u8, base: Option<usize>, properties: &[PropertyModifier]) -> Style {
    Style {
        index,
        builtin_id: 0,
        kind,
        base_style: base,
        next_style: index,
        linked_style: None,
        revision_marked: false,
        name: "fixture".into(),
        raw_flags: 0,
        properties: vec![StyleProperties {
            kind: "character",
            bytes: properties
                .iter()
                .flat_map(|p| {
                    p.sprm
                        .to_le_bytes()
                        .into_iter()
                        .chain(p.operand.iter().copied())
                })
                .collect(),
        }],
        raw_revision: vec![],
    }
}
fn fixture() -> (Stylesheet, FontTable) {
    let mut styles: Vec<_> = (0..14).map(|_| None).collect();
    styles[0] = Some(style(
        0,
        1,
        None,
        &[p(0x4873, &[9, 4]), p(0x4a43, &[24, 0])],
    ));
    styles[10] = Some(style(10, 2, None, &[]));
    styles[11] = Some(style(
        11,
        2,
        Some(10),
        &[p(0x0835, &[129]), p(0x2a48, &[1])],
    ));
    styles[12] = Some(style(
        12,
        2,
        Some(11),
        &[p(0x0835, &[129]), p(0x4a43, &[20, 0])],
    ));
    let fonts = FontTable {
        status: "fixture",
        fonts: vec![crate::office::doc::symbols::Font {
            index: 0,
            name: "Fixture".into(),
            alternate_name: None,
            charset: 0,
            family: 0,
            weight: 400,
            panose: vec![],
            signature: vec![],
        }],
    };
    (
        Stylesheet {
            schema_version: 1,
            status: "fixture",
            std_base_bytes: 10,
            raw_header: vec![0; 20],
            styles,
            unresolved: [""; 3],
        },
        fonts,
    )
}
#[test]
fn reset_and_ordered_ancestry_preserve_note_reference_presentation() {
    let (styles, fonts) = fixture();
    let run = resolve(
        &styles,
        &fonts,
        0,
        &[p(0x4a43, &[48, 0]), p(0x4a30, &[11, 0])],
    )
    .unwrap();
    assert_eq!(run.size_half_points, 24);
    assert!(run.bold);
    assert_eq!(run.vertical_alignment, 1);
    let child = resolve(
        &styles,
        &fonts,
        0,
        &[p(0x4a30, &[12, 0]), p(0x0835, &[129])],
    )
    .unwrap();
    assert!(child.bold);
    assert_eq!(child.size_half_points, 20);
    assert_eq!(child.vertical_alignment, 1);
}
#[test]
fn selected_style_preserves_hint_direction_and_later_direct_override() {
    let (styles, fonts) = fixture();
    let run = resolve(
        &styles,
        &fonts,
        0,
        &[
            p(0x085a, &[1]),
            p(0x286f, &[2]),
            p(0x4a30, &[11, 0]),
            p(0x4a43, &[22, 0]),
        ],
    )
    .unwrap();
    assert!(run.bidi);
    assert_eq!(run.hint, 2);
    assert_eq!(run.size_half_points, 22);
}
#[test]
fn selected_style_rejects_revision_cycle_and_wrong_kind() {
    let (mut styles, fonts) = fixture();
    styles.styles[11].as_mut().unwrap().revision_marked = true;
    assert!(resolve(&styles, &fonts, 0, &[p(0x4a30, &[11, 0])]).is_err());
    styles.styles[11].as_mut().unwrap().revision_marked = false;
    styles.styles[11].as_mut().unwrap().base_style = Some(11);
    assert!(resolve(&styles, &fonts, 0, &[p(0x4a30, &[11, 0])]).is_err());
    assert!(resolve(&styles, &fonts, 0, &[p(0x4a30, &[0, 0])]).is_err());
    assert!(resolve(&styles, &fonts, 0, &[p(0x4a30, &[11])]).is_err());
}

#[test]
fn repeated_public_resolutions_charge_one_document_expansion_budget() {
    let (mut styles, fonts) = fixture();
    let repeated = vec![p(0x0835, &[1]); 3000];
    styles.styles[11] = Some(style(11, 2, Some(10), &repeated));
    let mut work = 0;
    let mut limited = false;
    for _ in 0..1000 {
        match resolve_with_budget(&styles, &fonts, 0, &[p(0x4a30, &[11, 0])], &mut work) {
            Ok(run) => assert!(run.bold),
            Err(Error::Limit(_)) => {
                limited = true;
                break;
            }
            Err(error) => panic!("unexpected resolution error: {error}"),
        }
    }
    assert!(
        limited,
        "selected-style expansion must share the document budget"
    );
}

#[test]
fn paragraph_ancestry_without_cistd_shares_the_public_document_budget() {
    let (mut styles, fonts) = fixture();
    let mut properties = vec![p(0x0835, &[1]); 3000];
    properties.push(p(0x4873, &[9, 4]));
    styles.styles[0] = Some(style(0, 1, None, &properties));
    let mut work = 0;
    let mut limited = false;
    for _ in 0..1000 {
        match resolve_with_budget(&styles, &fonts, 0, &[], &mut work) {
            Ok(run) => assert!(run.bold),
            Err(Error::Limit(_)) => {
                limited = true;
                break;
            }
            Err(error) => panic!("unexpected resolution error: {error}"),
        }
    }
    assert!(
        limited,
        "paragraph ancestry replay must share the document budget"
    );
}

#[test]
fn direct_relative_toggles_use_selected_style_not_paragraph_baseline() {
    let (styles, fonts) = fixture();
    let inverted = resolve(
        &styles,
        &fonts,
        0,
        &[p(0x4a30, &[11, 0]), p(0x0835, &[129])],
    )
    .unwrap();
    assert!(!inverted.bold);
    let inherited = resolve(
        &styles,
        &fonts,
        0,
        &[p(0x4a30, &[11, 0]), p(0x0835, &[0]), p(0x0835, &[128])],
    )
    .unwrap();
    assert!(inherited.bold);
    let changed = resolve(
        &styles,
        &fonts,
        0,
        &[
            p(0x4a30, &[11, 0]),
            p(0x0835, &[129]),
            p(0x4a30, &[12, 0]),
            p(0x0835, &[129]),
        ],
    )
    .unwrap();
    assert!(changed.bold);
    assert_eq!(changed.size_half_points, 20);
}
