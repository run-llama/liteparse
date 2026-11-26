//! ocria-nano: LiteParse's own OCR models run in-process on ONNX Runtime.
//!
//! A port of the evaluation server (`ocr/lpocr/server.py` + `lpocr_rec.py`) with the settings
//! the benchmarks use: line detector (long side 1600, threshold 0.5, collinear merge 0.6),
//! column-gutter split, sideways-page retry, a script router feeding four recognizers (Latin,
//! CJK, RTL, world) with the low-confidence script rescue, homoglyph folding and the junk
//! filter. Results come back in the same shape the HTTP engine receives.
//!
//! ONNX Runtime is loaded at run time (`ort`'s `load-dynamic`): nothing links against it, and
//! the library is found at the path given to [`OcriaEngine::load`] or `ORT_DYLIB_PATH`.

mod det;
mod img;
pub mod package;
mod page;
pub(crate) mod pool;
mod rec;
#[cfg(feature = "ocr-serve")]
pub mod serve;

#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::pin::Pin;
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::OnceLock;

use super::{OcrEngine, OcrOptions, OcrResult};
use crate::error::LiteParseError;
pub use det::Adaptive;
use det::Detector;
use img::Img;
use pool::SessionPool;
use rec::{GROUPS, Recognizer, Router};

/// A line quad: [top-left, top-right, bottom-right, bottom-left] along the text.
pub(crate) type Quad = [[f32; 2]; 4];

#[derive(Debug, thiserror::Error)]
pub enum OcriaError {
    #[cfg(not(target_arch = "wasm32"))]
    #[error("ONNX Runtime: {0}")]
    Ort(#[from] ort::Error),
    #[error("ocria-nano model files: {0}")]
    Model(String),
}

impl From<OcriaError> for LiteParseError {
    fn from(e: OcriaError) -> Self {
        LiteParseError::Ocr(e.to_string())
    }
}

/// File names inside an ocria-nano model directory.
pub const DETECTOR_FILE: &str = "det.onnx";
pub const ROUTER_FILE: &str = "router.onnx";
/// The detector's recommended thresholds, `{"thresh": .., "merge": ..}` (optional).
pub const DETECTOR_META_FILE: &str = "det.meta.json";
/// `(text, confidence)` per box, whether the page was read upside down, the page's script group.
type PageReads = (Vec<rec::Read>, bool, usize);

/// A faint candidate ([`det::Faint`]) is kept only at this confidence. Rule fragments and
/// specks read as "|", "l" or a letter pair at 0.5-0.7; the lone "1"s of issue #615 read at 0.97+.
const FAINT_CONF: f32 = 0.9;

/// Detector thresholds for a model without `det.meta.json`.
const DET_THRESH: f32 = 0.5;
const DET_MERGE: f32 = 0.6;

/// Pipeline switches and thresholds. The defaults are the benchmarked configuration.
#[derive(Clone, Debug)]
pub struct OcriaOptions {
    /// ONNX Runtime intra-op threads, shared by every model session (0 = one per core). A
    /// lone page uses them all; pages OCR'd concurrently share them.
    pub threads: usize,
    pub det_long_side: usize,
    /// Detector probability threshold. `None`: the model's recommendation (`det.meta.json`
    /// `"thresh"`), else `DET_THRESH`. Calibration differs between detector versions.
    pub det_thresh: Option<f32>,
    /// Collinear box merge threshold. `None`: `det.meta.json` `"merge"`, else `DET_MERGE`.
    pub det_merge: Option<f32>,
    /// Second detection pass for small print (None = one pass at `det_long_side`).
    pub det_adaptive: Option<det::Adaptive>,
    /// Cut boxes that run across a column gutter.
    pub gutter: bool,
    /// Retry pages that look sideways a quarter turn round.
    pub orient: bool,
    /// Re-read low-confidence "Latin" pages with the other recognizers.
    pub rescue: bool,
    /// Drop readings with no letter or digit, or 1-2 unsure characters.
    pub junk: bool,
    /// Read faint short tokens next to detected lines, kept at `FAINT_CONF` (None = off).
    pub faint: Option<det::Faint>,
}

impl Default for OcriaOptions {
    fn default() -> Self {
        OcriaOptions {
            threads: 0,
            det_long_side: 1600,
            det_thresh: None,
            det_merge: None,
            det_adaptive: None,
            gutter: true,
            orient: true,
            rescue: true,
            junk: true,
            faint: Some(det::Faint {
                thresh: 0.1,
                max: 24,
            }),
        }
    }
}

pub struct OcriaEngine {
    det: Detector,
    router: Router,
    /// One recognizer per script group (GROUPS order); every group has one.
    recs: [Arc<Recognizer>; 4],
    opts: OcriaOptions,
    pages: std::sync::atomic::AtomicU64,
    /// Wall time inside `run`, all pages summed (µs).
    run_us: std::sync::atomic::AtomicU64,
}

#[cfg(not(target_arch = "wasm32"))]
fn cores() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// The process's ONNX Runtime environment (sessions are built from it), once `init_runtime` has
/// loaded the library. ort keeps only a weak reference, so this static keeps it alive.
#[cfg(not(target_arch = "wasm32"))]
static ENV: OnceLock<ort::environment::Environment> = OnceLock::new();

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn environment() -> Option<&'static ort::environment::Environment> {
    ENV.get()
}

