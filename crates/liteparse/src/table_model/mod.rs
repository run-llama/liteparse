//! Learned table structure (TableNet): text units on a page -> tables with rows, columns, cells,
//! spans and header rows. A port of the Python pipeline in `bench/tables/` (units.py, rules.py,
//! tn_core.py, tablenet.py); parity is checked stage by stage against fixtures written by
//! `bench/tables/dump_parity.py` (see the tests at the bottom).

pub mod decode;
pub mod features;
pub mod net;
pub mod units;

/// Units running against the page's main direction (a sidebar, a rotated label) take part in
/// the forward pass but are never table cells: their logits are set to -9 (as tablelab.run_model).
pub fn mask_minor(l: &mut net::Logits, minor: &[bool]) {
    let n = l.n;
    for (i, &m) in minor.iter().enumerate() {
        if !m {
            continue;
        }
        l.node[i * 2] = -9.0;
        l.node[i * 2 + 1] = -9.0;
        for k in 0..4 {
            for j in 0..n {
                l.pair[(k * n + i) * n + j] = -9.0;
                l.pair[(k * n + j) * n + i] = -9.0;
            }
        }
    }
}

/// Pages with more units than this skip the model (attention and decoding grow with N^2; such
/// pages are dense prose or indexes, where the heuristic path is kept).
pub const MAX_UNITS: usize = 1500;

/// What the model needs from an open PDFium page (built while the document is open: PDFium
/// isn't thread-safe), joined later with that page's projection signals.
#[derive(Clone, Debug)]
pub struct PageInput {
    pub units: Vec<units::Unit>,
    /// rendered page size in points
    pub wh: (f64, f64),
    pub rules: Option<features::Rules>,
    /// no usable text layer: the units come from the page's OCR items at `run_page`
    pub ocr: bool,
    /// the page for a model with an image branch
    pub img: Option<features::PageImage>,
    /// the page render, when `rules` / `img` are left to `run_page` (on the parallel workers,
    /// off the PDFium lock); with the (rules, img) wanted from it
    raw: Option<(features::RawRaster, bool, bool)>,
}

impl PageInput {
    /// Ruling lines and image input: built here from the deferred render, or as stored.
    fn raster(&self) -> (Option<features::Rules>, Option<features::PageImage>) {
        match &self.raw {
            Some((raw, r, i)) => features::process_raster(raw, *r, *i),
            None => (None, None),
        }
    }

    /// Render the page for the ruling lines and the image branch, for an input built by
    /// [`page_input`] with neither. The render (unlike the units, whose character boxes PDFium
    /// reports differently once extraction has run on the page) is the same from a reopened
    /// document, so a parse can build it later, overlapping OCR.
    pub fn attach_render(&mut self, page: &pdfium::Page, with_rules: bool, with_img: bool) {
        if !(with_rules || with_img) {
            return;
        }
        self.raw = features::render_raw(page).map(|r| (r, with_rules, with_img));
    }
}

/// Units and ruling lines of a page. A page without a usable text layer (under 20 characters)
/// is an OCR page: its units are built from LiteParse's OCR items once projected (`run_page`).
pub fn page_input(page: &pdfium::Page, with_rules: bool, with_img: bool) -> Option<PageInput> {
    let (units, wh) = units::pdf_units(page);
    if units.len() > MAX_UNITS {
        return None;
    }
    let ocr = units.iter().map(|u| u.text.chars().count()).sum::<usize>() < 20;
    let units = if ocr { Vec::new() } else { units };
    // One render serves the ruling lines and the image branch. Only the render needs PDFium;
    // turning it into rules and the image input is deferred to `run_page`, which runs pages on
    // the parallel workers.
    let raw = (with_rules || with_img)
        .then(|| features::render_raw(page).map(|r| (r, with_rules, with_img)))
        .flatten();
    Some(PageInput {
        units,
        wh,
        rules: None,
        ocr,
        img: None,
        raw,
    })
}

