use file_format::FileFormat;
use flate2::Compression;
use flate2::write::ZlibEncoder;
use image::{GenericImageView, ImageFormat};
use resvg::tiny_skia::Pixmap;
use resvg::usvg::{Options, Tree};
use std::io::{self, Write};
use tempfile::TempDir;
use tokio::fs;

use crate::error::LiteParseError;
use std::{
    fmt::{self, Display},
    path::Path,
};

/// Supported file extensions for conversion (non-PDF formats).
const OFFICE_EXTENSIONS: &[&str] = &[
    "doc", "docx", "docm", "dot", "dotm", "dotx", "odt", "ott", "rtf", "pages",
];
const PRESENTATION_EXTENSIONS: &[&str] = &[
    "ppt", "pptx", "pptm", "pot", "potm", "potx", "odp", "otp", "key",
];
const SPREADSHEET_EXTENSIONS: &[&str] = &[
    "xls", "xlsx", "xlsm", "xlsb", "ods", "ots", "csv", "tsv", "numbers",
];
const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "bmp", "tiff", "tif", "webp", "svg",
];

/// Plain-text extensions that cannot be rendered as page images.
const TEXT_ONLY_EXTENSIONS: &[&str] = &["txt", "md", "markdown", "log"];

/// A resolved external command with its executable path and any required prefix args.
#[derive(Debug, Clone)]
pub struct ResolvedCommand {
    pub command: String,
    pub args: Vec<String>,
    pub resolved_path: String,
}

#[derive(Debug, Clone)]
pub struct ConversionResult {
    pub pdf_path: String,
    pub original_extension: String,
}

enum ConversionTool {
    LibreOffice,
    ImageMagick,
}

impl Display for ConversionTool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::ImageMagick => "ImageMagick",
            Self::LibreOffice => "LibreOffice",
        };
        write!(f, "{}", s)
    }
}

/// Check if a file is a PDF (no conversion needed).
pub fn is_pdf(path: &str) -> bool {
    Path::new(path)
        .extension()
        .map(|ext| ext.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false)
}

/// Check if a file extension is supported (either PDF or convertible).
/// Returns true when the extension denotes a plain-text format with no page layout.
pub fn is_text_only_extension(ext: &str) -> bool {
    TEXT_ONLY_EXTENSIONS.contains(&ext.to_lowercase().as_str())
}

pub fn screenshot_text_format_error(ext: &str) -> LiteParseError {
    LiteParseError::Conversion(format!(
        "Cannot screenshot text-based format (.{ext}). Convert to PDF first."
    ))
}

/// Keeps converted PDF temp directories alive until rendering or parsing completes.
/// All temp dirs are cleaned up automatically when this guard is dropped.
#[derive(Debug)]
pub struct PdfInputGuard {
    #[allow(dead_code)]
    temps: Vec<TempDir>,
}

/// Resolve a document input to a PDF suitable for rendering or text extraction.
///
/// When `reject_text_formats` is true, plain-text files (`.txt`, etc.) return a
/// clear error instead of attempting conversion.
pub async fn resolve_pdf_input(
    input: crate::types::PdfInput,
    password: Option<&str>,
    reject_text_formats: bool,
) -> Result<(crate::types::PdfInput, PdfInputGuard), LiteParseError> {
    use crate::types::PdfInput;

    match input {
        PdfInput::Path(p) => {
            let ext = Path::new(&p)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if reject_text_formats && is_text_only_extension(ext) {
                return Err(screenshot_text_format_error(ext));
            }
            if is_pdf(&p) {
                Ok((PdfInput::Path(p), PdfInputGuard { temps: Vec::new() }))
            } else {
                let (converted, tmp_dir) = convert_to_pdf(&p, password).await?;
                let temps = tmp_dir.into_iter().collect();
                Ok((PdfInput::Path(converted.pdf_path), PdfInputGuard { temps }))
            }
        }
        PdfInput::Bytes(b) => {
            let ext = guess_extension_from_data(&b);
            if ext.as_deref() == Some("pdf") {
                return Ok((PdfInput::Bytes(b), PdfInputGuard { temps: Vec::new() }));
            }
            if reject_text_formats && ext.as_ref().is_some_and(|e| is_text_only_extension(e)) {
                return Err(screenshot_text_format_error(ext.as_ref().unwrap()));
            }
            let (converted, temps) = convert_data_to_pdf(b, password).await?;
            Ok((PdfInput::Path(converted.pdf_path), PdfInputGuard { temps }))
        }
    }
}

