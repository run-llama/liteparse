//! Ordered section geometry from source SEPX; distances remain in twips.
//! MS-DOC §2.6.4 requires explicit margins and gives fixed page-size defaults.
//! <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/46c3ec54-53ff-4c0a-b0d6-07ad15d2546e>
use super::{PropertyIssue, Value};
use crate::office::doc::{Error, Result, headers::HeaderInspection};
use serde::Serialize;

const MAX_SECTIONS: usize = 8192;
const MAX_MODIFIERS: usize = 262_144;
const MAX_OPERAND_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SectionGeometry {
    pub section_index: usize,
    pub source_cp: [usize; 2],
    pub page_width_twips: Value<u16>,
    pub page_height_twips: Value<u16>,
    pub landscape: Value<bool>,
    pub left_margin_twips: Option<Value<u16>>,
    pub right_margin_twips: Option<Value<u16>>,
    /// Negative source margins fix their absolute distance; positive margins can grow.
    pub top_margin_twips: Option<Value<i16>>,
    pub bottom_margin_twips: Option<Value<i16>>,
    pub gutter_twips: Value<u16>,
    pub header_distance_twips: Option<Value<u16>>,
    pub footer_distance_twips: Option<Value<u16>>,
    pub section_break: Value<u8>,
    pub column_count: Value<u16>,
    pub evenly_spaced_columns: Value<bool>,
    pub column_spacing_twips: Option<Value<u16>>,
    pub column_rule: Value<bool>,
    pub vertical_justification: Value<u8>,
    pub different_first_page: Value<bool>,
    pub grid_mode: Value<u16>,
    pub grid_line_pitch_twips: Option<Value<u16>>,
    /// Difference from Normal font pitch in 1/4096 point units.
    pub grid_character_spacing: Value<i32>,
    pub issues: Vec<PropertyIssue>,
}

/// Resolves known geometry while retaining every unsupported source operator as an issue.
/// This does not decide compatibility mode, font layout, or whole-document eligibility.
/// Installation-dependent defaults stay absent; missing required margins block geometry.
pub fn resolve(source: &HeaderInspection, body_end_cp: usize) -> Result<Vec<SectionGeometry>> {
    if !matches!(source.binary_version, 0xc1 | 0xd9 | 0x101 | 0x10c | 0x112) {
        return Err(Error::Unsupported("render section binary version"));
    }
    if source.sections.len() > MAX_SECTIONS || body_end_cp > 16 * 1024 * 1024 {
        return Err(Error::Limit("render sections"));
    }
    let mut count = 0usize;
    let mut bytes = 0usize;
    let mut cp = 0;
    let mut output = Vec::with_capacity(source.sections.len());
    for (index, section) in source.sections.iter().enumerate() {
        if section.index != index
            || section.source.start_cp != cp
            || section.source.end_cp <= cp
            || section.source.end_cp > body_end_cp
        {
            return Err(Error::Invalid("render section ownership"));
        }
        cp = section.source.end_cp;
        count = count
            .checked_add(section.properties.len())
            .filter(|n| *n <= MAX_MODIFIERS)
            .ok_or(Error::Limit("render section modifiers"))?;
        let mut geometry = SectionGeometry {
            section_index: index,
            source_cp: [section.source.start_cp, section.source.end_cp],
            page_width_twips: Value::defaulted(12240),
            page_height_twips: Value::defaulted(15840),
            landscape: Value::defaulted(false),
            left_margin_twips: None,
            right_margin_twips: None,
            top_margin_twips: None,
            bottom_margin_twips: None,
            gutter_twips: Value::defaulted(0),
            header_distance_twips: None,
            footer_distance_twips: None,
            section_break: Value::defaulted(2),
            column_count: Value::defaulted(1),
            evenly_spaced_columns: Value::defaulted(true),
            column_spacing_twips: None,
            column_rule: Value::defaulted(false),
            vertical_justification: Value::defaulted(0),
            different_first_page: Value::defaulted(false),
            grid_mode: Value::defaulted(0),
            grid_line_pitch_twips: None,
            grid_character_spacing: Value::defaulted(0),
            issues: Vec::new(),
        };
        if section.sepx_offset.is_none() {
            geometry.issue(None, None, "section_properties_absent");
        }
        for (i, property) in section.properties.iter().enumerate() {
            bytes = bytes
                .checked_add(property.operand.len())
                .filter(|n| *n <= MAX_OPERAND_BYTES)
                .ok_or(Error::Limit("render section operand bytes"))?;
            geometry.apply(i, property.sprm, &property.operand)?;
        }
        geometry.qualify();
        output.push(geometry);
    }
    if cp != body_end_cp {
        return Err(Error::Invalid("render section coverage"));
    }
    Ok(output)
}

