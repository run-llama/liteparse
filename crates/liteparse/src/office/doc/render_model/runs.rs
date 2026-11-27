//! Source character properties for the first Western-text rendering path.
//! Defaults and operand semantics follow MS-DOC §2.6.1; font defaults come from STSHIF.
//! <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/7022285b-9621-42e9-ad4d-4e02c115ef18>
use crate::office::doc::{
    Error, Result,
    formatting::{PropertyModifier, decode_character_modifiers},
    styles::Stylesheet,
    symbols::FontTable,
};
use serde::Serialize;
mod character_styles;
mod palette;
pub(crate) mod symbols;
pub use symbols::SourceSymbol;

#[derive(Clone, Copy)]
enum SourceControl {
    Ordinary,
    Anchor,
    Symbol,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct ComplexProperties {
    pub font: Option<usize>,
    pub size_half_points: Option<u16>,
    pub bold: bool,
    pub italic: bool,
    pub language: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunProperties {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_symbol: Option<SourceSymbol>,
    pub fonts: [usize; 3],
    pub size_half_points: u16,
    pub bold: bool,
    pub italic: bool,
    #[serde(skip_serializing_if = "hidden_is_false")]
    pub small_caps: bool,
    #[serde(skip_serializing_if = "hidden_is_false")]
    pub all_caps: bool,
    #[serde(skip_serializing_if = "hidden_is_false")]
    pub hidden: bool,
    #[serde(skip_serializing_if = "hidden_is_false")]
    pub underline: bool,
    pub language: u16,
    pub kerning_threshold_half_points: u16,
    pub color_rgb: Option<[u8; 3]>,
    pub shading_rgb: Option<[u8; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight_rgb: Option<[u8; 3]>,
    #[serde(skip_serializing_if = "scale_is_default")]
    pub scale_percent: u16,
    #[serde(skip_serializing_if = "spacing_is_zero")]
    pub character_spacing_twips: i16,
    pub complex: ComplexProperties,
    pub hint: u8,
    pub bidi: bool,
    pub vertical_alignment: u8,
    /// Source modifiers for complex-script properties, whether active or inactive.
    /// The caller qualifies the exact text before selecting active complex properties.
    pub complex_property_evidence: Vec<PropertyModifier>,
    pub applied: Vec<PropertyModifier>,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde skip_serializing_if passes a reference"
)]
fn spacing_is_zero(value: &i16) -> bool {
    *value == 0
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde skip_serializing_if passes a reference"
)]
fn scale_is_default(value: &u16) -> bool {
    *value == 100
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde skip_serializing_if passes a reference"
)]
fn hidden_is_false(hidden: &bool) -> bool {
    !hidden
}

/// Resolve a run only after its caller establishes a non-table, Western-text context.
/// The returned source properties still require per-character font selection and glyph checks.
/// CIstd selects source character styles; other resets and nonempty default styles remain guarded.
pub fn resolve(
    styles: &Stylesheet,
    fonts: &FontTable,
    paragraph_style: usize,
    direct: &[PropertyModifier],
) -> Result<RunProperties> {
    resolve_with_budget(styles, fonts, paragraph_style, direct, &mut 0)
}

/// Resolve run properties while charging selected-style expansion to a shared document budget.
pub fn resolve_with_budget(
    styles: &Stylesheet,
    fonts: &FontTable,
    paragraph_style: usize,
    direct: &[PropertyModifier],
    work: &mut usize,
) -> Result<RunProperties> {
    resolve_control(
        styles,
        fonts,
        paragraph_style,
        direct,
        work,
        SourceControl::Ordinary,
        &[],
    )
}

