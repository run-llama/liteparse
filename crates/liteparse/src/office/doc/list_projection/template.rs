use super::{
    Error, LabelLanguage, ListLevel, Result, alphabetic, decode_character_modifiers,
    english_language, roman,
};

#[derive(Clone, Copy)]
pub(super) struct Ancestor {
    value: i32,
    format: u8,
    language: LabelLanguage,
    rtl: bool,
}

impl Ancestor {
    pub(super) fn new(
        level: &ListLevel,
        value: i32,
        mut language: LabelLanguage,
        rtl: bool,
    ) -> Result<Self> {
        language.apply(&decode_character_modifiers(&level.raw_chpx)?)?;
        Ok(Self {
            value,
            format: level.number_format,
            language,
            rtl,
        })
    }

    pub(super) fn render(self, legal: bool) -> Result<String> {
        if self.rtl {
            return Err(Error::Unsupported("list inherited RTL number"));
        }
        number(self.format, self.value, self.language, legal)
    }
}

fn number(format: u8, value: i32, language: LabelLanguage, legal: bool) -> Result<String> {
    if matches!(format, 0x17 | 0xff) {
        return Ok(String::new());
    }
    if value < 0 {
        return Err(Error::Invalid("list inherited negative number"));
    }
    // MS-DOC LVLF preserves ArabicLZ when fLegal converts inherited numbers.
    // ECMA-376 ST_NumberFormat adds a zero only to values one through nine.
    if format == 0x16 {
        return Ok(if (1..=9).contains(&value) {
            format!("0{value}")
        } else {
            value.to_string()
        });
    }
    // ECMA-376 ST_NumberFormat defines direct fullwidth/Thai digits and circles 1–20.
    // Higher circled values use decimal text; zero remains outside this bounded subset.
    if !legal && matches!(format, 14 | 54) {
        return Ok(value
            .to_string()
            .chars()
            .map(|c| {
                char::from_u32(
                    (if format == 14 { 0xff10 } else { 0x0e50 }) + u32::from(c) - u32::from('0'),
                )
                .unwrap()
            })
            .collect());
    }
    if !legal && format == 18 {
        return match value {
            1..=20 => Ok(char::from_u32(0x2460 + value as u32 - 1)
                .unwrap()
                .to_string()),
            21.. => Ok(value.to_string()),
            _ => Err(Error::Unsupported("list enclosed circle zero")),
        };
    }
    // ST_NumberFormat repeats the 48 fullwidth Iroha characters after the last entry.
    if !legal && format == 21 {
        const IROHA: &str = "イロハニホヘトチリヌルヲワカヨタレソツネナラムウヰノオクヤマケフコエテアサキユメミシヱヒモセスン";
        if value == 0 {
            return Err(Error::Unsupported("list iroha zero"));
        }
        return Ok(IROHA
            .chars()
            .nth((value as usize - 1) % 48)
            .unwrap()
            .to_string());
    }
    // Chinese and Japanese counting agree for positive values below one hundred.
    // Larger place-value rules and their zero forms remain outside this bounded subset.
    if !legal && matches!(format, 11 | 37) {
        if !(1..=99).contains(&value) {
            return Err(Error::Unsupported("list East Asian counting range"));
        }
        let digit = |n: i32| "一二三四五六七八九".chars().nth(n as usize - 1).unwrap();
        let mut text = String::new();
        if value >= 20 {
            text.push(digit(value / 10));
        }
        if value >= 10 {
            text.push('十');
        }
        if value % 10 != 0 {
            text.push(digit(value % 10));
        }
        return Ok(text);
    }
    if format > 4 && !(legal && matches!(format, 11 | 14 | 18 | 21 | 37 | 54)) {
        return Err(Error::Unsupported("list inherited number format"));
    }
    match if legal { 0 } else { format } {
        0 => Ok(value.to_string()),
        1 => roman(value),
        2 => Ok(roman(value)?.to_ascii_lowercase()),
        format => {
            if language.hint != 0 || !language.latin.is_some_and(english_language) {
                return Err(Error::Unsupported("list alphabet language"));
            }
            alphabetic(value, format == 3)
        }
    }
}

/// Expand validated XST offsets using the current level and source-owned ancestor numbers.
/// MS-DOC §2.4.6.3 part 2 uses the closest previous paragraph in the same iLfo.
/// Each ancestor retains its effective number format and language, independently of this level.
pub(super) fn render(
    level: &ListLevel,
    index: u8,
    value: i32,
    language: LabelLanguage,
    mut ancestor: impl FnMut(u8) -> Result<String>,
) -> Result<String> {
    let mut positions = [None; 512];
    let mut terminated = false;
    let mut placeholders = 0;
    for &offset in &level.placeholder_offsets {
        if offset == 0 {
            terminated = true;
            continue;
        }
        if terminated {
            return Err(Error::Invalid("list noncontiguous placeholder offsets"));
        }
        let position = usize::from(offset - 1);
        let inherited = *level
            .number_text_utf16
            .get(position)
            .ok_or(Error::Invalid("list placeholder offset"))?;
        if inherited > u16::from(index) || positions[position].is_some() {
            return Err(Error::Invalid("list duplicate or deeper placeholder"));
        }
        positions[position] = Some(inherited as u8);
        placeholders += 1;
    }
    if placeholders == 0 || level.number_text_utf16.len() > positions.len() {
        return Err(Error::Unsupported("list number template placeholders"));
    }
    let mut output = String::new();
    let mut literal = Vec::new();
    for (position, &unit) in level.number_text_utf16.iter().enumerate() {
        if let Some(inherited) = positions[position] {
            append_literal(&mut output, &literal)?;
            literal.clear();
            let text = if inherited == index {
                number(level.number_format, value, language, level.flags & 4 != 0)?
            } else {
                ancestor(inherited)?
            };
            output.push_str(&text);
        } else {
            literal.push(unit);
        }
    }
    append_literal(&mut output, &literal)?;
    Ok(output)
}

fn append_literal(output: &mut String, units: &[u16]) -> Result<()> {
    let text =
        String::from_utf16(units).map_err(|_| Error::Invalid("list number template Unicode"))?;
    if text.chars().any(char::is_control) {
        return Err(Error::Unsupported("list number template literal"));
    }
    output.push_str(&text);
    Ok(())
}
