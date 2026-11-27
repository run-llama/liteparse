//! Table membership is derived from direct paragraph properties, including the end-piece PRM.
//! Paragraph styles cannot supply the properties preserved by PIstd (MS-DOC UpxPapx).
//! <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/659f448a-473a-44c6-8b69-a25984af3645>
use super::{MAX_STYLE_DEPTH, charge};
use crate::office::doc::{Error, Result, formatting::PropertyModifier, styles::Stylesheet};
use std::collections::BTreeMap;

pub(super) struct ParagraphProperties {
    pub style: Option<usize>,
    pub non_table: bool,
    pub table_depth: Option<u32>,
}

/// Apply direct PAPX followed by the paragraph-end PCD; membership defaults to zero.
/// PFInTable alone cannot prove a zero depth when PItap/PDtap or cell flags disagree.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/484822ee-a9d9-4af4-8423-29fda67a6a58>
#[expect(
    clippy::too_many_arguments,
    reason = "each argument is a separate DOC property source"
)]
pub(super) fn resolve(
    initial_style: usize,
    data: &[u8],
    prm: u16,
    groups: &[Vec<u8>],
    mark: u16,
    styles: &Stylesheet,
    style_cache: &mut BTreeMap<usize, bool>,
    work: &mut usize,
) -> Result<ParagraphProperties> {
    let mut properties = group(data, 1, work)?;
    properties.extend(piece(prm, groups, 1, work)?);
    let mut style = Some(initial_style);
    let mut in_table = false;
    let mut depth = 0i32;
    let mut row = false;
    let mut inner_cell = false;
    let mut inner_row = false;
    let mut known = true;
    for property in properties {
        let operand = &property.operand;
        match property.sprm {
            0x2416 => in_table = boolean(operand)?,
            0x2417 => row = boolean(operand)?,
            0x244b => inner_cell = boolean(operand)?,
            0x244c => inner_row = boolean(operand)?,
            0x6649 | 0x664a => {
                let value = crate::office::doc::u32_at(operand, 0)? as i32;
                depth = if property.sprm == 0x6649 {
                    value
                } else {
                    depth
                        .checked_add(value)
                        .ok_or(Error::Invalid("hidden table depth overflow"))?
                };
                if depth < 0 {
                    return Err(Error::Invalid("hidden negative table depth"));
                }
            }
            0x4600 => style = Some(crate::office::doc::u16_at(operand, 0)? as usize),
            0x2602 => {
                let value = operand[0] as i8 as i32;
                if let Some(index @ 1..=9) = style {
                    style = Some((index as i32 + value).clamp(1, 9) as usize);
                }
            }
            0xc601 => style = None,
            // Paragraph direction and numbering history preserve style and membership.
            0x2441 | 0x2443 => {
                boolean(operand)?;
            }
            // NumRM records historical numbering, leaving current style and table membership intact.
            0xc645 => {
                crate::office::doc::numbering_revision::decode_numrm_operand(operand)?;
            }
            sprm if style_preserving(sprm) => {}
            _ => {
                style = None;
                known = false;
            }
        }
    }
    let style_valid = if let Some(index) = style {
        validate_style(index, styles, style_cache, work)?
    } else {
        false
    };
    let non_table = known
        && style_valid
        && !in_table
        && depth == 0
        && !row
        && !inner_cell
        && !inner_row
        && mark != 7;
    Ok(ParagraphProperties {
        style: style.filter(|_| style_valid),
        non_table,
        table_depth: if non_table {
            Some(0)
        } else if known && in_table && depth > 0 {
            Some(depth as u32)
        } else {
            None
        },
    })
}

fn boolean(operand: &[u8]) -> Result<bool> {
    match operand {
        [0] => Ok(false),
        [1] => Ok(true),
        _ => Err(Error::Invalid("hidden paragraph Boolean")),
    }
}

/// Frame all operators, then select only the requested property scope.
/// MS-DOC Direct Paragraph/Character Formatting independently filter PCD's mixed groups.
pub(super) fn piece(
    prm: u16,
    groups: &[Vec<u8>],
    scope: u16,
    work: &mut usize,
) -> Result<Vec<PropertyModifier>> {
    if prm == 0 {
        return Ok(Vec::new());
    }
    if prm & 1 != 0 {
        let data = groups
            .get((prm >> 1) as usize)
            .ok_or(Error::Invalid("hidden piece property group"))?;
        return group(data, scope, work);
    }
    if scope == 2 {
        return crate::office::doc::visibility::piece_modifiers(prm, groups);
    }
    // Prm0 indexes have an explicit scope; only PIncLvl can change the paragraph style.
    // https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/35226a0b-9038-4427-83c2-3830a8554267
    let index = (prm >> 1) & 0x7f;
    let sprm = match index {
        0x04 => Some(0x2602),
        0x18 => Some(0x2416),
        0x19 => Some(0x2417),
        0x05 | 0x07 | 0x08 | 0x09 | 0x0c | 0x0d | 0x0e | 0x0f | 0x1d | 0x25 | 0x2c | 0x32
        | 0x33 | 0x35 | 0x36 | 0x37 | 0x38 | 0x39 | 0x3a | 0x78 | 0x7e => None,
        _ => {
            crate::office::doc::visibility::piece_modifiers(prm, groups)?;
            None
        }
    };
    charge(work, 3)?;
    Ok(sprm
        .into_iter()
        .map(|sprm| PropertyModifier {
            sprm,
            operand: vec![(prm >> 8) as u8],
        })
        .collect())
}

