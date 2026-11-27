use super::*;

#[test]
fn flags_signed_author_and_raw_timestamp_are_preserved() {
    for flag in [0, 1] {
        let operand = [7, flag, 0xfe, 0xff, 0x12, 0x34, 0x56, 0xff];
        let value = decode_operand(&operand).unwrap();
        assert_eq!(value.revision_present, flag == 1);
        assert_eq!(value.author_index, -2);
        assert_eq!(value.timestamp_raw, 0xff563412);
        assert_eq!(value.raw_proprmark, operand[1..]);
    }
}

#[test]
fn malformed_operands_are_rejected() {
    for len in 0..12 {
        let mut operand = vec![0; len];
        if len > 0 {
            operand[0] = 7;
        }
        assert_eq!(decode_operand(&operand).is_ok(), len == 8);
    }
    assert!(decode_operand(&[6, 0, 0, 0, 0, 0, 0, 0]).is_err());
    assert!(decode_operand(&[7, 2, 0, 0, 0, 0, 0, 0]).is_err());
}
