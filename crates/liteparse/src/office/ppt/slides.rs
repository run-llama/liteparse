//! Reads the deck: slide size, fonts, masters and slides, each sheet's shapes written
//! as DrawingML (see `native`) or taken from their metroBlob copies (see `metro`).
//!
//! Slide order and placeholder text come from the SlideListWithText containers in the
//! DocumentContainer: instance 0 lists slides, instance 1 masters. Each sheet's shapes
//! come from its PPDrawing, an OfficeArt drawing whose ClientTextbox records either
//! hold their own text or point into the sheet's SlideListWithText texts by index.

use crate::office::ppt::Error;
use crate::office::ppt::metro::{self, MetroShape};
use crate::office::ppt::native::{self, Native};
use crate::office::ppt::persist::Persist;
use crate::office::ppt::props::{Props, Scheme};
use crate::office::ppt::records::{self as r, Rec};
use crate::office::ppt::text::{self, Body, Cf, Env, Pf, Styles};
use std::collections::BTreeMap;

/// EMU per PowerPoint master unit (576 per inch; 914400 EMU per inch).
pub const EMU_PER_MASTER: f64 = 1587.5;
/// Nesting depth of shape groups followed before the rest is skipped.
const MAX_GROUP_DEPTH: usize = 32;

/// A decoded picture-store entry: format name and bytes.
pub type Blip = Option<(&'static str, Vec<u8>)>;

pub struct Deck {
    /// Slide size in EMU.
    pub width: i64,
    pub height: i64,
    /// Face name of font 0 in the FontCollection, the deck's default font.
    pub font: Option<String>,
    pub masters: Vec<Sheet>,
    pub slides: Vec<Slide>,
    /// The picture store, by 1-based `pib` order.
    pub blips: Vec<Blip>,
}

/// Shapes and background of a master or slide.
#[derive(Default)]
pub struct Sheet {
    pub items: Vec<Item>,
    /// `p:bg` content (`p:bgPr`), when the sheet sets its own background.
    pub background: Option<String>,
    /// A master's DrawingML theme part and `a:clrMap` attributes, kept by PowerPoint
    /// 2007+ in round-trip atoms. Shapes from metroBlobs refer to them.
    pub theme: Option<Vec<u8>>,
    pub clr_map: Option<String>,
    /// A master's DrawingML `p:txStyles` (RoundTripOArtTextStyles12Atom).
    pub tx_styles: Option<String>,
    /// A master's slide layouts by instance (RoundTripContentMasterInfo12Atom).
    pub layouts: Vec<(u16, MetroShape)>,
}

#[derive(Default)]
pub struct Slide {
    pub hidden: bool,
    /// Index into [`Deck::masters`].
    pub master: usize,
    /// Whether the master's shapes show on this slide.
    pub master_objects: bool,
    /// Instance of the master's round-trip layout the slide uses, when it has one.
    pub layout: Option<u16>,
    pub sheet: Sheet,
}

/// One shape on a sheet, in drawing order.
pub enum Item {
    /// DrawingML written from the binary shape, pictures as `r:embed="PIB<n>"`.
    Native(String),
    /// A shape's original DrawingML from its metroBlob; `rect` replaces its frame
    /// when the shape sat in a group (the XML's frame is then in group coordinates).
    Xml(MetroShape, Option<[i64; 4]>),
}

/// What [`read`] takes from the file.
#[derive(Clone, Copy)]
pub struct Options {
    /// Read metroBlobs, as PowerPoint does: a table, chart or SmartArt replaces its
    /// binary stand-in group, and a text shape (other than a placeholder) is drawn
    /// from its DrawingML copy, which keeps formatting the binary text cannot hold.
    pub metro: bool,
    /// Also emit a text-less shape's metroBlob when it carries text. PowerPoint saves
    /// such shapes (text with 3-D or other 2007 effects) as a picture of the shape:
    /// exact on the page, but its text is lost to extraction. The DrawingML copy keeps
    /// the text and is drawn with substitute fonts.
    pub metro_shapes: bool,
    /// The conversion time for "today" date fields, in seconds since the Unix epoch
    /// (UTC); `None` reads the system clock.
    pub now: Option<i64>,
}

/// Counts of skipped or unexpected structures, reported once per kind.
#[derive(Default)]
pub struct Skipped(BTreeMap<String, usize>);

impl Skipped {
    pub fn add(&mut self, what: impl Into<String>) {
        *self.0.entry(what.into()).or_default() += 1;
    }
    pub fn report(self, diags: &mut Vec<String>) {
        for (what, n) in self.0 {
            diags.push(format!("{what} (x{n})"));
        }
    }
}

/// Document-wide inputs every sheet reads.
struct Shared<'a> {
    doc: &'a [u8],
    persist: &'a Persist,
    fonts: Vec<String>,
    /// Text defaults from the Environment (TextPFExceptionAtom, TextCFExceptionAtom).
    base: (Pf, Cf),
    /// The Environment's master text styles, under every master's own.
    env_styles: Styles,
    first_number: u32,
    footer: Option<String>,
    /// User-entered date text, shown by date fields when flag 0x04 is set.
    user_date: Option<String>,
    /// HeadersFootersAtom flags for slides: 0x01 date, 0x08 slide number, 0x20 footer.
    hf_flags: u16,
    /// HeadersFootersAtom formatId: the date format of date placeholders.
    date_format: u8,
    /// The conversion time, as text for each date format (see [`crate::office::ppt::dates`]).
    now: i64,
    opts: Options,
    size: (i64, i64),
    /// Embedded OLE objects (exObjId) whose progId is an equation.
    equations: std::collections::HashSet<u32>,
}

/// A master's date, slide-number and footer placeholders: (type, shape, transform).
type Footers<'a> = Vec<(u8, Shape<'a>, Xf)>;

/// A master as slides use it.
struct MasterInfo<'a> {
    slide_id: u32,
    styles: Styles,
    scheme: Scheme,
    /// Properties of the master's placeholders by placeholder type, which a slide's
    /// placeholder of the matching type builds on.
    placeholders: Vec<(u8, Props<'a>)>,
    /// Round-trip layout instances, when the master also has its text styles.
    layouts: Vec<u16>,
    /// The master's date, slide-number and footer placeholders with their transform.
    footers: Footers<'a>,
    /// The master's background shape properties, resolved per slide scheme.
    background: Option<Props<'a>>,
}

