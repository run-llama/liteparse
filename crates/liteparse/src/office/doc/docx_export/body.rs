//! Story text, paragraphs, tables, fields, notes, headers, lists and drawings written
//! as WordprocessingML parts. Every story range is emitted through `blocks`.
use super::package::{self, NS, Part};
use super::props::{self, CharFlags, ICO, ParaFlags, Section, border, colorref, esc, rpr};
use super::source::{Source, i16_le, pair, u16_le, u32_le};
use super::styles::Styles;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;

/// A wrap polygon around the whole shape box (21600 units per side).
macro_rules! box_polygon {
    () => {
        concat!(
            "<wp:wrapPolygon edited=\"0\"><wp:start x=\"0\" y=\"0\"/>",
            "<wp:lineTo x=\"0\" y=\"21600\"/><wp:lineTo x=\"21600\" y=\"21600\"/>",
            "<wp:lineTo x=\"21600\" y=\"0\"/><wp:lineTo x=\"0\" y=\"0\"/></wp:wrapPolygon>"
        )
    };
}

const STORY_FOOTNOTE: usize = 1;
const STORY_HEADER: usize = 2;
const STORY_ANNOTATION: usize = 4;
const STORY_ENDNOTE: usize = 5;
const STORY_TEXTBOX: usize = 6;
const STORY_HEADER_TEXTBOX: usize = 7;

struct Para {
    start: usize,
    end: usize,
    istd: u16,
    grpprl: Vec<u8>,
    flags: ParaFlags,
}

/// File shape anchor (FSPA): rectangle in twips relative to its anchor frame.
#[derive(Clone, Copy)]
struct Fspa {
    spid: u32,
    rect: [i32; 4],
    flags: u16,
}

pub struct Writer<'a> {
    source: &'a Source,
    styles: &'a Styles,
    fonts: Vec<String>,
    /// Names of symbol-charset fonts (charset 2), whose glyphs replace Latin letters.
    symbol_fonts: Vec<String>,
    /// fontTable.xml entries: (name, alternate name, family class, charset).
    font_entries: Vec<(String, Option<String>, u8, u8)>,
    /// WPS Office writes a header's closing empty paragraph as content;
    /// Word for Mac shows it (a WPS form's header: two lines in Word's render).
    keeps_story_tail: bool,
    paras: Vec<Para>,
    /// Asset index for each picture marker CP, and for each shape id.
    picture_at_cp: HashMap<usize, usize>,
    picture_by_shape: HashMap<u32, usize>,
    assets: Vec<(String, Vec<u8>)>,
    shapes: HashMap<usize, Fspa>,
    /// Paint, type, order and group membership per shape id from the OfficeArt records.
    drawing: super::drawing::Drawing,
    /// Asset index of each picture-store entry (by `pib` - 1), from the lenient reader.
    blip_store: Vec<Option<usize>>,
    textboxes: HashMap<u32, (usize, usize)>,
    /// Textbox story entries in PLC order: [main document, headers].
    textbox_entries: [Vec<(usize, usize)>; 2],
    footnotes: HashMap<usize, usize>,
    endnotes: HashMap<usize, usize>,
    /// Comment id by reference CP, and each comment's (start, end, author, initials).
    comments: HashMap<usize, usize>,
    comment_items: Vec<(usize, usize, String, String)>,
    note_ranges: [Vec<(usize, usize)>; 2],
    section_marks: HashMap<usize, String>,
    lists: Option<crate::office::doc::lists::ListDefinitions>,
    rels: Vec<(String, &'static str, String)>,
    media: Vec<Part>,
    media_names: HashMap<(usize, usize), String>,
    next_id: usize,
    in_textbox: usize,
    counts: BTreeMap<&'static str, usize>,
    /// CP ranges [separator, end) holding the cached result of TOC fields.
    toc_results: Vec<(usize, usize)>,
    /// Result ranges of SHAPE fields: their shapes are inline.
    shape_results: Vec<(usize, usize)>,
    /// Main-story bookmarks by their first CP: written at the start of the paragraph
    /// holding that CP, so PAGEREF fields (table of contents page numbers) find their
    /// target's page.
    bookmark_starts: BTreeMap<usize, Vec<String>>,
    pub diagnostics: Vec<String>,
}

impl<'a> Writer<'a> {
    pub fn new(source: &'a Source, styles: &'a Styles, data: &[u8]) -> Self {
        let table = source
            .pair(pair::STTBFFFN)
            .and_then(|b| crate::office::doc::symbols::decode_font_table_lenient(b).ok())
            .map(|t| t.fonts)
            .unwrap_or_default();
        let symbol_fonts = table
            .iter()
            .filter(|f| f.charset == 2)
            .map(|f| f.name.clone())
            .collect();
        let font_entries = table
            .iter()
            .map(|f| {
                (
                    word_doc_face(&f.name, f.alternate_name.as_deref(), &f.signature),
                    f.alternate_name.clone(),
                    (f.family >> 4) & 7,
                    f.charset,
                )
            })
            .collect();
        let fonts = table
            .iter()
            .map(|f| word_doc_face(&f.name, f.alternate_name.as_deref(), &f.signature))
            .collect();
        let mut writer = Writer {
            source,
            styles,
            fonts,
            symbol_fonts,
            font_entries,
            keeps_story_tail: super::summary::read(data)
                .app
                .iter()
                .any(|(k, v)| *k == "Application" && v.contains("WPS")),
            paras: Vec::new(),
            picture_at_cp: HashMap::new(),
            picture_by_shape: HashMap::new(),
            assets: Vec::new(),
            shapes: HashMap::new(),
            drawing: source
                .pair(pair::DGGINFO)
                .map(super::drawing::read)
                .unwrap_or_default(),
            blip_store: Vec::new(),
            textboxes: HashMap::new(),
            textbox_entries: [Vec::new(), Vec::new()],
            footnotes: HashMap::new(),
            endnotes: HashMap::new(),
            comments: HashMap::new(),
            comment_items: Vec::new(),
            note_ranges: [Vec::new(), Vec::new()],
            section_marks: HashMap::new(),
            lists: crate::office::doc::lists::inspect_lists(data).ok(),
            rels: Vec::new(),
            media: Vec::new(),
            media_names: HashMap::new(),
            next_id: 1,
            in_textbox: 0,
            counts: BTreeMap::new(),
            toc_results: field_results(&source.text, "TOC"),
            shape_results: field_results(&source.text, "SHAPE"),
            bookmark_starts: body_bookmarks(data),
            diagnostics: Vec::new(),
        };
        writer.read_paras();
        writer.read_pictures(data);
        writer.read_blip_store();
        writer.read_shapes();
        writer.read_notes();
        writer.read_comments();
        writer
    }

    fn count(&mut self, what: &'static str) {
        *self.counts.entry(what).or_default() += 1;
    }

    fn read_paras(&mut self) {
        let mut paras = Vec::with_capacity(self.source.paras.len());
        for p in &self.source.paras {
            let mut flags = self.styles.para_flags(p.istd);
            props::ppr(&p.grpprl, &mut flags);
            paras.push(Para {
                start: p.start_cp,
                end: p.end_cp,
                istd: p.istd,
                grpprl: p.grpprl.clone(),
                flags,
            });
        }
        self.paras = paras;
    }

    /// Picture-store BLIPs for floating pictures the strict inspector did not resolve.
    fn read_blip_store(&mut self) {
        let Some(content) = self.source.pair(pair::DGGINFO) else {
            return;
        };
        for entry in super::blips::store(content, &self.source.word) {
            self.blip_store.push(entry.map(|(format, bytes)| {
                self.assets.push((format.to_string(), bytes));
                self.assets.len() - 1
            }));
        }
    }

    fn read_pictures(&mut self, data: &[u8]) {
        let Ok(pictures) = crate::office::doc::pictures::inspect_pictures(data) else {
            self.count("pictures_unreadable");
            return;
        };
        for asset in &pictures.assets {
            self.assets
                .push((asset.format.to_string(), asset.bytes().to_vec()));
        }
        for r in &pictures.references {
            let Some(asset) = r.asset_index else { continue };
            if let Some(cp) = r.source_cp {
                self.picture_at_cp.entry(cp).or_insert(asset);
            }
            if let Some(spid) = r.shape_id {
                self.picture_by_shape.entry(spid).or_insert(asset);
            }
        }
    }

    fn read_shapes(&mut self) {
        let s = self.source;
        for (index, base) in [
            (pair::PLCSPAMOM, 0),
            (pair::PLCSPAHDR, s.story_start(STORY_HEADER)),
        ] {
            let Some(plc) = s.pair(index) else { continue };
            let n = plc.len().saturating_sub(4) / 30;
            for i in 0..n {
                let at = 4 * (n + 1) + i * 26;
                let (Some(cp), Some(spid)) = (u32_le(plc, i * 4), u32_le(plc, at)) else {
                    continue;
                };
                let rect = [0, 1, 2, 3].map(|k| u32_le(plc, at + 4 + k * 4).unwrap_or(0) as i32);
                self.shapes.insert(
                    base + cp as usize,
                    Fspa {
                        spid,
                        rect,
                        flags: u16_le(plc, at + 20).unwrap_or(0),
                    },
                );
            }
        }
        for (k, (index, story)) in [
            (pair::PLCFTXBXTXT, STORY_TEXTBOX),
            (pair::PLCFHDRTXBXTXT, STORY_HEADER_TEXTBOX),
        ]
        .into_iter()
        .enumerate()
        {
            let Some(plc) = s.pair(index) else { continue };
            let n = plc.len().saturating_sub(4) / 26;
            let base = s.story_start(story);
            for i in 0..n {
                let (Some(a), Some(b)) = (u32_le(plc, i * 4), u32_le(plc, i * 4 + 4)) else {
                    continue;
                };
                self.textbox_entries[k].push((base + a as usize, base + b as usize));
                let Some(lid) = u32_le(plc, 4 * (n + 1) + i * 22 + 14) else {
                    continue;
                };
                if b > a {
                    self.textboxes
                        .insert(lid, (base + a as usize, base + b as usize));
                }
            }
        }
    }

    fn read_notes(&mut self) {
        let s = self.source;
        for (k, (refs, txt, story)) in [
            (pair::PLCFFNDREF, pair::PLCFFNDTXT, STORY_FOOTNOTE),
            (pair::PLCFENDREF, pair::PLCFENDTXT, STORY_ENDNOTE),
        ]
        .into_iter()
        .enumerate()
        {
            let (Some(r), Some(t)) = (s.pair(refs), s.pair(txt)) else {
                continue;
            };
            let n = r.len().saturating_sub(4) / 6;
            let base = s.story_start(story);
            for i in 0..n {
                let (Some(cp), Some(a), Some(b)) =
                    (u32_le(r, i * 4), u32_le(t, i * 4), u32_le(t, i * 4 + 4))
                else {
                    continue;
                };
                // Note ids 0 and 1 are reserved for the separator notes.
                let id = i + 2;
                if k == 0 {
                    self.footnotes.insert(cp as usize, id);
                } else {
                    self.endnotes.insert(cp as usize, id);
                }
                self.note_ranges[k].push((base + a as usize, base + b as usize));
            }
        }
    }

