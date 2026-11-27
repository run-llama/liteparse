//! The persist directory: which stream offset holds each persist object (document,
//! slides, masters) in the newest saved edit.
//!
//! `Current User` names the newest UserEditAtom. Each UserEditAtom points at its
//! PersistDirectoryAtom and at the previous edit (`offsetLastEdit`, 0 ends the chain).
//! A fast save appends an edit that lists only the objects it changed, so walking from
//! newest to oldest and keeping the first offset seen for each id gives the live state.

use crate::office::ppt::Error;
use crate::office::ppt::records::{self as r, Rec};
use std::collections::{HashMap, HashSet};

/// `headerToken` of a CurrentUserAtom whose document is encrypted.
const ENCRYPTED_TOKEN: u32 = 0xF3D1_C4DF;
/// Edits followed before giving up on a chain (real files have a handful).
const MAX_EDITS: usize = 4096;

pub struct Persist {
    pub offsets: HashMap<u32, usize>,
    /// Persist id of the DocumentContainer, from the newest edit.
    pub document: u32,
}

impl Persist {
    /// The record a persist id points at, if that offset holds a whole record.
    pub fn record(&self, doc: &[u8], id: u32) -> Option<Rec> {
        r::at(doc, *self.offsets.get(&id)?)
    }
}

pub fn read(
    current_user: Option<&[u8]>,
    doc: &[u8],
    diags: &mut Vec<String>,
) -> Result<Persist, Error> {
    let from_user = match current_user {
        Some(cu) => newest_edit(cu)?,
        None => None,
    };
    let first = match from_user {
        Some(off) if r::at(doc, off).is_some_and(|e| e.kind == r::USER_EDIT_ATOM) => off,
        _ => {
            diags.push(
                "Current User does not name a UserEditAtom; using the last one in the stream"
                    .into(),
            );
            last_edit_in_stream(doc).ok_or(Error::Invalid("no UserEditAtom"))?
        }
    };
    let mut offsets = HashMap::new();
    let mut document = None;
    let mut seen = HashSet::new();
    let mut next = Some(first);
    while let Some(off) = next {
        if !seen.insert(off) || seen.len() > MAX_EDITS {
            diags.push(format!("UserEditAtom chain loops at offset {off}"));
            break;
        }
        let Some(edit) = r::at(doc, off).filter(|e| e.kind == r::USER_EDIT_ATOM && e.len() >= 28)
        else {
            diags.push(format!(
                "no UserEditAtom at offset {off}; older edits ignored"
            ));
            break;
        };
        let b = edit.start;
        if document.is_none() {
            document = r::u32_at(doc, b + 16);
            // encryptSessionPersistIdRef is present only in encrypted documents.
            if edit.len() >= 32 {
                return Err(Error::Encrypted);
            }
        }
        let dir = r::u32_at(doc, b + 12).unwrap_or(0) as usize;
        match r::at(doc, dir).filter(|d| d.kind == r::PERSIST_DIRECTORY_ATOM) {
            Some(d) => add_directory(doc, d, &mut offsets),
            None => diags.push(format!("no PersistDirectoryAtom at offset {dir}")),
        }
        next = r::u32_at(doc, b + 8)
            .filter(|&v| v != 0)
            .map(|v| v as usize);
    }
    let document = document.ok_or(Error::Invalid("no document persist id"))?;
    if !offsets.contains_key(&document) {
        return Err(Error::Invalid(
            "document persist id missing from the persist directory",
        ));
    }
    Ok(Persist { offsets, document })
}

/// `offsetToCurrentEdit` of the CurrentUserAtom; `None` when the stream is unreadable.
fn newest_edit(cu: &[u8]) -> Result<Option<usize>, Error> {
    // Read the fixed fields only: some writers declare a record length (it covers the
    // user names) that runs past the end of the stream.
    if r::u16_at(cu, 2) != Some(r::CURRENT_USER_ATOM) {
        return Ok(None);
    }
    if r::u32_at(cu, 12) == Some(ENCRYPTED_TOKEN) {
        return Err(Error::Encrypted);
    }
    Ok(r::u32_at(cu, 16).map(|v| v as usize))
}

/// Entries are a u32 (persist id in the low 20 bits, count in the high 12) followed by
/// that many u32 stream offsets for consecutive ids. Ids already known are newer.
fn add_directory(doc: &[u8], dir: Rec, offsets: &mut HashMap<u32, usize>) {
    let mut pos = dir.start;
    while let Some(head) = r::u32_at(doc, pos).filter(|_| pos + 4 <= dir.end) {
        let (id, count) = (head & 0xF_FFFF, head >> 20);
        pos += 4;
        for k in 0..count {
            if pos + 4 > dir.end {
                return;
            }
            let off = r::u32_at(doc, pos).unwrap_or(0) as usize;
            offsets.entry(id + k).or_insert(off);
            pos += 4;
        }
    }
}

/// Top-level records of the PowerPoint Document stream are contiguous; the newest
/// UserEditAtom is the last one written.
fn last_edit_in_stream(doc: &[u8]) -> Option<usize> {
    let mut off = 0;
    let mut last = None;
    while let Some(rec) = r::at(doc, off) {
        if rec.kind == r::USER_EDIT_ATOM {
            last = Some(off);
        }
        off = rec.end;
    }
    last
}