pub fn read(
    doc: &[u8],
    pictures: &[u8],
    persist: &Persist,
    opts: Options,
    skipped: &mut Skipped,
) -> Result<Deck, Error> {
    let container = persist
        .record(doc, persist.document)
        .filter(|c| c.kind == r::DOCUMENT)
        .ok_or(Error::Invalid(
            "no DocumentContainer at the document persist offset",
        ))?;
    let kids = r::children(doc, container);
    let (mut width, mut height) = (9_144_000, 6_858_000);
    let mut first_number = 1;
    match kids.iter().find(|k| k.kind == r::DOCUMENT_ATOM) {
        Some(atom) => {
            let w = r::i32_at(doc, atom.start).unwrap_or(0);
            let h = r::i32_at(doc, atom.start + 4).unwrap_or(0);
            if w > 0 && h > 0 {
                width = (w as f64 * EMU_PER_MASTER).round() as i64;
                height = (h as f64 * EMU_PER_MASTER).round() as i64;
            } else {
                skipped.add("DocumentAtom slide size is not positive; using 10 x 7.5 in");
            }
            first_number = r::u16_at(doc, atom.start + 32).map_or(1, u32::from);
        }
        None => skipped.add("no DocumentAtom; using a 10 x 7.5 in slide"),
    }
    let mut shared = Shared {
        doc,
        persist,
        fonts: Vec::new(),
        base: (Pf::default(), Cf::default()),
        env_styles: Styles::empty(),
        first_number,
        footer: None,
        user_date: None,
        hf_flags: 0,
        date_format: 0,
        now: opts
            .now
            .unwrap_or_else(crate::office::ppt::dates::system_now),
        opts,
        size: (width, height),
        equations: Default::default(),
    };
    let mut blips = Vec::new();
    for k in &kids {
        match k.kind {
            r::ENVIRONMENT => read_environment(&mut shared, *k),
            r::PP_DRAWING_GROUP => {
                if let Some(dgg) = r::child(doc, *k, r::DGG_CONTAINER) {
                    blips = crate::office::doc::docx_export::blips::store(
                        &doc[dgg.start - 8..dgg.end],
                        pictures,
                    );
                }
            }
            r::EX_OBJ_LIST => {
                for embed in r::children(doc, *k)
                    .into_iter()
                    .filter(|c| c.kind == r::EX_EMBED)
                {
                    let parts = r::children(doc, embed);
                    let id = parts
                        .iter()
                        .find(|c| c.kind == r::EX_OLE_OBJ_ATOM)
                        .and_then(|c| r::u32_at(doc, c.start + 8));
                    let prog_id = parts
                        .iter()
                        .find(|c| c.kind == r::CSTRING && c.inst == 2)
                        .map(|c| utf16(&doc[c.start..c.end]));
                    if let (Some(id), Some(prog_id)) = (id, prog_id)
                        && crate::office::ppt::native::is_equation_prog_id(&prog_id)
                    {
                        shared.equations.insert(id);
                    }
                }
            }
            r::HEADERS_FOOTERS if k.inst == 3 => {
                let hf = r::children(doc, *k);
                let string = |inst: u16| {
                    hf.iter()
                        .find(|c| c.kind == r::CSTRING && c.inst == inst)
                        .map(|c| utf16(&doc[c.start..c.end]))
                };
                shared.footer = string(2);
                shared.user_date = string(0);
                let atom = hf.iter().find(|c| c.kind == r::HEADERS_FOOTERS_ATOM);
                shared.hf_flags = atom.and_then(|c| r::u16_at(doc, c.start + 2)).unwrap_or(0);
                shared.date_format = atom
                    .and_then(|c| r::u16_at(doc, c.start))
                    .map_or(0, |f| f as u8);
            }
            _ => {}
        }
    }
    let font = shared.fonts.first().cloned().filter(|f| !f.is_empty());
    let list = |inst: u16| {
        kids.iter()
            .find(|k| k.kind == r::SLIDE_LIST_WITH_TEXT && k.inst == inst)
            .map(|l| slide_list(doc, *l))
            .unwrap_or_default()
    };
    let (masters, infos) = read_masters(&shared, list(1), skipped);
    let mut slides = Vec::new();
    let mut entries = list(0);
    apply_outline_numbering(doc, &kids, &mut entries);
    if entries.is_empty() {
        skipped.add("no slide list; the deck has no slides");
    }
    for (i, entry) in entries.into_iter().enumerate() {
        let number = shared.first_number + i as u32;
        slides.push(read_slide(&shared, &infos, entry, number, skipped));
    }
    Ok(Deck {
        width,
        height,
        font,
        masters,
        slides,
        blips,
    })
}

fn read_environment(shared: &mut Shared, env: Rec) {
    let doc = shared.doc;
    for k in r::children(doc, env) {
        match k.kind {
            r::FONT_COLLECTION => {
                shared.fonts = r::children(doc, k)
                    .into_iter()
                    .filter(|f| f.kind == r::FONT_ENTITY_ATOM)
                    .map(|f| utf16(&doc[f.start..f.end.min(f.start + 64)]))
                    .collect();
            }
            r::TEXT_CF_EXCEPTION_ATOM => {
                if let Some(cf) = text::read_default_cf(doc, k) {
                    shared.base.1 = cf;
                }
            }
            r::TEXT_PF_EXCEPTION_ATOM => {
                if let Some(pf) = text::read_default_pf(doc, k) {
                    shared.base.0 = pf;
                }
            }
            r::TEXT_MASTER_STYLE_ATOM => shared.env_styles.add(doc, k),
            _ => {}
        }
    }
}

/// One SlidePersistAtom and the texts that follow it in SlideListWithText.
#[derive(Default)]
struct ListEntry {
    persist: u32,
    slide_id: u32,
    texts: Vec<Body>,
}

