//! Optional native ONNX OCR backend powered by [`oar-ocr`](https://crates.io/crates/oar-ocr).
//!
//! Models can come from local paths or in-memory bytes. With LiteParse's
//! `oar-ocr-auto-download` feature, registered bare file names are downloaded by
//! `oar-ocr`, SHA-256 verified, and cached under `$OAR_HOME` (default `~/.oar`).
//! Model selection and licensing remain explicit application decisions.

use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Mutex, Once};

pub use oar_ocr::core::ModelSource;
pub use oar_ocr::core::OCRError;
pub use oar_ocr::oarocr::{OAROCR, OAROCRBuilder};

use super::{OcrEngine, OcrOptions, OcrResult};
use crate::error::LiteParseError;

/// Surface `oar-ocr`'s error type as a LiteParse error so the fallible
/// constructors return `LiteParseError` like the rest of the crate.
impl From<OCRError> for LiteParseError {
    fn from(err: OCRError) -> Self {
        LiteParseError::Ocr(err.to_string())
    }
}

/// Source for the recognition character dictionary: a filesystem path or
/// in-memory bytes.
pub enum DictSource {
    /// A path to a character dictionary file. May be a bare registered file
    /// name when the `oar-ocr-auto-download` feature is enabled.
    Path(PathBuf),
    /// In-memory dictionary bytes, decoded as UTF-8 when the engine is built.
    Content(Vec<u8>),
}

impl DictSource {
    /// The path handed to `OAROCRBuilder::new`. For [`DictSource::Content`] this
    /// is an empty placeholder: `character_dict_content` takes precedence in
    /// `oar-ocr`, so the path is never read.
    fn builder_path(&self) -> PathBuf {
        match self {
            DictSource::Path(path) => path.clone(),
            DictSource::Content(_) => PathBuf::new(),
        }
    }
}

impl From<PathBuf> for DictSource {
    fn from(path: PathBuf) -> Self {
        DictSource::Path(path)
    }
}

impl From<&Path> for DictSource {
    fn from(path: &Path) -> Self {
        DictSource::Path(path.to_path_buf())
    }
}

impl From<String> for DictSource {
    fn from(path: String) -> Self {
        DictSource::Path(PathBuf::from(path))
    }
}

impl From<&str> for DictSource {
    fn from(path: &str) -> Self {
        DictSource::Path(PathBuf::from(path))
    }
}

impl From<Vec<u8>> for DictSource {
    fn from(bytes: Vec<u8>) -> Self {
        DictSource::Content(bytes)
    }
}

impl From<&[u8]> for DictSource {
    fn from(bytes: &[u8]) -> Self {
        DictSource::Content(bytes.to_vec())
    }
}

/// Native ONNX OCR adapter for LiteParse.
///
/// [`from_builder`](Self::from_builder) is the recommended constructor. It
/// applies single-image and single-region batches before building the runtime,
/// and page-level calls are serialized through a mutex. Those conservative
/// defaults avoid multiplying inference memory when LiteParse schedules several
/// OCR pages concurrently.
///
/// Advanced callers can use [`from_runtime`](Self::from_runtime) with an
/// already-built runtime, but then own the `oar-ocr` batch-size policy.
pub struct OarOcrEngine {
    runtime: Mutex<OAROCR>,
}

impl OarOcrEngine {
    /// Build an engine from detection, recognition, and dictionary artifacts.
    ///
    /// All three arguments accept local paths or in-memory bytes (models as
    /// [`ModelSource`], the dictionary as [`DictSource`]), so the whole pipeline
    /// can be embedded in the binary or loaded from disk. A dictionary path can
    /// be a bare registered file name when the `oar-ocr-auto-download` feature
    /// is enabled. Use [`from_builder`](Self::from_builder) to configure optional
    /// orientation, rectification, or model-specific settings.
    ///
    /// The recognizer and dictionary must match: a mismatched dictionary
    /// silently produces garbled text rather than an error. Prefer a preset such
    /// as [`ppocr_v6_tiny`](Self::ppocr_v6_tiny) when you want a known-good trio.
    pub fn from_models(
        text_detection_model: impl Into<ModelSource>,
        text_recognition_model: impl Into<ModelSource>,
        character_dict: impl Into<DictSource>,
    ) -> Result<Self, LiteParseError> {
        let dict = character_dict.into();
        let mut builder = OAROCRBuilder::new(
            text_detection_model,
            text_recognition_model,
            dict.builder_path(),
        );
        if let DictSource::Content(bytes) = dict {
            let content = String::from_utf8(bytes).map_err(|err| {
                LiteParseError::Ocr(format!("character dictionary is not valid UTF-8: {err}"))
            })?;
            builder = builder.character_dict_content(content);
        }
        Self::from_builder(builder)
    }

