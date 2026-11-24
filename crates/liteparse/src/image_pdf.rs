//! Images as one-page PDFs, for parsing an image the way a scanned page is parsed. Pure (no
//! file system), so the WebAssembly build wraps image bytes with it too.

use flate2::Compression;
use flate2::write::ZlibEncoder;
use image::{GenericImageView, ImageFormat};
use resvg::tiny_skia::Pixmap;
use resvg::usvg::{Options, Tree};
use std::io::{self, Write};

use crate::error::LiteParseError;

/// Separates RGB and alpha channels from raw RGBA8 bytes.
/// Used by the `resvg` SVG path where we always have RGBA output.
fn separate_rgb_and_alpha_from_rgba(rgba: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    let mut alpha = Vec::with_capacity(rgba.len() / 4);

    for px in rgba.chunks_exact(4) {
        rgb.push(px[0]);
        rgb.push(px[1]);
        rgb.push(px[2]);
        alpha.push(px[3]);
    }

    (rgb, alpha)
}

/// Zlib-compress a byte slice at a speed/size tradeoff tuned for PDF
/// embedding. Level 3 is ~3× faster than the zlib default (level 6) on
/// photographic data and only ≈3% larger; deflate dominates end-to-end
/// time for any lossily-compressed input (JPEG-except-passthrough, WebP)
/// where the pixel stream is high-entropy.
/// Wrap pixels in a zlib stream for a `/FlateDecode` image. Stored, not compressed: the PDF is a
/// temporary file rendered right away, and compressing a page-sized raster cost more than the
/// rest of the conversion (and PDFium then had to inflate it again).
fn deflate(data: &[u8]) -> io::Result<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(
        Vec::with_capacity(data.len() + data.len() / 64 + 64),
        Compression::none(),
    );
    encoder.write_all(data)?;
    encoder.finish()
}

/// The channel layout of the raster we're going to embed.
enum PixelData {
    /// Interleaved RGB, no alpha. Emitted as a single `/DeviceRGB` XObject.
    Rgb(Vec<u8>),
    /// De-interleaved RGB + 8-bit alpha mask. Emitted as an image XObject
    /// with a separate `/SMask` grayscale XObject.
    RgbWithAlpha { rgb: Vec<u8>, alpha: Vec<u8> },
}

/// PDF's default coordinate system is 150 points per inch. Images with no
/// embedded DPI metadata are assumed to be at this resolution, matching
/// ImageMagick's default behavior when converting images to PDF.
const DEFAULT_IMAGE_DPI: f32 = 150.0;

/// Read the pixel-density (DPI) metadata embedded in an image, if any.
///
/// This mirrors ImageMagick's behavior of sizing the resulting PDF page in
/// points based on the image's physical resolution rather than its pixel
/// dimensions.
///
/// Returns `(dpi_x, dpi_y)` in dots-per-inch, or `None` when the image has
/// no density metadata or the format doesn't carry it. Only PNG and JPEG
/// are inspected; other raster formats fall back to `DEFAULT_IMAGE_DPI`.
fn read_image_dpi(data: &[u8]) -> Option<(f32, f32)> {
    let format = image::guess_format(data).ok()?;
    match format {
        ImageFormat::Png => read_png_dpi(data),
        ImageFormat::Jpeg => {
            // The JPEG JFIF marker stores density and units (0=none/aspect,
            // 1=DPI, 2=dots-per-cm). Parse it directly since `image` doesn't
            // expose this. Scan segments up to the first SOF/EOI.
            let mut i = 0usize;
            if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
                return None;
            }
            i += 2;
            while i + 4 <= data.len() {
                if data[i] != 0xFF {
                    return None;
                }
                let marker = data[i + 1];
                i += 2;
                // Standalone markers with no length field.
                if marker == 0xD8 || marker == 0xD9 || (0xD0..=0xD7).contains(&marker) {
                    continue;
                }
                if i + 2 > data.len() {
                    return None;
                }
                let seg_len = u16::from_be_bytes([data[i], data[i + 1]]) as usize;
                if seg_len < 2 || i + seg_len > data.len() {
                    return None;
                }
                // APP0 with "JFIF\0" identifier carries the density fields.
                if marker == 0xE0 && seg_len >= 16 && &data[i + 2..i + 7] == b"JFIF\0" {
                    let units = data[i + 9];
                    let x_density = u16::from_be_bytes([data[i + 10], data[i + 11]]) as f32;
                    let y_density = u16::from_be_bytes([data[i + 12], data[i + 13]]) as f32;
                    return match units {
                        1 if x_density > 1.0 && y_density > 1.0 => Some((x_density, y_density)),
                        2 if x_density > 1.0 && y_density > 1.0 => {
                            // dots-per-cm → dots-per-inch
                            Some((x_density * 2.54, y_density * 2.54))
                        }
                        _ => None,
                    };
                }
                i += seg_len;
            }
            None
        }
        _ => None,
    }
}

