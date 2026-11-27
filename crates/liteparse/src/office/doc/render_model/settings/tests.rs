use super::*;

fn input(length: usize) -> (Vec<u8>, Vec<u8>) {
    let mut word = vec![0; 32];
    word[6..8].copy_from_slice(&0x409u16.to_le_bytes());
    let mut dop = vec![0; length];
    dop[10..12].copy_from_slice(&720u16.to_le_bytes());
    (word, dop)
}
fn named<'a>(settings: &'a DocumentSettings, name: &str) -> &'a CompatibilityOption {
    settings
        .compatibility
        .iter()
        .find(|p| p.name == name)
        .unwrap()
}

#[test]
fn dop_versions_preserve_feature_masks_without_inventing_layout_mode() {
    for (version, extra, length, schema) in [
        (0xc1, 0, 500, "Dop97"),
        (0xd9, 2, 544, "Dop2000"),
        (0x101, 2, 594, "Dop2002"),
        (0x10c, 2, 616, "Dop2003"),
        (0x112, 5, 674, "Dop2007"),
        (0x112, 5, 690, "Dop2010"),
        (0x112, 5, 694, "Dop2013"),
    ] {
        let (word, mut dop) = input(length);
        if length >= 544 {
            dop[540..542].copy_from_slice(&8u16.to_le_bytes());
        }
        if length >= 594 {
            dop[552..554].copy_from_slice(&0x800u16.to_le_bytes());
        }
        let settings = decode(version, extra, &word, &dop).unwrap();
        assert_eq!(settings.dop_schema, schema);
        assert_eq!(
            settings.feature_restrictions,
            if length >= 594 {
                Some(0x800)
            } else if length >= 544 {
                Some(8)
            } else {
                None
            }
        );
        assert!(settings.issues.is_empty());
        assert_eq!(settings.raw_dop, dop);
    }
}

#[test]
fn compatibility_duplicates_require_only_defined_bits_to_agree() {
    let (word, mut dop) = input(544);
    dop[9] = 2;
    dop[85] = 0;
    assert!(decode(0xd9, 2, &word, &dop).unwrap().issues.is_empty());
    dop[8] = 1;
    assert_eq!(
        decode(0xd9, 2, &word, &dop).unwrap().issues,
        ["copts60_copies_disagree"]
    );
    dop[84] = 1;
    assert_eq!(
        decode(0xd9, 2, &word, &dop).unwrap().issues,
        ["copts80_copies_disagree"]
    );
    dop[508] = 1;
    let settings = decode(0xd9, 2, &word, &dop).unwrap();
    assert!(settings.issues.is_empty());
    assert!(named(&settings, "no_tab_for_hanging_indent").enabled);
}

#[test]
fn source_bit_offsets_and_option_polarity_are_explicit() {
    let (word, mut dop) = input(544);
    dop[9] = 0x20;
    dop[85] = 0x20;
    dop[509] = 0x20;
    dop[512] = 4 | 8;
    dop[514] = 2;
    dop[516] = 1;
    let settings = decode(0xd9, 2, &word, &dop).unwrap();
    for (name, offset, bit) in [
        ("expand_shift_return", 9, 5),
        ("do_not_use_html_auto_spacing", 512, 2),
        ("do_not_adjust_line_height_in_table", 512, 3),
        ("grow_autofit", 514, 1),
        ("cached_column_balance", 516, 0),
    ] {
        let p = named(&settings, name);
        assert!(p.enabled);
        assert_eq!((p.dop_byte, p.bit), (offset, bit));
    }
    assert!(!named(&settings, "printer_metrics").enabled);
}

#[test]
fn schema_discrepancy_preserves_prefix_but_does_not_interpret_extensions() {
    let (word, mut dop) = input(690);
    dop[512] = 0xff;
    let settings = decode(0x10c, 2, &word, &dop).unwrap();
    assert_eq!(settings.issues, ["dop_version_length_mismatch"]);
    assert_eq!(settings.feature_restrictions, None);
    assert_eq!(settings.compatibility.len(), 31);
    assert_eq!(settings.raw_dop, dop);
}

#[test]
fn load_overrides_and_template_updates_remain_source_visible() {
    let (mut word, mut dop) = input(500);
    word[11] = 0x20;
    word[19] = 4;
    dop[5] = 0x40;
    let settings = decode(0xc1, 0, &word, &dop).unwrap();
    assert!(settings.override_normal_font_and_language);
    assert!(settings.override_section_geometry);
    assert!(settings.update_styles_from_template);
    assert_eq!(
        settings.issues,
        [
            "normal_font_language_installation_override",
            "section_geometry_installation_override"
        ]
    );
}

#[test]
fn ignored_historical_view_bits_do_not_set_a_rendering_policy() {
    let (word, mut dop) = input(500);
    dop[6] = 0x0c;
    let settings = decode(0xc1, 0, &word, &dop).unwrap();
    assert!(settings.issues.is_empty());
    assert!(!settings.saved_show_revision_markup);
    let value = serde_json::to_value(&settings).unwrap();
    assert!(value.get("saved_show_hidden").is_none());
    assert!(value.get("saved_show_field_results").is_none());
    assert_eq!(settings.raw_dop[6], 0x0c);
}

#[test]
fn zero_tabs_truncation_and_bounded_settings_are_explicit() {
    let (word, mut dop) = input(500);
    dop[10..12].fill(0);
    assert_eq!(
        decode(0xc1, 0, &word, &dop).unwrap().issues,
        ["zero_default_tab_interval"]
    );
    assert!(decode(0xc1, 0, &word, &dop[..499]).is_err());
    assert!(decode(0xc1, 0, &word, &vec![0; 4097]).is_err());
}
