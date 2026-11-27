use crate::office::doc::{Error, Result};

#[derive(Debug)]
pub(super) struct Instruction<'a> {
    pub code: u16,
    pub font: &'a str,
}

/// Decode numeric ANSI SYMBOL instructions with an explicit quoted font.
/// Encoding and presentation switches outside this grammar retain diagnostics.
pub(super) fn parse(text: &str) -> Result<Instruction<'_>> {
    if text.len() > 1024 || !text.is_ascii() {
        return Err(Error::Unsupported("symbol field instruction repertoire"));
    }
    let text = text.trim_matches([' ', '\t']);
    let mut words = text.splitn(3, [' ', '\t']).filter(|s| !s.is_empty());
    if !words
        .next()
        .is_some_and(|s| s.eq_ignore_ascii_case("SYMBOL"))
    {
        return Err(Error::Invalid("symbol field keyword"));
    }
    // Tokenize the numeric argument separately so repeated whitespace remains valid.
    let tail = text[6..].trim_start_matches([' ', '\t']);
    let end = tail
        .find([' ', '\t'])
        .ok_or(Error::Unsupported("symbol field explicit font"))?;
    let number = &tail[..end];
    let code = if let Some(hex) = number
        .strip_prefix("0x")
        .or_else(|| number.strip_prefix("0X"))
    {
        if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Invalid("symbol field hexadecimal code"));
        }
        u16::from_str_radix(hex, 16)
    } else {
        if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error::Unsupported("symbol field numeric code"));
        }
        number.parse::<u16>()
    }
    .map_err(|_| Error::Invalid("symbol field code range"))?;
    let suffix = tail[end..].trim_start_matches([' ', '\t']);
    let font = suffix
        .strip_prefix("\\f")
        .ok_or(Error::Unsupported("symbol field switches"))?;
    if !font.starts_with([' ', '\t']) {
        return Err(Error::Invalid("symbol field font switch"));
    }
    let font = font
        .trim_start_matches([' ', '\t'])
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .filter(|s| !s.is_empty() && !s.contains(['"', '\\', '\r', '\n']))
        .ok_or(Error::Unsupported("symbol field quoted font"))?;
    if !(0x20..=0xff).contains(&code) {
        return Err(Error::Unsupported("symbol field ANSI code range"));
    }
    Ok(Instruction { code, font })
}

#[cfg(test)]
mod tests;
