//! A sparse, bounded cache of VOB sectors, filled by the host as data arrives
//! (e.g. from HTTP range requests) and read by the engine and the decoders.
//!
//! Sectors are stored in chunks of [`CHUNK`] sectors. A VOB set must be
//! declared with its size before its sectors are used; an undeclared VOB set
//! counts as stripped. Reading a declared sector that hasn't arrived yet
//! reports it as pending, and [`SectorCache::wanted`] tells the host which
//! chunks to fetch next.

use std::collections::{HashMap, VecDeque};

use crate::navigator::{NavRead, NavSource, Vobs};
use crate::ifo::SECTOR;
use crate::nav::NavPack;

/// Sectors per chunk (128 KiB).
pub const CHUNK: u32 = 64;

#[derive(Debug)]
pub struct SectorCache {
    /// Declared VOB sets and their sizes in sectors.
    sizes: HashMap<Vobs, u32>,
    chunks: HashMap<(Vobs, u32), Vec<u8>>,
    /// Least recently used first.
    lru: VecDeque<(Vobs, u32)>,
    /// Upper bound on cached chunks.
    budget: usize,
    /// Chunks the reader asked for that aren't cached, oldest request first.
    misses: VecDeque<(Vobs, u32)>,
}

impl SectorCache {
    /// A cache holding at most `budget_bytes` of sector data.
    pub fn new(budget_bytes: usize) -> Self {
        SectorCache {
            sizes: HashMap::new(),
            chunks: HashMap::new(),
            lru: VecDeque::new(),
            budget: (budget_bytes / (CHUNK as usize * SECTOR)).max(4),
            misses: VecDeque::new(),
        }
    }

    /// Declare a VOB set of `sectors` sectors.
    pub fn declare(&mut self, vobs: Vobs, sectors: u32) {
        self.sizes.insert(vobs, sectors);
    }

    pub fn size(&self, vobs: Vobs) -> Option<u32> {
        self.sizes.get(&vobs).copied()
    }

    /// The byte range of `chunk` within the VOB set, clipped to its size.
    pub fn chunk_range(&self, vobs: Vobs, chunk: u32) -> Option<(u64, u64)> {
        let size = self.size(vobs)?;
        let first = chunk * CHUNK;
        if first >= size {
            return None;
        }
        let last = (first + CHUNK).min(size);
        Some((first as u64 * SECTOR as u64, last as u64 * SECTOR as u64))
    }

    fn touch(&mut self, key: (Vobs, u32)) {
        if let Some(i) = self.lru.iter().position(|k| *k == key) {
            self.lru.remove(i);
        }
        self.lru.push_back(key);
    }

    /// Store the data of `chunk` (whole sectors, the last chunk may be short).
    pub fn insert_chunk(&mut self, vobs: Vobs, chunk: u32, data: Vec<u8>) {
        let key = (vobs, chunk);
        self.misses.retain(|k| *k != key);
        self.chunks.insert(key, data);
        self.touch(key);
        while self.lru.len() > self.budget {
            if let Some(old) = self.lru.pop_front() {
                self.chunks.remove(&old);
            }
        }
    }

    pub fn has_chunk(&self, vobs: Vobs, chunk: u32) -> bool {
        self.chunks.contains_key(&(vobs, chunk))
    }

    /// One sector, if cached. A miss is remembered for [`Self::wanted`].
    pub fn sector(&mut self, vobs: Vobs, lba: u32) -> Option<&[u8]> {
        let key = (vobs, lba / CHUNK);
        if !self.chunks.contains_key(&key) {
            if self.size(vobs).is_some_and(|s| lba < s) && !self.misses.contains(&key) {
                self.misses.push_back(key);
            }
            return None;
        }
        self.touch(key);
        let off = (lba % CHUNK) as usize * SECTOR;
        self.chunks.get(&key)?.get(off..off + SECTOR)
    }

    /// Sectors `first..=last` concatenated, if all are cached.
    pub fn range(&mut self, vobs: Vobs, first: u32, last: u32) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity((last - first + 1) as usize * SECTOR);
        let mut missing = false;
        for lba in first..=last {
            match self.sector(vobs, lba) {
                Some(s) => out.extend_from_slice(s),
                None => missing = true,
            }
        }
        (!missing).then_some(out)
    }

    /// Chunks to fetch: every reported miss, then `ahead` chunks following
    /// `lba` in `vobs` (read-ahead), skipping what is cached.
    pub fn wanted(&self, around: Option<(Vobs, u32)>, ahead: u32) -> Vec<(Vobs, u32)> {
        let mut out: Vec<(Vobs, u32)> = self.misses.iter().copied().collect();
        if let Some((vobs, lba)) = around {
            if let Some(size) = self.size(vobs) {
                let first = lba / CHUNK;
                let last = (first + ahead).min(size.saturating_sub(1) / CHUNK);
                for c in first..=last {
                    let key = (vobs, c);
                    if !self.chunks.contains_key(&key) && !out.contains(&key) {
                        out.push(key);
                    }
                }
            }
        }
        out
    }
}

impl NavSource for SectorCache {
    fn nav_pack(&mut self, vobs: Vobs, lba: u32) -> Result<NavRead, String> {
        if self.size(vobs).is_none() {
            return Ok(NavRead::Stripped);
        }
        match self.sector(vobs, lba) {
            None => Ok(NavRead::Pending),
            Some(s) => match NavPack::parse(s) {
                Ok(Some(p)) => Ok(NavRead::Pack(p)),
                Ok(None) => Err(format!("sector {lba} is not a NAV pack")),
                Err(e) => Err(e.to_string()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn misses_become_wanted_and_eviction_is_lru() {
        let mut c = SectorCache::new(4 * CHUNK as usize * SECTOR);
        let v = Vobs::Menu(1);
        assert!(matches!(c.nav_pack(Vobs::Title(1), 0), Ok(NavRead::Stripped)));
        c.declare(v, 10 * CHUNK);
        assert!(matches!(c.nav_pack(v, 70), Ok(NavRead::Pending)));
        assert_eq!(c.wanted(None, 0), vec![(v, 1)]);
        assert_eq!(c.wanted(Some((v, 0)), 2), vec![(v, 1), (v, 0), (v, 2)]);
        for chunk in 0..5 {
            c.insert_chunk(v, chunk, vec![0; CHUNK as usize * SECTOR]);
        }
        assert!(!c.has_chunk(v, 0), "oldest chunk evicted");
        assert!(c.has_chunk(v, 4));
        assert!(c.wanted(None, 0).is_empty());
        assert_eq!(c.range(v, 64, 65).map(|d| d.len()), Some(2 * SECTOR));
    }
}
