pub mod atnlog;
pub mod attrs;
pub mod axis_spec;
pub mod catalogue_expr;
pub mod catalogue_pml;
pub mod descriptor;
pub mod direction_spec;
pub mod element;
pub mod explicit;
pub mod orientation;
pub mod point_list;
pub mod template;
pub mod template_file;
pub mod text;

#[cfg(test)]
mod descriptor_export_tests {
    #[test]
    fn descriptor_module_exports_extraction_status() {
        let status = super::descriptor::AttributeExtractionStatus::Decoded;
        assert_eq!(
            status,
            super::descriptor::AttributeExtractionStatus::Decoded
        );
    }
}

use crate::error::E3dReaderError;
use crate::index::IndexEntry;
use crate::page::cache::PageCache;
use crate::refno::{PageAddress, RefNo};

/// Structured view of an element record read from the database.
#[derive(Debug, Clone)]
pub struct ElementRecordView {
    pub refno: RefNo,
    pub address: PageAddress,
    pub raw_bytes: Vec<u8>,
    pub page_no: u32,
    pub byte_offset: u32,
}

const END_MARKER: [u8; 4] = [0, 0, 0, 7];
const MAX_RECORD_READ: usize = 64 * 1024;

/// Read an element record starting at the given index entry location.
///
/// The record format (from pdmsdb_engine_v2/db4/record_reader.rs):
/// 1. Optional padding (zeros)
/// 2. impl_len (4 bytes BE, in 4-byte words) → data length in bytes = impl_len * 4
/// 3. Record data bytes
/// 4. Optional continuation segments
/// 5. End marker: 0x00000007
pub fn read_element_record(
    cache: &mut PageCache,
    entry: &IndexEntry,
) -> Result<ElementRecordView, E3dReaderError> {
    let page_size = cache.page_size() as usize;
    let byte_offset = (entry.offset_words as usize) * 2;
    let byte_offset_u32 = byte_offset as u32;

    let mut all_bytes = Vec::with_capacity(4096);

    let mut abs_offset = entry.page_no as u64 * page_size as u64 + byte_offset as u64;
    let mut total_read = 0usize;

    while total_read < MAX_RECORD_READ {
        let page_no = (abs_offset / page_size as u64) as u32;
        let offset_in_page = (abs_offset % page_size as u64) as usize;

        let page_data = match cache.get_page(page_no) {
            Ok(page) => page,
            Err(err) => {
                if let E3dReaderError::Io { source, .. } = &err {
                    if source.kind() == std::io::ErrorKind::UnexpectedEof {
                        return Err(truncated_record(
                            entry,
                            byte_offset_u32,
                            format!("reached EOF after {total_read} bytes without end marker"),
                        ));
                    }
                }
                return Err(err);
            }
        };
        let available = page_size.saturating_sub(offset_in_page);
        let chunk = &page_data[offset_in_page..offset_in_page + available];
        all_bytes.extend_from_slice(chunk);
        total_read += available;
        abs_offset += available as u64;

        if find_end_marker(&all_bytes).is_some() {
            break;
        }
    }

    if let Some(end) = find_end_marker(&all_bytes) {
        all_bytes.truncate(end);
    } else {
        return Err(truncated_record(
            entry,
            byte_offset_u32,
            format!("read safety limit of {MAX_RECORD_READ} bytes without end marker"),
        ));
    }

    Ok(ElementRecordView {
        refno: entry.refno,
        address: entry.to_page_address(),
        raw_bytes: all_bytes,
        page_no: entry.page_no,
        byte_offset: byte_offset_u32,
    })
}

fn truncated_record(entry: &IndexEntry, byte_offset: u32, detail: String) -> E3dReaderError {
    E3dReaderError::TruncatedRecord {
        refno: entry.refno,
        page_no: entry.page_no,
        byte_offset,
        detail,
    }
}