    /// Comments: PlcfandRef holds reference CPs and 30-byte ATRD records (initials as a
    /// counted UTF-16 string at 0, author index at 20); PlcfandTxt holds each comment's
    /// range in the annotation story; GrpXstAtnOwners lists author names.
    fn read_comments(&mut self) {
        let s = self.source;
        let (Some(r), Some(t)) = (s.pair(pair::PLCFANDREF), s.pair(pair::PLCFANDTXT)) else {
            return;
        };
        let mut authors = Vec::new();
        let raw = s.pair(pair::GRPXSTATNOWNERS).unwrap_or(&[]);
        let mut at = 0;
        while let Some(len) = u16_le(raw, at) {
            let units: Vec<u16> = (0..len as usize)
                .filter_map(|k| u16_le(raw, at + 2 + k * 2))
                .collect();
            authors.push(String::from_utf16_lossy(&units));
            at += 2 + len as usize * 2;
        }
        let n = r.len().saturating_sub(4) / 34;
        let base = s.story_start(STORY_ANNOTATION);
        for i in 0..n {
            let (Some(cp), Some(a), Some(b)) =
                (u32_le(r, i * 4), u32_le(t, i * 4), u32_le(t, i * 4 + 4))
            else {
                continue;
            };
            let atrd = (n + 1) * 4 + i * 30;
            let initial_len = u16_le(r, atrd).unwrap_or(0).min(9) as usize;
            let initials: Vec<u16> = (0..initial_len)
                .filter_map(|k| u16_le(r, atrd + 2 + k * 2))
                .collect();
            let author = u16_le(r, atrd + 20)
                .and_then(|i| authors.get(i as usize).cloned())
                .unwrap_or_default();
            self.comments.insert(cp as usize, self.comment_items.len());
            self.comment_items.push((
                base + a as usize,
                base + b as usize,
                author,
                String::from_utf16_lossy(&initials),
            ));
        }
    }

    /// Paragraph indexes whose mark lies in [start, end).
    fn para_range(&self, start: usize, end: usize) -> std::ops::Range<usize> {
        let a = self.paras.partition_point(|p| p.end <= start);
        let b = self.paras.partition_point(|p| p.end <= end);
        a..b.max(a)
    }

    /// Block XML for the paragraphs ending in [start, end), with tables rebuilt from
    /// paragraph table depth. Always yields at least one paragraph.
    fn blocks(&mut self, start: usize, end: usize) -> String {
        let range = self.para_range(start, end);
        let indexes: Vec<usize> = range.collect();
        let mut out = String::new();
        self.emit(&indexes, 0, &mut out);
        if !out.contains("<w:p>") && !out.contains("<w:p ") {
            out.push_str("<w:p/>");
        }
        out
    }

    fn emit(&mut self, indexes: &[usize], depth: u32, out: &mut String) {
        let mut k = 0;
        while k < indexes.len() {
            if self.paras[indexes[k]].flags.depth() > depth {
                let mut m = k;
                while m < indexes.len() && self.paras[indexes[m]].flags.depth() > depth {
                    m += 1;
                }
                self.table(&indexes[k..m], depth + 1, out);
                k = m;
            } else {
                self.paragraph(indexes[k], out);
                k += 1;
            }
        }
    }

    fn is_row_end(&self, i: usize, depth: u32) -> bool {
        let p = &self.paras[i];
        if p.flags.depth() != depth {
            return false;
        }
        if depth == 1 {
            p.flags.ttp
        } else {
            p.flags.inner_ttp
        }
    }

    fn is_cell_end(&self, i: usize, depth: u32) -> bool {
        let p = &self.paras[i];
        if p.flags.depth() != depth {
            return false;
        }
        if depth == 1 {
            self.source.text.get(p.end - 1) == Some(&0x07)
        } else {
            p.flags.inner_cell
        }
    }

    fn table(&mut self, indexes: &[usize], depth: u32, out: &mut String) {
        // Rows: cell paragraph groups closed by a row-end paragraph carrying TAPX.
        let mut rows: Vec<(Vec<Vec<usize>>, Option<usize>)> = Vec::new();
        let mut cells: Vec<Vec<usize>> = Vec::new();
        let mut cell: Vec<usize> = Vec::new();
        for &i in indexes {
            if self.is_row_end(i, depth) {
                if !cell.is_empty() {
                    cells.push(std::mem::take(&mut cell));
                }
                rows.push((std::mem::take(&mut cells), Some(i)));
                continue;
            }
            cell.push(i);
            if self.is_cell_end(i, depth) {
                cells.push(std::mem::take(&mut cell));
            }
        }
        if !cell.is_empty() {
            cells.push(cell);
        }
        if !cells.is_empty() {
            self.count("table_row_without_end");
            rows.push((cells, None));
        }
        let rows: Vec<(Vec<Vec<usize>>, Row)> = rows
            .into_iter()
            .map(|(cells, end)| {
                let row = end.map_or_else(Row::default, |i| Row::read(&self.paras[i].grpprl));
                (cells, row)
            })
            .collect();
        // One grid over all row boundaries; each cell spans the columns it covers.
        let mut edges: Vec<i32> = Vec::new();
        for (cells, row) in &rows {
            if row.centers.len() >= 2 {
                edges.extend(row.centers.iter().map(|&c| c as i32));
            } else {
                let n = cells.len().max(1) as i32;
                edges.extend((0..=n).map(|k| k * 9000 / n));
            }
        }
        edges.sort_unstable();
        edges.dedup_by(|a, b| (*a - *b).abs() <= 5);
        if edges.len() < 2 {
            edges = vec![0, 9000];
        }
        let col = |x: i32| {
            edges
                .iter()
                .enumerate()
                .min_by_key(|&(_, &e)| (e - x).abs())
                .map_or(0, |(i, _)| i)
        };
        let first_row = &rows[0].1;
        let table_start = out.len();
        out.push_str("<w:tbl><w:tblPr>");
        let pct = autofit_percent_width(first_row, edges.len() - 1);
        match pct {
            Some(w) => write!(out, "<w:tblW w:w=\"{w}\" w:type=\"pct\"/>").unwrap(),
            None => out.push_str("<w:tblW w:w=\"0\" w:type=\"auto\"/>"),
        }
        if let Some(j) = first_row.jc {
            let v = match j {
                1 => "center",
                2 => "right",
                _ => "left",
            };
            write!(out, "<w:jc w:val=\"{v}\"/>").unwrap();
        }
        if let Some(w) = first_row.spacing {
            write!(out, "<w:tblCellSpacing w:w=\"{w}\" w:type=\"dxa\"/>").unwrap();
        }
        let gap = first_row.gap_half;
        // tblInd reaches the first cell's text: the cell edge plus its left margin.
        let left_margin = first_row.padding[1].unwrap_or(gap);
        let left = first_row.centers.first().copied().unwrap_or(0) as i32 + left_margin as i32;
        if first_row.jc.unwrap_or(0) == 0 {
            write!(out, "<w:tblInd w:w=\"{left}\" w:type=\"dxa\"/>").unwrap();
        }
        if !first_row.borders.is_empty() {
            write!(out, "<w:tblBorders>{}</w:tblBorders>", first_row.borders).unwrap();
        }
        if pct.is_none() {
            out.push_str("<w:tblLayout w:type=\"fixed\"/>");
        }
        let [top, left, bottom, right] = first_row.padding;
        out.push_str("<w:tblCellMar>");
        for (side, value) in [
            ("top", top),
            ("left", left.or(Some(gap))),
            ("bottom", bottom),
            ("right", right.or(Some(gap))),
        ] {
            if let Some(w) = value {
                write!(out, "<w:{side} w:w=\"{w}\" w:type=\"dxa\"/>").unwrap();
            }
        }
        out.push_str("</w:tblCellMar>");
        out.push_str("<w:tblLook w:val=\"0000\"/></w:tblPr><w:tblGrid>");
        for w in edges.windows(2) {
            write!(out, "<w:gridCol w:w=\"{}\"/>", w[1] - w[0]).unwrap();
        }
        out.push_str("</w:tblGrid>");
        for (cells, row) in rows {
            out.push_str("<w:tr><w:trPr>");
            if row.header {
                out.push_str("<w:tblHeader/>");
            }
            if let Some(h) = row.height {
                if h < 0 {
                    write!(
                        out,
                        "<w:trHeight w:val=\"{}\" w:hRule=\"exact\"/>",
                        -(h as i32)
                    )
                    .unwrap();
                } else if h > 0 {
                    write!(out, "<w:trHeight w:val=\"{h}\"/>").unwrap();
                }
            }
            if row.cant_split {
                out.push_str("<w:cantSplit/>");
            }
            let n = cells.len();
            let centers: Vec<i32> = if row.centers.len() > n {
                row.centers.iter().map(|&c| c as i32).collect()
            } else {
                let (a, b) = (edges[0], *edges.last().unwrap());
                (0..=n as i32)
                    .map(|k| a + (b - a) * k / n.max(1) as i32)
                    .collect()
            };
            let before = col(centers[0]);
            let last = col(centers[n.min(centers.len() - 1)]);
            if before > 0 {
                write!(out, "<w:gridBefore w:val=\"{before}\"/>").unwrap();
            }
            if last + 1 < edges.len() {
                write!(out, "<w:gridAfter w:val=\"{}\"/>", edges.len() - 1 - last).unwrap();
            }
            out.push_str("</w:trPr>");
            // Word97 horizontal merges: fMerged cells fold into the preceding fFirstMerged.
            let mut c = 0;
            while c < n {
                let tc = row.cells.get(c).cloned().unwrap_or_default();
                let mut span_end = c + 1;
                if tc.grf & 1 != 0 {
                    while span_end < n && row.cells.get(span_end).is_some_and(|t| t.grf & 2 != 0) {
                        span_end += 1;
                    }
                }
                let a = col(centers[c.min(centers.len() - 1)]);
                let b = col(centers[span_end.min(centers.len() - 1)]);
                let span = b.saturating_sub(a).max(1);
                let width =
                    centers[span_end.min(centers.len() - 1)] - centers[c.min(centers.len() - 1)];
                write!(
                    out,
                    "<w:tc><w:tcPr><w:tcW w:w=\"{}\" w:type=\"dxa\"/>",
                    width.max(0)
                )
                .unwrap();
                if span > 1 {
                    write!(out, "<w:gridSpan w:val=\"{span}\"/>").unwrap();
                }
                if tc.grf & 0x40 != 0 {
                    out.push_str("<w:vMerge w:val=\"restart\"/>");
                } else if tc.grf & 0x20 != 0 {
                    out.push_str("<w:vMerge/>");
                }
                let mut b = String::new();
                for (k, name) in ["top", "left", "bottom", "right"].iter().enumerate() {
                    if let Some(x) = row
                        .cell_borders
                        .get(&(c, k))
                        .cloned()
                        .or_else(|| tc.borders[k].clone())
                    {
                        b.push_str(&x.replace("<w:top", &format!("<w:{name}")));
                    }
                }
                if !b.is_empty() {
                    write!(out, "<w:tcBorders>{b}</w:tcBorders>").unwrap();
                }
                let mar: String = ["top", "left", "bottom", "right"]
                    .iter()
                    .enumerate()
                    .filter_map(|(k, name)| {
                        let w = row.cell_padding.get(&(c, k))?;
                        Some(format!(
                            "<w:{name} w:w=\"{}\" w:type=\"dxa\"/>",
                            (*w).max(0)
                        ))
                    })
                    .collect();

                if let Some(fill) = row.shading.get(c).filter(|f| !f.is_empty()) {
                    write!(
                        out,
                        "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{fill}\"/>"
                    )
                    .unwrap();
                }
                if !mar.is_empty() {
                    write!(out, "<w:tcMar>{mar}</w:tcMar>").unwrap();
                }
                // TC80 fVertical (0x4) rotates the cell text; fBackward (0x8) reads it bottom-up.
                match tc.grf & 0xc {
                    0xc => out.push_str("<w:textDirection w:val=\"btLr\"/>"),
                    0x4 => out.push_str("<w:textDirection w:val=\"tbRl\"/>"),
                    _ => {}
                }
                let v = (tc.grf >> 7) & 3;
                if v == 1 {
                    out.push_str("<w:vAlign w:val=\"center\"/>");
                } else if v == 2 {
                    out.push_str("<w:vAlign w:val=\"bottom\"/>");
                }
                out.push_str("</w:tcPr>");
                let mut content = String::new();
                for cell in &cells[c..span_end.min(n)] {
                    self.emit(cell, depth, &mut content);
                }
                // A cell must end with a paragraph, also after a nested table.
                if !content.contains("<w:p") || content.ends_with("</w:tbl>") {
                    content.push_str("<w:p/>");
                }
                out.push_str(&content);
                out.push_str("</w:tc>");
                c = span_end;
            }
            if n == 0 {
                out.push_str("<w:tc><w:p/></w:tc>");
            }
            out.push_str("</w:tr>");
        }
        out.push_str("</w:tbl>");
        if depth == 1 {
            let table = out.split_off(table_start);
            out.push_str(&float_framed_table(table));
        }
    }

