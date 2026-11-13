use std::borrow::Cow;

use image::ImageDecoder;
use serde::{Deserialize, Serialize};

use crate::error::LiteParseError;

/// File encoding accepted by OCR engines that receive compressed images.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OcrImageFormat {
    Jpeg,
    Png,
}

impl OcrImageFormat {
    pub fn mime_type(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
        }
    }

    pub fn file_name(self) -> &'static str {
        match self {
            Self::Jpeg => "image.jpg",
            Self::Png => "image.png",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodedOcrImage {
    #[serde(with = "base64_bytes")]
    pub data: Vec<u8>,
    pub format: OcrImageFormat,
}

/// One OCR image, with exactly one representation. Consumers borrow matching
/// data and convert only when they need another representation. Pixel edits
/// consume the image into pixels so stale encoded bytes cannot survive them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OcrImage {
    Pixels {
        #[serde(with = "base64_bytes")]
        data: Vec<u8>,
        width: u32,
        height: u32,
        /// One byte per pixel when true, three RGB bytes otherwise.
        grayscale: bool,
    },
    Encoded {
        image: EncodedOcrImage,
        width: u32,
        height: u32,
    },
}

impl OcrImage {
    pub fn dimensions(&self) -> (u32, u32) {
        match self {
            Self::Pixels { width, height, .. } | Self::Encoded { width, height, .. } => {
                (*width, *height)
            }
        }
    }

    /// Read image headers and retain JPEG/PNG bytes when no normalization is
    /// needed. Alpha, higher bit depths and EXIF orientation are normalized to
    /// RGB pixels, preserving the raw pixel frame used for page coordinates.
    pub fn from_file(path: &str) -> Result<Self, LiteParseError> {
        let bytes = std::fs::read(path)?;
        let reader = image::ImageReader::new(std::io::Cursor::new(bytes.as_slice()))
            .with_guessed_format()?;
        let format = match reader.format() {
            Some(image::ImageFormat::Jpeg) => Some(OcrImageFormat::Jpeg),
            Some(image::ImageFormat::Png) => Some(OcrImageFormat::Png),
            _ => None,
        };
        let mut decoder = reader.into_decoder()?;
        let (width, height) = decoder.dimensions();
        let mut limits = image::Limits::default();
        limits.reserve(decoder.total_bytes())?;
        decoder.set_limits(limits)?;
        if matches!(
            decoder.color_type(),
            image::ColorType::Rgb8 | image::ColorType::L8
        ) && let Some(format) = format
            && decoder.orientation()? == image::metadata::Orientation::NoTransforms
        {
            drop(decoder);
            return Ok(Self::Encoded {
                image: EncodedOcrImage {
                    data: bytes,
                    format,
                },
                width,
                height,
            });
        }
        Ok(Self::Pixels {
            data: decoded_pixels(image::DynamicImage::from_decoder(decoder)?, false),
            width,
            height,
            grayscale: false,
        })
    }

    pub fn pixels(&self, grayscale: bool) -> Result<Cow<'_, [u8]>, LiteParseError> {
        match self {
            Self::Pixels {
                data,
                grayscale: source,
                ..
            } if *source == grayscale => Ok(Cow::Borrowed(data)),
            Self::Pixels {
                data,
                grayscale: true,
                ..
            } => Ok(Cow::Owned(data.iter().flat_map(|v| [*v; 3]).collect())),
            Self::Pixels { data, .. } => {
                Ok(Cow::Owned(data.chunks_exact(3).map(rgb_luma).collect()))
            }
            Self::Encoded { image, .. } => Ok(Cow::Owned(decoded_pixels(
                image::load_from_memory(&image.data)?,
                grayscale,
            ))),
        }
    }

    pub fn into_pixels(self, grayscale: bool) -> Result<Vec<u8>, LiteParseError> {
        match self {
            Self::Pixels {
                data,
                grayscale: source,
                ..
            } if source == grayscale => Ok(data),
            image => Ok(image.pixels(grayscale)?.into_owned()),
        }
    }

    pub fn encoded(&self) -> Result<Cow<'_, EncodedOcrImage>, LiteParseError> {
        match self {
            Self::Encoded { image, .. } => Ok(Cow::Borrowed(image)),
            Self::Pixels {
                data,
                width,
                height,
                ..
            } => Ok(Cow::Owned(EncodedOcrImage {
                data: crate::extract::encode_pixels_png(data, *width, *height)?,
                format: OcrImageFormat::Png,
            })),
        }
    }
}

fn rgb_luma(rgb: &[u8]) -> u8 {
    // Match PDF rendering's integer Rec. 601 conversion.
    ((77 * u32::from(rgb[0]) + 150 * u32::from(rgb[1]) + 29 * u32::from(rgb[2])) >> 8) as u8
}

fn decoded_pixels(image: image::DynamicImage, grayscale: bool) -> Vec<u8> {
    match (image, grayscale) {
        (image::DynamicImage::ImageLuma8(image), true) => image.into_raw(),
        (image, grayscale) if !image.color().has_alpha() => {
            let rgb = image.into_rgb8();
            if grayscale {
                rgb.pixels().map(|pixel| rgb_luma(&pixel.0)).collect()
            } else {
                rgb.into_raw()
            }
        }
        (image, grayscale) => {
            let rgba = image.into_rgba8();
            let mut pixels = Vec::with_capacity(
                rgba.width() as usize * rgba.height() as usize * if grayscale { 1 } else { 3 },
            );
            for pixel in rgba.pixels() {
                let alpha = u32::from(pixel[3]);
                let rgb = [pixel[0], pixel[1], pixel[2]]
                    .map(|c| ((u32::from(c) * alpha + 255 * (255 - alpha) + 127) / 255) as u8);
                if grayscale {
                    pixels.push(rgb_luma(&rgb));
                } else {
                    pixels.extend_from_slice(&rgb);
                }
            }
            pixels
        }
    }
}

mod base64_bytes {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        STANDARD.decode(encoded).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_images_are_materialized_on_white_before_rgb_or_grayscale_ocr() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("alpha.png");
        image::RgbaImage::from_raw(3, 1, vec![0, 0, 255, 0, 0, 0, 0, 128, 0, 0, 255, 255])
            .unwrap()
            .save(&path)
            .unwrap();
        let image = OcrImage::from_file(path.to_str().unwrap()).unwrap();
        assert!(matches!(image, OcrImage::Pixels { .. }));
        assert_eq!(
            image.pixels(false).unwrap().as_ref(),
            [255, 255, 255, 127, 127, 127, 0, 0, 255]
        );
        assert_eq!(image.pixels(true).unwrap().as_ref(), [255, 127, 28]);
    }
}