fn slide_list(doc: &[u8], list: Rec) -> Vec<ListEntry> {
    let mut out: Vec<ListEntry> = Vec::new();
    let mut style: Option<Rec> = None;
    let finish = |e: Option<&mut ListEntry>, style: &mut Option<Rec>| {
        if let (Some(e), Some(s)) = (e, style.take())
            && let Some(b) = e.texts.last_mut()
        {
            b.runs = Some(text::read_runs(doc, s, b.units.len()));
        }
    };
    for k in r::children(doc, list) {
        match k.kind {
            r::SLIDE_PERSIST_ATOM => {
                finish(out.last_mut(), &mut style);
                out.push(ListEntry {
                    persist: r::u32_at(doc, k.start).unwrap_or(0),
                    slide_id: r::u32_at(doc, k.start + 12).unwrap_or(0),
                    texts: vec![],
                });
            }
            r::TEXT_HEADER_ATOM => {
                finish(out.last_mut(), &mut style);
                if let Some(e) = out.last_mut() {
                    e.texts.push(Body {
                        text_type: r::u32_at(doc, k.start).unwrap_or(4),
                        ..Body::default()
                    });
                }
            }
            r::TEXT_CHARS_ATOM | r::TEXT_BYTES_ATOM => {
                if let Some(b) = out.last_mut().and_then(|e| e.texts.last_mut()) {
                    b.units = decode_text(doc, k);
                }
            }
            r::STYLE_TEXT_PROP_ATOM => style = Some(k),
            r::TEXT_RULER_ATOM => {
                if let Some(b) = out.last_mut().and_then(|e| e.texts.last_mut()) {
                    b.ruler = Some(text::read_ruler(doc, k));
                }
            }
            r::TEXT_INTERACTIVE_INFO_ATOM => {
                if let (Some(b), Some(range)) = (
                    out.last_mut().and_then(|e| e.texts.last_mut()),
                    link_range(doc, k),
                ) {
                    b.links.push(range);
                }
            }
            _ => {}
        }
    }
    finish(out.last_mut(), &mut style);
    out
}