/// Load ONNX Runtime once per process from `path` (`None`: `ORT_DYLIB_PATH`), with one
/// intra-op thread pool that every session shares. The first call's `threads` wins.
#[cfg(not(target_arch = "wasm32"))]
fn init_runtime(path: Option<&Path>, threads: usize) -> Result<(), OcriaError> {
    static INIT: OnceLock<Result<(), String>> = OnceLock::new();
    INIT.get_or_init(|| {
        let threads = if threads == 0 { cores() } else { threads };
        // Microsoft's macOS/Linux ORT builds (1.29+) start a telemetry client (1DS) when the
        // library initialises, before any API call could turn it off. It uploads usage events
        // over HTTP, and its upload thread races static teardown at exit: ~15% of runs aborted
        // with `recursive_mutex lock failed`. The library reads this variable at that point.
        if std::env::var_os("ORT_DISABLE_TELEMETRY").is_none() {
            // SAFETY: set once, before the runtime loads; nothing in liteparse reads it, and the
            // only reader is the library about to be loaded below
            unsafe { std::env::set_var("ORT_DISABLE_TELEMETRY", "1") };
        }
        // load the library first: the thread-pool options already call into it
        let builder = match path {
            Some(p) => ort::init_from(p).map_err(|e| e.to_string())?,
            None => ort::init(),
        };
        let pool = ort::environment::GlobalThreadPoolOptions::default()
            .with_intra_threads(threads)
            .and_then(|o| o.with_spin_control(false))
            .map_err(|e| e.to_string())?;
        // ort enables ORT's event telemetry by default (ETW on Windows); keep it off everywhere
        let env = builder
            .with_name("liteparse-ocria")
            .with_telemetry(false)
            .with_global_thread_pool(pool)
            .build()
            .map_err(|e| e.to_string())?;
        let _ = ENV.set(env);
        Ok(())
    })
    .clone()
    .map_err(OcriaError::Model)
}

/// Sessions per model: the wide one, plus up to one two-thread session per two cores for pages
/// OCR'd concurrently.
#[cfg(not(target_arch = "wasm32"))]
fn pool(path: PathBuf) -> Result<SessionPool, OcriaError> {
    SessionPool::new(path, (cores() / 2).max(1))
}

#[cfg(not(target_arch = "wasm32"))]
fn recognizer(dir: &Path, group: &str) -> Result<Recognizer, OcriaError> {
    let model = dir.join(format!("rec_{group}.onnx"));
    let charset = dir.join(format!("rec_{group}.charset.json"));
    let chars: Vec<String> = serde_json::from_slice(
        &std::fs::read(&charset)
            .map_err(|e| OcriaError::Model(format!("{}: {e}", charset.display())))?,
    )
    .map_err(|e| OcriaError::Model(format!("{}: {e}", charset.display())))?;
    let meta = dir.join(format!("rec_{group}.meta.json"));
    let visual_rtl = std::fs::read(&meta)
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.get("visual_rtl").and_then(|x| x.as_bool()))
        .unwrap_or(false);
    Recognizer::new(pool(model)?, chars, visual_rtl)
}