    fn paragraph(&mut self, i: usize, out: &mut String) {
        let (start, end, istd) = (self.paras[i].start, self.paras[i].end, self.paras[i].istd);
        let mark = self.source.text.get(end - 1).copied().unwrap_or(0x0d);
        let text_end = if matches!(mark, 0x0d | 0x07 | 0x0c) {
            end - 1
        } else {
            end
        };
        out.push_str("<w:p><w:pPr>");
        if self.styles.get(istd).is_some_and(|s| s.kind == 1) {
            write!(out, "<w:pStyle w:val=\"s{istd}\"/>").unwrap();
        }
        let grpprl = self.paras[i].grpprl.clone();
        let direct_list = super::source::sprms(&grpprl).any(|(s, _)| s == 0x460b || s == 0x260a);
        let flags = self.paras[i].flags.clone();
        if direct_list {
            match flags.ilfo {
                Some(ilfo) if ilfo > 0 && ilfo < 0x7ff && self.has_num(ilfo) => write!(
                    out,
                    "<w:numPr><w:ilvl w:val=\"{}\"/><w:numId w:val=\"{ilfo}\"/></w:numPr>",
                    flags.ilvl.unwrap_or(0).min(8)
                )
                .unwrap(),
                Some(0) => {
                    out.push_str("<w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"0\"/></w:numPr>")
                }
                _ => {}
            }
        }
        let mut grpprl = grpprl;
        if super::source::sprms(&grpprl).any(|(s, _)| props::is_frame_sprm(s)) {
            let mut framed = self.styles.frame_sprms(istd);
            framed.extend_from_slice(&grpprl);
            grpprl = framed;
        }
        let p = props::ppr(&grpprl, &mut ParaFlags::default());
        out.push_str(&strip_frames(&p));
        if let Some(sect) = self.section_marks.get(&end).cloned() {
            out.push_str(&sect);
        }
        let mut mark_flags = CharFlags::default();
        let mark_rpr = self.char_rpr(end - 1, &mut mark_flags);
        if !mark_rpr.is_empty() {
            write!(out, "<w:rPr>{mark_rpr}</w:rPr>").unwrap();
        }
        out.push_str("</w:pPr>");
        let names: Vec<String> = self
            .bookmark_starts
            .range(start..end)
            .flat_map(|(_, names)| names.iter().cloned())
            .collect();
        for name in names {
            let id = self.next_id();
            write!(
                out,
                "<w:bookmarkStart w:id=\"{id}\" w:name=\"{}\"/><w:bookmarkEnd w:id=\"{id}\"/>",
                esc(&name)
            )
            .unwrap();
        }
        self.runs(start, text_end, out);
        out.push_str("</w:p>");
    }

    /// Some files set a symbol font (Symbol, Wingdings) on whole paragraphs of ordinary
    /// text; rendering them would turn every letter into a Greek or pictograph glyph. A
    /// run holding a Latin word of four or more letters keeps its text and loses that font.
    fn drop_symbol_font_from_words(&mut self, rpr: String, start: usize, end: usize) -> String {
        let Some(at) = rpr.find("<w:rFonts") else {
            return rpr;
        };
        let close = at + rpr[at..].find("/>").map_or(0, |i| i + 2);
        let element = &rpr[at..close];
        let symbolic = self
            .symbol_fonts
            .iter()
            .any(|f| element.contains(&format!("w:ascii=\"{}\"", esc(f))));
        let mut letters = 0;
        let has_word = self.source.text[start..end].iter().any(|&u| {
            let c = char::from_u32(u as u32).unwrap_or(' ');
            letters = if c.is_alphabetic() && (u as u32) < 0x250 {
                letters + 1
            } else {
                0
            };
            letters >= 4
        });
        if !symbolic || !has_word {
            return rpr;
        }
        self.count("symbol_font_dropped_from_words");
        format!("{}{}", &rpr[..at], &rpr[close..])
    }

    /// Style of the paragraph holding `cp`.
    fn para_istd(&self, cp: usize) -> Option<u16> {
        let i = self.paras.partition_point(|p| p.end <= cp);
        self.paras.get(i).filter(|p| p.start <= cp).map(|p| p.istd)
    }

    fn has_num(&self, ilfo: u16) -> bool {
        self.lists
            .as_ref()
            .is_some_and(|l| (ilfo as usize) <= l.overrides.len())
    }

    fn char_rpr(&self, cp: usize, flags: &mut CharFlags) -> String {
        let mut grpprl = self.source.chpx_at(cp).to_vec();
        grpprl.extend(self.source.piece_grpprl(cp));
        // 0x81 toggles flip the governing style: the paragraph style, then the run's
        // character style on top (toggles combine as XOR across the two).
        let istd = self.para_istd(cp);
        let mut probe = CharFlags::default();
        rpr(&grpprl, &self.fonts, &mut probe);
        let mut on = istd.map_or(Vec::new(), |i| self.styles.toggles_on(i));
        if let Some(c) = probe
            .istd
            .filter(|&c| self.styles.get(c).is_some_and(|s| s.kind == 2))
        {
            for name in self.styles.toggles_on(c) {
                match on.iter().position(|&n| n == name) {
                    Some(k) => {
                        on.remove(k);
                    }
                    None => on.push(name),
                }
            }
        }
        flags.style_on = on;
        let r = rpr(&grpprl, &self.fonts, flags);
        match flags.istd {
            Some(istd) if istd != 10 && self.styles.get(istd).is_some_and(|s| s.kind == 2) => {
                format!("<w:rStyle w:val=\"s{istd}\"/>{r}")
            }
            _ => r,
        }
    }

    /// Runs for [start, end): text grouped by identical character modifiers.
    fn runs(&mut self, start: usize, end: usize, out: &mut String) {
        let mut cp = start;
        while cp < end {
            let mut flags = CharFlags::default();
            let r = self.char_rpr(cp, &mut flags);
            let span_end = {
                let spans = &self.source.chars;
                let i = spans.partition_point(|s| s.end_cp <= cp);
                spans.get(i).map_or(end, |s| s.end_cp.min(end)).max(cp + 1)
            };
            if flags.deleted {
                self.count("deleted_revision_text_dropped");
                cp = span_end;
                continue;
            }
            let r = self.drop_symbol_font_from_words(r, cp, span_end);
            let rpr_xml = if r.is_empty() {
                String::new()
            } else {
                format!("<w:rPr>{r}</w:rPr>")
            };
            let mut text: Vec<u16> = Vec::new();
            let flush = |text: &mut Vec<u16>, out: &mut String, instr: bool| {
                if text.is_empty() {
                    return;
                }
                let s = esc(&String::from_utf16_lossy(text));
                let tag = if instr { "instrText" } else { "t" };
                write!(
                    out,
                    "<w:r>{rpr_xml}<w:{tag} xml:space=\"preserve\">{s}</w:{tag}></w:r>"
                )
                .unwrap();
                text.clear();
            };
            let in_instr = self.field_instr_depth(cp) > 0;
            let mut instr = in_instr;
            for at in cp..span_end {
                let unit = self.source.text[at];
                let special = flags.special;
                let simple = |tag: &str, out: &mut String| {
                    write!(out, "<w:r>{rpr_xml}{tag}</w:r>").unwrap();
                };
                match unit {
                    0x13 => {
                        flush(&mut text, out, instr);
                        match self.checkbox(at) {
                            Some(on) => simple(
                                &format!(
                                    "<w:fldChar w:fldCharType=\"begin\"><w:ffData><w:checkBox><w:sizeAuto/>\
                                     <w:default w:val=\"{}\"/></w:checkBox></w:ffData></w:fldChar>",
                                    u8::from(on)
                                ),
                                out,
                            ),
                            None => simple("<w:fldChar w:fldCharType=\"begin\"/>", out),
                        }
                        instr = true;
                    }
                    0x14 => {
                        flush(&mut text, out, instr);
                        simple("<w:fldChar w:fldCharType=\"separate\"/>", out);
                        instr = self.field_instr_depth(at + 1) > 0;
                    }
                    0x15 => {
                        flush(&mut text, out, instr);
                        simple("<w:fldChar w:fldCharType=\"end\"/>", out);
                        instr = self.field_instr_depth(at + 1) > 0;
                    }
                    _ if instr => {
                        if unit >= 0x20 || unit == 0x09 {
                            text.push(unit);
                        }
                    }
                    0x09 => {
                        flush(&mut text, out, instr);
                        simple("<w:tab/>", out);
                    }
                    0x0b => {
                        flush(&mut text, out, instr);
                        simple("<w:br/>", out);
                    }
                    0x0c => {
                        flush(&mut text, out, instr);
                        simple("<w:br w:type=\"page\"/>", out);
                    }
                    0x0e => {
                        flush(&mut text, out, instr);
                        simple("<w:br w:type=\"column\"/>", out);
                    }
                    0x1e => {
                        flush(&mut text, out, instr);
                        simple("<w:noBreakHyphen/>", out);
                    }
                    0x1f => {
                        flush(&mut text, out, instr);
                        simple("<w:softHyphen/>", out);
                    }
                    0x05 if special => {
                        flush(&mut text, out, instr);
                        if let Some(&id) = self.comments.get(&at) {
                            simple(&format!("<w:commentReference w:id=\"{id}\"/>"), out);
                        } else if at >= self.source.story_start(STORY_ANNOTATION)
                            && at < self.source.story_start(STORY_ENDNOTE)
                        {
                            simple("<w:annotationRef/>", out);
                        }
                    }
                    0x02 if special => {
                        flush(&mut text, out, instr);
                        let tag = self.note_mark(at);
                        simple(&tag, out);
                    }
                    0x01 | 0x08 if special && self.in_toc_result(at) => {
                        self.count("toc_picture_dropped");
                    }
                    0x01 if special => {
                        flush(&mut text, out, instr);
                        let pic = self.inline_picture(at, flags.pic_location);
                        out.push_str(
                            &pic.map_or(String::new(), |p| format!("<w:r>{rpr_xml}{p}</w:r>")),
                        );
                    }
                    0x08 if special => {
                        flush(&mut text, out, instr);
                        let shape = self.floating(at);
                        out.push_str(
                            &shape.map_or(String::new(), |p| format!("<w:r>{rpr_xml}{p}</w:r>")),
                        );
                    }
                    0x28 if special && flags.symbol.is_some() => {
                        flush(&mut text, out, instr);
                        let (ftc, code) = flags.symbol.unwrap();
                        let font = self.fonts.get(ftc as usize).cloned().unwrap_or_default();
                        simple(
                            &format!("<w:sym w:font=\"{}\" w:char=\"{code:04X}\"/>", esc(&font)),
                            out,
                        );
                    }
                    u if u < 0x20 => {}
                    // A note with a custom mark ("*") keeps that text at its reference
                    // position in place of the auto-numbered 0x02 character.
                    u if !special && self.custom_note_mark(at).is_some() => {
                        flush(&mut text, out, instr);
                        let tag = self.custom_note_mark(at).unwrap_or_default();
                        simple(&tag, out);
                        text.push(u);
                    }
                    u => text.push(u),
                }
            }
            flush(&mut text, out, instr);
            cp = span_end;
        }
    }