/// TextCharsAtom holds UTF-16LE; TextBytesAtom holds the low bytes of UTF-16 code
/// units, which is ISO 8859-1.
fn decode_text(doc: &[u8], rec: Rec) -> Vec<u16> {
    let body = &doc[rec.start..rec.end];
    if rec.kind == r::TEXT_BYTES_ATOM {
        return body.iter().map(|&b| b as u16).collect();
    }
    body.as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

fn utf16(b: &[u8]) -> String {
    let units: Vec<u16> = b
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// SlideAtom fields: masterIdRef and the follow-master flags (objects, scheme,
/// background).
fn slide_atom(doc: &[u8], sheet: Rec) -> Option<(u32, u16)> {
    let atom = r::child(doc, sheet, r::SLIDE_ATOM)?;
    Some((
        r::u32_at(doc, atom.start + 12)?,
        r::u16_at(doc, atom.start + 20)?,
    ))
}

fn own_scheme(doc: &[u8], sheet: Rec) -> Option<Scheme> {
    r::children(doc, sheet)
        .into_iter()
        .find(|k| k.kind == r::COLOR_SCHEME_ATOM && k.inst == 1)
        .and_then(|k| Scheme::read(doc, k))
}

/// Masters in list order. A title master (a SlideContainer in the master list) takes
/// its text styles from the main master it names.
fn read_masters<'a>(
    shared: &Shared<'a>,
    entries: Vec<ListEntry>,
    skipped: &mut Skipped,
) -> (Vec<Sheet>, Vec<MasterInfo<'a>>) {
    let doc = shared.doc;
    let mut infos: Vec<MasterInfo> = Vec::new();
    let mut recs = Vec::new();
    for e in &entries {
        let rec = shared
            .persist
            .record(doc, e.persist)
            .filter(|c| matches!(c.kind, r::MAIN_MASTER | r::SLIDE));
        let mut styles = shared.env_styles.clone();
        let mut scheme = Scheme::DEFAULT;
        if let Some(rec) = rec {
            for k in r::children(doc, rec) {
                match k.kind {
                    r::TEXT_MASTER_STYLE_ATOM => styles.add(doc, k),
                    r::ROUND_TRIP_TEXT_STYLES => {
                        if let Some(xml) = round_trip_part(
                            &doc[k.start..k.end],
                            "drs/slideMasters/slideMaster1.xml",
                        ) {
                            styles.add_kerning(&String::from_utf8_lossy(&xml));
                        }
                    }
                    _ => {}
                }
            }
            scheme = own_scheme(doc, rec).unwrap_or(scheme);
        } else {
            skipped.add("master persist id without a master container");
        }
        infos.push(MasterInfo {
            slide_id: e.slide_id,
            styles,
            scheme,
            placeholders: Vec::new(),
            layouts: Vec::new(),
            footers: Vec::new(),
            background: None,
        });
        recs.push(rec);
    }
    // Title masters: styles (and, when they follow it, the scheme) of their main master.
    for i in 0..infos.len() {
        let Some(rec) = recs[i].filter(|r| r.kind == r::SLIDE) else {
            continue;
        };
        let Some((main, flags)) = slide_atom(doc, rec) else {
            continue;
        };
        if let Some(m) = infos.iter().position(|m| m.slide_id == main) {
            infos[i].styles = infos[m].styles.clone();
            if flags & 2 != 0 {
                infos[i].scheme = infos[m].scheme;
            }
        }
    }
    let mut sheets = Vec::new();
    for (i, e) in entries.into_iter().enumerate() {
        let mut sheet = Sheet::default();
        if let Some(rec) = recs[i] {
            let (s, placeholders, footers, background) =
                read_sheet(shared, rec, &e.texts, &infos[i], None, true, skipped);
            sheet = s;
            infos[i].footers = footers;
            infos[i].background = background;
            for k in r::children(doc, rec) {
                match k.kind {
                    r::ROUND_TRIP_THEME => sheet.theme = round_trip_theme(&doc[k.start..k.end]),
                    r::ROUND_TRIP_TEXT_STYLES if shared.opts.metro => {
                        sheet.tx_styles = round_trip_part(
                            &doc[k.start..k.end],
                            "drs/slideMasters/slideMaster1.xml",
                        )
                        .map(|x| strip_decl(&String::from_utf8_lossy(&x)));
                    }
                    r::ROUND_TRIP_LAYOUT if shared.opts.metro => {
                        match metro::read_named(
                            &doc[k.start..k.end],
                            "drs/slideLayouts/slideLayout1.xml",
                        ) {
                            Ok(Some(l)) => sheet.layouts.push((k.inst, l)),
                            Ok(None) => {}
                            Err(e) => skipped.add(format!("round-trip layout unusable ({e})")),
                        }
                    }
                    r::ROUND_TRIP_COLOR_MAPPING => {
                        sheet.clr_map =
                            clr_map_attrs(&String::from_utf8_lossy(&doc[k.start..k.end]))
                    }
                    _ => {}
                }
            }
            infos[i].placeholders = placeholders;
            if sheet.tx_styles.is_some() {
                infos[i].layouts = sheet.layouts.iter().map(|l| l.0).collect();
            }
        }
        sheets.push(sheet);
    }
    if sheets.is_empty() {
        sheets.push(Sheet::default());
        infos.push(MasterInfo {
            slide_id: 0,
            styles: shared.env_styles.clone(),
            scheme: Scheme::DEFAULT,
            placeholders: Vec::new(),
            layouts: Vec::new(),
            footers: Vec::new(),
            background: None,
        });
    }
    (sheets, infos)
}

fn read_slide<'a>(
    shared: &Shared<'a>,
    masters: &[MasterInfo<'a>],
    entry: ListEntry,
    number: u32,
    skipped: &mut Skipped,
) -> Slide {
    let doc = shared.doc;
    let mut slide = Slide {
        master_objects: true,
        ..Slide::default()
    };
    let Some(container) = shared
        .persist
        .record(doc, entry.persist)
        .filter(|c| c.kind == r::SLIDE)
    else {
        skipped.add("slide persist id without a SlideContainer; slide left empty");
        return slide;
    };
    let (master_id, flags) = slide_atom(doc, container).unwrap_or((0, 7));
    slide.master = masters
        .iter()
        .position(|m| m.slide_id == master_id)
        .unwrap_or(0);
    slide.master_objects = flags & 1 != 0;
    let master = &masters[slide.master];
    let scheme = if flags & 2 != 0 {
        master.scheme
    } else {
        own_scheme(doc, container).unwrap_or(master.scheme)
    };
    let mut info = MasterInfo {
        slide_id: 0,
        styles: master.styles.clone(),
        scheme,
        placeholders: master.placeholders.clone(),
        layouts: Vec::new(),
        footers: master.footers.clone(),
        background: None,
    };
    for k in r::children(doc, container) {
        if k.kind == r::SLIDE_SHOW_SLIDE_INFO {
            // flags at offset 10: fManualAdvance, reserved, fHidden, ...
            slide.hidden = r::u16_at(doc, k.start + 10).is_some_and(|f| f & 4 != 0);
        }
        // RoundTripContentMasterId12Atom: mainMasterId, then the layout instance.
        if k.kind == r::ROUND_TRIP_LAYOUT_ID {
            slide.layout = r::u16_at(doc, k.start + 4).filter(|i| master.layouts.contains(i));
        }
    }
    // A slide on a round-trip layout draws its placeholders from their metroBlobs.
    info.layouts.extend(slide.layout);
    slide.sheet = read_sheet(
        shared,
        container,
        &entry.texts,
        &info,
        Some(number),
        false,
        skipped,
    )
    .0;
    if flags & 4 != 0 {
        // The master's background, with scheme colours taken from this slide's own
        // scheme when it has one (PowerPoint renders of 1997-2003 decks).
        slide.sheet.background = master
            .background
            .as_ref()
            .filter(|_| flags & 2 == 0)
            .map(|props| background_xml(props, &scheme));
    }
    slide
}

/// Shapes and background of one master or slide container.
/// On a master, also returns its placeholders' properties.
fn read_sheet<'a>(
    shared: &Shared<'a>,
    container: Rec,
    texts: &[Body],
    info: &MasterInfo<'a>,
    number: Option<u32>,
    is_master: bool,
    skipped: &mut Skipped,
) -> (Sheet, Vec<(u8, Props<'a>)>, Footers<'a>, Option<Props<'a>>) {
    let doc = shared.doc;
    let mut sheet = Sheet::default();
    let mut background_props = None;
    let mut used = vec![false; texts.len()];
    let env = Env {
        fonts: &shared.fonts,
        scheme: &info.scheme,
        styles: &info.styles,
        base: &shared.base,
    };
    let mut ctx = Shapes {
        shared,
        env: &env,
        texts,
        used: &mut used,
        items: &mut sheet.items,
        number,
        is_master,
        master_placeholders: &info.placeholders,
        placeholder_metro: !is_master && !info.layouts.is_empty(),
        placeholders: Vec::new(),
        footers: Vec::new(),
        slide_placeholders: Vec::new(),
        next_id: 2,
        seen: Default::default(),
        skipped,
    };
    if let Some(dg) =
        r::child(doc, container, r::PP_DRAWING).and_then(|d| r::child(doc, d, r::DG_CONTAINER))
    {
        for k in r::children(doc, dg) {
            match k.kind {
                r::SPGR_CONTAINER => ctx.group(k, Xf::IDENTITY, 0),
                // The background shape follows the shape tree.
                r::SP_CONTAINER => {
                    // An unset background fill colour is white, the OfficeArt default
                    // (PowerPoint renders of 1997-2003 decks); scheme colours are explicit.
                    let mut props = Props::default();
                    props.set(crate::office::ppt::props::id::FILL_COLOR, 0x00FF_FFFF);
                    for c in r::children(doc, k) {
                        if matches!(c.kind, r::OPT | r::SECONDARY_OPT | r::TERTIARY_OPT) {
                            props.add(doc, c);
                        }
                    }
                    sheet.background = Some(background_xml(&props, &info.scheme));
                    background_props = Some(props);
                }
                _ => {}
            }
        }
    }
    // Old files keep date, slide-number and footer placeholders on the master only:
    // a slide without its own shows the master's when its flags turn it on: the
    // slide's own HeadersFootersAtom, else the document's.
    if !is_master {
        let flags = r::child(doc, container, r::HEADERS_FOOTERS)
            .and_then(|hf| r::child(doc, hf, r::HEADERS_FOOTERS_ATOM))
            .and_then(|atom| r::u16_at(doc, atom.start + 2))
            .unwrap_or(shared.hf_flags);
        for (t, shape, xf) in &info.footers {
            let on = match t {
                7 => flags & 0x01 != 0,
                8 => flags & 0x08 != 0,
                _ => flags & 0x20 != 0,
            };
            if on && !ctx.slide_placeholders.contains(t) {
                let mut shape = shape.clone();
                shape.metro = None;
                ctx.emit(shape, *xf, false);
            }
        }
    }
    // Placeholder text no shape references still belongs to the slide.
    for (i, body) in texts.iter().enumerate() {
        if ctx.used[i] || body.text_type == 2 || body.text().trim().is_empty() || is_master {
            continue;
        }
        ctx.skipped
            .add("slide-list text without a shape; placed in a default box");
        let (w, h) = (shared.size.0 as f64, shared.size.1 as f64);
        let f = if matches!(body.text_type, 0 | 6) {
            [0.05, 0.04, 0.9, 0.17]
        } else {
            [0.05, 0.23, 0.9, 0.7]
        };
        let rect = [f[0] * w, f[1] * h, f[2] * w, f[3] * h].map(|v| v.round() as i64);
        let mut props = Props::default();
        props.add_flag(crate::office::ppt::props::id::FILL_BOOLS, 4, false);
        props.add_flag(crate::office::ppt::props::id::LINE_BOOLS, 3, false);
        let n = Native {
            kind: 202,
            flip_h: false,
            flip_v: false,
            rotation: 0.0,
            rect,
            props: &props,
            text: Some(body),
            fields: &[],
            equation: false,
        };
        let id = ctx.next_id;
        ctx.next_id += 1;
        if let Some(xml) = native::shape(&n, &env, id) {
            ctx.items.push(Item::Native(xml));
        }
    }
    let placeholders = ctx.placeholders;
    let footers = ctx.footers;
    (sheet, placeholders, footers, background_props)
}

/// `p:bgPr` content of a background shape's fill under `scheme`.
fn background_xml(props: &Props, scheme: &Scheme) -> String {
    format!(
        "<p:bgPr>{}<a:effectLst/></p:bgPr>",
        native::fill_xml(props, scheme)
    )
}

/// Affine map from a group's child coordinates to master units:
/// (x, y) -> (a x + b y + c, d x + e y + f), y pointing down.
#[derive(Clone, Copy, Debug)]
struct Xf {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

/// A shape's frame on the slide: (x, y, width, height) in EMU, clockwise rotation in
/// degrees and flips, as DrawingML states them (flip first, then rotate).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Placed {
    rect: [i64; 4],
    rotation: f64,
    flip_h: bool,
    flip_v: bool,
}

impl Xf {
    const IDENTITY: Xf = Xf {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 0.0,
        e: 1.0,
        f: 0.0,
    };

    fn point(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.b * y + self.c,
            self.d * x + self.e * y + self.f,
        )
    }

    /// `outer` applied after `self`.
    fn then(self, o: Xf) -> Xf {
        Xf {
            a: o.a * self.a + o.b * self.d,
            b: o.a * self.b + o.b * self.e,
            c: o.a * self.c + o.b * self.f + o.c,
            d: o.d * self.a + o.e * self.d,
            e: o.d * self.b + o.e * self.e,
            f: o.d * self.c + o.e * self.f + o.f,
        }
    }

    /// The members' transform of a group whose shape has `anchor` (in this transform's
    /// input space), child space `space`, rotation and flips: the space is fitted to
    /// the frame, then flipped and turned about the frame's centre.
    fn group(
        self,
        anchor: [f64; 4],
        space: [f64; 4],
        rotation: f64,
        flip_h: bool,
        flip_v: bool,
    ) -> Xf {
        let [l, t, r, b] = unswap(anchor, rotation);
        let ratio = |a: f64, b: f64| if b.abs() < 1e-9 { 1.0 } else { a / b };
        let sx = ratio(r - l, space[2] - space[0]);
        let sy = ratio(b - t, space[3] - space[1]);
        let fit = Xf {
            a: sx,
            b: 0.0,
            c: l - space[0] * sx,
            d: 0.0,
            e: sy,
            f: t - space[1] * sy,
        };
        let (cx, cy) = ((l + r) / 2.0, (t + b) / 2.0);
        let flip = Xf {
            a: if flip_h { -1.0 } else { 1.0 },
            b: 0.0,
            c: if flip_h { 2.0 * cx } else { 0.0 },
            d: 0.0,
            e: if flip_v { -1.0 } else { 1.0 },
            f: if flip_v { 2.0 * cy } else { 0.0 },
        };
        let (sin, cos) = rotation.to_radians().sin_cos();
        let turn = Xf {
            a: cos,
            b: -sin,
            c: cx - cos * cx + sin * cy,
            d: sin,
            e: cos,
            f: cy - sin * cx - cos * cy,
        };
        fit.then(flip).then(turn).then(self)
    }

    /// Where a shape with `anchor`, rotation and flips lands on the slide. A reflecting
    /// transform turns the shape the other way and flips it vertically.
    fn place(&self, anchor: [f64; 4], rotation: f64, flip_h: bool, flip_v: bool) -> Placed {
        let [l, t, r, b] = unswap(anchor, rotation);
        let (x, y) = self.point((l + r) / 2.0, (t + b) / 2.0);
        let (mut sx, mut sy) = (self.a.hypot(self.d), self.b.hypot(self.e));
        if (45.0..135.0).contains(&rotation.rem_euclid(180.0)) {
            std::mem::swap(&mut sx, &mut sy);
        }
        let (w, h) = ((r - l) * sx, (b - t) * sy);
        let turn = self.d.atan2(self.a).to_degrees();
        let mirrored = self.a * self.e - self.b * self.d < 0.0;
        let rotation = if mirrored {
            turn - rotation
        } else {
            turn + rotation
        };
        let emu = |v: f64| (v * EMU_PER_MASTER).round() as i64;
        Placed {
            rect: [emu(x - w / 2.0), emu(y - h / 2.0), emu(w), emu(h)],
            rotation: rotation.rem_euclid(360.0),
            flip_h,
            flip_v: flip_v ^ mirrored,
        }
    }
}

