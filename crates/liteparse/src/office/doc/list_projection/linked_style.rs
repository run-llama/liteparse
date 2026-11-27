use super::{
    Error, LabelLanguage, ListContext, ListLevel, PropertyModifier, Result, Selection, Stylesheet,
    decode_character_modifiers, u16_at, visible_modifier,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Characters {
    ascii_font: Option<u16>,
    language: LabelLanguage,
}

impl Characters {
    fn apply(&mut self, properties: &[PropertyModifier]) -> Result<()> {
        for property in properties {
            if !visible_modifier(property) {
                return Err(Error::Unsupported("linked list character properties"));
            }
            match property.sprm {
                0x4a4f => self.ascii_font = Some(u16_at(&property.operand, 0)?),
                0x083a | 0x083b if property.operand != [0] => {
                    return Err(Error::Unsupported("linked list character case"));
                }
                _ => {}
            }
        }
        self.language.apply(properties)
    }
}

pub(super) struct Cache {
    styles: BTreeMap<usize, Option<Characters>>,
    default: Option<Characters>,
    bytes_left: usize,
}

impl Cache {
    pub(super) fn new() -> Self {
        Self {
            styles: BTreeMap::new(),
            default: None,
            bytes_left: 8 * 1024 * 1024,
        }
    }

    fn default_characters(&mut self, styles: &Stylesheet) -> Result<Characters> {
        if let Some(value) = self.default {
            return Ok(value);
        }
        let value = Characters::default();
        if let Some(style) = styles.styles.get(10).and_then(Option::as_ref) {
            if style.kind != 2
                || style.base_style.is_some()
                || style.revision_marked
                || !style.raw_revision.is_empty()
            {
                return Err(Error::Unsupported("linked list default character style"));
            }
            for group in &style.properties {
                self.bytes_left = self
                    .bytes_left
                    .checked_sub(group.bytes.len())
                    .ok_or(Error::Limit("linked list default properties"))?;
                if group.kind != "character" || !group.bytes.is_empty() {
                    return Err(Error::Unsupported("linked list default property group"));
                }
            }
        }
        self.default = Some(value);
        Ok(value)
    }

    fn baseline(&mut self, styles: &Stylesheet, index: usize, depth: usize) -> Result<Characters> {
        if depth > 128 {
            return Err(Error::Unsupported("linked list style ancestry"));
        }
        if let Some(value) = self.styles.get(&index) {
            return value.ok_or(Error::Unsupported("linked list style properties"));
        }
        // An in-progress entry also rejects cycles. Invalid baselines stay cached.
        self.styles.insert(index, None);
        let style = styles
            .styles
            .get(index)
            .and_then(Option::as_ref)
            .filter(|s| {
                s.index == index && s.kind == 1 && !s.revision_marked && s.raw_revision.is_empty()
            })
            .ok_or(Error::Unsupported("linked list paragraph style"))?;
        let mut value = if let Some(parent) = style.base_style {
            self.baseline(styles, parent, depth + 1)?
        } else {
            self.default_characters(styles)?
        };
        for group in &style.properties {
            self.bytes_left = self
                .bytes_left
                .checked_sub(group.bytes.len())
                .ok_or(Error::Limit("linked list style properties"))?;
            match group.kind {
                "paragraph" if u16_at(&group.bytes, 0)? as usize == index => {}
                "character" => value.apply(&decode_character_modifiers(&group.bytes)?)?,
                _ => return Err(Error::Unsupported("linked list style property group")),
            }
        }
        self.styles.insert(index, Some(value));
        Ok(value)
    }

    /// The linked style is already the physical paragraph's resolved style.
    /// Both precedence paths must preserve language and use ordinary Latin character mappings.
    /// Different Latin font faces preserve the ASCII numeric label text; typography is separate.
    /// LVL character properties are last; reset operators remain unsupported.
    pub(super) fn qualify(
        &mut self,
        c: &ListContext<'_>,
        index: usize,
        baseline: &Selection,
        actual: &Selection,
        level: &ListLevel,
        direct: &[PropertyModifier],
    ) -> Result<LabelLanguage> {
        if baseline.ilfo != actual.ilfo
            || baseline.level != actual.level
            || baseline.in_table
            || actual.in_table
            || baseline.right_to_left
            || actual.right_to_left
            || !baseline.visible_style
            || !actual.visible_style
            || !matches!(level.number_format, 0..=4 | 0x16)
            || level.number_text_utf16.iter().any(|unit| *unit > 0x7f)
        {
            return Err(Error::Unsupported("linked list paragraph baseline"));
        }
        let style = self.baseline(c.styles, index, 0)?;
        let modifiers = decode_character_modifiers(&level.raw_chpx)?;
        let mut expected = style;
        expected.apply(&modifiers)?;
        let mut physical = style;
        physical.apply(direct)?;
        physical.apply(&modifiers)?;
        if expected.language != physical.language {
            return Err(Error::Unsupported("linked list direct character conflict"));
        }
        for characters in [expected, physical] {
            characters
                .ascii_font
                .or_else(|| {
                    (c.font_defaults_preserved == Some(true))
                        .then(|| u16_at(&c.styles.raw_header, 12).ok())
                        .flatten()
                })
                .and_then(|i| {
                    c.fonts
                        .fonts
                        .get(i as usize)
                        .filter(|f| f.index == i as usize)
                })
                .filter(|f| f.charset == 0)
                .ok_or(Error::Unsupported("linked list numeric font"))?;
        }
        if expected.language.hint != 0 {
            return Err(Error::Unsupported("linked list font selection"));
        }
        Ok(style.language)
    }
}