    /// Checked state of a FORMCHECKBOX field beginning at `begin`. Its instruction holds
    /// a 0x01 marker whose CPicLocation points at NilPICFAndBinData in the Data stream;
    /// the FFData follows the 0x44-byte header.
    fn checkbox(&self, begin: usize) -> Option<bool> {
        let text = &self.source.text;
        let end =
            (begin + 1..text.len().min(begin + 256)).find(|&i| matches!(text[i], 0x13..=0x15))?;
        let instr = String::from_utf16_lossy(&text[begin + 1..end]);
        if !instr.contains("FORMCHECKBOX") {
            return None;
        }
        let marker = (begin + 1..end).find(|&i| text[i] == 0x01);
        let ffdata = marker.and_then(|cp| {
            let mut flags = CharFlags::default();
            self.char_rpr(cp, &mut flags);
            let at = flags.pic_location? as usize + 0x44;
            self.source.data.get(at..)
        });
        let Some(ff) = ffdata else {
            return Some(false);
        };
        let bits = u16_le(ff, 4).unwrap_or(0);
        let res = (bits >> 2) & 0x1f;
        // wDef follows xstzName (u16 length, UTF-16 units, u16 terminator).
        let name_len = u16_le(ff, 10).unwrap_or(0) as usize;
        let default = u16_le(ff, 12 + name_len * 2 + 2).unwrap_or(0);
        Some(if res == 25 { default != 0 } else { res != 0 })
    }

    fn in_toc_result(&self, cp: usize) -> bool {
        self.toc_results
            .iter()
            .any(|&(sep, end)| sep < cp && cp < end)
    }

    /// Nesting depth of field instructions at `cp` (begin seen, separator not yet).
    fn field_instr_depth(&self, cp: usize) -> usize {
        // Walk back to the paragraph start of the enclosing story; fields that span
        // paragraphs are rare enough to be read from a bounded window.
        let window = cp.saturating_sub(4096);
        let mut stack: Vec<bool> = Vec::new();
        for &u in &self.source.text[window..cp] {
            match u {
                0x13 => stack.push(true),
                0x14 => {
                    if let Some(top) = stack.last_mut() {
                        *top = false;
                    }
                }
                0x15 => {
                    stack.pop();
                }
                _ => {}
            }
        }
        stack.iter().filter(|&&instr| instr).count()
    }

    /// The reference of a custom-marked note whose mark starts at `cp`.
    fn custom_note_mark(&self, cp: usize) -> Option<String> {
        if let Some(id) = self.footnotes.get(&cp) {
            return Some(format!(
                "<w:footnoteReference w:customMarkFollows=\"1\" w:id=\"{id}\"/>"
            ));
        }
        self.endnotes
            .get(&cp)
            .map(|id| format!("<w:endnoteReference w:customMarkFollows=\"1\" w:id=\"{id}\"/>"))
    }

    fn note_mark(&mut self, cp: usize) -> String {
        if let Some(id) = self.footnotes.get(&cp) {
            return format!("<w:footnoteReference w:id=\"{id}\"/>");
        }
        if let Some(id) = self.endnotes.get(&cp) {
            return format!("<w:endnoteReference w:id=\"{id}\"/>");
        }
        let s = self.source;
        if cp >= s.story_start(STORY_FOOTNOTE) && cp < s.story_start(STORY_HEADER) {
            return "<w:footnoteRef/>".into();
        }
        if cp >= s.story_start(STORY_ENDNOTE) && cp < s.story_start(STORY_TEXTBOX) {
            return "<w:endnoteRef/>".into();
        }
        String::new()
    }

    fn media(&mut self, asset: usize) -> Option<String> {
        let part = self.rels.len();
        let (format, bytes) = self.assets.get(asset)?.clone();
        let ext = match format.as_str() {
            "jpeg" | "jpg" => "jpeg",
            "png" => "png",
            "emf" => "emf",
            "wmf" => "wmf",
            "bmp" | "dib" => "bmp",
            "tiff" => "tiff",
            "gif" => "gif",
            "pict" => "pict",
            _ => return None,
        };
        let name = match self.media_names.get(&(asset, 0)) {
            Some(n) => n.clone(),
            None => {
                let n = format!("media/image{}.{ext}", self.media.len() + 1);
                let bytes = if ext == "bmp" && !bytes.starts_with(b"BM") {
                    super::blips::dib_to_bmp(&bytes)
                } else {
                    bytes
                };
                self.media.push(Part {
                    name: format!("word/{n}"),
                    bytes,
                });
                self.media_names.insert((asset, 0), n.clone());
                n
            }
        };
        if let Some((id, _, _)) = self.rels.iter().find(|r| r.2 == name) {
            return Some(id.clone());
        }
        let id = format!("rIdImg{}", part + 1);
        self.rels.push((id.clone(), "image", name));
        Some(id)
    }

    fn inline_picture(&mut self, cp: usize, location: Option<u32>) -> Option<String> {
        let lenient = || {
            let found =
                super::blips::inline(&self.source.data, location? as usize, &self.source.word);
            found.map(|(format, bytes)| (format.to_string(), bytes))
        };
        let asset = match self.picture_at_cp.get(&cp) {
            Some(&asset) => asset,
            None => match lenient() {
                Some(found) => {
                    self.count("inline_picture_recovered");
                    self.assets.push(found);
                    self.assets.len() - 1
                }
                None => {
                    self.count("inline_picture_unresolved");
                    return None;
                }
            },
        };
        let rid = self.media(asset)?;
        // PICF: picmid at 28 holds dxaGoal, dyaGoal, mx, my and four crop values.
        let (mut cx, mut cy) = (1_828_800i64, 1_371_600i64);
        let mut src = [0i64; 4];
        if let Some(picf) = location.and_then(|o| self.source.data.get(o as usize..)) {
            let goal_x = i16_le(picf, 28).unwrap_or(0) as i64;
            let goal_y = i16_le(picf, 30).unwrap_or(0) as i64;
            let mx = u16_le(picf, 32).unwrap_or(1000) as i64;
            let my = u16_le(picf, 34).unwrap_or(1000) as i64;
            let crop = |k: usize| i16_le(picf, 36 + k * 2).unwrap_or(0) as i64;
            let w = (goal_x - crop(0) - crop(2)) * mx / 1000;
            let h = (goal_y - crop(1) - crop(3)) * my / 1000;
            if w > 0 && h > 0 {
                cx = w * 635;
                cy = h * 635;
            }
            // Crops are twips of the picture's natural size; srcRect wants 1/1000 %.
            if goal_x > 0 && goal_y > 0 {
                let part = |c: i64, of: i64| (c * 100_000 / of).clamp(-100_000, 100_000);
                src = [
                    part(crop(0), goal_x),
                    part(crop(1), goal_y),
                    part(crop(2), goal_x),
                    part(crop(3), goal_y),
                ];
            }
            // Word 2000+ keeps the crop in the inline shape's properties instead.
            if src == [0; 4] {
                let [t, b, l, r] =
                    super::blips::inline_crop(&self.source.data, location.unwrap_or(0) as usize);
                let f = |v: i32| (v as i64 * 100_000 / 65_536).clamp(-100_000, 100_000);
                src = [f(l), f(t), f(r), f(b)];
            }
        }
        self.count("inline_picture");
        let id = self.next_id();
        Some(format!(
            "<w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{cx}\" cy=\"{cy}\"/>\
             <wp:docPr id=\"{id}\" name=\"Picture {id}\"/>{}</wp:inline></w:drawing>",
            pic_graphic(id, &rid, cx, cy, src)
        ))
    }

    fn next_id(&mut self) -> usize {
        self.next_id += 1;
        self.next_id
    }

    fn floating(&mut self, cp: usize) -> Option<String> {
        let Some(fspa) = self.shapes.get(&cp).copied() else {
            self.count("shape_anchor_without_fspa");
            return None;
        };
        let [mut l, mut t, mut r, mut b] = fspa.rect;
        let degrees = self
            .drawing
            .styles
            .get(&fspa.spid)
            .map_or(0.0, |s| s.degrees());
        if (45.0..135.0).contains(&degrees) || (225.0..315.0).contains(&degrees) {
            // Stored a quarter turn rotated: swap the box about its centre.
            let (mx, my, hw, hh) = ((l + r) / 2, (t + b) / 2, (r - l) / 2, (b - t) / 2);
            (l, t, r, b) = (mx - hh, my - hw, mx + hh, my + hw);
        }
        let (cx, cy) = (((r - l).max(1) as i64) * 635, ((b - t).max(1) as i64) * 635);
        let bx = (fspa.flags >> 1) & 3;
        let by = (fspa.flags >> 3) & 3;
        let wr = (fspa.flags >> 5) & 0xf;
        let below = fspa.flags & 0x4000 != 0;
        // The shape's own fBehindDocument overrides the FSPA fBelowText bit.
        let behind = self.drawing.styles.get(&fspa.spid).and_then(|s| s.behind);
        let place = self
            .drawing
            .styles
            .get(&fspa.spid)
            .map_or([None; 4], |s| s.placement);
        let (position_h, position_v) = anchor_position(place, bx as usize, by as usize, l, t);
        let wrap = wrap_element(wr);
        let Some((element, uri)) = self.shape_element(fspa.spid, (0, 0), (cx, cy), 0) else {
            self.count("shape_without_picture_or_text");
            return None;
        };
        let graphic = format!(
            "<a:graphic><a:graphicData uri=\"{uri}\">{element}</a:graphicData></a:graphic>"
        );
        let id = self.next_id();
        // Word 97-2003 keeps an inline shape as a SHAPE field around its anchor.
        if self
            .shape_results
            .iter()
            .any(|&(sep, end)| sep < cp && cp < end)
        {
            return Some(format!(
                "<w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{cx}\" cy=\"{cy}\"/>\
                 <wp:docPr id=\"{id}\" name=\"Shape {id}\"/><wp:cNvGraphicFramePr/>{graphic}</wp:inline></w:drawing>"
            ));
        }
        // Drawing order from the OfficeArt records; shapes without it stack by emission.
        let z = self
            .drawing
            .styles
            .get(&fspa.spid)
            .map_or(id, |s| s.z as usize + 1);
        Some(format!(
            "<w:drawing><wp:anchor distT=\"0\" distB=\"0\" distL=\"114300\" distR=\"114300\" simplePos=\"0\" relativeHeight=\"{z}\" \
             behindDoc=\"{}\" locked=\"0\" layoutInCell=\"1\" allowOverlap=\"1\"><wp:simplePos x=\"0\" y=\"0\"/>\
             {position_h}{position_v}\
             <wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/>{wrap}\
             <wp:docPr id=\"{id}\" name=\"Shape {id}\"/><wp:cNvGraphicFramePr/>{graphic}</wp:anchor></w:drawing>",
            u8::from(wr == 3 && behind.unwrap_or(below)),
        ))
    }

