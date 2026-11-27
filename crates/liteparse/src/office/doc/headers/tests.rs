use super::*;

fn sections(n: usize) -> Vec<HeaderSection> {
    (0..n)
        .map(|index| HeaderSection {
            index,
            plc_end_cp: index * 2 + 2,
            source: SourceRange {
                start_cp: index * 2,
                end_cp: index * 2 + 2,
            },
            sepx_offset: None,
            properties: Vec::new(),
            different_first_page: Some(false),
            legacy_presence_flags: None,
            references: Vec::new(),
            unresolved: Vec::new(),
        })
        .collect()
}
fn source(parts: &[&[u16]]) -> (Vec<u16>, Vec<u8>) {
    let mut units = Vec::new();
    let mut cps = vec![0u32];
    for part in parts {
        units.extend_from_slice(part);
        cps.push(units.len() as u32);
    }
    units.push(13);
    cps.push(u32::MAX);
    (units, cps.into_iter().flat_map(u32::to_le_bytes).collect())
}
#[test]
fn local_inherited_and_explicit_blank_keep_distinct_ownership() {
    let mut parts = vec![&[][..]; 24];
    parts[0] = &[2, 13];
    parts[7] = &[65, 13, 13];
    parts[19] = &[13, 13];
    let (units, plc) = source(&parts);
    let out = decode(0x101, 100, &units, &plc, sections(3), Some(1)).unwrap();
    assert_eq!(out.sections[0].references[1].status, "local");
    assert_eq!(out.sections[1].references[1].status, "inherited");
    assert_eq!(out.sections[1].references[1].origin_story_index, Some(7));
    assert_eq!(out.sections[2].references[1].status, "local");
    assert!(out.stories[19].explicit_blank);
    assert_eq!(
        out.stories[7].content,
        Some(SourceRange {
            start_cp: 102,
            end_cp: 104
        })
    );
    assert_eq!(
        out.stories[7].guard,
        Some(SourceRange {
            start_cp: 104,
            end_cp: 105
        })
    );
    assert_eq!(out.ignored_final_plc_cp, Some(u32::MAX));
    assert_eq!(out.different_even_and_odd_pages, Some(true));
    assert_eq!(out.sections[0].references[0].status, "absent");
}
#[test]
fn no_header_text_does_not_invent_stale_plc_content() {
    let out = decode(0xc1, 12, &[], &[0; 52], sections(1), Some(0)).unwrap();
    assert!(out.stories.is_empty());
    assert!(
        out.diagnostics
            .contains(&"stale_header_plc_without_header_story")
    );
    assert!(
        out.sections[0]
            .references
            .iter()
            .all(|r| r.status == "absent")
    );
    assert!(out.terminal_sentinel.is_none());
}
#[test]
fn extra_slots_preserve_ranges_without_assigning_section_roles() {
    let mut parts = vec![&[][..]; 13];
    parts[12] = &[65, 13, 13];
    let (units, plc) = source(&parts);
    let out = decode(0x101, 0, &units, &plc, sections(1), Some(0)).unwrap();
    assert!(out.diagnostics.contains(&"header_slot_count_mismatch"));
    assert_eq!(out.stories[12].role, None);
    assert!(
        out.sections[0]
            .references
            .iter()
            .all(|r| r.status == "unresolved_slot_count")
    );
    assert_eq!(
        out.stories[12].content,
        Some(SourceRange {
            start_cp: 0,
            end_cp: 2
        })
    );
}
#[test]
fn malformed_content_does_not_fall_back_to_previous_section() {
    let mut parts = vec![&[][..]; 24];
    parts[7] = &[65, 13, 13];
    parts[13] = &[66, 13];
    let (units, plc) = source(&parts);
    let out = decode(0x101, 0, &units, &plc, sections(3), Some(0)).unwrap();
    assert_eq!(out.sections[1].references[1].status, "unresolved_content");
    assert_eq!(out.sections[2].references[1].origin_story_index, Some(13));
    assert_eq!(out.sections[2].references[1].status, "unresolved_content");
    assert!(
        out.stories[13]
            .unresolved
            .contains(&"header_content_paragraph_end_missing")
    );
}
#[test]
fn coverage_initial_and_terminal_cps_are_checked_but_final_cp_is_ignored() {
    let (units, mut plc) = source(&[&[][..]; 12]);
    assert!(decode(0xc1, 0, &units, &plc, sections(1), None).is_ok());
    plc[..4].copy_from_slice(&1u32.to_le_bytes());
    assert!(decode(0xc1, 0, &units, &plc, sections(1), None).is_err());
    plc[..4].copy_from_slice(&0u32.to_le_bytes());
    let n = plc.len();
    plc[n - 8..n - 4].copy_from_slice(&1u32.to_le_bytes());
    assert!(decode(0xc1, 0, &units, &plc, sections(1), None).is_err());
}
#[test]
fn mode_properties_have_defaults_order_and_explicit_unknowns() {
    let mut s = sections(1).remove(0);
    properties(&[], &mut s).unwrap();
    assert_eq!(s.different_first_page, Some(false));
    properties(&[0x0a, 0x30, 1, 0x39, 0x32, 1], &mut s).unwrap();
    assert_eq!(s.different_first_page, Some(true));
    properties(&[0x0a, 0x30, 1, 0x0a, 0x30, 0], &mut s).unwrap();
    assert_eq!(s.different_first_page, Some(false));
    properties(&[0x0a, 0x30, 2], &mut s).unwrap();
    assert_eq!(s.different_first_page, None);
    let mut s = sections(1).remove(0);
    properties(&[0xff, 0x30, 0], &mut s).unwrap();
    assert_eq!(s.different_first_page, None);
    assert!(properties(&[0x0a, 0x30], &mut s).is_err());
}
#[test]
fn metadata_limits_apply_before_allocating_role_arrays() {
    assert!(matches!(
        decode(0xc1, 0, &[], &[], sections(MAX_SECTIONS + 1), None),
        Err(Error::Limit(_))
    ));
    let plc = vec![0; (MAX_SLOTS + 3) * 4];
    assert!(matches!(
        decode(0xc1, 0, &[13], &plc, sections(1), None),
        Err(Error::Limit(_))
    ));
}
#[test]
fn source_identity_checks_each_story_length_and_origin() {
    let data = crate::office::doc::tests::fixture();
    let mut source = crate::office::doc::inspect(&data).unwrap();
    let lengths = std::array::from_fn(|i| source.stories[i].utf16_units);
    validate_source(source.binary_version, &lengths, &source).unwrap();
    source.stories[2].start_cp += 1;
    assert!(validate_source(source.binary_version, &lengths, &source).is_err());
}

