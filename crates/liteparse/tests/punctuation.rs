use liteparse::types::PdfInput;
use liteparse::{LiteParse, LiteParseConfig};

#[tokio::test]
async fn punctuation_normalization_can_be_disabled() {
    for (normalize, expected) in [
        (true, "quote 'word' and \"word\"; range 1-2 - done"),
        (false, "quote ‘word’ and “word”; range 1–2 — done"),
    ] {
        // Deserialize the option just as a configuration consumer would. This
        // also exercises the pre-option baseline, which ignores the new key.
        let mut value = serde_json::to_value(LiteParseConfig {
            ocr_enabled: false,
            quiet: true,
            emit_word_boxes: true,
            ..Default::default()
        })
        .unwrap();
        value["normalize_punctuation"] = normalize.into();
        let config = serde_json::from_value(value).unwrap();
        let result = LiteParse::new(config)
            .parse_input(PdfInput::Bytes(
                include_bytes!("fixtures/typographic-punctuation.pdf").to_vec(),
            ))
            .await
            .unwrap();
        let page = &result.pages[0];
        assert_eq!(page.text.trim(), expected, "normalize={normalize}");
        let item_text = page
            .text_items
            .iter()
            .map(|i| i.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(item_text.trim(), expected, "normalize={normalize}");
        let words = page
            .text_items
            .iter()
            .flat_map(|item| &item.words)
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(words, expected, "normalize={normalize}");
    }
}

#[test]
fn older_configs_keep_normalization_enabled() {
    let mut value = serde_json::to_value(LiteParseConfig::default()).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .remove("normalize_punctuation");
    let config: LiteParseConfig = serde_json::from_value(value).unwrap();
    assert!(config.normalize_punctuation);
}
