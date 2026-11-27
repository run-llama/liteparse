use crate::office::doc::{Error, MAX_FILE_BYTES, Result, bytes, u16_at, u32_at};
use std::collections::{HashMap, HashSet};

const END: u32 = 0xffff_fffe;
const FREE: u32 = 0xffff_ffff;

struct Entry {
    name: String,
    kind: u8,
    left: u32,
    right: u32,
    child: u32,
    start: u32,
    size: usize,
}

/// Reads only root-level streams, so embedded objects cannot impersonate the document.
pub struct Compound<'a> {
    data: &'a [u8],
    sector: usize,
    fat: Vec<u32>,
    mini_fat: Vec<u32>,
    mini: Vec<u8>,
    streams: HashMap<String, Entry>,
}
impl<'a> Compound<'a> {
    pub fn open(data: &'a [u8]) -> Result<Self> {
        if data.len() > MAX_FILE_BYTES {
            return Err(Error::Limit("file size"));
        }
        if bytes(data, 0, 8)? != [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1] {
            return Err(Error::Unsupported("not an OLE compound file"));
        }
        let version = u16_at(data, 26)?;
        let shift = u16_at(data, 30)?;
        if !matches!((version, shift), (3, 9) | (4, 12))
            || u16_at(data, 28)? != 0xfffe
            || u16_at(data, 32)? != 6
            || u32_at(data, 56)? != 4096
        {
            return Err(Error::Invalid("compound header"));
        }
        let mut c = Self {
            data,
            sector: 1usize << shift,
            fat: vec![],
            mini_fat: vec![],
            mini: vec![],
            streams: HashMap::new(),
        };
        if data.len() < c.sector {
            return Err(Error::Invalid("truncated compound header"));
        }
        let sectors = data.len() / c.sector - 1;
        let nf = u32_at(data, 44)? as usize;
        let nd = u32_at(data, 72)? as usize;
        if nf > sectors || nd > sectors {
            return Err(Error::Invalid("allocation table size"));
        }
        let mut difat = Vec::new();
        for off in (76..512).step_by(4) {
            difat.push(u32_at(data, off)?);
        }
        let mut next = u32_at(data, 68)?;
        let mut seen = HashSet::new();
        for _ in 0..nd {
            if !seen.insert(next) {
                return Err(Error::Invalid("DIFAT cycle"));
            }
            let sector = c.sector(next)?;
            for off in (0..c.sector - 4).step_by(4) {
                difat.push(u32_at(sector, off)?);
            }
            next = u32_at(sector, c.sector - 4)?;
        }
        // Some writers end the DIFAT chain with FREE instead of END.
        if nd > 0 && next != END && next != FREE {
            return Err(Error::Invalid("DIFAT termination"));
        }
        let ids: Vec<_> = difat.into_iter().filter(|&id| id != FREE).collect();
        if ids.len() != nf {
            return Err(Error::Invalid("FAT count"));
        }
        seen.clear();
        for id in ids {
            if !seen.insert(id) {
                return Err(Error::Invalid("duplicate FAT sector"));
            }
            let sector = c.sector(id)?;
            let values: Vec<_> = sector
                .as_chunks::<4>()
                .0
                .iter()
                .map(|v| u32::from_le_bytes(*v))
                .collect();
            c.fat.extend(values);
        }
        let directory = c.chain(u32_at(data, 48)?, false)?;
        let mut entries = Vec::new();
        for d in directory.as_chunks::<128>().0 {
            let kind = d[66];
            let len = u16_at(d, 64)? as usize;
            // The root's name is never looked up; some writers leave it malformed.
            let name = if kind == 0 || kind == 5 {
                String::new()
            } else {
                if !(2..=64).contains(&len) || !len.is_multiple_of(2) || u16_at(d, len - 2)? != 0 {
                    return Err(Error::Invalid("directory name"));
                }
                let units: Vec<_> = d[..len - 2]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|v| u16::from_le_bytes([v[0], v[1]]))
                    .collect();
                String::from_utf16(&units).map_err(|_| Error::Invalid("directory UTF-16"))?
            };
            let size = u32_at(d, 120)? as u64
                | if version == 4 {
                    (u32_at(d, 124)? as u64) << 32
                } else {
                    0
                };
            // Only streams are read: an unused entry's or a storage's size is garbage
            // in some files.
            if kind == 2 && size > MAX_FILE_BYTES as u64 {
                return Err(Error::Limit("stream size"));
            }
            entries.push(Entry {
                name,
                kind,
                left: u32_at(d, 68)?,
                right: u32_at(d, 72)?,
                child: u32_at(d, 76)?,
                start: u32_at(d, 116)?,
                size: size as usize,
            });
        }
        let root = entries.first().ok_or(Error::Invalid("missing root"))?;
        if root.kind != 5 {
            return Err(Error::Invalid("root entry"));
        }
        c.mini = if root.size == 0 {
            vec![]
        } else {
            c.chain(root.start, false)?
        };
        if c.mini.len() < root.size {
            return Err(Error::Invalid("short mini stream"));
        }
        c.mini.truncate(root.size);
        let count = u32_at(data, 64)? as usize;
        if count > sectors {
            return Err(Error::Invalid("mini FAT count"));
        }
        if count > 0 {
            let raw = c.chain(u32_at(data, 60)?, false)?;
            if raw.len() != count * c.sector {
                return Err(Error::Invalid("mini FAT length"));
            }
            c.mini_fat = raw
                .as_chunks::<4>()
                .0
                .iter()
                .map(|v| u32::from_le_bytes(*v))
                .collect();
        }
        let mut todo = vec![root.child];
        let mut selected = HashSet::new();
        seen.clear();
        while let Some(id) = todo.pop() {
            if id == FREE {
                continue;
            }
            if !seen.insert(id) {
                return Err(Error::Invalid("directory cycle"));
            }
            let e = entries
                .get(id as usize)
                .ok_or(Error::Invalid("directory index"))?;
            if !matches!(e.kind, 1 | 2) {
                return Err(Error::Invalid("directory entry type"));
            }
            todo.extend([e.left, e.right]);
            if e.kind == 2 {
                selected.insert(id as usize);
            }
        }
        for (i, e) in entries.into_iter().enumerate() {
            if selected.contains(&i) && c.streams.insert(e.name.clone(), e).is_some() {
                return Err(Error::Invalid("duplicate stream"));
            }
        }
        Ok(c)
    }
    fn sector(&self, id: u32) -> Result<&'a [u8]> {
        let off = (id as usize)
            .checked_add(1)
            .and_then(|v| v.checked_mul(self.sector))
            .ok_or(Error::Invalid("sector overflow"))?;
        bytes(self.data, off, self.sector)
    }
    /// The bytes present of sector `id` when the file ends inside it (some writers
    /// leave the last sector short).
    fn short_sector(&self, id: u32) -> Option<&'a [u8]> {
        let off = (id as usize).checked_add(1)?.checked_mul(self.sector)?;
        self.data.get(off..).filter(|tail| !tail.is_empty())
    }
    fn chain(&self, mut id: u32, mini: bool) -> Result<Vec<u8>> {
        let fat = if mini { &self.mini_fat } else { &self.fat };
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        while id != END {
            if !seen.insert(id) {
                return Err(Error::Invalid("sector chain cycle"));
            }
            let next = *fat
                .get(id as usize)
                .ok_or(Error::Invalid("sector chain index"))?;
            let chunk = if mini {
                bytes(&self.mini, id as usize * 64, 64)?
            } else {
                self.sector(id)
                    .or_else(|e| self.short_sector(id).ok_or(e))?
            };
            if out.len() + self.sector.max(chunk.len()) > MAX_FILE_BYTES {
                return Err(Error::Limit("stream chain"));
            }
            out.extend_from_slice(chunk);
            // A file cut short inside its last sector reads that sector's missing bytes as zeros.
            if !mini && chunk.len() < self.sector {
                out.resize(out.len() + self.sector - chunk.len(), 0);
            }
            id = next;
        }
        Ok(out)
    }
    pub fn trailing_bytes(&self) -> usize {
        self.data.len() % self.sector
    }

    pub fn stream(&self, name: &str) -> Result<Vec<u8>> {
        let e = self
            .streams
            .get(name)
            .ok_or(Error::Invalid("missing root stream"))?;
        if e.size == 0 {
            return Ok(vec![]);
        }
        let mut out = self.chain(e.start, e.size < 4096)?;
        if out.len() < e.size {
            return Err(Error::Invalid("short stream"));
        }
        out.truncate(e.size);
        Ok(out)
    }
}
