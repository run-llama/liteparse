//! Qualify bounded OfficeArt group scaffolding and explicit nonpainting backgrounds.

use crate::office::doc::{Error, Result, bytes, u16_at, u32_at};
use serde::Serialize;
mod anchor;
mod appearance;
pub mod defaults;
pub mod groups;
mod image;
mod lines;
mod primitive;
mod rectangles;
pub use lines::LineShape;
pub use rectangles::RectangleShape;
pub mod image_properties;
mod omitted;
pub use image::ImageShape;
mod textbox;
mod textbox_style;
pub use anchor::{HorizontalOrigin, VerticalOrigin, WrapSide};
pub use textbox::{SquareWrap, TextboxShape, TopBottomWrap};

#[derive(Debug, Serialize)]
pub struct DrawingProof {
    table_range: [usize; 2],
    records: Vec<RecordProof>,
    background_shapes: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    textboxes: Vec<TextboxShape>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    images: Vec<ImageShape>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    lines: Vec<LineShape>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    rectangles: Vec<RectangleShape>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub omitted_shapes: Vec<groups::OmittedShape>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub group_approximations: Vec<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub document_defaults: Vec<defaults::DefaultEvidence>,
    basis: &'static str,
}

impl DrawingProof {
    pub fn rectangles(&self) -> &[RectangleShape] {
        &self.rectangles
    }

    pub fn lines(&self) -> &[LineShape] {
        &self.lines
    }
    pub fn images(&self) -> &[ImageShape] {
        &self.images
    }
    pub fn textboxes(&self) -> &[TextboxShape] {
        &self.textboxes
    }
}

#[derive(Debug, Serialize)]
struct RecordProof {
    table_range: [usize; 2],
    record_type: u16,
    instance: u16,
    properties: Vec<PropertyProof>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unused_empty_group_coordinates: Option<[i32; 4]>,
}

#[derive(Debug, Serialize)]
struct PropertyProof {
    table_range: [usize; 2],
    raw_property_id: u16,
    raw_operand: u32,
}

struct Record<'a> {
    version: u16,
    instance: u16,
    kind: u16,
    data: &'a [u8],
    payload_offset: usize,
}

struct Reader {
    records: Vec<RecordProof>,
    shape_ids: std::collections::BTreeSet<u32>,
    backgrounds: usize,
    anchors: Vec<textbox::RawAnchor>,
    used_anchors: std::collections::BTreeSet<usize>,
    textboxes: Vec<TextboxShape>,
    images: Vec<ImageShape>,
    lines: Vec<LineShape>,
    rectangles: Vec<RectangleShape>,
    header_anchors: Vec<textbox::RawAnchor>,
    used_header_anchors: std::collections::BTreeSet<usize>,
    store_count: usize,
    omitted_shapes: Vec<groups::OmittedShape>,
    group_approximations: Vec<u32>,
    document_defaults: Vec<defaults::DefaultEvidence>,
    default_text_properties: std::collections::BTreeMap<u16, u32>,
}

fn unsupported() -> Error {
    Error::Unsupported("render drawing content or properties")
}

impl Reader {
    fn add_textbox(&mut self, shape: TextboxShape) -> Result<()> {
        if self
            .textboxes
            .iter()
            .any(|old| old.textbox_record_index == shape.textbox_record_index)
        {
            return Err(Error::Invalid("render duplicate textbox record ownership"));
        }
        self.textboxes.push(shape);
        Ok(())
    }