#[test]
fn complete_compound_fixture_decodes_roles_and_ignores_undefined_last_cp() {
    use crate::office::doc::tests::{put16, put32};
    let mut data = crate::office::doc::tests::fixture();
    let mut parts = vec![&[][..]; 12];
    parts[9] = &[65, 13, 13];
    let (units, plc) = source(&parts);
    let w = &mut data[1536..5632];
    put32(w, 84, units.len() as u32);
    for (index, value) in units.iter().enumerate() {
        w[2058 + index] = *value as u8;
    }
    for (pair, offset, length) in [(6, 100, 20), (11, 200, plc.len()), (31, 300, 1)] {
        put32(w, 154 + pair * 8, offset as u32);
        put32(w, 158 + pair * 8, length as u32);
    }
    put16(w, 3000, 3);
    w[3002..3005].copy_from_slice(&[0x0a, 0x30, 1]);
    let t = &mut data[5632..9728];
    put32(t, 9, (10 + units.len()) as u32);
    put32(t, 100, 0);
    put32(t, 104, 999);
    put32(t, 110, 3000);
    t[200..200 + plc.len()].copy_from_slice(&plc);
    t[300] = 1;
    let raw = crate::office::doc::inspect(&data).unwrap();
    let output = inspect_headers(&data, &raw).unwrap();
    assert_eq!(output.sections[0].plc_end_cp, 999);
    assert_eq!(output.sections[0].source.end_cp, 5);
    assert_eq!(output.sections[0].different_first_page, Some(true));
    assert_eq!(output.stories[9].role, Some("odd_footer"));
    assert_eq!(
        output.stories[9].content,
        Some(SourceRange {
            start_cp: 10,
            end_cp: 12
        })
    );
    assert_eq!(output.ignored_final_plc_cp, Some(u32::MAX));
}

