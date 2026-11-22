//! `lit models`: download, list and remove LiteParse's optional model packages.
//!
//! One package today, `ocria-nano` (OCR + table models). Its files are pinned here by
//! SHA-256, as is the ONNX Runtime build it runs on, which comes from Microsoft's GitHub
//! release. A pull writes to a temporary sibling directory and renames it into place only
//! once every file has verified, so an interrupted pull never leaves a half-installed package.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::LiteParseError;
use crate::ocr::ocria::{RUNTIME_LIB, default_model_dir};

use crate::ocr::ocria::package::{MODEL_BASE_URL, OCRIA_NANO_FILES, PinnedFile};
/// The package name `lit models pull` takes.
pub use crate::ocr::ocria::package::{OCRIA_NANO, OCRIA_NANO_VERSION};

/// LayoutNet ships twice, one copy per platform: the Rust implementation (`tablenet.bin`) is the
/// faster one on macOS (Accelerate), ONNX Runtime (`tablenet.onnx`) everywhere else.
fn wanted(f: &PinnedFile) -> bool {
    match f.name {
        "tablenet.bin" => cfg!(target_os = "macos"),
        "tablenet.onnx" => !cfg!(target_os = "macos"),
        _ => true,
    }
}

/// ONNX Runtime 1.31.1 for one platform: its library inside Microsoft's NuGet package (the source
/// used first), and the GitHub release archive with the library's path inside it (the fallback).
struct RuntimeArchive {
    name: &'static str,
    sha256: &'static str,
    lib: &'static str,
    nuget: &'static str,
    /// sha256 of the library itself, as the signed NuGet package holds it
    nuget_sha256: &'static str,
}

const ORT_VERSION: &str = "1.31.1";

/// Microsoft's NuGet package of the same version: one archive (160 MB) holding every platform's
/// library, author-signed by Microsoft and countersigned by NuGet.org (2026-10-23). Only this
/// platform's library is fetched from it (HTTP range requests, 6-13 MB), checked against its own
/// pin; the whole package (pinned too) when the server ignores ranges. Its libraries are separate
/// builds from the GitHub archives' (the macOS one is Developer ID signed, the archive's ad hoc),
/// so the two sources are pinned apart.
const NUGET_NAME: &str = "microsoft.ml.onnxruntime.1.31.1.nupkg";
const NUGET_URL: &str = "https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime/1.31.1/microsoft.ml.onnxruntime.1.31.1.nupkg";
const NUGET_SHA256: &str = "6871ccbc97f3d5bdb4095a1dcec33aa78d56f6ed8de5980ef5fa1d9f2e7c679a";