/// An OCR page's units and their projection signals, straight from LiteParse's projected OCR
/// items (units.py `_lit_ocr_units`): one unit per item, box in page points, height un-padded
/// (/1.4), leaf ids numbered in item order.
fn ocr_units(
    input: &PageInput,
    sig: &crate::projection::ProjSignals,
) -> (Vec<units::Unit>, Vec<Option<features::Proj>>) {
    let (sx, sy) = (
        input.wh.0 / sig.width.max(1.0) as f64,
        input.wh.1 / sig.height.max(1.0) as f64,
    );
    let r1 = |v: f32| (v * 10.0).round() as i64;
    let mut ids: Vec<&Vec<u16>> = Vec::new();
    let (mut us, mut proj) = (Vec::new(), Vec::new());
    for it in &sig.items {
        let t = it.text.trim();
        let [x, y, w, h] = it.bx;
        if t.is_empty() || w <= 0.0 || h <= 0.0 {
            continue;
        }
        let bx = [
            x as f64 * sx,
            y as f64 * sy,
            (x + w) as f64 * sx,
            (y + h) as f64 * sy,
        ];
        let path = sig
            .leaves
            .iter()
            .find(|(b, _)| r1(b[0]) == r1(x) && r1(b[1]) == r1(y))
            .map(|l| &l.1);
        let leaf = match path {
            Some(p) => match ids.iter().position(|q| *q == p) {
                Some(i) => i as f32,
                None => {
                    ids.push(p);
                    (ids.len() - 1) as f32
                }
            },
            None => -1.0,
        };
        us.push(units::Unit {
            bx,
            pbox: bx,
            text: t.to_string(),
            h: (bx[3] - bx[1]) / 1.4,
            size: 0.0,
            bold: None,
            q: 0,
            minor: false,
            words: Vec::new(),
        });
        proj.push(Some(features::Proj {
            feat_text: it.faint.then(|| faint_mask(t)),
            flow: it.flow as u8 as f32,
            lc: it.lc as f32,
            rc: it.rc as f32,
            cc: it.cc as f32,
            snap: it.snap,
            block: it.block as f32,
            leaf,
        }));
    }
    (us, proj)
}

/// The text the table model's features read for an OCR faint-token read. LayoutNet reads a lone
/// value-like token ("1", "€1", "1.0") recovered between two cells as a reason to fuse the table's
/// rows (issue #615): the same token as an opaque word leaves the table intact. Tuned to the
/// LayoutNet of OCRia-nano-1 v1.0.0; a retrained model that has seen such tokens may not need it.
fn faint_mask(t: &str) -> String {
    "x".repeat(t.chars().count())
}

/// One unit's block class probabilities (layout models): its box on the page (points, top-left
/// origin), character count, and softmax over `BLOCK_CLASSES`.
#[derive(Clone, Debug)]
pub struct UnitClass {
    pub bbox: crate::types::Rect,
    pub chars: usize,
    pub probs: Vec<f32>,
}

/// Block classes of a layout model's class head (build_grounded.py BLOCK_CLASSES).
pub const BLOCK_CLASSES: [&str; 13] = [
    "Text",
    "Heading",
    "Title",
    "PageHeader",
    "PageFooter",
    "List",
    "Table",
    "Caption",
    "Footnote",
    "Formula",
    "Code",
    "FigureText",
    "KeyValue",
];

/// A layout block from a joint model: units grouped by the same_block
/// head, labelled by their summed class probabilities.
#[derive(Clone, Debug)]
pub struct ModelBlock {
    pub bbox: crate::types::Rect,
    /// index into BLOCK_CLASSES
    pub class: usize,
    /// the units' text in reading order
    pub text: String,
}

/// What the model found on a page: its tables and (layout models) every unit's block class and
/// its layout blocks.
#[derive(Clone, Debug, Default)]
pub struct PageModel {
    pub tables: Vec<ModelTable>,
    pub classes: Vec<UnitClass>,
    pub blocks: Vec<ModelBlock>,
}

/// A block's units split into its lines' runs: a new part starts where the vertical gap to the
/// next line is more than the block's median line height (a paragraph break; leading alone is a
/// fraction of it), or where the lines change between heading-like (the class head's Heading +
/// Title share over half, by characters) and body text, since the same_block head often joins
/// a heading to the paragraph under it.
fn split_paragraphs(g: &[usize], units: &[units::Unit], cls: &[f32], c: usize) -> Vec<Vec<usize>> {
    if g.len() < 2 || c < 3 {
        return vec![g.to_vec()];
    }
    let mut by_y: Vec<usize> = g.to_vec();
    by_y.sort_by(|&a, &b| {
        let (pa, pb) = (units[a].pbox, units[b].pbox);
        ((pa[1] + pa[3]) / 2.0).total_cmp(&((pb[1] + pb[3]) / 2.0))
    });
    // lines: units overlapping the current line by more than half the smaller height
    let mut lines: Vec<(f64, f64, Vec<usize>)> = Vec::new();
    for i in by_y {
        let p = units[i].pbox;
        match lines.last_mut() {
            Some(l) if (l.1.min(p[3]) - l.0.max(p[1])) > 0.5 * (l.1 - l.0).min(p[3] - p[1]) => {
                l.0 = l.0.min(p[1]);
                l.1 = l.1.max(p[3]);
                l.2.push(i);
            }
            _ => lines.push((p[1], p[3], vec![i])),
        }
    }
    if lines.len() < 2 {
        return vec![g.to_vec()];
    }
    let mut hs: Vec<f64> = lines.iter().map(|l| l.1 - l.0).collect();
    hs.sort_by(f64::total_cmp);
    let line_h = hs[hs.len() / 2].max(1.0);
    let headingish = |l: &[usize]| {
        let (mut head, mut all) = (0f32, 0f32);
        for &i in l {
            let row = &cls[i * c..(i + 1) * c];
            let m = row.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let e: Vec<f32> = row.iter().map(|v| (v - m).exp()).collect();
            let z: f32 = e.iter().sum();
            let n = units[i]
                .text
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .count()
                .max(1) as f32;
            head += n * (e[1] + e[2]) / z;
            all += n;
        }
        head * 2.0 > all
    };
    let mut parts: Vec<Vec<usize>> = vec![lines[0].2.clone()];
    for k in 1..lines.len() {
        let gap = lines[k].0 - lines[k - 1].1;
        if gap > line_h || headingish(&lines[k].2) != headingish(&lines[k - 1].2) {
            parts.push(Vec::new());
        }
        parts.last_mut().unwrap().extend_from_slice(&lines[k].2);
    }
    parts
}