/// Parse the PNG `pHYs` chunk (pixels-per-unit + unit specifier) to derive DPI.
///
/// Layout: 8-byte signature, then repeating chunks of
/// `[len:u32 be][type:4 bytes][data:len bytes][crc:u32]`. We scan for `pHYs`
/// (which must appear before IDAT per spec) and stop at IDAT/IEND. Its 9-byte
/// payload is `ppu_x:u32, ppu_y:u32, unit:u8` where unit 1 = meter and unit 0
/// = unspecified (aspect ratio only, not convertible to DPI).
fn read_png_dpi(data: &[u8]) -> Option<(f32, f32)> {
    const PNG_SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if data.len() < PNG_SIG.len() + 12 || &data[..PNG_SIG.len()] != PNG_SIG {
        return None;
    }
    let mut i = PNG_SIG.len();
    while i + 12 <= data.len() {
        let len = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
        let ty = &data[i + 4..i + 8];
        let body = i + 8;
        if body + len + 4 > data.len() {
            return None;
        }
        if ty == b"pHYs" && len == 9 {
            let ppu_x =
                u32::from_be_bytes([data[body], data[body + 1], data[body + 2], data[body + 3]]);
            let ppu_y = u32::from_be_bytes([
                data[body + 4],
                data[body + 5],
                data[body + 6],
                data[body + 7],
            ]);
            let unit = data[body + 8];
            // Unit 1 = meter; unit 0 = aspect-ratio only (no physical scale).
            if unit != 1 {
                return None;
            }
            // pixels-per-meter → pixels-per-inch (1 inch = 0.0254 m)
            let dpi_x = ppu_x as f32 * 0.0254;
            let dpi_y = ppu_y as f32 * 0.0254;
            if dpi_x > 1.0 && dpi_y > 1.0 {
                return Some((dpi_x, dpi_y));
            }
            return None;
        }
        // pHYs must precede IDAT per spec; stop early once we hit image data.
        if ty == b"IDAT" || ty == b"IEND" {
            return None;
        }
        i = body + len + 4; // skip payload + CRC
    }
    None
}

/// Parse the width/height and channel count out of a JPEG's SOF (Start Of
/// Frame) marker without decoding the pixels. Returns `(width, height,
/// components)` where `components` is 1 (grayscale), 3 (YCbCr/RGB), or 4
/// (CMYK/YCCK).
fn read_jpeg_dimensions(data: &[u8]) -> Option<(u32, u32, u8)> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut i = 2usize;
    while i + 4 <= data.len() {
        if data[i] != 0xFF {
            return None;
        }
        let marker = data[i + 1];
        i += 2;
        // Standalone markers with no length field.
        if marker == 0xD8 || marker == 0xD9 || (0xD0..=0xD7).contains(&marker) {
            continue;
        }
        if i + 2 > data.len() {
            return None;
        }
        let seg_len = u16::from_be_bytes([data[i], data[i + 1]]) as usize;
        if seg_len < 2 || i + seg_len > data.len() {
            return None;
        }
        // SOF0..SOF15 excluding DHT (0xC4), JPG (0xC8), DAC (0xCC): baseline,
        // progressive, and their variants all share this layout.
        let is_sof =
            (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC;
        if is_sof && seg_len >= 8 {
            let height = u16::from_be_bytes([data[i + 3], data[i + 4]]) as u32;
            let width = u16::from_be_bytes([data[i + 5], data[i + 6]]) as u32;
            let components = data[i + 7];
            return Some((width, height, components));
        }
        i += seg_len;
    }
    None
}

/// Rasterizes an SVG file to RGBA8 bytes + dimensions using resvg.
fn rasterize_svg(data: &[u8]) -> Result<(Vec<u8>, u32, u32), LiteParseError> {
    let opt = Options::default();
    let tree =
        Tree::from_data(data, &opt).map_err(|e| LiteParseError::Conversion(e.to_string()))?;

    let size = tree.size();
    let width = size.width().ceil() as u32;
    let height = size.height().ceil() as u32;

    let mut pixmap = Pixmap::new(width.max(1), height.max(1))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "could not read pixmap"))?;

    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );

    // tiny_skia's Pixmap stores premultiplied RGBA; un-premultiply so the
    // PDF's separate RGB/SMask streams composite correctly.
    let mut rgba = pixmap.data().to_vec();
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3];
        if a != 0 && a != 255 {
            px[0] = ((px[0] as u16 * 255) / a as u16) as u8;
            px[1] = ((px[1] as u16 * 255) / a as u16) as u8;
            px[2] = ((px[2] as u16 * 255) / a as u16) as u8;
        }
    }

    Ok((rgba, width, height))
}