impl OcriaEngine {
    /// Load the models in `dir` (det.onnx, router.onnx, rec_<group>.onnx +
    /// rec_<group>.charset.json [+ rec_<group>.meta.json] for latin / cjk / rtl / world).
    /// `runtime` is the ONNX Runtime shared library; `None` uses `ORT_DYLIB_PATH`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load(
        dir: &Path,
        runtime: Option<&Path>,
        opts: OcriaOptions,
    ) -> Result<Self, OcriaError> {
        init_runtime(runtime, opts.threads)?;
        // thresholds the detector was tuned with ship next to it (det.meta.json)
        let meta = std::fs::read(dir.join(DETECTOR_META_FILE)).ok();
        let recs = [
            Arc::new(recognizer(dir, GROUPS[0])?),
            Arc::new(recognizer(dir, GROUPS[1])?),
            Arc::new(recognizer(dir, GROUPS[2])?),
            Arc::new(recognizer(dir, GROUPS[3])?),
        ];
        Ok(Self::assemble(
            pool(dir.join(DETECTOR_FILE))?,
            meta.as_deref(),
            pool(dir.join(ROUTER_FILE))?,
            recs,
            opts,
        ))
    }

    /// The engine from model bytes (WebAssembly, where there is no model directory): the
    /// files of an ocria-nano package, as [`OcriaModels`] names them. Recognizers left out
    /// fall back to the Latin one, so a Latin-only page set needs just three model files.
    #[cfg(target_arch = "wasm32")]
    pub fn from_models(m: &OcriaModels, opts: OcriaOptions) -> Result<Self, OcriaError> {
        let rec = |r: &RecModel| -> Result<Recognizer, OcriaError> {
            let chars: Vec<String> = serde_json::from_slice(&r.charset)
                .map_err(|e| OcriaError::Model(format!("recognizer charset: {e}")))?;
            let visual_rtl = r
                .meta
                .as_deref()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(b).ok())
                .and_then(|v| v.get("visual_rtl").and_then(|x| x.as_bool()))
                .unwrap_or(false);
            Recognizer::new(SessionPool::from_bytes(&r.onnx)?, chars, visual_rtl)
        };
        let latin = Arc::new(rec(&m.latin)?);
        let other = |r: &Option<RecModel>| -> Result<Arc<Recognizer>, OcriaError> {
            Ok(match r {
                Some(r) => Arc::new(rec(r)?),
                None => latin.clone(),
            })
        };
        let recs = [
            latin.clone(),
            other(&m.cjk)?,
            other(&m.rtl)?,
            other(&m.world)?,
        ];
        Ok(Self::assemble(
            SessionPool::from_bytes(&m.det)?,
            m.det_meta.as_deref(),
            SessionPool::from_bytes(&m.router)?,
            recs,
            opts,
        ))
    }

    fn assemble(
        det_pool: SessionPool,
        det_meta: Option<&[u8]>,
        router_pool: SessionPool,
        recs: [Arc<Recognizer>; 4],
        opts: OcriaOptions,
    ) -> Self {
        let meta: serde_json::Value = det_meta
            .and_then(|b| serde_json::from_slice(b).ok())
            .unwrap_or_default();
        let from_meta = |k: &str| meta.get(k).and_then(|v| v.as_f64()).map(|v| v as f32);
        let mut det = Detector::new(
            det_pool,
            opts.det_long_side,
            opts.det_thresh
                .or(from_meta("thresh"))
                .unwrap_or(DET_THRESH),
            opts.det_merge.or(from_meta("merge")).unwrap_or(DET_MERGE),
        );
        det.adaptive = opts.det_adaptive;
        det.faint = opts.faint;
        OcriaEngine {
            det,
            router: Router::new(router_pool),
            recs,
            opts,
            pages: Default::default(),
            run_us: Default::default(),
        }
    }

    /// Cumulative timings: pages, wall time in `run`, and per model the calls, time waiting for
    /// a session and time inside the calls (JSON).
    pub fn stats(&self) -> serde_json::Value {
        use std::sync::atomic::Ordering::Relaxed;
        let model = |p: &pool::SessionPool| {
            let s = p.stats();
            serde_json::json!({
                "calls": s.calls.load(Relaxed),
                "wait_s": s.wait_us.load(Relaxed) as f64 / 1e6,
                "run_s": s.run_us.load(Relaxed) as f64 / 1e6,
            })
        };
        let mut models = serde_json::Map::new();
        models.insert("det".into(), model(&self.det.pool));
        models.insert("router".into(), model(&self.router.pool));
        for (g, r) in GROUPS.iter().zip(&self.recs) {
            models.insert(format!("rec_{g}"), model(&r.pool));
        }
        serde_json::json!({
            "pages": self.pages.load(Relaxed),
            "run_s": self.run_us.load(Relaxed) as f64 / 1e6,
            "models": models,
        })
    }

    /// OCR one page image (RGB or grey, row-major).
    pub fn run(&self, data: &[u8], width: u32, height: u32) -> Result<Vec<OcrResult>, OcriaError> {
        let t0 = web_time::Instant::now();
        let out = self.run_page(data, width, height);
        self.pages
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.run_us.fetch_add(
            t0.elapsed().as_micros() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        out
    }

    fn run_page(&self, data: &[u8], width: u32, height: u32) -> Result<Vec<OcrResult>, OcriaError> {
        let (w, h) = (width as usize, height as usize);
        let ch = if data.len() == w * h { 1 } else { 3 };
        let mut img0 = Img {
            w,
            h,
            ch,
            data: data.to_vec(),
        };
        if ch == 1 {
            // the server decodes every request as RGB
            img0 = Img {
                w,
                h,
                ch: 3,
                data: img0.data.iter().flat_map(|&v| [v, v, v]).collect(),
            };
        }
        let (mut boxes, mut faint) = self.det.detect_with_faint(&img0)?;
        let mut sideways = false;
        let mut flip_hint: Option<bool> = None;
        let turned;
        let mut img = &img0;
        let tall = page::mostly_tall(&boxes);
        if self.opts.orient && (page::looks_sideways(&boxes) || tall) {
            // maybe a page scanned on its side: detect again a quarter turn counter-clockwise and
            // keep that when it finds far more lines (the router's flip vote finishes 270 deg)
            turned = img0.rot90(1);
            let tboxes = self.det.detect(&turned)?;
            // a numeric table also has few long boxes, and turned, each stacked column of numbers
            // reads as one long line: the turn must also read better, not only find longer boxes
            // a page of upright boxes needs no line count: the turn must read clearly better
            let longer = page::long_lines(&tboxes) > 2 * page::long_lines(&boxes) + 5;
            let margin = if longer { 0.05 } else { 0.1 };
            if longer || tall {
                let (conf_t, flip_t) = self.sample_conf(&turned, &tboxes)?;
                if conf_t > self.sample_conf(&img0, &boxes)?.0 + margin {
                    img = &turned;
                    boxes = tboxes;
                    faint.clear();
                    sideways = true;
                    // the sample's widest lines settle upside down (the whole page's vote can
                    // miss on a page of short digit boxes, reading "1985" as "S86I")
                    flip_hint = Some(flip_t);
                }
            }
        }
        if self.opts.gutter {
            boxes = page::split_gutters(img, &boxes);
        }
        let (mut reads, flip, dom) = if boxes.is_empty() {
            (Vec::new(), false, 0)
        } else {
            self.read(img, &boxes, flip_hint)?
        };
        let n_lines = boxes.len();
        if !faint.is_empty() && !flip {
            // with the page's recognizer: a lone glyph is too short for the router
            let crops: Vec<Img> = faint.iter().map(|b| page::crop(img, b)).collect();
            let refs: Vec<&Img> = crops.iter().collect();
            let res = self.rec_for(dom).read(&refs)?;
            for (quad, (text, conf, _)) in faint.into_iter().zip(res) {
                let text = page::clean_latin(&text);
                if conf >= FAINT_CONF && page::is_short_token(&text) {
                    boxes.push(quad);
                    reads.push((text, conf, Vec::new()));
                }
            }
        }
        let mut out = Vec::new();
        for (k, (quad, (text, conf, spans))) in boxes.iter().zip(reads).enumerate() {
            if text.trim().is_empty() || (self.opts.junk && page::is_junk(&text, conf)) {
                continue;
            }
            let pts = if self.opts.orient {
                page::reading_frame(quad, flip, sideways, img0.w)
            } else {
                *quad
            };
            let (mut x1, mut y1, mut x2, mut y2) = (
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            );
            for p in &pts {
                x1 = x1.min(p[0]);
                y1 = y1.min(p[1]);
                x2 = x2.max(p[0]);
                y2 = y2.max(p[1]);
            }
            // upright lines only: merging places a rotated result's word boxes in another frame
            let upright = !flip
                && !sideways
                && pts[1][0] > pts[0][0]
                && (pts[1][1] - pts[0][1]).abs() < 0.2 * (pts[1][0] - pts[0][0]);
            let words = if upright {
                word_boxes(&text, &spans, &pts)
            } else {
                Vec::new()
            };
            out.push(OcrResult {
                text,
                // the server sends int() of each extent
                bbox: [x1.trunc(), y1.trunc(), x2.trunc(), y2.trunc()],
                confidence: conf,
                polygon: Some(pts),
                words,
                rotation: None,
                faint: k >= n_lines,
            });
        }
        Ok(out)
    }

    fn rec_for(&self, group: usize) -> &Recognizer {
        &self.recs[group]
    }

    /// Mean reading confidence over the widest boxes (up to 24), weighted by text length: how
    /// well the page reads in this frame.
    fn sample_conf(&self, img: &Img, boxes: &[Quad]) -> Result<(f32, bool), OcriaError> {
        let mut by_w: Vec<&Quad> = boxes.iter().collect();
        let width = |q: &Quad| {
            let xs = q.iter().map(|p| p[0]);
            xs.clone().fold(f32::NEG_INFINITY, f32::max) - xs.fold(f32::INFINITY, f32::min)
        };
        by_w.sort_by(|a, b| width(b).total_cmp(&width(a)));
        let sample: Vec<Quad> = by_w.into_iter().take(24).copied().collect();
        if sample.is_empty() {
            return Ok((0.0, false));
        }
        let (reads, flip, _) = self.read(img, &sample, None)?;
        let (mut num, mut den) = (0f32, 0f32);
        for (t, c, _) in &reads {
            let n = t.trim().chars().count().max(1) as f32;
            num += c * n;
            den += n;
        }
        Ok((if den > 0.0 { num / den } else { 0.0 }, flip))
    }

    /// Readings per box, whether the page was read upside down (`server.read`) and the page's
    /// script group.
    fn read(
        &self,
        img: &Img,
        boxes: &[Quad],
        flip_hint: Option<bool>,
    ) -> Result<PageReads, OcriaError> {
        let mut crops: Vec<Img> = boxes.iter().map(|b| page::crop(img, b)).collect();
        let (mut groups, flip, share) = self.router.route(&crops)?;
        let flip = flip_hint.unwrap_or(flip);
        if flip {
            crops = crops.iter().map(|c| c.rot90(2)).collect();
        }
        let n = crops.len();
        let mut out: Vec<rec::Read> = vec![(String::new(), 0f32, Vec::new()); n];
        let subset = |idx: &[usize]| -> Vec<&Img> { idx.iter().map(|&i| &crops[i]).collect() };
        for gi in 0..GROUPS.len() {
            let idx: Vec<usize> = (0..n).filter(|&i| groups[i] == gi).collect();
            if idx.is_empty() {
                continue;
            }
            for (&i, res) in idx.iter().zip(self.rec_for(gi).read(&subset(&idx))?) {
                out[i] = res;
            }
        }
        let mut dom = page::majority(&groups);
        // a routed reading this confident is a real reading, not the 0.5-0.7 junk of the wrong recognizer
        const KEEP_CONF: f32 = 0.75;
        if self.opts.rescue && dom == 0 {
            // a whole page of another script sent to the Latin recognizer reads as confident-
            // looking junk at 0.5-0.7; the right recognizer then scores 0.83-0.90
            const RESCUE_CONF: f64 = 0.75;
            const RESCUE_MARGIN: f64 = 0.15;
            // Only scripts the router saw a fair share of are tried. A page that needs the rescue
            // still carries its script in the router's shares (0.49 on the bilingual XFUND form
            // it fixes), while low-confidence Latin regions (chart labels, logos) carry almost
            // none: trying every script there switched them to other-script junk and made the
            // slowest pages ~13% slower.
            const RESCUE_SHARE: f64 = 0.2;
            let lat: Vec<usize> = (0..n).filter(|&i| groups[i] == dom).collect();
            if !lat.is_empty() && share[1..].iter().any(|&s| s >= RESCUE_SHARE) {
                let lat_conf = mean(lat.iter().map(|&i| out[i].1));
                if lat_conf < RESCUE_CONF {
                    let sample: Vec<usize> = lat.iter().copied().take(40).collect();
                    let (mut best, mut best_conf) = (None, lat_conf + RESCUE_MARGIN);
                    for gi in (1..GROUPS.len()).filter(|&gi| share[gi] >= RESCUE_SHARE) {
                        let c = mean(
                            self.rec_for(gi)
                                .read(&subset(&sample))?
                                .into_iter()
                                .map(|r| r.1),
                        );
                        if c >= best_conf {
                            best = Some(gi);
                            best_conf = c;
                        }
                    }
                    if let Some(gi) = best {
                        for (&i, res) in lat.iter().zip(self.rec_for(gi).read(&subset(&lat))?) {
                            // per line: a bilingual page keeps its confident Latin lines
                            if !(out[i].1 >= KEEP_CONF && out[i].1 >= res.1) {
                                out[i] = res;
                                groups[i] = gi;
                            }
                        }
                        dom = gi;
                    }
                }
            }
            // Per-line rescue when the page stays Latin: a map or form mixing English labels and noise boxes with
            // another script keeps the sample's mean below the page bar, though each line of that script reads at
            // 0.85-0.95 with its own recognizer. Every Latin-routed line on a page carrying the script's share is
            // re-read, not only weak ones: Armenian (ո h ս ...) reads as plausible Latin at 0.75+. A switch needs the
            // margin and the script's own characters, so English lines on a bilingual page stay Latin.
            if dom == 0 {
                const LINE_MARGIN: f32 = 0.1;
                const LINE_SHARE: f64 = 0.15;
                const LINE_CONF: f32 = 0.8;
                for gi in (1..GROUPS.len()).filter(|&gi| share[gi] >= LINE_SHARE) {
                    let weak: Vec<usize> = (0..n).filter(|&i| groups[i] == 0).collect();
                    if weak.is_empty() {
                        break;
                    }
                    for (&i, res) in weak.iter().zip(self.rec_for(gi).read(&subset(&weak))?) {
                        // a real line of the script reads at 0.83-0.95; specks and smudges on old scans read as
                        // 2-character CJK at 0.1-0.77, a margin above the Latin junk they replace
                        if res.1 >= out[i].1 + LINE_MARGIN
                            && res.1 >= LINE_CONF
                            && page::script_chars(gi, &res.0) >= 3
                            && page::script_chars(gi, &res.0) > page::latin_letters(&res.0)
                        {
                            out[i] = res;
                            groups[i] = gi;
                        }
                    }
                }
            }
        }
        // lines routed away from the page's script are read with the page's recognizer too;
        // the off-script reading must win clearly
        let odd: Vec<usize> = (0..n).filter(|&i| groups[i] != dom).collect();
        if !odd.is_empty() {
            for (&i, res) in odd.iter().zip(self.rec_for(dom).read(&subset(&odd))?) {
                // A confident reading in the routed script stands: the page's recognizer cannot write
                // that script (Latin drops CJK characters, CJK misspells English) and still scores high.
                // ... and is mostly that script: "REPՍBLQՍE" is a Latin line the world recognizer read
                let routed_ok = out[i].1 >= KEEP_CONF
                    && page::script_chars(groups[i], &out[i].0) >= 2
                    && page::script_chars(groups[i], &out[i].0) > page::latin_letters(&out[i].0);
                let replace = !routed_ok
                    && (out[i].1 < res.1 + 0.1 || page::script_chars(groups[i], &out[i].0) < 3);
                if replace {
                    out[i] = res;
                }
            }
        }
        // every line, not only Latin-routed ones: lines re-read by the page's Latin recognizer keep
        // their routed group, and the world recognizer writes Latin lines with lookalikes
        for o in out.iter_mut() {
            o.0 = page::clean_latin(&o.0);
        }
        Ok((out, flip, dom))
    }
}

