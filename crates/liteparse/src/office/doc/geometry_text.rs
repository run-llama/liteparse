//! Saved OfficeArt geometry text, owned by stream bytes rather than story character positions.
use crate::office::doc::{Error, Result, bytes, compound::Compound, document::fib, u16_at, u32_at};
use serde::Serialize;
mod properties;

#[derive(Debug, Serialize)]
pub struct GeometryText {
    pub status: &'static str,
    pub source_stream: &'static str,
    pub drawing_table_range: [usize; 2],
    pub shapes: Vec<ShapeText>,
}
#[derive(Debug, Serialize)]
pub struct ShapeText {
    pub story: &'static str,
    pub shape_id: u32,
    pub shape_type: u16,
    pub shape_flags: u32,
    pub shape_table_range: [usize; 2],
    pub source_text: Option<String>,
    pub candidates: Vec<properties::TextProperty>,
    pub unresolved: Vec<&'static str>,
}

#[derive(Clone, Copy)]
struct Record {
    start: usize,
    body: usize,
    end: usize,
    version: u16,
    instance: u16,
    kind: u16,
}
struct Reader<'a> {
    data: &'a [u8],
    base: usize,
    records_left: usize,
    work_left: usize,
    text_left: usize,
    shapes: Vec<ShapeText>,
}
impl Reader<'_> {
    fn record(&mut self, at: usize, end: usize) -> Result<Record> {
        self.records_left = self
            .records_left
            .checked_sub(1)
            .ok_or(Error::Limit("geometry text records"))?;
        let header = bytes(self.data, at, 8)?;
        let bits = u16_at(header, 0)?;
        let body = at
            .checked_add(8)
            .ok_or(Error::Limit("geometry text offset"))?;
        let stop = body
            .checked_add(u32_at(header, 4)? as usize)
            .filter(|n| *n <= end && *n <= self.data.len())
            .ok_or(Error::Invalid("geometry text record length"))?;
        self.work_left = self
            .work_left
            .checked_sub(stop - at)
            .ok_or(Error::Limit("geometry text record work"))?;
        Ok(Record {
            start: at,
            body,
            end: stop,
            version: bits & 15,
            instance: bits >> 4,
            kind: u16_at(header, 2)?,
        })
    }
    fn walk(&mut self, container: Record, story: &'static str, depth: usize) -> Result<()> {
        if depth > 32 {
            return Err(Error::Limit("geometry text group depth"));
        }
        let mut at = container.body;
        while at < container.end {
            let record = self.record(at, container.end)?;
            at = record.end;
            if record.kind == 0xf004 {
                self.shape(record, story)?;
            } else if record.version == 15 {
                self.walk(record, story, depth + 1)?;
            }
        }
        Ok(())
    }
    fn shape(&mut self, shape: Record, story: &'static str) -> Result<()> {
        if shape.version != 15 || shape.instance != 0 {
            return Err(Error::Invalid("geometry text shape container"));
        }
        let mut at = shape.body;
        let mut identity = None;
        let mut candidates = Vec::new();
        while at < shape.end {
            let record = self.record(at, shape.end)?;
            at = record.end;
            match record.kind {
                0xf00a => {
                    if identity.is_some() || record.version != 2 || record.end - record.body != 8 {
                        return Err(Error::Invalid("geometry text shape identity"));
                    }
                    identity = Some((
                        u32_at(self.data, record.body)?,
                        record.instance,
                        u32_at(self.data, record.body + 4)?,
                    ));
                }
                0xf00b | 0xf121 | 0xf122 => {
                    if record.version != 3 {
                        return Err(Error::Invalid("geometry text property table version"));
                    }
                    candidates.extend(properties::decode(
                        bytes(self.data, record.body, record.end - record.body)?,
                        record.instance as usize,
                        self.base + record.body,
                        record.kind,
                        &mut self.text_left,
                    )?);
                }
                _ => {}
            }
        }
        if candidates.is_empty() {
            return Ok(());
        }
        let (shape_id, shape_type, shape_flags) =
            identity.ok_or(Error::Invalid("geometry text missing shape identity"))?;
        if self.shapes.len() >= 8192 {
            return Err(Error::Limit("geometry text shapes"));
        }
        let source_text = (candidates.len() == 1).then(|| candidates[0].text.clone());
        let mut unresolved = vec![
            "visibility_not_resolved",
            "source_record_order_not_document_reading_order",
            "geometry_text_layout_not_rendered",
        ];
        if source_text.is_none() {
            unresolved.push("multiple_geometry_text_assignments_no_effective_value_selected");
        }
        self.shapes.push(ShapeText {
            story,
            shape_id,
            shape_type,
            shape_flags,
            shape_table_range: [self.base + shape.start, self.base + shape.end],
            source_text,
            candidates,
            unresolved,
        });
        Ok(())
    }
}

/// Extracts explicit saved geometry strings without inserting text into CLX stories.
/// MS-ODRAW §§2.2.7–11, 2.3.1 and 2.3.22.1–2 define property ownership and UTF-16 text.
pub fn inspect_geometry_text(data: &[u8]) -> Result<GeometryText> {
    let compound = Compound::open(data)?;
    let word = compound.stream("WordDocument")?;
    let (_, table_name, _, _, _) = fib(&word)?;
    let table = compound.stream(table_name)?;
    let start = u32_at(&word, 154 + 50 * 8)? as usize;
    let length = u32_at(&word, 158 + 50 * 8)? as usize;
    if length > 16 * 1024 * 1024 {
        return Err(Error::Limit("geometry text drawing bytes"));
    }
    let raw = if length == 0 {
        &[][..]
    } else {
        bytes(&table, start, length)?
    };
    decode(raw, start, table_name)
}
fn decode(data: &[u8], base: usize, stream: &'static str) -> Result<GeometryText> {
    let mut reader = Reader {
        data,
        base,
        records_left: 262_144,
        work_left: 128 * 1024 * 1024,
        text_left: 4 * 1024 * 1024,
        shapes: Vec::new(),
    };
    if !data.is_empty() {
        let root = reader.record(0, data.len())?;
        if root.kind != 0xf000 || root.version != 15 || root.instance != 0 {
            return Err(Error::Invalid("geometry text drawing group"));
        }
        let mut at = root.end;
        let mut stories = std::collections::BTreeSet::new();
        while at < data.len() {
            let label = data[at];
            at += 1;
            if label > 1 || !stories.insert(label) {
                return Err(Error::Invalid("geometry text drawing story"));
            }
            let drawing = reader.record(at, data.len())?;
            at = drawing.end;
            if drawing.kind != 0xf002 || drawing.version != 15 {
                return Err(Error::Invalid("geometry text drawing container"));
            }
            reader.walk(drawing, if label == 0 { "body" } else { "headers" }, 0)?;
        }
    }
    Ok(GeometryText {
        status: if reader.shapes.is_empty() {
            "no_explicit_geometry_text"
        } else {
            "source_text_available_display_order_and_visibility_unresolved"
        },
        source_stream: stream,
        drawing_table_range: [base, base + data.len()],
        shapes: reader.shapes,
    })
}
#[cfg(test)]
mod tests;
