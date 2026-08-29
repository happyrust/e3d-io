//! A fixed pool of page buffers in front of a [`PageSource`].
//!
//! The pool exists so that reading a page costs no allocation. The cache this
//! replaces called `read_page() -> Vec<u8>` on every miss, so one B+ tree
//! descent allocated and freed one vector per level.

use std::collections::HashMap;

use super::io::PageReader;
use super::{IoStats, PageId, PageSize, PageSource, ReadIntent};
use crate::error::E3dReaderError;

/// One reusable page buffer.
struct Slot {
    /// The page currently held, or `None` when the slot is free. A slot whose
    /// read failed is left free rather than left claiming the page it failed
    /// to read.
    id: Option<PageId>,
    buf: Box<[u8]>,
    /// Second-chance bit, set on every hit and cleared as the hand passes.
    referenced: bool,
}

/// Read-only page cache over a bounded pool of buffers.
///
/// Replacement is second-chance (CLOCK), not strict LRU: eviction is O(1)
/// amortised and needs no ordering list. Nothing should depend on *which* page
/// is evicted, only that the pool stays within `capacity`.
pub struct PageCache {
    source: PageReader,
    page_bytes: usize,
    capacity: usize,
    slots: Vec<Slot>,
    slot_of: HashMap<PageId, usize>,
    hand: usize,
    hits: u64,
    misses: u64,
}

impl PageCache {
    pub fn new(source: PageReader, capacity: usize) -> Self {
        let page_bytes = source.page_size().bytes();
        let capacity = capacity.max(1);
        Self {
            source,
            page_bytes,
            capacity,
            slots: Vec::new(),
            slot_of: HashMap::with_capacity(capacity),
            hand: 0,
            hits: 0,
            misses: 0,
        }
    }

    /// Page size in bytes.
    pub fn page_size(&self) -> u32 {
        self.page_bytes as u32
    }

    /// Page size as the unit-carrying type; prefer this over [`Self::page_size`].
    pub fn page_size_typed(&self) -> PageSize {
        self.source.page_size()
    }

    pub fn physical_page_count(&self) -> u32 {
        self.source.physical_page_count()
    }

    /// Fetch a page of the primary extent.
    pub fn get_page(&mut self, page_no: u32) -> Result<&[u8], E3dReaderError> {
        self.get(PageId::primary(page_no))
    }

    /// Fetch a page by full `(extent, page)` address.
    pub fn get(&mut self, id: PageId) -> Result<&[u8], E3dReaderError> {
        let slot = match self.slot_of.get(&id).copied() {
            Some(slot) => {
                self.hits += 1;
                self.slots[slot].referenced = true;
                slot
            }
            None => {
                self.misses += 1;
                self.load(id, ReadIntent::Demand)?
            }
        };
        Ok(&self.slots[slot].buf[..self.page_bytes])
    }

    /// Warm the pool with pages the caller expects to want.
    ///
    /// Charged to `IoStats::prefetched_pages`, never to `pages_read`, so a
    /// caller measuring the cost of a lookup as a difference of `pages_read`
    /// snapshots is not billed for the guesses. A failed speculative read is
    /// still returned: this layer does not decide that a read failure was
    /// unimportant.
    pub fn prefetch(
        &mut self,
        ids: impl IntoIterator<Item = PageId>,
    ) -> Result<(), E3dReaderError> {
        for id in ids {
            if self.slot_of.contains_key(&id) {
                continue;
            }
            self.load(id, ReadIntent::Prefetch)?;
        }
        Ok(())
    }

    /// Read counters of the underlying source, plus this cache's hit rate.
    pub fn stats(&self) -> IoStats {
        IoStats {
            cache_hits: self.hits,
            cache_misses: self.misses,
            ..self.source.stats()
        }
    }

    pub fn cached_count(&self) -> usize {
        self.slot_of.len()
    }

    pub fn reader(&self) -> &PageReader {
        &self.source
    }