#[test]
fn split_surrogates_and_missing_guards_remain_unresolved() {
    let mut parts = vec![&[][..]; 12];
    parts[6] = &[0xd83d];
    parts[7] = &[0xde00, 13, 13];
    let (units, plc) = source(&parts);
    let output = decode(0x101, 0, &units, &plc, sections(1), Some(0)).unwrap();
    assert!(
        output.stories[6]
            .unresolved
            .contains(&"header_guard_missing")
    );
    assert!(
        output.stories[7]
            .unresolved
            .contains(&"header_surrogate_boundary")
    );
    assert_eq!(
        output.sections[0].references[1].status,
        "unresolved_content"
    );
}

fn outline_sprm(sprm: u16, payload: usize) -> Vec<u8> {
    let mut out = sprm.to_le_bytes().to_vec();
    out.push(payload as u8);
    out.resize(payload + 3, 0);
    out
}

#[test]
fn legacy_outline_payloads_preserve_title_page_order_and_source_bytes() {
    for enabled in [0, 1] {
        let mut raw = vec![0x0a, 0x30, 1 - enabled];
        raw.extend(outline_sprm(0xd202, 212));
        raw.extend([0x0a, 0x30, enabled]);
        raw.extend(outline_sprm(0xd238, 36));
        let mut section = sections(1).remove(0);
        properties(&raw, &mut section).unwrap();
        assert_eq!(section.different_first_page, Some(enabled == 1));
        assert!(section.unresolved.is_empty());
        assert_eq!(
            section.properties[1].operand,
            outline_sprm(0xd202, 212)[2..]
        );
        assert_eq!(section.properties[3].operand, outline_sprm(0xd238, 36)[2..]);
    }
}

#[test]
fn unsupported_outline_sizes_and_unknown_modifiers_keep_mode_unresolved() {
    for (sprm, sizes) in [(0xd202, vec![0, 211, 213, 248]), (0xd238, vec![0, 35, 37])] {
        for size in sizes {
            let mut raw = outline_sprm(sprm, size);
            raw.extend([0x0a, 0x30, 1]);
            let mut section = sections(1).remove(0);
            properties(&raw, &mut section).unwrap();
            assert_eq!(section.different_first_page, None);
            assert!(
                section
                    .unresolved
                    .contains(&"invalid_outline_section_operand")
            );
        }
    }
    let mut raw = outline_sprm(0xd202, 212);
    raw.extend([0xfe, 0xd1, 2, 3, 0]);
    let mut section = sections(1).remove(0);
    properties(&raw, &mut section).unwrap();
    assert_eq!(section.different_first_page, None);
    assert!(section.unresolved.contains(&"unknown_section_modifier"));
}

#[test]
fn truncated_outline_payloads_are_rejected_before_mode_resolution() {
    for (sprm, size) in [(0xd202, 212), (0xd238, 36)] {
        let mut raw = outline_sprm(sprm, size);
        raw.pop();
        assert!(properties(&raw, &mut sections(1).remove(0)).is_err());
    }
    assert!(!outline_operand(0xd202, &[212]));
    assert!(!outline_operand(0xd238, &[35; 37]));
}

#[test]
fn ignored_section_extension_keeps_title_selection_and_raw_order() {
    for enabled in [0, 1] {
        let raw = [
            0x0a,
            0x30,
            1 - enabled,
            0xff,
            0xd1,
            2,
            3,
            0,
            0x0a,
            0x30,
            enabled,
        ];
        let mut section = sections(1).remove(0);
        properties(&raw, &mut section).unwrap();
        assert_eq!(section.different_first_page, Some(enabled == 1));
        assert!(section.unresolved.is_empty());
        assert_eq!(
            section
                .properties
                .iter()
                .map(|p| p.sprm)
                .collect::<Vec<_>>(),
            [0x300a, 0xd1ff, 0x300a]
        );
        assert_eq!(section.properties[1].operand, [2, 3, 0]);
    }
    let mut section = sections(1).remove(0);
    assert!(properties(&[0xff, 0xd1, 2, 3], &mut section).is_err());
    for raw in [
        &[0xff, 0xd1, 1, 3][..],
        &[0xff, 0xd1, 2, 4, 0][..],
        &[0xfe, 0xd1, 2, 3, 0][..],
    ] {
        let mut section = sections(1).remove(0);
        properties(raw, &mut section).unwrap();
        assert_eq!(section.different_first_page, None);
        assert_eq!(section.unresolved, ["unknown_section_modifier"]);
    }
}