/// A block's units split where their horizontal extents leave a gap wider than the units' median
/// height (about three word spaces), keeping the original order inside each part.
fn split_at_gaps(g: &[usize], units: &[units::Unit]) -> Vec<Vec<usize>> {
    let mut hs: Vec<f64> = g
        .iter()
        .map(|&i| units[i].pbox[3] - units[i].pbox[1])
        .collect();
    hs.sort_by(f64::total_cmp);
    let gap = hs.get(hs.len() / 2).copied().unwrap_or(0.0).max(1.0);
    let mut by_x: Vec<usize> = g.to_vec();
    by_x.sort_by(|&a, &b| units[a].pbox[0].total_cmp(&units[b].pbox[0]));
    let mut parts: Vec<Vec<usize>> = Vec::new();
    let mut right = f64::NEG_INFINITY;
    for i in by_x {
        let p = units[i].pbox;
        if parts.is_empty() || p[0] - right > gap {
            parts.push(Vec::new());
        }
        parts.last_mut().unwrap().push(i);
        right = right.max(p[2]);
    }
    parts
}

/// Units grouped into blocks from same_block logits: connected components of positive links,
/// then average linkage inside each component (average linkage over a whole dense page is
/// O(n^3); its components are small).
fn block_groups(l: &net::Logits, keep: &[usize]) -> Vec<Vec<usize>> {
    let n = l.n;
    let m = keep.len();
    let mut parent: Vec<usize> = (0..m).collect();
    fn root(p: &mut [usize], mut k: usize) -> usize {
        while p[k] != k {
            p[k] = p[p[k]];
            k = p[k];
        }
        k
    }
    for a in 0..m {
        for b in a + 1..m {
            if l.pair[(4 * n + keep[a]) * n + keep[b]] > 0.0 {
                let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
                if ra != rb {
                    parent[ra] = rb;
                }
            }
        }
    }
    let mut comps: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for a in 0..m {
        let r = root(&mut parent, a);
        comps.entry(r).or_default().push(a);
    }
    let mut out = Vec::new();
    for (_, c) in comps {
        let k = c.len();
        let mut p = vec![0f64; k * k];
        for x in 0..k {
            for y in 0..k {
                p[x * k + y] = l.pair[(4 * n + keep[c[x]]) * n + keep[c[y]]] as f64;
            }
        }
        for g in decode::avg_link(&p, k, 0.0) {
            out.push(g.into_iter().map(|x| keep[c[x]]).collect());
        }
    }
    out
}

/// A decoded table: its grid and its box on the page (points, top-left origin).
#[derive(Clone, Debug)]
pub struct ModelTable {
    pub bbox: crate::types::Rect,
    pub grid: decode::Grid,
}

