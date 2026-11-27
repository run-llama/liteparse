//! Writes the deck model as a PresentationML package: one theme, a slide master and
//! a blank layout per PPT master, and a slide per PPT slide.
//!
//! Shapes carry fully resolved formatting, so the masters only need their own shapes
//! and background; pptx-read still resolves every slide through slide -> layout ->
//! master -> theme.

use crate::office::ppt::metro::{MetroShape, Target};
use crate::office::ppt::slides::{Blip, Deck, Item, Sheet};
use std::io::Write;

const PML: &str = "application/vnd.openxmlformats-officedocument.presentationml";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS: &str = "xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"";
const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

pub fn write(deck: &Deck) -> zip::result::ZipResult<Vec<u8>> {
    let mut parts: Vec<(String, Vec<u8>)> = Vec::new();
    let mut add = |name: String, xml: String| parts.push((name, xml.into_bytes()));
    let n = deck.slides.len();
    let m = deck.masters.len();
    // Masters with their own theme use theme<i + 1>; the rest share theme1.
    let theme_of = |i: usize| {
        if deck.masters[i].theme.is_some() {
            i + 2
        } else {
            1
        }
    };
    let themes: Vec<usize> = (0..m)
        .filter(|&i| deck.masters[i].theme.is_some())
        .map(|i| i + 2)
        .collect();
    // Round-trip layouts are numbered after the masters' blank layouts (1..=m).
    let mut round_trip: Vec<(usize, u16, usize)> = Vec::new();
    for (i, sheet) in deck.masters.iter().enumerate() {
        for (inst, _) in &sheet.layouts {
            round_trip.push((i, *inst, m + 1 + round_trip.len()));
        }
    }
    let layout_of = |master: usize, inst: Option<u16>| {
        round_trip
            .iter()
            .find(|(i, k, _)| *i == master && Some(*k) == inst)
            .map_or(master + 1, |r| r.2)
    };
    add(
        "[Content_Types].xml".into(),
        content_types(n, m + round_trip.len(), &themes),
    );
    add(
        "_rels/.rels".into(),
        rels(&[("rId1", "officeDocument", "ppt/presentation.xml")]),
    );
    add("ppt/presentation.xml".into(), presentation(deck));
    // Masters take rId1..=m, the theme m+1, slides after it.
    let mut pres_rels = Vec::new();
    for i in 1..=m {
        pres_rels.push((
            format!("rId{i}"),
            "slideMaster",
            format!("slideMasters/slideMaster{i}.xml"),
        ));
    }
    pres_rels.push((
        format!("rId{}", m + 1),
        "theme",
        "theme/theme1.xml".to_string(),
    ));
    for i in 1..=n {
        pres_rels.push((
            format!("rId{}", m + 1 + i),
            "slide",
            format!("slides/slide{i}.xml"),
        ));
    }
    let refs: Vec<_> = pres_rels
        .iter()
        .map(|(a, b, c)| (a.as_str(), *b, c.as_str()))
        .collect();
    add("ppt/_rels/presentation.xml.rels".into(), rels(&refs));
    add(
        "ppt/theme/theme1.xml".into(),
        theme(deck.font.as_deref().unwrap_or("Arial")),
    );
    let mut media = Media::default();
    for (i, sheet) in deck.masters.iter().enumerate() {
        let theme_part = format!("../theme/theme{}.xml", theme_of(i));
        if let Some(t) = &sheet.theme {
            media
                .parts
                .push((format!("ppt/theme/theme{}.xml", i + 2), t.clone()));
        }
        let i = i + 1;
        let mut rel_list = vec![
            (
                "rId1".to_string(),
                format!("{REL}/slideLayout"),
                format!("../slideLayouts/slideLayout{i}.xml"),
                false,
            ),
            (
                "rId2".to_string(),
                format!("{REL}/theme"),
                theme_part,
                false,
            ),
        ];
        let mut layout_rids = vec!["rId1".to_string()];
        for (k, (_, num)) in round_trip
            .iter()
            .filter(|r| r.0 == i - 1)
            .map(|r| (r.1, r.2))
            .enumerate()
        {
            let rid = format!("rIdL{}", k + 1);
            rel_list.push((
                rid.clone(),
                format!("{REL}/slideLayout"),
                format!("../slideLayouts/slideLayout{num}.xml"),
                false,
            ));
            layout_rids.push(rid);
        }
        let (tree, bg) = sheet_xml(deck, sheet, &mut rel_list, &mut media);
        add(
            format!("ppt/slideMasters/slideMaster{i}.xml"),
            master(
                i,
                &tree,
                &bg,
                sheet.clr_map.as_deref(),
                &layout_rids,
                sheet.tx_styles.as_deref(),
            ),
        );
        for ((_, l), num) in sheet
            .layouts
            .iter()
            .zip(round_trip.iter().filter(|r| r.0 == i - 1).map(|r| r.2))
        {
            let mut layout_rels = vec![(
                "rIdMaster".to_string(),
                format!("{REL}/slideMaster"),
                format!("../slideMasters/slideMaster{i}.xml"),
                false,
            )];
            let xml = round_trip_layout(l, &mut layout_rels, &mut media);
            add(format!("ppt/slideLayouts/slideLayout{num}.xml"), xml);
            add(
                format!("ppt/slideLayouts/_rels/slideLayout{num}.xml.rels"),
                rels_owned(&layout_rels),
            );
        }
        add(
            format!("ppt/slideMasters/_rels/slideMaster{i}.xml.rels"),
            rels_owned(&rel_list),
        );
        add(format!("ppt/slideLayouts/slideLayout{i}.xml"), layout());
        add(
            format!("ppt/slideLayouts/_rels/slideLayout{i}.xml.rels"),
            rels(&[(
                "rId1",
                "slideMaster",
                &format!("../slideMasters/slideMaster{i}.xml"),
            )]),
        );
    }
    for (i, s) in deck.slides.iter().enumerate() {
        let mut rel_list = vec![(
            "rId1".to_string(),
            format!("{REL}/slideLayout"),
            format!(
                "../slideLayouts/slideLayout{}.xml",
                layout_of(s.master.min(m.saturating_sub(1)), s.layout)
            ),
            false,
        )];
        let (tree, bg) = sheet_xml(deck, &s.sheet, &mut rel_list, &mut media);
        let show = if s.hidden { " show=\"0\"" } else { "" };
        let master_sp = if s.master_objects {
            ""
        } else {
            " showMasterSp=\"0\""
        };
        add(
            format!("ppt/slides/slide{}.xml", i + 1),
            format!(
                "{DECL}<p:sld {NS}{show}{master_sp}><p:cSld>{bg}<p:spTree>{GROUP_PROPS}{tree}</p:spTree></p:cSld>\
<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"
            ),
        );
        add(
            format!("ppt/slides/_rels/slide{}.xml.rels", i + 1),
            rels_owned(&rel_list),
        );
    }
    parts.extend(media.parts);
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in &parts {
        out.start_file(name.as_str(), options)?;
        out.write_all(bytes)?;
    }
    Ok(out.finish()?.into_inner())
}

