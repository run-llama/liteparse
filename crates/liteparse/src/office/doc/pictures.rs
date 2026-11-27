//! Encoded picture assets with source anchors; image visibility and drawing layout stay explicit.
use crate::office::doc::{Error, Result, bytes, compound::Compound, document::fib, u16_at, u32_at};
use serde::Serialize;
use std::collections::BTreeMap;
mod officeart;
pub mod raster_store;
use officeart::{Budget, Record};
const MAX_ASSET_BYTES: usize = 32 * 1024 * 1024;
const MAX_REFERENCES: usize = 65_536;

#[derive(Debug, Serialize)]
pub struct PayloadSource {
    pub stream: &'static str,
    pub record_range: [usize; 2],
    pub payload_range: [usize; 2],
    pub compression: &'static str,
}
#[derive(Debug, Serialize)]
pub struct ImageAsset {
    pub format: &'static str,
    pub byte_length: usize,
    pub source: PayloadSource,
    #[serde(skip)]
    bytes: Vec<u8>,
}
impl ImageAsset {
    /// Original encoded raster or decompressed metafile bytes, ready for an image decoder.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}
#[derive(Debug, Serialize)]
pub struct PictureReference {
    pub source_cp: Option<usize>,
    pub group_anchor_cp: Option<usize>,
    pub story: &'static str,
    pub shape_id: Option<u32>,
    pub asset_index: Option<usize>,
    pub role: &'static str,
    pub unresolved: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct PictureInspection {
    pub status: &'static str,
    pub assets: Vec<ImageAsset>,
    pub source_picture_markers: usize,
    pub source_drawing_markers: usize,
    pub references: Vec<PictureReference>,
    pub diagnostics: Vec<String>,
}
struct Decoder<'a> {
    word: &'a [u8],
    budget: Budget,
    output: PictureInspection,
    cache: BTreeMap<(&'static str, usize, usize), usize>,
}
impl Decoder<'_> {
    fn asset(&mut self, data: &[u8], record: Record, stream: &'static str) -> Result<usize> {
        let key = (stream, record.offset, record.end);
        if let Some(index) = self.cache.get(&key) {
            return Ok(*index);
        }
        if self.output.assets.len() >= MAX_REFERENCES {
            return Err(Error::Limit("picture assets"));
        }
        let asset = officeart::file_block(data, record, stream, self.word, &mut self.budget)?;
        let index = self.output.assets.len();
        self.output.assets.push(asset);
        self.cache.insert(key, index);
        Ok(index)
    }
    fn reference(&mut self, reference: PictureReference) -> Result<()> {
        if self.output.references.len() >= MAX_REFERENCES {
            return Err(Error::Limit("picture references"));
        }
        self.output.references.push(reference);
        Ok(())
    }
}

/// Extract image bytes from modern binary Word without changing source text.
/// Inline PICF records use Data; floating FBSE delay offsets use WordDocument.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/8699a984-3718-44be-adae-08b05827f8b3>
pub fn inspect_pictures(data: &[u8]) -> Result<PictureInspection> {
    let compound = Compound::open(data)?;
    let word = compound.stream("WordDocument")?;
    let (_, table_name, _, _, _) = fib(&word)?;
    let table = compound.stream(table_name)?;
    let source = crate::office::doc::inspect(data)?;
    let anchors = crate::office::doc::textbox_anchors::inspect_textbox_anchors(data, &source)
        .map(|value| anchor_map(&value));
    let mut decoder = Decoder {
        word: &word,
        budget: Budget::default(),
        output: PictureInspection {
            status: "partial_source_picture_assets",
            source_picture_markers: 0,
            source_drawing_markers: 0,
            assets: vec![],
            references: vec![],
            diagnostics: vec![],
        },
        cache: BTreeMap::new(),
    };
    let off = u32_at(&word, 154 + 50 * 8)? as usize;
    let len = u32_at(&word, 158 + 50 * 8)? as usize;
    if len > 0 {
        bytes(&table, off, len)?;
        if let Err(error) = floating(&mut decoder, &table, off, off + len, anchors.as_ref().ok()) {
            if matches!(error, Error::Limit(_)) {
                return Err(error);
            }
            decoder
                .output
                .diagnostics
                .push(format!("floating pictures: {error}"));
        }
    }
    match (
        compound.stream("Data"),
        crate::office::doc::formatting_inspection::inspect_character_formatting(data),
    ) {
        (Ok(stream), Ok(formatting)) => {
            if let Err(error) = inline(&mut decoder, &stream, &source, &formatting) {
                if matches!(error, Error::Limit(_)) {
                    return Err(error);
                }
                decoder
                    .output
                    .diagnostics
                    .push(format!("inline pictures: {error}"));
            }
        }
        (Err(error), _) => decoder
            .output
            .diagnostics
            .push(format!("inline Data stream: {error}")),
        (_, Err(error)) => decoder
            .output
            .diagnostics
            .push(format!("inline properties: {error}")),
    }
    let covered: std::collections::BTreeSet<_> = decoder
        .output
        .references
        .iter()
        .flat_map(|r| [r.source_cp, r.group_anchor_cp])
        .flatten()
        .collect();
    for story in &source.stories {
        for (local, unit) in story.text.encode_utf16().enumerate() {
            if !matches!(unit, 1 | 8) {
                continue;
            }
            if unit == 1 {
                decoder.output.source_picture_markers += 1;
            } else {
                decoder.output.source_drawing_markers += 1;
            }
            let cp = story.start_cp + local;
            if !covered.contains(&cp) {
                decoder.reference(PictureReference {source_cp:Some(cp),group_anchor_cp:None,story:story.kind,
                    shape_id:None,asset_index:None,role:if unit==1{"picture_candidate"}else{"drawing_candidate"},
                    unresolved:vec!["source marker has no resolved picture relationship; may represent a non-picture shape or unresolved special properties".into()]})?;
            }
        }
    }
    Ok(decoder.output)
}

type AnchorMap = BTreeMap<(&'static str, u32), Option<usize>>;
fn anchor_map(anchors: &crate::office::doc::textbox_anchors::TextboxInspection) -> AnchorMap {
    let mut map = AnchorMap::new();
    for part in &anchors.parts {
        for shape in &part.shapes {
            map.entry((part.anchor_story, shape.shape_id))
                .and_modify(|cp| *cp = None)
                .or_insert(Some(shape.source_cp));
        }
    }
    map
}

fn floating(
    decoder: &mut Decoder<'_>,
    data: &[u8],
    start: usize,
    end: usize,
    anchors: Option<&AnchorMap>,
) -> Result<()> {
    let root = decoder.budget.record(data, start, end)?;
    if root.kind != 0xf000 || root.version != 15 {
        return Err(Error::Invalid("OfficeArt drawing group"));
    }
    let mut stores = vec![];
    let mut pos = root.start;
    while pos < root.end {
        let r = decoder.budget.record(data, pos, root.end)?;
        if r.kind == 0xf001 {
            if r.version != 15 {
                return Err(Error::Invalid("OfficeArt picture store"));
            }
            let mut cursor = r.start;
            while cursor < r.end {
                let block = decoder.budget.record(data, cursor, r.end)?;
                // Keep empty and unsupported slots so one-based picture IDs retain their meaning.
                let asset = decoder.asset(data, block, "Table");
                if let Err(Error::Limit(message)) = asset {
                    return Err(Error::Limit(message));
                }
                stores.push(asset.map_err(|e| e.to_string()));
                cursor = block.end;
            }
            if stores.len() != r.instance as usize {
                return Err(Error::Invalid("picture store count"));
            }
        }
        pos = r.end;
    }
    let mut pos = root.end;
    let mut drawings = Vec::new();
    let mut identities = BTreeMap::new();
    while pos < end {
        let story = match data[pos] {
            0 => "body",
            1 => "headers",
            _ => return Err(Error::Invalid("drawing story")),
        };
        pos += 1;
        let r = decoder.budget.record(data, pos, end)?;
        if r.kind != 0xf002 || r.version != 15 {
            return Err(Error::Invalid("OfficeArt drawing container"));
        }
        scan_shape_ids(decoder, data, r, story, &mut identities, 0)?;
        drawings.push((story, r));
        pos = r.end;
    }
    let mut unique_anchors = anchors.cloned().unwrap_or_default();
    let mut duplicates = 0;
    for (key, duplicate) in identities {
        if duplicate {
            unique_anchors.insert(key, None);
            duplicates += 1;
        }
    }
    if duplicates > 0 {
        decoder.output.diagnostics.push(format!(
            "{duplicates} duplicate drawing shape IDs have no unique source anchor"
        ));
    }
    for (story, r) in drawings {
        shapes(
            decoder,
            data,
            r,
            story,
            &stores,
            Some(&unique_anchors),
            (None, 0),
        )?;
    }
    Ok(())
}

// Validate all identities before emitting relationships: a later duplicate also
// invalidates earlier direct and group anchors in the same document story.
fn scan_shape_ids(
    decoder: &mut Decoder<'_>,
    data: &[u8],
    container: Record,
    story: &'static str,
    identities: &mut BTreeMap<(&'static str, u32), bool>,
    depth: usize,
) -> Result<()> {
    if depth > 32 {
        return Err(Error::Limit("drawing identity nesting"));
    }
    let mut pos = container.start;
    while pos < container.end {
        let r = decoder.budget.record(data, pos, container.end)?;
        if container.kind == 0xf004 && r.kind == 0xf00a {
            if r.version != 2 || r.end - r.start != 8 {
                return Err(Error::Invalid("shape identity"));
            }
            let key = (story, u32_at(data, r.start)?);
            if !identities.contains_key(&key) && identities.len() >= MAX_REFERENCES {
                return Err(Error::Limit("drawing identities"));
            }
            identities
                .entry(key)
                .and_modify(|duplicate| *duplicate = true)
                .or_insert(false);
        }
        if matches!(r.kind, 0xf003 | 0xf004) && r.version == 15 {
            scan_shape_ids(decoder, data, r, story, identities, depth + 1)?;
        }
        pos = r.end;
    }
    Ok(())
}
fn shapes(
    decoder: &mut Decoder<'_>,
    data: &[u8],
    container: Record,
    story: &'static str,
    stores: &[std::result::Result<usize, String>],
    anchors: Option<&AnchorMap>,
    group_scope: (Option<usize>, usize),
) -> Result<()> {
    let (group_anchor_cp, depth) = group_scope;
    if depth > 32 {
        return Err(Error::Limit("drawing nesting"));
    }
    if container.kind == 0xf004 {
        let mut shape_id = None;
        let mut properties = vec![];
        let mut pos = container.start;
        while pos < container.end {
            let r = decoder.budget.record(data, pos, container.end)?;
            if r.kind == 0xf00a {
                if r.end - r.start != 8 || r.version != 2 {
                    return Err(Error::Invalid("shape identity"));
                }
                if shape_id.is_some() {
                    return Err(Error::Invalid("duplicate shape identity record"));
                }
                shape_id = Some(u32_at(data, r.start)?);
            }
            if matches!(r.kind, 0xf00b | 0xf122) {
                properties.extend(picture_properties(data, r, false)?);
            }
            pos = r.end;
        }
        let source_cp =
            shape_id.and_then(|id| anchors.and_then(|a| a.get(&(story, id))).copied().flatten());
        for property in properties {
            let mut unresolved =
                vec!["drawing visibility, transforms and cropping unresolved".into()];
            if source_cp.is_none() {
                unresolved.push(
                    "shape has no unique direct source anchor; grouped ownership unresolved".into(),
                );
            }
            let asset_index = if let Some([start, end]) = property.complex {
                match decoder
                    .budget
                    .record(data, start, end)
                    .and_then(|r| decoder.asset(data, r, "Table"))
                {
                    Ok(index) => Some(index),
                    Err(e) => {
                        if matches!(e, Error::Limit(_)) {
                            return Err(e);
                        }
                        unresolved.push(e.to_string());
                        None
                    }
                }
            } else if property.value == 0 {
                continue;
            } else {
                match stores.get(property.value as usize - 1) {
                    Some(Ok(index)) => Some(*index),
                    Some(Err(e)) => {
                        unresolved.push(e.clone());
                        None
                    }
                    None => {
                        unresolved.push("picture store index out of bounds".into());
                        None
                    }
                }
            };
            decoder.reference(PictureReference {
                source_cp,
                group_anchor_cp,
                story,
                shape_id,
                asset_index,
                role: property.role,
                unresolved,
            })?;
        }
    } else {
        let mut group_anchor_cp = group_anchor_cp;
        let mut pos = container.start;
        while pos < container.end {
            let r = decoder.budget.record(data, pos, container.end)?;
            if container.kind == 0xf003 && r.kind == 0xf004 && pos == container.start {
                let mut p = r.start;
                let mut group_identity_seen = false;
                while p < r.end {
                    let child = decoder.budget.record(data, p, r.end)?;
                    if child.kind == 0xf00a && child.end - child.start == 8 {
                        if group_identity_seen {
                            return Err(Error::Invalid("duplicate group identity record"));
                        }
                        group_identity_seen = true;
                        let id = u32_at(data, child.start)?;
                        group_anchor_cp = anchors
                            .and_then(|a| a.get(&(story, id)))
                            .copied()
                            .flatten()
                            .or(group_anchor_cp);
                    }
                    p = child.end;
                }
            }
            if matches!(r.kind, 0xf003 | 0xf004) && r.version == 15 {
                shapes(
                    decoder,
                    data,
                    r,
                    story,
                    stores,
                    anchors,
                    (group_anchor_cp, depth + 1),
                )?;
            }
            pos = r.end;
        }
    }
    Ok(())
}
struct PictureProperty {
    value: u32,
    complex: Option<[usize; 2]>,
    role: &'static str,
}
/// Picture slots are distinct from complex names and equation metadata with ignored fBid.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-odraw/429e3263-f29a-4bd5-9713-ec7de3869fed>
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-odraw/80c5cbc0-c9dc-4f47-89ab-fbab33ab5451>
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-odraw/aaa94f58-eab7-4e88-ba37-de97d319c2e7>
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-odraw/3fcb6cc3-b8c2-481b-aa47-078898d87c78>
fn picture_properties(data: &[u8], r: Record, inline: bool) -> Result<Vec<PictureProperty>> {
    if r.version != 3 {
        return Err(Error::Invalid("shape property version"));
    }
    let count = r.instance as usize;
    let mut extra = r.start + count * 6;
    if extra > r.end {
        return Err(Error::Invalid("shape property count"));
    }
    let mut result = vec![];
    for n in 0..count {
        let p = r.start + n * 6;
        let id = u16_at(data, p)?;
        let value = u32_at(data, p + 2)?;
        let role = match id & 0x3fff {
            0x104 => Some("picture"),
            0x10f => Some("print_picture"),
            0x186 => Some("fill_picture"),
            _ => None,
        };
        // Inline BLIP flags and values are ignored; their payloads follow the shape in order.
        let complex = if id & 0x8000 != 0 && !(inline && role.is_some()) {
            let length = value as usize;
            if length > r.end - extra {
                return Err(Error::Invalid("complex picture property length"));
            }
            let range = [extra, extra + length];
            extra += length;
            Some(range)
        } else {
            None
        };
        if let Some(role) = role {
            if !inline && complex.is_none() && id & 0x4000 == 0 {
                return Err(Error::Invalid("picture property BLIP flag"));
            }
            result.push(PictureProperty {
                value,
                complex,
                role,
            });
        } else if inline
            && id & 0x4000 != 0
            && !matches!(id & 0x3fff, 0x105 | 0x30c | 0x380 | 0x381)
        {
            // pibName, equationXML, wzName and wzDescription explicitly ignore fBid.
            // Their complex data remains bounded above and does not consume a BLIP slot.
            return Err(Error::Unsupported("inline BLIP property role"));
        }
    }
    if extra != r.end {
        return Err(Error::Invalid("shape complex property tail"));
    }
    Ok(result)
}

fn inline(
    decoder: &mut Decoder<'_>,
    data: &[u8],
    source: &crate::office::doc::Inspection,
    formatting: &crate::office::doc::formatting_inspection::FormattingInspection,
) -> Result<()> {
    let revisions = crate::office::doc::visibility::resolve_revision_evidence(formatting)?;
    let mut groups = BTreeMap::new();
    let mut markers = BTreeMap::new();
    for story in &source.stories {
        for (local, unit) in story.text.encode_utf16().enumerate() {
            if unit == 1 {
                if markers.len() >= MAX_REFERENCES {
                    return Err(Error::Limit("picture markers"));
                }
                markers.insert(story.start_cp + local, story.kind);
            }
        }
    }
    for span in &revisions.spans {
        let Some(location) = span.evidence.iter().rev().find(|p| p.sprm == 0x6a03) else {
            continue;
        };
        if span.properties.special != Some(true) || !span.unresolved.is_empty() {
            continue;
        }
        let direct = &formatting.character_spans[span.source_span_index].properties;
        let prm = formatting.pieces[span.piece_index].raw_prm;
        if let std::collections::btree_map::Entry::Vacant(e) = groups.entry(prm) {
            e.insert(crate::office::doc::visibility::piece_modifiers(
                prm,
                &formatting.raw_property_groups,
            )?);
        }
        // CFData survives character-style/plain resets and defaults to false (MS-DOC 2.6.1).
        let mut binary = false;
        for property in direct.iter().chain(groups[&prm].iter()) {
            if property.sprm == 0x0806 {
                binary = match property.operand.as_slice() {
                    [0] => false,
                    [1] => true,
                    _ => return Err(Error::Invalid("picture CFData boolean")),
                };
            }
        }
        let offset = u32_at(&location.operand, 0)? as usize;
        for (&cp, &story) in markers.range(span.start_cp..span.end_cp) {
            if binary {
                decoder.reference(PictureReference {
                    source_cp: Some(cp),
                    group_anchor_cp: None,
                    story,
                    shape_id: None,
                    asset_index: None,
                    role: "binary_data",
                    unresolved: vec![
                        "CFData identifies arbitrary binary data, not a picture".into(),
                    ],
                })?;
                continue;
            }
            let result = inline_record(decoder, data, offset, cp, story);
            if let Err(error) = result {
                if matches!(error, Error::Limit(_)) {
                    return Err(error);
                }
                decoder.reference(PictureReference {
                    source_cp: Some(cp),
                    group_anchor_cp: None,
                    story,
                    shape_id: None,
                    asset_index: None,
                    role: "picture",
                    unresolved: vec![error.to_string()],
                })?;
            }
        }
    }
    Ok(())
}
fn inline_record(
    decoder: &mut Decoder<'_>,
    data: &[u8],
    offset: usize,
    cp: usize,
    story: &'static str,
) -> Result<()> {
    let length = u32_at(data, offset)? as usize;
    if length < 68 || u16_at(data, offset + 4)? != 68 {
        return Err(Error::Invalid("PICF header"));
    }
    bytes(data, offset, length)?;
    let end = offset + length;
    let mut pos = offset + 68;
    match u16_at(data, offset + 6)? {
        100 => {}
        102 => {
            if pos >= end {
                return Err(Error::Invalid("PICF name"));
            }
            pos += 1 + data[pos] as usize;
        }
        _ => return Err(Error::Unsupported("PICF mapping mode")),
    }
    let shape = decoder.budget.record(data, pos, end)?;
    if shape.kind != 0xf004 || shape.version != 15 {
        return Err(Error::Invalid("inline shape container"));
    }
    let mut properties = vec![];
    let mut shape_id = None;
    pos = shape.start;
    while pos < shape.end {
        let r = decoder.budget.record(data, pos, shape.end)?;
        if r.kind == 0xf00a && r.end - r.start == 8 {
            if shape_id.is_some() {
                return Err(Error::Invalid("duplicate inline shape identity record"));
            }
            shape_id = Some(u32_at(data, r.start)?);
        }
        if matches!(r.kind, 0xf00b | 0xf122) {
            properties.extend(picture_properties(data, r, true)?);
        }
        pos = r.end;
    }
    pos = shape.end;
    for property in properties {
        let r = decoder.budget.record(data, pos, end)?;
        let result = decoder.asset(data, r, "Data");
        pos = r.end;
        let mut unresolved = vec!["picture visibility, cropping and effects unresolved".into()];
        let asset_index = match result {
            Ok(index) => Some(index),
            Err(error) => {
                if matches!(error, Error::Limit(_)) {
                    return Err(error);
                }
                unresolved.push(error.to_string());
                None
            }
        };
        decoder.reference(PictureReference {
            source_cp: Some(cp),
            group_anchor_cp: None,
            story,
            shape_id,
            asset_index,
            role: property.role,
            unresolved,
        })?;
    }
    if pos != end {
        return Err(Error::Unsupported("unassociated inline image records"));
    }
    Ok(())
}

/// Export source-present assets into a new directory; existing paths are not overwritten.
/// Filenames come from validated formats and numeric IDs, independently of document names.
pub fn export_assets(
    inspection: &PictureInspection,
    directory: &std::path::Path,
) -> std::io::Result<Vec<String>> {
    use std::io::Write;
    if inspection
        .assets
        .iter()
        .any(|asset| !matches!(asset.format, "png" | "jpeg" | "wmf" | "emf"))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "unsupported image export format",
        ));
    }
    #[expect(clippy::create_dir, reason = "the export directory must not exist yet")]
    std::fs::create_dir(directory)?;
    let mut names = Vec::with_capacity(inspection.assets.len());
    for (index, asset) in inspection.assets.iter().enumerate() {
        let name = format!("asset-{index:05}.{}", asset.format);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(&name))?;
        file.write_all(asset.bytes())?;
        names.push(name);
    }
    let mut manifest = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("manifest.json"))?;
    serde_json::to_writer_pretty(
        &mut manifest,
        &serde_json::json!({"files":names,"pictures":inspection}),
    )?;
    manifest.write_all(b"\n")?;
    Ok(names)
}

#[cfg(test)]
mod tests;
