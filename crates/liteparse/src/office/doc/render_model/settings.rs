//! Document layout inputs from FIB and DOP, before content-dependent qualification.
//! DOP feature restrictions and compatibility option bits have separate meanings.
//! <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/841b5f72-487e-4fe7-8657-ec90d5af8750>
use crate::office::doc::{Error, Result, bytes, compound::Compound, document::fib, u16_at, u32_at};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompatibilityOption {
    pub name: &'static str,
    pub enabled: bool,
    /// Byte and bit within the DOP stream range, including inherited option copies.
    pub dop_byte: usize,
    pub bit: u8,
}
#[derive(Debug, Serialize)]
pub struct DocumentSettings {
    pub binary_version: u16,
    pub dop_schema: &'static str,
    pub raw_dop: Vec<u8>,
    /// FibBase.lid can be normalized to 0x0409 by Word. This recorded value does
    /// not establish the original installation language or installation defaults.
    pub recorded_install_language: u16,
    pub override_normal_font_and_language: bool,
    pub override_section_geometry: bool,
    pub default_tab_twips: u16,
    pub mirror_margins: bool,
    pub gutter_at_top: bool,
    pub different_even_and_odd_headers: bool,
    pub automatic_hyphenation: bool,
    pub hyphenate_capitals: bool,
    pub hyphenation_zone_twips: u16,
    pub consecutive_hyphen_limit: u16,
    pub update_styles_from_template: bool,
    pub saved_word97_compatibility: bool,
    pub saved_show_revision_markup: bool,
    pub saved_print_revision_markup: bool,
    pub two_pages_per_sheet: bool,
    pub feature_restrictions: Option<u16>,
    pub feature_restrictions_dop_byte: Option<usize>,
    pub compatibility: Vec<CompatibilityOption>,
    pub issues: Vec<&'static str>,
}

/// Reads a bounded source settings record without choosing a rendering view or layout mode.
/// A future model qualifier must assess every relevant option and recorded issue against
/// the actual document content; this result alone establishes no rendering eligibility.
pub fn inspect_settings(data: &[u8]) -> Result<DocumentSettings> {
    let compound = Compound::open(data)?;
    let word = compound.stream("WordDocument")?;
    // fib() validates csw=14, cslw=22 and the complete pair array (at least 93).
    // These checks establish the fixed FIB offsets used for the DOP reference.
    let (version, table_name, _, _, _) = fib(&word)?;
    let table = compound.stream(table_name)?;
    let pairs = usize::from(u16_at(&word, 152)?);
    let csw_new = u16_at(&word, 154 + pairs * 8)?;
    let offset = u32_at(&word, 154 + 31 * 8)? as usize;
    let length = u32_at(&word, 158 + 31 * 8)? as usize;
    decode(version, csw_new, &word, bytes(&table, offset, length)?)
}

fn decode(version: u16, csw_new: u16, word: &[u8], dop: &[u8]) -> Result<DocumentSettings> {
    if dop.len() > 4096 {
        return Err(Error::Limit("render document settings"));
    }
    bytes(dop, 0, 500)?;
    let (schema, length) = if csw_new == 0 {
        ("Dop97", 500)
    } else {
        match version {
            0xd9 => ("Dop2000", 544),
            0x101 => ("Dop2002", 594),
            0x10c => ("Dop2003", 616),
            0x112 => match dop.len() {
                674 => ("Dop2007", 674),
                690 => ("Dop2010", 690),
                694 => ("Dop2013", 694),
                _ => ("unknown_Dop2007_or_later", 0),
            },
            _ => return Err(Error::Unsupported("render DOP schema version")),
        }
    };
    let mut issues = Vec::new();
    if dop.len() != length {
        issues.push("dop_version_length_mismatch");
    }
    if csw_new == 0 && version != 0xc1 {
        issues.push("dop97_noncanonical_fib_version");
    }
    // The source repeats Copts60/Copts80. Only defined bits must agree; ignored bits
    // are retained in raw_dop without acquiring compatibility semantics.
    let old = u16_at(dop, 8)?;
    let copied = u16_at(dop, 84)?;
    if (old ^ copied) & 0xfdff != 0 {
        issues.push("copts60_copies_disagree");
    }
    let mut compatibility = Vec::new();
    for (bit, name) in COPTS60.iter().enumerate() {
        if let Some(name) = name {
            option(&mut compatibility, name, old as u32, bit, 8);
        }
    }
    let high = u16_at(dop, 86)?;
    for (bit, name) in COPTS80.iter().enumerate() {
        option(&mut compatibility, name, high as u32, bit, 86);
    }
    // A mismatched schema keeps its common prefix, but extension fields are not
    // interpreted merely because a longer byte buffer happens to contain them.
    let extended = csw_new != 0 && length != 0 && dop.len() == length;
    if extended {
        let duplicate = u32_at(dop, 508)?;
        if (duplicate ^ u32_at(dop, 84)?) & 0xffff_fdff != 0 {
            issues.push("copts80_copies_disagree");
        }
        let flags = u32_at(dop, 512)?;
        for (bit, name) in COPTS.iter().enumerate() {
            option(&mut compatibility, name, flags, bit, 512);
        }
        option(
            &mut compatibility,
            "cached_column_balance",
            u32_at(dop, 516)?,
            0,
            516,
        );
    }
    let feature_offset = if extended {
        Some(if version >= 0x101 { 552 } else { 540 })
    } else {
        None
    };
    let feature_restrictions = feature_offset.map(|off| u16_at(dop, off)).transpose()?;
    let fib_flags = u16_at(word, 10)?;
    let page_flags = bytes(word, 19, 1)?[0];
    if fib_flags & 0x2000 != 0 {
        issues.push("normal_font_language_installation_override");
    }
    if page_flags & 4 != 0 {
        issues.push("section_geometry_installation_override");
    }
    let source_flags = u16_at(dop, 4)?;
    let view_flags = u16_at(dop, 6)?;
    let default_tab_twips = u16_at(dop, 10)?;
    if default_tab_twips == 0 {
        issues.push("zero_default_tab_interval");
    }
    Ok(DocumentSettings {
        binary_version: version,
        dop_schema: schema,
        raw_dop: dop.to_vec(),
        recorded_install_language: u16_at(word, 6)?,
        override_normal_font_and_language: fib_flags & 0x2000 != 0,
        override_section_geometry: page_flags & 4 != 0,
        default_tab_twips,
        mirror_margins: view_flags & 0x20 != 0,
        gutter_at_top: u16_at(dop, 82)? & 0x8000 != 0,
        different_even_and_odd_headers: dop[0] & 1 != 0,
        automatic_hyphenation: source_flags & 0x1000 != 0,
        hyphenate_capitals: source_flags & 0x800 != 0,
        hyphenation_zone_twips: u16_at(dop, 14)?,
        consecutive_hyphen_limit: u16_at(dop, 16)?,
        update_styles_from_template: source_flags & 0x4000 != 0,
        saved_word97_compatibility: view_flags & 0x40 != 0,
        saved_show_revision_markup: view_flags & 0x800 != 0,
        saved_print_revision_markup: view_flags & 0x1000 != 0,
        two_pages_per_sheet: u16_at(dop, 90)? & 0x20 != 0,
        feature_restrictions,
        feature_restrictions_dop_byte: feature_offset,
        compatibility,
        issues,
    })
}