/// OfficeArt stores the anchor of a shape turned 45 to 135 degrees (or 225 to 315)
/// with width and height swapped about its centre; this swaps them back.
fn unswap([l, t, r, b]: [f64; 4], rotation: f64) -> [f64; 4] {
    let (mut w, mut h) = ((r - l).abs(), (b - t).abs());
    let (cx, cy) = ((l + r) / 2.0, (t + b) / 2.0);
    if (45.0..135.0).contains(&rotation.rem_euclid(180.0)) {
        std::mem::swap(&mut w, &mut h);
    }
    [cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0]
}

#[derive(Clone)]
enum Text {
    /// OutlineTextRefAtom: index into the sheet's SlideListWithText texts.
    Ref(usize),
    Own(Body),
}

/// What the converter reads from one OfficeArtSpContainer.
#[derive(Default, Clone)]
struct Shape<'a> {
    /// MSOSPT shape type (FSP instance).
    kind: u16,
    /// FSP flags: 0x40 flipH, 0x80 flipV, 0x400 background.
    flags: u32,
    props: Props<'a>,
    metro: Option<&'a [u8]>,
    /// ChildAnchor or ClientAnchor as (left, top, right, bottom).
    anchor: Option<[f64; 4]>,
    /// FSPGR: the coordinate space a group shape gives its members.
    space: Option<[f64; 4]>,
    text: Option<Text>,
    /// OEPlaceholderAtom placeholder type, when the shape is a placeholder.
    placeholder: Option<u8>,
    /// Characters the text's fields stand on: (position, field record type, the
    /// DateTimeMCAtom format index or 0).
    fields: Vec<(usize, u16, u8)>,
    /// Hyperlinked character ranges of the shape's text.
    links: Vec<(usize, usize)>,
    /// Automatic numbering per paragraph run, from the shape's "___PPT9" tag.
    auto_numbers: Vec<Option<(u16, i16)>>,
    /// ExObjRefAtom: the embedded OLE object the shape shows.
    ex_obj: Option<u32>,
}

