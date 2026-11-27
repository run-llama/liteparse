//! Source-order counters shared by list identifiers, with per-override first-use resets.
use crate::office::doc::{Error, Result};
use std::collections::{BTreeMap, BTreeSet};

// Weighted work charges bound stored event and family counts.
const MAX_WORK: usize = 8 * 1024 * 1024;

#[derive(Default)]
struct Family {
    positions: [Vec<usize>; 9],
    starts: [Option<(usize, i32)>; 9],
    seen: BTreeSet<(usize, u8)>,
    last_cp: Option<usize>,
}

pub(super) struct Counters {
    families: BTreeMap<u32, Family>,
    remaining_work: usize,
}
impl Default for Counters {
    fn default() -> Self {
        Self {
            families: BTreeMap::new(),
            remaining_work: MAX_WORK,
        }
    }
}
impl Counters {
    /// The caller resolves the counter initial separately from the presentation LVL.
    /// The presentation LVL supplies the restart threshold.
    /// A first use of an override resets before its paragraph; later switchbacks continue
    /// the shared LSID sequence. Ancestor display lookup remains separate and iLfo-scoped.
    #[expect(
        clippy::too_many_arguments,
        reason = "each argument is a separate list counter input"
    )]
    pub(super) fn advance(
        &mut self,
        lsid: u32,
        ilfo: usize,
        level: u8,
        cp: usize,
        initial: i32,
        threshold: u8,
        start_override: Option<i32>,
    ) -> Result<i32> {
        if level > 8
            || threshold > level
            || initial < 0
            || start_override.is_some_and(|v| !(0..=0x7fff).contains(&v))
        {
            return Err(Error::Invalid("list counter parameters"));
        }
        let fresh = !self.families.contains_key(&lsid);
        let cost = 64
            + if fresh {
                std::mem::size_of::<Family>() + 64
            } else {
                0
            };
        self.remaining_work = self
            .remaining_work
            .checked_sub(cost)
            .ok_or(Error::Limit("list counter history"))?;
        let family = self.families.entry(lsid).or_default();
        if family.last_cp.is_some_and(|last| cp <= last) {
            return Err(Error::Invalid("list counter source order"));
        }
        let index = usize::from(level);
        if family.seen.insert((ilfo, level))
            && let Some(value) = start_override
        {
            family.starts[index] = Some((cp, value));
        }
        let lower = family.positions[..usize::from(threshold)]
            .iter()
            .filter_map(|v| v.last().copied())
            .max();
        // Lower-level restarts use this paragraph's effective initial value; explicit
        // first-use resets retain the value declared by the historical override.
        let (anchor, origin) = match (lower, family.starts[index]) {
            (Some(a), Some((b, value))) if b > a => (Some(b), value),
            (Some(a), _) => (Some(a), initial),
            (None, Some((b, value))) => (Some(b), value),
            (None, None) => (None, initial),
        };
        let prior = &mut family.positions[index];
        let first = anchor.map_or(0, |at| prior.partition_point(|&position| position < at));
        let count =
            i32::try_from(prior.len() - first).map_err(|_| Error::Limit("list sequence value"))?;
        let value = origin
            .checked_add(count)
            .ok_or(Error::Limit("list sequence value"))?;
        prior.push(cp);
        family.last_cp = Some(cp);
        Ok(value)
    }
}

#[cfg(test)]
mod tests;
