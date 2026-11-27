//! Source-owned embedded picture properties, before anchor placement is qualified.
use crate::office::doc::{Error, Result, bytes, u16_at, u32_at};
use serde::Serialize;

#[derive(Debug, PartialEq, Serialize)]
pub struct ImageProperties {
    /// One-based OfficeArt BStore index; asset ownership is checked by the caller.
    pub blip_index: u32,
    pub source_name: Option<String>,
    pub source_name_kind: u32,
    /// Authored shape metadata, separate from the source image filename.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape_description: Option<String>,
    /// Explicit OfficeArt paint layer, checked against the owning Word anchor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behind_document: Option<bool>,
    /// Retained inline relative layer; Word layering remains explicitly unresolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline_relative_layer: Option<u32>,
    /// Separate shape fill state; the PIB raster remains independently selected.
    #[serde(skip_serializing_if = "filled_default")]
    pub fill_enabled: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unpainted_default_outline: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unpainted_fill_color: Option<u32>,
    /// Signed 16.16 source fractions in top, bottom, left, right order.
    pub crop_fixed: [i32; 4],
    /// Raw horizontal alignment/reference and vertical alignment/reference.
    pub positioning: [Option<u32>; 4],
    pub wrap_polygon_points: Option<usize>,
}

impl ImageProperties {
    /// Source signed 16.16 edges use top/bottom/left/right order; shared paint uses
    /// left/top/right/bottom. Bound the retained window before shared float layout.
    pub fn crop(&self) -> Result<(f64, f64, f64, f64)> {
        let [top, bottom, left, right] = self.crop_fixed;
        if self.crop_fixed.iter().any(|v| !(-65536..65536).contains(v))
            || i64::from(top) + i64::from(bottom) >= 65536
            || i64::from(left) + i64::from(right) >= 65536
        {
            return Err(Error::Unsupported("render picture crop rectangle"));
        }
        Ok((
            f64::from(left) / 65536.0,
            f64::from(top) / 65536.0,
            f64::from(right) / 65536.0,
            f64::from(bottom) / 65536.0,
        ))
    }
}

struct Property<'a> {
    id: u16,
    value: u32,
    complex: Option<&'a [u8]>,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde skip_serializing_if passes a reference"
)]
fn filled_default(value: &bool) -> bool {
    *value
}

fn invalid() -> Error {
    Error::Unsupported("render embedded picture properties")
}

// FOPTE operands precede all complex payloads. Each payload consumes exactly its
// declared byte extent, even when a later property supersedes its semantic value.
fn properties(data: &[u8], count: usize) -> Result<Vec<Property<'_>>> {
    if count > 4096 || data.len() > 65_536 {
        return Err(Error::Limit("render picture property bytes"));
    }
    let mut tail = count.checked_mul(6).ok_or_else(invalid)?;
    bytes(data, 0, tail)?;
    let mut result = Vec::with_capacity(count);
    let mut previous = None;
    for index in 0..count {
        let id = u16_at(data, index * 6)?;
        let key = id & 0x3fff;
        if previous.is_some_and(|old| old >= key) {
            return Err(invalid());
        }
        previous = Some(key);
        let value = u32_at(data, index * 6 + 2)?;
        let complex = if id & 0x8000 != 0 {
            let payload = bytes(data, tail, value as usize)?;
            tail = tail.checked_add(value as usize).ok_or_else(invalid)?;
            Some(payload)
        } else {
            None
        };
        result.push(Property { id, value, complex });
    }
    if tail != data.len() {
        return Err(invalid());
    }
    Ok(result)
}

fn name(data: &[u8]) -> Result<String> {
    if data.len() < 2 || !data.len().is_multiple_of(2) || data.len() > 8192 {
        return Err(invalid());
    }
    let units = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u16::from_le_bytes([p[0], p[1]]))
        .collect::<Vec<_>>();
    if units.last() != Some(&0) || units[..units.len() - 1].contains(&0) {
        return Err(invalid());
    }
    String::from_utf16(&units[..units.len() - 1]).map_err(|_| invalid())
}

fn polygon(data: &[u8]) -> Result<usize> {
    let count = u16_at(data, 0)? as usize;
    let allocated = u16_at(data, 2)? as usize;
    let size = u16_at(data, 4)?;
    if size != 8 || count > allocated || count > 4096 || data.len() != 6 + count * 8 {
        return Err(invalid());
    }
    Ok(count)
}

/// Decode the bounded raster appearance subset without selecting a page origin.
/// A saved position reference of zero stays raw; it does not qualify placement.
pub fn decode(
    primary: (&[u8], usize),
    tertiary: Option<(&[u8], usize)>,
) -> Result<ImageProperties> {
    decode_with_inline_layer(primary, tertiary, false)
}

/// Inline owners retain relative layering as unmodeled presentation evidence.
pub(crate) fn decode_inline(
    primary: (&[u8], usize),
    tertiary: Option<(&[u8], usize)>,
) -> Result<ImageProperties> {
    decode_with_inline_layer(primary, tertiary, true)
}

