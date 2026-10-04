//! Allocation uses a pinned index and the bucket page read when it opens.
use std::collections::{BTreeMap, HashMap};

use crate::core_rules::{CounterSemantics, RuleSet};
use crate::engine::{ReadOnlyEngine, ScanTier};
use crate::meta::bucket::BucketTable;
use crate::page::PageId;
use crate::refno::RefNo;

use super::TransactionError;

/// Whether page 1 contributed an allocation floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CounterUse {
    /// Byte-neutral optimization phase: keep the previous index-only policy.
    IndexOnly,
    /// Confirmed page-1 values are the first unused sequence numbers.
    NextAvailable,
    /// Counter interpretation is pending; only the index contributes.
    Unconfirmed,
    /// Page 1 is unavailable or malformed; only the index contributes.
    Unavailable,
}

/// The first allocation made in one extent, before the local counter advances.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtentAllocationBasis {
    pub index_max_word1: Option<u32>,
    pub bucket_floor: Option<u32>,
    pub first_word1: u32,
}

/// The snapshot and policy used by this transaction's allocator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocationBasis {
    pub index_session: u32,
    pub index_root: PageId,
    pub open_fast_path: bool,
    pub counter: CounterUse,
    pub bucket_error: Option<String>,
    pub extents: BTreeMap<u32, ExtentAllocationBasis>,
}

pub(crate) struct Allocator {
    floors: BTreeMap<u32, u32>,
    next: HashMap<u32, u32>,
    pub(crate) basis: AllocationBasis,
}

impl Allocator {
    pub(crate) fn open(engine: &mut ReadOnlyEngine) -> Result<Self, TransactionError> {
        Self::open_with_rules(engine, RuleSet::confirmed())
    }

    fn open_with_rules(engine: &mut ReadOnlyEngine, rules: &RuleSet) -> Result<Self, TransactionError> {
        let bucket = engine.read_page(1).map_err(|e| e.to_string()).and_then(BucketTable::parse);
        let mut allocator = Self::open_with_order(engine, rules.index_order().is_some())?;
        match bucket {
            Err(error) => {
                allocator.basis.counter = CounterUse::Unavailable;
                allocator.basis.bucket_error = Some(error);
            }
            Ok(table) => match rules.counter_semantics() {
                Some(CounterSemantics::NextAvailable) => {
                    allocator.basis.counter = CounterUse::NextAvailable;
                    for entry in table.entries {
                        if allocator.floors.insert(entry.extent_word0, entry.next_word1).is_some() {
                            allocator.basis.counter = CounterUse::Unavailable;
                            allocator.basis.bucket_error = Some(format!("duplicate bucket extent {}", entry.extent_word0));
                            allocator.floors.clear();
                            break;
                        }
                    }
                }
                None => allocator.basis.counter = CounterUse::Unconfirmed,
            },
        }
        // Eager fallback rows were seeded before counters were interpreted.
        let eager: Vec<_> = allocator.basis.extents.iter().map(|(&word0, b)| (word0, b.index_max_word1)).collect();
        for (word0, max) in eager { allocator.seed(word0, max)?; }
        Ok(allocator)
    }

    fn open_with_order(engine: &mut ReadOnlyEngine, lazy: bool) -> Result<Self, TransactionError> {
        let mut allocator = Self {
            floors: BTreeMap::new(),
            next: HashMap::new(),
            basis: AllocationBasis {
                index_session: engine.session().sesno,
                index_root: engine.index_root(),
                open_fast_path: lazy,
                counter: CounterUse::IndexOnly,
                bucket_error: None,
                extents: BTreeMap::new(),
            },
        };
        if !lazy {
            let mut max: BTreeMap<u32, u32> = BTreeMap::new();
            // Preserve the eager fallback's record validation as well as numbers.
            for element in engine.scan_elements(ScanTier::Header)? {
                let r = element?.refno;
                max.entry(r.word0).and_modify(|m| *m = (*m).max(r.word1)).or_insert(r.word1);
            }
            for (word0, max) in max { allocator.seed(word0, Some(max))?; }
        }
        Ok(allocator)
    }

    fn seed(&mut self, word0: u32, max: Option<u32>) -> Result<(), TransactionError> {
        let index_first = max.unwrap_or(0).checked_add(1).ok_or(TransactionError::RefnoExhausted { word0 })?;
        let floor = self.floors.get(&word0).copied();
        let first = index_first.max(floor.unwrap_or(0));
        self.next.insert(word0, first);
        self.basis.extents.insert(word0, ExtentAllocationBasis { index_max_word1: max, bucket_floor: floor, first_word1: first });
        Ok(())
    }

