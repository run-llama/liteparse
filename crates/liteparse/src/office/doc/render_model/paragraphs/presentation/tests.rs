use super::*;

fn property(sprm: u16, operand: &[u8]) -> PropertyModifier {
    PropertyModifier {
        sprm,
        operand: operand.to_vec(),
    }
}

#[test]
fn decoration_batch_preserves_order_without_changing_text_layout_properties() {
    let mut result = super::super::tests::paragraph();
    let before = result.clone();
    let properties = [
        property(0x6426, &[8, 1, 1, 0]),
        property(0x442d, &[0, 4]),
        property(0x4459, &[50, 0]),
        property(0x2433, &[1]),
        property(0xc64d, &[10, 0, 0, 0, 255, 255, 255, 255, 0, 1, 0]),
        property(0x6426, &[0, 0, 0, 0]),
    ];
    for p in &properties {
        super::super::apply(&mut result, p, 0).unwrap();
    }
    assert_eq!(result.applied, properties);
    result.applied.clear();
    assert_eq!(result, before);
}

#[test]
fn malformed_presentation_and_structural_properties_still_reject() {
    for p in [
        property(0x6426, &[8, 2, 1, 0]),
        property(0x6426, &[8, 1, 17, 0]),
        property(0xc64e, &[7, 0, 0, 0, 0, 8, 1, 0, 0]),
        property(0x442d, &[31, 0]),
        property(0xc64d, &[10, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0]),
        property(0x4459, &(-21i16).to_le_bytes()),
        property(0x2433, &[2]),
        property(0x2416, &[1]),
        property(0x2441, &[2]),
        property(0x6649, &[1, 0, 0, 0]),
        property(0x2666, &[0]),
        property(0x6429, &[0, 0, 0, 0]),
        property(0x442b, &[0, 128]),
    ] {
        assert!(
            super::super::apply(&mut super::super::tests::paragraph(), &p, 0).is_err(),
            "{p:?}"
        );
    }
}

#[test]
fn source_units_and_nil_shading_have_distinct_typed_evidence() {
    assert_eq!(
        decode(&property(0x4458, &(-20i16).to_le_bytes())).unwrap(),
        Some(Presentation::LineSpacing {
            before: true,
            hundredths: -20
        })
    );
    assert_eq!(
        decode(&property(0x442d, &[255, 255])).unwrap(),
        Some(Presentation::Shading {
            legacy: true,
            pattern: 63
        })
    );
    assert_eq!(decode(&property(0x2416, &[1])).unwrap(), None);
}

#[test]
fn frames_reflow_in_source_order_without_interpreting_absolute_coordinates() {
    let mut p = super::super::tests::paragraph();
    let source = [
        property(0x8418, &[37, 15]),
        property(0x8419, &[222, 255]),
        property(0x261b, &[160]),
        property(0x842f, &[180, 0]),
        property(0x841a, &[160, 5]),
    ];
    for op in &source {
        super::super::apply(&mut p, op, 0).unwrap();
    }
    assert_eq!(p.applied, source);
    assert_eq!((p.left_twips, p.right_twips, p.first_twips), (0, 0, 0));
    for op in [
        property(0x841a, &[255, 255]),
        property(0x8419, &[255, 127]),
        property(0x2423, &[6]),
    ] {
        assert!(decode(&op).is_err());
    }
}