/// Relationship entries as (id, full type URI, target, external).
type RelList = Vec<(String, String, String, bool)>;

fn rels_owned(list: &RelList) -> String {
    let refs: Vec<_> = list
        .iter()
        .map(|(a, b, c, d)| (a.clone(), b.as_str(), c.clone(), *d))
        .collect();
    rels_full(&refs)
}

/// Package parts shared between sheets: picture-store images (written once, by
/// `pib`) and metroBlob folders.
#[derive(Default)]
struct Media {
    parts: Vec<(String, Vec<u8>)>,
    pictures: std::collections::HashMap<u32, Option<String>>,
    blobs: usize,
}

impl Media {
    /// The part name of picture `pib`, written on first use.
    fn picture(&mut self, blips: &[Blip], pib: u32) -> Option<String> {
        if let Some(found) = self.pictures.get(&pib) {
            return found.clone();
        }
        let entry = blips
            .get((pib as usize).checked_sub(1)?)
            .and_then(|b| b.as_ref());
        let name = entry.map(|(format, bytes)| {
            let (ext, bytes) = match *format {
                "dib" => (
                    "bmp",
                    crate::office::doc::docx_export::blips::dib_to_bmp(bytes),
                ),
                "tiff" => ("tif", bytes.clone()),
                f => (f, bytes.clone()),
            };
            let name = format!("media/image{pib}.{ext}");
            self.parts.push((format!("ppt/{name}"), bytes));
            name
        });
        self.pictures.insert(pib, name.clone());
        name
    }
}