fn find_end_marker(data: &[u8]) -> Option<usize> {
    // Attribute payloads legitimately contain the integer value 7. The old
    // scanner treated the first aligned 0x00000007 anywhere in the implicit
    // region as EOF and truncated otherwise valid Dictionary UDA records.
    // `impl_len` includes the element header, so a record terminator cannot
    // occur before that declared boundary.
    let implicit_end = declared_implicit_end(data).unwrap_or(0);
    // Member and explicit blocks carry their own word length. Their payloads
    // may also contain an aligned integer 7, so the terminator scan must begin
    // after every complete block belonging to this element, not merely after
    // the implicit region.
    let start = declared_tail_end(data, implicit_end);
    for i in (start..data.len().saturating_sub(3)).step_by(4) {
        if data[i..i + 4] == END_MARKER {
            let looks_like_segment = i + 6 <= data.len()
                && data[i + 4] == 0x00
                && (data[i + 5] == 0x01 || data[i + 5] == 0x02);
            if !looks_like_segment {
                return Some(i + 4);
            }
        }
    }
    None
}

fn declared_tail_end(data: &[u8], start: usize) -> usize {
    let prefix = record_prefix(data);
    let Some(ref0) = read_be_u32(data, prefix + 4) else {
        return start;
    };
    let Some(ref1) = read_be_u32(data, prefix + 8) else {
        return start;
    };
    let mut cursor = start;
    loop {
        while data.get(cursor..cursor + 4).is_some_and(|word| {
            word == [0, 0, 0, 0]
                || (word == END_MARKER
                    && data
                        .get(cursor + 4..cursor + 6)
                        .is_some_and(|flag| flag == [0, 1] || flag == [0, 2]))
        }) {
            cursor += 4;
        }
        let Some(flag) = data
            .get(cursor..cursor + 2)
            .map(|bytes| u16::from_be_bytes(bytes.try_into().unwrap()))
        else {
            return cursor;
        };
        if flag != 1 && flag != 2 {
            return cursor;
        }
        let Some(words) = data
            .get(cursor + 2..cursor + 4)
            .map(|bytes| u16::from_be_bytes(bytes.try_into().unwrap()) as usize)
        else {
            return cursor;
        };
        let Some(end) = words
            .checked_mul(4)
            .and_then(|bytes| cursor.checked_add(bytes))
        else {
            return cursor;
        };
        if words < 3
            || read_be_u32(data, cursor + 4) != Some(ref0)
            || read_be_u32(data, cursor + 8) != Some(ref1)
        {
            return cursor;
        }
        if end > data.len() {
            return end;
        }
        cursor = end;
    }
}

