//! OPC package access: the zip container, part reads under size limits, and
//! relationship resolution.
//!
//! Part names are stored without a leading slash (`ppt/slides/slide1.xml`), which is
//! how zip entries are named. Relationship targets are resolved against the source
//! part's directory, and absolute targets (`/ppt/...`) are normalized to the same form.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::{Arc, Mutex};

use crate::office::pptx::config::Limits;
use crate::office::pptx::error::{DocError, DocResult};

/// A relationship from `_rels/*.rels`. `target` is the resolved part name for
/// internal targets, or the raw URI for `TargetMode="External"`.
#[derive(Debug, Clone)]
pub struct Relationship {
    pub id: String,
    pub rel_type: String,
    pub target: String,
    pub external: bool,
}

impl Relationship {
    /// The last path segment of the relationship type URI (`slide`, `slideLayout`,
    /// `image`, `hyperlink`, ...). Transitional and strict namespaces share it.
    pub fn kind(&self) -> &str {
        self.rel_type.rsplit('/').next().unwrap_or("")
    }
}

/// The package bytes, shared by every clone of the archive reader.
#[derive(Clone)]
struct SharedBytes(Arc<Vec<u8>>);

impl AsRef<[u8]> for SharedBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Safe to share between threads: each read clones the archive handle (the
/// bytes and the central directory are reference counted), so parts inflate
/// concurrently instead of queueing on one reader.
pub struct Package {
    archive: zip::ZipArchive<Cursor<SharedBytes>>,
    names: Vec<String>,
    lower_index: HashMap<String, usize>,
    max_part_bytes: u64,
    rels_cache: Mutex<HashMap<String, std::sync::Arc<Vec<Relationship>>>>,
    imported: Mutex<ImportedParts>,
}

#[derive(Default)]
struct ImportedParts {
    parts: HashMap<String, Arc<[u8]>>,
    content: HashMap<Arc<[u8]>, String>,
    bytes: usize,
    next: usize,
}

const MAX_IMPORTED_BYTES: usize = 8 * 1024 * 1024;
const MAX_IMPORTED_ASSETS: usize = 4096;
const IMPORT_PREFIX: &str = "__native_import_assets__/";

/// File signatures checked before the zip is opened.
const CFB_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

impl Package {
    pub fn open(bytes: Vec<u8>, limits: &Limits) -> DocResult<Package> {
        if bytes.starts_with(&CFB_MAGIC) {
            // An OOXML file inside a compound file is either password encryption
            // (`EncryptedPackage` stream) or IRM/DRM (`DRMEncryptedDataSpace`).
            if contains_utf16(&bytes, "EncryptedPackage")
                || contains_utf16(&bytes, "DRMEncryptedDataSpace")
            {
                return Err(DocError::Protected);
            }
            return Err(DocError::Broken(
                "compound file is not an OOXML package".into(),
            ));
        }
        if is_mojibake(&bytes) {
            return Err(DocError::Corrupt(
                "binary was transcoded as text (U+FFFD density)".into(),
            ));
        }
        if !bytes.starts_with(b"PK") {
            return Err(DocError::Broken("not a zip container".into()));
        }
        let bytes = trim_after_eocd(bytes);
        let archive = zip::ZipArchive::new(Cursor::new(SharedBytes(Arc::new(bytes))))
            .map_err(|e| DocError::Broken(format!("zip: {e}")))?;
        if archive.len() > limits.max_zip_entries as usize {
            return Err(DocError::Broken(format!(
                "{} zip entries exceeds the limit",
                archive.len()
            )));
        }
        let names: Vec<String> = archive
            .file_names()
            .map(|n| n.map(String::from))
            .collect::<Result<_, _>>()
            .map_err(|e| DocError::Broken(format!("zip entry name: {e}")))?;
        let mut lower_index = HashMap::new();
        for (i, name) in names.iter().enumerate() {
            lower_index.insert(name.to_ascii_lowercase(), i);
        }
        Ok(Package {
            archive,
            names,
            lower_index,
            max_part_bytes: limits.max_part_bytes,
            rels_cache: Mutex::new(HashMap::new()),
            imported: Mutex::new(ImportedParts::default()),
        })
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn exists(&self, part: &str) -> bool {
        self.lower_index.contains_key(&part.to_ascii_lowercase())
            || self.imported_part(part).is_some()
    }

    /// Registers immutable imported asset bytes without changing ZIP entries or relationships.
    /// Generated names are package-local and collision checked. The shared eight-MiB budget
    /// bounds all imported assets; ordinary per-part limits apply before copying bytes.
    #[expect(
        clippy::unwrap_in_result,
        reason = "the imported lock is held only for panic-free map updates, and `total` is checked above"
    )]
    pub fn import_asset(&self, bytes: &[u8]) -> DocResult<String> {
        let mut imported = self.imported.lock().unwrap();
        if let Some(name) = imported.content.get(bytes) {
            return Ok(name.clone());
        }
        let total = imported.bytes.checked_add(bytes.len());
        if bytes.len() as u64 > self.max_part_bytes
            || total.is_none_or(|total| total > MAX_IMPORTED_BYTES)
            || imported.parts.len() >= MAX_IMPORTED_ASSETS
        {
            return Err(DocError::LimitExceeded(
                "imported asset resource budget".into(),
            ));
        }
        let name = loop {
            let name = format!("{IMPORT_PREFIX}{}", imported.next);
            imported.next += 1;
            if !self.lower_index.contains_key(&name) {
                break name;
            }
        };
        imported.bytes = total.unwrap();
        let bytes: Arc<[u8]> = Arc::from(bytes);
        imported.content.insert(bytes.clone(), name.clone());
        imported.parts.insert(name.clone(), bytes);
        Ok(name)
    }

    fn imported_part(&self, part: &str) -> Option<Arc<[u8]>> {
        if !part
            .get(..IMPORT_PREFIX.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(IMPORT_PREFIX))
        {
            return None;
        }
        self.imported
            .lock()
            .unwrap()
            .parts
            .get(&part.to_ascii_lowercase())
            .cloned()
    }

    /// Reads a part's bytes. Lookup is case-insensitive because producers disagree
    /// with their own relationship targets about case.
    pub fn read(&self, part: &str) -> DocResult<Vec<u8>> {
        if let Some(bytes) = self.imported_part(part) {
            return Ok(bytes.to_vec());
        }
        let idx = *self
            .lower_index
            .get(&part.to_ascii_lowercase())
            .ok_or_else(|| DocError::MissingPart(part.to_string()))?;
        let mut archive = self.archive.clone();
        let mut file = archive
            .by_index(idx)
            .map_err(|e| DocError::Broken(format!("zip entry {part}: {e}")))?;
        if file.size() > self.max_part_bytes {
            return Err(DocError::LimitExceeded(format!(
                "part {part} is {} bytes",
                file.size()
            )));
        }
        let mut out = Vec::with_capacity(file.size() as usize);
        // `take` guards against a lying size header in the local entry.
        (&mut file)
            .take(self.max_part_bytes + 1)
            .read_to_end(&mut out)
            .map_err(|e| DocError::Broken(format!("inflate {part}: {e}")))?;
        if out.len() as u64 > self.max_part_bytes {
            return Err(DocError::LimitExceeded(format!(
                "part {part} inflates past the limit"
            )));
        }
        Ok(out)
    }

    /// Uncompressed size of a part as the zip directory records it, without
    /// inflating it.
    pub fn part_size(&self, part: &str) -> Option<u64> {
        if let Some(bytes) = self.imported_part(part) {
            return Some(bytes.len() as u64);
        }
        let idx = *self.lower_index.get(&part.to_ascii_lowercase())?;
        let mut archive = self.archive.clone();
        let file = archive.by_index_raw(idx).ok()?;
        Some(file.size())
    }

    /// The first `max` bytes of a part (enough for a picture header).
    pub fn read_head(&self, part: &str, max: u64) -> DocResult<Vec<u8>> {
        if let Some(bytes) = self.imported_part(part) {
            return Ok(
                bytes[..bytes.len().min(usize::try_from(max).unwrap_or(usize::MAX))].to_vec(),
            );
        }
        let idx = *self
            .lower_index
            .get(&part.to_ascii_lowercase())
            .ok_or_else(|| DocError::MissingPart(part.to_string()))?;
        let mut archive = self.archive.clone();
        let file = archive
            .by_index(idx)
            .map_err(|e| DocError::Broken(format!("zip entry {part}: {e}")))?;
        let mut out = Vec::new();
        file.take(max)
            .read_to_end(&mut out)
            .map_err(|e| DocError::Broken(format!("inflate {part}: {e}")))?;
        Ok(out)
    }

    pub fn read_string(&self, part: &str) -> DocResult<String> {
        let bytes = self.read(part)?;
        Ok(decode_xml_text(bytes))
    }

    /// Relationships of `part`, resolved. A missing rels part is an empty list.
    pub fn rels(&self, part: &str) -> std::sync::Arc<Vec<Relationship>> {
        if let Some(cached) = self
            .rels_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(part)
        {
            return cached.clone();
        }
        let rels_part = rels_part_name(part);
        let parsed = match self.read_string(&rels_part) {
            Ok(xml) => parse_rels(&xml, part),
            Err(_) => Vec::new(),
        };
        let arc = std::sync::Arc::new(parsed);
        self.rels_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(part.to_string(), arc.clone());
        arc
    }

    pub fn rel_by_id(&self, part: &str, id: &str) -> Option<Relationship> {
        self.rels(part).iter().find(|r| r.id == id).cloned()
    }

    pub fn rel_by_kind(&self, part: &str, kind: &str) -> Option<Relationship> {
        self.rels(part).iter().find(|r| r.kind() == kind).cloned()
    }
}