/// A sheet's shape tree XML and `p:bg` element, adding relationships to `rel_list`.
fn sheet_xml(
    deck: &Deck,
    sheet: &Sheet,
    rel_list: &mut RelList,
    media: &mut Media,
) -> (String, String) {
    let mut shapes = String::new();
    for item in &sheet.items {
        match item {
            Item::Native(xml) => shapes.push_str(&pictures(deck, xml, rel_list, media)),
            Item::Xml(m, rect) => shapes.push_str(&metro_shape(m, *rect, rel_list, media)),
        }
    }
    let bg = match &sheet.background {
        Some(bg) => format!("<p:bg>{}</p:bg>", pictures(deck, bg, rel_list, media)),
        None => String::new(),
    };
    (shapes, bg)
}

/// Replaces `r:embed="PIB<n>"` with a relationship to picture `n`. A picture missing
/// from the store loses its `a:blip`, which pptx-read draws as nothing.
fn pictures(deck: &Deck, xml: &str, rel_list: &mut RelList, media: &mut Media) -> String {
    const KEY: &str = "r:embed=\"PIB";
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(at) = rest.find(KEY) {
        out.push_str(&rest[..at]);
        let tail = &rest[at + KEY.len()..];
        let end = tail.find('"').unwrap_or(tail.len());
        let pib: u32 = tail[..end].parse().unwrap_or(0);
        match media.picture(&deck.blips, pib) {
            Some(name) => {
                let id = format!("rIdP{}", rel_list.len() + 1);
                rel_list.push((
                    id.clone(),
                    format!("{REL}/image"),
                    format!("../{name}"),
                    false,
                ));
                out.push_str(&format!("r:embed=\"{id}\""));
            }
            None => out.push_str("r:embed=\"\""),
        }
        rest = &tail[(end + 1).min(tail.len())..];
    }
    out.push_str(rest);
    out
}

fn content_types(slides: usize, masters: usize, themes: &[usize]) -> String {
    let media: String = [
        ("png", "image/png"),
        ("jpeg", "image/jpeg"),
        ("jpg", "image/jpeg"),
        ("gif", "image/gif"),
        ("bmp", "image/bmp"),
        ("dib", "image/bmp"),
        ("tif", "image/tiff"),
        ("tiff", "image/tiff"),
        ("emf", "image/x-emf"),
        ("wmf", "image/x-wmf"),
        ("svg", "image/svg+xml"),
        ("wdp", "image/vnd.ms-photo"),
    ]
    .iter()
    .map(|(e, t)| format!("<Default Extension=\"{e}\" ContentType=\"{t}\"/>"))
    .collect();
    let mut s = format!(
        "{DECL}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>{media}\
<Override PartName=\"/ppt/presentation.xml\" ContentType=\"{PML}.presentation.main+xml\"/>\
<Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>"
    );
    for t in themes {
        s.push_str(&format!(
            "<Override PartName=\"/ppt/theme/theme{t}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>"
        ));
    }
    for i in 1..=masters {
        s.push_str(&format!(
            "<Override PartName=\"/ppt/slideMasters/slideMaster{i}.xml\" ContentType=\"{PML}.slideMaster+xml\"/>\
<Override PartName=\"/ppt/slideLayouts/slideLayout{i}.xml\" ContentType=\"{PML}.slideLayout+xml\"/>"
        ));
    }
    for i in 1..=slides {
        s.push_str(&format!(
            "<Override PartName=\"/ppt/slides/slide{i}.xml\" ContentType=\"{PML}.slide+xml\"/>"
        ));
    }
    s.push_str("</Types>");
    s
}

