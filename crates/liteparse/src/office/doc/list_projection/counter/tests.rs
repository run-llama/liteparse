use super::*;

#[test]
fn first_override_precedes_its_label_and_switchback_keeps_shared_sequence() {
    let mut c = Counters::default();
    let values: Vec<_> = [
        (24, None),
        (24, None),
        (29, Some(1)),
        (24, None),
        (29, Some(1)),
    ]
    .into_iter()
    .enumerate()
    .map(|(cp, (ilfo, start))| c.advance(1, ilfo, 0, cp, 1, 0, start).unwrap())
    .collect();
    assert_eq!(values, [1, 2, 1, 2, 3]);
}

#[test]
fn effective_current_level_controls_initial_and_lower_restart() {
    let mut c = Counters::default();
    assert_eq!(c.advance(1, 1, 1, 0, 2, 1, None).unwrap(), 2);
    assert_eq!(c.advance(1, 2, 1, 1, 8, 1, None).unwrap(), 9);
    assert_eq!(c.advance(1, 1, 0, 2, 1, 0, None).unwrap(), 1);
    assert_eq!(c.advance(1, 2, 1, 3, 8, 1, None).unwrap(), 8);
    assert_eq!(c.advance(1, 1, 1, 4, 2, 0, None).unwrap(), 5);
}

#[test]
fn first_use_is_level_specific_and_lower_restart_supersedes_old_override() {
    let mut c = Counters::default();
    assert_eq!(c.advance(1, 3, 0, 0, 1, 0, Some(5)).unwrap(), 5);
    assert_eq!(c.advance(1, 3, 1, 1, 1, 1, Some(7)).unwrap(), 7);
    assert_eq!(c.advance(1, 3, 1, 2, 1, 1, Some(7)).unwrap(), 8);
    assert_eq!(c.advance(1, 3, 0, 3, 1, 0, Some(5)).unwrap(), 6);
    assert_eq!(c.advance(1, 3, 1, 4, 1, 1, Some(7)).unwrap(), 1);
}

#[test]
fn zero_and_separate_families_preserve_values_and_validate_bounds() {
    let mut c = Counters::default();
    assert_eq!(c.advance(1, 1, 0, 0, 1, 0, Some(0)).unwrap(), 0);
    assert_eq!(c.advance(2, 1, 0, 0, 8, 0, None).unwrap(), 8);
    assert_eq!(c.advance(1, 1, 0, 1, 1, 0, Some(0)).unwrap(), 1);
    assert!(c.advance(1, 1, 0, 1, 1, 0, None).is_err());
    assert!(c.advance(1, 1, 0, 2, 1, 1, None).is_err());
    assert!(c.advance(1, 1, 9, 2, 1, 0, None).is_err());
    assert!(c.advance(1, 1, 0, 2, 1, 0, Some(32768)).is_err());
    c.remaining_work = 0;
    assert!(matches!(
        c.advance(2, 1, 0, 1, 1, 0, None),
        Err(Error::Limit("list counter history"))
    ));
}
