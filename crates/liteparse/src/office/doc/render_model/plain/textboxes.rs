//! Retain source storage belonging to reusable textbox PLC records.
use crate::office::doc::{Error, Inspection, Result, Story, textbox_anchors::TextboxPart};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct TextboxStorage {
    pub story: &'static str,
    pub source_cp: [usize; 2],
    pub source_text: String,
    pub records: Vec<StorageRecord>,
    pub textboxes: Vec<AuthoredTextbox>,
}
#[derive(Debug, Serialize)]
pub struct AuthoredTextbox {
    pub record_index: usize,
    pub shape_id: u32,
    pub anchor_cp: usize,
    pub raw_anchor_flags: u16,
    pub rectangle_twips: [i32; 4],
    pub source_cp: [usize; 2],
    pub source_text: String,
    pub paragraphs: Vec<super::PlainParagraph>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<super::drawings::groups::GroupPlacement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_rectangle_twips: Option<[f64; 4]>,
}
#[derive(Debug, Serialize)]
pub struct StorageRecord {
    pub record_index: usize,
    pub raw_cp_range: [u32; 2],
    pub source_cp: [usize; 2],
    pub kind: &'static str,
}

/// FTXBXS distinguishes reusable storage from authored textbox content. Its final
/// record is reusable regardless of its flag and its terminal PLC bound is ignored.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/d0a81fe8-f9e8-4130-9233-84732e3d9420>
pub(super) fn inspect(data: &[u8], source: &Inspection) -> Result<Vec<TextboxStorage>> {
    if !source
        .stories
        .iter()
        .any(|s| matches!(s.kind, "textboxes" | "header_textboxes") && s.utf16_units != 0)
    {
        return Ok(Vec::new());
    }
    let anchors = crate::office::doc::textbox_anchors::inspect_textbox_anchors(data, source)?;
    let grouped = if anchors.parts.iter().any(|part| {
        part.records
            .iter()
            .any(|r| !r.reusable && r.anchor_index.is_none())
    }) {
        let container = crate::office::doc::compound::Compound::open(data)?;
        let word = container.stream("WordDocument")?;
        let (_, name, _, _, _) = crate::office::doc::document::fib(&word)?;
        super::drawings::qualify(&word, &container.stream(name)?)?
    } else {
        None
    };
    let mut output = Vec::new();
    for part in &anchors.parts {
        let story = source
            .stories
            .iter()
            .find(|s| s.kind == part.text_story)
            .ok_or(Error::Invalid("render textbox story missing"))?;
        if story.utf16_units != 0 {
            output.push(qualify_with_drawings(story, part, grouped.as_ref())?);
        }
    }
    validate_all(&output)?;
    Ok(output)
}
#[cfg(test)]
fn qualify(story: &Story, part: &TextboxPart) -> Result<TextboxStorage> {
    qualify_with_drawings(story, part, None)
}
fn qualify_with_drawings(
    story: &Story,
    part: &TextboxPart,
    drawing: Option<&super::drawings::DrawingProof>,
) -> Result<TextboxStorage> {
    let end = story
        .start_cp
        .checked_add(story.utf16_units)
        .ok_or(Error::Limit("render textbox CP"))?;
    let units: Vec<_> = story.text.encode_utf16().collect();
    let mut records = Vec::new();
    let mut textboxes = Vec::new();
    for (index, record) in part.records.iter().enumerate() {
        let start = story
            .start_cp
            .checked_add(record.raw_cp_range[0] as usize)
            .ok_or(Error::Limit("render textbox CP"))?;
        let terminal = index + 1 == part.records.len();
        let limit = if terminal {
            end
        } else {
            story
                .start_cp
                .checked_add(record.raw_cp_range[1] as usize)
                .ok_or(Error::Limit("render textbox CP"))?
        };
        if !record.reusable {
            if record.chain_shape_count != Some(1) {
                return Err(Error::Unsupported("render linked textbox chain"));
            }
            let grouped_shape = drawing.and_then(|d| {
                d.textboxes().iter().find(|s| {
                    Some(s.shape_id) == record.shape_id
                        && s.textbox_record_index == index
                        && s.group.is_some()
                })
            });
            let anchor = record
                .anchor_index
                .and_then(|i| part.shapes.get(i))
                .filter(|a| Some(a.shape_id) == record.shape_id)
                .or_else(|| {
                    grouped_shape.and_then(|s| s.group.as_ref()).and_then(|g| {
                        part.shapes
                            .iter()
                            .find(|a| a.shape_id == g.anchor_shape_id && a.source_cp == g.anchor_cp)
                    })
                })
                .ok_or(Error::Unsupported("render textbox shape ownership"))?;
            let range = record
                .source
                .as_ref()
                .ok_or(Error::Invalid("render textbox source range"))?;
            if [range.start_cp, range.end_cp] != [start, limit] {
                return Err(Error::Invalid("render textbox source mismatch"));
            }
            let text = String::from_utf16(
                units
                    .get(start - story.start_cp..limit - story.start_cp)
                    .ok_or(Error::Invalid("render textbox text range"))?,
            )
            .map_err(|_| Error::Invalid("render textbox UTF16"))?;
            textboxes.push(AuthoredTextbox {
                record_index: index,
                shape_id: record.shape_id.ok_or(Error::Invalid("textbox shape ID"))?,
                anchor_cp: anchor.source_cp,
                raw_anchor_flags: anchor.raw_flags,
                rectangle_twips: anchor.rectangle_twips,
                source_cp: [start, limit],
                source_text: text,
                paragraphs: Vec::new(),
                group: grouped_shape.and_then(|s| s.group.clone()),
                group_rectangle_twips: grouped_shape.and_then(|s| s.group_rectangle_twips),
            });
        }
        records.push(StorageRecord {
            record_index: index,
            raw_cp_range: record.raw_cp_range,
            source_cp: [start, limit],
            kind: if !record.reusable {
                "authored_textbox"
            } else if terminal {
                "terminal_reusable_storage"
            } else {
                "reusable_slot"
            },
        });
    }
    let result = TextboxStorage {
        story: story.kind,
        source_cp: [story.start_cp, end],
        source_text: story.text.clone(),
        records,
        textboxes,
    };
    result.validate()?;
    Ok(result)
}
/// Join every authored record to its independently decoded drawing and anchor.
/// Shape identity joins storage records independently of drawing paint order.
pub(super) fn validate_drawings(
    stories: &[TextboxStorage],
    proof: Option<&super::drawings::DrawingProof>,
) -> Result<()> {
    use super::drawings::{HorizontalOrigin, VerticalOrigin};
    let boxes: Vec<_> = stories.iter().flat_map(|s| &s.textboxes).collect();
    let shapes = proof.map_or(&[][..], |p| p.textboxes());
    if boxes.len() != shapes.len() {
        return Err(Error::Invalid("render textbox drawing cardinality"));
    }
    let by_id: std::collections::BTreeMap<_, _> = boxes.iter().map(|b| (b.shape_id, b)).collect();
    if by_id.len() != boxes.len() {
        return Err(Error::Invalid("render duplicate textbox shape"));
    }
    for shape in shapes {
        let item = by_id
            .get(&shape.shape_id)
            .ok_or(Error::Invalid("render textbox drawing identity"))?;
        if item.record_index != shape.textbox_record_index
            || item.shape_id != shape.shape_id
            || item.anchor_cp != shape.anchor_cp
            || item.rectangle_twips != shape.rectangle_twips
            || item.group != shape.group
            || item.group_rectangle_twips != shape.group_rectangle_twips
        {
            return Err(Error::Invalid("render textbox drawing identity"));
        }
        let header = stories.iter().any(|story| {
            story.story == "header_textboxes"
                && story.textboxes.iter().any(|b| b.shape_id == shape.shape_id)
        });
        if shape.anchor_story != if header { "headers" } else { "body" } {
            return Err(Error::Invalid("textbox anchor story identity"));
        }
        if let Some(wrap) = &shape.square_wrap
            && (header || shape.group.is_some() || wrap.raw_anchor_flags != item.raw_anchor_flags)
        {
            return Err(Error::Invalid("textbox wrapping source ownership"));
        }
        if let Some(wrap) = &shape.top_bottom_wrap
            && (header || shape.group.is_some() || wrap.raw_anchor_flags != item.raw_anchor_flags)
        {
            return Err(Error::Invalid("textbox wrapping source ownership"));
        }
        let unsupported_placement = if header {
            shape.group.is_some()
        } else {
            shape.horizontal_origin != HorizontalOrigin::Column
                || shape.vertical_origin != VerticalOrigin::Paragraph
                || shape.behind_text
        };
        if unsupported_placement {
            return Err(Error::Unsupported("render textbox placement context"));
        }
    }
    Ok(())
}
pub(super) fn anchor_cps(stories: &[TextboxStorage]) -> Result<std::collections::BTreeSet<usize>> {
    let mut owners = std::collections::BTreeMap::new();
    for item in stories.iter().flat_map(|s| &s.textboxes) {
        let group = item.group.as_ref().map(|g| g.anchor_shape_id);
        if let Some(previous) = owners.insert(item.anchor_cp, group)
            && (group.is_none() || previous != group)
        {
            return Err(Error::Invalid("duplicate textbox anchor"));
        }
    }
    Ok(owners.into_keys().collect())
}
pub(super) fn validate_all(stories: &[TextboxStorage]) -> Result<()> {
    if stories.len() > 2 {
        return Err(Error::Limit("render textbox stories"));
    }
    for (index, story) in stories.iter().enumerate() {
        story.validate()?;
        if index != 0
            && (stories[index - 1].story == story.story
                || stories[index - 1].source_cp[1] > story.source_cp[0])
        {
            return Err(Error::Invalid("render textbox story order"));
        }
    }
    Ok(())
}
impl TextboxStorage {
    fn validate(&self) -> Result<()> {
        if !matches!(self.story, "textboxes" | "header_textboxes")
            || self.source_cp[0] >= self.source_cp[1]
            || self.source_text.len() > 4 * 1024 * 1024
            || self.records.is_empty()
            || self.records.len() > 65536
        {
            return Err(Error::Invalid("render textbox storage"));
        }
        let units: Vec<_> = self.source_text.encode_utf16().collect();
        if units.len() != self.source_cp[1] - self.source_cp[0] {
            return Err(Error::Invalid("render textbox source length"));
        }
        let mut identities = std::collections::BTreeSet::new();
        for textbox in &self.textboxes {
            if !identities.insert(textbox.shape_id) {
                return Err(Error::Invalid("render duplicate textbox shape"));
            }
        }
        let mut active = self.textboxes.iter();
        let mut cp = self.source_cp[0];
        for (index, record) in self.records.iter().enumerate() {
            let terminal = index + 1 == self.records.len();
            if record.record_index != index
                || record.source_cp[0] != cp
                || record.source_cp[1] < cp
                || record.source_cp[1] > self.source_cp[1]
                || record.raw_cp_range[0] as usize != cp - self.source_cp[0]
            {
                return Err(Error::Invalid("render textbox storage partition"));
            }
            if terminal {
                if record.kind != "terminal_reusable_storage"
                    || record.source_cp[1] != self.source_cp[1]
                {
                    return Err(Error::Invalid("render terminal textbox storage"));
                }
            } else if record.kind == "authored_textbox" {
                let item = active
                    .next()
                    .filter(|b| b.record_index == index)
                    .ok_or(Error::Invalid("render active textbox record"))?;
                if item.source_cp != record.source_cp
                    || record.source_cp[1] <= cp + 1
                    || item.source_text.encode_utf16().collect::<Vec<_>>()
                        != units[cp - self.source_cp[0]..record.source_cp[1] - self.source_cp[0]]
                    || !item.source_text.ends_with('\r')
                {
                    return Err(Error::Invalid("render active textbox text"));
                }
            } else if record.kind != "reusable_slot"
                || record.source_cp[1] != cp + 1
                || record.raw_cp_range[1] as usize != record.source_cp[1] - self.source_cp[0]
                || units.get(cp - self.source_cp[0]) != Some(&13)
            {
                return Err(Error::Invalid("render reusable textbox storage"));
            }
            cp = record.source_cp[1];
        }
        if self.textboxes.len()
            != self
                .records
                .iter()
                .filter(|r| r.kind == "authored_textbox")
                .count()
            || self
                .textboxes
                .windows(2)
                .any(|w| w[0].record_index >= w[1].record_index || w[0].shape_id == w[1].shape_id)
        {
            return Err(Error::Invalid("render textbox identity"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
