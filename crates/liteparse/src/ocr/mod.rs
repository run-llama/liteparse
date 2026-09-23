use std::pin::Pin;

use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
pub mod http_simple;
#[cfg(all(feature = "oar-ocr", not(target_arch = "wasm32")))]
pub mod oar;
#[cfg(all(feature = "tesseract", not(target_arch = "wasm32")))]
pub mod tesseract;

/// A single word-level OCR result with bounding box and confidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrResult {
    pub text: String,
    /// Bounding box in pixel coordinates: [x1, y1, x2, y2] (left, top, right, bottom).
    pub bbox: [f32; 4],
    /// Confidence score in 0.0–1.0 range.
    pub confidence: f32,
    /// Optional 4-point polygon of the (possibly rotated) detection,
    /// ordered top-left → top-right → bottom-right → bottom-left in the
    /// glyphs' upright reading frame. When present, allows the projector
    /// to recover orientation for rotated text. None for axis-aligned-only
    /// engines (e.g. Tesseract word boxes).
    pub polygon: Option<[[f32; 2]; 4]>,
}

pub struct OcrOptions {
    pub language: String,
    /// Resolution (pixels per inch) the page image was rendered at. OCR engines
    /// that can't infer DPI from raw pixel buffers (e.g. Tesseract fed RGB bytes)
    /// use this so their internal point-size estimates are correct.
    pub dpi: f32,
}

/// On native targets, `OcrEngine` and its returned futures must be `Send` so
/// they can be moved across thread boundaries by the multi-threaded tokio
/// runtime. On wasm32 there is only a single thread and JS-backed engines
/// hold `!Send` types (`JsValue`, `js_sys::Function`, ...), so we relax
/// those bounds for the wasm target.
#[cfg(not(target_arch = "wasm32"))]
pub trait OcrEngine: Send + Sync {
    fn name(&self) -> &str;
    /// Whether this engine prefers a single-channel grayscale buffer: engines
    /// that binarize internally (Tesseract) do; color-trained engines want RGB.
    fn prefers_grayscale(&self) -> bool {
        false
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
                + Send
                + '_,
        >,
    >;
}

#[cfg(target_arch = "wasm32")]
pub trait OcrEngine: Send + Sync {
    fn name(&self) -> &str;
    /// Whether this engine prefers a single-channel grayscale buffer: engines
    /// that binarize internally (Tesseract) do; color-trained engines want RGB.
    fn prefers_grayscale(&self) -> bool {
        false
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
    >;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyEngine;
    impl OcrEngine for DummyEngine {
        fn name(&self) -> &str {
            "dummy"
        }
        fn recognize<'a, 'b: 'a, 'c: 'a>(
            &'a self,
            _image_data: &'c [u8],
            _width: u32,
            _height: u32,
            options: &'b OcrOptions,
        ) -> Pin<
            Box<
                dyn Future<
                        Output = Result<Vec<OcrResult>, Box<dyn std::error::Error + Send + Sync>>,
                    > + Send
                    + '_,
            >,
        > {
            Box::pin(async move {
                Ok(vec![OcrResult {
                    text: format!("lang={}", options.language),
                    bbox: [0.0, 0.0, 10.0, 10.0],
                    confidence: 0.9,
                    polygon: None,
                }])
            })
        }
    }

    #[tokio::test]
    async fn test_engine_trait_object() {
        let engine: Box<dyn OcrEngine> = Box::new(DummyEngine);
        assert_eq!(engine.name(), "dummy");
        let opts = OcrOptions {
            language: "eng".into(),
            dpi: 150.0,
        };
        let r = engine.recognize(&[], 1, 1, &opts).await.unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].text, "lang=eng");
        assert_eq!(r[0].bbox, [0.0, 0.0, 10.0, 10.0]);
        assert!((r[0].confidence - 0.9).abs() < 1e-6);
    }
}