/// The caller must prove a one-CP U+0008 shape anchor and its effective CFSpec.
pub(crate) fn resolve_shape_anchor(
    styles: &Stylesheet,
    fonts: &FontTable,
    paragraph_style: usize,
    direct: &[PropertyModifier],
    work: &mut usize,
) -> Result<RunProperties> {
    resolve_control(
        styles,
        fonts,
        paragraph_style,
        direct,
        work,
        SourceControl::Anchor,
        &[],
    )
}
/// The caller proves a one-CP U+0002 note or U+0005 comment marker and its effective CFSpec.
pub(crate) fn resolve_owned_note_marker(
    styles: &Stylesheet,
    fonts: &FontTable,
    paragraph_style: usize,
    direct: &[PropertyModifier],
    work: &mut usize,
) -> Result<RunProperties> {
    resolve_control(
        styles,
        fonts,
        paragraph_style,
        direct,
        work,
        SourceControl::Anchor,
        &[],
    )
}
/// Resolve source-owned table differences before paragraph and character overrides.
/// Conditional role qualification belongs to the supplied cell ownership proof.
pub(crate) fn resolve_table_text(
    styles: &Stylesheet,
    fonts: &FontTable,
    paragraph_style: usize,
    direct: &[PropertyModifier],
    table: &[PropertyModifier],
    work: &mut usize,
) -> Result<RunProperties> {
    resolve_control(
        styles,
        fonts,
        paragraph_style,
        direct,
        work,
        SourceControl::Ordinary,
        table,
    )
}
fn resolve_control(
    styles: &Stylesheet,
    fonts: &FontTable,
    paragraph_style: usize,
    direct: &[PropertyModifier],
    work: &mut usize,
    control: SourceControl,
    table: &[PropertyModifier],
) -> Result<RunProperties> {
    let character = styles
        .styles
        .get(10)
        .and_then(Option::as_ref)
        .ok_or(Error::Invalid("render default character style"))?;
    if character.index != 10
        || character.kind != 2
        || character.base_style.is_some()
        || character.linked_style.is_some()
        || character.revision_marked
        || !character.raw_revision.is_empty()
        || character
            .properties
            .iter()
            .any(|g| g.kind != "character" || !g.bytes.is_empty())
    {
        return Err(Error::Unsupported(
            "render default character style properties",
        ));
    }
    let defaults = [
        crate::office::doc::u16_at(&styles.raw_header, 12)? as usize,
        crate::office::doc::u16_at(&styles.raw_header, 14)? as usize,
        crate::office::doc::u16_at(&styles.raw_header, 16)? as usize,
    ];
    let mut state = RunProperties {
        source_symbol: None,
        fonts: defaults,
        size_half_points: 20,
        bold: false,
        italic: false,
        small_caps: false,
        all_caps: false,
        underline: false,
        hidden: false,
        language: 0,
        kerning_threshold_half_points: 0,
        color_rgb: None,
        shading_rgb: None,
        highlight_rgb: None,
        scale_percent: 100,
        character_spacing_twips: 0,
        complex: ComplexProperties::default(),
        hint: 0,
        bidi: false,
        vertical_alignment: 0,
        complex_property_evidence: Vec::new(),
        applied: Vec::new(),
    };
    // The caller has already selected conditional cell roles. These differences
    // precede paragraph styles and direct formatting under MS-DOC §2.4.6.6.
    for property in table {
        validate_font(property, fonts.fonts.len())?;
        character_styles::charge_state(work, &state)?;
        let baseline = state.clone();
        apply(&mut state, property, &baseline)?;
    }
    let layers = style_layers_with_budget(styles, paragraph_style, "character", work)?;
    // StdfBase delegates basedOn to ECMA-376: ancestor properties precede
    // derived properties. Each style's relative toggles use its inherited state.
    for layer in layers.iter().rev() {
        character_styles::charge_state(work, &state)?;
        let baseline = state.clone();
        for property in layer {
            validate_font(property, fonts.fonts.len())?;
            apply(&mut state, property, &baseline)?;
        }
    }
    character_styles::charge_state(work, &state)?;
    character_styles::charge_state(work, &state)?;
    let paragraph_baseline = state.clone();
    let mut baseline = state.clone();
    if direct.len() > 262_144 {
        return Err(Error::Limit("render direct character properties"));
    }
    for property in direct {
        validate_font(property, fonts.fonts.len())?;
        *work = work
            .checked_add(property.operand.len())
            .filter(|n| *n <= 8 * 1024 * 1024)
            .ok_or(Error::Limit("render character property bytes"))?;
        if matches!(control, SourceControl::Symbol) && property.sprm == 0x6a09 {
            if property.operand.len() != 4 {
                return Err(Error::Invalid("render source symbol operand"));
            }
            state.applied.push(property.clone());
        } else if (matches!(control, SourceControl::Anchor)
            && matches!(property.sprm, 0x0855 | 0x6a03))
            || (matches!(control, SourceControl::Symbol) && property.sprm == 0x0855)
        {
            if !(property.sprm == 0x0855
                && matches!(property.operand.as_slice(), [0 | 1 | 128 | 129])
                || property.sprm == 0x6a03 && property.operand.len() == 4)
            {
                return Err(Error::Invalid("render shape anchor property"));
            }
            // CFSpec is established by the source anchor or symbol proof.
            // CPicLocation is retained here only for qualified anchors.
            // Preserve these operands through character-style selection.
            state.applied.push(property.clone());
        } else if property.sprm == 0x4a30 {
            character_styles::select(
                &mut state,
                &paragraph_baseline,
                styles,
                fonts,
                property,
                work,
            )?;
            character_styles::charge_state(work, &state)?;
            baseline = state.clone();
        } else {
            apply(&mut state, property, &baseline)?;
        }
    }
    for index in state.fonts {
        if fonts.fonts.get(index).is_none_or(|f| f.index != index) {
            return Err(Error::Invalid("render font index"));
        }
    }
    // No-proofing language remains explicit for paragraph-mark qualification.
    // Other accepted primary-language IDs use Western, Greek, Cyrillic or East Asian text.
    if state.language != 0x0400
        && !matches!(
            state.language & 0x3ff,
            0x04 | 0x08
                | 0x19
                | 0x2f
                | 0x06
                | 0x07
                | 0x09
                | 0x0a
                | 0x0b
                | 0x0c
                | 0x0f
                | 0x10
                | 0x11
                | 0x12
                | 0x13
                | 0x14
                | 0x16
                | 0x1d
        )
    {
        return Err(Error::Unsupported("render Western language qualification"));
    }
    Ok(state)
}

