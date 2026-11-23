//! TableNet forward pass in plain Rust (no ONNX runtime, so it builds for wasm32 too).
//!
//! Mirrors `bench/tables/tablenet.py`: an input MLP over the unit features (all but the last two
//! id columns), a pre-norm transformer encoder (ReLU feed-forward, no final norm), a 2-way node
//! head (in_table, header) and four bilinear pair heads (same_table / row / col / cell) plus a
//! linear term over pairwise geometry (`pair_geometry`). Weights come from
//! `bench/tables/export_tablenet_rs.py`: `b"TNET"`, u32 LE header length, JSON header, raw LE f32.

use std::collections::HashMap;

use serde::Deserialize;

/// Unit features per row: 66 fed to the encoder + block id + leaf id.
pub const N_IDS: usize = 2;

#[derive(Deserialize, Clone, Debug)]
pub struct NetConfig {
    pub d: usize,
    pub layers: usize,
    pub heads: usize,
    pub pair_d: usize,
    pub stack_feats: bool,
    pub rule_feats: bool,
    /// joint layout model: a fifth pair head (same_block, not computed here) and a block class head
    #[serde(default)]
    pub layout: bool,
    #[serde(default)]
    pub n_cls: usize,
    /// image branch (tablenet.py ImgBranch): CNN channels and cross-attention layers
    #[serde(default)]
    pub img: Option<ImgCfg>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ImgCfg {
    #[serde(default = "default_img_c")]
    pub c: usize,
    #[serde(default)]
    pub xattn: usize,
}

fn default_img_c() -> usize {
    64
}

/// Side of the square canvas the image branch reads (tablenet.py / train_tablenet.IMG_SIDE).
pub const IMG_SIDE: usize = 1024;

/// One page's input to the image branch: the page as grey ink in [0, 1] (white = 0) on the
/// IMG_SIDE x IMG_SIDE canvas, top-left aligned; unit boxes and the real image extent as
/// fractions of the canvas.
pub struct ImgInput {
    pub canvas: Vec<f32>,
    pub boxes: Vec<[f32; 4]>,
    pub valid: (f32, f32),
}

#[derive(Deserialize)]
struct TensorMeta {
    name: String,
    shape: Vec<usize>,
    offset: usize,
}

#[derive(Deserialize)]
struct Header {
    cfg: NetConfig,
    feat_dim: usize,
    #[serde(default)]
    tensors: Vec<TensorMeta>,
}

/// ONNX metadata key holding the model's header (`{"cfg": .., "feat_dim": ..}`, as in the `.bin`), so
/// a package can ship LayoutNet as ONNX only.
#[cfg(feature = "ocria-nano")]
pub const ONNX_HEADER_KEY: &str = "liteparse.tablenet";

/// A row-major weight matrix [out, in] with its bias.
struct Linear {
    w: Vec<f32>,
    b: Vec<f32>,
    n_out: usize,
    n_in: usize,
}

impl Linear {
    /// No weights: the layers of a model that runs through ONNX Runtime only.
    #[cfg(feature = "ocria-nano")]
    fn empty() -> Linear {
        Linear {
            w: Vec::new(),
            b: Vec::new(),
            n_out: 0,
            n_in: 0,
        }
    }