impl SectionGeometry {
    fn issue(&mut self, modifier_index: Option<usize>, sprm: Option<u16>, reason: &'static str) {
        self.issues.push(PropertyIssue {
            modifier_index,
            sprm,
            reason,
        });
    }

    fn apply(&mut self, i: usize, sprm: u16, operand: &[u8]) -> Result<()> {
        let word = || -> Result<u16> {
            let b: [u8; 2] = operand
                .try_into()
                .map_err(|_| Error::Invalid("render section word operand"))?;
            Ok(u16::from_le_bytes(b))
        };
        let byte = || -> Result<u8> {
            match operand {
                [value] => Ok(*value),
                _ => Err(Error::Invalid("render section byte operand")),
            }
        };
        let boolean = || -> Result<bool> {
            match byte()? {
                0 => Ok(false),
                1 => Ok(true),
                _ => Err(Error::Invalid("render section Boolean")),
            }
        };
        match sprm {
            0xd1ff if crate::office::doc::formatting::section::ignored_d1ff(sprm, operand) => {}
            0xb01f | 0xb020 => {
                let n = word()?;
                if !(144..=31680).contains(&n) {
                    return Err(Error::Invalid("render page dimension"));
                }
                let field = if sprm == 0xb01f {
                    &mut self.page_width_twips
                } else {
                    &mut self.page_height_twips
                };
                *field = Value::explicit(n, i);
            }
            0xb021 | 0xb022 | 0xb017 | 0xb018 | 0x900c => {
                let n = word()?;
                if n > 31680 {
                    return Err(Error::Invalid("render section nonnegative distance"));
                }
                let field = match sprm {
                    0xb021 => &mut self.left_margin_twips,
                    0xb022 => &mut self.right_margin_twips,
                    0xb017 => &mut self.header_distance_twips,
                    0xb018 => &mut self.footer_distance_twips,
                    _ => &mut self.column_spacing_twips,
                };
                *field = Some(Value::explicit(n, i));
            }
            0x9023 | 0x9024 => {
                let n = word()? as i16;
                if !(-31665..=31665).contains(&n) {
                    return Err(Error::Invalid("render vertical margin"));
                }
                let field = if sprm == 0x9023 {
                    &mut self.top_margin_twips
                } else {
                    &mut self.bottom_margin_twips
                };
                *field = Some(Value::explicit(n, i));
            }
            0x9031 => {
                let n = word()?;
                if !(1..=31680).contains(&n) {
                    return Err(Error::Invalid("render grid line pitch"));
                }
                self.grid_line_pitch_twips = Some(Value::explicit(n, i));
            }
            0x5032 => {
                let n = word()?;
                if n > 3 {
                    return Err(Error::Invalid("render grid mode"));
                }
                self.grid_mode = Value::explicit(n, i);
            }
            0x7030 => {
                let raw: [u8; 4] = operand
                    .try_into()
                    .map_err(|_| Error::Invalid("render grid character operand"))?;
                let n = i32::from_le_bytes(raw);
                if !(-670925..=6488064).contains(&n) {
                    return Err(Error::Invalid("render grid character pitch"));
                }
                self.grid_character_spacing = Value::explicit(n, i);
            }
            0xb025 => self.gutter_twips = Value::explicit(word()?, i),
            // SBOrientationOperand is the printer orientation enumeration, not Bool8.
            // https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/f35a0396-e4d0-4d8e-8b7f-6114d01b864a
            0x301d => {
                let landscape = match byte()? {
                    1 => false,
                    2 => true,
                    _ => return Err(Error::Invalid("render section orientation")),
                };
                self.landscape = Value::explicit(landscape, i);
            }
            0x3005 => self.evenly_spaced_columns = Value::explicit(boolean()?, i),
            0x300a => self.different_first_page = Value::explicit(boolean()?, i),
            0x3019 => self.column_rule = Value::explicit(boolean()?, i),
            0x3009 => {
                let n = byte()?;
                if n > 4 {
                    return Err(Error::Invalid("render section break"));
                }
                self.section_break = Value::explicit(n, i);
            }
            0x301a => {
                let n = byte()?;
                if n > 3 {
                    return Err(Error::Invalid("render vertical justification"));
                }
                self.vertical_justification = Value::explicit(n, i);
            }
            0x500b => {
                let n = word()?;
                if n > 43 {
                    return Err(Error::Invalid("render column count"));
                }
                self.column_count = Value::explicit(n + 1, i);
            }
            // Note numbering is resolved by the source note-label API. These supported
            // formats do not change section dimensions or page margins.
            0x5040 | 0x5042 => {
                if !matches!(word()?, 0 | 1 | 2 | 9) {
                    self.issue(Some(i), Some(sprm), "note_number_format_unresolved");
                }
            }
            // Printer source and paper matching do not alter the recorded page geometry.
            0x5007 | 0x5008 | 0x5026 => {
                word()?;
            }
            0x3006 => {
                boolean()?;
            }
            0x703a if operand.len() == 4 => {}
            0x3239 => {
                boolean()?;
                self.issue(Some(i), Some(sprm), "section_revision_history_unresolved");
            }
            _ => self.issue(Some(i), Some(sprm), "section_property_unresolved"),
        }
        Ok(())
    }

