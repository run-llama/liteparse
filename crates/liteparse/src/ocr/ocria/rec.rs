//! Line recognizers (CTC, decoded in-graph) and the script router (`lpocr_rec.Recognizer`,
//! `lpocr_rec.LineRouter`).

use super::OcriaError;
use super::img::{Img, resize_area, resize_linear};
use super::pool::SessionPool;

/// Recognizer input height.
pub const H: usize = 32;
const STRIDE: usize = 4;
const MAX_W: usize = 1280;
const BUCKET_W: usize = 128;
const BATCH: usize = 32;

/// Grey line image at height `H`, width scaled with the aspect ratio (`Recognizer._prep`).
fn prep(crop: &Img, min_w: usize, max_w: usize) -> Img {
    let g = crop.to_gray();
    let nw = ((g.w as f64 * H as f64 / g.h.max(1) as f64).round_ties_even() as usize)
        .clamp(min_w, max_w);
    if g.h > H {
        resize_area(&g, nw, H)
    } else {
        resize_linear(&g, nw, H)
    }
}

pub struct Recognizer {
    pub(super) pool: SessionPool,
    chars: Vec<String>,
    /// Exported in visual order: RTL readings are reordered to logical order.
    visual_rtl: bool,
    /// Input pixels per output timestep (4 or 8).
    stride: usize,
}

impl Recognizer {
    pub fn new(
        pool: SessionPool,
        chars: Vec<String>,
        visual_rtl: bool,
    ) -> Result<Self, OcriaError> {
        // timestep stride from one probe run: input width / output steps
        let (shape, _) = pool
            .infer([1, 1, H, 64], vec![1f32; H * 64], &["ids"])?
            .i64("ids")?;
        let steps = shape[1];
        Ok(Recognizer {
            pool,
            chars,
            visual_rtl,
            stride: 64 / steps.max(1),
        })
    }

    /// (text, confidence) per crop. Lines are batched by width (sorted, 32 per batch, padded
    /// to a multiple of 128 px) exactly as the reference does: a line's output depends on the
    /// padded width it was run at.
    pub fn read(&self, crops: &[&Img]) -> Result<Vec<(String, f32)>, OcriaError> {
        let lines: Vec<Img> = crops.iter().map(|c| prep(c, STRIDE * 2, MAX_W)).collect();
        let mut order: Vec<usize> = (0..lines.len()).collect();
        order.sort_by_key(|&i| lines[i].w);
        let mut out = vec![(String::new(), 0f32); lines.len()];
        for idx in order.chunks(BATCH) {
            let maxw = idx.iter().map(|&i| lines[i].w).max().unwrap_or(1);
            let w = maxw.div_ceil(BUCKET_W) * BUCKET_W;
            let mut x = vec![1f32; idx.len() * H * w];
            for (j, &i) in idx.iter().enumerate() {
                let l = &lines[i];
                for y in 0..H {
                    for xx in 0..l.w {
                        x[(j * H + y) * w + xx] =
                            (l.data[y * l.w + xx] as f64 / 127.5 - 1.0) as f32;
                    }
                }
            }
            let mut res = self.pool.infer([idx.len(), 1, H, w], x, &["ids", "conf"])?;
            let (shape, ids) = res.i64("ids")?;
            let (_, conf) = res.f32("conf")?;
            let t_all = shape[1];
            for (j, &i) in idx.iter().enumerate() {
                let t = (lines[i].w / self.stride).min(t_all);
                let (mut text, mut cs, mut prev) = (String::new(), Vec::new(), 0i64);
                for k in 0..t {
                    let c = ids[j * t_all + k];
                    if c != 0 && c != prev {
                        if let Some(s) = self.chars.get((c - 1) as usize) {
                            text.push_str(s);
                        }
                        cs.push(conf[j * t_all + k]);
                    }
                    prev = c;
                }
                if self.visual_rtl && text.chars().any(is_rtl_char) {
                    text = visual_to_logical(&text);
                }
                let mean = if cs.is_empty() {
                    0.0
                } else {
                    (cs.iter().map(|&v| v as f64).sum::<f64>() / cs.len() as f64) as f32
                };
                out[i] = (text, mean);
            }
        }
        Ok(out)
    }
}

fn is_rtl_char(ch: char) -> bool {
    ('\u{0590}'..='\u{08ff}').contains(&ch) || ('\u{fb1d}'..='\u{feff}').contains(&ch)
}