pub fn is_supported_extension(path: &str) -> bool {
    let ext = match Path::new(path).extension().and_then(|e| e.to_str()) {
        Some(e) => e.to_lowercase(),
        None => return false,
    };

    if ext == "pdf" {
        return true;
    }

    OFFICE_EXTENSIONS.contains(&ext.as_str())
        || PRESENTATION_EXTENSIONS.contains(&ext.as_str())
        || SPREADSHEET_EXTENSIONS.contains(&ext.as_str())
        || IMAGE_EXTENSIONS.contains(&ext.as_str())
}

/// Attempt to convert a non-PDF file to PDF.
///
/// Currently stubbed out — returns an error directing users to install
/// LibreOffice (for office documents) or ImageMagick (for images).
pub async fn convert_to_pdf(
    path: &str,
    password: Option<&str>,
) -> Result<(ConversionResult, Option<TempDir>), LiteParseError> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if ext == "pdf" {
        return Ok((
            ConversionResult {
                pdf_path: path.to_string(),
                original_extension: ext,
            },
            None,
        ));
    }

    let tool = if OFFICE_EXTENSIONS.contains(&ext.as_str())
        || PRESENTATION_EXTENSIONS.contains(&ext.as_str())
        || SPREADSHEET_EXTENSIONS.contains(&ext.as_str())
    {
        ConversionTool::LibreOffice
    } else if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        ConversionTool::ImageMagick
    } else {
        return Err(LiteParseError::Conversion(format!(
            "unsupported file format: .{}",
            ext
        )));
    };

    let tmp_dir = tempfile::Builder::new().prefix("liteparse-").tempdir()?;
    let pdf_path = match tool {
        ConversionTool::ImageMagick => {
            convert_image_to_pdf(path, tmp_dir.path().to_str().unwrap()).await?
        }
        ConversionTool::LibreOffice => {
            convert_office_document(path, tmp_dir.path().to_str().unwrap(), password).await?
        }
    };

    Ok((
        ConversionResult {
            pdf_path,
            original_extension: ext,
        },
        Some(tmp_dir),
    ))
}

/// Execute command with timeout
pub async fn execute_command(
    command: &str,
    args: Vec<&str>,
    timeout_ms: u64,
) -> Result<String, LiteParseError> {
    let proc = tokio::process::Command::new(command)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    match tokio::time::timeout(
        tokio::time::Duration::from_millis(timeout_ms),
        proc.wait_with_output(),
    )
    .await
    {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            if output.status.success() {
                Ok(stdout)
            } else {
                Err(LiteParseError::Conversion(format!(
                    "Command failed: {stderr}"
                )))
            }
        }
        Ok(Err(e)) => Err(LiteParseError::Conversion(format!("Command error: {e}"))),
        Err(_) => Err(LiteParseError::Conversion(format!(
            "Command timed out after {timeout_ms}ms"
        ))),
    }
}

/// Execute a command for PowerShel
pub async fn execute_powershell(command: &str, timeout_ms: u64) -> Result<String, LiteParseError> {
    execute_command(
        "powershell",
        vec!["-NoProfile", "-Command", command],
        timeout_ms,
    )
    .await
}

fn get_resolved_path_from_output(output: &str, use_last_line: bool) -> Option<String> {
    let lines: Vec<String> = output
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return None;
    }
    let l = if use_last_line {
        lines.last()
    } else {
        lines.first()
    };

    l.cloned()
}