/// Word boxes from the recognizer's word spans (shares of the line along its reading direction),
/// placed on the line's quad (top-left, top-right, bottom-right, bottom-left in reading order).
/// Empty unless there is one span per word of the final text: cleanup that merged or split words
/// leaves the line one box.
fn word_boxes(text: &str, spans: &[(f32, f32)], pts: &[[f32; 2]; 4]) -> Vec<crate::ocr::OcrWord> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() < 2 || words.len() != spans.len() {
        return Vec::new();
    }
    let at =
        |a: [f32; 2], b: [f32; 2], f: f32| [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f];
    words
        .iter()
        .zip(spans)
        .map(|(w, &(f0, f1))| {
            let q = [
                at(pts[0], pts[1], f0),
                at(pts[0], pts[1], f1),
                at(pts[3], pts[2], f1),
                at(pts[3], pts[2], f0),
            ];
            let xs = q.iter().map(|p| p[0]);
            let ys = q.iter().map(|p| p[1]);
            crate::ocr::OcrWord {
                text: w.to_string(),
                bbox: [
                    xs.clone().fold(f32::INFINITY, f32::min),
                    ys.clone().fold(f32::INFINITY, f32::min),
                    xs.fold(f32::NEG_INFINITY, f32::max),
                    ys.fold(f32::NEG_INFINITY, f32::max),
                ],
            }
        })
        .collect()
}