fn validate_font(property: &PropertyModifier, count: usize) -> Result<()> {
    if matches!(property.sprm, 0x4a4f..=0x4a51 | 0x4a5e)
        && (property.operand.len() != 2
            || crate::office::doc::u16_at(&property.operand, 0)? as usize >= count)
    {
        return Err(Error::Invalid("render source font index"));
    }
    Ok(())
}

pub(super) fn style_layers(
    styles: &Stylesheet,
    index: usize,
    group: &str,
) -> Result<Vec<Vec<PropertyModifier>>> {
    style_layers_with_budget(styles, index, group, &mut 0)
}

fn style_layers_with_budget(
    styles: &Stylesheet,
    index: usize,
    group: &str,
    work: &mut usize,
) -> Result<Vec<Vec<PropertyModifier>>> {
    let mut next = Some(index);
    let mut seen = Vec::new();
    let mut layers = Vec::new();
    while let Some(index) = next {
        if seen.contains(&index) || seen.len() >= 4094 {
            return Err(Error::Invalid("render style ancestry"));
        }
        character_styles::charge(work, 1)?;
        seen.push(index);
        let style = styles
            .styles
            .get(index)
            .and_then(Option::as_ref)
            .ok_or(Error::Invalid("render paragraph style"))?;
        if style.index != index
            || style.kind != 1
            || style.revision_marked
            || !style.raw_revision.is_empty()
        {
            return Err(Error::Unsupported("render style identity or revisions"));
        }
        // A linked character style is the run-application partner of this
        // paragraph style. Only basedOn contributes inherited properties.
        // MS-DOC StdfPost2000 delegates this pairing to ECMA-376 §17.7.4.6.
        if let Some(linked) = style.linked_style {
            let partner = styles
                .styles
                .get(linked)
                .and_then(Option::as_ref)
                .ok_or(Error::Invalid("render linked style index"))?;
            if partner.index != linked || partner.kind != 2 {
                return Err(Error::Invalid("render linked character style"));
            }
        }
        let mut properties = Vec::new();
        for source in &style.properties {
            *work = work
                .checked_add(source.bytes.len() + 1)
                .filter(|n| *n <= 8 * 1024 * 1024)
                .ok_or(Error::Limit("render style work"))?;
            if !matches!(source.kind, "paragraph" | "character") {
                return Err(Error::Unsupported("render style property group"));
            }
            let bytes = if source.kind == "paragraph" {
                if crate::office::doc::u16_at(&source.bytes, 0)? as usize != index {
                    return Err(Error::Invalid("render style PAPX identity"));
                }
                &source.bytes[2..]
            } else {
                &source.bytes
            };
            if source.kind == group {
                properties.extend(if group == "paragraph" {
                    super::paragraphs::decode(bytes)?
                } else {
                    decode_character_modifiers(bytes)?
                });
            }
        }
        layers.push(properties);
        next = style.base_style;
    }
    Ok(layers)
}

