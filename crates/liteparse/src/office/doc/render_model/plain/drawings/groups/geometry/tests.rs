use super::*;

#[test]
fn translated_scaled_group_retains_fractional_child_coordinates() {
    let transform = Transform {
        group_shape_id: 7,
        source_bounds: [100, 200, 400, 600],
        parent_frame_twips: [20.0, 40.0, 220.0, 240.0],
        table_range: [0, 24],
    };
    let placed = transform.apply([175, 250, 325, 550]).unwrap();
    assert_eq!(placed, [70.0, 65.0, 170.0, 215.0]);
    assert!((transform.apply([101, 200, 400, 600]).unwrap()[0] - 20.666666666666668).abs() < 1e-10);
    let nested = Transform {
        group_shape_id: 8,
        source_bounds: [0, 0, 10, 10],
        parent_frame_twips: placed,
        table_range: [24, 48],
    };
    assert_eq!(
        nested.apply([2, 2, 8, 8]).unwrap(),
        [90.0, 95.0, 150.0, 185.0]
    );
}

#[test]
fn degenerate_or_unbounded_group_transforms_are_rejected() {
    let mut transform = Transform {
        group_shape_id: 7,
        source_bounds: [0, 0, 0, 10],
        parent_frame_twips: [0.0, 0.0, 10.0, 10.0],
        table_range: [0, 24],
    };
    assert!(transform.apply([0, 0, 1, 1]).is_err());
    transform.source_bounds = [0, 0, 1, 1];
    assert!(transform.apply([0, 0, i32::MAX, i32::MAX]).is_err());
    transform.parent_frame_twips = [f64::NAN, 0.0, 10.0, 10.0];
    assert!(transform.apply([0, 0, 1, 1]).is_err());
}
