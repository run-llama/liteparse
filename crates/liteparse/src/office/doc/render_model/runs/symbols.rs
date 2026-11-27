//! Font-specific source codes retain their Unicode transcription separately from glyph selection.
use super::*;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SourceSymbol {
    pub font_index: usize,
    pub source_code: u16,
    pub unicode: char,
    pub basis: &'static str,
}

pub(crate) fn from_evidence(
    span: &crate::office::doc::symbols::SymbolSpan,
    fonts: &FontTable,
) -> Result<SourceSymbol> {
    let font = fonts
        .fonts
        .get(span.font_index)
        .filter(|f| f.index == span.font_index && f.charset == 2)
        .ok_or(Error::Unsupported("render source symbol font"))?;
    let (unicode, basis) = crate::office::doc::symbols::map_symbol(font, span.source_code);
    let unicode = unicode
        .filter(|c| !c.is_control())
        .ok_or(Error::Unsupported("render source symbol mapping"))?;
    if span.special != Some(true) || span.unicode != Some(unicode) || span.basis != basis {
        return Err(Error::Invalid("render source symbol evidence"));
    }
    Ok(SourceSymbol {
        font_index: span.font_index,
        source_code: span.source_code,
        unicode,
        basis,
    })
}

pub(crate) fn resolve(
    styles: &Stylesheet,
    fonts: &FontTable,
    paragraph_style: usize,
    direct: &[PropertyModifier],
    work: &mut usize,
    symbol: &SourceSymbol,
) -> Result<RunProperties> {
    let last = direct
        .iter()
        .rev()
        .find(|p| p.sprm == 0x6a09)
        .ok_or(Error::Invalid("render source symbol assignment"))?;
    if last.operand.len() != 4
        || crate::office::doc::u16_at(&last.operand, 0)? as usize != symbol.font_index
        || crate::office::doc::u16_at(&last.operand, 2)? != symbol.source_code
    {
        return Err(Error::Invalid("render source symbol assignment"));
    }
    let mut value = resolve_control(
        styles,
        fonts,
        paragraph_style,
        direct,
        work,
        SourceControl::Symbol,
        &[],
    )?;
    value.source_symbol = Some(symbol.clone());
    Ok(value)
}
