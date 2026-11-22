//! One model's inference: [`SessionPool::infer`] runs it on one NCHW `f32` input named `x` and
//! returns the named outputs. Natively a pool of ONNX Runtime sessions; on wasm32 a tract plan
//! (pure Rust, so the engine builds into the WebAssembly module with no runtime to load).
//!
//! Native: `ort`'s `Session::run` takes `&mut self`, so one session serves one call at a time.
//!
//! A call that arrives when the model is otherwise idle (one page, the common `lit parse`
//! case) runs on the "wide" session, which uses the process-wide intra-op pool (every core).
//! Calls that overlap it (pages OCR'd concurrently) take "narrow" sessions with their own two
//! threads, created on demand: concurrent runs on one shared pool mostly queue on each other,
//! while narrow sessions scale with the number of pages.

use std::sync::atomic::AtomicU64;

use super::OcriaError;

#[cfg(not(any(target_arch = "wasm32", feature = "ocria-tract")))]
pub use native::SessionPool;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::open;
// tract: the WebAssembly build, and natively with the `ocria-tract` test feature (WASM parity checks)
#[cfg(any(target_arch = "wasm32", feature = "ocria-tract"))]
pub use wasm::SessionPool;

/// A model's named outputs from one call.
pub struct Outputs(Vec<(String, Output)>);

enum Output {
    F32(Vec<usize>, Vec<f32>),
    I64(Vec<usize>, Vec<i64>),
}

impl Outputs {
    fn take(&mut self, name: &str) -> Result<Output, OcriaError> {
        let i = self
            .0
            .iter()
            .position(|(n, _)| n == name)
            .ok_or_else(|| OcriaError::Model(format!("model has no output `{name}`")))?;
        Ok(self.0.swap_remove(i).1)
    }

    /// The `f32` output `name`: (shape, data).
    pub fn f32(&mut self, name: &str) -> Result<(Vec<usize>, Vec<f32>), OcriaError> {
        match self.take(name)? {
            Output::F32(s, d) => Ok((s, d)),
            Output::I64(..) => Err(OcriaError::Model(format!("output `{name}` is not f32"))),
        }
    }

    /// The `i64` output `name`: (shape, data).
    pub fn i64(&mut self, name: &str) -> Result<(Vec<usize>, Vec<i64>), OcriaError> {
        match self.take(name)? {
            Output::I64(s, d) => Ok((s, d)),
            Output::F32(..) => Err(OcriaError::Model(format!("output `{name}` is not i64"))),
        }
    }
}

