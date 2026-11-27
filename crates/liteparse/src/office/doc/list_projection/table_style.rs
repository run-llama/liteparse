//! Appearance-only table inheritance for labels outside every table-member sequence.
use super::{BTreeSet, Error, Result, Stylesheet, bytes, u16_at};

/// Derived table styles may change row borders while retaining the validated default's
/// membership and visibility properties. Each successful ancestor is cached once.
/// MS-DOC §§2.9.16, 2.9.22, 2.9.43 and 2.9.302 define the border payload.
pub(super) fn validate(
    index: usize,
    styles: &Stylesheet,
    validated: &mut BTreeSet<usize>,
    remaining_bytes: &mut usize,
) -> Result<()> {
    validate_inner(index, styles, validated, remaining_bytes, false)
}

/// A closed nested row can isolate visibility uncertainty when its style cannot
/// change list selection. Keep this proof out of the stricter appearance cache.
pub(super) fn validate_nested_visibility(
    index: usize,
    styles: &Stylesheet,
    remaining_bytes: &mut usize,
) -> Result<()> {
    validate_inner(index, styles, &mut BTreeSet::new(), remaining_bytes, true)
}

fn validate_inner(
    mut index: usize,
    styles: &Stylesheet,
    validated: &mut BTreeSet<usize>,
    remaining_bytes: &mut usize,
    allow_hidden: bool,
) -> Result<()> {
    let mut chain = BTreeSet::new();
    while index != 11 && !validated.contains(&index) {
        if chain.len() >= 64 {
            return Err(Error::Limit("list table style inheritance depth"));
        }
        if !chain.insert(index) {
            return Err(Error::Invalid("list table style inheritance cycle"));
        }
        let style = styles
            .styles
            .get(index)
            .and_then(Option::as_ref)
            .filter(|s| {
                s.index == index
                    && s.kind == 3
                    && !s.revision_marked
                    && s.raw_revision.is_empty()
                    && s.linked_style.is_none()
            })
            .ok_or(Error::Unsupported(
                "list derived table style identity or history",
            ))?;
        for group in &style.properties {
            *remaining_bytes = remaining_bytes
                .checked_sub(group.bytes.len())
                .ok_or(Error::Limit("list table style property bytes"))?;
            match group.kind {
                "paragraph"
                    if group.bytes.len() == 2 && u16_at(&group.bytes, 0)? as usize == index => {}
                "character" if group.bytes.is_empty() => {}
                "character"
                    if allow_hidden
                        && group.bytes.len() % 3 == 0
                        && group
                            .bytes
                            .as_chunks::<3>()
                            .0
                            .iter()
                            .all(|p| matches!(p, [0x3c, 0x08, 0 | 1 | 128 | 129])) => {}
                "table" => {
                    let mut offset = 0;
                    while offset < group.bytes.len() {
                        let sprm = u16_at(&group.bytes, offset)?;
                        offset += 2;
                        match sprm {
                            0x563a => {
                                if u16_at(&group.bytes, offset)? as usize != index {
                                    return Err(Error::Invalid("list table style identity"));
                                }
                                offset += 2;
                            }
                            0xd613 => {
                                borders(bytes(&group.bytes, offset, 49)?)?;
                                offset += 49;
                            }
                            _ => {
                                return Err(Error::Unsupported(
                                    "list derived table style property",
                                ));
                            }
                        }
                    }
                }
                _ => return Err(Error::Unsupported("list derived table style group")),
            }
        }
        index = style
            .base_style
            .ok_or(Error::Unsupported("list table style default ancestry"))?;
    }
    validated.extend(chain);
    Ok(())
}

fn borders(operand: &[u8]) -> Result<()> {
    if operand.len() != 49 || operand[0] != 48 {
        return Err(Error::Invalid("list table style borders size"));
    }
    for border in operand[1..].as_chunks::<8>().0 {
        if !matches!(border[3], 0 | 255) || !matches!(border[5], 0 | 1 | 3 | 5..=27) {
            return Err(Error::Invalid("list table style border color or type"));
        }
    }
    Ok(())
}