fn decode_with_inline_layer(
    primary: (&[u8], usize),
    tertiary: Option<(&[u8], usize)>,
    inline: bool,
) -> Result<ImageProperties> {
    let mut result = ImageProperties {
        blip_index: 0,
        source_name: None,
        source_name_kind: 0,
        shape_name: None,
        shape_description: None,
        behind_document: None,
        inline_relative_layer: None,
        fill_enabled: true,
        unpainted_default_outline: false,
        unpainted_fill_color: None,
        crop_fixed: [0; 4],
        positioning: [None; 4],
        wrap_polygon_points: None,
    };
    let mut no_line = false;
    let mut retained_fill_color = None;
    for group in std::iter::once(primary).chain(tertiary) {
        for property in properties(group.0, group.1)? {
            match property.id {
                // MS-ODRAW 2.3.20.1 locks editing operations; saved appearance is unchanged.
                // The exact identifier also enforces the required non-BLIP, simple operand.
                0x7f => {}
                // MS-ODRAW 2.3.4.43 / Appendix A note 33: Word uses dhgt.
                // Only CP-owned inline pictures retain this unresolved layer.
                0x3aa if inline => result.inline_relative_layer = Some(property.value),
                // Imported-OLE provenance is retained in raw properties; fake masters do not paint.
                0x43f if property.value & (property.value >> 16) & 2 == 0 => {}
                // Enabled shape fill color stays explicit as unapplied presentation beside the raster.
                0x181 => retained_fill_color = Some(property.value),
                // Relative resizing controls editor UI and preserves saved geometry.
                0x33f if property.value == 0x100010 => {}
                // Explicit default layout-in-cell/allow-overlap values preserve the
                // same placement policy as omitted Group Shape Boolean properties.
                0x3bf
                    if property.value & !0x82008200 == 0
                        && ((property.value & (property.value >> 16))
                            | (0x8200 & !(property.value >> 16)))
                            == 0x8200 => {}
                // Explicit false script-anchor/really-hidden flags preserve visible pictures.
                0x3bf if property.value == 0x1800000 => {}
                0x100..=0x103 => {
                    result.crop_fixed[(property.id - 0x100) as usize] = property.value as i32;
                }
                0x4104 if property.value > 0 => result.blip_index = property.value,
                0x8105 | 0xc105 => {
                    result.source_name = Some(name(property.complex.ok_or_else(invalid)?)?);
                }
                0x105 | 0x4105 if property.value == 0 => result.source_name = None,
                // Name interpretation flags alone do not establish an external link.
                0x106 if property.value <= 2 => result.source_name_kind = property.value,
                // BLIP flags use paired value/use bits; unset use bits select false defaults.
                // All seven effective flags must remain false in this raster appearance subset.
                0x13f if property.value & (property.value >> 16) & 0x7f == 0 => {}
                // Text Boolean reserved bits are ignored. Active text fitting/margins
                // remain outside the text-free picture container contract.
                0xbf if property.value & (property.value >> 16) & 0xa == 0 => {}
                // PIB selects the raster independently of the separate shape fill.
                // Preserve explicit no-fill while retaining default other visual flags.
                0x1bf => {
                    let effective = (property.value & (property.value >> 16))
                        | (0x14 & !(property.value >> 16));
                    if effective & 0x66 != 0x4 {
                        return Err(invalid());
                    }
                    result.fill_enabled = effective & 0x10 != 0;
                }
                0x1ff if property.value == 0x80000 => no_line = true,
                // MS-ODRAW 2.3.4.1–4: fBid is ignored for shape names/descriptions.
                // Simple values must be zero; complex values own terminated UTF-16 bytes.
                0x8380 | 0xc380 => {
                    result.shape_name = Some(name(property.complex.ok_or_else(invalid)?)?);
                }
                0x8381 | 0xc381 => {
                    result.shape_description = Some(name(property.complex.ok_or_else(invalid)?)?);
                }
                0x380 | 0x4380 if property.value == 0 => result.shape_name = None,
                0x381 | 0x4381 if property.value == 0 => result.shape_description = None,
                0x8383 | 0xc383 => {
                    result.wrap_polygon_points =
                        Some(polygon(property.complex.ok_or_else(invalid)?)?);
                }
                0x383 | 0x4383 if property.value == 0 => result.wrap_polygon_points = None,
                0x38f | 0x391 if property.value <= 5 => {
                    result.positioning[(property.id - 0x38f) as usize] = Some(property.value);
                }
                0x390 | 0x392 if property.value <= 4 => {
                    result.positioning[(property.id - 0x38f) as usize] = Some(property.value);
                }
                // MS-ODRAW 2.3.4.44: bit 5 selects behind-document paint; bit 21 enables it.
                0x3bf
                    if property.value & !0x220022 == 0
                        && property.value & (property.value >> 16) & 2 == 0 =>
                {
                    if property.value & 0x200000 != 0 {
                        result.behind_document = Some(property.value & 0x20 != 0);
                    }
                }
                0x53f if property.value == 0x10000 => {}
                _ => return Err(invalid()),
            }
        }
    }
    result.unpainted_default_outline = !no_line;
    result.unpainted_fill_color = retained_fill_color.filter(|_| result.fill_enabled);
    if result.blip_index == 0 {
        return Err(invalid());
    }
    result.crop()?;
    Ok(result)
}

#[cfg(test)]
mod tests;
