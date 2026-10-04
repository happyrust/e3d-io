//! The page-1 bucket table: `core.dll`'s per-extent refno counter
//! (`docs/evidence/2026-09-22-core-dll-refno-allocation.md`, S6b read side).
//!
//! Physical page 1 of a dabacon file is a **bucket page** (its first word is the
//! page type 1, as page 0 is 0, a session page 3 and an index page 5). It holds,
//! for each extent that has grown, the next `word1` to hand out there — the
//! counter `DB_DB::updateBucketList` rebuilds from the live elements at save
//! (`db_get_bucket` / `db_add_bucket`, routine 360). A new element takes its
//! `word1` from this counter, which is why a database that has deleted elements
//! keeps allocating past the highest live `word1`: the number is not reused.
//!
//! Layout (big-endian words), read off four installed databases:
//!
//! ```text
//! word 0 : 1            page type (bucket)
//! word 1 : 0xFFFFFFFF   marker
//! word 2 : 2 + 3·n      n = number of entries below
//! word 3.. : { [extent_word0][next_word1][0] }·n
//! ```
//!
//! Only the **secondary** extents appear — the ones E3D allocates new elements
//! into. The primary extent (`primary_extent_ref`) holds the bootstrap `WORLD`
//! and `MNUM` and does not grow, so it has no entry; a database whose elements
//! are all in the primary extent (a small dictionary) has no entries at all.
//! `ams8000_0001`: `(24384, 26496)` `(32576, 98)`; `ams251142_0001`:
//! `(2013287686, 947)`; `TES023`: `(17419, 331)`; `ams5100_0001`: none.
//!
//! This crate reads the table (to know what the next number would be) but does
//! not maintain it on write yet: the write path hands out `max(word1) + 1` in
//! the owner's extent and leaves page 1 alone (a page-0 re-read / the sentinel
//! lock guards a concurrent writer). Because `updateBucketList` re-derives the
//! counter from the elements at save, `max + 1` is what E3D converges to; the
//! remaining gap is the reserve E3D keeps above the live maximum. Maintaining
//! page 1 on write is tracked as S6b in the plan.

/// One extent's next-`word1` counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BucketEntry {
    /// The extent's `RefNo::word0` prefix — `primary_extent_ref + 0x2000·(ext-1)`.
    pub extent_word0: u32,
    /// The next `word1` E3D would hand out in this extent: above every live
    /// `word1` there (a deleted number is not reused).
    pub next_word1: u32,
    /// The entry's third word. Zero on every installed database; carried so a
    /// round-trip is exact and a non-zero value is not silently dropped.
    pub extra: u32,
}

/// The page-1 bucket table: one [`BucketEntry`] per grown extent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BucketTable {
    pub entries: Vec<BucketEntry>,
}

/// The bucket page's type word (word 0).
pub const BUCKET_PAGE_TYPE: u32 = 1;
/// The marker at word 1.
const MARKER: u32 = 0xFFFF_FFFF;
/// Words of the fixed header before the entries.
const HEADER_WORDS: usize = 3;
/// Words per entry.
const ENTRY_WORDS: usize = 3;