    /// y[n, out] = x[n, in] W^T + b (a blocked SIMD GEMM; W is [out, in] row-major, so W^T is
    /// read with row stride 1 and column stride `in`)
    fn forward(&self, x: &[f32], n: usize) -> Vec<f32> {
        let mut y = vec![0f32; n * self.n_out];
        for r in 0..n {
            y[r * self.n_out..(r + 1) * self.n_out].copy_from_slice(&self.b);
        }
        if n > 0 {
            // SAFETY: the slices hold n*in, out*in and n*out elements, matching the strides
            unsafe {
                gemm(
                    n,
                    self.n_in,
                    self.n_out,
                    1.0,
                    x.as_ptr(),
                    self.n_in as isize,
                    1,
                    self.w.as_ptr(),
                    1,
                    self.n_in as isize,
                    1.0,
                    y.as_mut_ptr(),
                    self.n_out as isize,
                    1,
                );
            }
        }
        y
    }
}

struct LayerNorm {
    g: Vec<f32>,
    b: Vec<f32>,
}

impl LayerNorm {
    fn forward(&self, x: &[f32], n: usize) -> Vec<f32> {
        let d = self.g.len();
        let mut y = vec![0f32; x.len()];
        for r in 0..n {
            let xr = &x[r * d..(r + 1) * d];
            let mean = xr.iter().sum::<f32>() / d as f32;
            let var = xr.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / d as f32;
            let inv = 1.0 / (var + 1e-5).sqrt();
            for k in 0..d {
                y[r * d + k] = (xr[k] - mean) * inv * self.g[k] + self.b[k];
            }
        }
        y
    }
}

struct Layer {
    in_proj: Linear,
    out_proj: Linear,
    lin1: Linear,
    lin2: Linear,
    norm1: LayerNorm,
    norm2: LayerNorm,
}

/// The weight file's tensors by name, taken out one by one while building the layers.
struct Tensors(HashMap<String, (Vec<usize>, Vec<f32>)>);

impl Tensors {
    fn take(&mut self, name: &str) -> Result<(Vec<usize>, Vec<f32>), String> {
        self.0
            .remove(name)
            .ok_or_else(|| format!("missing tensor {name}"))
    }
    fn linear(&mut self, p: &str) -> Result<Linear, String> {
        let (ws, w) = self.take(&format!("{p}.weight"))?;
        let (_, b) = self.take(&format!("{p}.bias"))?;
        Ok(Linear {
            w,
            b,
            n_out: ws[0],
            n_in: ws[1],
        })
    }
    /// Conv2d (no bias) + BatchNorm2d (eval statistics) folded into one convolution.
    fn conv_bn(&mut self, conv: &str, bn: &str, stride: usize, pad: usize) -> Result<Conv, String> {
        let (ws, mut w) = self.take(&format!("{conv}.weight"))?;
        let g = self.take(&format!("{bn}.weight"))?.1;
        let beta = self.take(&format!("{bn}.bias"))?.1;
        let mean = self.take(&format!("{bn}.running_mean"))?.1;
        let var = self.take(&format!("{bn}.running_var"))?.1;
        let (cout, cin, k) = (ws[0], ws[1], ws[2]);
        let per = cin * k * k;
        let mut b = vec![0f32; cout];
        for o in 0..cout {
            let sc = g[o] / (var[o] + 1e-5).sqrt();
            for v in &mut w[o * per..(o + 1) * per] {
                *v *= sc;
            }
            b[o] = beta[o] - mean[o] * sc;
        }
        Ok(Conv {
            w,
            b,
            cin,
            cout,
            k,
            stride,
            pad,
        })
    }
    fn norm(&mut self, p: &str) -> Result<LayerNorm, String> {
        Ok(LayerNorm {
            g: self.take(&format!("{p}.weight"))?.1,
            b: self.take(&format!("{p}.bias"))?.1,
        })
    }
}

/// A 2D convolution with its BatchNorm folded in, followed by ReLU. Weight [cout, cin * k * k].
struct Conv {
    w: Vec<f32>,
    b: Vec<f32>,
    cin: usize,
    cout: usize,
    k: usize,
    stride: usize,
    pad: usize,
}

impl Conv {
    /// x [cin, h, w] -> [cout, ho, wo] (im2col + GEMM, then ReLU)
    fn forward(&self, x: &[f32], h: usize, w: usize) -> (Vec<f32>, usize, usize) {
        let (k, s, p) = (self.k, self.stride, self.pad);
        let ho = (h + 2 * p - k) / s + 1;
        let wo = (w + 2 * p - k) / s + 1;
        let kk = self.cin * k * k;
        let np = ho * wo;
        let mut col = vec![0f32; kk * np];
        for c in 0..self.cin {
            for ky in 0..k {
                for kx in 0..k {
                    let row =
                        &mut col[((c * k + ky) * k + kx) * np..((c * k + ky) * k + kx + 1) * np];
                    for oy in 0..ho {
                        let iy = (oy * s + ky) as isize - p as isize;
                        if iy < 0 || iy >= h as isize {
                            continue;
                        }
                        let src = &x[(c * h + iy as usize) * w..(c * h + iy as usize + 1) * w];
                        let dst = &mut row[oy * wo..(oy + 1) * wo];
                        for (ox, d) in dst.iter_mut().enumerate() {
                            let ix = (ox * s + kx) as isize - p as isize;
                            if ix >= 0 && ix < w as isize {
                                *d = src[ix as usize];
                            }
                        }
                    }
                }
            }
        }
        let mut y = vec![0f32; self.cout * np];
        for (o, yo) in y.chunks_exact_mut(np).enumerate() {
            yo.fill(self.b[o]);
        }
        // SAFETY: w is [cout, kk], col [kk, np], y [cout, np], all row-major
        unsafe {
            gemm(
                self.cout,
                kk,
                np,
                1.0,
                self.w.as_ptr(),
                kk as isize,
                1,
                col.as_ptr(),
                np as isize,
                1,
                1.0,
                y.as_mut_ptr(),
                np as isize,
                1,
            );
        }
        for v in y.iter_mut() {
            *v = v.max(0.0);
        }
        (y, ho, wo)
    }
}

/// tablenet.py ImgBranch: patchify CNN (stride 16), per-unit samples on a GH x GW grid inside
/// each unit box -> MLP added to the unit's input, and (xattn) stride-32 patch tokens the top
/// encoder layers cross-attend to.
struct ImgBranch {
    convs: Vec<Conv>,
    c: usize,
    unit0: Linear,
    unit2: Linear,
    tok: Option<Linear>,
    attn: Vec<(Linear, Linear)>,
    norms: Vec<LayerNorm>,
}

const IMG_GH: usize = 2;
const IMG_GW: usize = 6;

/// tablenet.py `_pos2d`: sin-cos of (x, y) in [0, 1], `dim` values.
fn pos2d(x: f32, y: f32, dim: usize, out: &mut [f32]) {
    let k = dim / 4;
    let step = -(1000f32).ln() / (k.max(2) - 1) as f32;
    for i in 0..k {
        let fr = (i as f32 * step).exp() * std::f32::consts::PI * 64.0;
        out[i] = (x * fr).sin();
        out[k + i] = (x * fr).cos();
        out[2 * k + i] = (y * fr).sin();
        out[3 * k + i] = (y * fr).cos();
    }
}

/// Bilinear sample of channel plane `m` [h, w] at fractional position (fx, fy) of the map
/// (grid_sample, align_corners=False, zeros outside).
#[inline]
fn bilinear(m: &[f32], h: usize, w: usize, full_h: usize, full_w: usize, fx: f32, fy: f32) -> f32 {
    // (full_h, full_w): the full map's size, of which `m` [h, w] may be the top-left crop
    let x = fx * full_w as f32 - 0.5;
    let y = fy * full_h as f32 - 0.5;
    let (x0, y0) = (x.floor(), y.floor());
    let (ax, ay) = (x - x0, y - y0);
    let at = |yy: f32, xx: f32| -> f32 {
        if yy < 0.0 || xx < 0.0 || yy >= h as f32 || xx >= w as f32 {
            0.0
        } else {
            m[yy as usize * w + xx as usize]
        }
    };
    at(y0, x0) * (1.0 - ax) * (1.0 - ay)
        + at(y0, x0 + 1.0) * ax * (1.0 - ay)
        + at(y0 + 1.0, x0) * (1.0 - ax) * ay
        + at(y0 + 1.0, x0 + 1.0) * ax * ay
}

pub struct TableNet {
    pub cfg: NetConfig,
    pub feat_dim: usize,
    inp0: Linear,
    inp2: Linear,
    layers: Vec<Layer>,
    node: Linear,
    qk: Linear,
    pair_geo: Linear,
    /// block class head (layout models)
    cls: Option<Linear>,
    img: Option<ImgBranch>,
    /// The same model exported to ONNX, run through ONNX Runtime when the model package ships it
    /// (see [`TableNet::attach_onnx`]); `forward` falls back to the Rust implementation otherwise.
    #[cfg(feature = "ocria-nano")]
    onnx: Option<OnnxBackend>,
}

/// LayoutNet through ONNX Runtime: idle single-threaded sessions, one per concurrent page (pages
/// run on parallel workers, so one intra-op thread per call is the fastest use of the cores).
#[cfg(feature = "ocria-nano")]
struct OnnxBackend {
    path: std::path::PathBuf,
    idle: std::sync::Mutex<Vec<ort::session::Session>>,
}

#[cfg(feature = "ocria-nano")]
impl OnnxBackend {
    /// The model on one page; `None` (the caller falls back to Rust) if ONNX Runtime fails.
    fn forward(&self, feats: &[f32], pairf: &[f32], n: usize, img: &ImgInput) -> Option<Logits> {
        use ort::value::Tensor;
        let fd = feats.len() / n.max(1);
        let inputs = ort::inputs![
            "feats" => Tensor::from_array(([1usize, n, fd], feats.to_vec())).ok()?,
            "mask" => Tensor::from_array(([1usize, n], vec![true; n])).ok()?,
            "pairf" => Tensor::from_array(([1usize, n, n, 2], pairf.to_vec())).ok()?,
            "image" => Tensor::from_array(([1usize, 1, IMG_SIDE, IMG_SIDE], img.canvas.clone())).ok()?,
            "boxes" => Tensor::from_array(([1usize, n, 4], img.boxes.iter().flatten().copied().collect::<Vec<f32>>())).ok()?,
            "valid" => Tensor::from_array(([1usize, 2], vec![img.valid.0, img.valid.1])).ok()?,
        ];
        let popped = self.idle.lock().ok()?.pop();
        let mut sess = match popped {
            Some(s) => s,
            None => crate::ocr::ocria::pool::open(&self.path, Some(1)).ok()?,
        };
        let logits = (|| {
            let out = sess.run(inputs).ok()?;
            let (_, node) = out["node"].try_extract_tensor::<f32>().ok()?;
            let (pshape, pair) = out["pair"].try_extract_tensor::<f32>().ok()?;
            let (cshape, cls) = out["cls"].try_extract_tensor::<f32>().ok()?;
            Some(Logits {
                n,
                node: node.to_vec(),
                pair: pair.to_vec(),
                cls: cls.to_vec(),
                n_cls: cshape[2] as usize,
                n_pair: pshape[1] as usize,
            })
        })();
        if let Ok(mut idle) = self.idle.lock() {
            idle.push(sess);
        }
        logits
    }
}

/// Node logits [n, 2], pair logits [4, n, n] (row-major) and, from a layout model, block class
/// logits [n, n_cls] (empty otherwise).
pub struct Logits {
    pub n: usize,
    pub node: Vec<f32>,
    pub pair: Vec<f32>,
    pub cls: Vec<f32>,
    pub n_cls: usize,
    /// pair heads present in `pair`: 4, or 5 with same_block (layout blocks)
    pub n_pair: usize,
}

impl Logits {
    #[inline]
    pub fn node(&self, i: usize, k: usize) -> f32 {
        self.node[i * 2 + k]
    }
    #[inline]
    pub fn pair(&self, k: usize, i: usize, j: usize) -> f32 {
        self.pair[(k * self.n + i) * self.n + j]
    }
}

#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    // eight independent accumulators over fixed-size chunks: LLVM turns this into SIMD
    // multiply-adds (NEON / SSE); the summation order differs from PyTorch's only in rounding
    let ca = a.chunks_exact(8);
    let cb = b.chunks_exact(8);
    let (ra, rb) = (ca.remainder(), cb.remainder());
    let mut acc = [0f32; 8];
    for (x, y) in ca.zip(cb) {
        for k in 0..8 {
            acc[k] += x[k] * y[k];
        }
    }
    let mut s = (acc[0] + acc[4]) + (acc[1] + acc[5]) + (acc[2] + acc[6]) + (acc[3] + acc[7]);
    for (x, y) in ra.iter().zip(rb) {
        s += x * y;
    }
    s
}

