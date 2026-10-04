//! Required Corpus evidence for alloc.index_order, independent of predecessor.
mod common;

use e3d_io::index::IndexCursor;
use e3d_io::session::{DbView, SessionSelector};

#[test]
fn ordinary_index_keys_are_strictly_ordered_on_latest_and_retained_sessions() {
    for path in common::corpus::CORPUS {
        let mut latest = DbView::open(path).expect("required Corpus must be installed");
        let chain = latest.walk_sessions().unwrap();
        let mut sessions = vec![chain[0].sesno, chain[chain.len() / 2].sesno, chain.last().unwrap().sesno];
        sessions.sort_unstable();
        sessions.dedup();
        for sesno in sessions {
            let mut view = DbView::open_at(path, SessionSelector::Exact(sesno)).unwrap();
            let mut previous = None;
            let mut count = 0;
            for r in IndexCursor::new(&mut view).walk() {
                let key = r.expect("complete original index walk").refno;
                assert!(key.word0 <= i32::MAX as u32 && key.word1 <= i32::MAX as u32, "{path} session {sesno}: nonnegative ordinary native words");
                assert!(previous.is_none_or(|p| p < key), "{path} session {sesno}: {previous:?} >= {key}");
                previous = Some(key);
                count += 1;
            }
            println!("ALLOC-ORDER {path} session={sesno} keys={count} violations=0");
        }
    }
}