    /// Build the PP-OCRv6 Tiny pipeline, downloading the detection model,
    /// recognition model, and matching dictionary on first use.
    #[cfg(feature = "oar-ocr-auto-download")]
    pub fn ppocr_v6_tiny() -> Result<Self, LiteParseError> {
        Self::from_models(
            "pp-ocrv6_tiny_det.onnx",
            "pp-ocrv6_tiny_rec.onnx",
            "ppocrv6_tiny_dict.txt",
        )
    }

    /// Build the PP-OCRv6 **small** pipeline via auto-download.
    ///
    /// Requires the `oar-ocr-auto-download` feature. Trades throughput for
    /// higher accuracy than [`ppocr_v6_tiny`](Self::ppocr_v6_tiny); the small
    /// recognizer uses the full `ppocrv6_dict.txt`.
    #[cfg(feature = "oar-ocr-auto-download")]
    pub fn ppocr_v6_small() -> Result<Self, LiteParseError> {
        Self::from_models(
            "pp-ocrv6_small_det.onnx",
            "pp-ocrv6_small_rec.onnx",
            "ppocrv6_dict.txt",
        )
    }

    /// Build the PP-OCRv6 **medium** pipeline via auto-download.
    ///
    /// Requires the `oar-ocr-auto-download` feature. The most accurate (and
    /// heaviest) PP-OCRv6 configuration; like [`ppocr_v6_small`](Self::ppocr_v6_small)
    /// it uses the full `ppocrv6_dict.txt`.
    #[cfg(feature = "oar-ocr-auto-download")]
    pub fn ppocr_v6_medium() -> Result<Self, LiteParseError> {
        Self::from_models(
            "pp-ocrv6_medium_det.onnx",
            "pp-ocrv6_medium_rec.onnx",
            "ppocrv6_dict.txt",
        )
    }

    /// Build an engine with conservative batch defaults from a caller-configured
    /// `oar-ocr` pipeline.
    ///
    /// The adapter overrides `image_batch_size` and `region_batch_size` to one.
    /// LiteParse invokes the backend with one rendered page at a time, so image
    /// batching does not improve throughput here. A region batch of one avoids
    /// multiplying temporary recognition tensors on dense pages.
    pub fn from_builder(builder: OAROCRBuilder) -> Result<Self, LiteParseError> {
        let runtime = builder.image_batch_size(1).region_batch_size(1).build()?;
        Ok(Self::from_runtime(runtime))
    }

    /// Wrap an already-built `oar-ocr` runtime.
    ///
    /// Inference calls remain serialized, but the runtime's internal image and
    /// region batch sizes are preserved. Prefer [`from_builder`](Self::from_builder)
    /// unless larger batches have been measured against an explicit memory
    /// budget.
    pub fn from_runtime(runtime: OAROCR) -> Self {
        Self {
            runtime: Mutex::new(runtime),
        }
    }

    fn recognize_sync(
        &self,
        image_data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Vec<OcrResult>, Box<dyn std::error::Error + Send + Sync>> {
        let image = rgb_image(image_data, width, height)?;
        let runtime = self.runtime.lock().map_err(|_| {
            io::Error::other("oar-ocr runtime mutex was poisoned by a previous panic")
        })?;
        let mut predictions = runtime.predict(vec![image])?;
        let prediction = predictions.pop().ok_or_else(|| {
            io::Error::other("oar-ocr returned no prediction for a non-empty image batch")
        })?;

        Ok(prediction
            .text_regions
            .into_iter()
            .filter_map(region_to_result)
            .collect())
    }
}

impl OcrEngine for OarOcrEngine {
    fn name(&self) -> &str {
        "oar-ocr"
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
    > {
        warn_language_ignored_once(&options.language);
        // `ocr_and_merge_rendered` polls this future on a blocking worker, so
        // synchronous ONNX inference does not occupy an async runtime worker.
        Box::pin(async move { self.recognize_sync(image_data, width, height) })
    }
}

static LANGUAGE_IGNORED_WARNING: Once = Once::new();

/// Warn once per process that this backend ignores `OcrOptions::language`.
fn warn_language_ignored_once(language: &str) {
    if language.is_empty() {
        return;
    }
    LANGUAGE_IGNORED_WARNING.call_once(|| {
        eprintln!(
            "[oar-ocr] ignoring OcrOptions::language ({language:?}); recognition language is fixed by the model and character dictionary"
        );
    });
}

fn rgb_image(image_data: &[u8], width: u32, height: u32) -> Result<image::RgbImage, io::Error> {
    if width == 0 || height == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid zero-sized RGB image: {width}x{height}"),
        ));
    }

    let expected_len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("RGB image dimensions overflow: {width}x{height}"),
            )
        })?;

    if image_data.len() != expected_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "invalid RGB buffer length for {width}x{height}: expected {expected_len} bytes, got {}",
                image_data.len()
            ),
        ));
    }

    image::RgbImage::from_raw(width, height, image_data.to_vec()).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("failed to construct RGB image from {width}x{height} buffer"),
        )
    })
}