/// np.mean over Python floats.
fn mean(v: impl Iterator<Item = f32>) -> f64 {
    let (s, n) = v.fold((0f64, 0usize), |(s, n), x| (s + x as f64, n + 1));
    if n == 0 { f64::NAN } else { s / n as f64 }
}

#[cfg(not(target_arch = "wasm32"))]
impl OcrEngine for OcriaEngine {
    fn name(&self) -> &str {
        "ocria-nano"
    }

    fn recognize<'a, 'b: 'a, 'c: 'a>(
        &'a self,
        image_data: &'c [u8],
        width: u32,
        height: u32,
        _options: &'b OcrOptions,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Vec<OcrResult>, Box<dyn std::error::Error + Send + Sync>>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            self.run(image_data, width, height)
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
        })
    }
}

#[cfg(target_arch = "wasm32")]
impl OcrEngine for OcriaEngine {
    fn name(&self) -> &str {
        "ocria-nano"
    }

    fn recognize<'a, 'b: 'a, 'c: 'a>(
        &'a self,
        image_data: &'c [u8],
        width: u32,
        height: u32,
        _options: &'b OcrOptions,
    ) -> super::OcrFuture<'a> {
        Box::pin(async move {
            self.run(image_data, width, height)
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
        })
    }
}

/// One recognizer's files: the model, its charset (`rec_<group>.charset.json`) and the
/// optional `rec_<group>.meta.json`.
#[cfg(target_arch = "wasm32")]
pub struct RecModel {
    pub onnx: Vec<u8>,
    pub charset: Vec<u8>,
    pub meta: Option<Vec<u8>>,
}