/// Describes an image ready to be written into the PDF, in whatever
/// encoding is cheapest for the caller to produce.
struct EmbeddedImage {
    width: u32,
    height: u32,
    dpi_x: f32,
    dpi_y: f32,
    payload: ImagePayload,
}

enum ImagePayload {
    /// JPEG passthrough: the original compressed bytes are copied verbatim
    /// into a `/DCTDecode` XObject. This is what ImageMagick does for JPEG
    /// input and it avoids a full decode + zlib re-encode round-trip.
    Jpeg { bytes: Vec<u8>, components: u8 },
    /// PNG passthrough: the PNG's compressed image data, embedded as `/FlateDecode` with the PNG
    /// predictors (`/Predictor 15`), which decode it exactly as a PNG decoder would.
    Png(PngStream),
    /// Decoded raster, embedded as `/FlateDecode`.
    Flate(PixelData),
}

struct PngStream {
    idat: Vec<u8>,
    /// `/DeviceGray` / `/DeviceRGB`, or an `/Indexed` palette colour space
    color_space: String,
    colors: u8,
}

/// The PNG as a passthrough image stream, when PDF decodes it to the same pixels as the
/// `image` decoder: 8-bit, non-interlaced, grey / RGB / palette, no transparency (`tRNS`
/// would need a soft mask). `None` for anything else, which is decoded instead.
fn png_stream(data: &[u8]) -> Option<(u32, u32, PngStream)> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !data.starts_with(SIG) {
        return None;
    }
    let (mut i, mut ihdr, mut palette, mut idat) = (SIG.len(), None, None, Vec::new());
    while i + 12 <= data.len() {
        let len = u32::from_be_bytes(data[i..i + 4].try_into().ok()?) as usize;
        let kind = &data[i + 4..i + 8];
        let body = data.get(i + 8..i + 8 + len)?;
        match kind {
            b"IHDR" if len == 13 => ihdr = Some(body.to_vec()),
            b"PLTE" => palette = Some(body.to_vec()),
            b"tRNS" => return None,
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        i += 12 + len;
    }
    let h = ihdr?;
    let (width, height) = (
        u32::from_be_bytes(h[0..4].try_into().ok()?),
        u32::from_be_bytes(h[4..8].try_into().ok()?),
    );
    let (bit_depth, color_type, interlace) = (h[8], h[9], h[12]);
    if bit_depth != 8 || interlace != 0 || idat.is_empty() || width == 0 || height == 0 {
        return None;
    }
    let (color_space, colors) = match color_type {
        0 => ("/DeviceGray".to_string(), 1),
        2 => ("/DeviceRGB".to_string(), 3),
        3 => {
            let pal = palette?;
            if pal.is_empty() || pal.len() % 3 != 0 {
                return None;
            }
            let hex: String = pal.iter().map(|b| format!("{b:02x}")).collect();
            (
                format!("[/Indexed /DeviceRGB {} <{hex}>]", pal.len() / 3 - 1),
                1,
            )
        }
        _ => return None,
    };
    Some((
        width,
        height,
        PngStream {
            idat,
            color_space,
            colors,
        },
    ))
}

