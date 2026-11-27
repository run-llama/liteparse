use super::*;
use crate::office::doc::formatting::PropertyModifier;
fn option(sprm: u16, operand: &[u8]) -> SectionOption {
    SectionOption {
        modifier_index: 0,
        name: "fixture",
        property: PropertyModifier {
            sprm,
            operand: operand.to_vec(),
        },
    }
}
#[test]
fn ordered_disable_and_continuous_start_are_source_defined() {
    let mut source = vec![
        option(0x5015, &[3, 0]),
        option(0x501b, &[8, 0]),
        option(0x3013, &[2]),
    ];
    let s = resolve(&source).unwrap().unwrap();
    assert_eq!(s.authored_start_zero_based, 8);
    assert_eq!(s.effective_start_zero_based(), 0);
    source.push(option(0x3013, &[1]));
    assert_eq!(
        resolve(&source)
            .unwrap()
            .unwrap()
            .effective_start_zero_based(),
        8
    );
    source.push(option(0x5015, &[0, 0]));
    assert!(resolve(&source).unwrap().is_none());
    for p in [
        option(0x5015, &[101, 0]),
        option(0x3013, &[3]),
        option(0x501b, &[0]),
        option(0x9016, &[255, 255]),
    ] {
        assert!(resolve(&[p]).is_err());
    }
}