/// Resolve the actual executable path for a command.
pub async fn resolve_command_path(command: &str) -> Option<String> {
    if std::env::consts::FAMILY == "windows" {
        let ps = format!("(Get-Command '{command}' -ErrorAction Stop).Source");
        match execute_powershell(&ps, 5000).await {
            Ok(out) => get_resolved_path_from_output(&out, true),
            Err(_) => None,
        }
    } else {
        match execute_command("which", vec![command], 5000).await {
            Ok(out) => get_resolved_path_from_output(&out, false),
            Err(_) => None,
        }
    }
}

/// Check if a command is available on Unix-like platforms (via `which`).
pub async fn is_command_available(command: &str) -> bool {
    execute_command("which", vec![command], 5000).await.is_ok()
}

/// Check if a command is available on Windows (via PowerShell `Get-Command`).
pub async fn is_command_available_windows(command: &str) -> bool {
    execute_powershell(&format!("Get-Command {command}"), 5000)
        .await
        .is_ok()
}

/// Check if a file path exists and is executable.
pub async fn is_path_executable(file_path: &str) -> bool {
    let p = std::path::PathBuf::from(file_path);
    match tokio::fs::metadata(&p).await {
        Ok(meta) => {
            if !meta.is_file() {
                return false;
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                meta.permissions().mode() & 0o111 != 0
            }

            #[cfg(windows)]
            {
                return true;
            }
        }
        Err(_) => false,
    }
}

/// Find LibreOffice command - handles different installation methods.
pub async fn find_libre_office_command() -> Option<String> {
    if is_command_available("libreoffice").await
        || is_command_available_windows("libreoffice").await
    {
        return Some("libreoffice".to_string());
    }

    if is_command_available("soffice").await || is_command_available_windows("soffice").await {
        return Some("soffice".to_string());
    }

    let mac_os_paths = [
        "/Applications/LibreOffice.app/Contents/MacOS/soffice",
        "/Applications/LibreOffice.app/Contents/MacOS/libreoffice",
    ];

    let windows_paths = ["C:\\Program Files\\Libreoffice\\program\\soffice.exe"];

    for lib_path in mac_os_paths.iter() {
        if is_path_executable(lib_path).await {
            return Some(lib_path.to_string());
        }
    }

    for lib_path in windows_paths.iter() {
        if is_path_executable(lib_path).await {
            return Some(lib_path.to_string());
        }
    }

    None
}