/// Each unit's projection signals: the projected items whose centres fall in its page box
/// (padded by a fifth of its height) - flow averaged, anchor counts maxed, snap / block / leaf by
/// vote (first seen wins a tie) - as units.projection_signals does with the dump.
fn join_signals(
    input: &PageInput,
    sig: &crate::projection::ProjSignals,
) -> Vec<Option<features::Proj>> {
    let (w, h) = input.wh;
    let (sx, sy) = (
        w / sig.width.max(1.0) as f64,
        h / sig.height.max(1.0) as f64,
    );
    let centre = |b: &[f32; 4]| {
        (
            (b[0] + b[2] / 2.0) as f64 * sx,
            (b[1] + b[3] / 2.0) as f64 * sy,
        )
    };
    let items: Vec<(f64, f64)> = sig.items.iter().map(|it| centre(&it.bx)).collect();
    let mut leaf_ids: Vec<&Vec<u16>> = Vec::new();
    let leaves: Vec<(f64, f64, usize)> = sig
        .leaves
        .iter()
        .map(|(b, path)| {
            let id = match leaf_ids.iter().position(|p| *p == path) {
                Some(i) => i,
                None => {
                    leaf_ids.push(path);
                    leaf_ids.len() - 1
                }
            };
            let (cx, cy) = centre(b);
            (cx, cy, id)
        })
        .collect();
    fn vote<T: PartialEq + Copy>(v: &[T]) -> Option<T> {
        let mut c: Vec<(T, usize)> = Vec::new();
        for &x in v {
            match c.iter_mut().find(|(k, _)| *k == x) {
                Some(e) => e.1 += 1,
                None => c.push((x, 1)),
            }
        }
        let mut best: Option<(T, usize)> = None;
        for (k, n) in c {
            if best.is_none_or(|(_, bn)| n > bn) {
                best = Some((k, n));
            }
        }
        best.map(|b| b.0)
    }
    input
        .units
        .iter()
        .map(|u| {
            let b = u.pbox;
            let pad = 0.2 * (b[3] - b[1]);
            let inside = |x: f64, y: f64| {
                b[0] - pad <= x && x <= b[2] + pad && b[1] - pad <= y && y <= b[3] + pad
            };
            let hit: Vec<&crate::projection::ProjSignalItem> = sig
                .items
                .iter()
                .zip(&items)
                .filter(|(_, p)| inside(p.0, p.1))
                .map(|(it, _)| it)
                .collect();
            if hit.is_empty() {
                return None;
            }
            let lv: Vec<usize> = leaves
                .iter()
                .filter(|&&(x, y, _)| inside(x, y))
                .map(|l| l.2)
                .collect();
            Some(features::Proj {
                feat_text: None,
                flow: hit.iter().map(|it| it.flow as u8 as f32).sum::<f32>() / hit.len() as f32,
                lc: hit.iter().map(|it| it.lc).max().unwrap() as f32,
                rc: hit.iter().map(|it| it.rc).max().unwrap() as f32,
                cc: hit.iter().map(|it| it.cc).max().unwrap() as f32,
                snap: vote(&hit.iter().map(|it| it.snap).collect::<Vec<_>>()).unwrap_or(' '),
                block: vote(&hit.iter().map(|it| it.block).collect::<Vec<_>>()).unwrap_or(0) as f32,
                leaf: vote(&lv).map_or(-1.0, |l| l as f32),
            })
        })
        .collect()
}

/// Run the model on one page. A page whose text mostly runs in one rotated direction (a table
/// set sideways) is turned upright first: units, ruling lines and the page image all go into that
/// reading frame, which is how the model saw tables in training, and every box it returns comes
/// back to the page. Left as is, the units' reading-frame geometry and the sideways render
/// disagree and the model finds a fragment of the table.
pub fn run_page(
    net: &net::TableNet,
    layout_net: Option<&net::TableNet>,
    input: &PageInput,
    sig: Option<&crate::projection::ProjSignals>,
    opt: &decode::DecodeOptions,
) -> PageModel {
    let Some(q) = main_rotation(input) else {
        return run_page_framed(net, layout_net, input, sig, opt);
    };
    let (w, h) = input.wh;
    let (rules, img) = {
        let deferred = input.raster();
        (
            input.rules.clone().or(deferred.0),
            input.img.clone().or(deferred.1),
        )
    };
    let upright = PageInput {
        units: input
            .units
            .iter()
            .map(|u| units::Unit {
                pbox: units::to_frame(u.pbox, q, w, h),
                ..u.clone()
            })
            .collect(),
        wh: if q % 2 == 1 { (h, w) } else { (w, h) },
        rules: rules.map(|r| rotate_rules(&r, q, w, h)),
        ocr: input.ocr,
        img: img.map(|im| rotate_image(&im, q)),
        raw: None,
    };
    let mut pm = run_page_framed(net, layout_net, &upright, sig, opt);
    let back = |r: &mut crate::types::Rect| {
        let b = units::from_frame(
            [
                r.x as f64,
                r.y as f64,
                (r.x + r.width) as f64,
                (r.y + r.height) as f64,
            ],
            q,
            w,
            h,
        );
        *r = crate::types::Rect {
            x: b[0] as f32,
            y: b[1] as f32,
            width: (b[2] - b[0]) as f32,
            height: (b[3] - b[1]) as f32,
        };
    };
    for t in &mut pm.tables {
        back(&mut t.bbox);
        for c in &mut t.grid.cells {
            c.bx = units::from_frame(c.bx, q, w, h);
        }
    }
    for c in &mut pm.classes {
        back(&mut c.bbox);
    }
    for b in &mut pm.blocks {
        back(&mut b.bbox);
    }
    pm
}

/// The rotated reading direction (1..3) carrying at least 60% of a text-layer page's
/// characters, if any.
fn main_rotation(input: &PageInput) -> Option<u8> {
    if input.ocr {
        return None;
    }
    let mut by_q = [0usize; 4];
    for u in &input.units {
        by_q[(u.q & 3) as usize] += u.text.chars().filter(|c| !c.is_whitespace()).count();
    }
    let total: usize = by_q.iter().sum();
    (1..4u8).find(|&q| total > 0 && by_q[q as usize] * 10 >= total * 6)
}