fn read_be_u32(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        data.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn declared_implicit_end(data: &[u8]) -> Option<usize> {
    let prefix = record_prefix(data);
    let words = u32::from_be_bytes(data.get(prefix..prefix + 4)?.try_into().ok()?) as usize;
    let bytes = words.checked_mul(4)?;
    (words >= 6 && bytes <= MAX_RECORD_READ).then_some(prefix + bytes)
}

fn record_prefix(data: &[u8]) -> usize {
    let mut prefix = 0usize;
    while let Some(word) = data.get(prefix..prefix + 4) {
        if word != [0, 0, 0, 0] && word != [0, 0, 0, 7] {
            break;
        }
        prefix += 4;
    }
    prefix
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::io::PageReader;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    const TEST_PAGE_SIZE: usize = 128;

    fn test_cache(mut pages: Vec<Vec<u8>>) -> (TempDir, PageCache) {
        assert!(pages.len() >= 2);
        pages[0][0x34..0x38].copy_from_slice(&(TEST_PAGE_SIZE as u32 / 4).to_be_bytes());

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut file = File::create(&path).unwrap();
        for page in pages {
            assert_eq!(page.len(), TEST_PAGE_SIZE);
            file.write_all(&page).unwrap();
        }
        file.flush().unwrap();

        let reader = PageReader::open(&path).unwrap();
        (dir, PageCache::new(reader, 4))
    }

    fn entry(offset_words: u32) -> IndexEntry {
        IndexEntry {
            refno: RefNo::new(1, 1),
            page_no: 0,
            offset_words,
            flag: 0,
        }
    }

    #[test]
    fn reads_record_until_end_marker() {
        let mut page0 = vec![0u8; TEST_PAGE_SIZE];
        let page1 = vec![0u8; TEST_PAGE_SIZE];
        page0[64..80].copy_from_slice(&[0, 0, 0, 3, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 7]);
        let (_dir, mut cache) = test_cache(vec![page0, page1]);

        let view = read_element_record(&mut cache, &entry(32)).unwrap();
        assert_eq!(view.byte_offset, 64);
        assert_eq!(view.raw_bytes.len(), 16);
        assert_eq!(&view.raw_bytes[12..16], &END_MARKER);
    }

    #[test]
    fn reads_record_across_page_boundary() {
        let mut page0 = vec![0u8; TEST_PAGE_SIZE];
        let mut page1 = vec![0u8; TEST_PAGE_SIZE];
        page0[120..128].copy_from_slice(&[0, 0, 0, 3, 0, 0, 0, 1]);
        page1[0..8].copy_from_slice(&[0, 0, 0, 1, 0, 0, 0, 7]);
        let (_dir, mut cache) = test_cache(vec![page0, page1]);

        let view = read_element_record(&mut cache, &entry(60)).unwrap();
        assert_eq!(view.raw_bytes.len(), 16);
        assert_eq!(&view.raw_bytes[12..16], &END_MARKER);
    }

    #[test]
    fn missing_end_marker_returns_truncated_record() {
        let mut page0 = vec![0u8; TEST_PAGE_SIZE];
        let page1 = vec![0u8; TEST_PAGE_SIZE];
        page0[64..76].copy_from_slice(&[0, 0, 0, 3, 0, 0, 0, 1, 0, 0, 0, 1]);
        let (_dir, mut cache) = test_cache(vec![page0, page1]);

        let err = read_element_record(&mut cache, &entry(32)).unwrap_err();
        assert!(matches!(
            err,
            E3dReaderError::TruncatedRecord {
                page_no: 0,
                byte_offset: 64,
                ..
            }
        ));
    }

    #[test]
    fn segment_like_marker_is_not_record_end() {
        let mut page0 = vec![0u8; TEST_PAGE_SIZE];
        let page1 = vec![0u8; TEST_PAGE_SIZE];
        page0[64..88].copy_from_slice(&[
            0, 0, 0, 3, 0, 0, 0, 7, 0, 1, 0, 0, 0, 0, 0, 2, 0, 0, 0, 7, 0, 0, 0, 0,
        ]);
        let (_dir, mut cache) = test_cache(vec![page0, page1]);

        let view = read_element_record(&mut cache, &entry(32)).unwrap();
        assert_eq!(view.raw_bytes.len(), 20);
    }

    #[test]
    fn integer_seven_inside_declared_implicit_region_is_not_record_end() {
        let mut data = vec![0u8; 36];
        data[0..4].copy_from_slice(&8u32.to_be_bytes());
        data[12..16].copy_from_slice(&7u32.to_be_bytes());
        data[32..36].copy_from_slice(&7u32.to_be_bytes());
        assert_eq!(declared_implicit_end(&data), Some(32));
        assert_eq!(find_end_marker(&data), Some(36));
    }

    #[test]
    fn integer_seven_inside_declared_explicit_block_is_not_record_end() {
        let mut data = vec![0u8; 52];
        data[0..4].copy_from_slice(&6u32.to_be_bytes());
        data[4..8].copy_from_slice(&1u32.to_be_bytes());
        data[8..12].copy_from_slice(&1u32.to_be_bytes());
        data[24..26].copy_from_slice(&1u16.to_be_bytes());
        data[26..28].copy_from_slice(&6u16.to_be_bytes());
        data[28..32].copy_from_slice(&1u32.to_be_bytes());
        data[32..36].copy_from_slice(&1u32.to_be_bytes());
        data[36..40].copy_from_slice(&7u32.to_be_bytes());
        data[48..52].copy_from_slice(&END_MARKER);

        assert_eq!(declared_tail_end(&data, 24), 48);
        assert_eq!(find_end_marker(&data), Some(52));
    }
}