impl BucketTable {
    /// Parse physical page 1. `None`-shaped errors are returned as `Err(String)`
    /// naming what did not fit the shape, so a page that is not a bucket table is
    /// refused rather than read as garbage.
    pub fn parse(page: &[u8]) -> Result<Self, String> {
        let word = |i: usize| -> Result<u32, String> {
            page.get(i * 4..i * 4 + 4)
                .map(|b| u32::from_be_bytes(b.try_into().unwrap()))
                .ok_or_else(|| format!("bucket page is too short for word {i} ({} bytes)", page.len()))
        };
        let page_type = word(0)?;
        if page_type != BUCKET_PAGE_TYPE {
            return Err(format!("page 1 word 0 is {page_type}, not the bucket page type {BUCKET_PAGE_TYPE}"));
        }
        let marker = word(1)?;
        if marker != MARKER {
            return Err(format!("page 1 word 1 is {marker:#x}, not the bucket marker {MARKER:#x}"));
        }
        // Word 2 is `2 + 3·n`. Take the entry count from it, and stop early at a
        // zero extent word so a shorter-than-stated table is read, not overrun.
        let n = word(2)?;
        if n < 2 || !(n - 2).is_multiple_of(ENTRY_WORDS as u32) {
            return Err(format!("bucket word count {n} is not 2 + 3*n"));
        }
        let count = ((n - 2) / ENTRY_WORDS as u32) as usize;
        let available = (page.len() / 4).saturating_sub(HEADER_WORDS) / ENTRY_WORDS;
        if count > available {
            return Err(format!("bucket page claims {count} entries but holds at most {available}"));
        }
        let mut entries = Vec::with_capacity(count);
        for index in 0..count {
            let base = HEADER_WORDS + index * ENTRY_WORDS;
            let extent_word0 = word(base)?;
            if extent_word0 == 0 {
                break;
            }
            entries.push(BucketEntry {
                extent_word0,
                next_word1: word(base + 1)?,
                extra: word(base + 2)?,
            });
        }
        Ok(Self { entries })
    }

    /// The next `word1` E3D would hand out in `extent_word0`, if the table names
    /// that extent (the primary extent and an ungrown one are not named).
    pub fn next_word1(&self, extent_word0: u32) -> Option<u32> {
        self.entries.iter().find(|e| e.extent_word0 == extent_word0).map(|e| e.next_word1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(words: &[u32]) -> Vec<u8> {
        let mut page = vec![0u8; 2048];
        for (i, w) in words.iter().enumerate() {
            page[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
        }
        page
    }

    #[test]
    fn two_entries_are_read_as_the_ams8000_bucket_table() {
        let table = BucketTable::parse(&page(&[1, 0xFFFF_FFFF, 8, 24_384, 26_496, 0, 32_576, 98, 0])).unwrap();
        assert_eq!(
            table.entries,
            vec![
                BucketEntry { extent_word0: 24_384, next_word1: 26_496, extra: 0 },
                BucketEntry { extent_word0: 32_576, next_word1: 98, extra: 0 },
            ]
        );
        assert_eq!(table.next_word1(24_384), Some(26_496));
        assert_eq!(table.next_word1(32_576), Some(98));
        assert_eq!(table.next_word1(16_192), None, "the primary extent has no counter");
    }

    #[test]
    fn no_entries_is_a_dictionary_that_never_grew() {
        let table = BucketTable::parse(&page(&[1, 0xFFFF_FFFF, 2])).unwrap();
        assert!(table.entries.is_empty());
        assert_eq!(table.next_word1(13_292), None);
    }

    #[test]
    fn a_page_that_is_not_a_bucket_table_is_refused() {
        assert!(BucketTable::parse(&page(&[0, 0xFFFF_FFFF, 2])).is_err(), "page type 0 is page 0, not a bucket");
        assert!(BucketTable::parse(&page(&[1, 0, 2])).is_err(), "the marker must be present");
        assert!(BucketTable::parse(&[0u8; 4]).is_err(), "a truncated page is refused");
    }

    #[test]
    fn a_zero_extent_word_stops_the_walk() {
        // Word 2 claims three entries, but the second is zeroed: read one.
        let table = BucketTable::parse(&page(&[1, 0xFFFF_FFFF, 11, 24_384, 26_496, 0, 0, 0, 0])).unwrap();
        assert_eq!(table.entries.len(), 1);
    }

    #[test]
    fn malformed_count_is_refused_before_allocating_entries() {
        for count in [0, 1, 3, 4, 6, u32::MAX] {
            assert!(BucketTable::parse(&page(&[1, MARKER, count])).is_err(), "bad word count {count}");
        }
        assert!(BucketTable::parse(&[1u32, MARKER, 5].into_iter().flat_map(u32::to_be_bytes).collect::<Vec<_>>()).is_err());
    }
}
