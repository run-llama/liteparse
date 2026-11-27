//! Source-selected conditional character properties for rectangular table cells.
mod effective;
mod roles;
pub(super) use effective::populate;
#[cfg(test)]
pub(super) use effective::populate_with_budget;
mod ownership;
use super::{charge, paragraph};
use crate::office::doc::{Error, Result, formatting::PropertyModifier, styles::Stylesheet};
pub(super) use ownership::enrich;
use serde::Serialize;

#[derive(Clone, Debug, Default, Serialize)]
pub struct TextState {
    pub bold: bool,
    pub italic: bool,
    pub complex_bold: bool,
    pub complex_italic: bool,
    pub all_caps: bool,
    pub small_caps: bool,
    pub hidden: bool,
    pub fonts: [Option<u16>; 4],
    pub languages: [Option<u16>; 3],
    pub color_rgb: Option<[u8; 3]>,
    pub color_palette: Option<u16>,
    pub complex_color_palette: Option<u16>,
    pub size_half_points: Option<u16>,
    pub complex_size_half_points: Option<u16>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Applied {
    pub style_index: usize,
    pub condition: Option<u16>,
    pub property: PropertyModifier,
}
#[derive(Clone, Debug, Serialize)]
pub struct CellText {
    pub source_range: [usize; 2],
    pub row: usize,
    pub column: usize,
    pub conditions: Vec<u16>,
    pub state: TextState,
    pub applied: Vec<Applied>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub effective_runs: Vec<EffectiveRun>,
}
#[derive(Clone, Debug, Serialize)]
pub struct EffectiveRun {
    pub source_range: [usize; 2],
    pub properties: Option<crate::office::doc::render_model::runs::RunProperties>,
    pub unresolved: Option<String>,
}
#[derive(Clone)]
struct Branch {
    condition: u16,
    properties: Vec<PropertyModifier>,
    style: usize,
}
pub(super) struct Style {
    pub ancestors: Vec<usize>,
    global: Vec<Applied>,
    branches: Vec<Branch>,
    row_band: Option<u8>,
    column_band: Option<u8>,
}

fn cnf(data: &[u8]) -> Result<(u16, &[u8])> {
    if data.len() < 3 || usize::from(data[0]) + 1 != data.len() {
        return Err(Error::Invalid("conditional style operand size"));
    }
    let condition = crate::office::doc::u16_at(data, 1)?;
    if !matches!(
        condition,
        1 | 2 | 4 | 8 | 0x10 | 0x20 | 0x40 | 0x80 | 0x100 | 0x200 | 0x400 | 0x800
    ) {
        return Err(Error::Invalid("conditional style condition"));
    }
    Ok((condition, &data[3..]))
}

fn apply(state: &mut TextState, property: &PropertyModifier) -> Result<()> {
    let b = property.operand.as_slice();
    let toggle = || match b {
        [0] => Ok(false),
        [1] => Ok(true),
        _ => Err(Error::Unsupported("conditional relative toggle")),
    };
    match property.sprm {
        0x0835 => state.bold = toggle()?,
        0x0836 => state.italic = toggle()?,
        0x085c => state.complex_bold = toggle()?,
        0x085d => state.complex_italic = toggle()?,
        0x083a => state.small_caps = toggle()?,
        0x083b => state.all_caps = toggle()?,
        0x083c => state.hidden = toggle()?,
        0x4a4f..=0x4a51 | 0x4a5e => {
            let n = crate::office::doc::u16_at(b, 0)?;
            if n > 32767 {
                return Err(Error::Invalid("conditional font index"));
            }
            let slot = if property.sprm == 0x4a5e {
                3
            } else {
                usize::from(property.sprm - 0x4a4f)
            };
            state.fonts[slot] = Some(n);
        }
        0x486d | 0x4873 => state.languages[0] = Some(crate::office::doc::u16_at(b, 0)?),
        0x486e | 0x4874 => state.languages[1] = Some(crate::office::doc::u16_at(b, 0)?),
        0x485f => state.languages[2] = Some(crate::office::doc::u16_at(b, 0)?),
        0x2a42 => {
            let [n] = b else {
                return Err(Error::Invalid("conditional palette operand"));
            };
            if *n > 16 {
                return Err(Error::Invalid("conditional palette index"));
            }
            state.color_palette = Some(u16::from(*n));
            state.color_rgb = None;
        }
        0x4a60 => {
            let n = crate::office::doc::u16_at(b, 0)?;
            if n > 16 {
                return Err(Error::Invalid("conditional complex palette"));
            }
            state.complex_color_palette = Some(n);
        }
        0x6870 => {
            let [r, g, blue, auto] = *b else {
                return Err(Error::Invalid("conditional RGB operand"));
            };
            if !matches!(auto, 0 | 255) {
                return Err(Error::Invalid("conditional RGB flag"));
            }
            state.color_rgb = (auto == 0).then_some([r, g, blue]);
            state.color_palette = None;
        }
        0x4a43 | 0x4a61 => {
            let n = crate::office::doc::u16_at(b, 0)?;
            if n > 3276 || property.sprm == 0x4a43 && n < 2 {
                return Err(Error::Invalid("conditional font size"));
            }
            if property.sprm == 0x4a43 {
                state.size_half_points = Some(n);
            } else {
                state.complex_size_half_points = Some(n);
            }
        }
        _ => return Err(Error::Unsupported("conditional character property")),
    }
    Ok(())
}

/// Decode table-style ancestors with source-scoped conditional envelopes.
/// Character state is evaluated only after a physical cell supplies its enabled roles.
pub(super) fn decode(index: usize, styles: &Stylesheet, budget: &mut usize) -> Result<Style> {
    let mut ancestors = Vec::new();
    let mut next = Some(index);
    while let Some(i) = next {
        charge(budget, 32)?;
        if ancestors.len() == 64 || ancestors.contains(&i) {
            return Err(Error::Invalid("conditional style ancestry"));
        }
        let s = styles
            .styles
            .get(i)
            .and_then(Option::as_ref)
            .ok_or(Error::Invalid("conditional selected style"))?;
        if s.index != i
            || s.kind != 3
            || s.revision_marked
            || !s.raw_revision.is_empty()
            || s.linked_style.is_some()
        {
            return Err(Error::Unsupported("conditional style history or identity"));
        }
        ancestors.push(i);
        next = s.base_style;
    }
    let mut result = Style {
        ancestors: ancestors.clone(),
        global: Vec::new(),
        branches: Vec::new(),
        row_band: None,
        column_band: None,
    };
    for &i in ancestors.iter().rev() {
        let s = styles.styles[i].as_ref().unwrap();
        let mut seen = [false; 3];
        for g in &s.properties {
            let slot = match g.kind {
                "table" => 0,
                "paragraph" => 1,
                "character" => 2,
                _ => return Err(Error::Unsupported("conditional style group")),
            };
            if seen[slot] {
                return Err(Error::Invalid("conditional duplicate group"));
            }
            seen[slot] = true;
            if slot == 1 {
                if g.bytes.len() != 2 || crate::office::doc::u16_at(&g.bytes, 0)? as usize != i {
                    return Err(Error::Unsupported("conditional paragraph properties"));
                }
                continue;
            }
            for property in paragraph::group(&g.bytes, 0, budget)? {
                if slot == 2 && property.sprm == 0xca85 {
                    let (condition, bytes) = cnf(&property.operand)?;
                    let properties = paragraph::group(bytes, 0, budget)?;
                    for p in &properties {
                        apply(&mut TextState::default(), p)?;
                    }
                    result.branches.push(Branch {
                        condition,
                        properties,
                        style: i,
                    });
                } else if slot == 2 {
                    apply(&mut TextState::default(), &property)?;
                    result.global.push(Applied {
                        style_index: i,
                        condition: None,
                        property,
                    });
                } else {
                    match property.sprm {
                        0x563a
                            if crate::office::doc::u16_at(&property.operand, 0)? as usize == i => {}
                        0xd613 if super::valid_borders(&property.operand) => {}
                        0xf617 | 0xf661
                            if property.operand.len() == 3 && property.operand == [3, 0, 0] => {}
                        0xd634 if super::valid_default_padding(&property.operand) => {}
                        0xd687 if super::valid_cell_shading(0xd612, &property.operand, 1) => {}
                        0x3488 | 0x3489 => {
                            let [width @ 1..=3] = property.operand.as_slice() else {
                                return Err(Error::Invalid("conditional band size"));
                            };
                            if property.sprm == 0x3488 {
                                result.row_band = Some(*width);
                            } else {
                                result.column_band = Some(*width);
                            }
                        }
                        0xd66a => {
                            let (_, bytes) = cnf(&property.operand)?;
                            for p in paragraph::group(bytes, 0, budget)? {
                                if !matches!(p.sprm, 0xd47f | 0xd680..=0xd686)
                                    || !crate::office::doc::hidden_visibility::appearance::character_border(
                                        0xca72, &p.operand,
                                    )
                                {
                                    return Err(Error::Unsupported("conditional table decoration"));
                                }
                            }
                        }
                        _ => return Err(Error::Unsupported("conditional table style property")),
                    }
                }
            }
        }
        if !seen.into_iter().all(|x| x) {
            return Err(Error::Invalid("conditional missing style group"));
        }
    }
    Ok(result)
}

impl Style {
    fn cell(
        &self,
        range: [usize; 2],
        position: roles::Position<'_>,
        flags: u16,
        budget: &mut usize,
    ) -> Result<CellText> {
        if self
            .branches
            .iter()
            .any(|b| matches!(b.condition, 0x40 | 0x80))
            && self.row_band.is_none()
            || self
                .branches
                .iter()
                .any(|b| matches!(b.condition, 0x10 | 0x20))
                && self.column_band.is_none()
        {
            return Err(Error::Unsupported("conditional missing band size"));
        }
        let row = position.row;
        let column = position.column;
        let conditions = roles::conditions(position, flags, self.row_band, self.column_band)?;
        let mut state = TextState::default();
        let mut applied = Vec::new();
        for p in &self.global {
            charge(budget, 32 + p.property.operand.len())?;
            apply(&mut state, &p.property)?;
            applied.push(p.clone());
        }
        for &condition in &conditions {
            for branch in self.branches.iter().filter(|b| b.condition == condition) {
                for property in &branch.properties {
                    charge(budget, 32 + property.operand.len())?;
                    apply(&mut state, property)?;
                    applied.push(Applied {
                        style_index: branch.style,
                        condition: Some(condition),
                        property: property.clone(),
                    });
                }
            }
        }
        Ok(CellText {
            source_range: range,
            row,
            column,
            conditions,
            state,
            applied,
            effective_runs: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests;
