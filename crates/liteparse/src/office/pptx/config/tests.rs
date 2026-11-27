use super::*;

#[test]
fn parses_page_ranges() {
    assert_eq!(parse_target_pages("0, 2-4,4, x, 7"), vec![0, 2, 3, 4, 7]);
    assert_eq!(parse_target_pages("5-3"), vec![3, 4, 5]);
    assert!(parse_target_pages("").is_empty());
}

#[test]
fn accepts_worker_pdfium_config() {
    let json = r#"{"verbose":false,"targetPages":"0","maxPages":0,"jpegQuality":90,"pngCompressLevel":4,
        "annotation":{"enabled":true},"path":{"enabled":true,"maxPerPage":100000},
        "text":{"enabled":true,"fragmentedFontMapPath":"/x"},
        "image":{"enabled":true,"maxPerPage":30,"outputDir":"/o"},
        "screenshot":{"enabled":true,"format":"jpg","dpi":200,"maxEdge":1600,"outputDir":"/o","detectRectangles":true}}"#;
    let cfg: Config = serde_json::from_str(json).unwrap();
    assert!(cfg.text.enabled && cfg.screenshot.enabled && cfg.screenshot.detect_rectangles);
    assert_eq!(cfg.png_level(), 4);
    assert!(cfg.presentation.speaker_notes);
}
