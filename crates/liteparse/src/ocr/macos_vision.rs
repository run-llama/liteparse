//! Native Apple Vision OCR. No Python runtime, external command, or model
//! download is required. All Objective-C objects are local to one call so
//! the engine can be shared by LiteParse's bounded OCR worker pool.

use std::{ops::Range, pin::Pin};

use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};
use objc2::{AnyThread, available, msg_send, rc::Retained, rc::autoreleasepool};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSError, NSRange, NSString};
use objc2_vision::{
    VNImageRequestHandler, VNRecognizeTextRequest, VNRecognizedText, VNRectangleObservation,
    VNRequest, VNRequestTextRecognitionLevel,
};
use unicode_segmentation::UnicodeSegmentation;

use super::{OcrEngine, OcrOptions, OcrResult};
use crate::{config::MacOcrRecognition, error::LiteParseError};

/// An opt-in macOS 10.15+ OCR engine using the system's Vision framework.
///
/// Languages are Vision tags such as `en-US` or `zh-Hans`, in priority order.
/// An empty list enables automatic detection on macOS 13+, and uses Vision's
/// default recognition languages on older systems. `OcrOptions::language`
/// is not used: it carries LiteParse's separate Tesseract language setting.
pub struct MacOcrEngine {
    languages: Vec<String>,
    recognition: MacOcrRecognition,
}

impl MacOcrEngine {
    pub fn new(
        languages: Vec<String>,
        recognition: MacOcrRecognition,
    ) -> Result<Self, LiteParseError> {
        let supported = Self::supported_languages(recognition)?;
        let mut resolved = Vec::with_capacity(languages.len());
        for language in languages {
            let Some(native) = supported
                .iter()
                .find(|native| native.eq_ignore_ascii_case(&language))
            else {
                return Err(LiteParseError::Config(format!(
                    "macos-vision does not support language {language:?} with {} recognition on this macOS version; supported languages: {}",
                    recognition.as_str(),
                    supported.join(", ")
                )));
            };
            if !resolved.contains(native) {
                resolved.push(native.clone());
            }
        }
        Ok(Self {
            languages: resolved,
            recognition,
        })
    }

    /// Query the running OS instead of maintaining an OS-dependent language list.
    pub fn supported_languages(
        recognition: MacOcrRecognition,
    ) -> Result<Vec<String>, LiteParseError> {
        if !available!(macos = 10.15) {
            return Err(LiteParseError::Config(
                "the macos-vision OCR backend requires macOS 10.15 or later".into(),
            ));
        }
        autoreleasepool(|_| {
            let request = Self::request(recognition, &[]);
            let languages = if available!(macos = 12.0) {
                // SAFETY: The instance API is available starting in macOS 12,
                // and this request is local to the current thread.
                unsafe { request.supportedRecognitionLanguagesAndReturnError() }
            } else {
                legacy_supported_languages(&request)
            }
            .map_err(|error| vision_error("query supported languages", &error))?;
            let languages: Vec<_> = languages
                .iter()
                .map(|language| language.to_string())
                .collect();
            if languages.is_empty() {
                return Err(LiteParseError::Ocr(
                    "macos-vision reported no supported recognition languages".into(),
                ));
            }
            Ok(languages)
        })
    }

    fn request(
        recognition: MacOcrRecognition,
        languages: &[String],
    ) -> Retained<VNRecognizeTextRequest> {
        let request = VNRecognizeTextRequest::new();
        request.setRecognitionLevel(match recognition {
            MacOcrRecognition::Accurate => VNRequestTextRecognitionLevel::Accurate,
            MacOcrRecognition::Fast => VNRequestTextRecognitionLevel::Fast,
        });
        // Set the level before languages: language availability depends on it.
        if languages.is_empty() {
            if available!(macos = 13.0) {
                request.setAutomaticallyDetectsLanguage(true);
            }
        } else {
            let languages: Vec<_> = languages
                .iter()
                .map(|tag| NSString::from_str(tag))
                .collect();
            request.setRecognitionLanguages(&NSArray::from_retained_slice(&languages));
        }
        request
    }

