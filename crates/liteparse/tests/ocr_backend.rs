//! Selection and configuration compatibility; no network or OCR models needed.
use liteparse::config::{MacOcrRecognition, OcrBackend};
use liteparse::{LiteParse, LiteParseConfig};

#[test]
fn old_serialized_configs_keep_the_existing_defaults() {
    let mut json = serde_json::to_value(LiteParseConfig::default()).unwrap();
    let object = json.as_object_mut().unwrap();
    for key in ["ocr_backend", "mac_ocr_languages", "mac_ocr_recognition"] {
        object.remove(key);
    }
    let config: LiteParseConfig = serde_json::from_value(json).unwrap();
    assert_eq!(config.ocr_backend, OcrBackend::Auto);
    assert_eq!(config.mac_ocr_recognition, MacOcrRecognition::Accurate);
    assert!(config.mac_ocr_languages.is_empty());
    assert_eq!(config.ocr_enabled, cfg!(feature = "tesseract"));
}

#[test]
fn auto_still_prefers_the_http_server() {
    let parser = LiteParse::new(LiteParseConfig {
        ocr_enabled: true,
        ocr_server_url: Some("http://127.0.0.1:1/ocr".into()),
        ..Default::default()
    });
    assert_eq!(parser.ocr_engine().unwrap().unwrap().name(), "http-ocr");
}

#[test]
fn auto_keeps_tesseract_as_the_default_builtin() {
    let parser = LiteParse::new(LiteParseConfig {
        ocr_enabled: true,
        ..Default::default()
    });
    #[cfg(feature = "tesseract")]
    assert_eq!(parser.ocr_engine().unwrap().unwrap().name(), "tesseract");
    #[cfg(not(feature = "tesseract"))]
    assert!(
        parser
            .ocr_engine()
            .err()
            .unwrap()
            .to_string()
            .contains("tesseract feature is disabled")
    );
}

#[test]
fn explicit_backends_reject_missing_or_conflicting_server_settings() {
    let parser = LiteParse::new(LiteParseConfig {
        ocr_enabled: true,
        ocr_backend: OcrBackend::Http,
        ..Default::default()
    });
    assert!(
        parser
            .ocr_engine()
            .err()
            .unwrap()
            .to_string()
            .contains("requires ocr_server_url")
    );
    for backend in [OcrBackend::Tesseract, OcrBackend::MacosVision] {
        let parser = LiteParse::new(LiteParseConfig {
            ocr_enabled: true,
            ocr_backend: backend,
            ocr_server_url: Some("http://127.0.0.1:1/ocr".into()),
            ..Default::default()
        });
        assert!(
            parser
                .ocr_engine()
                .err()
                .unwrap()
                .to_string()
                .contains("cannot be used")
        );
    }
}

#[test]
fn disabling_ocr_never_initializes_a_backend() {
    let parser = LiteParse::new(LiteParseConfig {
        ocr_enabled: false,
        ocr_backend: OcrBackend::MacosVision,
        mac_ocr_languages: vec!["invalid".into()],
        ..Default::default()
    });
    assert!(parser.ocr_engine().unwrap().is_none());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn caller_supplied_engine_takes_precedence_over_backend_validation() {
    let engine = std::sync::Arc::new(liteparse::ocr::http_simple::HttpOcrEngine::new(
        "http://127.0.0.1:1/custom".into(),
    ));
    let parser = LiteParse::new(LiteParseConfig {
        ocr_enabled: true,
        ocr_backend: OcrBackend::MacosVision,
        mac_ocr_languages: vec!["invalid".into()],
        ocr_server_url: Some("http://127.0.0.1:1/conflict".into()),
        ..Default::default()
    })
    .with_ocr_engine(engine.clone());
    let selected = parser.ocr_engine().unwrap().unwrap();
    let expected: std::sync::Arc<dyn liteparse::ocr::OcrEngine> = engine;
    assert!(std::sync::Arc::ptr_eq(&selected, &expected));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn requesting_vision_on_other_platforms_reports_the_requirement() {
    let parser = LiteParse::new(LiteParseConfig {
        ocr_enabled: true,
        ocr_backend: OcrBackend::MacosVision,
        ..Default::default()
    });
    assert!(
        parser
            .ocr_engine()
            .err()
            .unwrap()
            .to_string()
            .contains("requires macOS")
    );
}
