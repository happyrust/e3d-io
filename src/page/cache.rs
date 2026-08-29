use std::collections::HashMap;

use super::io::PageReader;
use crate::error::E3dReaderError;

/// Read-only page cache. Keeps recently read pages in memory.
///
/// No dirty tracking needed since we're read-only.
pub struct PageCache {
    reader: PageReader,
    pages: HashMap<u32, Vec<u8>>,
    capacity: usize,
    access_order: Vec<u32>,
}

impl PageCache {
    pub fn new(reader: PageReader, capacity: usize) -> Self {
        Self {
            reader,
            pages: HashMap::with_capacity(capacity),
            capacity: capacity.max(4),
            access_order: Vec::with_capacity(capacity),
        }
    }

    pub fn page_size(&self) -> u32 {
        self.reader.page_size()
    }

    pub fn physical_page_count(&self) -> u32 {
        self.reader.physical_page_count()
    }

    /// Get a page, reading from disk if not cached.
    pub fn get_page(&mut self, page_no: u32) -> Result<&[u8], E3dReaderError> {
        if !self.pages.contains_key(&page_no) {
            if self.pages.len() >= self.capacity {
                self.evict_oldest();
            }
            let data = self.reader.read_page(page_no)?;
            self.pages.insert(page_no, data);
        }
        self.touch(page_no);
        Ok(self.pages.get(&page_no).unwrap())
    }

    /// Read a page without caching (for one-off reads).
    pub fn read_uncached(&mut self, page_no: u32) -> Result<Vec<u8>, E3dReaderError> {
        self.reader.read_page(page_no)
    }

    pub fn cached_count(&self) -> usize {
        self.pages.len()
    }

    pub fn reader(&self) -> &PageReader {
        &self.reader
    }

    fn touch(&mut self, page_no: u32) {
        if let Some(pos) = self.access_order.iter().position(|&p| p == page_no) {
            self.access_order.remove(pos);
        }
        self.access_order.push(page_no);
    }

    fn evict_oldest(&mut self) {
        if let Some(oldest) = self.access_order.first().copied() {
            self.access_order.remove(0);
            self.pages.remove(&oldest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn physical_page_count_is_exposed_through_cache() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let mut bytes = vec![0u8; 2 * 4096];
        bytes[0x34..0x38].copy_from_slice(&1024u32.to_be_bytes());
        file.write_all(&bytes).unwrap();
        let cache = PageCache::new(PageReader::open(file.path()).unwrap(), 4);
        assert_eq!(cache.physical_page_count(), 2);
    }
}