fn region_to_result(region: oar_ocr::oarocr::TextRegion) -> Option<OcrResult> {
    let text = region.text?.trim().to_owned();
    let confidence = region.confidence?;
    if text.is_empty() || !confidence.is_finite() {
        return None;
    }

    let points = &region.bounding_box.points;
    if points.is_empty()
        || points
            .iter()
            .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return None;
    }

    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for point in points {
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    }

    let polygon = match points.as_slice() {
        [a, b, c, d] => Some([[a.x, a.y], [b.x, b.y], [c.x, c.y], [d.x, d.y]]),
        _ => None,
    };

    Some(OcrResult {
        text,
        bbox: [min_x, min_y, max_x, max_y],
        confidence: confidence.clamp(0.0, 1.0),
        polygon,
        words: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use oar_ocr::oarocr::TextRegion;
    use oar_ocr::processors::{BoundingBox, Point};

    use super::*;

    #[test]
    fn rejects_non_rgb_buffers() {
        let error = rgb_image(&[0; 11], 2, 2).unwrap_err();
        assert!(error.to_string().contains("expected 12 bytes, got 11"));
    }

    #[test]
    fn maps_text_confidence_bbox_and_quad() {
        let points = vec![
            Point::new(10.0, 5.0),
            Point::new(30.0, 7.0),
            Point::new(28.0, 20.0),
            Point::new(8.0, 18.0),
        ];
        let region = TextRegion::with_recognition(
            BoundingBox::new(points),
            Some(Arc::<str>::from(" hello ")),
            Some(1.2),
        );

        let result = region_to_result(region).unwrap();
        assert_eq!(result.text, "hello");
        assert_eq!(result.bbox, [8.0, 5.0, 30.0, 20.0]);
        assert_eq!(result.confidence, 1.0);
        assert_eq!(
            result.polygon,
            Some([[10.0, 5.0], [30.0, 7.0], [28.0, 20.0], [8.0, 18.0]])
        );
    }

    #[test]
    fn dict_source_paths_vs_bytes() {
        // String/path inputs are dictionary paths and reach the builder verbatim.
        assert!(matches!(DictSource::from("dict.txt"), DictSource::Path(_)));
        assert_eq!(
            DictSource::from(PathBuf::from("dict.txt")).builder_path(),
            PathBuf::from("dict.txt")
        );
        // Byte inputs are in-memory content; the builder path is an unused
        // placeholder because `character_dict_content` takes precedence.
        let content = DictSource::from(b"a\nb\n".to_vec());
        assert!(matches!(content, DictSource::Content(_)));
        assert_eq!(content.builder_path(), PathBuf::new());
    }

    #[test]
    fn drops_unrecognized_or_invalid_regions() {
        let box_ = BoundingBox::from_coords(0.0, 0.0, 10.0, 10.0);
        assert!(region_to_result(TextRegion::new(box_.clone())).is_none());
        assert!(
            region_to_result(TextRegion::with_recognition(
                box_.clone(),
                Some(Arc::<str>::from("   ")),
                Some(0.9),
            ))
            .is_none()
        );
        assert!(
            region_to_result(TextRegion::with_recognition(
                box_,
                Some(Arc::<str>::from("text")),
                Some(f32::NAN),
            ))
            .is_none()
        );
    }
}