/// Ruling lines in reading direction `q`'s frame: each segment's box goes through
/// [`units::to_frame`], and a quarter turn swaps horizontal and vertical.
fn rotate_rules(r: &features::Rules, q: u8, w: f64, h: f64) -> features::Rules {
    let mut out = features::Rules::default();
    let segs =
        r.h.iter()
            .map(|s| [s[1], s[0], s[2], s[0]])
            .chain(r.v.iter().map(|s| [s[0], s[1], s[0], s[2]]));
    for [x0, y0, x1, y1] in segs {
        let b = units::to_frame([x0 as f64, y0 as f64, x1 as f64, y1 as f64], q, w, h);
        let (b0, b1, b2, b3) = (b[0] as f32, b[1] as f32, b[2] as f32, b[3] as f32);
        if (b3 - b1).abs() <= (b2 - b0).abs() {
            out.h.push([(b1 + b3) / 2.0, b0.min(b2), b0.max(b2)]);
        } else {
            out.v.push([(b0 + b2) / 2.0, b1.min(b3), b1.max(b3)]);
        }
    }
    out
}

/// The image-branch page turned into reading direction `q`'s frame (the pixel form of
/// [`units::to_frame`]: q = 1 maps (x, y) to (h - 1 - y, x)).
fn rotate_image(im: &features::PageImage, q: u8) -> features::PageImage {
    let (w, h) = (im.w, im.h);
    let (nw, nh) = if q % 2 == 1 { (h, w) } else { (w, h) };
    let mut px = vec![255u8; nw * nh];
    for y in 0..h {
        for x in 0..w {
            let (nx, ny) = match q {
                1 => (h - 1 - y, x),
                2 => (w - 1 - x, h - 1 - y),
                _ => (y, w - 1 - x),
            };
            px[ny * nw + nx] = im.px[y * w + x];
        }
    }
    features::PageImage {
        w: nw,
        h: nh,
        px,
        k: im.k,
    }
}