fn rels_part_name(part: &str) -> String {
    match part.rfind('/') {
        Some(i) => format!("{}/_rels/{}.rels", &part[..i], &part[i + 1..]),
        None => format!("_rels/{part}.rels"),
    }
}

fn parse_rels(xml: &str, source_part: &str) -> Vec<Relationship> {
    let Ok(doc) = roxmltree::Document::parse(xml) else {
        return Vec::new();
    };
    let base_dir = match source_part.rfind('/') {
        Some(i) => &source_part[..i],
        None => "",
    };
    doc.root_element()
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "Relationship")
        .filter_map(|n| {
            let id = n.attribute("Id")?.to_string();
            let rel_type = n.attribute("Type").unwrap_or("").to_string();
            let raw = n.attribute("Target").unwrap_or("");
            let external = n.attribute("TargetMode") == Some("External");
            let target = if external {
                raw.to_string()
            } else {
                resolve_target(base_dir, raw)
            };
            Some(Relationship {
                id,
                rel_type,
                target,
                external,
            })
        })
        .collect()
}

/// Resolves a relative OPC target (`../media/image1.png`) against a directory.
pub fn resolve_target(base_dir: &str, target: &str) -> String {
    let target = percent_decode(target);
    let mut segments: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        base_dir.split('/').filter(|s| !s.is_empty()).collect()
    };
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            s => segments.push(s),
        }
    }
    segments.join("/")
}

fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            // The two bytes after `%` can end inside a multi-byte char, so they are
            // checked as bytes; a pair that does not parse as hex stays literal.
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Decodes a part as text: UTF-8 with or without BOM, or UTF-16 with a BOM.
fn decode_xml_text(bytes: Vec<u8>) -> String {
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        let le = bytes[0] == 0xFF;
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| {
                if le {
                    u16::from_le_bytes([c[0], c[1]])
                } else {
                    u16::from_be_bytes([c[0], c[1]])
                }
            })
            .collect();
        return String::from_utf16_lossy(&units);
    }
    let start = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        3
    } else {
        0
    };
    match String::from_utf8(bytes[start..].to_vec()) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    }
}

/// Same rule as the worker's `isMojibakeCorruptedBinaryFile`: at least 5% of the
/// bytes are UTF-8 replacement characters and the whole file is valid UTF-8, which
/// a real zip (deflate streams) never is.
fn is_mojibake(bytes: &[u8]) -> bool {
    const REPLACEMENT: [u8; 3] = [0xEF, 0xBF, 0xBD];
    if bytes.is_empty() {
        return false;
    }
    let count = bytes.windows(3).filter(|w| *w == REPLACEMENT).count();
    if (count * 3) as f64 / (bytes.len() as f64) < 0.05 {
        return false;
    }
    std::str::from_utf8(bytes).is_ok()
}

fn contains_utf16(haystack: &[u8], needle: &str) -> bool {
    let pattern: Vec<u8> = needle
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    haystack
        .windows(pattern.len())
        .any(|w| w == pattern.as_slice())
}

/// Drops bytes after the end-of-central-directory record (and its comment). Some
/// producers pad the file, which strict zip readers reject.
fn trim_after_eocd(mut bytes: Vec<u8>) -> Vec<u8> {
    const SIG: [u8; 4] = [0x50, 0x4B, 0x05, 0x06];
    let search_from = bytes.len().saturating_sub(65_557 + 22);
    let Some(pos) = bytes[search_from..].windows(4).rposition(|w| w == SIG) else {
        return bytes;
    };
    let eocd = search_from + pos;
    if eocd + 22 > bytes.len() {
        return bytes;
    }
    let comment_len = u16::from_le_bytes([bytes[eocd + 20], bytes[eocd + 21]]) as usize;
    let end = eocd + 22 + comment_len;
    if end < bytes.len() {
        bytes.truncate(end);
    }
    bytes
}

#[cfg(test)]
mod tests;