/// Cumulative timings of one model's calls (see `SessionPool::stats`).
#[derive(Default)]
pub struct PoolStats {
    pub calls: AtomicU64,
    /// Waiting for a free session (or opening a new one).
    pub wait_us: AtomicU64,
    /// Inside the call: input copy, the model, output extraction.
    pub run_us: AtomicU64,
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Condvar, Mutex};

    use ort::session::Session;
    use web_time::Instant;

    use super::{OcriaError, Output, Outputs, PoolStats};

    /// Intra-op threads of a narrow session.
    const NARROW_THREADS: usize = 2;

    pub struct SessionPool {
        path: PathBuf,
        wide: Mutex<Option<Session>>,
        /// Idle narrow sessions and the number created so far.
        narrow: Mutex<(Vec<Session>, usize)>,
        max_narrow: usize,
        freed: Condvar,
        in_flight: AtomicUsize,
        stats: PoolStats,
    }

    impl SessionPool {
        /// Run the model on `x` (input `x`, NCHW `shape`) and return the outputs `outs`.
        pub fn infer(
            &self,
            shape: [usize; 4],
            x: Vec<f32>,
            outs: &[&str],
        ) -> Result<Outputs, OcriaError> {
            let input = ort::value::Tensor::from_array((shape, x))?;
            self.run(|sess| {
                let res = sess.run(ort::inputs!["x" => input])?;
                let mut v = Vec::with_capacity(outs.len());
                for &name in outs {
                    let o = &res[name];
                    let out = match o.try_extract_tensor::<f32>() {
                        Ok((s, d)) => {
                            Output::F32(s.iter().map(|&n| n as usize).collect(), d.to_vec())
                        }
                        Err(_) => {
                            let (s, d) = o.try_extract_tensor::<i64>()?;
                            Output::I64(s.iter().map(|&n| n as usize).collect(), d.to_vec())
                        }
                    };
                    v.push((name.to_string(), out));
                }
                Ok(Outputs(v))
            })
        }

        /// A pool for the model at `path` (the wide session opens now, so a missing or broken
        /// model fails at load time); at most `max_narrow` narrow sessions.
        pub fn new(path: PathBuf, max_narrow: usize) -> Result<Self, OcriaError> {
            let wide = open(&path, None)?;
            Ok(SessionPool {
                path,
                wide: Mutex::new(Some(wide)),
                narrow: Mutex::new((Vec::new(), 0)),
                max_narrow: max_narrow.max(1),
                freed: Condvar::new(),
                in_flight: AtomicUsize::new(0),
                stats: PoolStats::default(),
            })
        }

        pub fn stats(&self) -> &PoolStats {
            &self.stats
        }

        /// Run `f` with a session to itself.
        fn run<R>(
            &self,
            f: impl FnOnce(&mut Session) -> Result<R, OcriaError>,
        ) -> Result<R, OcriaError> {
            let alone = self.in_flight.fetch_add(1, Ordering::SeqCst) == 0;
            let out = self.run_inner(alone, f);
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            out
        }

        fn run_inner<R>(
            &self,
            alone: bool,
            f: impl FnOnce(&mut Session) -> Result<R, OcriaError>,
        ) -> Result<R, OcriaError> {
            let t0 = Instant::now();
            let timed =
                |session: &mut Session,
                 f: Box<dyn FnOnce(&mut Session) -> Result<R, OcriaError> + '_>| {
                    let t1 = Instant::now();
                    let out = f(session);
                    let st = &self.stats;
                    st.calls.fetch_add(1, Ordering::Relaxed);
                    st.wait_us
                        .fetch_add((t1 - t0).as_micros() as u64, Ordering::Relaxed);
                    st.run_us
                        .fetch_add(t1.elapsed().as_micros() as u64, Ordering::Relaxed);
                    out
                };
            if alone && let Ok(mut slot) = self.wide.try_lock() {
                let mut session = slot.take().expect("wide session");
                let out = timed(&mut session, Box::new(f));
                *slot = Some(session);
                return out;
            }
            let mut session = {
                let mut st = self.narrow.lock().unwrap();
                loop {
                    if let Some(s) = st.0.pop() {
                        break s;
                    }
                    if st.1 < self.max_narrow {
                        st.1 += 1;
                        drop(st);
                        match open(&self.path, Some(NARROW_THREADS)) {
                            Ok(s) => break s,
                            Err(e) => {
                                self.narrow.lock().unwrap().1 -= 1;
                                return Err(e);
                            }
                        }
                    }
                    st = self.freed.wait(st).unwrap();
                }
            };
            let out = timed(&mut session, Box::new(f));
            self.narrow.lock().unwrap().0.push(session);
            self.freed.notify_one();
            out
        }
    }

    /// A session on the shared thread pool (`threads` None) or on its own `threads` threads.
    pub(crate) fn open(
        path: &std::path::Path,
        threads: Option<usize>,
    ) -> Result<Session, OcriaError> {
        if !path.exists() {
            return Err(OcriaError::Model(format!("missing {}", path.display())));
        }
        // No CPU memory arena: each session's arena grew to its peak and kept it (pages of
        // different sizes, lines of different widths), ~2 GB for one 16-core process. Small
        // models allocate little per call, so plain allocation costs about nothing.
        let cpu = ort::ep::CPU::default().with_arena_allocator(false).build();
        let env = crate::ocr::ocria::environment()
            .ok_or_else(|| OcriaError::Model("ONNX Runtime is not loaded".into()))?;
        // CPU only. ONNX Runtime 1.31 selects a device-based EP by default, which on macOS
        // compiles every model for CoreML at session creation (~1 s per process, different numerics).
        let builder = Session::builder(env)?
            .with_auto_device(ort::session::builder::AutoDevicePolicy::PreferCPU)
            .map_err(ort::Error::from)?
            .with_execution_providers([cpu])
            .map_err(ort::Error::from)?;
        let mut builder = match threads {
            None => builder,
            Some(t) => builder
                .with_independent_thread_pool()
                .and_then(|b| b.with_intra_threads(t))
                .and_then(|b| b.with_intra_op_spinning(false))
                .map_err(ort::Error::from)?,
        };
        Ok(builder.commit_from_file(path)?)
    }
}

