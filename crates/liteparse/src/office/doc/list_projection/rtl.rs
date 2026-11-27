//! Logical RTL labels require source font selection before visual shaping is deferred.
//! MS-DOC §§2.4.6.5–6, 2.6.1 and 2.9.272 define style, toggle and font provenance.
use super::*;
use crate::office::doc::formatting::ToggleOperand;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct RtlLabelProperties {
    /// Font used by the generated number; literal punctuation may select ASCII properties.
    pub font_index: usize,
    pub ascii_literal_font_index: Option<usize>,
    pub language_id: Option<u16>,
    pub character_right_to_left: bool,
    pub complex_script_hint: bool,
    pub complex_bold: bool,
    pub complex_italic: bool,
}

#[derive(Clone, Copy, Default)]
struct Properties {
    ascii: Option<u16>,
    complex_font: Option<u16>,
    latin: Option<u16>,
    complex_language: Option<u16>,
    hint: u8,
    rtl: Option<bool>,
    bold: bool,
    italic: bool,
}

fn toggle(p: &PropertyModifier, baseline: Option<bool>) -> Result<Option<bool>> {
    Ok(match p.toggle()? {
        Some(ToggleOperand::Off) => Some(false),
        Some(ToggleOperand::On) => Some(true),
        Some(ToggleOperand::Style) => baseline,
        Some(ToggleOperand::NotStyle) => baseline.map(|v| !v),
        None => return Err(Error::Invalid("list RTL toggle")),
    })
}

pub(super) fn visible(p: &PropertyModifier) -> bool {
    super::visible_modifier(p)
        || (matches!(p.sprm, 0x085a | 0x085c | 0x085d)
            && matches!(p.operand.as_slice(), [0 | 1 | 128 | 129]))
}

impl Properties {
    fn apply(&mut self, properties: &[PropertyModifier], baseline: Self) -> Result<()> {
        for p in properties {
            if !visible(p) {
                return Err(Error::Unsupported("list RTL character property"));
            }
            match p.sprm {
                0x4a4f => self.ascii = Some(u16_at(&p.operand, 0)?),
                0x4a5e => self.complex_font = Some(u16_at(&p.operand, 0)?),
                0x486d | 0x4873 => self.latin = Some(u16_at(&p.operand, 0)?),
                0x485f => self.complex_language = Some(u16_at(&p.operand, 0)?),
                0x286f => {
                    self.hint = bytes(&p.operand, 0, 1)?[0];
                    if !matches!(self.hint, 0..=2 | 255) {
                        return Err(Error::Invalid("list RTL font hint"));
                    }
                }
                0x085a => self.rtl = toggle(p, baseline.direction())?,
                0x085c => self.bold = toggle(p, Some(baseline.bold))?.unwrap(),
                0x085d => self.italic = toggle(p, Some(baseline.italic))?.unwrap(),
                _ => {}
            }
        }
        Ok(())
    }

    fn direction(self) -> Option<bool> {
        self.rtl.or_else(|| {
            let language = if self.hint == 2 {
                self.complex_language
            } else {
                self.latin
            }?;
            if arabic_language(language) || matches!(language, 0x040d | 0x0429 | 0x0420) {
                Some(true)
            } else if english_language(language) || french_language(language) {
                Some(false)
            } else {
                None
            }
        })
    }
}

fn arabic_language(language: u16) -> bool {
    language & 0x03ff == 1 && (1..=16).contains(&(language >> 10))
}