    fn recognize_pixels(
        &self,
        pixels: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Vec<OcrResult>, LiteParseError> {
        let pixel_count = (width as usize)
            .checked_mul(height as usize)
            .filter(|&count| count > 0)
            .ok_or_else(|| {
                LiteParseError::Ocr("macos-vision requires nonzero image dimensions".into())
            })?;
        let color = if pixels.len() == pixel_count {
            ExtendedColorType::L8
        } else if pixel_count.checked_mul(3) == Some(pixels.len()) {
            ExtendedColorType::Rgb8
        } else {
            return Err(LiteParseError::Ocr(format!(
                "macos-vision expected a packed grayscale or RGB image ({width}x{height}), got {} bytes",
                pixels.len()
            )));
        };
        // Vision accepts encoded image data. Use an in-memory PNG so buffer
        // ownership and pixel format remain explicit across the FFI boundary.
        let mut encoded = Vec::new();
        PngEncoder::new(&mut encoded).write_image(pixels, width, height, color)?;

        autoreleasepool(|_| {
            let request = Self::request(self.recognition, &self.languages);
            let handler = VNImageRequestHandler::initWithData_options(
                VNImageRequestHandler::alloc(),
                &NSData::with_bytes(&encoded),
                &NSDictionary::new(),
            );
            let requests = NSArray::from_slice(&[&*request as &VNRequest]);
            handler
                .performRequests_error(&requests)
                .map_err(|error| vision_error("recognize image", &error))?;
            let Some(observations) = request.results() else {
                return Ok(Vec::new());
            };
            let mut results: Vec<OcrResult> = Vec::new();
            for observation in observations {
                let candidates = observation.topCandidates(1);
                let Some(candidate) = candidates.firstObject() else {
                    continue;
                };
                let text = candidate.string().to_string();
                let confidence = candidate.confidence();
                if !confidence.is_finite() {
                    return Err(LiteParseError::Ocr(
                        "macos-vision returned invalid confidence".into(),
                    ));
                }
                let mut previous_end = None;
                for (bytes, range) in word_ranges(&text) {
                    let rectangle = word_rectangle(&candidate, range)?;
                    // SAFETY: The rectangle is a live Vision result, used on
                    // the same thread as its request. No ObjC values escape.
                    let rect = unsafe { rectangle.boundingBox() };
                    let bbox = pixel_bbox(
                        [
                            rect.origin.x,
                            rect.origin.y,
                            rect.size.width,
                            rect.size.height,
                        ],
                        width,
                        height,
                    )?;
                    // Accurate recognition can return the same word box for
                    // multiple Unicode segments (e.g. CJK or hyphenated text).
                    // Combine contiguous segments sharing that geometry.
                    if previous_end == Some(bytes.start)
                        && let Some(previous) = results.last_mut().filter(|previous| {
                            previous
                                .bbox
                                .iter()
                                .zip(bbox)
                                .all(|(a, b)| (a - b).abs() < 0.5)
                        })
                    {
                        previous.text.push_str(&text[bytes.clone()]);
                    } else {
                        results.push(OcrResult {
                            text: text[bytes.clone()].to_string(),
                            bbox,
                            confidence: confidence.clamp(0.0, 1.0),
                            polygon: None,
                        });
                    }
                    previous_end = Some(bytes.end);
                }
            }
            Ok(results)
        })
    }
}

impl OcrEngine for MacOcrEngine {
    fn name(&self) -> &str {
        "macos-vision"
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
        // The pipeline already executes native recognition on a blocking
        // worker. Construct and release all Vision objects within that call.
        Box::pin(async move {
            self.recognize_pixels(image_data, width, height)
                .map_err(Into::into)
        })
    }
}

fn vision_error(action: &str, error: &NSError) -> LiteParseError {
    LiteParseError::Ocr(format!("macos-vision failed to {action}: {error}"))
}

// The instance language-query API was added in macOS 12. Keep the class API
// for macOS 10.15/11 without invoking an unavailable selector on those systems.
#[allow(deprecated)]
fn legacy_supported_languages(
    request: &VNRecognizeTextRequest,
) -> Result<Retained<NSArray<NSString>>, Retained<NSError>> {
    // SAFETY: Both this class method and the request's revision accessor are
    // available on every OS supporting VNRecognizeTextRequest (macOS 10.15+).
    unsafe {
        VNRecognizeTextRequest::supportedRecognitionLanguagesForTextRecognitionLevel_revision_error(
            request.recognitionLevel(),
            request.revision(),
        )
    }
}

fn word_rectangle(
    candidate: &VNRecognizedText,
    range: NSRange,
) -> Result<Retained<VNRectangleObservation>, LiteParseError> {
    let mut error: Option<Retained<NSError>> = None;
    // SAFETY: This documented selector is omitted by objc2-vision 0.3's
    // generated bindings. NSRange uses UTF-16 offsets into candidate.string;
    // word_ranges produces in-bounds ranges at Unicode scalar boundaries.
    // The out parameter is retained by msg_send's writeback support.
    let rectangle: Option<Retained<VNRectangleObservation>> =
        unsafe { msg_send![candidate, boundingBoxForRange: range, error: &mut error] };
    if let Some(error) = error {
        return Err(vision_error("locate recognized word", &error));
    }
    rectangle.ok_or_else(|| {
        LiteParseError::Ocr("macos-vision returned no bounding box for a recognized word".into())
    })
}

/// Word segmentation includes punctuation and uses UTF-16 indices for Vision.
/// Unlike split_whitespace alone, Unicode word boundaries also segment CJK.
fn word_ranges(text: &str) -> Vec<(Range<usize>, NSRange)> {
    let mut ranges = Vec::new();
    let mut byte_offset = 0;
    let mut utf16_offset = 0;
    for piece in text.split_inclusive(char::is_whitespace) {
        let chunk = piece.trim_end_matches(char::is_whitespace);
        if !chunk.is_empty() {
            let mut boundaries = chunk.unicode_word_indices().map(|(start, _)| start);
            boundaries.next(); // Include any leading punctuation in the first word.
            let mut start = 0;
            for end in boundaries.chain(std::iter::once(chunk.len())) {
                let length = chunk[start..end].encode_utf16().count();
                ranges.push((
                    byte_offset + start..byte_offset + end,
                    NSRange::new(utf16_offset, length),
                ));
                utf16_offset += length;
                start = end;
            }
        }
        utf16_offset += piece[chunk.len()..].encode_utf16().count();
        byte_offset += piece.len();
    }
    ranges
}

fn pixel_bbox(rect: [f64; 4], width: u32, height: u32) -> Result<[f32; 4], LiteParseError> {
    let [x, y, w, h] = rect;
    if !rect.iter().all(|coordinate| coordinate.is_finite()) || w <= 0.0 || h <= 0.0 {
        return Err(LiteParseError::Ocr(
            "macos-vision returned an invalid word bounding box".into(),
        ));
    }
    // Vision uses normalized coordinates with a lower-left origin. LiteParse
    // expects raster pixels with a top-left origin; its merge handles DPI.
    let bbox = [
        (x.clamp(0.0, 1.0) * width as f64) as f32,
        ((1.0 - y - h).clamp(0.0, 1.0) * height as f64) as f32,
        ((x + w).clamp(0.0, 1.0) * width as f64) as f32,
        ((1.0 - y).clamp(0.0, 1.0) * height as f64) as f32,
    ];
    if bbox[0] >= bbox[2] || bbox[1] >= bbox[3] {
        return Err(LiteParseError::Ocr(
            "macos-vision returned a word outside the image".into(),
        ));
    }
    Ok(bbox)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_language_query_remains_usable() {
        autoreleasepool(|_| {
            for recognition in [MacOcrRecognition::Accurate, MacOcrRecognition::Fast] {
                let request = MacOcrEngine::request(recognition, &[]);
                let supported = legacy_supported_languages(&request).unwrap();
                assert!(
                    supported
                        .iter()
                        .any(|language| language.to_string() == "en-US")
                );
            }
        });
    }

    #[test]
    fn ranges_preserve_punctuation_and_use_utf16_offsets() {
        let text = "(Café), 😀résumé! 中文";
        let ranges = word_ranges(text);
        let words: Vec<_> = ranges
            .iter()
            .map(|(bytes, _)| &text[bytes.clone()])
            .collect();
        assert_eq!(words, ["(Café),", "😀résumé!", "中", "文"]);
        for (bytes, range) in ranges {
            assert_eq!(range.location, text[..bytes.start].encode_utf16().count());
            assert_eq!(range.length, text[bytes].encode_utf16().count());
        }
    }

    #[test]
    fn converts_normalized_bottom_left_to_pixel_top_left() {
        assert_eq!(
            pixel_bbox([0.1, 0.7, 0.2, 0.1], 1000, 500).unwrap(),
            [100.0, 100.0, 300.0, 150.0]
        );
        assert!(pixel_bbox([f64::NAN, 0.0, 1.0, 1.0], 100, 100).is_err());
        assert!(pixel_bbox([0.0, 0.0, 0.0, 1.0], 100, 100).is_err());
        assert!(pixel_bbox([2.0, 0.0, 1.0, 1.0], 100, 100).is_err());
    }
}