/// An ocria-nano package's files as bytes ([`OcriaEngine::from_models`]).
#[cfg(target_arch = "wasm32")]
pub struct OcriaModels {
    /// `det.onnx` and the optional `det.meta.json`.
    pub det: Vec<u8>,
    pub det_meta: Option<Vec<u8>>,
    /// `router.onnx`.
    pub router: Vec<u8>,
    pub latin: RecModel,
    pub cjk: Option<RecModel>,
    pub rtl: Option<RecModel>,
    pub world: Option<RecModel>,
}

/// Where ocria-nano's model directory lives: `$LITEPARSE_MODEL_DIR/ocria-nano`, else
/// `~/.cache/liteparse/models/ocria-nano` (`%LOCALAPPDATA%\liteparse\models` on Windows).
#[cfg(not(target_arch = "wasm32"))]
pub fn default_model_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("LITEPARSE_MODEL_DIR") {
        return Some(PathBuf::from(d).join("ocria-nano"));
    }
    #[cfg(windows)]
    let base =
        std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("liteparse").join("models"));
    #[cfg(not(windows))]
    let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache/liteparse/models"));
    base.map(|b| b.join("ocria-nano"))
}

/// Whether a complete ocria-nano package is installed (models + runtime), checked once per
/// process: an installed package makes ocria-nano the default OCR backend.
#[cfg(not(target_arch = "wasm32"))]
pub fn installed() -> bool {
    static INSTALLED: OnceLock<bool> = OnceLock::new();
    *INSTALLED.get_or_init(|| {
        default_model_dir().is_some_and(|d| {
            d.join(DETECTOR_FILE).exists()
                && (std::env::var_os("ORT_DYLIB_PATH").is_some()
                    || d.join("runtime").join(RUNTIME_LIB).exists())
        })
    })
}

