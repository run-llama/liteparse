//! Single source characters whose explicit CSymbol assignment has a known Unicode mapping.
use crate::office::doc::{
    Result,
    render_model::runs::{self, SourceSymbol},
    symbols::FontTable,
    visibility::RevisionEvidence,
};
use std::collections::BTreeMap;

pub(super) fn inspect(
    fonts: &FontTable,
    revisions: &RevisionEvidence,
    units: &[u16],
) -> Result<BTreeMap<usize, SourceSymbol>> {
    let evidence = crate::office::doc::symbols::resolve_symbol_evidence(fonts, revisions)?;
    let mut selected = BTreeMap::new();
    for span in &evidence.spans {
        if span.end_cp != span.start_cp + 1
            || span.unicode.is_none()
            || units
                .get(span.start_cp)
                .and_then(|u| char::from_u32(u32::from(*u)))
                .is_none_or(char::is_control)
        {
            continue;
        }
        if let Ok(symbol) = runs::symbols::from_evidence(span, fonts) {
            selected.insert(span.start_cp, symbol);
        }
    }
    Ok(selected)
}