fn option(
    out: &mut Vec<CompatibilityOption>,
    name: &'static str,
    flags: u32,
    bit: usize,
    offset: usize,
) {
    out.push(CompatibilityOption {
        name,
        enabled: flags & (1 << bit) != 0,
        dop_byte: offset + bit / 8,
        bit: (bit % 8) as u8,
    });
}

// MS-DOC §§2.7.11–2.7.13 specify the source bit order. These names preserve
// option meaning for content-dependent qualification rather than encode a mode number.
const COPTS60: [Option<&str>; 16] = [
    Some("no_tab_for_hanging_indent"),
    Some("no_space_raise_lower"),
    Some("suppress_spacing_after_page_break"),
    Some("wrap_trailing_spaces"),
    Some("print_colors_black"),
    Some("no_column_balance"),
    Some("convert_mail_merge_escape"),
    Some("suppress_top_spacing"),
    Some("original_word_table_rules"),
    None,
    Some("show_breaks_in_frames"),
    Some("swap_borders_facing_pages"),
    Some("leave_backslash_alone"),
    Some("expand_shift_return"),
    Some("do_not_underline_trailing_spaces"),
    Some("do_not_balance_single_double_byte_width"),
];
const COPTS80: [&str; 16] = [
    "suppress_top_spacing_mac5",
    "spacing_in_whole_points",
    "print_body_before_header",
    "no_external_leading",
    "do_not_make_space_for_underline",
    "mac_small_caps",
    "two_point_external_leading_only",
    "truncate_font_height",
    "font_substitution_by_size",
    "line_wrap_like_word6",
    "word6_border_rules",
    "exact_line_height_on_top",
    "suppress_bottom_spacing",
    "wordperfect_space_width",
    "wordperfect_justification",
    "printer_metrics",
];
const COPTS: [&str; 32] = [
    "shape_layout_like_word97",
    "footnote_layout_like_word97",
    "do_not_use_html_auto_spacing",
    "do_not_adjust_line_height_in_table",
    "forget_last_tab_alignment",
    "autospace_fullwidth_alpha",
    "align_table_rows_independently",
    "raw_table_width",
    "table_rows_apart",
    "word97_line_breaking",
    "do_not_break_wrapped_tables",
    "do_not_snap_to_grid_in_cell",
    "do_not_allow_field_end_selection",
    "ethiopic_amharic_breaking_rules",
    "do_not_wrap_punctuation",
    "do_not_use_asian_break_rules",
    "word2002_table_style_rules",
    "grow_autofit",
    "normal_style_for_list",
    "ignore_numbering_indent_tab",
    "alternate_east_asian_line_breaking",
    "same_style_spacing_in_table",
    "word11_indent_rules",
    "do_not_autofit_constrained_tables",
    "autofit_like_word11",
    "underline_numbering_tab",
    "hangul_width_like_word11",
    "split_page_break_and_paragraph_mark",
    "do_not_vertically_align_cell_with_shapes",
    "do_not_break_constrained_forced_tables",
    "do_not_vertically_align_textbox",
    "word11_kerning_pairs",
];

// The closed source-name set keeps unknown caller-supplied options rejected.
pub(crate) fn known_compatibility_name(name: &str) -> bool {
    name == "cached_column_balance"
        || COPTS60
            .iter()
            .flatten()
            .chain(COPTS80.iter())
            .chain(COPTS.iter())
            .any(|known| *known == name)
}

#[cfg(test)]
mod tests;
