//! Keep unsupported shape appearance separate from complete textbox text ownership.
use super::{Record, defaults, unsupported};
use crate::office::doc::{Error, Result, u16_at, u32_at};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Appearance {
    pub properties: Vec<(u16, u32)>,
    pub source_next_shape: Option<u32>,
    pub wrap_distances: [Option<i32>; 4],
    pub approximations: Vec<defaults::DefaultEvidence>,
}

pub(super) fn resolve(
    records: &[Record<'_>],
    inherited: &BTreeMap<u16, u32>,
    shape_id: u32,
) -> Result<Appearance> {
    let mut local = BTreeMap::new();
    let mut uncertain = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut approximations = Vec::new();
    for record in records {
        if !matches!(record.kind, 0xf00b | 0xf121 | 0xf122) || !seen.insert(record.kind) {
            return Err(unsupported());
        }
        let evidence = defaults::decode(record)?;
        let values = (0..usize::from(record.instance))
            .map(|i| Ok((u16_at(record.data, i * 6)?, u32_at(record.data, i * 6 + 2)?)))
            .collect::<Result<Vec<_>>>()?;
        if record.kind == 0xf122
            && values
                == [
                    (0x53f, 0x10000),
                    (0x57f, 0x80000),
                    (0x5bf, 0x80000),
                    (0x5ff, 0x80000),
                    (0x63f, 0x80000),
                ]
        {
            continue;
        }
        let mut partial = false;
        for (raw, value) in values {
            let key = raw & 0x3fff;
            if matches!(key, 0x80 | 0x8a) && raw != key {
                return Err(Error::Unsupported("textbox source identity property flags"));
            }
            if matches!(key, 0x81..=0x84) && (raw != key || value > 0x0132_f540) {
                return Err(Error::Unsupported("textbox inset property"));
            }
            if matches!(key, 0x384..=0x387) && raw != key {
                return Err(Error::Unsupported("textbox wrap distance property flags"));
            }
            let supported = matches!(
                (raw, value),
                (0x80, _)
                    | (0x81..=0x84, _)
                    | (0x87, 0)
                    | (0x8a, _)
                    | (0x384..=0x387, _)
                    | (0x1bf, 0x100010 | 0x100000)
                    | (0x1ff, 0x80000)
            ) || (raw == 0x181 && value >> 24 == 0);
            if !supported {
                uncertain.insert(key);
                local.remove(&key);
                partial = true;
                continue;
            }
            if uncertain.contains(&key) {
                partial = true;
                continue;
            }
            if local.get(&key).is_some_and(|old| *old != value) {
                return Err(Error::Unsupported("conflicting textbox property tables"));
            }
            // Retain the raw record even when wrapping consumes these values;
            // their application units remain an explicit layout inference.
            partial |= matches!(key, 0x384..=0x387);
            local.insert(key, value);
        }
        if partial && let Some(mut evidence) = evidence {
            evidence.limitation = "Unsupported shape paint and text-frame effects remain unapplied; source text and supported insets are retained and painted with explicit partial fidelity.";
            approximations.push(evidence);
        }
    }
    let wrap_distances =
        std::array::from_fn(|i| local.remove(&(0x384 + i as u16)).map(|v| v as i32));
    let source_next_shape = local.remove(&0x8a);
    if source_next_shape.is_some_and(|next| next != 0 && next != shape_id) {
        return Err(Error::Unsupported("render textbox linked target"));
    }
    let mut properties = inherited.clone();
    properties.extend(local);
    Ok(Appearance {
        properties: properties.into_iter().collect(),
        source_next_shape,
        wrap_distances,
        approximations,
    })
}

#[cfg(test)]
mod tests;