pub(super) fn group(data: &[u8], scope: u16, work: &mut usize) -> Result<Vec<PropertyModifier>> {
    charge(work, data.len() + 1)?;
    let mut offset = 0;
    let mut result = Vec::new();
    while offset < data.len() {
        let sprm = crate::office::doc::u16_at(data, offset)?;
        offset += 2;
        let sgc = (sprm >> 10) & 7;
        if !(1..=5).contains(&sgc) {
            return Err(Error::Unsupported("hidden property scope"));
        }
        let length =
            crate::office::doc::paragraph_references::operand_length(sprm, &data[offset..])?;
        let operand = crate::office::doc::bytes(data, offset, length)?;
        offset += length;
        if scope == 0 || sgc == scope {
            charge(work, std::mem::size_of::<PropertyModifier>() + length)?;
            result.push(PropertyModifier {
                sprm,
                operand: operand.to_vec(),
            });
        }
    }
    Ok(result)
}

fn validate_style(
    index: usize,
    styles: &Stylesheet,
    cache: &mut BTreeMap<usize, bool>,
    work: &mut usize,
) -> Result<bool> {
    let mut chain = Vec::new();
    let mut current = Some(index);
    let mut valid = true;
    while let Some(index) = current {
        charge(work, chain.len() + 1)?;
        if let Some(value) = cache.get(&index) {
            valid = *value;
            break;
        }
        if chain.len() == MAX_STYLE_DEPTH || chain.contains(&index) {
            return Err(Error::Invalid("hidden paragraph style cycle"));
        }
        chain.push(index);
        let Some(style) = styles.styles.get(index).and_then(Option::as_ref) else {
            valid = false;
            break;
        };
        if style.kind != 1 || style.revision_marked {
            valid = false;
            break;
        }
        for payload in style.properties.iter().filter(|p| p.kind == "paragraph") {
            if crate::office::doc::u16_at(&payload.bytes, 0)? as usize != index {
                return Err(Error::Invalid("hidden paragraph style identity"));
            }
            for property in group(&payload.bytes[2..], 0, work)? {
                // PFBiDi changes layout direction, leaving text visibility and membership intact.
                if property.sprm == 0x2441 {
                    boolean(&property.operand)?;
                    continue;
                }
                if (property.sprm >> 10) & 7 != 1
                    || matches!(
                        property.sprm,
                        0x2416 | 0x2417 | 0x6649 | 0x664a | 0x244b | 0x244c
                    )
                    || !style_preserving(property.sprm)
                {
                    valid = false;
                }
            }
        }
        if !valid {
            break;
        }
        current = style.base_style;
    }
    for index in chain {
        cache.insert(index, valid);
    }
    Ok(valid)
}
fn style_preserving(sprm: u16) -> bool {
    matches!(
        sprm,
        0x2416
            | 0x2417
            | 0x6649
            | 0x563a
            | 0x460b
            | 0x260a
            | 0x242a
            | 0x2431
            | 0x2433
            | 0x2434
            | 0x2435
            | 0x2436
            | 0x2447
            | 0x2462
            | 0x246d
            | 0x8418
            | 0x8419
            | 0x841a
            | 0x261b
            | 0x2423
            | 0x442b
            | 0x442c
            | 0x442d
            | 0x842e
            | 0x842f
            | 0xc64d
            | 0xc64e
            | 0xc64f
            | 0xc650
            | 0xc651
            | 0xc652
            | 0xc653
            | 0x6629
            | 0xc66c
            | 0x4455
            | 0x4456
            | 0x4457
            | 0x4458
            | 0x4459
            | 0x4439
            | 0x2403
            | 0x2405
            | 0x2406
            | 0x2407
            | 0x240c
            | 0xc60d
            | 0x840e
            | 0x840f
            | 0x4610
            | 0x8411
            | 0x6412
            | 0xa413
            | 0xa414
            | 0xc615
            | 0x6424
            | 0x6425
            | 0x6426
            | 0x6427
            | 0x6428
            | 0x6429
            | 0x2437
            | 0x2438
            | 0x2448
            | 0x245b
            | 0x245c
            | 0x845d
            | 0x845e
            | 0x8460
            | 0x2461
            | 0x6467
            | 0x2640
    )
}