fn runtime_archive() -> Option<RuntimeArchive> {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    Some(match (os, arch) {
        ("macos", "aarch64") => RuntimeArchive {
            name: "onnxruntime-osx-arm64-1.31.1.tgz",
            sha256: "2e1344a1926ee50eb25b38153cb6992c3567515aef0e677a1a520a1798896d6f",
            lib: "onnxruntime-osx-arm64-1.31.1/lib/libonnxruntime.1.31.1.dylib",
            nuget: "runtimes/osx-arm64/native/libonnxruntime.dylib",
            nuget_sha256: "2685dc296147c47b0dbd8e0615547b12fc0c445d158d1f374cbd0705d9626fdc",
        },
        ("linux", "x86_64") => RuntimeArchive {
            name: "onnxruntime-linux-x64-1.31.1.tgz",
            sha256: "c849e36a1ef3e22d4ff217b9d1ce4a3cf5b9467b975610913ae959ad5b16bdc9",
            lib: "onnxruntime-linux-x64-1.31.1/lib/libonnxruntime.so.1.31.1",
            nuget: "runtimes/linux-x64/native/libonnxruntime.so",
            nuget_sha256: "a96f132e5eb5ca04e23a7d2fc1078fb95aa591e8d71d0f237702ece3a1e0f4ea",
        },
        ("linux", "aarch64") => RuntimeArchive {
            name: "onnxruntime-linux-aarch64-1.31.1.tgz",
            sha256: "5317505e2da99be65f8e2775ba3c2afd6d2961fd78aba1bd8f56159f48820c78",
            lib: "onnxruntime-linux-aarch64-1.31.1/lib/libonnxruntime.so.1.31.1",
            nuget: "runtimes/linux-arm64/native/libonnxruntime.so",
            nuget_sha256: "7e17c55d675be73f67a218dbecb55522395f61dafb3da03ebc7ade56a4c2f813",
        },
        ("windows", "x86_64") => RuntimeArchive {
            name: "onnxruntime-win-x64-1.31.1.zip",
            sha256: "5869e26bb25314375f2d5ef1877e3ea06b1543d6c710e6e4a1ec2f7177ab4b2c",
            lib: "onnxruntime-win-x64-1.31.1/lib/onnxruntime.dll",
            nuget: "runtimes/win-x64/native/onnxruntime.dll",
            nuget_sha256: "cb2fa518b18356617f6f9f460c012501da2819ec7e5c1d80bd8f5de20492db0c",
        },
        ("windows", "aarch64") => RuntimeArchive {
            name: "onnxruntime-win-arm64-1.31.1.zip",
            sha256: "817f4129073f84f81cde3ff92b1a4e06d007691e7208deed08a8b6d6df143f7f",
            lib: "onnxruntime-win-arm64-1.31.1/lib/onnxruntime.dll",
            nuget: "runtimes/win-arm64/native/onnxruntime.dll",
            nuget_sha256: "3ec71b45c71ed057096f4914fddb379a38b9ec813794052b96fec7de81e2ea20",
        },
        _ => return None,
    })
}

const LICENSE_NOTICE: &str = "\
ocria-nano is LiteParse's own OCR and table model package. Its weights are NOT covered by
LiteParse's open-source licence: they come under the ocria-nano model licence, installed
alongside them as LICENSE. By downloading you agree to its terms. ONNX Runtime (MIT, Microsoft)
is downloaded with it.";

fn err(msg: impl Into<String>) -> LiteParseError {
    LiteParseError::Other(msg.into())
}

fn sha256_file(path: &Path) -> Result<String, LiteParseError> {
    let mut f = std::fs::File::open(path).map_err(|e| err(format!("{}: {e}", path.display())))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).map_err(|e| err(e.to_string()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}

fn verify(path: &Path, sha256: &str) -> Result<(), LiteParseError> {
    let got = sha256_file(path)?;
    if got != sha256 {
        return Err(err(format!(
            "checksum mismatch for {}: expected {sha256}, got {got}",
            path.display()
        )));
    }
    Ok(())
}

/// A Hugging Face access token for a private model repo: `HF_TOKEN`, else the token
/// `hf auth login` stores (`$HF_HOME/token`, default `~/.cache/huggingface/token`).
fn hf_token() -> Option<String> {
    if let Ok(t) = std::env::var("HF_TOKEN")
        && !t.trim().is_empty()
    {
        return Some(t.trim().to_string());
    }
    let home = std::env::var_os("HF_HOME").map(PathBuf::from).or_else(|| {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache").join("huggingface"))
    })?;
    let t = std::fs::read_to_string(home.join("token")).ok()?;
    let t = t.trim();
    (!t.is_empty()).then(|| t.to_string())
}

async fn download(url: &str, dst: &Path, quiet: bool) -> Result<(), LiteParseError> {
    let mut req = reqwest::Client::new().get(url);
    // a private (pre-release) model repo needs the user's Hugging Face token; sent only there
    if url.starts_with("https://huggingface.co/")
        && let Some(token) = hf_token()
    {
        req = req.bearer_auth(token);
    }
    let mut resp = req
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| err(format!("download {url}: {e}")))?;
    let total = resp.content_length();
    let mut f = std::fs::File::create(dst).map_err(|e| err(format!("{}: {e}", dst.display())))?;
    let mut done = 0u64;
    // a live counter only on a terminal; in logs (CI, pipes) every redraw would be a new chunk
    let live = !quiet && total.is_some() && std::io::IsTerminal::is_terminal(&std::io::stderr());
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| err(format!("download {url}: {e}")))?
    {
        f.write_all(&chunk).map_err(|e| err(e.to_string()))?;
        done += chunk.len() as u64;
        if live && let Some(t) = total {
            eprint!("\r  {:>5.1} / {:.1} MB", done as f64 / 1e6, t as f64 / 1e6);
        }
    }
    if live {
        eprintln!();
    }
    Ok(())
}

