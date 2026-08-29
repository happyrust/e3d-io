use std::io::Write;

use e3d_io::page::cache::PageCache;
use e3d_io::page::io::PageReader;

#[test]
fn cache_reports_complete_physical_pages() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    let mut bytes = vec![0u8; 2 * 4096 + 19];
    bytes[0x34..0x38].copy_from_slice(&1024u32.to_be_bytes());
    file.write_all(&bytes).unwrap();
    let cache = PageCache::new(PageReader::open(file.path()).unwrap(), 4);
    assert_eq!(cache.physical_page_count(), 2);
}