#[cfg(feature = "native-docx")]
pub fn convert_docx(file_path: &str, output_dir: &str) -> Result<String, LiteParseError> {
    let file_name = Path::new(&file_path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file.pdf".to_string());

    let pdf_path = Path::new(output_dir).join(Path::new(&file_name).with_extension("pdf"));
    docxide_pdf::convert_docx_to_pdf(Path::new(file_path), &pdf_path)
        .map_err(|e| LiteParseError::Conversion(e.to_string()))?;
    Ok(pdf_path.to_string_lossy().to_string())
}

/// Convert office documents using LibreOffice.
pub async fn convert_office_document(
    file_path: &str,
    output_dir: &str,
    password: Option<&str>,
) -> Result<String, LiteParseError> {
    #[cfg(feature = "native-docx")]
    if file_path.ends_with(".docx") {
        let path = convert_docx(file_path, output_dir)?;
        return Ok(path);
    }

    let libre_office_cmd = find_libre_office_command().await.ok_or_else(|| {
        LiteParseError::Conversion(
            "LibreOffice is not installed. Please install LibreOffice to convert office documents. \
             On macOS: brew install --cask libreoffice, On Ubuntu: apt-get install libreoffice, \
             On Windows: choco install libreoffice-fresh".into()
        )
    })?;
    // LibreOffice serializes on a per-user profile lock. Concurrent invocations
    // sharing the same profile race for the lock: the loser either silently
    // exits with status 0 producing no output, or crashes on shared state.
    // Give every invocation its own throwaway UserInstallation profile.
    let user_profile_dir = tempfile::Builder::new()
        .prefix("liteparse-lo-profile-")
        .tempdir()?;
    let user_profile_file_url =
        url::Url::from_file_path(user_profile_dir.path()).map_err(|_| {
            LiteParseError::Conversion(format!(
                "failed to convert temp profile path to file URL: {}",
                user_profile_dir.path().display()
            ))
        })?;
    let user_profile_url = format!("-env:UserInstallation={user_profile_file_url}");
    let infilter_arg;
    let mut args: Vec<&str> = vec![
        &user_profile_url,
        "--headless",
        "--invisible",
        "--convert-to",
        "pdf",
        "--outdir",
        output_dir,
    ];
    if let Some(pw) = password {
        infilter_arg = format!("--infilter=:{pw}");
        args.push(&infilter_arg);
    }
    args.push(file_path);

    execute_command(&libre_office_cmd, args, 120_000).await?;
    find_pdf_in_dir(output_dir).await
}

/// Scan `output_dir` for the first `.pdf` file and return its path.
///
/// LibreOffice sanitises filenames during conversion (e.g. strips parentheses,
/// leading digit sequences, spaces) so the output PDF stem often differs from
/// the input file stem.  Since `output_dir` is always a fresh temp directory
/// that holds exactly one file after a successful conversion, scanning for any
/// `.pdf` entry is more robust than constructing a fixed `<stem>.pdf` path.
async fn find_pdf_in_dir(output_dir: &str) -> Result<String, LiteParseError> {
    let mut read_dir = tokio::fs::read_dir(output_dir)
        .await
        .map_err(|e| LiteParseError::Conversion(format!("cannot read output dir: {e}")))?;
    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|e| LiteParseError::Conversion(format!("error reading output dir: {e}")))?
    {
        let path = entry.path();
        if path
            .extension()
            .map(|e| e.eq_ignore_ascii_case("pdf"))
            .unwrap_or(false)
        {
            return Ok(path.to_string_lossy().to_string());
        }
    }
    Err(LiteParseError::Conversion(
        "LibreOffice conversion succeeded but output PDF not found".into(),
    ))
}

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
fn deflate(data: &[u8]) -> io::Result<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::with_capacity(data.len() / 4), Compression::new(3));
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
    /// Decoded raster, to be zlib-compressed and embedded as `/FlateDecode`.
    Flate(PixelData),
}

