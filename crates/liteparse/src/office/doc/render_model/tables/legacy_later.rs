//! Ordered Word 2000 extensions to the legacy physical cell definition.
use super::{PropertyModifier, RowLayout, bool8, modern, style_defaults};
use crate::office::doc::{Error, Result};

#[derive(Default)]
pub(super) struct State {
    preferred: Option<u16>,
    modern_borders: bool,
    padding: [Option<u16>; 4],
}
impl State {
    // The defining grid supplies legacy gap-derived margins. Explicit defaults
    // survive a later definition and override only their selected sides.
    pub(super) fn grid(&self, row: &mut RowLayout) {
        for cell in &mut row.cells {
            for (target, value) in cell.margins_twips.iter_mut().zip(self.padding) {
                if let Some(value) = value {
                    *target = value;
                }
            }
        }
    }

    pub(super) fn apply(&mut self, property: &PropertyModifier, row: &mut RowLayout) -> Result<()> {
        let bytes = &property.operand;
        match property.sprm {
            0xd613 => {
                if bytes.len() != 49 || bytes[0] != 48 {
                    return Err(Error::Invalid("render modern row borders"));
                }
                self.modern_borders = true;
                for (target, bytes) in row.borders.iter_mut().zip(bytes[1..].as_chunks::<8>().0) {
                    *target = Some(modern::border(bytes)?);
                }
            }
            0xd61a..=0xd61d => colors(property.sprm, bytes, row)?,
            0xf614 => self.preferred = modern::table_width(bytes)?,
            0x3615 => row.autofit = bool8(bytes)?,
            0xf661 => row.preferred_indent_twips = modern::preferred_indent(bytes)?,
            0xf617 | 0xf618 => {
                if modern::table_width(bytes)?.is_some_and(|value| value != 0) {
                    return Err(Error::Unsupported("render extra table indent"));
                }
            }
            0x740a => modern::table_look(bytes)?,
            0xd634 => {
                let mut values = [0; 4];
                style_defaults::padding(bytes, &mut values)?;
                for (index, mask) in [1, 2, 4, 8].into_iter().enumerate() {
                    if bytes[3] & mask != 0 {
                        self.padding[index] = Some(values[index]);
                    }
                }
                self.grid(row);
            }
            _ => return Err(Error::Unsupported("render table property")),
        }
        Ok(())
    }

    pub(super) fn finish(self, row: &mut RowLayout) -> Result<()> {
        // DOC delegates displayed borders to tcBorders: None inherits the row
        // border, while Nil remains an explicit suppression. Applied bytes stay raw.
        if self.modern_borders {
            for cell in &mut row.cells {
                for border in &mut cell.borders {
                    if border
                        .as_ref()
                        .is_some_and(|b| b.kind == super::BorderKind::None)
                    {
                        *border = None;
                    }
                }
            }
        }
        let width = i32::from(row.cells.last().unwrap().edges_twips[1])
            - i32::from(row.cells[0].edges_twips[0]);
        if row.autofit {
            row.autofit_preferred_width = self.preferred.map(super::Width::Twips);
        } else if self
            .preferred
            .is_some_and(|value| i32::from(value) != width)
        {
            return Err(Error::Unsupported(
                "render preferred table width requires resizing",
            ));
        }
        Ok(())
    }
}

// Color arrays are indexed by the current physical cell definition. Their absent
// border sentinel is distinct from an automatic color on an existing border.
fn colors(sprm: u16, bytes: &[u8], row: &mut RowLayout) -> Result<()> {
    let count = row.cells.len();
    if count == 0 || bytes.len() != 1 + 4 * count || usize::from(bytes[0]) != 4 * count {
        return Err(Error::Invalid("render cell border color cardinality"));
    }
    let side = usize::from(sprm - 0xd61a);
    for (cell, color) in row.cells.iter_mut().zip(bytes[1..].as_chunks::<4>().0) {
        if *color == [255; 4] {
            cell.borders[side] = Some(super::Border {
                raw: u64::from(u32::MAX),
                kind: super::BorderKind::Nil,
                eighth_points: 0,
                color_rgb: None,
            });
            continue;
        }
        let rgb = match color[3] {
            0 => Some(u32::from(color[0]) << 16 | u32::from(color[1]) << 8 | u32::from(color[2])),
            255 => None,
            _ => return Err(Error::Invalid("render cell border color")),
        };
        let border = cell.borders[side].as_mut().ok_or(Error::Unsupported(
            "render cell border color without definition",
        ))?;
        if border.kind == super::BorderKind::None && rgb.is_some() {
            return Err(Error::Unsupported("render colored inherited cell border"));
        }
        border.color_rgb = rgb;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
