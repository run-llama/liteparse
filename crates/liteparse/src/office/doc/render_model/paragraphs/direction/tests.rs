use super::*;
use crate::office::doc::formatting::PropertyModifier;

#[test]
fn source_alignment_aliases_and_direction_resets_preserve_physical_meaning() {
    let mut p = super::super::tests::paragraph();
    let apply = |p: &mut ParagraphProperties, sprm, value| {
        super::super::apply(
            p,
            &PropertyModifier {
                sprm,
                operand: vec![value],
            },
            0,
        )
        .unwrap();
    };
    apply(&mut p, 0x2441, 1);
    assert!(resolve(&p).unwrap().rtl);
    assert_eq!(resolve(&p).unwrap().physical_justification, 0);
    apply(&mut p, 0x2461, 0);
    assert_eq!(resolve(&p).unwrap().physical_justification, 2);
    apply(&mut p, 0x2403, 0);
    assert_eq!(resolve(&p).unwrap().physical_justification, 0);
    apply(&mut p, 0x2461, 2);
    assert_eq!(resolve(&p).unwrap().physical_justification, 0);
    apply(&mut p, 0x2441, 0);
    assert!(!resolve(&p).unwrap().rtl);
    assert_eq!(resolve(&p).unwrap().physical_justification, 2);
}

#[test]
fn rtl_layout_context_and_malformed_direction_stay_explicit() {
    let mut p = super::super::tests::paragraph();
    p.applied.push(PropertyModifier {
        sprm: 0x2441,
        operand: vec![1],
    });
    p.left_twips = 120;
    assert!(resolve(&p).is_err());
    p.left_twips = 0;
    p.first_twips = -120;
    assert!(resolve(&p).is_err());
    p.first_twips = 0;
    for value in [vec![], vec![2], vec![128], vec![1, 0]] {
        p.applied[0].operand = value;
        assert!(resolve(&p).is_err());
    }
}