/// Decode / analyse the input file and produce an `EmbeddedImage`.
fn prepare_image(data: Vec<u8>, is_svg: bool) -> Result<EmbeddedImage, LiteParseError> {
    if is_svg {
        let (rgba, width, height) = rasterize_svg(&data)?;
        let (rgb, alpha) = separate_rgb_and_alpha_from_rgba(&rgba);
        // SVG has no intrinsic DPI — resvg rasterizes at CSS px which the PDF
        // spec treats as equivalent to points (1/72").
        return Ok(EmbeddedImage {
            width,
            height,
            dpi_x: DEFAULT_IMAGE_DPI,
            dpi_y: DEFAULT_IMAGE_DPI,
            payload: ImagePayload::Flate(PixelData::RgbWithAlpha { rgb, alpha }),
        });
    }

    // JPEG fast path: no decode, no re-encode. Parse just enough of the
    // container to fill in the XObject dictionary and hand the compressed
    // bytes to the PDF viewer via `/DCTDecode`.
    if image::guess_format(&data).ok() == Some(ImageFormat::Jpeg)
        && let Some((width, height, components)) = read_jpeg_dimensions(&data)
    {
        // Only pass through color spaces PDF's DCTDecode understands
        // directly. Anything exotic falls through to the decode path.
        if matches!(components, 1 | 3 | 4) {
            let (dpi_x, dpi_y) =
                read_image_dpi(&data).unwrap_or((DEFAULT_IMAGE_DPI, DEFAULT_IMAGE_DPI));
            return Ok(EmbeddedImage {
                width,
                height,
                dpi_x,
                dpi_y,
                payload: ImagePayload::Jpeg {
                    bytes: data,
                    components,
                },
            });
        }
    }

    // General raster path: decode with `image`, then keep only the channels
    // we actually need. Skipping the alpha split when the source has no
    // alpha halves the pixel data we hand to zlib and drops the `/SMask`
    // XObject entirely.
    if let Some((width, height, png)) = png_stream(&data) {
        let (dpi_x, dpi_y) =
            read_image_dpi(&data).unwrap_or((DEFAULT_IMAGE_DPI, DEFAULT_IMAGE_DPI));
        return Ok(EmbeddedImage {
            width,
            height,
            dpi_x,
            dpi_y,
            payload: ImagePayload::Png(png),
        });
    }

    let img = image::load_from_memory(&data)?;
    let (width, height) = img.dimensions();
    let (dpi_x, dpi_y) = read_image_dpi(&data).unwrap_or((DEFAULT_IMAGE_DPI, DEFAULT_IMAGE_DPI));

    let pixels = if img.color().has_alpha() {
        let rgba = img.to_rgba8().into_raw();
        let (rgb, alpha) = separate_rgb_and_alpha_from_rgba(&rgba);
        PixelData::RgbWithAlpha { rgb, alpha }
    } else {
        PixelData::Rgb(img.into_rgb8().into_raw())
    };

    Ok(EmbeddedImage {
        width,
        height,
        dpi_x,
        dpi_y,
        payload: ImagePayload::Flate(pixels),
    })
}

