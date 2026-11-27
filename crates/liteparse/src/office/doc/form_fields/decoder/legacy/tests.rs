use super::*;

fn payload(state: u16, default: u16) -> Vec<u8> {
    let mut b = (1 | state << 2).to_le_bytes().to_vec();
    b.extend([0, 0, 20, 0, 0, 0, 4, b'N', b'a', b'm', b'e', 0]);
    b.extend(default.to_le_bytes());
    b.extend([0; 10]);
    b
}

#[test]
fn current_state_is_distinct_from_default_and_undefined() {
    for (state, default, expected) in [(0, 1, false), (1, 0, true), (25, 1, false)] {
        let b = payload(state, default);
        let out = decode(&b).unwrap();
        assert_eq!(out.raw_current_state, state as u8);
        assert_eq!(out.default_state, Some(default));
        assert_eq!(out.effective_checked, Some(expected));
        assert_eq!(out.name, "Name");
        assert_eq!(out.legacy_encoding.unwrap().payload_bytes, b.len());
    }
}

#[test]
fn full_record_framing_rejects_truncation_and_trailing_data() {
    let b = payload(25, 0);
    for end in 0..b.len() {
        assert!(decode(&b[..end]).is_err());
    }
    let mut extra = b.clone();
    extra.push(0);
    assert!(matches!(
        decode(&extra),
        Err(Error::Unsupported("versionless checkbox trailing bytes"))
    ));
    for offset in [2, 6, 7, 13, 15, 16, 18, 20, 22, 24] {
        let mut bad = b.clone();
        bad[offset] = 2;
        assert!(decode(&bad).is_err(), "offset {offset}");
    }
}

#[test]
fn unsupported_flags_types_states_and_encodings_are_explicit() {
    for flags in [0u16, 2, 3, 9, 0x81, 0x401, 0x8001] {
        let mut b = payload(25, 0);
        b[..2].copy_from_slice(&flags.to_le_bytes());
        assert!(decode(&b).is_err());
    }
    let mut b = payload(25, 0);
    b[9] = 0xe9;
    assert!(matches!(
        decode(&b),
        Err(Error::Unsupported("versionless form string code page"))
    ));
    b = payload(25, 0);
    b[8] = 21;
    assert!(matches!(decode(&b), Err(Error::Limit(_))));
    b = payload(25, 0);
    b.splice(16..18, [1, b'X', 0]);
    assert!(matches!(
        decode(&b),
        Err(Error::Unsupported("versionless checkbox auxiliary string"))
    ));
}

#[test]
fn nilpicf_container_and_budget_remain_required() {
    let b = payload(1, 0);
    let mut record = vec![0; 68];
    record[..4].copy_from_slice(&((68 + b.len()) as u32).to_le_bytes());
    record[4..6].copy_from_slice(&68u16.to_le_bytes());
    record.extend(b);
    let (out, used) = super::super::decode(&record, 0, &mut 1024).unwrap();
    assert_eq!(used, record.len());
    assert_eq!(out.effective_checked, Some(true));
    assert!(super::super::decode(&record, 0, &mut 1).is_err());
    record[4] = 67;
    assert!(super::super::decode(&record, 0, &mut 1024).is_err());
}
