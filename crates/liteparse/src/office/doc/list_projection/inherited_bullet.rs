//! Logical ASCII bullet identity when every available source font layer agrees.
//! This proof does not select a renderer face or establish number-text typography.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct Agreement(u16);

impl Agreement {
    pub(super) fn font(self) -> u16 {
        self.0
    }
}

fn charge(budget: &mut usize, bytes: usize) -> Result<()> {
    *budget = budget
        .checked_sub(bytes)
        .ok_or(Error::Limit("inherited bullet property work"))?;
    Ok(())
}

fn properties_agree(properties: &[PropertyModifier], font: u16, budget: &mut usize) -> Result<()> {
    for p in properties {
        charge(budget, 2 + p.operand.len())?;
        if !visible_modifier(p) {
            return Err(Error::Unsupported("inherited bullet character property"));
        }
        match p.sprm {
            0x4a4f | 0x4a50 | 0x4a51 | 0x4a5e => {
                if p.operand.len() != 2 || u16_at(&p.operand, 0)? != font {
                    return Err(Error::Unsupported("inherited bullet conflicting font"));
                }
            }
            0x286f if p.operand != [0] => {
                return Err(Error::Unsupported("inherited bullet font routing"));
            }
            _ => {}
        }
    }
    Ok(())
}

fn style_agrees(c: &ListContext<'_>, start: usize, font: u16, budget: &mut usize) -> Result<()> {
    let mut current = Some(start);
    let mut seen = BTreeSet::new();
    while let Some(index) = current {
        charge(budget, 1)?;
        if seen.len() >= 128 || !seen.insert(index) {
            return Err(Error::Invalid("inherited bullet style ancestry"));
        }
        let style = c
            .styles
            .styles
            .get(index)
            .and_then(Option::as_ref)
            .filter(|s| {
                s.index == index
                    && s.kind == 1
                    && !s.revision_marked
                    && s.raw_revision.is_empty()
                    && s.linked_style.is_none()
            })
            .ok_or(Error::Unsupported("inherited bullet paragraph style"))?;
        for group in &style.properties {
            charge(budget, group.bytes.len())?;
            match group.kind {
                "paragraph" if u16_at(&group.bytes, 0)? as usize == index => {}
                "character" => {
                    properties_agree(&decode_character_modifiers(&group.bytes)?, font, budget)?
                }
                _ => return Err(Error::Unsupported("inherited bullet style group")),
            }
        }
        current = style.base_style;
    }
    Ok(())
}

pub(super) fn resolve(
    c: &ListContext<'_>,
    start: usize,
    mark: usize,
    level: &ListLevel,
    budget: &mut usize,
) -> Result<Agreement> {
    if level.number_format != 0x17
        || level.number_text_utf16.len() != 1
        || !(0x21..=0x7e).contains(&level.number_text_utf16[0])
    {
        return Err(Error::Unsupported("inherited bullet scalar"));
    }
    let header = &c.styles.raw_header;
    let font = u16_at(header, 12)?;
    if u16_at(header, 14)? != font
        || u16_at(header, 16)? != font
        || (header.len() >= 20 && u16_at(header, 18)? != font)
    {
        return Err(Error::Unsupported(
            "inherited bullet default font disagreement",
        ));
    }
    let entry = c
        .fonts
        .fonts
        .get(usize::from(font))
        .filter(|f| f.index == usize::from(font) && f.charset == 0 && f.alternate_name.is_none())
        .ok_or(Error::Unsupported("inherited bullet ordinary font"))?;
    for other in &c.fonts.fonts {
        charge(budget, 1 + other.name.len())?;
        if other.index != entry.index && other.name.eq_ignore_ascii_case(&entry.name) {
            return Err(Error::Unsupported(
                "inherited bullet duplicate font identity",
            ));
        }
    }
    if entry.name.is_empty() {
        return Err(Error::Unsupported("inherited bullet font identity"));
    }
    if let Some(style) = c.styles.styles.get(10).and_then(Option::as_ref) {
        if style.index != 10
            || style.kind != 2
            || style.base_style.is_some()
            || style.linked_style.is_some()
            || style.revision_marked
            || !style.raw_revision.is_empty()
        {
            return Err(Error::Unsupported(
                "inherited bullet default character style",
            ));
        }
        for group in &style.properties {
            charge(budget, group.bytes.len())?;
            if group.kind != "character" || !group.bytes.is_empty() {
                return Err(Error::Unsupported(
                    "inherited bullet default character properties",
                ));
            }
        }
    }
    let paragraph = c
        .paragraphs
        .spans
        .get(c.paragraphs.spans.partition_point(|p| p.end_cp <= mark))
        .filter(|p| p.start_cp <= start && p.end_cp == mark + 1)
        .ok_or(Error::Unsupported("inherited bullet paragraph ownership"))?;
    style_agrees(
        c,
        usize::from(paragraph.style_index.unwrap_or(0)),
        font,
        budget,
    )?;
    let mut cp = start;
    let first = c
        .formatting
        .character_spans
        .partition_point(|s| s.end_cp <= start);
    for span in &c.formatting.character_spans[first..] {
        if cp > mark {
            break;
        }
        charge(budget, 1)?;
        if span.start_cp > cp || span.end_cp <= cp {
            return Err(Error::Invalid("inherited bullet character coverage"));
        }
        properties_agree(&span.properties, font, budget)?;
        cp = span.end_cp.min(mark + 1);
    }
    if cp != mark + 1 {
        return Err(Error::Invalid(
            "inherited bullet incomplete character coverage",
        ));
    }
    let first = c.formatting.pieces.partition_point(|p| p.end_cp <= start);
    let mut cp = start;
    for piece in &c.formatting.pieces[first..] {
        if cp > mark {
            break;
        }
        charge(budget, 1)?;
        if piece.start_cp > cp || piece.end_cp <= cp || piece.raw_prm != 0 {
            return Err(Error::Unsupported("inherited bullet piece properties"));
        }
        cp = piece.end_cp.min(mark + 1);
    }
    if cp != mark + 1 {
        return Err(Error::Invalid("inherited bullet incomplete piece coverage"));
    }
    charge(budget, level.raw_chpx.len())?;
    properties_agree(&decode_character_modifiers(&level.raw_chpx)?, font, budget)?;
    Ok(Agreement(font))
}