/// Copy `src` to `dst` (following symlinks), or download it when `src` is a URL.
async fn fetch(src: &str, dst: &Path, quiet: bool) -> Result<(), LiteParseError> {
    if src.starts_with("https://") || src.starts_with("http://") {
        download(src, dst, quiet).await
    } else {
        std::fs::copy(src, dst)
            .map(|_| ())
            .map_err(|e| err(format!("{src}: {e}")))
    }
}

/// Pull options: where the model files come from (a released URL base by default, or a local
/// directory holding the same file names), and whether to print progress.
pub struct PullOptions {
    pub from: Option<PathBuf>,
    pub quiet: bool,
}

/// `lit models pull ocria-nano[@version]`: install the package into its model directory.
pub async fn pull(spec: &str, opts: &PullOptions) -> Result<PathBuf, LiteParseError> {
    let (name, version) = spec.split_once('@').unwrap_or((spec, "latest"));
    if name != OCRIA_NANO {
        return Err(err(format!(
            "unknown model package {name:?} (available: {OCRIA_NANO})"
        )));
    }
    if version != "latest" && version != OCRIA_NANO_VERSION {
        return Err(err(format!(
            "this lit installs {OCRIA_NANO}@{OCRIA_NANO_VERSION}; upgrade lit for {version}"
        )));
    }
    let rt = runtime_archive().ok_or_else(|| {
        err(format!(
            "ocria-nano has no ONNX Runtime build for {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
    })?;
    let dir = default_model_dir().ok_or_else(|| err("no home directory to install models into"))?;
    if !opts.quiet {
        eprintln!("{LICENSE_NOTICE}\n");
        eprintln!(
            "Installing {OCRIA_NANO}@{OCRIA_NANO_VERSION} into {}",
            dir.display()
        );
    }
    let parent = dir.parent().ok_or_else(|| err("bad model directory"))?;
    std::fs::create_dir_all(parent).map_err(|e| err(e.to_string()))?;
    let tmp = parent.join(format!(".{OCRIA_NANO}.partial"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("runtime")).map_err(|e| err(e.to_string()))?;
    let result = async {
        for f in OCRIA_NANO_FILES.iter().filter(|f| wanted(f)) {
            let src = match &opts.from {
                Some(d) => d.join(f.name).to_string_lossy().into_owned(),
                None => format!("{MODEL_BASE_URL}/{}", f.name),
            };
            if !opts.quiet {
                eprintln!("  {} ({:.1} MB)", f.name, f.size as f64 / 1e6);
            }
            let dst = tmp.join(f.name);
            fetch(&src, &dst, opts.quiet).await?;
            verify(&dst, f.sha256)?;
        }
        // the licence travels with the weights; a local source may not have one yet
        let lic = match &opts.from {
            Some(d) => d.join("LICENSE").to_string_lossy().into_owned(),
            None => format!("{MODEL_BASE_URL}/LICENSE"),
        };
        if opts.from.is_none() || Path::new(&lic).exists() {
            fetch(&lic, &tmp.join("LICENSE"), true).await?;
        }
        // ONNX Runtime: a copy of either pinned archive in <from>/runtime/, else the library from the
        // NuGet package (range requests, else the whole package), else the GitHub release archive
        let lib_dst = tmp.join("runtime").join(RUNTIME_LIB);
        let local = opts.from.as_ref().and_then(|d| {
            [
                (rt.name, rt.sha256, rt.lib),
                (NUGET_NAME, NUGET_SHA256, rt.nuget),
            ]
            .into_iter()
            .map(|(n, sha, member)| (d.join("runtime").join(n), n, sha, member))
            .find(|(p, ..)| p.exists())
        });
        if !opts.quiet {
            eprintln!("  ONNX Runtime {ORT_VERSION} ({})", rt.nuget);
        }
        if let Some((p, n, sha, member)) = local {
            let src = p.to_string_lossy();
            install_runtime(&src, &tmp.join(n), sha, member, &lib_dst, opts.quiet).await?;
        } else {
            let nuget = async {
                if !fetch_zip_member(NUGET_URL, rt.nuget, &lib_dst).await? {
                    if !opts.quiet {
                        eprintln!("  no range requests; downloading the whole package (160 MB)");
                    }
                    let arch = tmp.join(NUGET_NAME);
                    install_runtime(
                        NUGET_URL,
                        &arch,
                        NUGET_SHA256,
                        rt.nuget,
                        &lib_dst,
                        opts.quiet,
                    )
                    .await?;
                }
                verify(&lib_dst, rt.nuget_sha256)
            }
            .await;
            if let Err(e) = nuget {
                if !opts.quiet {
                    eprintln!(
                        "  NuGet: {e}\n  falling back to the GitHub release ({})",
                        rt.name
                    );
                }
                let gh = format!(
                    "https://github.com/microsoft/onnxruntime/releases/download/v{ORT_VERSION}/{}",
                    rt.name
                );
                let arch = tmp.join(rt.name);
                install_runtime(&gh, &arch, rt.sha256, rt.lib, &lib_dst, opts.quiet).await?;
            }
        }
        let manifest = serde_json::json!({
            "package": OCRIA_NANO, "version": OCRIA_NANO_VERSION, "onnxruntime": ORT_VERSION,
        });
        std::fs::write(tmp.join("manifest.json"), manifest.to_string())
            .map_err(|e| err(e.to_string()))?;
        Ok::<(), LiteParseError>(())
    }
    .await;
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| err(format!("{}: {e}", dir.display())))?;
    }
    std::fs::rename(&tmp, &dir).map_err(|e| err(e.to_string()))?;
    if !opts.quiet {
        eprintln!(
            "Done. OCR now uses ocria-nano by default (`--ocr-backend tesseract` to override, \
             `lit models rm ocria-nano` to uninstall)."
        );
    }
    Ok(dir)
}

/// One byte range of `url`; `None` when the server answers with the whole body (200, not 206).
async fn get_range(
    client: &reqwest::Client,
    url: &str,
    range: &str,
) -> Result<Option<(Vec<u8>, u64)>, LiteParseError> {
    let resp = client
        .get(url)
        .header(reqwest::header::RANGE, format!("bytes={range}"))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| err(format!("download {url}: {e}")))?;
    if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Ok(None);
    }
    // "bytes a-b/total"
    let total = resp
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit('/').next())
        .and_then(|v| v.parse::<u64>().ok())
        .ok_or_else(|| err(format!("download {url}: no Content-Range total")))?;
    let body = resp
        .bytes()
        .await
        .map_err(|e| err(format!("download {url}: {e}")))?;
    Ok(Some((body.to_vec(), total)))
}

