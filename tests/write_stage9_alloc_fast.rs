//! Fast allocator against an independent full-index and page-1 model.
mod common;

use std::collections::BTreeMap;
use e3d_io::index::IndexCursor;
use e3d_io::meta::bucket::BucketTable;
use e3d_io::record::template::TemplateProvider;
use e3d_io::session::{DbView, SessionSelector};
use e3d_io::spatial::SpatialPolicy;
use e3d_io::write::{CounterUse, Transaction};
use e3d_io::RefNo;

#[test]
fn lazy_open_and_first_allocations_match_the_index_scan_on_corpus_sessions() {
    for path in common::corpus::CORPUS {
        let mut latest = DbView::open(path).expect("required Corpus");
        let chain = latest.walk_sessions().unwrap();
        let mut sessions = vec![chain[0].sesno, chain[chain.len() / 2].sesno];
        sessions.sort_unstable();
        sessions.dedup();
        for session in sessions {
            let mut view = DbView::open_at(path, SessionSelector::Exact(session)).unwrap();
            let mut model: BTreeMap<u32, (RefNo, u32)> = BTreeMap::new();
            for item in IndexCursor::new(&mut view).walk() {
                let r = item.unwrap().refno;
                model.entry(r.word0).and_modify(|(_, max)| *max = (*max).max(r.word1)).or_insert((r, r.word1));
            }
            let buckets = BucketTable::parse(view.pages_mut().get_page(1).unwrap()).unwrap();
            let mut templates = TemplateProvider::open(common::DESVIR, Some(0x000B_0692)).unwrap();
            let before = e3d_io::read_stats::snapshot();
            let mut tx = Transaction::open_at(path, session, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
            let after = e3d_io::read_stats::snapshot();
            assert_eq!(after.scan_calls, before.scan_calls, "open must not scan elements");
            assert!(tx.allocation_basis().open_fast_path);
            assert_eq!(tx.allocation_basis().index_session, session);
            assert_eq!(tx.allocation_basis().counter, CounterUse::NextAvailable);
            assert!(tx.allocation_basis().extents.is_empty());
            for (word0, (owner, max)) in model {
                let first = (max + 1).max(buckets.next_word1(word0).unwrap_or(0));
                for i in 1..=3 {
                    let actual = tx.insert(owner, e3d_attlib::db1_hash("EQUI"), None, |_, _| Ok(())).unwrap();
                    assert_eq!(actual, RefNo::new(word0, first + i - 1), "{path} session {session}");
                }
                assert_eq!(tx.allocation_basis().extents[&word0].index_max_word1, Some(max));
            }
            drop(tx);
        }
    }
}