    /// DrawingML element for one shape placed at `off` with size `ext` (EMU at the top
    /// level, the parent group's coordinates inside a group), with its graphicData URI.
    /// `depth` is the group nesting level: 0 is the anchored shape itself.
    fn shape_element(
        &mut self,
        spid: u32,
        off: (i64, i64),
        ext: (i64, i64),
        depth: usize,
    ) -> Option<(String, &'static str)> {
        const WPS: &str = "http://schemas.microsoft.com/office/word/2010/wordprocessingShape";
        let style = self.drawing.styles.get(&spid).copied();
        let flips = style.map_or(String::new(), |s| {
            format!(
                "{}{}",
                if s.flip_h { " flipH=\"1\"" } else { "" },
                if s.flip_v { " flipV=\"1\"" } else { "" }
            )
        });
        // OfficeArt keeps shapes turned 45-135 or 225-315 degrees with their anchor box
        // already rotated a quarter turn; DrawingML wants the unrotated box.
        let degrees = style.map_or(0.0, |s| s.degrees());
        let quarter = (45.0..135.0).contains(&degrees) || (225.0..315.0).contains(&degrees);
        // The anchored shape itself was unrotated by `floating`.
        let (off, ext) = if quarter && depth > 0 {
            let (cx, cy) = (off.0 + ext.0 / 2, off.1 + ext.1 / 2);
            ((cx - ext.1 / 2, cy - ext.0 / 2), (ext.1, ext.0))
        } else {
            (off, ext)
        };
        let rot = if degrees != 0.0 {
            format!(" rot=\"{}\"", (degrees * 60000.0).round() as i64)
        } else {
            String::new()
        };
        let xfrm = |extra: &str| {
            format!(
                "<a:xfrm{rot}{flips}><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/>{extra}</a:xfrm>",
                off.0,
                off.1,
                ext.0.max(1),
                ext.1.max(1)
            )
        };
        // lTxid's high half picks the textbox story entry (main or header drawing);
        // the PLC's lid is the fallback when the shape has none.
        let by_txid = style.and_then(|st| {
            let entry = (st.txid? >> 16).checked_sub(1)? as usize;
            self.textbox_entries[usize::from(st.header)]
                .get(entry)
                .copied()
        });
        if let Some((a, z)) = by_txid
            .filter(|(a, z)| z > a)
            .or_else(|| self.textboxes.get(&spid).copied())
        {
            if self.in_textbox > 0 {
                return None;
            }
            self.in_textbox += 1;
            let content = self.blocks(a, z);
            self.in_textbox -= 1;
            self.count("textbox");
            let paint = style.map_or("<a:noFill/><a:ln><a:noFill/></a:ln>".into(), |s| {
                s.paint_xml()
            });
            let geom = style
                .and_then(|s| s.preset())
                .filter(|&g| g != "line")
                .unwrap_or("rect");
            return Some((
                format!(
                    "<wps:wsp><wps:cNvSpPr txBox=\"1\"/><wps:spPr>{}\
                     <a:prstGeom prst=\"{geom}\"><a:avLst/></a:prstGeom>{paint}</wps:spPr>\
                     <wps:txbx><w:txbxContent>{content}</w:txbxContent></wps:txbx>\
                     <wps:bodyPr rot=\"0\" vert=\"horz\" wrap=\"square\" lIns=\"91440\" tIns=\"45720\" rIns=\"91440\" bIns=\"45720\" anchor=\"t\"><a:noAutofit/></wps:bodyPr>\
                     </wps:wsp>",
                    xfrm("")
                ),
                WPS,
            ));
        }
        let stored = style.and_then(|s| s.pib).and_then(|pib| {
            self.blip_store
                .get(pib.checked_sub(1)? as usize)
                .copied()
                .flatten()
        });
        let by_shape = self
            .picture_by_shape
            .get(&spid)
            .copied()
            .filter(|_| takes_mapped_picture(style.as_ref()));
        if let Some(asset) = by_shape.or(stored) {
            let rid = self.media(asset)?;
            self.count("floating_picture");
            let id = self.next_id + 1;
            return Some((
                format!(
                    "<pic:pic><pic:nvPicPr><pic:cNvPr id=\"{id}\" name=\"Picture {id}\"/><pic:cNvPicPr/></pic:nvPicPr>\
                     <pic:blipFill><a:blip r:embed=\"{rid}\"/>{}<a:stretch><a:fillRect/></a:stretch></pic:blipFill>\
                     <pic:spPr>{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic>",
                    // OPT crops are 16.16 fractions: top, bottom, left, right.
                    src_rect(style.map_or([0; 4], |s| {
                        let f = |v: i32| (v as i64 * 100_000 / 65_536).clamp(-100_000, 100_000);
                        [f(s.crop[2]), f(s.crop[0]), f(s.crop[3]), f(s.crop[1])]
                    })),
                    xfrm("")
                ),
                "http://schemas.openxmlformats.org/drawingml/2006/picture",
            ));
        }
        if let Some((text, font, emphasis)) = self.drawing.wordart.get(&spid).cloned() {
            self.count("wordart");
            let xml = wordart_xml(&text, &font, emphasis, style, ext, depth == 0, &xfrm(""));
            return Some((xml, WPS));
        }
        if let Some(members) = self
            .drawing
            .groups
            .get(&spid)
            .cloned()
            .filter(|_| depth < 8)
        {
            let [gl, gt, gr, gb] = style.and_then(|s| s.group_space).unwrap_or([0, 0, 1, 1]);
            let mut inner = String::new();
            for m in members {
                let Some([l, t, r, b]) = self.drawing.styles.get(&m).and_then(|s| s.child_anchor)
                else {
                    continue;
                };
                let child = (i64::from(l), i64::from(t));
                let size = (i64::from(r) - i64::from(l), i64::from(b) - i64::from(t));
                if let Some((element, _)) = self.shape_element(m, child, size, depth + 1) {
                    inner.push_str(&element);
                }
            }
            if inner.is_empty() {
                return None;
            }
            self.count("shape_group");
            let space = format!(
                "<a:chOff x=\"{gl}\" y=\"{gt}\"/><a:chExt cx=\"{}\" cy=\"{}\"/>",
                (gr - gl).max(1),
                (gb - gt).max(1)
            );
            let (tag, nv) = if depth == 0 {
                ("wpg:wgp", String::new())
            } else {
                let id = self.next_id();
                (
                    "wpg:grpSp",
                    format!("<wpg:cNvPr id=\"{id}\" name=\"Group {id}\"/>"),
                )
            };
            return Some((
                format!(
                    "<{tag}>{nv}<wpg:cNvGrpSpPr/><wpg:grpSpPr>{}</wpg:grpSpPr>{inner}</{tag}>",
                    xfrm(&space)
                ),
                "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup",
            ));
        }
        let style = style?;
        let geometry = match (style.preset(), self.drawing.paths.get(&spid).cloned()) {
            (Some(geom), _) => format!("<a:prstGeom prst=\"{geom}\"><a:avLst/></a:prstGeom>"),
            (None, Some(path)) => {
                self.count("freeform_shape");
                format!(
                    "<a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/><a:rect l=\"0\" t=\"0\" r=\"r\" b=\"b\"/><a:pathLst>{path}</a:pathLst></a:custGeom>"
                )
            }
            (None, None) => return None,
        };
        self.count("plain_shape");
        Some((
            format!(
                "<wps:wsp><wps:cNvSpPr/><wps:spPr>{}{geometry}{}</wps:spPr><wps:bodyPr/></wps:wsp>",
                xfrm(""),
                style.paint_xml()
            ),
            WPS,
        ))
    }

    /// Section list from PlcfSed: (end CP, properties).
    fn sections(&self) -> Vec<(usize, Section)> {
        let s = self.source;
        let Some(plc) = s.pair(pair::PLCFSED) else {
            return vec![(s.stories[0], Section::default())];
        };
        let n = plc.len().saturating_sub(4) / 16;
        let mut out = Vec::new();
        for i in 0..n {
            let end = u32_le(plc, i * 4 + 4).unwrap_or(0) as usize;
            let fc = u32_le(plc, 4 * (n + 1) + i * 12 + 2).unwrap_or(u32::MAX) as usize;
            let props = if fc == u32::MAX as usize {
                Section::default()
            } else {
                let cb = u16_le(&s.word, fc).unwrap_or(0) as usize;
                props::section(s.word.get(fc + 2..fc + 2 + cb).unwrap_or(&[]))
            };
            out.push((end, props));
        }
        if out.is_empty() {
            out.push((s.stories[0], Section::default()));
        }
        out
    }

    /// Header/footer stories from PlcfHdd: per section [even hdr, odd hdr, even ftr,
    /// odd ftr, first hdr, first ftr] after six note-separator stories.
    fn header_ranges(&self) -> Vec<[Option<(usize, usize)>; 6]> {
        let s = self.source;
        let Some(plc) = s.pair(pair::PLCFHDD) else {
            return Vec::new();
        };
        let base = s.story_start(STORY_HEADER);
        let cps: Vec<usize> = (0..plc.len() / 4)
            .filter_map(|i| u32_le(plc, i * 4))
            .map(|c| c as usize)
            .collect();
        let stories = cps.len().saturating_sub(1);
        let mut out = Vec::new();
        let mut k = 6;
        while k + 6 <= stories {
            let mut slots = [None; 6];
            for (j, slot) in slots.iter_mut().enumerate() {
                let (a, b) = (cps[k + j], cps[k + j + 1]);
                if b > a {
                    *slot = Some((base + a, base + b));
                }
            }
            out.push(slots);
            k += 6;
        }
        out
    }

    fn part_xml(
        &mut self,
        root: &str,
        start: usize,
        end: usize,
    ) -> (String, Vec<(String, &'static str, String)>) {
        let saved = std::mem::take(&mut self.rels);
        // A story ends with a guard paragraph mark that Word does not display.
        let body_end = if !(self.keeps_story_tail && root == "hdr")
            && end > start + 1
            && self.source.text.get(end - 1) == Some(&0x0d)
            && self.source.text.get(end - 2) == Some(&0x0d)
        {
            end - 1
        } else {
            end
        };
        let body = self.blocks(start, body_end);
        let rels = std::mem::replace(&mut self.rels, saved);
        (
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:{root} {NS}>{body}</w:{root}>"
            ),
            rels,
        )
    }