/// python-bidi `get_display(s, base_dir="R")` (itself unicode-bidi): reorder a visual-order
/// reading into logical order with an RTL paragraph level.
fn visual_to_logical(s: &str) -> String {
    use unicode_bidi::{BidiInfo, Level};
    let info = BidiInfo::new(s, Some(Level::rtl()));
    let mut out = String::with_capacity(s.len());
    for para in &info.paragraphs {
        out.push_str(&info.reorder_line(para, para.range.clone()));
    }
    out
}

pub const GROUPS: [&str; 4] = ["latin", "cjk", "rtl", "world"];
const ROUTER_W: usize = 512;
const ROUTER_BATCH: usize = 64;

pub struct Router {
    pub(super) pool: SessionPool,
}

impl Router {
    pub fn new(pool: SessionPool) -> Self {
        Router { pool }
    }

    fn prep(crop: &Img) -> Vec<f32> {
        let g = prep(crop, 8, usize::MAX);
        // tile short lines across the router width, then cut to it
        let mut x = vec![0f32; H * ROUTER_W];
        for y in 0..H {
            for xx in 0..ROUTER_W {
                let v = g.data[y * g.w + xx % g.w];
                x[y * ROUTER_W + xx] = v as f32 / 127.5 - 1.0;
            }
        }
        x
    }

    fn softmax(z: &[f32]) -> Vec<f32> {
        let m = z.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let e: Vec<f32> = z.iter().map(|v| (v - m).exp()).collect();
        let s: f32 = e.iter().sum();
        e.iter().map(|v| v / s).collect()
    }

    /// Group probabilities per crop and P(upside down).
    fn probs(&self, crops: &[Img]) -> Result<(Vec<[f32; 4]>, Vec<f32>), OcriaError> {
        let (mut gs, mut fs) = (Vec::new(), Vec::new());
        for part in crops.chunks(ROUTER_BATCH) {
            let mut x = Vec::with_capacity(part.len() * H * ROUTER_W);
            for c in part {
                x.extend(Self::prep(c));
            }
            let mut res = self
                .pool
                .infer([part.len(), 1, H, ROUTER_W], x, &["group", "flip"])?;
            let (_, g) = res.f32("group")?;
            let (_, f) = res.f32("flip")?;
            for j in 0..part.len() {
                let p = Self::softmax(&g[j * 4..j * 4 + 4]);
                gs.push([p[0], p[1], p[2], p[3]]);
                fs.push(Self::softmax(&f[j * 2..j * 2 + 2])[1]);
            }
        }
        Ok((gs, fs))
    }

    /// Page-level routing (`LineRouter.route`): (group index per crop, turn the page?, each
    /// group's share of the line width). Every line takes the page's dominant group unless it is
    /// long enough to judge and confidently another; Latin unless another group holds a clear
    /// majority of the line width.
    pub fn route(&self, crops: &[Img]) -> Result<(Vec<usize>, bool, [f64; 4]), OcriaError> {
        const OVERRIDE: f32 = 0.9;
        const MIN_ASPECT: f64 = 4.0;
        const FLIP_MIN_LINES: usize = 3;
        const FLIP_VOTE: f64 = 0.7;
        const OTHER_SHARE: f64 = 0.65;
        if crops.is_empty() {
            return Ok((Vec::new(), false, [0.0; 4]));
        }
        let (gp, fp) = self.probs(crops)?;
        let w: Vec<f64> = crops.iter().map(|c| c.w as f64).collect();
        let wsum: f64 = w.iter().sum();
        let mut share = [0f64; 4];
        for (p, &wi) in gp.iter().zip(&w) {
            for k in 0..4 {
                share[k] += p[k] as f64 * wi;
            }
        }
        for s in share.iter_mut() {
            *s /= wsum;
        }
        let mut dom = argmax(&share);
        if dom != 0 && share[dom] <= OTHER_SHARE {
            dom = 0;
        }
        let groups = gp
            .iter()
            .zip(crops)
            .map(|(p, c)| {
                let own = argmax(&p.map(|v| v as f64));
                let pmax = p.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                if own != dom && pmax > OVERRIDE && c.w as f64 >= MIN_ASPECT * c.h as f64 {
                    own
                } else {
                    dom
                }
            })
            .collect();
        let vote: f64 = fp.iter().zip(&w).map(|(&f, &wi)| f as f64 * wi).sum();
        let flip = crops.len() >= FLIP_MIN_LINES && vote > FLIP_VOTE * wsum;
        Ok((groups, flip, share))
    }
}

/// np.argmax: the first maximum.
pub fn argmax(v: &[f64]) -> usize {
    let mut best = 0;
    for i in 1..v.len() {
        if v[i] > v[best] {
            best = i;
        }
    }
    best
}