/// An image (PNG, JPEG, TIFF, WebP, BMP, GIF, ... or SVG when `is_svg`) as a one-page PDF whose page
/// is the image at its physical size (its DPI metadata, else 150 dpi). PNG data and JPEG bytes are
/// embedded as they are where PDF can read them; other formats are decoded and stored.
pub fn image_to_pdf(data: Vec<u8>, is_svg: bool) -> Result<Vec<u8>, LiteParseError> {
    let EmbeddedImage {
        width,
        height,
        dpi_x,
        dpi_y,
        payload,
    } = prepare_image(data, is_svg)?;

    // Size the PDF page in points so the embedded image is displayed at its
    // native physical resolution. A 2400×3000 300-DPI scan becomes an
    // 8"×10" (576×720 pt) page rather than a nonsensical 33"×42" one.
    // The image XObject stays at full pixel resolution; only the CTM shrinks.
    let page_width_pt = width as f32 * 72.0 / dpi_x;
    let page_height_pt = height as f32 * 72.0 / dpi_y;

    // Object layout depends on whether we need a separate soft-mask XObject.
    //   With SMask:    1=Pages, 2=Image, 3=SMask, 4=Page, 5=Contents, 6=Catalog  (7 entries)
    //   Without SMask: 1=Pages, 2=Image, 3=Page, 4=Contents, 5=Catalog          (6 entries)
    let image_object_id: u32 = 2;

    let mut pdf_data: Vec<u8> = Vec::with_capacity(1 << 16);
    writeln!(pdf_data, "%PDF-1.4")?;

    // Emit the image XObject(s) and remember their byte offsets for xref.
    let image_object_pos = pdf_data.len();
    let mask_object_pos: Option<usize> = match &payload {
        ImagePayload::Jpeg { bytes, components } => {
            let color_space = match components {
                1 => "/DeviceGray",
                4 => "/DeviceCMYK",
                _ => "/DeviceRGB",
            };
            writeln!(
                pdf_data,
                "{} 0 obj\n<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {} /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>",
                image_object_id,
                width,
                height,
                color_space,
                bytes.len()
            )?;
            writeln!(pdf_data, "stream")?;
            pdf_data.extend(bytes);
            writeln!(pdf_data, "\nendstream\nendobj")?;
            None
        }
        ImagePayload::Png(png) => {
            writeln!(
                pdf_data,
                "{} 0 obj\n<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {} /BitsPerComponent 8 /Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors {} /BitsPerComponent 8 /Columns {} >> /Length {} >>",
                image_object_id,
                width,
                height,
                png.color_space,
                png.colors,
                width,
                png.idat.len()
            )?;
            writeln!(pdf_data, "stream")?;
            pdf_data.extend(&png.idat);
            writeln!(pdf_data, "\nendstream\nendobj")?;
            None
        }
        ImagePayload::Flate(PixelData::Rgb(rgb)) => {
            let rgb_data = deflate(rgb)?;
            writeln!(
                pdf_data,
                "{} 0 obj\n<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>",
                image_object_id,
                width,
                height,
                rgb_data.len()
            )?;
            writeln!(pdf_data, "stream")?;
            pdf_data.extend(&rgb_data);
            writeln!(pdf_data, "endstream\nendobj")?;
            None
        }
        ImagePayload::Flate(PixelData::RgbWithAlpha { rgb, alpha }) => {
            let rgb_data = deflate(rgb)?;
            let mask_data = deflate(alpha)?;
            let mask_object_id = image_object_id + 1;
            writeln!(
                pdf_data,
                "{} 0 obj\n<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} /SMask {} 0 R >>",
                image_object_id,
                width,
                height,
                rgb_data.len(),
                mask_object_id
            )?;
            writeln!(pdf_data, "stream")?;
            pdf_data.extend(&rgb_data);
            writeln!(pdf_data, "endstream\nendobj")?;

            let pos = pdf_data.len();
            writeln!(
                pdf_data,
                "{} 0 obj\n<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>",
                mask_object_id,
                width,
                height,
                mask_data.len()
            )?;
            writeln!(pdf_data, "stream")?;
            pdf_data.extend(&mask_data);
            writeln!(pdf_data, "endstream\nendobj")?;
            Some(pos)
        }
    };

    // Assign the remaining object ids based on whether the SMask was emitted.
    let next_id = if mask_object_pos.is_some() { 4 } else { 3 };
    let page_object_id = next_id;
    let content_stream_object_id = next_id + 1;
    let catalog_object_id = next_id + 2;

    let page_object_pos = pdf_data.len();
    writeln!(
        pdf_data,
        "{} 0 obj\n<< /Type /Page /Parent 1 0 R /MediaBox [0 0 {:.4} {:.4}] /Contents {} 0 R /Resources << /XObject << /Im{} {} 0 R >> >> >>",
        page_object_id,
        page_width_pt,
        page_height_pt,
        content_stream_object_id,
        image_object_id,
        image_object_id
    )?;
    writeln!(pdf_data, "endobj")?;

    let content_stream_pos = pdf_data.len();
    // The CTM scales the 1×1 unit-square image XObject to fill the page in
    // point-space; MediaBox uses the same values, so together they preserve
    // the source image's physical (DPI-aware) size.
    let content = format!(
        "q\n{:.4} 0 0 {:.4} 0 0 cm\n/Im{} Do\nQ",
        page_width_pt, page_height_pt, image_object_id
    );
    writeln!(
        pdf_data,
        "{} 0 obj\n<< /Length {} >>",
        content_stream_object_id,
        content.len()
    )?;
    writeln!(pdf_data, "stream\n{}\nendstream\nendobj", content)?;

    let pages_object_pos = pdf_data.len();
    writeln!(
        pdf_data,
        "1 0 obj\n<< /Type /Pages /Kids [ {} 0 R ] /Count 1 >>",
        page_object_id
    )?;
    writeln!(pdf_data, "endobj")?;

    let catalog_object_pos = pdf_data.len();
    writeln!(
        pdf_data,
        "{} 0 obj\n<< /Type /Catalog /Pages 1 0 R >>",
        catalog_object_id
    )?;
    writeln!(pdf_data, "endobj")?;

    let xref_start = pdf_data.len();
    let total_objects = catalog_object_id + 1; // ids are 1..=catalog_object_id
    writeln!(pdf_data, "xref")?;
    writeln!(pdf_data, "0 {}", total_objects)?;
    writeln!(pdf_data, "0000000000 65535 f ")?;
    writeln!(pdf_data, "{:010} 00000 n ", pages_object_pos)?;
    writeln!(pdf_data, "{:010} 00000 n ", image_object_pos)?;
    if let Some(pos) = mask_object_pos {
        writeln!(pdf_data, "{:010} 00000 n ", pos)?;
    }
    writeln!(pdf_data, "{:010} 00000 n ", page_object_pos)?;
    writeln!(pdf_data, "{:010} 00000 n ", content_stream_pos)?;
    writeln!(pdf_data, "{:010} 00000 n ", catalog_object_pos)?;

    writeln!(
        pdf_data,
        "trailer\n<< /Size {} /Root {} 0 R >>",
        total_objects, catalog_object_id
    )?;
    writeln!(pdf_data, "startxref\n{}", xref_start)?;
    writeln!(pdf_data, "%%EOF")?;

    Ok(pdf_data)
}