    pub fn document(&mut self) -> Vec<Part> {
        let mut parts = Vec::new();
        let sections = self.sections();
        let headers = self.header_ranges();
        let mut doc_rels: Vec<(String, &'static str, String)> = vec![
            ("rIdStyles".into(), "styles", "styles.xml".into()),
            ("rIdSettings".into(), "settings", "settings.xml".into()),
        ];
        let mut extra_parts = Vec::new();
        let mut sect_xml = Vec::new();
        let mut written: HashMap<(usize, usize), String> = HashMap::new();
        for (i, (_, section)) in sections.iter().enumerate() {
            let mut refs = String::new();
            if let Some(slots) = headers.get(i) {
                let roles = [
                    ("header", "even"),
                    ("header", "default"),
                    ("footer", "even"),
                    ("footer", "default"),
                    ("header", "first"),
                    ("footer", "first"),
                ];
                for (slot, (kind, role)) in slots.iter().zip(roles) {
                    let Some(range) = *slot else { continue };
                    let rid = if let Some(rid) = written.get(&range) {
                        rid.clone()
                    } else {
                        let n = extra_parts.len() + 1;
                        let name = format!("{kind}{n}.xml");
                        let root = if kind == "header" { "hdr" } else { "ftr" };
                        let (xml, rels) = self.part_xml(root, range.0, range.1);
                        if !rels.is_empty() {
                            extra_parts.push(Part {
                                name: format!("word/_rels/{name}.rels"),
                                bytes: package::rels(&rels).into_bytes(),
                            });
                        }
                        extra_parts.push(Part {
                            name: format!("word/{name}"),
                            bytes: xml.into_bytes(),
                        });
                        let rid = format!("rIdHf{n}");
                        doc_rels.push((rid.clone(), kind, name));
                        written.insert(range, rid.clone());
                        rid
                    };
                    write!(
                        refs,
                        "<w:{kind}Reference w:type=\"{role}\" r:id=\"{rid}\"/>"
                    )
                    .unwrap();
                }
            }
            sect_xml.push(format!("<w:sectPr>{refs}{}</w:sectPr>", section.xml()));
        }
        // Section properties sit on the paragraph whose mark ends the section.
        for (i, (end, _)) in sections.iter().enumerate() {
            if i + 1 < sections.len() {
                let at = self.paras.partition_point(|p| p.end < *end);
                let end_cp = self.paras.get(at).map_or(*end, |p| p.end);
                self.section_marks.insert(end_cp, sect_xml[i].clone());
            }
        }
        let main_end = self.source.stories[0];
        let mut body = self.blocks(0, main_end);
        body.push_str(sect_xml.last().map_or("<w:sectPr/>", String::as_str));
        for (k, (root, item, file)) in [
            ("footnotes", "footnote", "footnotes.xml"),
            ("endnotes", "endnote", "endnotes.xml"),
        ]
        .into_iter()
        .enumerate()
        {
            if self.note_ranges[k].is_empty() {
                continue;
            }
            let saved = std::mem::take(&mut self.rels);
            let mut xml = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:{root} {NS}>\
                 <w:{item} w:type=\"separator\" w:id=\"0\"><w:p><w:r><w:separator/></w:r></w:p></w:{item}>\
                 <w:{item} w:type=\"continuationSeparator\" w:id=\"1\"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:{item}>"
            );
            for (i, (a, b)) in self.note_ranges[k].clone().into_iter().enumerate() {
                let content = self.blocks(a, b);
                write!(xml, "<w:{item} w:id=\"{}\">{content}</w:{item}>", i + 2).unwrap();
            }
            write!(xml, "</w:{root}>").unwrap();
            let rels = std::mem::replace(&mut self.rels, saved);
            if !rels.is_empty() {
                extra_parts.push(Part {
                    name: format!("word/_rels/{file}.rels"),
                    bytes: package::rels(&rels).into_bytes(),
                });
            }
            extra_parts.push(Part {
                name: format!("word/{file}"),
                bytes: xml.into_bytes(),
            });
            doc_rels.push((format!("rId{root}"), root, file.into()));
        }
        if !self.comment_items.is_empty() {
            let saved = std::mem::take(&mut self.rels);
            let mut xml = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:comments {NS}>"
            );
            for (id, (a, b, author, initials)) in self.comment_items.clone().into_iter().enumerate()
            {
                let content = self.blocks(a, b);
                write!(
                    xml,
                    "<w:comment w:id=\"{id}\" w:author=\"{}\" w:initials=\"{}\">{content}</w:comment>",
                    esc(&author),
                    esc(&initials)
                )
                .unwrap();
            }
            xml.push_str("</w:comments>");
            let rels = std::mem::replace(&mut self.rels, saved);
            if !rels.is_empty() {
                extra_parts.push(Part {
                    name: "word/_rels/comments.xml.rels".into(),
                    bytes: package::rels(&rels).into_bytes(),
                });
            }
            extra_parts.push(Part {
                name: "word/comments.xml".into(),
                bytes: xml.into_bytes(),
            });
            doc_rels.push(("rIdComments".into(), "comments", "comments.xml".into()));
        }
        if !self.font_entries.is_empty() {
            extra_parts.push(Part {
                name: "word/fontTable.xml".into(),
                bytes: self.font_table_xml().into_bytes(),
            });
            doc_rels.push(("rIdFontTable".into(), "fontTable", "fontTable.xml".into()));
        }
        if let Some(numbering) = self.numbering_xml() {
            extra_parts.push(Part {
                name: "word/numbering.xml".into(),
                bytes: numbering.into_bytes(),
            });
            doc_rels.push(("rIdNumbering".into(), "numbering", "numbering.xml".into()));
        }
        let document = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:document {NS}><w:body>{body}</w:body></w:document>"
        );
        doc_rels.append(&mut self.rels);
        parts.push(Part {
            name: "word/document.xml".into(),
            bytes: document.into_bytes(),
        });
        parts.push(Part {
            name: "word/styles.xml".into(),
            bytes: self
                .styles
                .xml(
                    &self.fonts,
                    self.lists.as_ref().map_or(0, |l| l.overrides.len()),
                )
                .into_bytes(),
        });
        parts.push(Part {
            name: "word/settings.xml".into(),
            bytes: self.settings_xml().into_bytes(),
        });
        parts.push(Part {
            name: "word/_rels/document.xml.rels".into(),
            bytes: package::rels(&doc_rels).into_bytes(),
        });
        parts.append(&mut extra_parts);
        parts.append(&mut self.media);
        parts.push(Part {
            name: "_rels/.rels".into(),
            bytes: package::ROOT_RELS.as_bytes().to_vec(),
        });
        let types = package::content_types(&parts);
        parts.insert(
            0,
            Part {
                name: "[Content_Types].xml".into(),
                bytes: types.into_bytes(),
            },
        );
        for (what, n) in &self.counts {
            self.diagnostics.push(format!("{what}: {n}"));
        }
        parts
    }

    fn settings_xml(&self) -> String {
        let dop = self.source.pair(pair::DOP).unwrap_or(&[]);
        let facing = u16_le(dop, 0).unwrap_or(0) & 1 != 0;
        let tab = u16_le(dop, 10).filter(|&t| t > 0).unwrap_or(720);
        // Copts sits at DOP offset 512 (Word 2000+) after a copy of Copts80 at 508; the
        // copy must match the one at 84 (bit 9 aside) before the field is trusted.
        let copts = u32_le(dop, 508)
            .zip(u32_le(dop, 84))
            .filter(|(copy, first)| (copy ^ first) & 0xffff_fdff == 0)
            .and_then(|_| u32_le(dop, 512));
        let mut compat = String::new();
        // Copts60 fNoTabForInd (bit 0): a hanging indent adds no automatic tab stop.
        if u16_le(dop, 8).is_some_and(|c| c & 0x1 != 0) {
            compat.push_str("<w:noTabHangInd/>");
        }
        // Copts60 (DOP offset 8) fDntBlnSbDbWid clear: spaces beside East Asian text
        // take half an em.
        if u16_le(dop, 8).is_some_and(|c| c & 0x8000 == 0) {
            compat.push_str("<w:balanceSingleByteDoubleByteWidth/>");
        }
        // Copts80 (DOP offset 84) fNoLeading: line heights leave out external leading.
        if u32_le(dop, 84).is_some_and(|c| c & 0x8_0000 != 0) {
            compat.push_str("<w:noLeading/>");
        }
        if copts.is_some_and(|c| c & 0x4 != 0) {
            compat.push_str("<w:doNotUseHTMLParagraphAutoSpacing/>");
        }
        // Cell lines snap to the grid unless fDontAdjustLineHeightInTable is set.
        if copts.is_some_and(|c| c & 0x8 == 0) {
            compat.push_str("<w:adjustLineHeightInTable/>");
        }
        // DopTypography (DOP offset 90) iJustification, bits 1-2: East Asian punctuation
        // compression (1 punctuation, 2 punctuation and Japanese kana).
        let spacing = match u16_le(dop, 90).map(|t| (t >> 1) & 3) {
            Some(1) => "<w:characterSpacingControl w:val=\"compressPunctuation\"/>",
            Some(2) => "<w:characterSpacingControl w:val=\"compressPunctuationAndJapaneseKana\"/>",
            _ => "",
        };
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <w:settings xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
             {}<w:defaultTabStop w:val=\"{tab}\"/>{spacing}\
             <w:compat>{compat}<w:compatSetting w:name=\"compatibilityMode\" w:uri=\"http://schemas.microsoft.com/office/word\" w:val=\"11\"/></w:compat>\
             </w:settings>",
            if facing { "<w:evenAndOddHeaders/>" } else { "" }
        )
    }

    /// Font table with each font's family class and the alternate name Word falls back to
    /// when the font is missing: the recorded alternate, the base family of a style-
    /// qualified name, else a stand-in for the family class (roman, swiss, modern).
    fn font_table_xml(&self) -> String {
        let mut out = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <w:fonts xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">",
        );
        for (name, alt, family, charset) in &self.font_entries {
            // The class stand-ins are Western faces: skip symbol, CJK, Thai and Mac
            // charsets, and fonts with non-ASCII names.
            let western = !matches!(charset, 2 | 77 | 128..=136 | 222) && name.is_ascii();
            let class = ["auto", "roman", "swiss", "modern", "script", "decorative"]
                .get(*family as usize)
                .copied()
                .unwrap_or("auto");
            let stand_in = match class {
                "roman" if western => Some("Times New Roman"),
                "swiss" if western => Some("Arial"),
                "modern" if western => Some("Courier New"),
                _ => None,
            };
            let alt = alt
                .clone()
                .or_else(|| base_family(name))
                .or_else(|| stand_in.map(str::to_string));
            write!(out, "<w:font w:name=\"{}\">", esc(name)).unwrap();
            if let Some(alt) = alt.filter(|a| !a.trim().is_empty()) {
                write!(out, "<w:altName w:val=\"{}\"/>", esc(&alt)).unwrap();
            }
            write!(out, "<w:family w:val=\"{class}\"/></w:font>").unwrap();
        }
        out.push_str("</w:fonts>");
        out
    }

    fn numbering_xml(&self) -> Option<String> {
        let lists = self.lists.as_ref()?;
        if lists.overrides.is_empty() {
            return None;
        }
        let mut out = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:numbering {NS}>"
        );
        for (a, list) in lists.lists.iter().enumerate() {
            write!(out, "<w:abstractNum w:abstractNumId=\"{a}\">").unwrap();
            for (l, level) in list.levels.iter().enumerate().take(9) {
                let fmt = match level.number_format {
                    0 => "decimal",
                    1 => "upperRoman",
                    2 => "lowerRoman",
                    3 => "upperLetter",
                    4 => "lowerLetter",
                    5 => "ordinal",
                    6 => "cardinalText",
                    7 => "ordinalText",
                    22 => "decimalZero",
                    23 => "bullet",
                    255 => "none",
                    _ => "decimal",
                };
                let mut text = String::new();
                for &u in &level.number_text_utf16 {
                    if u < 9 {
                        write!(text, "%{}", u + 1).unwrap();
                    } else {
                        text.push_str(&esc(&String::from_utf16_lossy(&[u])));
                    }
                }
                let jc = match level.flags & 3 {
                    1 => "center",
                    2 => "right",
                    _ => "left",
                };
                let suff = match level.following_character {
                    1 => "space",
                    2 => "nothing",
                    _ => "tab",
                };
                write!(
                    out,
                    "<w:lvl w:ilvl=\"{l}\"><w:start w:val=\"{}\"/><w:numFmt w:val=\"{fmt}\"/>\
                     <w:suff w:val=\"{suff}\"/><w:lvlText w:val=\"{text}\"/><w:lvlJc w:val=\"{jc}\"/>",
                    level.start_at
                )
                .unwrap();
                let p = props::ppr(&level.raw_papx, &mut ParaFlags::default());
                if !p.is_empty() {
                    write!(out, "<w:pPr>{}</w:pPr>", strip_frames(&p)).unwrap();
                }
                let r = rpr(&level.raw_chpx, &self.fonts, &mut CharFlags::default());
                if !r.is_empty() {
                    write!(out, "<w:rPr>{r}</w:rPr>").unwrap();
                }
                out.push_str("</w:lvl>");
            }
            out.push_str("</w:abstractNum>");
        }
        for (i, o) in lists.overrides.iter().enumerate() {
            write!(
                out,
                "<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"{}\"/>",
                i + 1,
                o.list_index
            )
            .unwrap();
            for lvl in &o.levels {
                if let Some(start) = exported_start(lvl) {
                    write!(
                        out,
                        "<w:lvlOverride w:ilvl=\"{}\"><w:startOverride w:val=\"{}\"/></w:lvlOverride>",
                        lvl.level, start
                    )
                    .unwrap();
                }
            }
            out.push_str("</w:num>");
        }
        out.push_str("</w:numbering>");
        Some(out)
    }
}

