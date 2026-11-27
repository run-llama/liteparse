use super::*;

fn plc(cp: u32, end: u32, id: u32) -> Vec<u8> {
    let mut data = cp.to_le_bytes().to_vec();
    data.extend(end.to_le_bytes());
    data.extend(id.to_le_bytes());
    for coordinate in [0i32, -540, 8820, 1050] {
        data.extend(coordinate.to_le_bytes());
    }
    data.extend(0x74u16.to_le_bytes());
    data.extend(0u32.to_le_bytes());
    data
}

#[test]
fn plc_retains_signed_frame_and_exact_source_cp() {
    let result = anchors(&plc(7, 20, 3), 20).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].shape_id, 3);
    assert_eq!(result[0].cp, 7);
    assert_eq!(result[0].rectangle, [0, -540, 8820, 1050]);
}

#[test]
fn malformed_partition_and_out_of_story_anchor_are_rejected() {
    let data = plc(7, 20, 3);
    for length in 1..data.len() {
        if length != 4 {
            assert!(anchors(&data[..length], 20).is_err(), "{length}");
        }
    }
    assert!(anchors(&plc(20, 21, 3), 20).is_err());
    assert!(anchors(&plc(7, 7, 3), 20).is_err());
}

#[test]
fn duplicate_shape_ids_and_nonincreasing_cps_do_not_form_ownership() {
    for (second_cp, second_id) in [(8u32, 3u32), (7, 4), (6, 4)] {
        let mut data = 7u32.to_le_bytes().to_vec();
        data.extend(second_cp.to_le_bytes());
        data.extend(20u32.to_le_bytes());
        data.extend(&plc(7, 20, 3)[8..]);
        data.extend(&plc(second_cp, 20, second_id)[8..]);
        assert!(anchors(&data, 20).is_err());
    }
}
