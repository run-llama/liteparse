#![cfg(target_arch = "wasm32")]

use std::collections::{HashMap, HashSet};
use std::future::{Future, poll_fn};
use std::pin::{Pin, pin};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use liteparse::ocr::{OcrEngine, OcrOptions, OcrResult};
use liteparse::stages::{self, OcrRaster};
use wasm_bindgen_test::wasm_bindgen_test;

#[derive(Default)]
struct State {
    started: Vec<u32>,
    completed: Vec<u32>,
    released: HashSet<u32>,
    wakers: HashMap<u32, Waker>,
    active: usize,
    peak: usize,
}

#[derive(Default)]
struct ControlledEngine {
    state: Mutex<State>,
}

impl ControlledEngine {
    fn release(&self, page: u32) {
        let waker = {
            let mut state = self.state.lock().unwrap();
            state.released.insert(page);
            state.wakers.remove(&page).expect("job must have started")
        };
        waker.wake();
    }
}

impl OcrEngine for ControlledEngine {
    fn name(&self) -> &str {
        "controlled-stage-test"
    }

    fn recognize<'a, 'b: 'a, 'c: 'a>(
        &'a self,
        image_data: &'c [u8],
        width: u32,
        height: u32,
        options: &'b OcrOptions,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Vec<OcrResult>, Box<dyn std::error::Error + Send + Sync>>>
                + '_,
        >,
    > {
        assert_eq!(image_data, vec![width as u8; width as usize]);
        assert_eq!(height, 1);
        assert_eq!(options.language, "eng");
        assert_eq!(options.dpi, 150.0);
        let mut started = false;
        Box::pin(poll_fn(move |cx| {
            let mut state = self.state.lock().unwrap();
            if !started {
                state.started.push(width);
                state.active += 1;
                state.peak = state.peak.max(state.active);
                started = true;
            }
            if !state.released.contains(&width) {
                state.wakers.insert(width, cx.waker().clone());
                return Poll::Pending;
            }
            state.active -= 1;
            state.completed.push(width);
            Poll::Ready(Ok(vec![OcrResult {
                text: format!("page-{width}"),
                bbox: [0.0, 0.0, 1.0, 1.0],
                confidence: 0.99,
                polygon: None,
            }]))
        }))
    }
}

fn check_recognition(num_workers: usize) {
    // Use page numbers that differ from input order.
    let page_numbers = [9, 3, 7];
    let rasters = page_numbers
        .iter()
        .map(|&page_number| OcrRaster {
            page_number,
            pixels: vec![page_number as u8; page_number],
            width: page_number as u32,
            height: 1,
            dpi: 150.0,
            has_native_text: false,
            image_rects: vec![],
        })
        .collect();
    let engine = Arc::new(ControlledEngine::default());
    let mut recognition = pin!(stages::recognize(
        rasters,
        engine.clone(),
        "eng",
        num_workers
    ));
    let mut cx = Context::from_waker(Waker::noop());
    let limit = num_workers.clamp(1, page_numbers.len());

    assert!(recognition.as_mut().poll(&mut cx).is_pending());
    assert_eq!(engine.state.lock().unwrap().started.len(), limit);

    for completed in 1..=page_numbers.len() {
        // Finish the most recently started active job first.
        // With multiple active jobs, keep earlier jobs pending while
        // later jobs complete.
        let page = {
            let state = engine.state.lock().unwrap();
            *state
                .started
                .iter()
                .rev()
                .find(|page| !state.completed.contains(page))
                .unwrap()
        };
        engine.release(page);
        let result = recognition.as_mut().poll(&mut cx);
        let state = engine.state.lock().unwrap();
        assert_eq!(state.completed.len(), completed);
        assert_eq!(
            state.started.len(),
            (limit + completed).min(page_numbers.len())
        );
        assert_eq!(state.peak, limit);
        if completed < page_numbers.len() {
            assert!(
                result.is_pending(),
                "stage returned before all jobs finished"
            );
            assert_eq!(state.active, limit.min(page_numbers.len() - completed));
        } else {
            let Poll::Ready(outcomes) = result else {
                panic!("stage did not return after all jobs finished");
            };
            assert_eq!(state.active, 0);
            if limit > 1 {
                assert_ne!(state.completed, vec![9, 3, 7]);
            }
            assert_eq!(outcomes.len(), page_numbers.len());
            for (outcome, page_number) in outcomes.iter().zip(page_numbers) {
                assert_eq!(outcome.page_number, page_number);
                assert!(outcome.error.is_none());
                assert_eq!(outcome.results.len(), 1);
                assert_eq!(outcome.results[0].text, format!("page-{page_number}"));
            }
        }
    }
}

#[wasm_bindgen_test]
fn bounded_concurrency_refills_and_preserves_input_order() {
    check_recognition(2);
}

#[wasm_bindgen_test]
fn zero_workers_uses_one_slot() {
    check_recognition(0);
}

#[wasm_bindgen_test]
fn one_worker_is_sequential() {
    check_recognition(1);
}

#[wasm_bindgen_test]
fn worker_limit_can_exceed_raster_count() {
    check_recognition(8);
}

#[wasm_bindgen_test]
fn window_collects_ready_results_and_reuses_all_slots() {
    let engine = Arc::new(ControlledEngine::default());
    let mut window = stages::OcrWindow::new(engine.clone(), "eng", 2);
    let mut cx = Context::from_waker(Waker::noop());
    for page in [9, 3] {
        let raster = OcrRaster {
            page_number: page,
            pixels: vec![page as u8; page],
            width: page as u32,
            height: 1,
            dpi: 150.0,
            has_native_text: false,
            image_rects: vec![],
        };
        assert!(
            pin!(window.submit(raster))
                .as_mut()
                .poll(&mut cx)
                .is_ready()
        );
    }
    window.complete_ready();
    assert_eq!(window.available_capacity(), 0);
    assert!(window.take_completed().is_empty());
    engine.release(3);
    engine.release(9);
    window.complete_ready();
    assert_eq!(window.available_capacity(), 2);
    let outcomes = window.take_completed();
    assert_eq!(
        outcomes.iter().map(|o| o.page_number).collect::<Vec<_>>(),
        vec![9, 3]
    );
    assert!(window.take_completed().is_empty());
    let raster = OcrRaster {
        page_number: 7,
        pixels: vec![7; 7],
        width: 7,
        height: 1,
        dpi: 150.0,
        has_native_text: false,
        image_rects: vec![],
    };
    assert!(
        pin!(window.submit(raster))
            .as_mut()
            .poll(&mut cx)
            .is_ready()
    );
    window.complete_ready();
    engine.release(7);
    let mut finish = pin!(window.finish());
    let Poll::Ready(remaining) = finish.as_mut().poll(&mut cx) else {
        panic!("released job must finish");
    };
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].page_number, 7);
}
