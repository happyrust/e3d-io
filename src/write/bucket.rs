//! Page 1 on write: the bucket counters a [`Transaction`](super::Transaction)
//! raises in place (S6b, requirement 13; `docs/adr/0003-page-1-in-place-bucket-write.md`).
//!
//! E3D hands out a new element's `word1` from the counter on physical page 1
//! ([`crate::meta::bucket`]) and reuses a number e3d-io issued and deleted when
//! the counter never passed it (`bucket.collision`). It reads and writes page 1
//! in place, not through page 0 (`bucket.persist`), so the counter is raised
//! where it lives.
//!
//! Each listed extent's counter becomes `max(counter, required)`, `required`
//! being one past the highest `word1` of that extent in the pinned index (the
//! base, or Merge_Latest when merging) and in the batch's final inserts. Nothing
//! else on the page changes: entries keep their order, word 2 its count, and
//! every third word — E3D's claim — the value it had. An extent page 1 does not
//! list gets no entry.

use std::collections::BTreeMap;

use crate::core_rules::{BucketCollision, BucketPersist, CounterSemantics, RuleSet};
use crate::engine::{ReadOnlyEngine, ScanTier};
use crate::meta::bucket::{BucketTable, ENTRY_WORDS, HEADER_WORDS};
use crate::refno::RefNo;

use super::TransactionError;

/// Page 1 with the batch's counters raised, or `None` when it stays as it is:
/// no counter to raise, a page that does not read as a bucket table, an extent
/// listed twice, or a page-1 rule not yet confirmed. `engine` reads the pinned
/// session; `inserted` are the batch's final inserts.
pub(crate) fn raised_page1(
    rules: &RuleSet,
    engine: &mut ReadOnlyEngine,
    page1: &[u8],
    inserted: &[RefNo],
) -> Result<Option<Vec<u8>>, TransactionError> {
    if !writes_page1(rules) {
        return Ok(None);
    }
    let Ok(table) = BucketTable::parse(page1) else {
        return Ok(None);
    };
    let mut highest: BTreeMap<u32, Option<u32>> = BTreeMap::new();
    for entry in &table.entries {
        if highest.insert(entry.extent_word0, None).is_some() {
            return Ok(None);
        }
    }
    if highest.is_empty() {
        return Ok(None);
    }
    index_maxima(rules, engine, &mut highest)?;
    for refno in inserted {
        if let Some(max) = highest.get_mut(&refno.word0) {
            *max = Some(max.map_or(refno.word1, |m| m.max(refno.word1)));
        }
    }
    let mut required = BTreeMap::new();
    for (word0, max) in highest {
        if let Some(max) = max {
            required.insert(word0, max.checked_add(1).ok_or(TransactionError::RefnoExhausted { word0 })?);
        }
    }
    Ok(raise(page1, &required))
}

/// Whether the rules let the write side touch page 1 at all (D7: not until
/// collision, persistence and counter meaning are all confirmed).
fn writes_page1(rules: &RuleSet) -> bool {
    rules.bucket_collision() == Some(BucketCollision::HistoricalReuse)
        && rules.bucket_persist() == Some(BucketPersist::InPlace)
        && rules.counter_semantics() == Some(CounterSemantics::NextAvailable)
}

/// Each listed extent's highest `word1` in the engine's index: the
/// rightmost-path seek of the Open_Fast_Path when `alloc.index_order` is
/// confirmed, else one scan, as [`super::alloc`] does.
fn index_maxima(
    rules: &RuleSet,
    engine: &mut ReadOnlyEngine,
    highest: &mut BTreeMap<u32, Option<u32>>,
) -> Result<(), TransactionError> {
    if rules.index_order().is_some() {
        let root = engine.index_root();
        for (&word0, max) in highest.iter_mut() {
            let mut cursor = engine.index_cursor();
            let hit = match word0.checked_add(1) {
                Some(next) => cursor.last_before(RefNo::new(next, 0)),
                None => cursor.last(),
            }
            .map_err(|error| crate::error::E3dReaderError::IndexError { page_no: root.page, detail: error.to_string() })?;
            *max = hit.filter(|r| r.refno.word0 == word0).map(|r| r.refno.word1);
        }
    } else {
        for element in engine.scan_elements(ScanTier::Header)? {
            let refno = element?.refno;
            if let Some(max) = highest.get_mut(&refno.word0) {
                *max = Some(max.map_or(refno.word1, |m| m.max(refno.word1)));
            }
        }
    }
    Ok(())
}

/// `page1` with each listed extent's counter raised to its `required` value,
/// or `None` when none rises. Counters only go up, and only counter words are
/// written.
pub(crate) fn raise(page1: &[u8], required: &BTreeMap<u32, u32>) -> Option<Vec<u8>> {
    let table = BucketTable::parse(page1).ok()?;
    let mut page = page1.to_vec();
    let mut raised = false;
    for (index, entry) in table.entries.iter().enumerate() {
        let Some(&floor) = required.get(&entry.extent_word0) else {
            continue;
        };
        if floor > entry.next_word1 {
            super::pages::put_be_word(&mut page, (HEADER_WORDS + index * ENTRY_WORDS + 1) * 4, floor);
            raised = true;
        }
    }
    raised.then_some(page)
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

    fn words(page: &[u8], count: usize) -> Vec<u32> {
        (0..count).map(|i| u32::from_be_bytes(page[i * 4..i * 4 + 4].try_into().unwrap())).collect()
    }

    const AMS8000: [u32; 9] = [1, 0xFFFF_FFFF, 8, 24_384, 26_506, 0, 32_576, 98, 0];

    #[test]
    fn a_counter_below_its_floor_rises_and_nothing_else_moves() {
        let before = page(&AMS8000);
        let after = raise(&before, &BTreeMap::from([(24_384, 26_509)])).unwrap();
        assert_eq!(words(&after, 9), [1, 0xFFFF_FFFF, 8, 24_384, 26_509, 0, 32_576, 98, 0]);
        assert_eq!(after[36..], before[36..]);
    }

    #[test]
    fn a_counter_never_goes_down() {
        let before = page(&AMS8000);
        assert_eq!(raise(&before, &BTreeMap::from([(24_384, 26_000)])), None);
        assert_eq!(raise(&before, &BTreeMap::from([(24_384, 26_506)])), None, "equal is not a raise");
    }

    #[test]
    fn the_third_word_is_kept_as_found() {
        let mut claimed = AMS8000;
        claimed[5] = 4;
        let after = raise(&page(&claimed), &BTreeMap::from([(24_384, 26_510), (32_576, 120)])).unwrap();
        assert_eq!(words(&after, 9), [1, 0xFFFF_FFFF, 8, 24_384, 26_510, 4, 32_576, 120, 0]);
    }

    #[test]
    fn an_extent_page_1_does_not_list_gets_no_entry() {
        assert_eq!(raise(&page(&AMS8000), &BTreeMap::from([(16_192, 900)])), None);
        assert_eq!(raise(&page(&[1, 0xFFFF_FFFF, 2]), &BTreeMap::from([(13_292, 5)])), None, "a dictionary with no entries");
    }

    #[test]
    fn a_page_that_is_not_a_bucket_table_is_left_alone() {
        let mut not_a_bucket = AMS8000;
        not_a_bucket[0] = 0;
        assert_eq!(raise(&page(&not_a_bucket), &BTreeMap::from([(24_384, 30_000)])), None);
    }

    #[test]
    fn the_confirmed_rules_write_page_1() {
        assert!(writes_page1(RuleSet::confirmed()));
    }
}