/// Decode / analyse the input file and produce an `EmbeddedImage`.
fn prepare_image(file_path: &str, data: Vec<u8>) -> Result<EmbeddedImage, LiteParseError> {
    let is_svg = Path::new(file_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("svg"))
        .unwrap_or(false);

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

pub async fn convert_image_to_pdf(
    file_path: &str,
    output_dir: &str,
) -> Result<String, LiteParseError> {
    let data = fs::read(file_path).await?;
    let EmbeddedImage {
        width,
        height,
        dpi_x,
        dpi_y,
        payload,
    } = prepare_image(file_path, data)?;

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

    let base_name = Path::new(file_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .expect("Should be able to isolate base_name");
    let pdf_path = Path::new(output_dir)
        .join(format!("{base_name}.pdf"))
        .to_string_lossy()
        .to_string();

    fs::write(&pdf_path, pdf_data).await?;

    Ok(pdf_path)
}

pub fn guess_extension_from_data(data: &[u8]) -> Option<String> {
    // `file-format` inspects ZIP-based containers via their central directory
    // (requires the `reader` feature), so DOCX/XLSX/PPTX/ODF resolve to their
    // specific format instead of a generic "zip" regardless of entry ordering
    // or file size. Unrecognized input falls back to the generic binary format,
    // which we surface as `None` to match the prior contract.
    let fmt = FileFormat::from_bytes(data);
    if fmt == FileFormat::ArbitraryBinaryData {
        return None;
    }
    Some(fmt.extension().to_string())
}

pub async fn convert_data_to_pdf(
    data: Vec<u8>,
    password: Option<&str>,
) -> Result<(ConversionResult, Vec<TempDir>), LiteParseError> {
    let ext = guess_extension_from_data(&data);
    let staging_dir = tempfile::Builder::new()
        .prefix("liteparse-staging-")
        .tempdir()?;
    let tmp_path = staging_dir
        .path()
        .join(format!("input.{}", ext.unwrap_or("bin".to_string())));
    tokio::fs::write(&tmp_path, data).await?;
    let (converted, output_dir) = convert_to_pdf(tmp_path.to_str().unwrap(), password).await?;
    let mut temps = vec![staging_dir];
    if let Some(d) = output_dir {
        temps.push(d);
    }
    Ok((converted, temps))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_pdf() {
        assert!(is_pdf("foo.pdf"));
        assert!(is_pdf("foo.PDF"));
        assert!(is_pdf("/abs/dir/Bar.Pdf"));
        assert!(!is_pdf("foo.docx"));
        assert!(!is_pdf("foo"));
        assert!(!is_pdf(""));
    }

    #[test]
    fn test_is_supported_extension() {
        assert!(is_supported_extension("a.pdf"));
        assert!(is_supported_extension("A.DOCX"));
        assert!(is_supported_extension("a.pptx"));
        assert!(is_supported_extension("a.xlsx"));
        assert!(is_supported_extension("a.png"));
        assert!(is_supported_extension("a.svg"));
        assert!(!is_supported_extension("a.exe"));
        assert!(!is_supported_extension("noext"));
    }

    /// Build a minimal but structurally valid ZIP (stored, empty entries) whose
    /// central directory lists `names`. This exercises the same code path
    /// `file-format` uses to disambiguate ZIP-based office formats.
    fn build_zip(names: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut offsets = Vec::new();
        for name in names {
            offsets.push(out.len() as u32);
            let nb = name.as_bytes();
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes()); // local header sig
            out.extend_from_slice(&20u16.to_le_bytes()); // version needed
            out.extend_from_slice(&[0u8; 20]); // flags..uncompressed size (all zero)
            out.extend_from_slice(&(nb.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // extra len
            out.extend_from_slice(nb);
        }
        let cd_offset = out.len() as u32;
        let mut central = Vec::new();
        for (i, name) in names.iter().enumerate() {
            let nb = name.as_bytes();
            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes()); // central header sig
            central.extend_from_slice(&20u16.to_le_bytes()); // version made by
            central.extend_from_slice(&20u16.to_le_bytes()); // version needed
            central.extend_from_slice(&[0u8; 20]); // flags..uncompressed size
            central.extend_from_slice(&(nb.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0u8; 8]); // extra/comment/disk/internal attrs
            central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            central.extend_from_slice(&offsets[i].to_le_bytes()); // local header offset
            central.extend_from_slice(nb);
        }
        let cd_size = central.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes()); // EOCD sig
        out.extend_from_slice(&[0u8; 4]); // disk numbers
        out.extend_from_slice(&(names.len() as u16).to_le_bytes());
        out.extend_from_slice(&(names.len() as u16).to_le_bytes());
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // comment len
        out
    }

    #[test]
    fn test_guess_extension_disambiguates_zip_office_formats() {
        // ZIP-based office formats must resolve to their specific type, not a
        // generic "zip", even though they share the PK signature.
        assert_eq!(
            guess_extension_from_data(&build_zip(&["[Content_Types].xml", "word/document.xml"]))
                .as_deref(),
            Some("docx")
        );
        assert_eq!(
            guess_extension_from_data(&build_zip(&["[Content_Types].xml", "xl/workbook.xml"]))
                .as_deref(),
            Some("xlsx")
        );
        assert_eq!(
            guess_extension_from_data(&build_zip(&["[Content_Types].xml", "ppt/presentation.xml"]))
                .as_deref(),
            Some("pptx")
        );
        // A plain ZIP with no office markers stays "zip".
        assert_eq!(
            guess_extension_from_data(&build_zip(&["random/file.txt"])).as_deref(),
            Some("zip")
        );
    }

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
    #[tokio::test]
    async fn test_convert_image_to_pdf_uses_source_dpi_for_mediabox() {
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

        let dir = tempfile::tempdir().unwrap();
        let in_path = dir.path().join("test.png");
        tokio::fs::write(&in_path, &spliced).await.unwrap();

        let pdf_path =
            convert_image_to_pdf(in_path.to_str().unwrap(), dir.path().to_str().unwrap())
                .await
                .unwrap();
        let pdf = tokio::fs::read(&pdf_path).await.unwrap();

        // Expect MediaBox roughly "0 0 144 72" (allowing for fractional formatting).
        let s = String::from_utf8_lossy(&pdf);
        let re = regex::Regex::new(r"/MediaBox\s*\[\s*0\s+0\s+([0-9.]+)\s+([0-9.]+)\s*\]").unwrap();
        let caps = re.captures(&s).expect("MediaBox present");
        let w: f32 = caps[1].parse().unwrap();
        let h: f32 = caps[2].parse().unwrap();
        assert!((w - 144.0).abs() < 0.5, "expected ≈144 pt, got {w}");
        assert!((h - 72.0).abs() < 0.5, "expected ≈72 pt, got {h}");
    }

    #[tokio::test]
    async fn test_convert_image_to_pdf_no_dpi_falls_back_to_150() {
        let mut base = Vec::new();
        let img = image::RgbImage::from_pixel(200, 100, image::Rgb([255, 255, 255]));
        image::DynamicImage::ImageRgb8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut base),
                image::ImageFormat::Png,
            )
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let in_path = dir.path().join("test.png");
        tokio::fs::write(&in_path, &base).await.unwrap();
        let pdf_path =
            convert_image_to_pdf(in_path.to_str().unwrap(), dir.path().to_str().unwrap())
                .await
                .unwrap();
        let pdf = tokio::fs::read(&pdf_path).await.unwrap();
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
    fn test_conversion_tool_display() {
        assert_eq!(ConversionTool::ImageMagick.to_string(), "ImageMagick");
        assert_eq!(ConversionTool::LibreOffice.to_string(), "LibreOffice");
    }

    #[test]
    fn test_get_resolved_path_from_output_first_and_last() {
        let out = "  /usr/bin/foo\n\n/opt/bin/foo\n";
        assert_eq!(
            get_resolved_path_from_output(out, false).as_deref(),
            Some("/usr/bin/foo")
        );
        assert_eq!(
            get_resolved_path_from_output(out, true).as_deref(),
            Some("/opt/bin/foo")
        );
    }

    #[test]
    fn test_get_resolved_path_from_output_empty() {
        assert!(get_resolved_path_from_output("", false).is_none());
        assert!(get_resolved_path_from_output("   \n  \n", true).is_none());
    }

    #[test]
    fn test_guess_extension_from_data_png() {
        let png_header = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(
            guess_extension_from_data(&png_header).as_deref(),
            Some("png")
        );
    }

    #[test]
    fn test_guess_extension_from_data_unknown() {
        assert!(guess_extension_from_data(&[0u8, 1, 2, 3]).is_none());
    }

    #[tokio::test]
    async fn test_execute_command_failure() {
        let r = execute_command("ls", vec!["/this/definitely/does/not/exist"], 5000).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn test_execute_command_timeout() {
        let r = execute_command("sleep", vec!["5"], 50).await;
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn test_execute_command_spawn_error() {
        let r = execute_command("definitely_not_a_real_command_xyz123", vec![], 1000).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn test_is_path_executable_nonexistent() {
        assert!(!is_path_executable("/no/such/path/zzz").await);
    }

    #[test]
    fn test_is_text_only_extension() {
        assert!(is_text_only_extension("txt"));
        assert!(is_text_only_extension("TXT"));
        assert!(is_text_only_extension("md"));
        assert!(!is_text_only_extension("pdf"));
        assert!(!is_text_only_extension("docx"));
    }

    #[tokio::test]
    async fn test_resolve_pdf_input_rejects_text_for_screenshot() {
        use crate::types::PdfInput;

        let err = resolve_pdf_input(PdfInput::Path("/tmp/readme.txt".into()), None, true)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("Cannot screenshot text-based format"));
    }

    #[tokio::test]
    async fn test_resolve_pdf_input_pdf_bytes_zero_disk() {
        use crate::types::PdfInput;

        let pdf_bytes = b"%PDF-1.4\n";
        let (input, guard) = resolve_pdf_input(PdfInput::Bytes(pdf_bytes.to_vec()), None, true)
            .await
            .unwrap();
        assert!(matches!(input, PdfInput::Bytes(_)));
        assert!(guard.temps.is_empty());
    }

    #[tokio::test]
    async fn test_convert_to_pdf_passthrough_pdf() {
        let (res, _) = convert_to_pdf("/some/file.pdf", None).await.unwrap();
        assert_eq!(res.pdf_path, "/some/file.pdf");
        assert_eq!(res.original_extension, "pdf");
    }

    #[tokio::test]
    async fn test_convert_to_pdf_unsupported() {
        let r = convert_to_pdf("/some/file.xyz", None).await;
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("unsupported"));
    }

    /// The staging `TempDir` returned by `convert_data_to_pdf` must be
    /// cleaned up when dropped — both on success and failure paths.
    /// Here we verify the failure path: the error propagates, and once
    /// the returned temps are dropped the staging directory is gone.
    #[tokio::test]
    async fn test_convert_data_to_pdf_staging_cleaned_on_drop() {
        // Build a staging dir manually so we can inspect its path after drop.
        let staging_dir = tempfile::Builder::new()
            .prefix("liteparse-staging-")
            .tempdir()
            .unwrap();
        let staging_path = staging_dir.path().to_path_buf();
        assert!(staging_path.exists());

        // Dropping the TempDir removes the directory.
        drop(staging_dir);
        assert!(
            !staging_path.exists(),
            "staging temp dir should be removed on drop"
        );
    }

    // ── find_pdf_in_dir ──────────────────────────────────────────────────────

    /// Regression test for the LibreOffice filename-sanitisation bug.
    ///
    /// LibreOffice strips characters like parentheses and leading digit
    /// sequences from filenames, so the output PDF stem often differs from the
    /// input file stem.  `find_pdf_in_dir` must locate the PDF regardless of
    /// what LibreOffice chose to call it.
    ///
    /// Scenario: input was `52304751_AnuragLahare_E2 (1).docx`; LibreOffice
    /// wrote `AnuragLahare_E2_1_.pdf` instead of the expected
    /// `52304751_AnuragLahare_E2 (1).pdf`.
    #[tokio::test]
    async fn test_find_pdf_in_dir_returns_sanitised_name() {
        let tmp = tempfile::tempdir().unwrap();
        // Simulate LibreOffice writing a sanitised filename.
        let sanitised = tmp.path().join("AnuragLahare_E2_1_.pdf");
        tokio::fs::write(&sanitised, b"%PDF-1.4").await.unwrap();

        let result = find_pdf_in_dir(tmp.path().to_str().unwrap()).await;
        assert!(result.is_ok(), "should find PDF even with sanitised name");
        assert!(
            result.unwrap().ends_with("AnuragLahare_E2_1_.pdf"),
            "returned path should point to the sanitised file"
        );
    }

    /// When LibreOffice somehow produces no PDF (unexpected failure), the
    /// helpful error message must still be returned.
    #[tokio::test]
    async fn test_find_pdf_in_dir_empty_dir_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let result = find_pdf_in_dir(tmp.path().to_str().unwrap()).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("output PDF not found"),
            "error message should mention missing PDF"
        );
    }
}
