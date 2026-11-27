//! Bounded EQ overstrike syntax with literal operands and optional vertical shifts.
use crate::office::doc::{Error, Result};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Composite {
    pub operator: &'static str,
    pub alignment: &'static str,
    pub projection: &'static str,
    pub operands: Vec<Operand>,
}
#[derive(Debug, Serialize)]
pub struct Operand {
    pub text: String,
    pub source_range: [usize; 2],
    pub vertical_shift_points: i16,
    pub projected_utf8_range: Option<[usize; 2]>,
}

/// Overstrike operands share horizontal space; source order is retained separately
/// from their optional vertical displacement. No arithmetic is evaluated.
pub(super) fn parse(code: &str, source_cp: usize) -> Result<Composite> {
    if code.len() > 16_384 {
        return Err(Error::Limit("composite equation instruction"));
    }
    let expression = code.trim();
    let rest = expression
        .get(..2)
        .filter(|s| s.eq_ignore_ascii_case("EQ"))
        .and_then(|_| expression.get(2..))
        .filter(|s| s.starts_with(char::is_whitespace))
        .ok_or(Error::Unsupported("composite equation identity"))?
        .trim_start();
    let mut rest = rest
        .get(..2)
        .filter(|s| s.eq_ignore_ascii_case("\\o"))
        .and_then(|_| rest.get(2..))
        .ok_or(Error::Unsupported("composite equation operator"))?
        .trim_start();
    let mut alignment = "center";
    if rest.starts_with('\\') {
        alignment = match rest.get(..3).map(str::to_ascii_lowercase).as_deref() {
            Some("\\al") => "left",
            Some("\\ac") => "center",
            Some("\\ar") => "right",
            Some("\\ad") => "justified",
            _ => return Err(Error::Unsupported("composite equation alignment")),
        };
        rest = rest[3..].trim_start();
    }
    let content = rest
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .ok_or(Error::Invalid("composite equation envelope"))?;
    let content_offset = content.as_ptr() as usize - code.as_ptr() as usize;
    let mut ranges = Vec::new();
    let mut depth = 0;
    let mut begin = 0;
    for (i, c) in content.char_indices() {
        match c {
            '(' => {
                depth += 1;
                if depth > 1 {
                    return Err(Error::Unsupported("nested composite equation"));
                }
            }
            ')' => {
                if depth == 0 {
                    return Err(Error::Invalid("composite equation parentheses"));
                }
                depth -= 1;
            }
            ',' if depth == 0 => {
                ranges.push(begin..i);
                begin = i + 1;
            }
            c if c.is_control() => return Err(Error::Invalid("composite equation control")),
            _ => {}
        }
    }
    if depth != 0 {
        return Err(Error::Invalid("composite equation parentheses"));
    }
    ranges.push(begin..content.len());
    if !(2..=16).contains(&ranges.len()) {
        return Err(Error::Unsupported("composite equation operand count"));
    }
    let mut operands = Vec::new();
    for range in ranges {
        let original = &content[range.clone()];
        let mut value = original;
        let mut shift = 0;
        if original.trim_start().starts_with('\\') {
            let start = original.trim_start();
            let tail = start
                .get(..2)
                .filter(|s| s.eq_ignore_ascii_case("\\s"))
                .and_then(|_| start.get(2..))
                .ok_or(Error::Unsupported("composite operand operator"))?
                .trim_start();
            let direction = match tail.get(..3).map(str::to_ascii_lowercase).as_deref() {
                Some("\\up") => 1,
                Some("\\do") => -1,
                _ => return Err(Error::Unsupported("composite vertical shift")),
            };
            let tail = tail[3..].trim_start();
            let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 || digits > 3 {
                return Err(Error::Invalid("composite vertical distance"));
            }
            let amount: i16 = tail[..digits]
                .parse()
                .map_err(|_| Error::Invalid("composite vertical distance"))?;
            if amount > 127 {
                return Err(Error::Unsupported("composite vertical distance"));
            }
            shift = direction * amount;
            value = tail[digits..]
                .trim()
                .strip_prefix('(')
                .and_then(|s| s.strip_suffix(')'))
                .ok_or(Error::Invalid("composite shifted literal"))?;
        }
        if value.trim().is_empty()
            || value
                .chars()
                .any(|c| c.is_control() || matches!(c, '\\' | '(' | ')' | ',' | ';' | '"'))
        {
            return Err(Error::Unsupported("composite nonliteral operand"));
        }
        let byte_start =
            content_offset + range.start + (value.as_ptr() as usize - original.as_ptr() as usize);
        let start = source_cp
            .checked_add(code[..byte_start].encode_utf16().count())
            .ok_or(Error::Limit("composite source range"))?;
        let end = start
            .checked_add(value.encode_utf16().count())
            .ok_or(Error::Limit("composite source range"))?;
        operands.push(Operand {
            text: value.to_owned(),
            source_range: [start, end],
            vertical_shift_points: shift,
            projected_utf8_range: None,
        });
    }
    Ok(Composite {
        operator: "overstrike",
        alignment,
        projection: "source_operands_in_order_overlay_and_vertical_placement_unresolved",
        operands,
    })
}

#[cfg(test)]
mod tests;
