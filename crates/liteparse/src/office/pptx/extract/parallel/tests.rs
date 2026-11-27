use super::*;
use crate::office::pptx::extract::run_read;
use serde_json::Value;

#[test]
fn config_wins_over_environment_and_default() {
    assert_eq!(resolve_workers(Some(3), Some("6"), 8, 100), 3);
    assert_eq!(resolve_workers(None, Some("6"), 8, 100), 6);
    assert_eq!(resolve_workers(None, Some(" 2 "), 8, 100), 2);
    assert_eq!(resolve_workers(None, Some("many"), 8, 100), DEFAULT_WORKERS);
    assert_eq!(resolve_workers(None, None, 8, 100), DEFAULT_WORKERS);
}

#[test]
fn default_is_capped_by_cores_and_any_count_by_pages() {
    assert_eq!(resolve_workers(None, None, 1, 100), 1);
    assert_eq!(resolve_workers(None, None, 2, 100), 2.min(DEFAULT_WORKERS));
    assert_eq!(resolve_workers(Some(16), None, 2, 100), 16);
    assert_eq!(resolve_workers(Some(16), None, 32, 5), 5);
    assert_eq!(resolve_workers(Some(0), None, 8, 5), 1);
    assert_eq!(resolve_workers(None, None, 8, 0), 1);
}

#[test]
fn pictures_are_released_once_every_page_reaching_them_is_built() {
    let set = |parts: &[&str]| {
        parts
            .iter()
            .map(|p| (*p).to_string())
            .collect::<HashSet<_>>()
    };
    let releases = Releases::new(vec![
        set(&["bg.png", "a.png"]),
        set(&["bg.png", "b.png"]),
        set(&["bg.png", "a.png"]),
    ]);
    let store = ImageStore::default();
    let left = |r: &Releases| {
        let s = lock(&r.state);
        let mut done: Vec<String> = s.1.iter().cloned().collect();
        done.sort();
        done
    };
    // Pages finish out of order, as they do across threads.
    releases.page_built(1, &store);
    assert_eq!(left(&releases), vec!["b.png"]);
    releases.page_built(2, &store);
    assert_eq!(left(&releases), vec!["b.png"]);
    releases.page_built(0, &store);
    assert_eq!(left(&releases), vec!["a.png", "b.png", "bg.png"]);
}

#[test]
fn claims_stay_within_the_window_of_the_writer() {
    let store = Arc::new(ImageStore::default());
    let layers = LayerCache::default();
    let cfg: Config = serde_json::from_str("{}").unwrap();
    let pages: Vec<(u32, usize)> = (0..10).map(|p| (p, 0)).collect();
    let weights = vec![1u64; 10];
    let shared = Shared {
        images: &store,
        layers: &layers,
        cfg: &cfg,
        pages: &pages,
        weights: &weights,
        started: Instant::now(),
        window: 4,
        progress: Mutex::new(Progress {
            next: 0,
            written: 0,
            in_flight: 0,
        }),
        changed: Condvar::new(),
        stop: AtomicBool::new(false),
        timed_out: AtomicBool::new(false),
        releases: None,
    };
    let claimed: Vec<usize> = (0..4).filter_map(|_| shared.claim()).collect();
    assert_eq!(claimed, vec![0, 1, 2, 3]);
    std::thread::scope(|s| {
        let waiter = s.spawn(|| shared.claim());
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(!waiter.is_finished());
        lock(&shared.progress).written = 1;
        shared.changed.notify_all();
        assert_eq!(waiter.join().unwrap(), Some(4));
    });
    shared.halt();
    assert_eq!(shared.claim(), None);
}

/// Runs `run_read` on `deck` with `workers` threads; returns the index (minus
/// its RSS) with paths made relative, the pages file and every output file.
fn run(deck: &[u8], workers: u32) -> (Value, String, Vec<(String, Vec<u8>)>) {
    let dir = std::env::temp_dir().join(format!(
        "pptx-read-parallel-{workers}-{}",
        std::process::id()
    ));
    drop(std::fs::remove_dir_all(&dir));
    std::fs::create_dir_all(dir.join("out")).unwrap();
    let input = dir.join("in.pptx");
    std::fs::write(&input, deck).unwrap();
    let out = dir.join("out");
    let cfg = format!(
        r#"{{"workers":{workers},"text":{{"enabled":true}},"path":{{"enabled":true}},
        "screenshot":{{"enabled":true,"dpi":30,"outputDir":"{o}"}},
        "pdf":{{"enabled":true,"outputPath":"{o}/doc.pdf"}}}}"#,
        o = out.display()
    );
    let (index, pages) = (dir.join("index.json"), dir.join("pages.jsonl"));
    run_read(
        &cfg,
        input.to_str().unwrap(),
        index.to_str().unwrap(),
        pages.to_str().unwrap(),
    )
    .unwrap();
    let root = dir.display().to_string();
    let mut v: Value = serde_json::from_str(
        &std::fs::read_to_string(&index)
            .unwrap()
            .replace(&root, "ROOT"),
    )
    .unwrap();
    v.as_object_mut().unwrap().remove("peakRssBytes");
    let pages = std::fs::read_to_string(&pages)
        .unwrap()
        .replace(&root, "ROOT");
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(&out)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect();
    files.sort();
    std::fs::remove_dir_all(&dir).ok();
    (v, pages, files)
}

#[test]
fn threads_write_the_same_output_as_the_sequential_loop() {
    let deck = crate::office::pptx::extract::tests::deck(7, false);
    let one = run(&deck, 1);
    assert_eq!(one.1.lines().count(), 7);
    assert!(one.2.iter().any(|(n, _)| n == "page_7.jpg"));
    for workers in [2, 3, 8] {
        assert_eq!(run(&deck, workers), one, "{workers} workers");
    }
}

#[test]
fn heavy_slides_are_built_one_at_a_time() {
    let store = Arc::new(ImageStore::default());
    let layers = LayerCache::default();
    let cfg: Config = serde_json::from_str("{}").unwrap();
    let pages: Vec<(u32, usize)> = (0..4).map(|p| (p, 0)).collect();
    let big = IN_FLIGHT_BYTES / 2 + 1;
    let weights = vec![big, big, 10, IN_FLIGHT_BYTES * 3];
    let shared = Shared {
        images: &store,
        layers: &layers,
        cfg: &cfg,
        pages: &pages,
        weights: &weights,
        started: Instant::now(),
        window: 8,
        progress: Mutex::new(Progress {
            next: 0,
            written: 0,
            in_flight: 0,
        }),
        changed: Condvar::new(),
        stop: AtomicBool::new(false),
        timed_out: AtomicBool::new(false),
        releases: None,
    };
    assert_eq!(shared.claim(), Some(0));
    std::thread::scope(|s| {
        // Page 1 does not fit next to page 0; it is claimed once page 0 is built.
        let waiter = s.spawn(|| shared.claim());
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(!waiter.is_finished());
        shared.built(0);
        assert_eq!(waiter.join().unwrap(), Some(1));
    });
    assert_eq!(shared.claim(), Some(2));
    shared.built(1);
    shared.built(2);
    // Larger than the whole budget: still built, alone.
    assert_eq!(shared.claim(), Some(3));
    assert_eq!(lock(&shared.progress).in_flight, IN_FLIGHT_BYTES * 3);
}