impl Shape<'_> {
    fn rotation(&self) -> f64 {
        self.props
            .get(crate::office::ppt::props::id::ROTATION)
            .map_or(0.0, |v| v as i32 as f64 / 65536.0)
    }
}

struct Shapes<'s, 'a> {
    shared: &'s Shared<'a>,
    env: &'s Env<'s>,
    texts: &'s [Body],
    used: &'s mut Vec<bool>,
    items: &'s mut Vec<Item>,
    /// Slide number for slide-number fields; `None` on masters.
    number: Option<u32>,
    is_master: bool,
    master_placeholders: &'s [(u8, Props<'a>)],
    /// Whether placeholders may come from metroBlobs: the slide's layout is written.
    placeholder_metro: bool,
    /// Placeholder properties collected on a master.
    placeholders: Vec<(u8, Props<'a>)>,
    /// Date, slide-number and footer placeholders collected on a master.
    footers: Footers<'a>,
    /// Placeholder types a slide has itself.
    slide_placeholders: Vec<u8>,
    next_id: u32,
    skipped: &'s mut Skipped,
    /// A master's shapes written so far, without their ids: an exact copy of one
    /// (PowerPoint keeps such copies in some masters) is drawn once.
    seen: std::collections::HashSet<String>,
}

impl<'s, 'a> Shapes<'s, 'a> {
    /// Walks an OfficeArtSpgrContainer. Its first SpContainer is the group shape itself:
    /// its anchor (in the parent's space, `xf`) and FSPGR set the members' transform.
    fn group(&mut self, spgr: Rec, xf: Xf, depth: usize) {
        let doc = self.shared.doc;
        let mut inner = xf;
        for (i, k) in r::children(doc, spgr).into_iter().enumerate() {
            match k.kind {
                r::SP_CONTAINER => {
                    let shape = self.shape(k);
                    if i == 0 {
                        // A table, chart or SmartArt saved by PowerPoint 2007+ is a
                        // group of stand-in shapes whose group shape holds the frame.
                        if self.group_frame(&shape, xf, depth) {
                            return;
                        }
                        if let (Some(a), Some(s)) = (shape.anchor, shape.space) {
                            let (h, v) = (shape.flags & 0x40 != 0, shape.flags & 0x80 != 0);
                            inner = xf.group(a, s, shape.rotation(), h, v);
                        }
                    } else {
                        self.emit(shape, inner, depth > 0);
                    }
                }
                r::SPGR_CONTAINER if depth < MAX_GROUP_DEPTH => self.group(k, inner, depth + 1),
                r::SPGR_CONTAINER => self.skipped.add("shape group nested too deep; skipped"),
                other => self
                    .skipped
                    .add(format!("unknown record 0x{other:04X} in a shape group")),
            }
        }
    }

    /// Emits a group shape's graphic-frame metroBlob; true when it replaced the group.
    fn group_frame(&mut self, shape: &Shape, xf: Xf, depth: usize) -> bool {
        let Some(blob) = shape.metro else {
            return false;
        };
        match metro::read(blob) {
            Ok(Some(m)) if m.frame => {
                let rect = shape
                    .anchor
                    .map(|a| xf.place(a, shape.rotation(), false, false).rect);
                self.items.push(Item::Xml(m, rect.filter(|_| depth > 1)));
                true
            }
            Ok(_) => false,
            Err(e) => {
                self.skipped.add(format!("group metroBlob unusable ({e})"));
                false
            }
        }
    }