    fn record<'a>(&mut self, data: &'a [u8], at: &mut usize, base: usize) -> Result<Record<'a>> {
        if self.records.len() >= 4096 {
            return Err(Error::Limit("render drawing records"));
        }
        let start = *at;
        let header = u16_at(data, start)?;
        let kind = u16_at(data, start + 2)?;
        let length = u32_at(data, start + 4)? as usize;
        let payload = start.checked_add(8).ok_or_else(unsupported)?;
        let body = bytes(data, payload, length)?;
        *at = payload.checked_add(length).ok_or_else(unsupported)?;
        let mut properties = Vec::new();
        if matches!(kind, 0xf00b | 0xf121 | 0xf122) && length >= (header >> 4) as usize * 6 {
            for offset in (0..(header >> 4) as usize * 6).step_by(6) {
                properties.push(PropertyProof {
                    table_range: [base + payload + offset, base + payload + offset + 6],
                    raw_property_id: u16_at(body, offset)?,
                    raw_operand: u32_at(body, offset + 2)?,
                });
            }
        }
        self.records.push(RecordProof {
            table_range: [base + start, base + *at],
            record_type: kind,
            instance: header >> 4,
            properties,
            unused_empty_group_coordinates: None,
        });
        Ok(Record {
            version: header & 15,
            instance: header >> 4,
            kind,
            data: body,
            payload_offset: base + payload,
        })
    }

    fn shape(&mut self, record: Record<'_>, group: bool, empty_group: bool) -> Result<()> {
        require(&record, 0xf004, 15, Some(0))?;
        let mut at = 0;
        if group {
            let coordinates = self.record(record.data, &mut at, record.payload_offset)?;
            require(&coordinates, 0xf009, 1, Some(0))?;
            if coordinates.data.len() != 16 || !empty_group && coordinates.data != [0; 16] {
                return Err(unsupported());
            }
            // An empty top group has no child anchors expressed in these coordinates.
            // Keep its source bounds while preserving the strict occupied-group check.
            if coordinates.data != [0; 16] {
                self.records
                    .last_mut()
                    .ok_or_else(unsupported)?
                    .unused_empty_group_coordinates = Some(std::array::from_fn(|i| {
                    i32::from_le_bytes(coordinates.data[i * 4..i * 4 + 4].try_into().unwrap())
                }));
            }
        }
        let shape = self.record(record.data, &mut at, record.payload_offset)?;
        require(&shape, 0xf00a, 2, Some(u16::from(!group)))?;
        if shape.data.len() != 8
            || u32_at(shape.data, 4)? != if group { 5 } else { 0xe00 }
            || !self.shape_ids.insert(u32_at(shape.data, 0)?)
        {
            return Err(unsupported());
        }
        if !group {
            let options = self.record(record.data, &mut at, record.payload_offset)?;
            require(&options, 0xf00b, 3, None)?;
            // MS-ODRAW fill/line use bits make both paint operations explicitly false.
            // Shape Boolean identifies a background; bWMode concerns monochrome output.
            let mut required = [false; 3];
            if options.data.len() != options.instance as usize * 6 {
                return Err(unsupported());
            }
            let mut previous = None;
            for property in options.data.as_chunks::<6>().0 {
                let key = u16_at(property, 0)?;
                let value = u32_at(property, 2)?;
                if previous.is_some_and(|p| p >= key) {
                    return Err(unsupported());
                }
                previous = Some(key);
                match (key, value) {
                    // No-fill hit testing changes editor selection, while fill stays off.
                    (0x1bf, 0x100000 | 0x110000 | 0x110001) => required[0] = true,
                    (0x1ff, 0x80000) => required[1] = true,
                    (0x33f, 0x10001) => required[2] = true,
                    (0x1cb, 0) | (0x304, 9) => {}
                    _ => return Err(unsupported()),
                }
            }
            if !required.into_iter().all(|v| v) {
                return Err(unsupported());
            }
            if at < record.data.len() {
                let client = self.record(record.data, &mut at, record.payload_offset)?;
                require(&client, 0xf011, 0, Some(0))?;
                if client.data.len() != 4 {
                    return Err(unsupported());
                }
            }
            // MS-DOC 2.9.169 defines the four-byte client integer as ignorable.
            // Textbox records and additional property blocks fail the exact partition.
            self.backgrounds += 1;
        }
        if at != record.data.len() {
            return Err(unsupported());
        }
        Ok(())
    }

    fn drawing(&mut self, record: Record<'_>, story: u8) -> Result<()> {
        require(&record, 0xf002, 15, Some(0))?;
        let mut at = 0;
        let info = self.record(record.data, &mut at, record.payload_offset)?;
        require(&info, 0xf008, 0, None)?;
        if info.data.len() != 8 {
            return Err(unsupported());
        }
        if at < record.data.len() && u16_at(record.data, at + 2)? == 0xf118 {
            let history = self.record(record.data, &mut at, record.payload_offset)?;
            require(&history, 0xf118, 0, None)?;
            if history.data.len() != usize::from(history.instance) * 4 {
                return Err(unsupported());
            }
            for offset in (0..history.data.len()).step_by(4) {
                if u16_at(history.data, offset)? <= u16_at(history.data, offset + 2)? {
                    return Err(unsupported());
                }
            }
        }
        let group = self.record(record.data, &mut at, record.payload_offset)?;
        require(&group, 0xf003, 15, Some(0))?;
        let mut child_at = 0;
        let child = self.record(group.data, &mut child_at, group.payload_offset)?;
        self.shape(child, true, child_at == group.data.len())?;
        while child_at < group.data.len() {
            let child = self.record(group.data, &mut child_at, group.payload_offset)?;
            if child.kind == 0xf003 {
                groups::decode(self, child, story, None)?;
            } else if textbox::has_client_textbox(&child)? {
                let textbox = textbox::decode(self, child, story)?;
                self.add_textbox(textbox)?;
            } else if u16_at(child.data, 0)? >> 4 == 20 {
                let line = lines::decode(self, child, story)?;
                self.lines.push(line);
            } else if u16_at(child.data, 0)? >> 4 == 75 {
                let image = image::decode(self, child, story)?;
                self.images.push(image);
            } else if story == 1 && u16_at(child.data, 0)? >> 4 == 1 {
                let rectangle = rectangles::decode(self, child, story)?;
                self.rectangles.push(rectangle);
            } else if u16_at(child.data, 0)? >> 4 == 1 {
                omitted::rectangle(self, child, story)?;
            } else {
                let textbox = textbox::decode(self, child, story)?;
                self.add_textbox(textbox)?;
            }
        }
        if at < record.data.len() {
            let background = self.record(record.data, &mut at, record.payload_offset)?;
            self.shape(background, false, false)?;
        }
        if at != record.data.len() {
            return Err(unsupported());
        }
        Ok(())
    }
}