/// Resolve outside-table label properties at the physical terminating mark.
/// Conflicting inherited language values and repeated toggles retain unresolved merge ordering.
pub(super) fn resolve(
    styles: &Stylesheet,
    style_index: usize,
    direct: &[PropertyModifier],
    level: &ListLevel,
    fonts: &FontTable,
    budget: &mut usize,
) -> Result<RtlLabelProperties> {
    let mut state = Properties {
        ascii: styles
            .raw_header
            .get(12..14)
            .map(|b| u16::from_le_bytes([b[0], b[1]])),
        complex_font: styles
            .raw_header
            .get(18..20)
            .map(|b| u16::from_le_bytes([b[0], b[1]])),
        ..Default::default()
    };
    let mut current = Some(style_index);
    let mut seen = BTreeSet::new();
    let mut operators = BTreeMap::new();
    let mut inherited = Vec::new();
    while let Some(index) = current {
        if seen.len() >= 128 || !seen.insert(index) {
            return Err(Error::Invalid("list RTL style inheritance"));
        }
        let style = styles
            .styles
            .get(index)
            .and_then(Option::as_ref)
            .filter(|s| s.kind == 1 && !s.revision_marked && s.raw_revision.is_empty())
            .ok_or(Error::Unsupported("list RTL paragraph style"))?;
        for group in style.properties.iter().filter(|g| g.kind == "character") {
            *budget = budget
                .checked_sub(group.bytes.len())
                .ok_or(Error::Limit("list RTL property bytes"))?;
            for property in decode_character_modifiers(&group.bytes)? {
                let identity = match property.sprm {
                    0x4873 => 0x486d,
                    0x4874 => 0x486e,
                    sprm => sprm,
                };
                if let Some(previous) = operators.get(&identity) {
                    if matches!(identity, 0x486d | 0x486e) && previous == &property.operand {
                        continue;
                    }
                    return Err(Error::Unsupported("list RTL repeated inherited property"));
                }
                operators.insert(identity, property.operand.clone());
                inherited.push(property);
            }
        }
        current = style.base_style;
    }
    let default = state;
    state.apply(&inherited, default)?;
    let baseline = state;
    *budget = budget
        .checked_sub(direct.iter().map(|p| p.operand.len() + 2).sum())
        .ok_or(Error::Limit("list RTL property bytes"))?;
    state.apply(direct, baseline)?;
    *budget = budget
        .checked_sub(level.raw_chpx.len())
        .ok_or(Error::Limit("list RTL property bytes"))?;
    state.apply(&decode_character_modifiers(&level.raw_chpx)?, baseline)?;
    let rtl = state
        .direction()
        .ok_or(Error::Unsupported("list RTL character direction"))?;
    if matches!(state.hint, 1 | 255) {
        return Err(Error::Unsupported("list RTL font selection"));
    }
    let forced_complex = rtl || state.hint == 2;
    let complex = forced_complex || level.number_format == 46;
    let font = if complex {
        state.complex_font
    } else {
        state.ascii
    }
    .ok_or(Error::Unsupported("list RTL font unavailable"))? as usize;
    let font = fonts
        .fonts
        .get(font)
        .filter(|f| f.index == font)
        .ok_or(Error::Invalid("list RTL font index"))?;
    if font.charset == 2 {
        return Err(Error::Unsupported("list RTL symbol font"));
    }
    // Mixed-script literal templates need per-character font selection; keep that gap explicit.
    if !forced_complex && level.number_text_utf16.iter().any(|u| *u > 0x7f) {
        return Err(Error::Unsupported("list RTL mixed template fonts"));
    }
    let ascii_literal_font_index = if !forced_complex
        && level
            .number_text_utf16
            .iter()
            .any(|u| (0x20..=0x7f).contains(u))
    {
        let index = state
            .ascii
            .ok_or(Error::Unsupported("list RTL literal font unavailable"))?
            as usize;
        let font = fonts
            .fonts
            .get(index)
            .filter(|f| f.index == index)
            .ok_or(Error::Invalid("list RTL literal font index"))?;
        if font.charset == 2 {
            return Err(Error::Unsupported("list RTL literal symbol font"));
        }
        Some(index)
    } else {
        None
    };
    Ok(RtlLabelProperties {
        font_index: font.index,
        ascii_literal_font_index,
        language_id: if complex {
            state.complex_language
        } else {
            state.latin
        },
        character_right_to_left: rtl,
        complex_script_hint: state.hint == 2,
        complex_bold: state.bold,
        complex_italic: state.italic,
    })
}

/// Word's Arabic alphabetic numbering repeats a letter after each 28-number cycle.
/// MS-OI29500 §17.18.59(h,x) requires U+200C after the whole sequence number.
pub(super) fn arabic_alpha(value: i32) -> Result<String> {
    const ALPHABET: [char; 28] = [
        'أ', 'ب', 'ت', 'ث', 'ج', 'ح', 'خ', 'د', 'ذ', 'ر', 'ز', 'س', 'ش', 'ص', 'ض', 'ط', 'ظ', 'ع',
        'غ', 'ف', 'ق', 'ك', 'ل', 'م', 'ن', 'ه', 'و', 'ي',
    ];
    if !(1..=28 * 512).contains(&value) {
        return Err(Error::Unsupported("list Arabic alphabetic range"));
    }
    let zero = (value - 1) as usize;
    let mut text = ALPHABET[zero % 28].to_string().repeat(zero / 28 + 1);
    text.push('\u{200c}');
    Ok(text)
}

// MS-LCID assigned French language IDs use the French LTR language default.
fn french_language(language: u16) -> bool {
    matches!(
        language,
        0x000c
            | 0x040c
            | 0x080c
            | 0x0c0c
            | 0x100c
            | 0x140c
            | 0x180c
            | 0x1c0c
            | 0x200c
            | 0x240c
            | 0x280c
            | 0x2c0c
            | 0x300c
            | 0x340c
            | 0x380c
            | 0x3c0c
    )
}