    fn shape(&mut self, sp: Rec) -> Shape<'a> {
        let d: &'a [u8] = self.shared.doc;
        let mut shape = Shape::default();
        let mut tables = Vec::new();
        let rect_i32 = |rec: Rec, order: [usize; 4]| -> Option<[f64; 4]> {
            let v: Option<Vec<f64>> = order
                .iter()
                .map(|&i| r::i32_at(d, rec.start + 4 * i).map(f64::from))
                .collect();
            v.map(|v| [v[0], v[1], v[2], v[3]])
        };
        for k in r::children(d, sp) {
            match k.kind {
                r::FSP => {
                    shape.kind = k.inst;
                    shape.flags = r::u32_at(d, k.start + 4).unwrap_or(0);
                }
                r::FSPGR => shape.space = rect_i32(k, [0, 1, 2, 3]),
                r::CHILD_ANCHOR => shape.anchor = rect_i32(k, [0, 1, 2, 3]),
                // PPT's ClientAnchor is top, left, right, bottom: SmallRectStruct (i16)
                // or, when 16 bytes long, RectStruct (i32).
                r::CLIENT_ANCHOR if shape.anchor.is_none() => {
                    shape.anchor = if k.len() >= 16 {
                        rect_i32(k, [1, 0, 2, 3])
                    } else {
                        let v = |i: usize| r::i16_at(d, k.start + 2 * i).map(f64::from);
                        (|| Some([v(1)?, v(0)?, v(2)?, v(3)?]))()
                    };
                }
                r::CLIENT_TEXTBOX => self.textbox(k, &mut shape),
                r::CLIENT_DATA => {
                    if let Some(ph) = r::child(d, k, r::OE_PLACEHOLDER_ATOM) {
                        shape.placeholder = d.get(ph.start + 4).copied();
                    }
                    if let Some(atom) = pp9_text_props(d, k) {
                        shape.auto_numbers = text::read_auto_numbers(d, atom);
                    }
                    shape.ex_obj =
                        r::child(d, k, r::EX_OBJ_REF_ATOM).and_then(|a| r::u32_at(d, a.start));
                }
                r::OPT | r::SECONDARY_OPT | r::TERTIARY_OPT => {
                    tables.push(k);
                    if self.shared.opts.metro && shape.metro.is_none() {
                        shape.metro = metro::blob(d, k);
                    }
                }
                0xF11E => {}
                other => self
                    .skipped
                    .add(format!("unknown record 0x{other:04X} in a shape")),
            }
        }
        shape.props = match shape.placeholder {
            Some(t) if !self.is_master => self.placeholder_base(t),
            _ => Props::default(),
        };
        // A picture has no outline unless it sets one itself.
        if shape.kind == native::PICTURE_FRAME {
            shape
                .props
                .add_flag(crate::office::ppt::props::id::LINE_BOOLS, 3, false);
        }
        for k in tables {
            shape.props.add(d, k);
        }
        if self.is_master
            && let Some(t) = shape.placeholder
        {
            self.placeholders.push((t, shape.props.clone()));
        }
        shape
    }

    /// Properties a slide placeholder of type `t` starts from: those of the master
    /// placeholder it stands for, or no fill and no line when the master has none.
    fn placeholder_base(&self, t: u8) -> Props<'a> {
        let master_type = match t {
            7..=10 => t,
            13 | 17 => 1,
            15 => 3,
            16 => 4,
            _ => 2,
        };
        let find = |want: u8| {
            self.master_placeholders
                .iter()
                .find(|(k, _)| *k == want)
                .map(|(_, p)| p.clone())
        };
        let fallback = match master_type {
            3 => find(1),
            4 => find(2),
            _ => None,
        };
        find(master_type).or(fallback).unwrap_or_else(|| {
            let mut p = Props::default();
            p.add_flag(crate::office::ppt::props::id::FILL_BOOLS, 4, false);
            p.add_flag(crate::office::ppt::props::id::LINE_BOOLS, 3, false);
            p
        })
    }

    fn textbox(&mut self, tb: Rec, shape: &mut Shape) {
        let d = self.shared.doc;
        let mut body: Option<Body> = None;
        let mut style = None;
        for k in r::children(d, tb) {
            match k.kind {
                r::OUTLINE_TEXT_REF_ATOM => {
                    if let Some(i) = r::u32_at(d, k.start) {
                        shape.text = Some(Text::Ref(i as usize));
                    }
                }
                r::TEXT_HEADER_ATOM => {
                    body = Some(Body {
                        text_type: r::u32_at(d, k.start).unwrap_or(4),
                        ..Body::default()
                    });
                }
                r::TEXT_CHARS_ATOM | r::TEXT_BYTES_ATOM => {
                    if let Some(b) = body.as_mut() {
                        b.units = decode_text(d, k);
                    }
                }
                r::STYLE_TEXT_PROP_ATOM => style = Some(k),
                r::TEXT_RULER_ATOM => {
                    if let Some(b) = body.as_mut() {
                        b.ruler = Some(text::read_ruler(d, k));
                    }
                }
                r::TEXT_INTERACTIVE_INFO_ATOM => shape.links.extend(link_range(d, k)),
                r::SLIDE_NUMBER_MC_ATOM
                | r::DATE_TIME_MC_ATOM
                | r::GENERIC_DATE_MC_ATOM
                | r::HEADER_MC_ATOM
                | r::FOOTER_MC_ATOM
                | r::RTF_DATE_TIME_MC_ATOM => {
                    if let Some(p) = r::u32_at(d, k.start) {
                        let format = d.get(k.start + 4).copied().unwrap_or(0);
                        shape.fields.push((p as usize, k.kind, format));
                    }
                }
                _ => {}
            }
        }
        if let Some(mut b) = body {
            if let Some(s) = style {
                b.runs = Some(text::read_runs(d, s, b.units.len()));
            }
            if !matches!(shape.text, Some(Text::Ref(_))) || !b.units.is_empty() {
                shape.text = Some(Text::Own(b));
            }
        }
    }

    fn emit(&mut self, shape: Shape<'a>, xf: Xf, in_group: bool) {
        let (h, v) = (shape.flags & 0x40 != 0, shape.flags & 0x80 != 0);
        let placed = shape.anchor.map(|a| xf.place(a, shape.rotation(), h, v));
        let rect = placed.map(|p| p.rect);
        // Master placeholders are slide templates; slides carry their own copies,
        // except of date, slide-number and footer placeholders.
        if self.is_master
            && let Some(t @ 7..=9) = shape.placeholder
        {
            self.footers.push((t, shape, xf));
            return;
        }
        if self.is_master && shape.placeholder.is_some() {
            return;
        }
        if let Some(t) = shape.placeholder {
            self.slide_placeholders.push(t);
        }
        let body: Option<Body> = match &shape.text {
            None => None,
            Some(Text::Own(b)) => {
                let mut b = b.clone();
                b.links.extend(&shape.links);
                Some(b)
            }
            Some(Text::Ref(i)) => match self.texts.get(*i) {
                Some(b) => {
                    self.used[*i] = true;
                    let mut b = b.clone();
                    b.links.extend(&shape.links);
                    Some(b)
                }
                None => {
                    self.skipped
                        .add("OutlineTextRefAtom index past the slide's texts");
                    None
                }
            },
        };
        let mut body = body;
        if let Some(runs) = body.as_mut().and_then(|b| b.runs.as_mut()) {
            for (para, auto) in runs.paras.iter_mut().zip(&shape.auto_numbers) {
                if auto.is_some() {
                    para.2.auto_number = *auto;
                }
            }
        }
        let has_text = body.as_ref().is_some_and(|b| !b.units.is_empty());
        if let Some(blob) = shape
            .metro
            .filter(|_| has_text || self.shared.opts.metro_shapes)
        {
            let binary = body.as_ref().map(Body::text).unwrap_or_default();
            match metro::read(blob).and_then(|m| m.map(|m| fill(m, &binary)).transpose()) {
                // A placeholder's XML leaves its look to its layout, so it is used
                // only on slides whose round-trip layout is written.
                Ok(Some(m))
                    if m.xml.contains("<a:t>")
                        && (self.placeholder_metro || !m.xml.contains("<p:ph")) =>
                {
                    // PowerPoint places a placeholder copy by its binary anchor: its
                    // XML may leave the frame to a layout placeholder it does not match.
                    let placed = in_group || shape.placeholder.is_some();
                    self.items.push(Item::Xml(m, rect.filter(|_| placed)));
                    return;
                }
                Ok(_) => {}
                Err(e) => self
                    .skipped
                    .add(format!("metroBlob unusable ({e}); using the binary shape")),
            }
        }
        let Some(placed) = placed else {
            self.skipped.add("shape without an anchor skipped");
            return;
        };
        let fields: Vec<(usize, String)> = shape
            .fields
            .iter()
            .map(|&(p, kind, format)| {
                let t = match kind {
                    r::SLIDE_NUMBER_MC_ATOM => self.number.map_or_else(
                        || crate::office::ppt::text::SLIDE_NUMBER_FIELD.to_string(),
                        |n| n.to_string(),
                    ),
                    r::FOOTER_MC_ATOM => self.shared.footer.clone().unwrap_or_default(),
                    // An inserted date-time field shows the conversion date, as
                    // PowerPoint shows the day it opens the file. A generic date field
                    // shows the user's fixed date when the document selects one.
                    r::DATE_TIME_MC_ATOM => {
                        crate::office::ppt::dates::format(self.shared.now, format)
                    }
                    r::GENERIC_DATE_MC_ATOM | r::RTF_DATE_TIME_MC_ATOM
                        if self.shared.hf_flags & 0x04 != 0 =>
                    {
                        self.shared.user_date.clone().unwrap_or_default()
                    }
                    r::GENERIC_DATE_MC_ATOM | r::RTF_DATE_TIME_MC_ATOM => {
                        crate::office::ppt::dates::format(self.shared.now, self.shared.date_format)
                    }
                    _ => String::new(),
                };
                (p, t)
            })
            .collect();
        let n = Native {
            kind: shape.kind,
            flip_h: placed.flip_h,
            flip_v: placed.flip_v,
            rotation: placed.rotation,
            rect: placed.rect,
            props: &shape.props,
            text: body.as_ref(),
            fields: &fields,
            equation: shape
                .ex_obj
                .is_some_and(|e| self.shared.equations.contains(&e)),
        };
        let id = self.next_id;
        self.next_id += 1;
        if let Some(xml) = native::shape(&n, self.env, id) {
            if self.is_master {
                let key = xml
                    .replace(&format!("id=\"{id}\""), "")
                    .replace(&format!(" {id}\""), "\"");
                if !self.seen.insert(key) {
                    return;
                }
            }
            self.items.push(Item::Native(xml));
        }
    }
}