fn require(record: &Record<'_>, kind: u16, version: u16, instance: Option<u16>) -> Result<()> {
    if record.kind != kind
        || record.version != version
        || instance.is_some_and(|v| record.instance != v)
    {
        return Err(unsupported());
    }
    Ok(())
}

pub(super) fn qualify(word: &[u8], table: &[u8]) -> Result<Option<DrawingProof>> {
    inspect_streams(word, table)
}

/// Qualify bounded OfficeArt and FSPA ownership from complete WordDocument and
/// selected table streams. FIB offsets and story lengths are checked before reads;
/// returned shape CPs still require binding to the caller's authored story text.
pub fn inspect_streams(word: &[u8], table: &[u8]) -> Result<Option<DrawingProof>> {
    let (_, _, lengths, _, _) = crate::office::doc::document::fib(word)?;
    let start = u32_at(word, 154 + 50 * 8)? as usize;
    let length = u32_at(word, 158 + 50 * 8)? as usize;
    let anchor_length = u32_at(word, 158 + 40 * 8)? as usize;
    if length == 0 {
        if anchor_length != 0 || u32_at(word, 158 + 41 * 8)? != 0 {
            return Err(unsupported());
        }
        return Ok(None);
    }
    let anchors = textbox::anchors(
        bytes(table, u32_at(word, 154 + 40 * 8)? as usize, anchor_length)?,
        lengths[0],
    )?;
    let mut header_anchors = textbox::anchors(
        bytes(
            table,
            u32_at(word, 154 + 41 * 8)? as usize,
            u32_at(word, 158 + 41 * 8)? as usize,
        )?,
        lengths[2],
    )?;
    for anchor in &mut header_anchors {
        anchor.cp += lengths[0] + lengths[1];
    }
    if length > 4 * 1024 * 1024 {
        return Err(Error::Limit("render drawing bytes"));
    }
    qualify_with_story_anchors(bytes(table, start, length)?, start, anchors, header_anchors)
        .map(Some)
}

#[cfg(test)]
fn qualify_content(data: &[u8], base: usize) -> Result<DrawingProof> {
    qualify_with_story_anchors(data, base, Vec::new(), Vec::new())
}

#[cfg(test)]
fn qualify_with_anchors(
    data: &[u8],
    base: usize,
    anchors: Vec<textbox::RawAnchor>,
) -> Result<DrawingProof> {
    qualify_with_story_anchors(data, base, anchors, Vec::new())
}