/// The page's tables: features, model, decoding.
fn run_page_framed(
    net: &net::TableNet,
    layout_net: Option<&net::TableNet>,
    input: &PageInput,
    sig: Option<&crate::projection::ProjSignals>,
    opt: &decode::DecodeOptions,
) -> PageModel {
    let (units, proj): (std::borrow::Cow<[units::Unit]>, Vec<Option<features::Proj>>) = if input.ocr
    {
        let Some(s) = sig else {
            return PageModel::default();
        };
        let (us, proj) = ocr_units(input, s);
        (us.into(), proj)
    } else {
        let proj = match sig {
            Some(s) => join_signals(input, s),
            None => vec![None; input.units.len()],
        };
        (input.units.as_slice().into(), proj)
    };
    let n = units.len();
    if !(2..=MAX_UNITS).contains(&n)
        || units.iter().map(|u| u.text.chars().count()).sum::<usize>() < 20
    {
        return PageModel::default();
    }
    let f32x4 = |b: [f64; 4]| [b[0] as f32, b[1] as f32, b[2] as f32, b[3] as f32];
    let fu: Vec<features::FUnit> = units
        .iter()
        .zip(proj)
        .map(|(u, p)| features::FUnit {
            bx: f32x4(u.bx),
            pbox: f32x4(u.pbox),
            text: u.text.clone(),
            h: u.h as f32,
            bold: u.bold,
            proj: p,
        })
        .collect();
    let wh = (input.wh.0 as f32, input.wh.1 as f32);
    let deferred = input.raster();
    let rules = input.rules.as_ref().or(deferred.0.as_ref());
    let page_img = input.img.as_ref().or(deferred.1.as_ref());
    let feats = features::unit_features(&fu, wh, rules);
    let pairf = match rules {
        Some(r) => features::pair_rule_features(&fu.iter().map(|u| u.pbox).collect::<Vec<_>>(), r),
        None => vec![0f32; n * n * 2],
    };
    let img = page_img
        .filter(|_| net.has_img() || layout_net.is_some_and(|l| l.has_img()))
        .map(|im| im.input(&fu.iter().map(|u| u.pbox).collect::<Vec<_>>()));
    let mut logits = net.forward(&feats, &pairf, n, img.as_ref());
    let minor: Vec<bool> = units.iter().map(|u| u.minor).collect();
    mask_minor(&mut logits, &minor);
    let du: Vec<decode::DUnit> = units
        .iter()
        .map(|u| decode::DUnit {
            bx: u.bx,
            text: u.text.clone(),
            h: u.h,
        })
        .collect();
    let mut out = Vec::new();
    for g in decode::decode_tables(&du, &logits, opt) {
        let g: Vec<usize> = g.into_iter().filter(|&i| !minor[i]).collect();
        if g.len() < 2 {
            continue;
        }
        let mut grid = decode::decode_grid(&du, &g, &logits, opt);
        // rules are in page points, cell boxes in the reading frame: the same on upright text
        let mut ruled = false;
        if let Some(r) = rules.filter(|_| g.iter().all(|&i| units[i].q == 0)) {
            let words: Vec<&[(f64, f64, String)]> =
                units.iter().map(|u| u.words.as_slice()).collect();
            match grid.lattice(&r.h, &r.v, &du, &words, &g) {
                Some(l) => {
                    grid = l;
                    ruled = true;
                }
                None => grid.merge_unruled_rows(&r.h),
            }
        }
        // one column is a list or a stack of lines, not a table (seen on OCR pages: a
        // signature block's lines grouped into a 1-wide grid); tables of contents, charts and
        // multi-column prose read as grids are no tables either
        // (a grid drawn in ruling lines is no table of contents)
        if grid.n_cols < 2 || (!ruled && grid.is_toc()) || grid.is_chart() || grid.is_prose() {
            continue;
        }
        let mut g = g;
        if g.iter().all(|&i| units[i].q == 0) {
            g.extend(grid.place_rotated(&units, &minor));
        }
        let mut b = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for &i in &g {
            let p = units[i].pbox;
            b = [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[2]),
                b[3].max(p[3]),
            ];
        }
        let bbox = crate::types::Rect {
            x: b[0] as f32,
            y: b[1] as f32,
            width: (b[2] - b[0]) as f32,
            height: (b[3] - b[1]) as f32,
        };
        out.push(ModelTable { bbox, grid });
    }
    // block classes: from the layout model when one is given (same units and features), else
    // from the table model's own class head (a joint model)
    let layout_logits = layout_net
        .filter(|l| l.feat_dim == net.feat_dim)
        .map(|l| l.forward(&feats, &pairf, n, img.as_ref()));
    let cl = layout_logits.as_ref().unwrap_or(&logits);
    let mut classes = Vec::new();
    if cl.n_cls > 0 {
        let c = cl.n_cls;
        for (i, u) in units.iter().enumerate() {
            if u.minor {
                continue;
            }
            let z = &cl.cls[i * c..(i + 1) * c];
            let m = z.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let e: Vec<f32> = z.iter().map(|v| (v - m).exp()).collect();
            let sum: f32 = e.iter().sum();
            let p = u.pbox;
            classes.push(UnitClass {
                bbox: crate::types::Rect {
                    x: p[0] as f32,
                    y: p[1] as f32,
                    width: (p[2] - p[0]) as f32,
                    height: (p[3] - p[1]) as f32,
                },
                chars: u.text.chars().count(),
                probs: e.iter().map(|v| v / sum).collect(),
            });
        }
    }
    let mut blocks = Vec::new();
    if cl.n_pair == 5 && cl.n_cls > 0 && layout_net.is_none() {
        let keep: Vec<usize> = (0..n).filter(|&i| !units[i].minor).collect();
        let c = cl.n_cls;
        for g in block_groups(cl, &keep) {
            let mut sum = vec![0f32; c];
            for &i in &g {
                for (k, v) in cl.cls[i * c..(i + 1) * c].iter().enumerate() {
                    sum[k] += v;
                }
            }
            let class = (0..c)
                .max_by(|&a, &b| sum[a].total_cmp(&sum[b]))
                .unwrap_or(0);
            // Groups split at wide horizontal gaps: running headers and footers are rows of
            // separate items (page number, title, section tabs) that the same_block head joins
            // into one page-wide block, and side-by-side text blocks get joined the same way;
            // also at paragraph breaks and heading / body changes.
            let parts: Vec<Vec<usize>> = split_at_gaps(&g, &units)
                .into_iter()
                .flat_map(|part| split_paragraphs(&part, &units, &cl.cls, c))
                .collect();
            for g in parts {
                // each part takes its own class
                let class = {
                    let mut s = vec![0f32; c];
                    for &i in &g {
                        for (k, v) in cl.cls[i * c..(i + 1) * c].iter().enumerate() {
                            s[k] += v;
                        }
                    }
                    (0..c)
                        .max_by(|&a, &b| s[a].total_cmp(&s[b]))
                        .unwrap_or(class)
                };
                let boxes: Vec<[f64; 4]> = g.iter().map(|&i| units[i].bx).collect();
                let text = decode::reading_order(&boxes)
                    .into_iter()
                    .map(|k| units[g[k]].text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                let mut b = [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ];
                for &i in &g {
                    let p = units[i].pbox;
                    b = [
                        b[0].min(p[0]),
                        b[1].min(p[1]),
                        b[2].max(p[2]),
                        b[3].max(p[3]),
                    ];
                }
                blocks.push(ModelBlock {
                    bbox: crate::types::Rect {
                        x: b[0] as f32,
                        y: b[1] as f32,
                        width: (b[2] - b[0]) as f32,
                        height: (b[3] - b[1]) as f32,
                    },
                    class,
                    text,
                });
            }
        }
    }
    PageModel {
        tables: out,
        classes,
        blocks,
    }
}