    fn load(&mut self, id: PageId, intent: ReadIntent) -> Result<usize, E3dReaderError> {
        let slot = self.claim_slot();

        // Split the borrow: the read takes `&self.source` while filling
        // `&mut self.slots[slot].buf`.
        let Self { source, slots, .. } = self;
        source.read_with_intent(id, &mut slots[slot].buf, intent)?;

        self.slots[slot].id = Some(id);
        self.slots[slot].referenced = true;
        self.slot_of.insert(id, slot);
        Ok(slot)
    }

    /// Return a free slot, evicting by second chance once the pool is full.
    fn claim_slot(&mut self) -> usize {
        if self.slots.len() < self.capacity {
            self.slots.push(Slot {
                id: None,
                buf: vec![0u8; self.page_bytes].into_boxed_slice(),
                referenced: false,
            });
            return self.slots.len() - 1;
        }

        loop {
            let candidate = self.hand;
            self.hand = (self.hand + 1) % self.slots.len();
            if self.slots[candidate].referenced {
                self.slots[candidate].referenced = false;
                continue;
            }
            if let Some(evicted) = self.slots[candidate].id.take() {
                self.slot_of.remove(&evicted);
            }
            return candidate;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    const WORDS: u32 = 32;
    const PAGE_BYTES: usize = WORDS as usize * 4;

    fn db_file(dir: &Path, pages: usize) -> PathBuf {
        let mut bytes = vec![0u8; PAGE_BYTES * pages];
        bytes[0x34..0x38].copy_from_slice(&WORDS.to_be_bytes());
        for page in 0..pages {
            bytes[page * PAGE_BYTES] = page as u8;
        }
        let path = dir.join("ams1000_0001");
        let mut file = File::create(&path).unwrap();
        file.write_all(&bytes).unwrap();
        file.flush().unwrap();
        path
    }

    fn cache(dir: &Path, pages: usize, capacity: usize) -> PageCache {
        PageCache::new(PageReader::open(db_file(dir, pages)).unwrap(), capacity)
    }

    #[test]
    fn physical_page_count_is_exposed_through_cache() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path(), 2, 4);
        assert_eq!(cache.page_size(), PAGE_BYTES as u32);
        assert_eq!(cache.physical_page_count(), 2);
    }

    #[test]
    fn a_repeated_page_is_served_from_the_pool() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = cache(dir.path(), 4, 4);

        assert_eq!(cache.get_page(2).unwrap()[0], 2);
        assert_eq!(cache.get_page(2).unwrap()[0], 2);

        let stats = cache.stats();
        assert_eq!(stats.cache_misses, 1);
        assert_eq!(stats.cache_hits, 1);
        assert_eq!(stats.pages_read, 1, "the second get must not touch disk");
    }

    #[test]
    fn the_pool_never_exceeds_its_capacity() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = cache(dir.path(), 16, 3);
        for page in 0..16 {
            assert_eq!(cache.get_page(page).unwrap()[0], page as u8);
        }
        assert!(cache.cached_count() <= 3, "{}", cache.cached_count());
        assert_eq!(cache.stats().pages_read, 16);
    }

    #[test]
    fn a_prefetched_page_is_a_hit_and_is_not_billed_as_a_read() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = cache(dir.path(), 4, 4);

        cache
            .prefetch([PageId::primary(1), PageId::primary(3)])
            .unwrap();
        assert_eq!(cache.get_page(1).unwrap()[0], 1);

        let stats = cache.stats();
        assert_eq!(stats.prefetched_pages, 2);
        assert_eq!(stats.pages_read, 0);
        assert_eq!(stats.cache_hits, 1);
        assert_eq!(stats.cache_misses, 0);
    }

    #[test]
    fn a_failed_read_leaves_no_slot_claiming_the_page() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = cache(dir.path(), 2, 4);

        assert!(cache.get_page(0).is_ok());
        assert!(cache.get_page(7).is_err());
        assert_eq!(cache.cached_count(), 1);
        // The failure must be reproducible rather than answered from a slot
        // left holding whatever the failed read wrote.
        assert!(cache.get_page(7).is_err());
    }
}