/// The process-wide engine as an [`OcrEngine`] that loads on its first page: documents that
/// need no OCR never load ONNX Runtime or the models.
#[cfg(not(target_arch = "wasm32"))]
pub struct LazyEngine;

/// [`LazyEngine`] once the package is known to be there (the check is a few file lookups;
/// loading waits for the first OCR'd page).
#[cfg(not(target_arch = "wasm32"))]
pub fn lazy_engine() -> Result<Arc<LazyEngine>, LiteParseError> {
    if !installed() {
        // the same message the load would give
        shared_engine()?;
    }
    Ok(Arc::new(LazyEngine))
}

#[cfg(not(target_arch = "wasm32"))]
impl OcrEngine for LazyEngine {
    fn name(&self) -> &str {
        "ocria-nano"
    }

    fn recognize<'a, 'b: 'a, 'c: 'a>(
        &'a self,
        image_data: &'c [u8],
        width: u32,
        height: u32,
        _options: &'b OcrOptions,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Vec<OcrResult>, Box<dyn std::error::Error + Send + Sync>>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            let engine = shared_engine()
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
            engine
                .run(image_data, width, height)
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
        })
    }
}

/// ONNX Runtime's shared-library file name on this platform.
#[cfg(not(target_arch = "wasm32"))]
pub const RUNTIME_LIB: &str = if cfg!(target_os = "macos") {
    "libonnxruntime.dylib"
} else if cfg!(windows) {
    "onnxruntime.dll"
} else {
    "libonnxruntime.so"
};