impl TableNet {
    /// Parse the exported weight file.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 8 || &bytes[..4] != b"TNET" {
            return Err("not a TableNet weight file".into());
        }
        let hlen = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
        let header: Header = serde_json::from_slice(&bytes[8..8 + hlen])
            .map_err(|e| format!("weight header: {e}"))?;
        let data = &bytes[8 + hlen..];
        let mut t: HashMap<String, (Vec<usize>, Vec<f32>)> = HashMap::new();
        for m in &header.tensors {
            let n: usize = m.shape.iter().product();
            let start = m.offset * 4;
            let raw = data
                .get(start..start + n * 4)
                .ok_or_else(|| format!("tensor {} out of range", m.name))?;
            let v = raw
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            t.insert(m.name.clone(), (m.shape.clone(), v));
        }
        let mut t = Tensors(t);
        let inp0 = t.linear("inp.0")?;
        let inp2 = t.linear("inp.2")?;
        let node = t.linear("node")?;
        let qk = t.linear("qk")?;
        let pair_geo = t.linear("pair_geo")?;
        let cls = if header.cfg.layout {
            Some(t.linear("cls")?)
        } else {
            None
        };
        let mut layers = Vec::new();
        for l in 0..header.cfg.layers {
            let p = format!("enc.layers.{l}");
            let (ws, w) = t.take(&format!("{p}.self_attn.in_proj_weight"))?;
            let (_, b) = t.take(&format!("{p}.self_attn.in_proj_bias"))?;
            let in_proj = Linear {
                w,
                b,
                n_out: ws[0],
                n_in: ws[1],
            };
            let out_proj = t.linear(&format!("{p}.self_attn.out_proj"))?;
            let lin1 = t.linear(&format!("{p}.linear1"))?;
            let lin2 = t.linear(&format!("{p}.linear2"))?;
            let norm1 = t.norm(&format!("{p}.norm1"))?;
            let norm2 = t.norm(&format!("{p}.norm2"))?;
            layers.push(Layer {
                in_proj,
                out_proj,
                lin1,
                lin2,
                norm1,
                norm2,
            });
        }
        let img = match &header.cfg.img {
            None => None,
            Some(ic) => {
                // tablenet.py ImgBranch.cnn: [conv 8x8/8, BN, ReLU, (3x3/1), (3x3/2), (3x3/1)]
                let convs = vec![
                    t.conv_bn("img.cnn.0", "img.cnn.1", 8, 0)?,
                    t.conv_bn("img.cnn.3.0", "img.cnn.3.1", 1, 1)?,
                    t.conv_bn("img.cnn.4.0", "img.cnn.4.1", 2, 1)?,
                    t.conv_bn("img.cnn.5.0", "img.cnn.5.1", 1, 1)?,
                ];
                let mut attn = Vec::new();
                let mut norms = Vec::new();
                for i in 0..ic.xattn {
                    let (ws, w) = t.take(&format!("img.attn.{i}.in_proj_weight"))?;
                    let (_, b) = t.take(&format!("img.attn.{i}.in_proj_bias"))?;
                    let inp = Linear {
                        w,
                        b,
                        n_out: ws[0],
                        n_in: ws[1],
                    };
                    attn.push((inp, t.linear(&format!("img.attn.{i}.out_proj"))?));
                    norms.push(t.norm(&format!("img.norm.{i}"))?);
                }
                Some(ImgBranch {
                    convs,
                    c: ic.c,
                    unit0: t.linear("img.unit.0")?,
                    unit2: t.linear("img.unit.2")?,
                    tok: if ic.xattn > 0 {
                        Some(t.linear("img.tok")?)
                    } else {
                        None
                    },
                    attn,
                    norms,
                })
            }
        };
        Ok(TableNet {
            cfg: header.cfg,
            feat_dim: header.feat_dim,
            inp0,
            inp2,
            layers,
            node,
            qk,
            pair_geo,
            cls,
            img,
            #[cfg(feature = "ocria-nano")]
            onnx: None,
        })
    }

    /// Run the model through ONNX Runtime from `model` (the same weights exported to ONNX, with the
    /// image branch) when the runtime of the package in `package_dir` loads. Returns whether it did.
    #[cfg(feature = "ocria-nano")]
    pub fn attach_onnx(&mut self, model: &std::path::Path, package_dir: &std::path::Path) -> bool {
        if self.img.is_none()
            || !model.exists()
            || !crate::ocr::ocria::runtime_for_package(package_dir)
        {
            return false;
        }
        // open one session now, so a bad file falls back to Rust here rather than on every page
        match crate::ocr::ocria::pool::open(model, Some(1)) {
            Ok(sess) => {
                self.onnx = Some(OnnxBackend {
                    path: model.to_path_buf(),
                    idle: std::sync::Mutex::new(vec![sess]),
                });
                true
            }
            Err(_) => false,
        }
    }

    /// Self-attention block (pre-norm input `h`), returns the attention output [n, d].
    fn attention(&self, l: &Layer, h: &[f32], n: usize) -> Vec<f32> {
        let d = self.cfg.d;
        let heads = self.cfg.heads;
        let hd = d / heads;
        let qkv = l.in_proj.forward(h, n); // [n, 3d]: q | k | v
        let scale = 1.0 / (hd as f32).sqrt();
        let mut ctx = vec![0f32; n * d];
        let mut scores = vec![0f32; n * n];
        for hh in 0..heads {
            // scores = Q_h K_h^T * scale (Q_h, K_h are strided column blocks of qkv)
            // SAFETY: every pointer + stride stays inside qkv (n * 3d), scores (n * n), ctx (n * d)
            unsafe {
                gemm(
                    n,
                    hd,
                    n,
                    scale,
                    qkv.as_ptr().add(hh * hd),
                    3 * d as isize,
                    1,
                    qkv.as_ptr().add(d + hh * hd),
                    1,
                    3 * d as isize,
                    0.0,
                    scores.as_mut_ptr(),
                    n as isize,
                    1,
                );
            }
            for row in scores.chunks_exact_mut(n) {
                let mx = row.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                let mut z = 0f32;
                for v in row.iter_mut() {
                    *v = (*v - mx).exp();
                    z += *v;
                }
                let inv = 1.0 / z;
                for v in row.iter_mut() {
                    *v *= inv;
                }
            }
            // ctx_h = P V_h, written into ctx's column block for this head
            unsafe {
                gemm(
                    n,
                    n,
                    hd,
                    1.0,
                    scores.as_ptr(),
                    n as isize,
                    1,
                    qkv.as_ptr().add(2 * d + hh * hd),
                    3 * d as isize,
                    1,
                    0.0,
                    ctx.as_mut_ptr().add(hh * hd),
                    d as isize,
                    1,
                );
            }
        }
        l.out_proj.forward(&ctx, n)
    }

    /// Whether the model has an image branch (it then wants `ImgInput` in `forward`).
    pub fn has_img(&self) -> bool {
        self.img.is_some() || self.cfg.img.is_some()
    }

    /// The model from its ONNX export alone (a package without `tablenet.bin`): the header comes from
    /// the ONNX metadata ([`ONNX_HEADER_KEY`]) and every page runs through ONNX Runtime, loaded from
    /// the package in `package_dir`. `None` when the runtime, the file or its header is missing.
    #[cfg(feature = "ocria-nano")]
    pub fn from_onnx(model: &std::path::Path, package_dir: &std::path::Path) -> Option<TableNet> {
        if !model.exists() || !crate::ocr::ocria::runtime_for_package(package_dir) {
            return None;
        }
        let sess = crate::ocr::ocria::pool::open(model, Some(1)).ok()?;
        let header = {
            let meta = sess.metadata().ok()?;
            meta.custom(ONNX_HEADER_KEY)?
        };
        let header: Header = serde_json::from_str(&header).ok()?;
        // the export takes the page image; a model without the image branch ships as `.bin`
        header.cfg.img.as_ref()?;
        Some(TableNet {
            cfg: header.cfg,
            feat_dim: header.feat_dim,
            inp0: Linear::empty(),
            inp2: Linear::empty(),
            layers: Vec::new(),
            node: Linear::empty(),
            qk: Linear::empty(),
            pair_geo: Linear::empty(),
            cls: None,
            img: None,
            onnx: Some(OnnxBackend {
                path: model.to_path_buf(),
                idle: std::sync::Mutex::new(vec![sess]),
            }),
        })
    }

    /// Logits that decode to nothing (no tables, links or blocks): the result for a page a
    /// model loaded from ONNX alone couldn't run (ONNX Runtime failed on it).
    fn neutral_logits(&self, n: usize) -> Logits {
        // no class head and no same_block links: block decoding skips the page
        Logits {
            n,
            node: vec![-1.0; n * 2],
            pair: vec![-1.0; 4 * n * n],
            cls: Vec::new(),
            n_cls: 0,
            n_pair: 4,
        }
    }

    /// The image branch on one page: per-unit features [n, d] to add to the encoder input, and
    /// (xattn) the patch tokens [T, d] with their padding flags and the units' positions [n, d].
    #[allow(clippy::type_complexity)]
    fn image_features(
        &self,
        ib: &ImgBranch,
        im: &ImgInput,
        n: usize,
    ) -> (Vec<f32>, Option<(Vec<f32>, Vec<bool>, Vec<f32>)>) {
        let d = self.cfg.d;
        // Only the top-left region holding the page and every unit box, plus a margin past the
        // CNN's reach (receptive field 72 px, boundary effects 32 px), is computed: unit samples and
        // unmasked tokens lie inside it, so the result equals the full-canvas pass. Unit boxes can
        // reach past the page (text placed off the page's edge); blank canvas there still has
        // non-zero CNN features, so the region must cover them too.
        const MARGIN: usize = 96;
        let crop = |v: f32| -> usize {
            (((v * IMG_SIDE as f32).ceil() as usize + MARGIN).div_ceil(16) * 16).min(IMG_SIDE)
        };
        let (bx1, by1) = im
            .boxes
            .iter()
            .take(n)
            .fold((0f32, 0f32), |(x, y), b| (x.max(b[2]), y.max(b[3])));
        let (cw, ch0) = (crop(im.valid.0.max(bx1)), crop(im.valid.1.max(by1)));
        let mut fm = vec![0f32; ch0 * cw];
        for y in 0..ch0 {
            fm[y * cw..(y + 1) * cw].copy_from_slice(&im.canvas[y * IMG_SIDE..y * IMG_SIDE + cw]);
        }
        let (mut h, mut w) = (ch0, cw);
        let full = IMG_SIDE / 16;
        for cv in &ib.convs {
            let (y, ho, wo) = cv.forward(&fm, h, w);
            fm = y;
            h = ho;
            w = wo;
        }
        let c = ib.c;
        let plane = h * w;
        let per = c * IMG_GH * IMG_GW;
        let mut s = vec![0f32; n * per];
        for (i, b) in im.boxes.iter().enumerate().take(n) {
            for gh in 0..IMG_GH {
                let fy = b[1] + (b[3] - b[1]) * (gh as f32 + 0.5) / IMG_GH as f32;
                for gw in 0..IMG_GW {
                    let fx = b[0] + (b[2] - b[0]) * (gw as f32 + 0.5) / IMG_GW as f32;
                    for ch in 0..c {
                        s[i * per + ch * IMG_GH * IMG_GW + gh * IMG_GW + gw] =
                            bilinear(&fm[ch * plane..(ch + 1) * plane], h, w, full, full, fx, fy);
                    }
                }
            }
        }
        let mut u = ib.unit0.forward(&s, n);
        for v in u.iter_mut() {
            *v = v.max(0.0);
        }
        let u = ib.unit2.forward(&u, n);
        let Some(tok_lin) = &ib.tok else {
            return (u, None);
        };
        // stride-32 tokens: 2x2 average pool of the stride-16 map at the full grid's positions;
        // tokens past the page's extent are masked in training, so only the others are built
        let (gth, gtw) = (full / 2, full / 2);
        let mut cells = Vec::new();
        for ty in 0..gth {
            for tx in 0..gtw {
                let (px, py) = (
                    (tx as f32 + 0.5) / gtw as f32,
                    (ty as f32 + 0.5) / gth as f32,
                );
                if px <= im.valid.0 && py <= im.valid.1 && 2 * ty + 1 < h && 2 * tx + 1 < w {
                    cells.push((ty, tx, px, py));
                }
            }
        }
        let nt = cells.len();
        let mut t = vec![0f32; nt * c];
        for (k, &(ty, tx, _, _)) in cells.iter().enumerate() {
            let (y, x) = (2 * ty, 2 * tx);
            for ch in 0..c {
                let m = &fm[ch * plane..(ch + 1) * plane];
                t[k * c + ch] =
                    (m[y * w + x] + m[y * w + x + 1] + m[(y + 1) * w + x] + m[(y + 1) * w + x + 1])
                        / 4.0;
            }
        }
        let mut tok = tok_lin.forward(&t, nt);
        let pad = vec![false; nt];
        let mut pe = vec![0f32; d];
        for (k, &(_, _, px, py)) in cells.iter().enumerate() {
            pos2d(px, py, d, &mut pe);
            for (a, b) in tok[k * d..(k + 1) * d].iter_mut().zip(&pe) {
                *a += b;
            }
        }
        let mut upos = vec![0f32; n * d];
        for (i, b) in im.boxes.iter().enumerate().take(n) {
            pos2d(
                (b[0] + b[2]) / 2.0,
                (b[1] + b[3]) / 2.0,
                d,
                &mut upos[i * d..(i + 1) * d],
            );
        }
        (u, Some((tok, pad, upos)))
    }

    /// x += cross-attention of the units to the image tokens (pre-norm query + unit positions).
    #[allow(clippy::too_many_arguments)]
    fn cross_attention(
        &self,
        ib: &ImgBranch,
        i: usize,
        x: &mut [f32],
        n: usize,
        tok: &[f32],
        pad: &[bool],
        upos: &[f32],
    ) {
        let d = self.cfg.d;
        let heads = self.cfg.heads;
        let hd = d / heads;
        let nt = pad.len();
        let mut q = ib.norms[i].forward(x, n);
        for (a, b) in q.iter_mut().zip(upos) {
            *a += b;
        }
        let (inp, out) = &ib.attn[i];
        let lin = |rows: std::ops::Range<usize>, v: &[f32], m: usize| -> Vec<f32> {
            Linear {
                w: inp.w[rows.start * d..rows.end * d].to_vec(),
                b: inp.b[rows.clone()].to_vec(),
                n_out: rows.len(),
                n_in: d,
            }
            .forward(v, m)
        };
        let qq = lin(0..d, &q, n);
        let kk = lin(d..2 * d, tok, nt);
        let vv = lin(2 * d..3 * d, tok, nt);
        let scale = 1.0 / (hd as f32).sqrt();
        let mut ctx = vec![0f32; n * d];
        let mut sc = vec![0f32; n * nt];
        for hh in 0..heads {
            // scores = Q_h K_h^T * scale, [n, nt]
            // SAFETY: strided column blocks of qq (n * d) and kk (nt * d) into sc (n * nt)
            unsafe {
                gemm(
                    n,
                    hd,
                    nt,
                    scale,
                    qq.as_ptr().add(hh * hd),
                    d as isize,
                    1,
                    kk.as_ptr().add(hh * hd),
                    1,
                    d as isize,
                    0.0,
                    sc.as_mut_ptr(),
                    nt as isize,
                    1,
                );
            }
            for row in sc.chunks_exact_mut(nt) {
                let mut mx = f32::NEG_INFINITY;
                for (v, &p) in row.iter().zip(pad) {
                    if !p {
                        mx = mx.max(*v);
                    }
                }
                let mut z = 0f32;
                for (v, &p) in row.iter_mut().zip(pad) {
                    *v = if p { 0.0 } else { (*v - mx).exp() };
                    z += *v;
                }
                let inv = 1.0 / z;
                for v in row.iter_mut() {
                    *v *= inv;
                }
            }
            // ctx_h = P V_h into ctx's column block for this head
            unsafe {
                gemm(
                    n,
                    nt,
                    hd,
                    1.0,
                    sc.as_ptr(),
                    nt as isize,
                    1,
                    vv.as_ptr().add(hh * hd),
                    d as isize,
                    1,
                    0.0,
                    ctx.as_mut_ptr().add(hh * hd),
                    d as isize,
                    1,
                );
            }
        }
        let o = out.forward(&ctx, n);
        for (a, b) in x.iter_mut().zip(o) {
            *a += b;
        }
    }

    /// feats: [n, feat_dim] row-major; pairf: [n, n, 2] rule pair inputs (zeros when unknown);
    /// img: the page image for a model with an image branch (None: the branch adds nothing, as
    /// for a training page without a render).
    pub fn forward(
        &self,
        feats: &[f32],
        pairf: &[f32],
        n: usize,
        img: Option<&ImgInput>,
    ) -> Logits {
        #[cfg(feature = "ocria-nano")]
        if let (Some(o), Some(im)) = (&self.onnx, img)
            && let Some(l) = o.forward(feats, pairf, n, im)
        {
            return l;
        }
        if self.layers.is_empty() {
            return self.neutral_logits(n);
        }
        let fd = self.feat_dim;
        let fin = fd - N_IDS;
        let mut x0 = vec![0f32; n * fin];
        for r in 0..n {
            x0[r * fin..(r + 1) * fin].copy_from_slice(&feats[r * fd..r * fd + fin]);
        }
        let mut x = self.inp0.forward(&x0, n);
        for v in x.iter_mut() {
            *v = v.max(0.0);
        }
        let mut x = self.inp2.forward(&x, n);
        let mut xtok = None;
        if let (Some(ib), Some(im)) = (&self.img, img) {
            let (u, tk) = self.image_features(ib, im, n);
            for (a, b) in x.iter_mut().zip(u) {
                *a += b;
            }
            xtok = tk;
        }
        let xa = self.img.as_ref().map_or(0, |ib| ib.attn.len());
        let nl = self.layers.len();
        for (li, l) in self.layers.iter().enumerate() {
            let h = l.norm1.forward(&x, n);
            let a = self.attention(l, &h, n);
            for (xv, av) in x.iter_mut().zip(a) {
                *xv += av;
            }
            let h = l.norm2.forward(&x, n);
            let mut f = l.lin1.forward(&h, n);
            for v in f.iter_mut() {
                *v = v.max(0.0);
            }
            let f = l.lin2.forward(&f, n);
            for (xv, fv) in x.iter_mut().zip(f) {
                *xv += fv;
            }
            if let (Some(ib), Some((tok, pad, upos))) = (&self.img, &xtok)
                && li + xa >= nl
            {
                self.cross_attention(ib, li + xa - nl, &mut x, n, tok, pad, upos);
            }
        }
        let node = self.node.forward(&x, n);
        let qk = self.qk.forward(&x, n); // [n, 2 * P * pd]: (q | k) x head x pd
        let pd = self.cfg.pair_d;
        // P pair heads in the projection (5 with layout); the 4 table heads are always computed,
        // same_block (head 4) only for layout models, which emit layout blocks
        let np = if self.cfg.layout { 5 } else { 4 };
        let nh = if self.cfg.layout { 5 } else { 4 };
        let row = (2 * np * pd) as isize;
        let scale = 1.0 / (pd as f32).sqrt();
        let mut pair = vec![0f32; nh * n * n];
        for h in 0..nh {
            // pair_h = q_h k_h^T * scale
            // SAFETY: strided column blocks of qk (n * 2P pd) into pair's n x n block for head h
            unsafe {
                gemm(
                    n,
                    pd,
                    n,
                    scale,
                    qk.as_ptr().add(h * pd),
                    row,
                    1,
                    qk.as_ptr().add(np * pd + h * pd),
                    1,
                    row,
                    0.0,
                    pair.as_mut_ptr().add(h * n * n),
                    n as isize,
                    1,
                );
            }
            // symmetrise
            for i in 0..n {
                for j in i + 1..n {
                    let a = (h * n + i) * n + j;
                    let b = (h * n + j) * n + i;
                    let m = (pair[a] + pair[b]) / 2.0;
                    pair[a] = m;
                    pair[b] = m;
                }
            }
        }
        // + linear term over pairwise geometry
        let ng = self.pair_geo.n_in;
        let mut g = vec![0f32; ng];
        for i in 0..n {
            for j in 0..n {
                self.pair_geometry(feats, pairf, n, i, j, &mut g);
                for h in 0..nh {
                    let w = &self.pair_geo.w[h * ng..(h + 1) * ng];
                    pair[(h * n + i) * n + j] += dot(&g, w) + self.pair_geo.b[h];
                }
            }
        }
        let (cls, n_cls) = match &self.cls {
            Some(c) => (c.forward(&x, n), c.n_out),
            None => (Vec::new(), 0),
        };
        Logits {
            n,
            node,
            pair,
            cls,
            n_cls,
            n_pair: nh,
        }
    }

    /// Pair inputs of tablenet.py `pair_geometry` for (i, j), into `g`.
    fn pair_geometry(&self, f: &[f32], pairf: &[f32], n: usize, i: usize, j: usize, g: &mut [f32]) {
        let fd = self.feat_dim;
        let fi = &f[i * fd..(i + 1) * fd];
        let fj = &f[j * fd..(j + 1) * fd];
        let (cxi, cyi, x0i, x1i, y0i, y1i) = (fi[0], fi[1], fi[4], fi[5], fi[6], fi[7]);
        let (cxj, cyj, x0j, x1j, y0j, y1j) = (fj[0], fj[1], fj[4], fj[5], fj[6], fj[7]);
        let hhi = (y1i - y0i).max(1e-4);
        let hhj = (y1j - y0j).max(1e-4);
        let dy = (cyi - cyj).abs() / hhi;
        let dx = (cxi - cxj).abs() * 10.0;
        let xov = (x1i.min(x1j) - x0i.max(x0j)) * 20.0;
        let yov = (y1i.min(y1j) - y0i.max(y0j)) / hhi;
        let (bi, bj, li, lj) = (fi[fd - 2], fj[fd - 2], fi[fd - 1], fj[fd - 1]);
        let same_blk = if bi == bj && bi >= 0.0 { 1.0 } else { 0.0 };
        let same_leaf = if li == lj && li >= 0.0 { 1.0 } else { 0.0 };
        g[0] = dy.min(20.0) / 20.0;
        g[1] = dx.min(20.0) / 20.0;
        g[2] = xov.clamp(-5.0, 5.0) / 5.0;
        g[3] = yov.clamp(-5.0, 2.0) / 5.0;
        g[4] = (cxi - cxj) * 5.0;
        g[5] = (cyi - cyj) * 5.0;
        g[6] = same_blk;
        g[7] = same_leaf;
        let mut k = 8;
        if self.cfg.stack_feats {
            let hm = hhi.min(hhj);
            let vgap = (y0j - y1i).max(y0i - y1j) / hm;
            let wi = (x1i - x0i).max(1e-4);
            let wj = (x1j - x0j).max(1e-4);
            let xov_frac = (x1i.min(x1j) - x0i.max(x0j)) / wi.min(wj);
            let la = (x0i - x0j).abs() / hm;
            let ra = (x1i - x1j).abs() / hm;
            let ca = ((x0i + x1i) - (x0j + x1j)).abs() / 2.0 / hm;
            g[8] = vgap.clamp(-3.0, 5.0) / 5.0;
            g[9] = xov_frac.clamp(-2.0, 1.0);
            g[10] = la.min(10.0) / 10.0;
            g[11] = ra.min(10.0) / 10.0;
            g[12] = ca.min(10.0) / 10.0;
            k = 13;
        }
        if self.cfg.rule_feats {
            g[k] = pairf[(i * n + j) * 2];
            g[k + 1] = pairf[(i * n + j) * 2 + 1];
        }
    }
}