    fn qualify(&mut self) {
        for (present, sprm, reason) in [
            (
                self.left_margin_twips.is_some(),
                0xb021,
                "left_margin_missing",
            ),
            (
                self.right_margin_twips.is_some(),
                0xb022,
                "right_margin_missing",
            ),
            (
                self.top_margin_twips.is_some(),
                0x9023,
                "top_margin_missing",
            ),
            (
                self.bottom_margin_twips.is_some(),
                0x9024,
                "bottom_margin_missing",
            ),
        ] {
            if !present {
                self.issue(None, Some(sprm), reason);
            }
        }
        if let (Some(left), Some(right)) = (self.left_margin_twips, self.right_margin_twips)
            && u32::from(left.value) + u32::from(right.value) + u32::from(self.gutter_twips.value)
                >= u32::from(self.page_width_twips.value)
        {
            self.issue(None, None, "nonpositive_section_text_width");
        }
        if self.evenly_spaced_columns.value
            && self.column_count.value > 1
            && let (Some(left), Some(right), Some(spacing)) = (
                self.left_margin_twips,
                self.right_margin_twips,
                self.column_spacing_twips,
            )
        {
            let margins =
                u32::from(left.value) + u32::from(right.value) + u32::from(self.gutter_twips.value);
            let gaps = u32::from(self.column_count.value - 1) * u32::from(spacing.value);
            if margins + gaps >= u32::from(self.page_width_twips.value) {
                self.issue(None, None, "nonpositive_section_column_width");
            }
        }
        if self.grid_mode.value != 0 {
            self.issue(
                self.grid_mode.modifier_index,
                Some(0x5032),
                "active_document_grid_layout_unresolved",
            );
            if self.grid_line_pitch_twips.is_none() {
                self.issue(
                    None,
                    Some(0x9031),
                    "active_document_grid_line_pitch_missing",
                );
            }
        }
        if let (Some(top), Some(bottom)) = (self.top_margin_twips, self.bottom_margin_twips)
            && u32::from(top.value.unsigned_abs()) + u32::from(bottom.value.unsigned_abs())
                >= u32::from(self.page_height_twips.value)
        {
            self.issue(None, None, "nonpositive_section_text_height");
        }
        if self.column_count.value > 1 && self.column_spacing_twips.is_none() {
            self.issue(
                None,
                Some(0x900c),
                "column_spacing_installation_default_unresolved",
            );
        }
        if !self.evenly_spaced_columns.value {
            self.issue(None, None, "individual_column_geometry_unresolved");
        }
    }
}

#[cfg(test)]
mod tests;