    pub(crate) fn allocate(&mut self, engine: &mut ReadOnlyEngine, word0: u32) -> Result<RefNo, TransactionError> {
        if !self.next.contains_key(&word0) {
            let max = if self.basis.open_fast_path {
                let root = self.basis.index_root;
                let mut cursor = engine.index_cursor();
                let hit = match word0.checked_add(1) {
                    Some(next) => cursor.last_before(RefNo::new(next, 0)),
                    None => cursor.last(),
                }.map_err(|error| crate::error::E3dReaderError::IndexError { page_no: root.page, detail: error.to_string() })?;
                hit.filter(|r| r.refno.word0 == word0).map(|r| r.refno.word1)
            } else { None };
            self.seed(word0, max)?;
        }
        let next = self.next.get_mut(&word0).expect("extent seeded");
        if (*next as i32) < 0 { return Err(TransactionError::RefnoExhausted { word0 }); }
        let refno = RefNo::new(word0, *next);
        *next = next.checked_add(1).ok_or(TransactionError::RefnoExhausted { word0 })?;
        Ok(refno)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_rules_choose_only_the_confirmed_sources() {
        use crate::core_rules::Slot;
        let path = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";
        for (counter, order) in [(false, true), (true, false), (false, false)] {
            let mut rules = *RuleSet::confirmed();
            if !counter { rules.counter_semantics = Slot::Pending; }
            if !order { rules.index_order = Slot::Pending; }
            let mut engine = ReadOnlyEngine::open(path).expect("required fixture");
            let mut alloc = Allocator::open_with_rules(&mut engine, &rules).unwrap();
            assert_eq!(alloc.basis.open_fast_path, order);
            assert_eq!(alloc.basis.counter, if counter { CounterUse::NextAvailable } else { CounterUse::Unconfirmed });
            assert_eq!(alloc.basis.extents.is_empty(), order);
            let r = alloc.allocate(&mut engine, 24384).unwrap();
            let basis = &alloc.basis.extents[&24384];
            let want = (basis.index_max_word1.unwrap() + 1).max(basis.bucket_floor.unwrap_or(0));
            assert_eq!(r.word1, want);
            assert_eq!(basis.bucket_floor.is_some(), counter);
        }
    }

    #[test]
    fn overflow_and_signed_limit_never_wrap() {
        let path = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";
        let mut engine = ReadOnlyEngine::open(path).expect("required fixture");
        let mut alloc = Allocator::open_with_order(&mut engine, true).unwrap();
        assert!(matches!(alloc.seed(24384, Some(u32::MAX)), Err(TransactionError::RefnoExhausted { .. })));
        alloc.floors.insert(24384, i32::MAX as u32);
        alloc.seed(24384, None).unwrap();
        assert_eq!(alloc.allocate(&mut engine, 24384).unwrap().word1, i32::MAX as u32);
        assert!(matches!(alloc.allocate(&mut engine, 24384), Err(TransactionError::RefnoExhausted { .. })));
        assert_eq!(alloc.next[&24384], i32::MAX as u32 + 1);
    }

    #[test]
    fn unconfirmed_order_keeps_eager_scan_and_matches_lazy_numbers() {
        let path = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";
        let mut model = ReadOnlyEngine::open(path).expect("required fixture");
        let mut maxima = BTreeMap::new();
        for (r, _, _) in model.indexed_refnos().unwrap() {
            maxima.entry(r.word0).and_modify(|m: &mut u32| *m = (*m).max(r.word1)).or_insert(r.word1);
        }
        let mut eager_engine = ReadOnlyEngine::open(path).unwrap();
        let mut lazy_engine = ReadOnlyEngine::open(path).unwrap();
        let mut eager = Allocator::open_with_order(&mut eager_engine, false).unwrap();
        let mut lazy = Allocator::open_with_order(&mut lazy_engine, true).unwrap();
        assert_eq!(eager.basis.extents.len(), maxima.len(), "fallback scans during open");
        assert!(lazy.basis.extents.is_empty(), "confirmed order defers extent seeks");
        maxima.insert(u32::MAX, 0);
        for (word0, max) in maxima {
            for i in 1..=3 {
                let expected = RefNo::new(word0, max + i);
                assert_eq!(eager.allocate(&mut eager_engine, word0).unwrap(), expected);
                assert_eq!(lazy.allocate(&mut lazy_engine, word0).unwrap(), expected);
            }
        }
    }
}