fn apply(
    state: &mut RunProperties,
    property: &PropertyModifier,
    baseline: &RunProperties,
) -> Result<()> {
    let operand = &property.operand;
    let u16_value = || {
        if operand.len() != 2 {
            return Err(Error::Invalid("render character operand size"));
        }
        crate::office::doc::u16_at(operand, 0)
    };
    let toggle = |baseline: bool| -> Result<bool> {
        match operand.as_slice() {
            [0] => Ok(false),
            [1] => Ok(true),
            [128] => Ok(baseline),
            [129] => Ok(!baseline),
            _ => Err(Error::Invalid("render character toggle")),
        }
    };
    match property.sprm {
        0x4a4f..=0x4a51 => state.fonts[(property.sprm - 0x4a4f) as usize] = u16_value()? as usize,
        0x4a43 => {
            let size = u16_value()?;
            if !(2..=3276).contains(&size) {
                return Err(Error::Invalid("render font size"));
            }
            state.size_half_points = size;
        }
        0x2a3e => {
            state.underline = match operand.as_slice() {
                [0] => false,
                [1] => true,
                [_] => return Err(Error::Unsupported("render underline style")),
                _ => return Err(Error::Invalid("render underline operand")),
            };
        }
        0x8840 => {
            let value = u16_value()? as i16;
            if !(-31680..=31680).contains(&value) {
                return Err(Error::Invalid("render character spacing"));
            }
            state.character_spacing_twips = value;
        }
        // Animated decorations retain their source operand for the renderer's
        // explicit static-output omission report.
        0x2859 => match operand.as_slice() {
            [0..=6] => {}
            _ => return Err(Error::Invalid("render text animation operand")),
        },
        // Font substitution hints remain source evidence; actual selected fonts
        // and glyph coverage are independently validated by the renderer.
        0x2a86 => match operand.as_slice() {
            [0 | 1 | 2 | 4] => {}
            _ => return Err(Error::Invalid("render font substitution hint")),
        },
        0x2a0c => state.highlight_rgb = palette::resolve(operand)?,
        0x4852 => {
            let value = u16_value()?;
            if !(1..=600).contains(&value) {
                return Err(Error::Invalid("render character scale"));
            }
            state.scale_percent = value;
        }
        0x2a42 => {
            state.color_rgb = palette::resolve(operand)?;
        }
        0x6870 => {
            state.color_rgb = match operand.as_slice() {
                [r, g, b, 0] => Some([*r, *g, *b]),
                [_, _, _, 255] => None,
                _ => return Err(Error::Invalid("render RGB text color")),
            };
        }
        0x2a48 => {
            state.vertical_alignment = match operand.as_slice() {
                [v @ 0..=2] => *v,
                _ => return Err(Error::Invalid("render superscript or subscript")),
            };
        }
        0x4866 => {
            let value = u16_value()?;
            if value == 0xffff {
                state.shading_rgb = None;
            } else if value >> 10 == 0 && value & 31 <= 16 {
                state.shading_rgb = match (value >> 5) & 31 {
                    8 => Some([255; 3]),
                    1 => Some([0; 3]),
                    _ => return Err(Error::Unsupported("render shading palette")),
                };
            } else {
                return Err(Error::Unsupported("render patterned shading"));
            }
        }
        0xca71 => {
            if operand.len() != 11
                || operand[0] != 10
                || !matches!(operand[4], 0 | 255)
                || !matches!(operand[8], 0 | 255)
            {
                return Err(Error::Invalid("render shading operand"));
            }
            if crate::office::doc::u16_at(operand, 9)? != 0 {
                return Err(Error::Unsupported("render patterned shading"));
            }
            state.shading_rgb = (operand[8] == 0).then_some([operand[5], operand[6], operand[7]]);
        }
        0x484b => {
            let value = u16_value()?;
            if value > 3276 {
                return Err(Error::Invalid("render kerning threshold"));
            }
            state.kerning_threshold_half_points = value;
        }
        0x0835 => state.bold = toggle(baseline.bold)?,
        0x0836 => state.italic = toggle(baseline.italic)?,
        0x083a => state.small_caps = toggle(baseline.small_caps)?,
        0x083b => state.all_caps = toggle(baseline.all_caps)?,
        0x083c => state.hidden = toggle(baseline.hidden)?,
        0x486d | 0x4873 => state.language = u16_value()?,
        0x486e | 0x4874 => {
            u16_value()?;
            state.complex_property_evidence.push(property.clone());
        }
        0x485f => {
            state.complex.language = Some(u16_value()?);
            state.complex_property_evidence.push(property.clone());
        }
        0x4a5e => {
            state.complex.font = Some(u16_value()? as usize);
            state.complex_property_evidence.push(property.clone());
        }
        0x4a61 => {
            if !(2..=3276).contains(&u16_value()?) {
                return Err(Error::Invalid("render complex font size"));
            }
            state.complex.size_half_points = Some(u16_value()?);
            state.complex_property_evidence.push(property.clone());
        }
        0x085c | 0x085d => {
            if property.sprm == 0x085c {
                state.complex.bold = toggle(baseline.complex.bold)?;
            } else {
                state.complex.italic = toggle(baseline.complex.italic)?;
            }
            state.complex_property_evidence.push(property.clone());
        }
        0x286f => match operand.as_slice() {
            [value @ 0..=2] => state.hint = *value,
            _ => return Err(Error::Unsupported("render character hint")),
        },
        0x085a => state.bidi = toggle(baseline.bidi)?,
        // An explicit false routing property has a strict Bool8 operand.
        0x0882 if operand.as_slice() == [0] => {}
        // Proofing suppression affects spelling/grammar checks. Retain its
        // source toggle without changing the resolved visual run properties.
        0x0875 if matches!(operand.as_slice(), [0 | 1 | 128 | 129]) => {}
        // Save-session identifiers retain edit provenance; revision visibility
        // is independently qualified before constructing render runs.
        0x6815..=0x6817 if operand.len() == 4 => {}
        _ => return Err(Error::Unsupported("render character property")),
    }
    state.applied.push(property.clone());
    Ok(())
}

/// Selects the source font slot for unambiguous East Asian text and hinted punctuation.
pub fn font_slot(c: char, hint: u8) -> usize {
    if matches!(c, '\u{b}' | '\u{c}' | '\u{e}' | '\u{1f}') {
        2
    } else if c.is_ascii() {
        0
    } else if east_asian_character(c as u32) || hint == 1 {
        1
    } else {
        2
    }
}

/// East Asian characters admitted through the existing Unicode shaping path.
pub fn east_asian_character(c: u32) -> bool {
    matches!(c, 0x1100..=0x11ff | 0x2e80..=0x2fdf | 0x3000..=0x31ff
        | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xac00..=0xd7af
        | 0xf900..=0xfaff | 0xff01..=0xffef | 0x20000..=0x2ffff)
}

#[cfg(test)]
mod tests;