/// Paragraph XML is emitted in schema order by `props::ppr`; nothing to strip yet.
fn strip_frames(p: &str) -> String {
    p.to_string()
}

/// a:srcRect for a picture crop given as (left, top, right, bottom) in 1/1000 percent.
/// Whether a shape draws the picture its blip references map to it. A picture frame
/// always does; any other shape only while filled, since the picture is its fill
/// (a cover's frame rectangles with fill off carry photo fills Word does not show).
/// The percentage width (fiftieths of a percent) a converted table keeps. A
/// single-column AutoFit table narrower than the text width keeps its width and
/// autofits, as in Word's own conversion; other tables keep the .doc's fixed cell
/// widths, which already match Word.
fn autofit_percent_width(row: &Row, columns: usize) -> Option<i16> {
    row.pct_width
        .filter(|&w| row.autofit && columns <= 1 && w < 5000)
}

/// [`autofit_percent_width`] for a row's raw table modifiers (tests).
#[cfg(test)]
pub(super) fn autofit_percent_width_of(grpprl: &[u8], columns: usize) -> Option<i16> {
    autofit_percent_width(&Row::read(grpprl), columns)
}

pub(super) fn takes_mapped_picture(style: Option<&super::drawing::ShapeStyle>) -> bool {
    style.is_none_or(|s| s.kind == 75 || s.fill.is_some())
}

fn src_rect([l, t, r, b]: [i64; 4]) -> String {
    if [l, t, r, b] == [0; 4] {
        return String::new();
    }
    format!("<a:srcRect l=\"{l}\" t=\"{t}\" r=\"{r}\" b=\"{b}\"/>")
}

fn pic_graphic(id: usize, rid: &str, cx: i64, cy: i64, crop: [i64; 4]) -> String {
    let src = src_rect(crop);
    format!(
        "<a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:pic>\
         <pic:nvPicPr><pic:cNvPr id=\"{id}\" name=\"Picture {id}\"/><pic:cNvPicPr/></pic:nvPicPr>\
         <pic:blipFill><a:blip r:embed=\"{rid}\"/>{src}<a:stretch><a:fillRect/></a:stretch></pic:blipFill>\
         <pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr>\
         </pic:pic></a:graphicData></a:graphic>"
    )
}

#[derive(Default, Clone)]
struct Tc {
    grf: u16,
    borders: [Option<String>; 4],
}

/// Table row properties from the row-end paragraph's modifiers (TAPX).
#[derive(Default)]
struct Row {
    centers: Vec<i16>,
    cells: Vec<Tc>,
    gap_half: i16,
    height: Option<i16>,
    header: bool,
    cant_split: bool,
    jc: Option<u8>,
    borders: String,
    shading: Vec<String>,
    cell_borders: HashMap<(usize, usize), String>,
    /// Default cell padding (top, left, bottom, right) in twips from
    /// sprmTCellPaddingDefault; Word 2000+ uses it in place of dxaGapHalf.
    padding: [Option<i16>; 4],
    /// sprmTCellPadding: per-cell top/bottom padding, (cell, side) -> twips, sides
    /// 0 top and 2 bottom.
    cell_padding: HashMap<(usize, usize), i16>,
    /// sprmTCellSpacingDefault: spacing between cells in twips (DOCX
    /// `w:tblCellSpacing`, half the gap between neighbouring cells).
    spacing: Option<i16>,
    /// sprmTTableWidth given as a percentage (fiftieths of a percent of the text
    /// width), kept only for an AutoFit table (sprmTFAutofit set).
    pct_width: Option<i16>,
    autofit: bool,
}