/// Load ONNX Runtime for the model package in `dir` (`ORT_DYLIB_PATH`, else `<dir>/runtime/`), as
/// the OCR engine does, for other in-process models of the package (LayoutNet). `false` when the
/// runtime isn't there or doesn't load.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn runtime_for_package(dir: &Path) -> bool {
    let runtime = match std::env::var_os("ORT_DYLIB_PATH") {
        Some(p) => PathBuf::from(p),
        None => dir.join("runtime").join(RUNTIME_LIB),
    };
    runtime.exists() && init_runtime(Some(&runtime), 0).is_ok()
}

/// The process-wide engine, loaded on first use from [`default_model_dir`]; ONNX Runtime from
/// `ORT_DYLIB_PATH`, else `<model dir>/runtime/`.
#[cfg(not(target_arch = "wasm32"))]
pub fn shared_engine() -> Result<Arc<OcriaEngine>, LiteParseError> {
    static ENGINE: OnceLock<Result<Arc<OcriaEngine>, String>> = OnceLock::new();
    ENGINE
        .get_or_init(|| {
            let dir = default_model_dir().ok_or("no home directory to find the ocria-nano models in")?;
            if !dir.join(DETECTOR_FILE).exists() {
                return Err(format!(
                    "ocria-nano models not found in {}; download them with `lit models pull ocria-nano`",
                    dir.display()
                ));
            }
            let runtime = match std::env::var_os("ORT_DYLIB_PATH") {
                Some(p) => PathBuf::from(p),
                None => dir.join("runtime").join(RUNTIME_LIB),
            };
            if !runtime.exists() {
                return Err(format!(
                    "ONNX Runtime not found at {}; run `lit models pull ocria-nano` again",
                    runtime.display()
                ));
            }
            OcriaEngine::load(&dir, Some(&runtime), OcriaOptions::default())
                .map(Arc::new)
                .map_err(|e| e.to_string())
        })
        .clone()
        .map_err(LiteParseError::Ocr)
}