impl decode::Grid {
    /// A table of contents: at most three columns, and most rows end in a page number (arabic or
    /// roman) with the arabic ones non-decreasing down the table. Rendered as text, not a table.
    pub fn is_toc(&self) -> bool {
        if self.n_cols > 3 {
            return false;
        }
        let mut rows: Vec<Vec<(usize, &str)>> = vec![Vec::new(); self.n_rows];
        for c in &self.cells {
            let t = c.text.trim();
            if !t.is_empty() {
                rows[c.r0].push((c.c0, t));
            }
        }
        let rows: Vec<_> = rows.into_iter().filter(|r| !r.is_empty()).collect();
        let mut hits = 0;
        let mut arabic = Vec::new();
        for r in &rows {
            let last = r.iter().max_by_key(|(q, _)| *q).unwrap().1;
            let roman = last.len() <= 7 && last.chars().all(|ch| "ivxlcdmIVXLCDM".contains(ch));
            let num = last.len() <= 4 && last.chars().all(|ch| ch.is_ascii_digit());
            if r.len() >= 2 && (roman || num) {
                hits += 1;
                if num {
                    arabic.push(last.parse::<u32>().unwrap_or(0));
                }
            }
        }
        let rising = arabic.windows(2).filter(|w| w[1] < w[0]).count() <= arabic.len() / 10;
        hits >= 3 && hits * 5 >= rows.len() * 3 && rising
    }

    /// Multi-column prose read as a table: long, mostly non-numeric cells whose text runs on
    /// down each column, a cell that doesn't end a sentence followed by one that starts
    /// lowercase ("calendar 2024, more than 70%" / "will be from recycled content"). A real
    /// table's cells are independent, so its columns rarely flow even when one column is long.
    pub fn is_prose(&self) -> bool {
        let words = |t: &str| t.split_whitespace().count();
        let cells: Vec<&decode::GridCell> = self
            .cells
            .iter()
            .filter(|c| !c.text.trim().is_empty())
            .collect();
        if cells.len() < 6 {
            return false;
        }
        let mut wc: Vec<usize> = cells.iter().map(|c| words(&c.text)).collect();
        wc.sort_unstable();
        if wc[wc.len() / 2] < 4 {
            return false;
        }
        let numeric = cells
            .iter()
            .filter(|c| words(&c.text) <= 3 && c.text.chars().any(|ch| ch.is_ascii_digit()))
            .count();
        if numeric * 10 >= cells.len() * 3 {
            return false;
        }
        let mut cols: std::collections::BTreeMap<usize, Vec<&decode::GridCell>> =
            Default::default();
        for c in &cells {
            if c.c0 == c.c1 {
                cols.entry(c.c0).or_default().push(c);
            }
        }
        let (mut pairs, mut flow) = (0usize, 0usize);
        for col in cols.values_mut() {
            col.sort_by_key(|c| c.r0);
            for w in col.windows(2) {
                if w[1].r0 > w[0].r1 + 2 {
                    continue; // not adjacent: more than one empty row between them
                }
                pairs += 1;
                let up = w[0].text.trim_end();
                let open = !up.ends_with(['.', '!', '?', ':', ';']);
                let lower = w[1]
                    .text
                    .trim_start()
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_lowercase());
                flow += (open && lower) as usize;
            }
        }
        pairs >= 3 && flow * 10 >= pairs * 4
    }

    /// A chart read as a table: a sparse, mostly numeric grid with an axis - some column holding
    /// at least three lone numbers in an evenly stepped, monotonic run (the tick labels).
    pub fn is_chart(&self) -> bool {
        let slots = self.n_rows * self.n_cols;
        if slots == 0 {
            return false;
        }
        let parse = |t: &str| -> Option<f64> {
            let t = t.trim().trim_end_matches('%').replace(',', "");
            if t.is_empty()
                || !t
                    .chars()
                    .all(|ch| ch.is_ascii_digit() || ch == '.' || ch == '-' || ch == '+')
            {
                return None;
            }
            t.parse::<f64>().ok()
        };
        let (mut filled, mut nonempty, mut numeric) = (0, 0, 0);
        let mut cols: std::collections::BTreeMap<usize, Vec<(usize, f64)>> = Default::default();
        let mut flow: Vec<((usize, usize), f64)> = Vec::new();
        for c in &self.cells {
            if c.text.trim().is_empty() {
                continue;
            }
            nonempty += 1;
            filled += (c.r1 - c.r0 + 1) * (c.c1 - c.c0 + 1);
            if let Some(v) = parse(&c.text) {
                numeric += 1;
                if c.c0 == c.c1 {
                    cols.entry(c.c0).or_default().push((c.r0, v));
                }
                flow.push(((c.r0, c.c0), v));
            }
        }
        if filled * 2 >= slots || numeric * 2 < nonempty {
            return false;
        }
        let stepped = |vs: &[f64]| {
            vs.len() >= 3 && {
                let d: Vec<f64> = vs.windows(2).map(|w| w[1] - w[0]).collect();
                d[0] != 0.0
                    && d.iter()
                        .all(|&x| (x - d[0]).abs() <= 1e-6 * d[0].abs().max(1.0))
            }
        };
        // a column of ticks (y axis), or every number on the grid in reading order (an x axis of
        // years strewn diagonally, one per row)
        flow.sort_by_key(|&(rc, _)| rc);
        let all: Vec<f64> = flow.iter().map(|&(_, v)| v).collect();
        stepped(&all)
            || cols.values_mut().any(|vs| {
                vs.sort_by_key(|&(r, _)| r);
                stepped(&vs.iter().map(|&(_, v)| v).collect::<Vec<_>>())
            })
    }

    /// As LiteParse's `MergedTable` rows: covered slots omitted, empty slots as empty cells; the
    /// header is the run of header rows at the top (MergedTable heads only leading rows).
    pub fn to_span_rows(&self) -> (Vec<Vec<crate::markdown_layout::SpanCell>>, usize) {
        use crate::markdown_layout::SpanCell;
        let mut at = std::collections::HashMap::new();
        let mut taken = std::collections::HashSet::new();
        for (k, c) in self.cells.iter().enumerate() {
            at.insert((c.r0, c.c0), k);
            for r in c.r0..=c.r1 {
                for q in c.c0..=c.c1 {
                    taken.insert((r, q));
                }
            }
        }
        let mut rows = Vec::with_capacity(self.n_rows);
        for r in 0..self.n_rows {
            let mut row = Vec::new();
            let mut q = 0;
            while q < self.n_cols {
                if let Some(&k) = at.get(&(r, q)) {
                    let c = &self.cells[k];
                    let b = c.bx;
                    row.push(SpanCell {
                        text: c.text.clone(),
                        colspan: (c.c1 - q + 1) as u16,
                        rowspan: (c.r1 - r + 1) as u16,
                        bbox: Some(crate::types::Rect {
                            x: b[0] as f32,
                            y: b[1] as f32,
                            width: (b[2] - b[0]) as f32,
                            height: (b[3] - b[1]) as f32,
                        }),
                        text_item_indices: Vec::new(),
                    });
                    q = c.c1 + 1;
                } else if taken.contains(&(r, q)) {
                    q += 1;
                } else {
                    row.push(SpanCell {
                        text: String::new(),
                        colspan: 1,
                        rowspan: 1,
                        bbox: None,
                        text_item_indices: Vec::new(),
                    });
                    q += 1;
                }
            }
            rows.push(row);
        }
        let header_rows = (0..self.n_rows)
            .take_while(|r| self.head_rows.contains(r))
            .count();
        (rows, header_rows)
    }
}