fn rels(entries: &[(&str, &str, &str)]) -> String {
    let full: Vec<String> = entries
        .iter()
        .map(|(_, k, _)| format!("{REL}/{k}"))
        .collect();
    let list: Vec<_> = entries
        .iter()
        .zip(&full)
        .map(|((id, _, target), kind)| (id.to_string(), kind.as_str(), target.to_string(), false))
        .collect();
    rels_full(&list)
}

/// Relationships as (id, full type URI, target, external).
fn rels_full(entries: &[(String, &str, String, bool)]) -> String {
    let mut s = format!(
        "{DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">"
    );
    for (id, kind, target, external) in entries {
        let mode = if *external {
            " TargetMode=\"External\""
        } else {
            ""
        };
        s.push_str(&format!(
            "<Relationship Id=\"{id}\" Type=\"{kind}\" Target=\"{}\"{mode}/>",
            escape(target)
        ));
    }
    s.push_str("</Relationships>");
    s
}

/// Level properties for a text style: `sizes` in hundredths of a point per level.
fn levels(sizes: &[u32], bullets: bool) -> String {
    let mut s = String::new();
    for (i, sz) in sizes.iter().enumerate() {
        let indent = 342_900 * i as i64;
        let bullet = if bullets {
            format!(
                "marL=\"{}\" indent=\"-342900\"><a:buFont typeface=\"Arial\"/><a:buChar char=\"&#8226;\"/",
                indent + 342_900
            )
        } else {
            format!("marL=\"{indent}\" indent=\"0\"><a:buNone/")
        };
        s.push_str(&format!(
            "<a:lvl{l}pPr {bullet}><a:defRPr sz=\"{sz}\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill>\
<a:latin typeface=\"+mn-lt\"/><a:ea typeface=\"+mn-ea\"/><a:cs typeface=\"+mn-cs\"/></a:defRPr></a:lvl{l}pPr>",
            l = i + 1
        ));
    }
    s
}

fn presentation(deck: &Deck) -> String {
    let mut ids = String::new();
    for i in 0..deck.slides.len() {
        ids.push_str(&format!(
            "<p:sldId id=\"{}\" r:id=\"rId{}\"/>",
            256 + i,
            i + 2 + deck.masters.len()
        ));
    }
    let masters: String = (0..deck.masters.len())
        .map(|i| {
            format!(
                "<p:sldMasterId id=\"{}\" r:id=\"rId{}\"/>",
                2147483648u64 + 256 * i as u64,
                i + 1
            )
        })
        .collect();
    format!(
        "{DECL}<p:presentation {NS}><p:sldMasterIdLst>{masters}</p:sldMasterIdLst>\
<p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx=\"{w}\" cy=\"{h}\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/>\
<p:defaultTextStyle>{lv}</p:defaultTextStyle></p:presentation>",
        w = deck.width,
        h = deck.height,
        lv = levels(&[1800; 5], false)
    )
}

fn theme(font: &str) -> String {
    let font = escape(font);
    let solid = |c: &str| format!("<a:solidFill><a:schemeClr val=\"{c}\"/></a:solidFill>");
    let fills = format!("{}{}{}", solid("phClr"), solid("phClr"), solid("phClr"));
    let line = format!("<a:ln w=\"9525\">{}</a:ln>", solid("phClr"));
    let colors = [
        ("dk1", "000000"),
        ("lt1", "FFFFFF"),
        ("dk2", "1F497D"),
        ("lt2", "EEECE1"),
        ("accent1", "4F81BD"),
        ("accent2", "C0504D"),
        ("accent3", "9BBB59"),
        ("accent4", "8064A2"),
        ("accent5", "4BACC6"),
        ("accent6", "F79646"),
        ("hlink", "0000FF"),
        ("folHlink", "800080"),
    ]
    .iter()
    .map(|(n, v)| format!("<a:{n}><a:srgbClr val=\"{v}\"/></a:{n}>"))
    .collect::<String>();
    format!(
        "{DECL}<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"PPT\">\
<a:themeElements><a:clrScheme name=\"PPT\">{colors}</a:clrScheme>\
<a:fontScheme name=\"PPT\"><a:majorFont><a:latin typeface=\"{font}\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:majorFont>\
<a:minorFont><a:latin typeface=\"{font}\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:minorFont></a:fontScheme>\
<a:fmtScheme name=\"PPT\"><a:fillStyleLst>{fills}</a:fillStyleLst><a:lnStyleLst>{line}{line}{line}</a:lnStyleLst>\
<a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle>\
<a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst>{fills}</a:bgFillStyleLst></a:fmtScheme>\
</a:themeElements></a:theme>"
    )
}

