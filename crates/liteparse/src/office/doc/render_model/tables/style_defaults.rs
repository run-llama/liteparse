//! Source-qualified cell defaults from a bounded table-style inheritance chain.
//! This initial decoder accepts empty text contributions and explicit padding only;
//! conditional formatting and other painting remain unresolved.
use super::{Border, CellLayout};
use crate::office::doc::{Error, Result, formatting::PropertyModifier, styles::Stylesheet};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CellDefaults {
    pub selected_style: usize,
    pub base_to_derived: Vec<usize>,
    pub cell: CellLayout,
    pub row_borders: [Option<Border>; 6],
    pub applied: Vec<(usize, PropertyModifier)>,
}

/// Resolve supported table-style cell defaults, retaining every table operand.
/// UpxTapx expressly ignores embedded TIstd; direct row TIstd uses a separate path.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/eed2eef0-2943-4c2f-b348-3f1d1af59fdf>
pub fn resolve(styles: &Stylesheet, selected: usize) -> Result<CellDefaults> {
    let mut chain = Vec::new();
    let mut seen = BTreeSet::new();
    let mut next = Some(selected);
    let mut budget = 8 * 1024 * 1024usize;
    while let Some(index) = next {
        if chain.len() == 64 || !seen.insert(index) {
            return Err(Error::Limit("render table style ancestry"));
        }
        let style = styles
            .styles
            .get(index)
            .and_then(Option::as_ref)
            .ok_or(Error::Unsupported(
                "render selected table style unavailable",
            ))?;
        if style.index != index
            || style.kind != 3
            || style.revision_marked
            || !style.raw_revision.is_empty()
        {
            return Err(Error::Unsupported(
                "render table style identity or revision",
            ));
        }
        budget = budget
            .checked_sub(1)
            .ok_or(Error::Limit("render table style work"))?;
        chain.push(index);
        next = style.base_style;
    }
    chain.reverse();
    let mut result = CellDefaults {
        selected_style: selected,
        base_to_derived: chain.clone(),
        cell: CellLayout {
            edges_twips: [0, 0],
            borders: Default::default(),
            preferred_width: None,
            margins_twips: [0, 108, 0, 108],
            vertical_alignment: 0,
            raw_flags: 0,
            no_wrap: false,
            shading_rgb: None,
        },
        row_borders: Default::default(),
        applied: Vec::new(),
    };
    for index in chain {
        let style = styles.styles[index].as_ref().unwrap();
        let mut before = false;
        let mut groups = BTreeSet::new();
        for group in &style.properties {
            if !groups.insert(group.kind) {
                return Err(Error::Invalid(
                    "render duplicate table style property group",
                ));
            }
            budget = budget
                .checked_sub(group.bytes.len() + 1)
                .ok_or(Error::Limit("render table style bytes"))?;
            match group.kind {
                "character" if group.bytes.is_empty() => {}
                "paragraph" if group.bytes.as_slice() == (index as u16).to_le_bytes() => {}
                "table" => {
                    for p in decode(&group.bytes)? {
                        budget = budget
                            .checked_sub(std::mem::size_of::<(usize, PropertyModifier)>())
                            .ok_or(Error::Limit("render table style output"))?;
                        match p.sprm {
                            0x563a if p.operand.len() == 2 => {}
                            0xf617 if index == 11 && p.operand == [3, 0, 0] => before = true,
                            0xf661 if p.operand == [3, 0, 0] => {}
                            0xd634 => padding(&p.operand, &mut result.cell.margins_twips)?,
                            0xd613 => row_borders(&p.operand, &mut result.row_borders)?,
                            _ => {
                                return Err(Error::Unsupported(
                                    "render table style cell contribution",
                                ));
                            }
                        }
                        result.applied.push((index, p));
                    }
                }
                _ => return Err(Error::Unsupported("render table style text contribution")),
            }
        }
        if index == 11 && !before {
            return Err(Error::Invalid("render default table style leading width"));
        }
    }
    Ok(result)
}
/// Recheck a serialized style contribution before a public model is rendered.
pub fn validate(value: &CellDefaults) -> Result<()> {
    if value.base_to_derived.is_empty()
        || value.base_to_derived.len() > 64
        || value.base_to_derived.last() != Some(&value.selected_style)
        || value.base_to_derived.iter().collect::<BTreeSet<_>>().len()
            != value.base_to_derived.len()
    {
        return Err(Error::Invalid("render recorded table style ancestry"));
    }
    let mut cell = CellLayout {
        edges_twips: [0, 0],
        borders: Default::default(),
        preferred_width: None,
        margins_twips: [0, 108, 0, 108],
        vertical_alignment: 0,
        raw_flags: 0,
        no_wrap: false,
        shading_rgb: None,
    };
    let mut borders = Default::default();
    let mut cursor = 0;
    let mut normal_before = false;
    let mut budget = 8 * 1024 * 1024usize;
    for (index, p) in &value.applied {
        budget = budget
            .checked_sub(p.operand.len() + std::mem::size_of::<PropertyModifier>())
            .ok_or(Error::Limit("render recorded table style work"))?;
        while value.base_to_derived.get(cursor) != Some(index) {
            cursor += 1;
            if cursor == value.base_to_derived.len() {
                return Err(Error::Invalid("render recorded style order"));
            }
        }
        match p.sprm {
            0x563a if p.operand.len() == 2 => {}
            0xf617 if *index == 11 && p.operand == [3, 0, 0] => normal_before = true,
            0xf661 if p.operand == [3, 0, 0] => {}
            0xd634 => padding(&p.operand, &mut cell.margins_twips)?,
            0xd613 => row_borders(&p.operand, &mut borders)?,
            _ => return Err(Error::Unsupported("render recorded style contribution")),
        }
    }
    if value.base_to_derived.contains(&11) && !normal_before
        || cell != value.cell
        || borders != value.row_borders
    {
        return Err(Error::Invalid("render recorded style cell defaults"));
    }
    Ok(())
}
fn row_borders(bytes: &[u8], borders: &mut [Option<Border>; 6]) -> Result<()> {
    if bytes.len() != 49 || bytes[0] != 48 {
        return Err(Error::Invalid("render style row border operand"));
    }
    for (slot, bytes) in borders.iter_mut().zip(bytes[1..].as_chunks::<8>().0) {
        *slot = Some(super::modern::border(bytes)?);
    }
    Ok(())
}
fn decode(data: &[u8]) -> Result<Vec<PropertyModifier>> {
    let mut offset = 0;
    let mut out = Vec::new();
    while offset < data.len() {
        let sprm = crate::office::doc::u16_at(data, offset)?;
        offset += 2;
        if (sprm >> 10) & 7 != 5 {
            return Err(Error::Invalid("render non-table style modifier"));
        }
        let size = crate::office::doc::paragraph_references::operand_length(sprm, &data[offset..])?;
        let operand = crate::office::doc::bytes(data, offset, size)?.to_vec();
        offset += size;
        if out.len() == 262144 {
            return Err(Error::Limit("render table style modifier count"));
        }
        out.push(PropertyModifier { sprm, operand });
    }
    Ok(out)
}
pub(super) fn padding(data: &[u8], margins: &mut [u16; 4]) -> Result<()> {
    let [6, 0, 1, sides, unit, lo, hi] = *data else {
        return Err(Error::Invalid("render style padding operand"));
    };
    let value = u16::from_le_bytes([lo, hi]);
    if sides & !15 != 0 || !matches!(unit, 0 | 3) || value > 31680 {
        return Err(Error::Invalid("render style padding value"));
    }
    if unit == 0 {
        return Err(Error::Unsupported("render style nil cell padding"));
    }
    for (i, mask) in [1, 2, 4, 8].into_iter().enumerate() {
        if sides & mask != 0 {
            margins[i] = value;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests;