/// `matrixmultiply::sgemm`'s contract (row / column strides for every operand), dispatched to
/// Apple's Accelerate `cblas_sgemm` on macOS when the layout maps onto BLAS (each input
/// row-major or transposed, the output row-major), else `matrixmultiply`. Accelerate runs on the
/// AMX units: ~several x faster for the encoder and image-branch shapes.
///
/// # Safety
/// As `matrixmultiply::sgemm`: every pointer + stride stays inside its buffer.
#[allow(clippy::too_many_arguments)]
unsafe fn gemm(
    m: usize,
    k: usize,
    n: usize,
    alpha: f32,
    a: *const f32,
    rsa: isize,
    csa: isize,
    b: *const f32,
    rsb: isize,
    csb: isize,
    beta: f32,
    c: *mut f32,
    rsc: isize,
    csc: isize,
) {
    #[cfg(target_os = "macos")]
    if csc == 1 && (csa == 1 || rsa == 1) && (csb == 1 || rsb == 1) {
        let (ta, lda) = if csa == 1 {
            (accelerate::NO_TRANS, rsa)
        } else {
            (accelerate::TRANS, csa)
        };
        let (tb, ldb) = if csb == 1 {
            (accelerate::NO_TRANS, rsb)
        } else {
            (accelerate::TRANS, csb)
        };
        // SAFETY: same operands as the matrixmultiply call; BLAS reads A (m x k), B (k x n) and
        // writes C (m x n) through the given leading dimensions.
        unsafe {
            accelerate::cblas_sgemm(
                accelerate::ROW_MAJOR,
                ta,
                tb,
                m as i32,
                n as i32,
                k as i32,
                alpha,
                a,
                lda.max(1) as i32,
                b,
                ldb.max(1) as i32,
                beta,
                c,
                rsc.max(1) as i32,
            );
        }
        return;
    }
    // SAFETY: forwarded unchanged; the caller upholds matrixmultiply's contract.
    unsafe { matrixmultiply::sgemm(m, k, n, alpha, a, rsa, csa, b, rsb, csb, beta, c, rsc, csc) }
}

#[cfg(target_os = "macos")]
mod accelerate {
    pub const ROW_MAJOR: i32 = 101;
    pub const NO_TRANS: i32 = 111;
    pub const TRANS: i32 = 112;

    #[link(name = "Accelerate", kind = "framework")]
    unsafe extern "C" {
        #[allow(clippy::too_many_arguments)]
        pub fn cblas_sgemm(
            order: i32,
            trans_a: i32,
            trans_b: i32,
            m: i32,
            n: i32,
            k: i32,
            alpha: f32,
            a: *const f32,
            lda: i32,
            b: *const f32,
            ldb: i32,
            beta: f32,
            c: *mut f32,
            ldc: i32,
        );
    }
}