fn le16(b: &[u8], at: usize) -> usize {
    u16::from_le_bytes([b[at], b[at + 1]]) as usize
}

fn le32(b: &[u8], at: usize) -> u64 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as u64
}

/// Write one member of the remote zip `url` to `dst` without downloading the rest: the end of
/// central directory record, the central directory, then the member's local header and data
/// (stored or deflated, checked against its CRC-32). `false` when the server ignores range
/// requests, so the caller downloads the whole archive instead.
async fn fetch_zip_member(url: &str, member: &str, dst: &Path) -> Result<bool, LiteParseError> {
    let bad = |what: &str| err(format!("{url}: {what}"));
    let client = reqwest::Client::new();
    // the EOCD record (22 bytes) ends the file, after a comment of at most 65,535 bytes
    let Some((tail, total)) = get_range(&client, url, "-65557").await? else {
        return Ok(false);
    };
    let eocd = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&i| tail[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
        .ok_or_else(|| bad("no end of central directory"))?;
    let (cd_size, cd_off) = (le32(&tail, eocd + 12), le32(&tail, eocd + 16));
    if cd_off == 0xffff_ffff || cd_off + cd_size > total {
        return Err(bad("zip64 or truncated central directory"));
    }
    let tail_off = total - tail.len() as u64;
    let cd = if cd_off >= tail_off {
        tail[(cd_off - tail_off) as usize..][..cd_size as usize].to_vec()
    } else {
        let range = format!("{cd_off}-{}", cd_off + cd_size - 1);
        get_range(&client, url, &range)
            .await?
            .ok_or_else(|| bad("range ignored"))?
            .0
    };
    let mut at = 0;
    let entry = loop {
        if at + 46 > cd.len() || cd[at..at + 4] != [0x50, 0x4b, 0x01, 0x02] {
            return Err(bad(&format!("{member} not found")));
        }
        let (nlen, xlen, clen) = (le16(&cd, at + 28), le16(&cd, at + 30), le16(&cd, at + 32));
        if cd.get(at + 46..at + 46 + nlen) == Some(member.as_bytes()) {
            break at;
        }
        at += 46 + nlen + xlen + clen;
    };
    let (method, crc) = (le16(&cd, entry + 10), le32(&cd, entry + 16) as u32);
    let (csize, usize_) = (le32(&cd, entry + 20), le32(&cd, entry + 24));
    let lho = le32(&cd, entry + 42);
    let head = get_range(&client, url, &format!("{lho}-{}", lho + 29))
        .await?
        .ok_or_else(|| bad("range ignored"))?
        .0;
    if head.len() < 30 || head[..4] != [0x50, 0x4b, 0x03, 0x04] {
        return Err(bad("bad local header"));
    }
    let start = lho + 30 + le16(&head, 26) as u64 + le16(&head, 28) as u64;
    let data = if csize == 0 {
        Vec::new()
    } else {
        get_range(&client, url, &format!("{start}-{}", start + csize - 1))
            .await?
            .ok_or_else(|| bad("range ignored"))?
            .0
    };
    let mut out = Vec::with_capacity(usize_ as usize);
    match method {
        0 => out.extend_from_slice(&data),
        8 => {
            flate2::read::DeflateDecoder::new(&data[..])
                .read_to_end(&mut out)
                .map_err(|e| bad(&format!("{member}: {e}")))?;
        }
        m => return Err(bad(&format!("{member}: compression method {m}"))),
    }
    let mut c = flate2::Crc::new();
    c.update(&out);
    if out.len() as u64 != usize_ || c.sum() != crc {
        return Err(bad(&format!("{member}: size or CRC mismatch")));
    }
    std::fs::write(dst, &out).map_err(|e| err(format!("{}: {e}", dst.display())))?;
    Ok(true)
}

/// Fetch the runtime archive `src` to `archive`, check it, write its member `member` to `lib`
/// and delete the archive.
async fn install_runtime(
    src: &str,
    archive: &Path,
    sha256: &str,
    member: &str,
    lib: &Path,
    quiet: bool,
) -> Result<(), LiteParseError> {
    let result = async {
        fetch(src, archive, quiet).await?;
        verify(archive, sha256)?;
        extract_one(archive, member, lib)
    }
    .await;
    let _ = std::fs::remove_file(archive);
    result
}

/// Write the archive member `member` (a .tgz, or a .zip / .nupkg) to `dst`.
fn extract_one(archive: &Path, member: &str, dst: &Path) -> Result<(), LiteParseError> {
    let f = std::fs::File::open(archive).map_err(|e| err(e.to_string()))?;
    let name = archive.to_string_lossy();
    let mut out = std::fs::File::create(dst).map_err(|e| err(e.to_string()))?;
    if name.ends_with(".zip") || name.ends_with(".nupkg") {
        let mut z = zip::ZipArchive::new(f).map_err(|e| err(e.to_string()))?;
        let found = z
            .file_names()
            .find(|n| n.trim_start_matches("./") == member)
            .map(str::to_string);
        let found = found.ok_or_else(|| err(format!("{member} not found in {name}")))?;
        let mut m = z
            .by_name(&found)
            .map_err(|e| err(format!("{member}: {e}")))?;
        std::io::copy(&mut m, &mut out).map_err(|e| err(e.to_string()))?;
        return Ok(());
    }
    let mut t = tar::Archive::new(flate2::read::GzDecoder::new(f));
    for entry in t.entries().map_err(|e| err(e.to_string()))? {
        let mut entry = entry.map_err(|e| err(e.to_string()))?;
        // release tarballs name their members "./onnxruntime-…"
        if entry
            .path()
            .map_err(|e| err(e.to_string()))?
            .to_string_lossy()
            .trim_start_matches("./")
            == member
        {
            std::io::copy(&mut entry, &mut out).map_err(|e| err(e.to_string()))?;
            return Ok(());
        }
    }
    Err(err(format!("{member} not found in {name}")))
}

/// An installed package: its directory, version and size on disk.
pub struct Installed {
    pub name: &'static str,
    pub version: Option<String>,
    pub dir: PathBuf,
    pub bytes: u64,
    /// Every pinned file is present.
    pub complete: bool,
}

fn dir_size(p: &Path) -> u64 {
    std::fs::read_dir(p)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

/// `lit models list`.
pub fn list() -> Vec<Installed> {
    let Some(dir) = default_model_dir() else {
        return Vec::new();
    };
    if !dir.exists() {
        return Vec::new();
    }
    let version = std::fs::read(dir.join("manifest.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| {
            v.get("version")
                .and_then(|x| x.as_str())
                .map(str::to_string)
        });
    let complete = OCRIA_NANO_FILES
        .iter()
        .filter(|f| wanted(f))
        .all(|f| dir.join(f.name).exists())
        && dir.join("runtime").join(RUNTIME_LIB).exists();
    vec![Installed {
        name: OCRIA_NANO,
        version,
        bytes: dir_size(&dir),
        dir,
        complete,
    }]
}

/// `lit models list` output, one line per installed package.
pub fn list_report() -> String {
    let installed = list();
    if installed.is_empty() {
        return "No model packages installed (try `lit models pull ocria-nano`).".to_string();
    }
    installed
        .iter()
        .map(|m| {
            format!(
                "{}@{}  {:.1} MB  {}{}",
                m.name,
                m.version.as_deref().unwrap_or("?"),
                m.bytes as f64 / 1e6,
                m.dir.display(),
                if m.complete {
                    ""
                } else {
                    "  (incomplete: pull again)"
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `lit models rm ocria-nano`: delete the installed package. Returns the removed directory.
pub fn remove(name: &str) -> Result<Option<PathBuf>, LiteParseError> {
    if name != OCRIA_NANO {
        return Err(err(format!(
            "unknown model package {name:?} (available: {OCRIA_NANO})"
        )));
    }
    let Some(dir) = default_model_dir() else {
        return Ok(None);
    };
    if !dir.exists() {
        return Ok(None);
    }
    std::fs::remove_dir_all(&dir).map_err(|e| err(format!("{}: {e}", dir.display())))?;
    Ok(Some(dir))
}

/// The installed ocria-nano TableNet weights, if any: this platform's copy (see `wanted`), else
/// whichever the installed package has.
pub fn installed_table_model() -> Option<PathBuf> {
    let dir = default_model_dir()?;
    let names = if cfg!(target_os = "macos") {
        ["tablenet.bin", "tablenet.onnx"]
    } else {
        ["tablenet.onnx", "tablenet.bin"]
    };
    names.iter().map(|n| dir.join(n)).find(|p| p.exists())
}
