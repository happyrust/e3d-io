//! Mandatory Corpus allocation checks; the diagnostic uses only the read side.
mod common;

use std::collections::BTreeMap;
use std::process::Command;
use e3d_io::index::IndexCursor;
use e3d_io::meta::bucket::BucketTable;
use e3d_io::record::template::TemplateProvider;
use e3d_io::session::{DbView, SessionSelector};
use e3d_io::spatial::SpatialPolicy;
use e3d_io::write::Transaction;
use proptest::prelude::*;

fn check(path: &str, session: u32) {
    let mut view = DbView::open_at(path, SessionSelector::Exact(session)).expect("required Corpus session");
    let mut maxima = BTreeMap::new();
    for item in IndexCursor::new(&mut view).walk() {
        let r = item.unwrap().refno;
        maxima.entry(r.word0).and_modify(|(_, m): &mut (_, u32)| *m = (*m).max(r.word1)).or_insert((r, r.word1));
    }
    let buckets = BucketTable::parse(view.pages_mut().get_page(1).unwrap()).unwrap();
    let mut templates = TemplateProvider::open(common::DESVIR, Some(0x000B_0692)).unwrap();
    let mut tx = Transaction::open_at(path, session, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
    for (extent, (owner, max)) in &maxima {
        let expected = (max + 1).max(buckets.next_word1(*extent).unwrap_or(0));
        let actual = tx.insert(*owner, e3d_attlib::db1_hash("EQUI"), None, |_, _| Ok(())).unwrap();
        assert_eq!(actual.word1, expected, "{path} session {session} extent {extent}");
    }
    // One bucket extent and one extent absent from page 1 cross-check the CLI;
    // all extents above are compared with the independent full walk.
    let mut extents = vec![maxima.keys().find(|e| buckets.next_word1(**e).is_some()).copied().unwrap_or(0),
        maxima.keys().find(|e| buckets.next_word1(**e).is_none()).copied().unwrap_or(0)];
    extents.sort_unstable(); extents.dedup();
    for extent in extents {
        let output = Command::new(env!("CARGO_BIN_EXE_e3d-descriptor"))
            .args(["next-word1", "--db", path, "--session", &session.to_string(), "--extent", &extent.to_string()]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let expected = (maxima.get(&extent).map_or(0, |(_, m)| *m) + 1).max(buckets.next_word1(extent).unwrap_or(0));
        assert_eq!(json["next_word1"], expected);
        assert_eq!(json["sesno"], session);
    }
}

#[test]
fn corpus_latest_all_extents_match_full_scan_and_read_cli() {
    for path in common::corpus::CORPUS {
        let view = DbView::open(path).expect("required Corpus");
        check(path, view.session().sesno);
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 16, failure_persistence: None, ..ProptestConfig::default() })]
    #[test]
    fn retained_sessions_match_independent_model(db in 0usize..5, fraction in 0usize..100) {
        let path = common::corpus::CORPUS[db];
        let mut view = DbView::open(path).expect("required Corpus");
        let chain = view.walk_sessions().unwrap();
        check(path, chain[fraction * (chain.len()-1) / 99].sesno);
    }
}