/// `data` as PDF bytes: a PDF (its header in the first KB, where PDF readers look) as it is, an
/// image or SVG document wrapped by [`image_to_pdf`], anything else as it is (the PDF reader then
/// says why it can't open it). For byte input that has no file name to go by.
pub fn pdf_or_image_to_pdf(data: Vec<u8>) -> Result<Vec<u8>, LiteParseError> {
    let head = &data[..data.len().min(1024)];
    if head.windows(5).any(|w| w == b"%PDF-") {
        return Ok(data);
    }
    if image::guess_format(&data).is_ok() {
        return image_to_pdf(data, false);
    }
    if is_svg(head) {
        return image_to_pdf(data, true);
    }
    Ok(data)
}

/// An SVG document: `<svg` as the first element, after an optional XML declaration, comments and
/// doctype.
fn is_svg(head: &[u8]) -> bool {
    let text = String::from_utf8_lossy(head);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    (text.starts_with("<?xml")
        || text.starts_with("<!--")
        || text.starts_with("<!DOCTYPE svg")
        || text.starts_with("<svg"))
        && text.contains("<svg")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal in-memory PNG with an optional `pHYs` chunk. Uses the
    /// `image` crate to encode a 1×1 pixel image, then splices the pHYs chunk
    /// in *before* the IDAT chunk (per spec). Returns the resulting bytes.
    fn make_png_with_phys(dpi_x: u32, dpi_y: u32, unit: u8) -> Vec<u8> {
        // Encode a valid 1×1 PNG we can splice into.
        let mut base = Vec::new();
        {
            let img = image::RgbImage::from_pixel(1, 1, image::Rgb([255, 255, 255]));
            image::DynamicImage::ImageRgb8(img)
                .write_to(
                    &mut std::io::Cursor::new(&mut base),
                    image::ImageFormat::Png,
                )
                .unwrap();
        }
        // Build the pHYs chunk: ppu_x, ppu_y (pixels/meter), unit
        let ppu_x = (dpi_x as f32 / 0.0254) as u32;
        let ppu_y = (dpi_y as f32 / 0.0254) as u32;
        let mut payload = Vec::with_capacity(9);
        payload.extend_from_slice(&ppu_x.to_be_bytes());
        payload.extend_from_slice(&ppu_y.to_be_bytes());
        payload.push(unit);
        // CRC covers chunk type + data.
        let mut crc_input = Vec::from(b"pHYs" as &[u8]);
        crc_input.extend_from_slice(&payload);
        let crc = crc32fast::hash(&crc_input);
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        chunk.extend_from_slice(b"pHYs");
        chunk.extend_from_slice(&payload);
        chunk.extend_from_slice(&crc.to_be_bytes());

        // Find IDAT and insert pHYs before it.
        let idat_pos = base
            .windows(4)
            .position(|w| w == b"IDAT")
            .expect("png must have IDAT");
        // The length field is 4 bytes before the chunk type.
        let insert_at = idat_pos - 4;
        let mut out = Vec::with_capacity(base.len() + chunk.len());
        out.extend_from_slice(&base[..insert_at]);
        out.extend_from_slice(&chunk);
        out.extend_from_slice(&base[insert_at..]);
        out
    }

    #[test]
    fn test_read_png_dpi_meters_unit() {
        let png = make_png_with_phys(300, 300, 1);
        let (dx, dy) = read_image_dpi(&png).expect("should read 300 dpi");
        assert!((dx - 300.0).abs() < 0.5, "dx={dx}");
        assert!((dy - 300.0).abs() < 0.5, "dy={dy}");
    }

    #[test]
    fn test_read_png_dpi_unspecified_unit_is_none() {
        // Unit 0 = aspect ratio only, not convertible to DPI.
        let png = make_png_with_phys(300, 300, 0);
        assert!(read_image_dpi(&png).is_none());
    }

    #[test]
    fn test_read_png_dpi_missing_phys_is_none() {
        // Encode a plain PNG without any pHYs chunk.
        let mut buf = Vec::new();
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([0, 0, 0]));
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        assert!(read_image_dpi(&buf).is_none());
    }

    /// Build a minimal JPEG with a JFIF APP0 marker declaring the given density.
    /// The pixel data doesn't need to be valid — our parser stops at the APP0.
    fn make_jpeg_with_jfif(units: u8, x_density: u16, y_density: u16) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&[0xFF, 0xD8]); // SOI
        out.extend_from_slice(&[0xFF, 0xE0]); // APP0 marker
        out.extend_from_slice(&16u16.to_be_bytes()); // segment length
        out.extend_from_slice(b"JFIF\0"); // identifier
        out.extend_from_slice(&[0x01, 0x02]); // version 1.02
        out.push(units); // density units
        out.extend_from_slice(&x_density.to_be_bytes());
        out.extend_from_slice(&y_density.to_be_bytes());
        out.extend_from_slice(&[0, 0]); // thumbnail 0×0
        out.extend_from_slice(&[0xFF, 0xD9]); // EOI
        out
    }

    #[test]
    fn test_read_jpeg_dpi_units_1() {
        let jpg = make_jpeg_with_jfif(1, 300, 300);
        let (dx, dy) = read_image_dpi(&jpg).expect("jfif DPI");
        assert_eq!(dx, 300.0);
        assert_eq!(dy, 300.0);
    }

    #[test]
    fn test_read_jpeg_dpi_units_2_dpcm_to_dpi() {
        // 118 dots/cm ≈ 300 DPI.
        let jpg = make_jpeg_with_jfif(2, 118, 118);
        let (dx, _) = read_image_dpi(&jpg).expect("jfif DPI");
        assert!((dx - 118.0 * 2.54).abs() < 0.1, "dx={dx}");
    }

    #[test]
    fn test_read_jpeg_dpi_units_0_is_none() {
        let jpg = make_jpeg_with_jfif(0, 1, 1);
        assert!(read_image_dpi(&jpg).is_none());
    }

    /// The whole point of the DPI-aware conversion: a 300-DPI source image
    /// must produce a MediaBox in points that reflects its *physical* size
    /// (pixels × 72 / dpi), not its pixel count. This is what matches
    /// ImageMagick's behavior and prevents downstream render upscaling.
    #[test]
    fn test_image_to_pdf_uses_source_dpi_for_mediabox() {
        // 600×300 px @ 300 DPI → expected MediaBox 144×72 pt (2"×1").
        let png = make_png_with_phys(300, 300, 1);
        // Overwrite the encoded 1×1 base image with a real 600×300 one so
        // the reported pixel dimensions match what we want to test.
        let img = image::RgbImage::from_pixel(600, 300, image::Rgb([255, 255, 255]));
        let mut base = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut base),
                image::ImageFormat::Png,
            )
            .unwrap();
        // Re-splice the pHYs from `png` (built above with 300 DPI) into the
        // real-sized PNG.
        let phys_start = png.windows(4).position(|w| w == b"pHYs").unwrap() - 4;
        let phys_end = phys_start + 4 + 4 + 9 + 4; // len+type+payload+crc
        let phys_chunk = &png[phys_start..phys_end];
        let idat_pos = base.windows(4).position(|w| w == b"IDAT").unwrap() - 4;
        let mut spliced = Vec::with_capacity(base.len() + phys_chunk.len());
        spliced.extend_from_slice(&base[..idat_pos]);
        spliced.extend_from_slice(phys_chunk);
        spliced.extend_from_slice(&base[idat_pos..]);

        let pdf = image_to_pdf(spliced, false).unwrap();

        // Expect MediaBox roughly "0 0 144 72" (allowing for fractional formatting).
        let s = String::from_utf8_lossy(&pdf);
        let re = regex::Regex::new(r"/MediaBox\s*\[\s*0\s+0\s+([0-9.]+)\s+([0-9.]+)\s*\]").unwrap();
        let caps = re.captures(&s).expect("MediaBox present");
        let w: f32 = caps[1].parse().unwrap();
        let h: f32 = caps[2].parse().unwrap();
        assert!((w - 144.0).abs() < 0.5, "expected ≈144 pt, got {w}");
        assert!((h - 72.0).abs() < 0.5, "expected ≈72 pt, got {h}");
    }

    #[test]
    fn png_stream_passes_through_plain_8bit_png_only() {
        use image::{ImageBuffer, ImageEncoder, Luma, Rgb, Rgba, codecs::png::PngEncoder};
        let enc = |buf: &[u8], w: u32, h: u32, t: image::ExtendedColorType| {
            let mut out = Vec::new();
            PngEncoder::new(&mut out).write_image(buf, w, h, t).unwrap();
            out
        };
        let rgb: ImageBuffer<Rgb<u8>, _> =
            ImageBuffer::from_fn(7, 5, |x, y| Rgb([x as u8, y as u8, 9]));
        let (w, h, s) =
            png_stream(&enc(rgb.as_raw(), 7, 5, image::ExtendedColorType::Rgb8)).unwrap();
        assert_eq!(
            (w, h, s.colors, s.color_space.as_str()),
            (7, 5, 3, "/DeviceRGB")
        );
        let grey: ImageBuffer<Luma<u8>, _> =
            ImageBuffer::from_fn(3, 3, |x, _| Luma([x as u8 * 80]));
        let (_, _, s) =
            png_stream(&enc(grey.as_raw(), 3, 3, image::ExtendedColorType::L8)).unwrap();
        assert_eq!((s.colors, s.color_space.as_str()), (1, "/DeviceGray"));
        // alpha needs a soft mask: decoded instead
        let rgba: ImageBuffer<Rgba<u8>, _> = ImageBuffer::from_fn(2, 2, |_, _| Rgba([1, 2, 3, 4]));
        assert!(png_stream(&enc(rgba.as_raw(), 2, 2, image::ExtendedColorType::Rgba8)).is_none());
        // 16-bit samples would be rounded differently: decoded instead
        let wide = vec![0u8; 2 * 2 * 3 * 2];
        assert!(png_stream(&enc(&wide, 2, 2, image::ExtendedColorType::Rgb16)).is_none());
        assert!(png_stream(b"not a png").is_none());
    }

    #[test]
    fn test_image_to_pdf_no_dpi_falls_back_to_150() {
        let mut base = Vec::new();
        let img = image::RgbImage::from_pixel(200, 100, image::Rgb([255, 255, 255]));
        image::DynamicImage::ImageRgb8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut base),
                image::ImageFormat::Png,
            )
            .unwrap();
        let pdf = image_to_pdf(base, false).unwrap();
        let s = String::from_utf8_lossy(&pdf);

        // MediaBox should now be pixels * 72 / 150 = pixels * 0.48
        let re = regex::Regex::new(r"/MediaBox\s*\[\s*0\s+0\s+([0-9.]+)\s+([0-9.]+)\s*\]").unwrap();
        let caps = re.captures(&s).expect("MediaBox present");
        let w: f32 = caps[1].parse().unwrap();
        let h: f32 = caps[2].parse().unwrap();

        let expected_w = 200.0 * 72.0 / 150.0; // 96.0
        let expected_h = 100.0 * 72.0 / 150.0; // 48.0
        assert!(
            (w - expected_w).abs() < 0.5,
            "expected {expected_w} pt, got {w}"
        );
        assert!(
            (h - expected_h).abs() < 0.5,
            "expected {expected_h} pt, got {h}"
        );

        // The embedded XObject must still carry the FULL pixel resolution —
        // only the page's physical size (MediaBox) should shrink, not the
        // actual image data written into the stream.
        let xobj_re = regex::Regex::new(
            r"/Type\s*/XObject\s*/Subtype\s*/Image\s*/Width\s+(\d+)\s*/Height\s+(\d+)",
        )
        .unwrap();
        let xobj_caps = xobj_re.captures(&s).expect("XObject present");
        let px_w: u32 = xobj_caps[1].parse().unwrap();
        let px_h: u32 = xobj_caps[2].parse().unwrap();
        assert_eq!(px_w, 200, "XObject width must retain full pixel count");
        assert_eq!(px_h, 100, "XObject height must retain full pixel count");
    }

    #[test]
    fn pdf_or_image_wraps_images_and_svg_and_passes_pdf_through() {
        let pdf = b"%PDF-1.7\n%%EOF\n".to_vec();
        assert_eq!(pdf_or_image_to_pdf(pdf.clone()).unwrap(), pdf);
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(30, 20, image::Rgb([0, 0, 0])))
            .write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        let wrapped = pdf_or_image_to_pdf(png).unwrap();
        assert!(wrapped.starts_with(b"%PDF-"));
        let svg = br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10"/></svg>"#;
        assert!(
            pdf_or_image_to_pdf(svg.to_vec())
                .unwrap()
                .starts_with(b"%PDF-")
        );
        let junk = b"not a document".to_vec();
        assert_eq!(pdf_or_image_to_pdf(junk.clone()).unwrap(), junk);
    }
}