impl Row {
    fn read(grpprl: &[u8]) -> Row {
        let mut row = Row::default();
        let mut shading: Vec<Option<String>> = Vec::new();
        for (sprm, op) in super::source::sprms(grpprl) {
            match sprm {
                0xd608 => {
                    let n = op.first().copied().unwrap_or(0) as usize;
                    row.centers = (0..=n).filter_map(|k| i16_le(op, 1 + k * 2)).collect();
                    let tc_at = 1 + (n + 1) * 2;
                    row.cells = (0..n)
                        .map(|k| {
                            let at = tc_at + k * 20;
                            let Some(grf) = u16_le(op, at) else {
                                return Tc::default();
                            };
                            let mut tc = Tc {
                                grf,
                                ..Default::default()
                            };
                            for (j, slot) in tc.borders.iter_mut().enumerate() {
                                let brc = op.get(at + 4 + j * 4..at + 8 + j * 4);
                                // brcNil (all bits set) removes the table's border on this side.
                                *slot = if brc == Some(&[0xff; 4][..]) {
                                    Some("<w:top w:val=\"nil\"/>".to_string())
                                } else {
                                    brc.and_then(|b| border("top", b))
                                        .filter(|b| !b.contains("nil"))
                                };
                            }
                            tc
                        })
                        .collect();
                }
                0x9602 => row.gap_half = i16_le(op, 0).unwrap_or(0),
                0x3615 => row.autofit = op.first().is_some_and(|&v| v != 0),
                // sprmTTableWidth: ftsWidth (2 = fiftieths of a percent), wWidth.
                0xf614 => {
                    row.pct_width = match (op.first(), i16_le(op, 1)) {
                        (Some(&2), Some(w)) if w > 0 => Some(w),
                        _ => None,
                    };
                }
                // CSSA: itcFirst, itcLim, sides (1 top, 2 left, 4 bottom, 8 right),
                // ftsWidth (3 = twips), wWidth.
                0xd634 => {
                    if let (Some(&sides), Some(&3), Some(w)) = (op.get(2), op.get(3), i16_le(op, 4))
                    {
                        for (k, slot) in row.padding.iter_mut().enumerate() {
                            if sides & (1 << k) != 0 {
                                *slot = Some(w);
                            }
                        }
                    }
                }
                // CSSA as for padding; ftsWidth 0 (nil) clears the spacing.
                0xd633 => {
                    row.spacing = match (op.get(3), i16_le(op, 4)) {
                        (Some(&3) | Some(&0x13), Some(w)) if w > 0 => Some(w),
                        _ => None,
                    };
                }
                0xd632 => {
                    if let (Some(&first), Some(&lim), Some(&sides), Some(&3), Some(w)) =
                        (op.first(), op.get(1), op.get(2), op.get(3), i16_le(op, 4))
                    {
                        for c in first as usize..lim as usize {
                            // Only non-zero top and bottom: zero and left/right overrides
                            // here do not move cell text in Word (sample: 3 documents
                            // worse with them).
                            for k in [0, 2] {
                                if sides & (1 << k) != 0 && w > 0 {
                                    row.cell_padding.insert((c, k), w);
                                }
                            }
                        }
                    }
                }
                0x9407 => row.height = i16_le(op, 0),
                0x3404 => row.header = op.first() == Some(&1),
                0x3403 | 0x3644 => row.cant_split = op.first() == Some(&1),
                0x5400 | 0x548a => row.jc = op.first().copied(),
                0xd605 | 0xd613 => {
                    let size = if sprm == 0xd605 { 4 } else { 8 };
                    let names = ["top", "left", "bottom", "right", "insideH", "insideV"];
                    row.borders = names
                        .iter()
                        .enumerate()
                        .filter_map(|(k, name)| border(name, op.get(k * size..(k + 1) * size)?))
                        .collect();
                }
                0xd620 | 0xd62f => {
                    // TableBordersOperand: itcFirst, itcLim, grfbrc (top, left, bottom, right), brc.
                    let (Some(&first), Some(&lim), Some(&grf)) = (op.first(), op.get(1), op.get(2))
                    else {
                        continue;
                    };
                    let Some(b) = border("top", &op[3..]) else {
                        continue;
                    };
                    for c in first as usize..lim as usize {
                        for k in 0..4 {
                            if grf & (1 << k) != 0 {
                                row.cell_borders.insert((c, k), b.clone());
                            }
                        }
                    }
                }
                0xd612 | 0xd616 | 0xd60c => {
                    let offset = match sprm {
                        0xd612 => 0,
                        0xd616 => 22,
                        _ => 44,
                    };
                    for k in 0..op.len() / 10 {
                        let back = u32_le(op, k * 10 + 4).unwrap_or(0xff00_0000);
                        let idx = offset + k;
                        if shading.len() <= idx {
                            shading.resize(idx + 1, None);
                        }
                        shading[idx] = (back >> 24 != 0xff).then(|| colorref(back));
                    }
                }
                0xd609 => {
                    for k in 0..op.len() / 2 {
                        let v = u16_le(op, k * 2).unwrap_or(0);
                        let back = ((v >> 5) & 0x1f) as usize;
                        if shading.len() <= k {
                            shading.resize(k + 1, None);
                        }
                        if back != 0 && shading[k].is_none() {
                            shading[k] = ICO.get(back).map(|s| s.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
        // Cells without a shading entry keep no fill; keep indexes aligned.
        row.shading = shading.into_iter().map(|s| s.unwrap_or_default()).collect();
        row
    }
}

/// WordArt as a borderless textbox: centered text in the shape's font and fill colour.
/// WordArt stretches its text over the shape, so at the top level (`ext` in EMU) the size
/// is fitted to the box, counting ideographs as 1 em and other characters as 0.55 em.
fn wordart_xml(
    text: &str,
    font: &str,
    (bold, italic): (bool, bool),
    style: Option<super::drawing::ShapeStyle>,
    ext: (i64, i64),
    emu: bool,
    xfrm: &str,
) -> String {
    let lines: Vec<&str> = text
        .split(['\n', '\r', '\u{b}'])
        .filter(|l| !l.is_empty())
        .collect();
    let widest = lines
        .iter()
        .map(|l| {
            l.chars()
                .map(|c| if (c as u32) >= 0x2e80 { 1.0 } else { 0.55 })
                .sum::<f64>()
        })
        .fold(1.0, f64::max);
    let size = if emu {
        let (w, h) = (ext.0 as f64 / 12700.0, ext.1 as f64 / 12700.0);
        (h / lines.len().max(1) as f64 / 1.2)
            .min(w / widest)
            .clamp(6.0, 200.0)
    } else {
        24.0
    };
    // Run colour has no alpha: blend the fill towards white by its opacity.
    let opacity = style.map_or(1.0, |s| s.fill_opacity as f64 / 65536.0);
    let color = style.and_then(|s| s.fill).unwrap_or(0);
    let blend = |shift: u32| {
        let c = ((color >> shift) & 0xff) as f64;
        ((c * opacity + 255.0 * (1.0 - opacity)).round() as u32) << shift
    };
    let color = blend(16) | blend(8) | blend(0);
    let font = esc(font);
    let emphasis = format!(
        "{}{}",
        if bold { "<w:b/>" } else { "" },
        if italic { "<w:i/>" } else { "" }
    );
    let paragraphs: String = lines
        .iter()
        .map(|l| {
            format!(
                "<w:p><w:pPr><w:jc w:val=\"center\"/><w:spacing w:before=\"0\" w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr>\
                 <w:r><w:rPr><w:rFonts w:ascii=\"{font}\" w:hAnsi=\"{font}\" w:eastAsia=\"{font}\"/>{emphasis}<w:color w:val=\"{color:06X}\"/>\
                 <w:sz w:val=\"{0}\"/><w:szCs w:val=\"{0}\"/></w:rPr><w:t xml:space=\"preserve\">{1}</w:t></w:r></w:p>",
                (size * 2.0).round() as i64,
                esc(l)
            )
        })
        .collect();
    format!(
        "<wps:wsp><wps:cNvSpPr txBox=\"1\"/><wps:spPr>{xfrm}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/><a:ln><a:noFill/></a:ln></wps:spPr>\
         <wps:txbx><w:txbxContent>{paragraphs}</w:txbxContent></wps:txbx>\
         <wps:bodyPr rot=\"0\" vert=\"horz\" wrap=\"none\" lIns=\"0\" tIns=\"0\" rIns=\"0\" bIns=\"0\" anchor=\"ctr\"><a:noAutofit/></wps:bodyPr></wps:wsp>"
    )
}

/// The family behind a style-qualified font name ("Calibri-Bold", "Arial Bold Italic"),
/// which Word resolves to the base family's styled face.
/// The face Word for Mac draws for a `.doc` font it does not have. A `_GB2312`
/// family whose stored alternate is 宋体 draws with SimSun; 楷体_GB2312 otherwise
/// draws with Microsoft YaHei, though the same name in a .docx draws with KaiTi
/// (Word renders of the source files: YaHei or SimSun, KaiTi never).
fn word_doc_face(name: &str, alternate: Option<&str>, signature: &[u8]) -> String {
    // Word for Mac has no TH SarabunIT๙ and draws a .doc's with Browallia New when
    // the font table signs it, with Cordia New when the signature is empty.
    if name == "TH SarabunIT๙" {
        let signed = signature.iter().any(|&b| b != 0);
        return if signed {
            "Browallia New"
        } else {
            "Cordia New"
        }
        .to_string();
    }
    if name.ends_with("_GB2312") && alternate.map(str::trim) == Some("宋体") {
        return "宋体".to_string();
    }
    if name == "楷体_GB2312" {
        return "Microsoft YaHei".to_string();
    }
    name.to_string()
}

fn base_family(name: &str) -> Option<String> {
    let mut base = name.trim();
    loop {
        let before = base;
        for suffix in [
            "Bold",
            "Italic",
            "Oblique",
            "Regular",
            "BoldItalic",
            "BoldMT",
            "MT",
            "PSMT",
        ] {
            for sep in ['-', ' ', ','] {
                if let Some(rest) = base.strip_suffix(suffix).and_then(|r| r.strip_suffix(sep)) {
                    base = rest.trim_end();
                }
            }
        }
        if base == before {
            break;
        }
    }
    (base != name.trim() && !base.is_empty()).then(|| base.to_string())
}

/// wp:positionH and wp:positionV for a floating shape. The FSPA gives the frames (`bx`,
/// `by`: margin, page, column/paragraph) and the offset `l`, `t` in twips; a shape's
/// posh/posv alignment (1 left/top, 2 centre, 3 right/bottom, 4 inside, 5 outside)
/// replaces the offset, measured against posrelh/posrelv's frame. posrelv defaults to
/// the paragraph (msoprvText), where Word offers no vertical alignment.
pub(super) fn anchor_position(
    place: [Option<u32>; 4],
    bx: usize,
    by: usize,
    l: i32,
    t: i32,
) -> (String, String) {
    const ALIGN_H: [&str; 5] = ["left", "center", "right", "inside", "outside"];
    const ALIGN_V: [&str; 5] = ["top", "center", "bottom", "inside", "outside"];
    let mut h = ["margin", "page", "column", "column"][bx.min(3)];
    let mut v = ["margin", "page", "paragraph", "paragraph"][by.min(3)];
    let pick = |code: Option<u32>, names: &[&'static str; 5]| {
        code.and_then(|c| names.get((c as usize).checked_sub(1)?))
            .copied()
    };
    let align_h = pick(place[0], &ALIGN_H);
    let align_v = pick(place[2].filter(|_| place[3].unwrap_or(2) != 2), &ALIGN_V);
    if align_h.is_some() {
        h = match place[1] {
            Some(0) => "margin",
            Some(1) => "page",
            Some(2) => "column",
            Some(3) => "character",
            _ => h,
        };
    }
    if align_v.is_some() {
        v = match place[3] {
            Some(0) => "margin",
            Some(1) => "page",
            Some(3) => "line",
            _ => v,
        };
    }
    let pos = |align: Option<&str>, offset: i32| {
        align.map_or(
            format!("<wp:posOffset>{}</wp:posOffset>", offset as i64 * 635),
            |a| format!("<wp:align>{a}</wp:align>"),
        )
    };
    (
        format!(
            "<wp:positionH relativeFrom=\"{h}\">{}</wp:positionH>",
            pos(align_h, l)
        ),
        format!(
            "<wp:positionV relativeFrom=\"{v}\">{}</wp:positionV>",
            pos(align_v, t)
        ),
    )
}

/// Word 97-2003 floats a table whose paragraphs all carry the same frame. For a frame
/// anchored to the page or to the text the frame becomes the table's `tblpPr`, with
/// the same vertical anchor, and leaves the paragraphs (Word renders: page anchors 9
/// documents better and 2 slightly worse; text anchors 13 better and 5 worse).
/// Margin-anchored frames, and other tables, are left as they are.
pub(super) fn float_framed_table(table: String) -> String {
    let frames: Vec<&str> = table
        .match_indices("<w:framePr ")
        .map(|(i, _)| &table[i..i + table[i..].find("/>").map_or(0, |e| e + 2)])
        .collect();
    let paragraphs = table.matches("<w:p>").count() + table.matches("<w:p ").count();
    let Some(&frame) = frames.first() else {
        return table;
    };
    if frame.contains("w:vAnchor=\"margin\"")
        || frames.len() != paragraphs
        || frames.iter().any(|f| *f != frame)
        || table[5..].contains("<w:tbl>")
    {
        return table;
    }
    let attr = |name: &str| {
        let key = format!(" w:{name}=\"");
        frame
            .find(&key)
            .map(|i| &frame[i + key.len()..])
            .and_then(|rest| rest.split('"').next())
    };
    let mut tblp = String::from("<w:tblpPr");
    if let Some(s) = attr("hSpace") {
        write!(tblp, " w:leftFromText=\"{s}\" w:rightFromText=\"{s}\"").unwrap();
    }
    if let Some(s) = attr("vSpace") {
        write!(tblp, " w:topFromText=\"{s}\" w:bottomFromText=\"{s}\"").unwrap();
    }
    write!(
        tblp,
        " w:vertAnchor=\"{}\" w:horzAnchor=\"{}\"",
        attr("vAnchor").unwrap_or("text"),
        attr("hAnchor").unwrap_or("text")
    )
    .unwrap();
    match (attr("x"), attr("xAlign")) {
        (Some(x), _) => write!(tblp, " w:tblpX=\"{x}\"").unwrap(),
        (None, Some(a)) => write!(tblp, " w:tblpXSpec=\"{a}\"").unwrap(),
        _ => {}
    }
    match (attr("y"), attr("yAlign")) {
        (Some(y), _) => write!(tblp, " w:tblpY=\"{y}\"").unwrap(),
        (None, Some(a)) => write!(tblp, " w:tblpYSpec=\"{a}\"").unwrap(),
        _ => {}
    }
    tblp.push_str("/>");
    table
        .replace(frame, "")
        .replacen("<w:tblPr>", &format!("<w:tblPr>{tblp}"), 1)
}

/// Result ranges (separator, end) of fields whose instruction starts with `name`. TOC:
/// Word copies pictures and shape anchors from a heading into its TOC entry; they already
/// render at the heading, so the copies are dropped. SHAPE: the shape is inline.
pub(super) fn field_results(text: &[u16], name: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    // Per open field: (begin CP, separator CP once seen).
    let mut stack: Vec<(usize, Option<usize>)> = Vec::new();
    for (cp, &u) in text.iter().enumerate() {
        match u {
            0x13 => stack.push((cp, None)),
            0x14 => {
                if let Some(top) = stack.last_mut() {
                    top.1 = Some(cp);
                }
            }
            0x15 => {
                if let Some((begin, Some(sep))) = stack.pop() {
                    let instr = String::from_utf16_lossy(&text[begin + 1..sep]);
                    let word = instr.split_whitespace().next().unwrap_or("");
                    if word.eq_ignore_ascii_case(name) {
                        out.push((sep, cp));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The `w:startOverride` of a list level override: its start when `fStartAt` (0x10)
/// is set. Word ignores a start outside [MS-DOC]'s 0..=0x7FFF (a list whose stored
/// start is 134954608 counts from its level's start).
pub(super) fn exported_start(lvl: &crate::office::doc::lists::LevelOverride) -> Option<i32> {
    (lvl.raw_flags & 0x10 != 0 && (0..=0x7fff).contains(&lvl.raw_start_at))
        .then_some(lvl.raw_start_at)
}

/// The DrawingML wrap element of an [MS-DOC] FSPA.wr value: 1 top and bottom, 3 none,
/// 4 tight, 5 through; 0 and 2 are square. Tight and through wrap around the shape's box.
pub(super) fn wrap_element(wr: u16) -> &'static str {
    match wr {
        1 => "<wp:wrapTopAndBottom/>",
        3 => "<wp:wrapNone/>",
        4 => concat!(
            "<wp:wrapTight wrapText=\"bothSides\">",
            box_polygon!(),
            "</wp:wrapTight>"
        ),
        5 => concat!(
            "<wp:wrapThrough wrapText=\"bothSides\">",
            box_polygon!(),
            "</wp:wrapThrough>"
        ),
        _ => "<wp:wrapSquare wrapText=\"bothSides\"/>",
    }
}

/// The document's main-story bookmarks by first CP; empty when the tables are absent
/// or unreadable.
fn body_bookmarks(data: &[u8]) -> BTreeMap<usize, Vec<String>> {
    let mut out: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    if let Ok(b) = crate::office::doc::bookmarks::inspect_bookmarks(data) {
        for e in b.entries.into_iter().filter(|e| e.story == Some("body")) {
            out.entry(e.source_cp[0]).or_default().push(e.name);
        }
    }
    out
}
