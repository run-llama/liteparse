//! Source row geometry for unmerged depth-one DOC tables.
//! Table identity, text styles and accepted revision ownership are separate inputs
//! to the adapter; a decoded row alone does not qualify a document for rendering.
use crate::office::doc::{
    Error, Result, formatting::PropertyModifier, table_structure::decode_table_definition,
};
use serde::Serialize;
use std::collections::BTreeSet;
pub mod grid;
pub mod horizontal;
mod legacy_later;
pub mod merges;
mod modern;
pub mod source;
pub mod style_defaults;
pub mod text_direction;

const MAX_CELLS: usize = 63;
const MAX_ROWS: usize = 16384;
const MAX_OPERAND_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Border {
    pub raw: u64,
    pub kind: BorderKind,
    pub eighth_points: u8,
    pub color_rgb: Option<u32>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum BorderKind {
    None,
    Nil,
    Single,
    Double,
    Triple,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CellLayout {
    pub edges_twips: [i16; 2],
    /// Top, logical left, bottom, logical right; None means no local border assignment.
    pub borders: [Option<Border>; 4],
    pub preferred_width: Option<Width>,
    pub margins_twips: [u16; 4],
    pub vertical_alignment: u8,
    pub raw_flags: u16,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub no_wrap: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shading_rgb: Option<u32>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Width {
    Twips(u16),
    PercentFiftieths(u16),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Height {
    Auto,
    AtLeast(u16),
    Exact(u16),
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RowLayout {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub horizontal_merge_basis: Option<&'static str>,
    pub cells: Vec<CellLayout>,
    /// Top, left, bottom, right, inside horizontal, inside vertical.
    pub borders: [Option<Border>; 6],
    pub height: Height,
    pub cant_split: bool,
    pub header: bool,
    pub physical_alignment: Option<u16>,
    pub gap_half_twips: u16,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub autofit: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autofit_preferred_width: Option<Width>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_indent_twips: Option<i16>,
    pub applied: Vec<PropertyModifier>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub style_defaults: Vec<style_defaults::CellDefaults>,
}

impl Default for RowLayout {
    fn default() -> Self {
        Self {
            horizontal_merge_basis: None,
            cells: Vec::new(),
            borders: Default::default(),
            height: Height::Auto,
            cant_split: false,
            header: false,
            physical_alignment: None,
            gap_half_twips: 0,
            autofit: false,
            autofit_preferred_width: None,
            preferred_indent_twips: None,
            applied: Vec::new(),
            style_defaults: Vec::new(),
        }
    }
}

/// Decode Brc80 with explicit nil distinguished from an unassigned border.
/// Supported line bands retain automatic/palette colors and per-stripe widths.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/cfab8014-e477-4e33-b50f-a23b8476f6f3>
pub fn border80(data: &[u8]) -> Result<Border> {
    let raw = u32::from_le_bytes(
        data.try_into()
            .map_err(|_| Error::Invalid("render Brc80 size"))?,
    );
    if raw == u32::MAX {
        return Ok(Border {
            raw: u64::from(raw),
            kind: BorderKind::Nil,
            eighth_points: 0,
            color_rgb: None,
        });
    }
    if data[2] > 16 || data[1] == 0x1a || data[1] == 0x1b {
        return Err(Error::Invalid("render Brc80 fields"));
    }
    if data[3] & 0x7f != 0 {
        return Err(Error::Unsupported("render border spacing or effects"));
    }
    let kind = match data[1] {
        0 => BorderKind::None,
        1 => BorderKind::Single,
        3 => BorderKind::Double,
        10 => BorderKind::Triple,
        _ => return Err(Error::Unsupported("render border type")),
    };
    let palette = [
        0x000000, 0x000000, 0x0000ff, 0x00ffff, 0x00ff00, 0xff00ff, 0xff0000, 0xffff00, 0xffffff,
        0x000080, 0x008080, 0x008000, 0x800080, 0x800000, 0x808000, 0x808080, 0xc0c0c0,
    ];
    Ok(Border {
        raw: u64::from(raw),
        kind,
        eighth_points: if matches!(
            kind,
            BorderKind::Single | BorderKind::Double | BorderKind::Triple
        ) {
            data[0].max(2)
        } else {
            0
        },
        color_rgb: (data[2] != 0).then_some(palette[data[2] as usize]),
    })
}

fn bool8(data: &[u8]) -> Result<bool> {
    match data {
        [0] => Ok(false),
        [1] => Ok(true),
        _ => Err(Error::Invalid("render table Bool8")),
    }
}
fn signed(data: &[u8]) -> Result<i16> {
    Ok(i16::from_le_bytes(data.try_into().map_err(|_| {
        Error::Invalid("render table signed operand")
    })?))
}

/// Apply a bounded, ordered sequence of direct table properties to source defaults.
/// The caller must first prove absence of inherited table-style contributions and
/// revision walls. Unsupported operators remain errors rather than implicit defaults.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/b39a6648-501c-4361-8366-4f042f579469>
pub fn decode_row(binary_version: u16, properties: &[PropertyModifier]) -> Result<RowLayout> {
    if properties.len() > 262144 {
        return Err(Error::Limit("render table modifiers"));
    }
    let mut budget = MAX_OPERAND_BYTES;
    let mut row = RowLayout::default();
    let mut explicit_gap = false;
    let mut later = legacy_later::State::default();
    for property in properties {
        budget = budget
            .checked_sub(property.operand.len())
            .ok_or(Error::Limit("render table operands"))?;
        let data = &property.operand;
        match property.sprm {
            0x5400 => {
                let value = signed(data)?;
                if !(0..=2).contains(&value) {
                    return Err(Error::Invalid("render table alignment"));
                }
                row.physical_alignment = Some(value as u16);
            }
            0x9602 => {
                let value = signed(data)?;
                if value < 0 {
                    return Err(Error::Invalid("render table half gap"));
                }
                // Historical Word modifies an existing left edge when this property changes.
                // Qualification currently requires the offset before the defining grid.
                if !row.cells.is_empty() {
                    return Err(Error::Unsupported("render table post-grid half gap"));
                }
                row.gap_half_twips = value as u16;
                explicit_gap = true;
            }
            0x3403 => {
                let value = bool8(data)?;
                if binary_version > 0xd9 {
                    return Err(Error::Unsupported("render legacy row split property"));
                }
                row.cant_split = value;
            }
            0x3466 => row.cant_split = bool8(data)?,
            0x3404 => row.header = bool8(data)?,
            0x9407 => {
                let value = signed(data)?;
                row.height = match value {
                    0 => Height::Auto,
                    1.. => Height::AtLeast(value as u16),
                    _ => Height::Exact(value.unsigned_abs()),
                };
            }
            0xd605 => {
                if data.len() != 25 || data[0] != 24 {
                    return Err(Error::Invalid("render row Brc80 array"));
                }
                for (target, source) in row.borders.iter_mut().zip(data[1..].as_chunks::<4>().0) {
                    *target = Some(border80(source)?);
                }
            }
            0xd608 => {
                if binary_version <= 0xd9 && !explicit_gap {
                    return Err(Error::Unsupported("render legacy cell padding default"));
                }
                let decoded = decode_table_definition(data)?;
                if decoded.cells.is_empty() {
                    return Err(Error::Unsupported("render empty cell definition"));
                }
                let records = &data[3 + 2 * decoded.grid_twips.len()..];
                let mut cells = Vec::with_capacity(decoded.cells.len());
                for (index, definition) in decoded.cells.iter().enumerate() {
                    let flags = definition.raw_flags;
                    text_direction::validate(flags)?;
                    if flags & 0x7000 != 0 || (flags & 3 != 0 && binary_version != 0xc1) {
                        return Err(Error::Unsupported(
                            "render horizontal merged or directed table cell",
                        ));
                    }
                    let edges = [decoded.grid_twips[index], decoded.grid_twips[index + 1]];
                    if edges[0] >= edges[1] {
                        return Err(Error::Unsupported("render nonpositive cell width"));
                    }
                    let mut borders: [Option<Border>; 4] = Default::default();
                    if let Some(record) = records.as_chunks::<20>().0.get(index) {
                        for (target, source) in
                            borders.iter_mut().zip(record[4..].as_chunks::<4>().0)
                        {
                            *target = Some(border80(source)?);
                        }
                    }
                    let width = definition.preferred_width_raw as u16;
                    let preferred_width = match definition.preferred_width_unit {
                        0 | 1 => None,
                        2 => Some(Width::PercentFiftieths(width)),
                        3 => Some(Width::Twips(width)),
                        _ => return Err(Error::Invalid("render cell width unit")),
                    };
                    // Historical TAP uses dxaGapHalf on both sides of each cell.
                    // Newer table padding defaults are independent of that origin offset.
                    let padding = if binary_version <= 0xd9 {
                        row.gap_half_twips
                    } else {
                        108
                    };
                    cells.push(CellLayout {
                        edges_twips: edges,
                        borders,
                        preferred_width,
                        margins_twips: [0, padding, 0, padding],
                        vertical_alignment: ((flags >> 7) & 3) as u8,
                        raw_flags: flags,
                        no_wrap: false,
                        shading_rgb: None,
                    });
                }
                row.cells = cells;
                if binary_version == 0xd9 {
                    later.grid(&mut row);
                }
            }
            0xd62b => merges::apply(&mut row.cells, data)?,
            0x7629 => text_direction::apply(&mut row.cells, data)?,
            _ if binary_version == 0xd9 => later.apply(property, &mut row)?,
            _ => return Err(Error::Unsupported("render table property")),
        }
        row.applied.push(property.clone());
    }
    if row.cells.is_empty() || row.cells.len() > MAX_CELLS {
        return Err(Error::Invalid("render table row missing cells"));
    }
    if binary_version == 0xd9 {
        later.finish(&mut row)?;
    }
    if row.cells.iter().any(|cell| cell.raw_flags & 3 != 0) {
        row.horizontal_merge_basis = Some(horizontal::WORD97_TC_BITS);
        horizontal::project(&row)?;
    }
    Ok(row)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GridPlan {
    pub edges_twips: Vec<i16>,
    /// Each row has one [start, limit) grid interval per physical source cell.
    pub cell_intervals: Vec<Vec<[usize; 2]>>,
}

/// Reconcile unequal row grids only after their table identity has been established.
/// Grid spans describe source cell widths; they do not suppress or merge cell content.
pub fn common_grid(rows: &[RowLayout]) -> Result<GridPlan> {
    if rows.is_empty() || rows.len() > MAX_ROWS {
        return Err(Error::Limit("render grid rows"));
    }
    let mut edges = BTreeSet::new();
    let first = rows[0]
        .cells
        .first()
        .ok_or(Error::Invalid("render grid cells"))?
        .edges_twips[0];
    let last = rows[0]
        .cells
        .last()
        .ok_or(Error::Invalid("render grid cells"))?
        .edges_twips[1];
    let mut count = 0usize;
    for row in rows {
        count = count
            .checked_add(row.cells.len())
            .filter(|n| *n <= 262144)
            .ok_or(Error::Limit("render grid cells"))?;
        if row.cells.is_empty()
            || row.cells.len() > MAX_CELLS
            || row.cells[0].edges_twips[0] != first
            || row.cells.last().unwrap().edges_twips[1] != last
            || row
                .cells
                .windows(2)
                .any(|w| w[0].edges_twips[1] != w[1].edges_twips[0])
        {
            return Err(Error::Unsupported("render irregular table outer grid"));
        }
        for cell in &row.cells {
            if cell.edges_twips[0] >= cell.edges_twips[1] {
                return Err(Error::Invalid("render cell grid order"));
            }
            edges.extend(cell.edges_twips);
            if edges.len() > 4096 {
                return Err(Error::Limit("render common grid columns"));
            }
        }
    }
    let edges_twips: Vec<_> = edges.into_iter().collect();
    let mut cell_intervals = Vec::with_capacity(rows.len());
    for row in rows {
        cell_intervals.push(
            row.cells
                .iter()
                .map(|c| {
                    Ok([
                        edges_twips
                            .binary_search(&c.edges_twips[0])
                            .map_err(|_| Error::Invalid("render cell start"))?,
                        edges_twips
                            .binary_search(&c.edges_twips[1])
                            .map_err(|_| Error::Invalid("render cell end"))?,
                    ])
                })
                .collect::<Result<Vec<_>>>()?,
        );
    }
    Ok(GridPlan {
        edges_twips,
        cell_intervals,
    })
}

#[cfg(test)]
mod tests;

/// Decode source-selected modern styles without treating their existence as empty.
pub fn decode_row_with_styles(
    version: u16,
    properties: &[PropertyModifier],
    styles: &crate::office::doc::styles::Stylesheet,
) -> Result<RowLayout> {
    if !properties.iter().any(|p| p.sprm == 0x7621) {
        if styles.styles.iter().flatten().any(|s| s.kind == 3) {
            return Err(Error::Unsupported(
                "render legacy grid table style contribution",
            ));
        }
        return decode_row(version, properties);
    }
    let mut ids = BTreeSet::from([11usize]);
    for p in properties.iter().filter(|p| p.sprm == 0x563a) {
        if p.operand.len() != 2 {
            return Err(Error::Invalid("render table style operand"));
        }
        ids.insert(crate::office::doc::u16_at(&p.operand, 0)? as usize);
    }
    if ids.len() > 64 {
        return Err(Error::Limit("render row selected style count"));
    }
    let defaults = ids
        .into_iter()
        .map(|i| style_defaults::resolve(styles, i))
        .collect::<Result<Vec<_>>>()?;
    modern::decode(version, properties, &defaults)
}
pub fn decode_recorded_row(version: u16, row: &RowLayout) -> Result<RowLayout> {
    if row.style_defaults.is_empty() {
        decode_row(version, &row.applied)
    } else {
        modern::decode(version, &row.applied, &row.style_defaults)
    }
}
