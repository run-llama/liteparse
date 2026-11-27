use crate::office::doc::{Error, Result, Story, textbox_anchors::TextboxInspection};

/// Active FTXBXS ranges own logical textbox paragraphs independently of shape placement.
/// The final reusable record and intermediate spare slots remain source text without labels.
pub(super) fn active_ranges(
    source: &Story,
    metadata: &TextboxInspection,
) -> Result<Vec<[usize; 2]>> {
    if source.utf16_units > super::MAX_UNITS || metadata.parts.len() > 2 {
        return Err(Error::Limit("textbox list source units"));
    }
    let mut parts = metadata
        .parts
        .iter()
        .filter(|part| part.text_story == source.kind);
    let part = parts
        .next()
        .ok_or(Error::Invalid("textbox list story metadata"))?;
    if parts.next().is_some()
        || !matches!(
            (part.text_story, part.anchor_story),
            ("textboxes", "body") | ("header_textboxes", "headers")
        )
    {
        return Err(Error::Invalid("textbox list story ownership"));
    }
    if part.records.len() > 65_536 {
        return Err(Error::Limit("textbox list records"));
    }
    let units: Vec<u16> = source.text.encode_utf16().collect();
    if units.len() != source.utf16_units {
        return Err(Error::Invalid("textbox list source length"));
    }
    let mut ranges = Vec::new();
    let mut previous = None;
    for (index, record) in part.records.iter().enumerate() {
        let [start, end] = record.raw_cp_range.map(|cp| cp as usize);
        if record.record_index != index || previous.is_some_and(|cp| cp != start) {
            return Err(Error::Invalid("textbox list record order"));
        }
        previous = Some(end);
        if index + 1 == part.records.len() {
            if !record.reusable || record.source.is_some() {
                return Err(Error::Invalid("textbox list terminal spare"));
            }
            continue;
        }
        let slice = units
            .get(start..end)
            .filter(|s| !s.is_empty())
            .ok_or(Error::Invalid("textbox list source range"))?;
        if slice.last() != Some(&13)
            || (start != 0 && units.get(start - 1) != Some(&13))
            || char::decode_utf16(slice.iter().copied()).any(|c| c.is_err())
        {
            return Err(Error::Invalid("textbox list paragraph boundary"));
        }
        if record.reusable {
            if slice.len() != 1 || record.source.is_some() {
                return Err(Error::Invalid("textbox list reusable range"));
            }
            continue;
        }
        let expected = [
            source.start_cp.checked_add(start),
            source.start_cp.checked_add(end),
        ];
        let owned = record
            .source
            .as_ref()
            .ok_or(Error::Invalid("textbox list active source"))?;
        if expected != [Some(owned.start_cp), Some(owned.end_cp)]
            || slice.len() <= 1
            || record.shape_id.is_none()
            || !record
                .chain_shape_count
                .is_some_and(|count| (1..=65_536).contains(&count))
        {
            return Err(Error::Invalid("textbox list active ownership"));
        }
        ranges.push([owned.start_cp, owned.end_cp]);
    }
    if part.records.is_empty() && !units.is_empty() {
        return Err(Error::Invalid("textbox list records absent"));
    }
    Ok(ranges)
}