const GROUP_PROPS: &str = "<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
<p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>";

const CLR_MAP: &str = "<p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" \
accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>";

/// A slide master holding a PPT master's own shapes and background. The text styles
/// only matter to shapes without explicit formatting, such as metroBlob shapes.
fn master(
    index: usize,
    tree: &str,
    bg: &str,
    clr_map: Option<&str>,
    layout_rids: &[String],
    tx_styles: Option<&str>,
) -> String {
    let base = 2147483648u64 + 256 * (index as u64 - 1);
    let layouts: String = layout_rids
        .iter()
        .enumerate()
        .map(|(k, rid)| {
            format!(
                "<p:sldLayoutId id=\"{}\" r:id=\"{rid}\"/>",
                base + 1 + k as u64
            )
        })
        .collect();
    let clr = match clr_map {
        Some(a) => format!("<p:clrMap {a}/>"),
        None => CLR_MAP.to_string(),
    };
    let bg = if bg.is_empty() {
        "<p:bg><p:bgRef idx=\"1001\"><a:schemeClr val=\"bg1\"/></p:bgRef></p:bg>"
    } else {
        bg
    };
    let title_style = levels(&[4400], false).replace("+mn-", "+mj-");
    let styles = match tx_styles {
        Some(t) => t.to_string(),
        None => format!(
            "<p:txStyles><p:titleStyle>{title_style}</p:titleStyle><p:bodyStyle>{}</p:bodyStyle>\
<p:otherStyle>{}</p:otherStyle></p:txStyles>",
            levels(&[3200, 2800, 2400, 2000, 2000], true),
            levels(&[1800; 5], false)
        ),
    };
    format!(
        "{DECL}<p:sldMaster {NS}><p:cSld>{bg}<p:spTree>{GROUP_PROPS}{tree}</p:spTree></p:cSld>{clr}\
<p:sldLayoutIdLst>{layouts}</p:sldLayoutIdLst>{styles}</p:sldMaster>"
    )
}

/// A round-trip layout with only its placeholders: they give slide placeholders
/// their frames and text styles. Its other shapes and background are already in the
/// PPT master written for that layout.
fn round_trip_layout(l: &MetroShape, rel_list: &mut RelList, media: &mut Media) -> String {
    let xml = placeholders_only(&l.xml);
    let rels = l
        .rels
        .iter()
        .filter(|r| !r.kind.ends_with("/slideMaster"))
        .map(|r| crate::office::ppt::metro::Rel {
            id: r.id.clone(),
            kind: r.kind.clone(),
            target: match &r.target {
                Target::Part(p) => Target::Part(p.clone()),
                Target::External(u) => Target::External(u.clone()),
            },
        })
        .collect();
    let kept = MetroShape {
        xml,
        frame: false,
        rels,
        parts: l.parts.clone(),
    };
    format!("{DECL}{}", metro_shape(&kept, None, rel_list, media))
}