/// Puts the binary text into a metro shape's masked text. Graphic frames, and shapes
/// whose binary stand-in carries no text, keep their XML text as it is.
fn fill(mut m: MetroShape, binary: &str) -> Result<MetroShape, String> {
    if !m.frame {
        m.xml = metro::fill_text(&m.xml, binary)
            .ok_or_else(|| "its text does not match the binary text".to_string())?;
    }
    Ok(m)
}

/// `theme/theme/theme1.xml` of a RoundTripTheme12Atom's zip.
fn round_trip_theme(zip: &[u8]) -> Option<Vec<u8>> {
    round_trip_part(zip, "theme/theme/theme1.xml")
}

/// One part of a round-trip atom's zip.
fn round_trip_part(zip: &[u8], name: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).ok()?;
    let f = archive.by_name(name).ok()?;
    let mut out = Vec::new();
    f.take(8 << 20).read_to_end(&mut out).ok()?;
    Some(out)
}

/// The attributes of a RoundTripColorMapping12Atom's `a:clrMap` element.
fn clr_map_attrs(xml: &str) -> Option<String> {
    let start = xml.find("<a:clrMap")? + "<a:clrMap".len();
    let end = start + xml[start..].find("/>")?;
    let attrs: Vec<&str> = xml[start..end]
        .split_whitespace()
        .filter(|a| !a.starts_with("xmlns"))
        .collect();
    (!attrs.is_empty()).then(|| attrs.join(" "))
}

/// TextInteractiveInfoAtom: the [begin, end) character range of a hyperlink.
fn link_range(doc: &[u8], atom: Rec) -> Option<(usize, usize)> {
    let a = r::u32_at(doc, atom.start)? as usize;
    let b = r::u32_at(doc, atom.start + 4)? as usize;
    (a < b).then_some((a, b))
}

/// XML text without its declaration.
fn strip_decl(xml: &str) -> String {
    match xml.find("?>") {
        Some(i) if xml.trim_start().starts_with("<?xml") => xml[i + 2..].trim_start().to_string(),
        _ => xml.trim_start().to_string(),
    }
}

/// The BinaryTagDataBlob of a ProgTags container's "___PPT9" tag; its data is a list
/// of records.
fn pp9_blob(d: &[u8], tags: Rec) -> Option<Rec> {
    r::children(d, tags)
        .into_iter()
        .filter(|t| t.kind == r::PROG_BINARY_TAG)
        .find_map(|tag| {
            let kids = r::children(d, tag);
            let name = kids.iter().find(|c| c.kind == r::CSTRING)?;
            if utf16(&d[name.start..name.end]) != "___PPT9" {
                return None;
            }
            kids.into_iter().find(|c| c.kind == r::BINARY_TAG_DATA_BLOB)
        })
}

/// The StyleTextProp9Atom of a shape's ClientData (ProgTags > "___PPT9" tag).
fn pp9_text_props(d: &[u8], client_data: Rec) -> Option<Rec> {
    let blob = pp9_blob(d, r::child(d, client_data, r::PROG_TAGS)?)?;
    r::child(d, blob, r::STYLE_TEXT_PROP9_ATOM)
}

/// Numbering of slide-list (outline) texts, kept at document level: DocInfoList >
/// ProgTags > "___PPT9" > OutlineTextProps9Container, whose entries name a slide id and
/// a text type. The n-th entry for a (slide, type) pair numbers that slide's n-th text
/// of the type.
fn apply_outline_numbering(d: &[u8], doc_children: &[Rec], entries: &mut [ListEntry]) {
    let Some(blob) = doc_children
        .iter()
        .filter(|k| k.kind == r::DOC_INFO_LIST)
        .flat_map(|k| r::children(d, *k))
        .filter(|k| k.kind == r::PROG_TAGS)
        .find_map(|tags| pp9_blob(d, tags))
    else {
        return;
    };
    let Some(outline) = r::child(d, blob, r::OUTLINE_TEXT_PROPS9) else {
        return;
    };
    let mut seen: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    let mut header: Option<(u32, u32)> = None;
    for k in r::children(d, outline) {
        match k.kind {
            r::OUTLINE_TEXT_PROPS_HEADER => {
                header = r::u32_at(d, k.start).zip(r::u32_at(d, k.start + 4));
            }
            r::STYLE_TEXT_PROP9_ATOM => {
                let Some(key) = header.take() else { continue };
                let n = seen.entry(key).or_insert(0);
                let target = entries
                    .iter_mut()
                    .filter(|e| e.slide_id == key.0)
                    .flat_map(|e| e.texts.iter_mut())
                    .filter(|b| b.text_type == key.1)
                    .nth(*n);
                *n += 1;
                let Some(runs) = target.and_then(|b| b.runs.as_mut()) else {
                    continue;
                };
                let numbers = text::read_auto_numbers(d, k);
                for (para, auto) in runs.paras.iter_mut().zip(numbers) {
                    if auto.is_some() {
                        para.2.auto_number = auto;
                    }
                }
            }
            _ => {}
        }
    }
}