#[cfg(any(target_arch = "wasm32", feature = "ocria-tract"))]
mod wasm {
    use std::collections::HashMap;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};

    use tract_onnx::prelude::*;
    use web_time::Instant;

    use super::{OcriaError, Output, Outputs, PoolStats};

    type Plan = Arc<TypedRunnableModel>;

    /// One model on tract. Models whose graph tract can optimize for symbolic input dims
    /// (the recognizers and router: any batch, any width) get one plan; the detector's
    /// feature pyramid needs concrete sizes, so it gets one plan per input shape, built on
    /// first use (page sizes are multiples of 32, so few distinct shapes occur).
    pub struct SessionPool {
        model: InferenceModel,
        outputs: Vec<String>,
        symbolic: Option<Plan>,
        fixed: Mutex<HashMap<[usize; 4], Plan>>,
        stats: PoolStats,
    }

    fn err(e: impl std::fmt::Display) -> OcriaError {
        OcriaError::Model(format!("tract: {e}"))
    }

    impl SessionPool {
        /// The ONNX model `bytes` (a broken model fails here, at load time).
        pub fn from_bytes(bytes: &[u8]) -> Result<Self, OcriaError> {
            // exports may declare output / intermediate shapes tract can't reconcile with
            // the graph; let it infer them
            let model = tract_onnx::onnx()
                .with_ignore_output_shapes(true)
                .with_ignore_value_info(true)
                .model_for_read(&mut std::io::Cursor::new(bytes))
                .map_err(err)?;
            let outputs = model
                .output_outlets()
                .map_err(err)?
                .iter()
                .map(|&o| {
                    model
                        .outlet_label(o)
                        .map(str::to_string)
                        .unwrap_or_else(|| model.node(o.node).name.clone())
                })
                .collect();
            let symbolic = model
                .clone()
                .into_optimized()
                .and_then(|m| m.into_runnable())
                .ok();
            Ok(SessionPool {
                model,
                outputs,
                symbolic,
                fixed: Mutex::new(HashMap::new()),
                stats: PoolStats::default(),
            })
        }

        pub fn stats(&self) -> &PoolStats {
            &self.stats
        }

        /// The model at `path` (native testing build, `ocria-tract`).
        #[cfg(not(target_arch = "wasm32"))]
        pub fn new(path: std::path::PathBuf, _max_narrow: usize) -> Result<Self, OcriaError> {
            let bytes = std::fs::read(&path)
                .map_err(|e| OcriaError::Model(format!("{}: {e}", path.display())))?;
            Self::from_bytes(&bytes)
        }

        fn plan(&self, shape: [usize; 4]) -> Result<Plan, OcriaError> {
            if let Some(p) = &self.symbolic {
                return Ok(p.clone());
            }
            let mut fixed = self.fixed.lock().unwrap();
            if let Some(p) = fixed.get(&shape) {
                return Ok(p.clone());
            }
            let p = self
                .model
                .clone()
                .with_input_fact(0, f32::fact(shape).into())
                .and_then(|m| m.into_optimized())
                .and_then(|m| m.into_runnable())
                .map_err(err)?;
            fixed.insert(shape, p.clone());
            Ok(p)
        }

        /// Run the model on `x` (input `x`, NCHW `shape`) and return the outputs `outs`.
        pub fn infer(
            &self,
            shape: [usize; 4],
            x: Vec<f32>,
            outs: &[&str],
        ) -> Result<Outputs, OcriaError> {
            let t0 = Instant::now();
            let plan = self.plan(shape)?;
            let t1 = Instant::now();
            let input = tract_ndarray::ArrayD::from_shape_vec(shape.to_vec(), x).map_err(err)?;
            let res = plan.run(tvec!(input.into_tensor().into())).map_err(err)?;
            let mut v = Vec::with_capacity(outs.len());
            for &name in outs {
                let i =
                    self.outputs.iter().position(|n| n == name).ok_or_else(|| {
                        OcriaError::Model(format!("model has no output `{name}`"))
                    })?;
                let t = &res[i];
                let shape = t.shape().to_vec();
                let out = if t.datum_type() == i64::datum_type() {
                    Output::I64(
                        shape,
                        t.to_plain_array_view::<i64>()
                            .map_err(err)?
                            .iter()
                            .copied()
                            .collect(),
                    )
                } else {
                    Output::F32(
                        shape,
                        t.to_plain_array_view::<f32>()
                            .map_err(err)?
                            .iter()
                            .copied()
                            .collect(),
                    )
                };
                v.push((name.to_string(), out));
            }
            let st = &self.stats;
            st.calls.fetch_add(1, Ordering::Relaxed);
            st.wait_us
                .fetch_add((t1 - t0).as_micros() as u64, Ordering::Relaxed);
            st.run_us
                .fetch_add(t1.elapsed().as_micros() as u64, Ordering::Relaxed);
            Ok(Outputs(v))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Output, Outputs};

    #[test]
    fn outputs_by_name_and_type() {
        let mut o = Outputs(vec![
            ("ids".into(), Output::I64(vec![1, 2], vec![3, 4])),
            ("conf".into(), Output::F32(vec![1, 2], vec![0.5, 0.25])),
        ]);
        assert!(o.f32("ids").is_err(), "wrong type is an error");
        let mut o2 = Outputs(vec![("ids".into(), Output::I64(vec![1, 2], vec![3, 4]))]);
        assert_eq!(o2.i64("ids").unwrap(), (vec![1, 2], vec![3, 4]));
        assert_eq!(o.f32("conf").unwrap(), (vec![1, 2], vec![0.5, 0.25]));
        assert!(o.f32("conf").is_err(), "each output is taken once");
        assert!(o.i64("missing").is_err());
    }
}
