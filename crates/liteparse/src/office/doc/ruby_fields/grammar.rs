use crate::office::doc::{Error, Result};
use std::ops::Range;

pub(super) struct Parsed<'a> {
    pub base: Range<usize>,
    pub guide: Range<usize>,
    pub font: &'a str,
    pub half_points: u16,
    pub shift: u16,
    pub position: &'static str,
    pub alignment: u8,
}

/// Microsoft RTF 1.9.1, p.196 documents the EQ phonetic-guide construct.
/// Offsets refer to literal UTF-8 instruction bytes; escaping remains unsupported.
/// <https://officeprotocoldoc.z19.web.core.windows.net/files/Archive_References/%5bMSFT-RTF%5d.pdf>
pub(super) fn parse(text: &str) -> Result<Parsed<'_>> {
    let mut p = Parser { text, offset: 0 };
    p.token("EQ")?;
    p.token("\\*")?;
    p.token("jc")?;
    let alignment = p.number()?;
    if alignment > 5 {
        return Err(Error::Unsupported("ruby alignment"));
    }
    p.token("\\*")?;
    p.token("\"Font:")?;
    let start = p.offset;
    let length = p
        .remaining()
        .find('"')
        .ok_or(Error::Invalid("ruby font quote"))?;
    let font = &text[start..start + length];
    if font.is_empty() || font.len() > 1024 || font.chars().any(|c| c.is_control() || c == '\\') {
        return Err(Error::Unsupported("ruby font literal"));
    }
    p.offset += length + 1;
    p.token("\\*")?;
    p.token("hps")?;
    let half_points = p.number()?;
    if half_points == 0 {
        return Err(Error::Invalid("ruby guide size"));
    }
    p.token("\\o")?;
    p.token("\\ad")?;
    p.token("(")?;
    p.token("\\s")?;
    p.space();
    let position = if p
        .remaining()
        .get(..3)
        .is_some_and(|s| s.eq_ignore_ascii_case("\\up"))
    {
        p.token("\\up")?;
        "above"
    } else {
        p.token("\\do")?;
        "below"
    };
    let shift = p.number()?;
    p.token("(")?;
    let guide = p.literal(')')?;
    p.token(",")?;
    let base = p.literal(')')?;
    p.space();
    if !p.remaining().is_empty() {
        return Err(Error::Unsupported("ruby trailing instruction"));
    }
    Ok(Parsed {
        base,
        guide,
        font,
        half_points,
        shift,
        position,
        alignment: alignment as u8,
    })
}

struct Parser<'a> {
    text: &'a str,
    offset: usize,
}
impl Parser<'_> {
    fn remaining(&self) -> &str {
        &self.text[self.offset..]
    }
    fn space(&mut self) {
        while self
            .text
            .as_bytes()
            .get(self.offset)
            .is_some_and(|b| matches!(b, b' ' | b'\t'))
        {
            self.offset += 1;
        }
    }
    fn token(&mut self, expected: &str) -> Result<()> {
        self.space();
        if !self
            .remaining()
            .get(..expected.len())
            .is_some_and(|s| s.eq_ignore_ascii_case(expected))
        {
            return Err(Error::Unsupported("ruby instruction grammar"));
        }
        self.offset += expected.len();
        Ok(())
    }
    fn number(&mut self) -> Result<u16> {
        self.space();
        let start = self.offset;
        let mut value = 0u16;
        while let Some(b'0'..=b'9') = self.text.as_bytes().get(self.offset) {
            value = value
                .checked_mul(10)
                .and_then(|v| v.checked_add((self.text.as_bytes()[self.offset] - b'0') as u16))
                .ok_or(Error::Limit("ruby numeric operand"))?;
            self.offset += 1;
        }
        if start == self.offset {
            return Err(Error::Invalid("ruby numeric operand"));
        }
        Ok(value)
    }
    fn literal(&mut self, closing: char) -> Result<Range<usize>> {
        let start = self.offset;
        let length = self
            .remaining()
            .find(closing)
            .ok_or(Error::Invalid("ruby operand close"))?;
        let text = &self.text[start..start + length];
        if text.trim().is_empty()
            || text
                .chars()
                .any(|c| c.is_control() || matches!(c, '(' | ',' | '\\' | '"'))
        {
            return Err(Error::Unsupported("ruby nonliteral operand"));
        }
        self.offset += length + 1;
        Ok(start..start + length)
    }
}