fn qualify_with_story_anchors(
    data: &[u8],
    base: usize,
    anchors: Vec<textbox::RawAnchor>,
    header_anchors: Vec<textbox::RawAnchor>,
) -> Result<DrawingProof> {
    let mut reader = Reader {
        records: Vec::new(),
        shape_ids: Default::default(),
        backgrounds: 0,
        anchors,
        used_anchors: Default::default(),
        textboxes: Vec::new(),
        images: Vec::new(),
        lines: Vec::new(),
        rectangles: Vec::new(),
        header_anchors,
        used_header_anchors: Default::default(),
        store_count: 0,
        omitted_shapes: Vec::new(),
        group_approximations: Vec::new(),
        document_defaults: Vec::new(),
        default_text_properties: Default::default(),
    };
    let mut at = 0;
    let root = reader.record(data, &mut at, base)?;
    require(&root, 0xf000, 15, Some(0))?;
    let mut root_at = 0;
    let info = reader.record(root.data, &mut root_at, root.payload_offset)?;
    require(&info, 0xf006, 0, Some(0))?;
    if info.data.len() < 16
        || (info.data.len() - 16) % 8 != 0
        || u32_at(info.data, 4)? as usize != (info.data.len() - 16) / 8 + 1
    {
        return Err(unsupported());
    }
    if root_at < root.data.len() && u16_at(root.data, root_at + 2)? == 0xf001 {
        let store = reader.record(root.data, &mut root_at, root.payload_offset)?;
        require(&store, 0xf001, 15, None)?;
        let mut pos = 0;
        while pos < store.data.len() {
            let entry = reader.record(store.data, &mut pos, store.payload_offset)?;
            require(&entry, 0xf007, 2, None)?;
            if entry.data.len() < 36 {
                return Err(unsupported());
            }
            reader.store_count += 1;
        }
        if reader.store_count != store.instance as usize {
            return Err(unsupported());
        }
    }
    let mut root_properties = std::collections::BTreeSet::new();
    while root_at < root.data.len() {
        let record = reader.record(root.data, &mut root_at, root.payload_offset)?;
        if !root_properties.insert(record.kind) {
            return Err(unsupported());
        }
        match record.kind {
            0xf00b | 0xf121 | 0xf122 => {
                if let Some(evidence) = defaults::decode(&record)? {
                    for &(key, value) in &evidence.applied_text_properties {
                        if reader
                            .default_text_properties
                            .get(&key)
                            .is_some_and(|old| *old != value)
                        {
                            return Err(Error::Unsupported(
                                "conflicting drawing default inset tables",
                            ));
                        }
                        reader.default_text_properties.insert(key, value);
                    }
                    reader.document_defaults.push(evidence);
                }
            }
            0xf11a | 0xf11e => {
                require(&record, record.kind, 0, None)?;
                if record.data.len() != usize::from(record.instance) * 4 {
                    return Err(unsupported());
                }
            }
            _ => return Err(unsupported()),
        }
    }
    if root_at != root.data.len() {
        return Err(unsupported());
    }
    let mut stories = std::collections::BTreeSet::new();
    while at < data.len() {
        let story = data[at];
        at += 1;
        if story > 1 || !stories.insert(story) {
            return Err(unsupported());
        }
        let drawing = reader.record(data, &mut at, base)?;
        reader.drawing(drawing, story)?;
    }
    if reader.used_anchors.len() != reader.anchors.len()
        || reader.used_header_anchors.len() != reader.header_anchors.len()
    {
        return Err(unsupported());
    }
    let basis = if !reader.document_defaults.is_empty()
        || !reader.omitted_shapes.is_empty()
        || !reader.group_approximations.is_empty()
    {
        "source_owned_partial_scene_with_retained_unapplied_drawing_properties"
    } else if !reader.images.is_empty() {
        "source_owned_raster_overlay_frames_require_asset_and_page_validation"
    } else if !reader.rectangles.is_empty() {
        "source_owned_solid_header_rectangles_require_anchor_and_paint_validation"
    } else if !reader.lines.is_empty() {
        "source_owned_solid_header_lines_require_anchor_and_paint_validation"
    } else if reader.textboxes.is_empty() {
        "normal_color_explicit_no_fill_no_line_no_text_no_inherited_effects"
    } else {
        "source_owned_rectangular_textboxes_with_qualified_overlay_frames"
    };
    Ok(DrawingProof {
        table_range: [base, base + data.len()],
        records: reader.records,
        background_shapes: reader.backgrounds,
        textboxes: reader.textboxes,
        images: reader.images,
        lines: reader.lines,
        rectangles: reader.rectangles,
        omitted_shapes: reader.omitted_shapes,
        group_approximations: reader.group_approximations,
        document_defaults: reader.document_defaults,
        basis,
    })
}

#[cfg(test)]
mod tests;
