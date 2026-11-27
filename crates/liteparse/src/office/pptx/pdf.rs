//! The screenshot-only PDF: one page per emitted slide, sized to the slide in
//! points, drawing a single full-page raster. JPEG rasters are embedded as-is
//! (`DCTDecode`); PNG rasters are re-encoded as Flate RGB.

use std::io::Write;

/// One page: its size in points and a raster file on disk.
pub struct PdfPage {
    pub width_pt: f64,
    pub height_pt: f64,
    pub image_path: String,
}

struct Raster {
    data: Vec<u8>,
    width: u32,
    height: u32,
    filter: &'static str,
}

fn load_raster(path: &str) -> std::io::Result<Raster> {
    let bytes = std::fs::read(path)?;
    if bytes.starts_with(&[0xFF, 0xD8]) {
        // The frame header gives the size without decoding the scan data.
        let (width, height) = match jpeg_size(&bytes) {
            Some(size) => size,
            None => {
                let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
                    .map_err(|e| std::io::Error::other(e.to_string()))?;
                (img.width(), img.height())
            }
        };
        return Ok(Raster {
            width,
            height,
            data: bytes,
            filter: "DCTDecode",
        });
    }
    let img = image::load_from_memory(&bytes).map_err(|e| std::io::Error::other(e.to_string()))?;
    let rgb = img.to_rgb8();
    let (width, height) = (rgb.width(), rgb.height());
    let data = miniz_oxide::deflate::compress_to_vec_zlib(rgb.as_raw(), 6);
    Ok(Raster {
        width,
        height,
        data,
        filter: "FlateDecode",
    })
}

/// Width and height from a JPEG's start-of-frame segment (any SOFn marker).
fn jpeg_size(b: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 4 <= b.len() {
        if b[i] != 0xFF {
            return None;
        }
        let marker = b[i + 1];
        // Fill bytes and standalone markers (TEM, RSTn) carry no length.
        if marker == 0xFF {
            i += 1;
            continue;
        }
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        let sof = matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if sof {
            let seg = b.get(i + 4..i + 2 + len)?;
            let h = u16::from_be_bytes([*seg.get(1)?, *seg.get(2)?]) as u32;
            let w = u16::from_be_bytes([*seg.get(3)?, *seg.get(4)?]) as u32;
            return (w > 0 && h > 0).then_some((w, h));
        }
        if marker == 0xDA || len < 2 {
            return None;
        }
        i += 2 + len;
    }
    None
}

/// Writes through to `inner`, counting bytes so xref offsets need no buffer.
struct Counted<W: Write> {
    inner: W,
    pos: usize,
}

impl<W: Write> Counted<W> {
    fn put(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.inner.write_all(bytes)?;
        self.pos += bytes.len();
        Ok(())
    }
}

/// Streams the PDF to `path`, holding one page raster in memory at a time. A
/// failed write leaves no partial file behind.
pub fn write_pdf(path: &str, pages: &[PdfPage]) -> std::io::Result<()> {
    let result = stream_pdf(path, pages);
    if result.is_err() {
        drop(std::fs::remove_file(path));
    }
    result
}

fn stream_pdf(path: &str, pages: &[PdfPage]) -> std::io::Result<()> {
    let mut out = Counted {
        inner: std::io::BufWriter::new(std::fs::File::create(path)?),
        pos: 0,
    };
    let buf = &mut out;
    let mut offsets: Vec<usize> = Vec::new();
    buf.put(b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n")?;
    // Object numbering: 1 catalog, 2 pages, then per page: page, contents, image.
    let n = pages.len();
    let page_obj = |i: usize| 3 + i * 3;
    let obj = |buf: &mut Counted<_>, offsets: &mut Vec<usize>, body: &[u8]| {
        offsets.push(buf.pos);
        let id = offsets.len();
        buf.put(format!("{id} 0 obj\n").as_bytes())?;
        buf.put(body)?;
        buf.put(b"\nendobj\n")
    };
    obj(buf, &mut offsets, b"<< /Type /Catalog /Pages 2 0 R >>")?;
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", page_obj(i))).collect();
    obj(
        buf,
        &mut offsets,
        format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")).as_bytes(),
    )?;
    for (i, p) in pages.iter().enumerate() {
        let raster = load_raster(&p.image_path)?;
        let (w, h) = (p.width_pt, p.height_pt);
        let page = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w:.3} {h:.3}] /Resources << /XObject << /Im0 {} 0 R >> >> /Contents {} 0 R >>",
            page_obj(i) + 2,
            page_obj(i) + 1
        );
        obj(buf, &mut offsets, page.as_bytes())?;
        let content = format!("q {w:.3} 0 0 {h:.3} 0 0 cm /Im0 Do Q");
        obj(
            buf,
            &mut offsets,
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            )
            .as_bytes(),
        )?;
        // The image object is written in pieces to avoid copying the raster.
        offsets.push(buf.pos);
        let id = offsets.len();
        buf.put(
            format!(
                "{id} 0 obj\n<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /{} /Length {} >>\nstream\n",
                raster.width,
                raster.height,
                raster.filter,
                raster.data.len()
            )
            .as_bytes(),
        )?;
        buf.put(&raster.data)?;
        buf.put(b"\nendstream\nendobj\n")?;
    }
    let xref = buf.pos;
    buf.put(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes())?;
    for off in &offsets {
        buf.put(format!("{off:010} 00000 n \n").as_bytes())?;
    }
    buf.put(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len() + 1
        )
        .as_bytes(),
    )?;
    out.inner.flush()
}

#[cfg(test)]
mod tests;