/// Layout XML without its background and without shape-tree children that hold no
/// placeholder.
fn placeholders_only(xml: &str) -> String {
    let Ok(doc) = roxmltree::Document::parse(xml) else {
        return xml.to_string();
    };
    let mut cut: Vec<std::ops::Range<usize>> = Vec::new();
    for n in doc.descendants().filter(|n| n.is_element()) {
        match n.tag_name().name() {
            "bg" if n.parent().is_some_and(|p| p.tag_name().name() == "cSld") => {
                cut.push(n.range())
            }
            "spTree" => {
                for c in n.children().filter(|c| c.is_element()) {
                    let keep = matches!(c.tag_name().name(), "nvGrpSpPr" | "grpSpPr")
                        || c.descendants().any(|d| d.tag_name().name() == "ph");
                    if !keep {
                        cut.push(c.range());
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = xml.to_string();
    cut.sort_by_key(|r| std::cmp::Reverse(r.start));
    for r in cut {
        out.replace_range(r, "");
    }
    out
}

fn layout() -> String {
    format!(
        "{DECL}<p:sldLayout {NS} type=\"blank\" preserve=\"1\"><p:cSld name=\"Blank\"><p:spTree>{GROUP_PROPS}</p:spTree></p:cSld>\
<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"
    )
}

/// A metro shape's XML with its relationship ids renamed into the slide's id space
/// and, for group members, its frame set to `rect`. The blob's parts are copied to
/// `ppt/metro/m<n>/`, keeping their layout below the blob's `drs/` folder.
fn metro_shape(
    m: &MetroShape,
    rect: Option<[i64; 4]>,
    rel_list: &mut RelList,
    media: &mut Media,
) -> String {
    let mut xml = m.xml.clone();
    let folder = if m.parts.is_empty() {
        String::new()
    } else {
        media.blobs += 1;
        format!("metro/m{}", media.blobs)
    };
    for (path, bytes) in &m.parts {
        media
            .parts
            .push((format!("ppt/{folder}/{path}"), bytes.clone()));
    }
    for rel in &m.rels {
        let id = format!("rIdM{}", rel_list.len() + 1);
        xml = xml.replace(&format!("=\"{}\"", rel.id), &format!("=\"{id}\""));
        match &rel.target {
            Target::External(url) => rel_list.push((id, rel.kind.clone(), url.clone(), true)),
            Target::Part(path) => {
                rel_list.push((id, rel.kind.clone(), format!("../{folder}/{path}"), false))
            }
        }
    }
    match rect {
        Some(r) => set_frame(&xml, r),
        None => xml,
    }
}

/// Rewrites the first `a:off`/`a:ext` pair of the shape's first transform.
fn set_frame(xml: &str, [x, y, w, h]: [i64; 4]) -> String {
    let Some(xfrm) = xml.find(":xfrm") else {
        // No transform of its own (a placeholder): add one to the shape properties.
        let frame =
            format!("<a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{w}\" cy=\"{h}\"/></a:xfrm>");
        if let Some(i) = xml.find("<p:spPr/>") {
            let mut out = xml.to_string();
            out.replace_range(
                i..i + "<p:spPr/>".len(),
                &format!("<p:spPr>{frame}</p:spPr>"),
            );
            return out;
        }
        return match xml.find("<p:spPr>") {
            Some(i) => {
                let mut out = xml.to_string();
                out.insert_str(i + "<p:spPr>".len(), &frame);
                out
            }
            None => xml.to_string(),
        };
    };
    let mut out = xml.to_string();
    for (tag, a, b) in [
        (":off ", ("x", x), ("y", y)),
        (":ext ", ("cx", w), ("cy", h)),
    ] {
        let Some(at) = out[xfrm..].find(tag).map(|i| i + xfrm) else {
            continue;
        };
        let Some(end) = out[at..].find('>').map(|i| i + at) else {
            continue;
        };
        let mut t = out[at..end].to_string();
        for (name, v) in [a, b] {
            t = set_attr(&t, name, v);
        }
        out.replace_range(at..end, &t);
    }
    out
}

fn set_attr(tag: &str, name: &str, value: i64) -> String {
    let key = format!(" {name}=\"");
    match tag.find(&key) {
        Some(i) => {
            let start = i + key.len();
            let end = tag[start..].find('"').map_or(tag.len(), |j| start + j);
            format!("{}{value}{}", &tag[..start], &tag[end..])
        }
        None => tag.to_string(),
    }
}

/// XML-escapes text and drops characters XML 1.0 cannot carry.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' | '\n' => out.push(c),
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => {}
            c => out.push(c),
        }
    }
    out
}