/// The model from its weights (`tablenet.bin`), for callers without a model path (WASM). Each
/// call parses and keeps one model for the life of the process; load it once and reuse it.
pub fn load_bytes(bytes: &[u8]) -> Result<&'static net::TableNet, String> {
    Ok(Box::leak(Box::new(net::TableNet::from_bytes(bytes)?)))
}

/// The model, loaded once from its path on first use.
pub fn load(path: &std::path::Path) -> Option<&'static net::TableNet> {
    static NET: std::sync::OnceLock<Option<net::TableNet>> = std::sync::OnceLock::new();
    NET.get_or_init(|| {
        // a package that ships LayoutNet as ONNX only (see models::installed_table_model); an
        // older package's ONNX export has no header, but the `.bin` beside it does
        let mut path = path.to_path_buf();
        if path.extension().is_some_and(|e| e == "onnx") {
            #[cfg(feature = "ocria-nano")]
            if let Some(net) = net::TableNet::from_onnx(&path, path.parent()?) {
                return Some(net);
            }
            path.set_extension("bin");
        }
        let bytes = std::fs::read(&path).ok()?;
        #[allow(unused_mut)]
        let mut net = net::TableNet::from_bytes(&bytes).ok()?;
        // The ONNX export of the same model, shipped next to it in the model package, runs through
        // ONNX Runtime: 1.4-2x faster than the Rust kernels on x86 Linux. Not on macOS, where the
        // Rust path's matrix products go through Accelerate and beat ONNX Runtime (~1.6x).
        #[cfg(all(feature = "ocria-nano", not(target_os = "macos")))]
        if let Some(dir) = path.parent() {
            net.attach_onnx(&path.with_extension("onnx"), dir);
        }
        Some(net)
    })
    .as_ref()
}

#[cfg(test)]
mod faint_mask_tests {
    use super::*;

    #[test]
    fn masked_token_keeps_its_length_and_reads_as_a_word() {
        assert_eq!(faint_mask("1"), "x");
        assert_eq!(faint_mask("€1"), "xx");
    }
}
