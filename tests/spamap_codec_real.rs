//! Codec_Check on the installed Corpus (Property 5): Key0, entry count and k,
//! and Spatial_Flags counted apart on SPAMAP_Baseline and on every Corpus
//! database's newest session, and a mismatch planted in one item moving that
//! item's count alone.
//!
//! Until the codec's rules are confirmed, what the candidates make of the
//! Corpus is evidence data: printed, not judged. What is judged now is that
//! under the confirmed rules every miss waits on a pending rule (D7). Once
//! the rules are confirmed that same assertion is the whole criterion — the
//! only misses left may be the exclusion list's (requirement 4.17).

mod common;

use std::collections::HashMap;

use common::corpus::{CORPUS, SPAMAP_BASELINE, SPAMAP_BASELINE_SESSION};
use common::{ATTLIB, DESVIR};
use e3d_attlib::AttlibData;
use e3d_io::core_rules::RuleSet;
use e3d_io::index::{read_spamap, SpamapEntry, SpamapRead};
use e3d_io::record::template::TemplateProvider;
use e3d_io::session::{DbView, SessionSelector};
use e3d_io::spatial::aabb::{BuiltInAabb, ChainComposer};
use e3d_io::spatial::candidates;
use e3d_io::spatial::check::{check, Authors, Checked, Item, Miss};
use e3d_io::spatial::codec::SpamapCodec;
use e3d_io::spatial::state::ReadState;
use e3d_io::RefNo;
use proptest::prelude::*;
use proptest::sample::Index;
use proptest::test_runner::TestRunner;

const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;

/// One session's SPAMAP table, its element state and who wrote each page.
struct Session {
    sesno: u32,
    table: Vec<SpamapEntry>,
    state: ReadState,
    authors: Authors,
}

impl Session {
    /// `None` when the session lists no SPAMAP table.
    fn open(path: &str, at: SessionSelector) -> Option<Session> {
        let mut view = DbView::open_at(path, at).unwrap_or_else(|error| panic!("{path}: open: {error}"));
        let sesno = view.selected_session();
        let authors = Authors::from_sessions(&view.walk_sessions().expect("walk the sessions"));
        let table = match read_spamap(&mut view).unwrap_or_else(|error| panic!("{path}: {error}")) {
            SpamapRead::NoTable { .. } => return None,
            SpamapRead::Table { table, .. } => table.entries().to_vec(),
        };
        let attlib = AttlibData::parse_file(ATTLIB).expect("attlib");
        let templates = TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir");
        let state = ReadState::open(path, sesno, attlib, templates).expect("open the element state");
        Some(Session { sesno, table, state, authors })
    }

    /// The items over `table` under `rules`, with nothing excluded.
    fn check(&self, rules: &RuleSet, table: &[SpamapEntry]) -> Checked {
        let authored = |refno: RefNo| self.authors.e3d_wrote(&self.state, refno);
        let composer = ChainComposer::new(rules);
        let built_in = BuiltInAabb::new(rules, &composer);
        check(table, &self.state, &SpamapCodec::new(rules), &built_in, &authored, &|_| false)
    }
}

fn items(checked: &Checked) -> [(&'static str, &Item); 3] {
    [("Key0", &checked.key0), ("count and k", &checked.entries_k), ("flags", &checked.flags)]
}

/// Whether a listed miss waits on pending rules alone.
fn waits(miss: &Miss) -> bool {
    !miss.reasons.is_empty() && miss.reasons.iter().all(|reason| reason.starts_with("unconfirmed:"))
}

/// An item's agree, total, waiting and `(agree, total)` by noun.
type Counts = (u64, u64, u64, Vec<(String, (u64, u64))>);

/// An item's counts, without the misses it lists.
fn counts(item: &Item) -> Counts {
    (item.agree, item.total, item.waiting, item.by_noun.clone().into_iter().collect())
}

// Feature: write-path-phase-3, Property 5: Codec 三项核对分开计数
#[test]
fn every_corpus_table_is_checked_in_three_items_counted_apart() {
    if common::missing(&[DESVIR, ATTLIB]) || common::missing(CORPUS) {
        return;
    }
    let candidates = RuleSet::candidates(&candidates::pick());
    let sessions = std::iter::once((SPAMAP_BASELINE, SessionSelector::Exact(SPAMAP_BASELINE_SESSION)))
        .chain(CORPUS.iter().map(|path| (*path, SessionSelector::Latest)));
    for (path, at) in sessions {
        let Some(session) = Session::open(path, at) else {
            eprintln!("{path}: no SPAMAP table");
            continue;
        };
        let confirmed = session.check(RuleSet::confirmed(), &session.table);
        let candidate = session.check(&candidates, &session.table);
        assert!(session.state.failures().is_empty(), "{path}: {:?}", session.state.failures());
        assert_eq!(confirmed.key0.total, session.table.len() as u64, "{path}: Key0 counts every entry");
        assert_eq!(
            (candidate.authored_elements, candidate.excluded_elements),
            (confirmed.authored_elements, 0),
            "{path}: the elements counted do not depend on the rules"
        );
        for ((name, under_confirmed), (_, under_candidates)) in items(&confirmed).into_iter().zip(items(&candidate)) {
            assert_eq!(under_confirmed.total, under_candidates.total, "{path}: {name} counts the same cases whatever the rules");
            let decided: Vec<_> = under_confirmed.misses.iter().filter(|miss| !waits(miss)).collect();
            assert_eq!(
                under_confirmed.agree + under_confirmed.waiting,
                under_confirmed.total,
                "{path} session {}: {name} under the confirmed rules has misses that wait on no rule, as {decided:?}",
                session.sesno
            );
        }
        eprintln!(
            "{path} session {}: {} entries, {} elements E3D wrote; candidates: {}",
            session.sesno,
            session.table.len(),
            candidate.authored_elements,
            items(&candidate)
                .map(|(name, item)| format!("{name} {} / {} ({} waiting)", item.agree, item.total, item.waiting))
                .join(", ")
        );
        for (noun, (agree, total)) in &candidate.flags.by_noun {
            if agree > &0 {
                eprintln!("    flags of {noun}: {agree} / {total}");
            }
        }
    }
}

/// Which item a planted mismatch goes into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Planted {
    Key0,
    K,
    Flags,
}

// Feature: write-path-phase-3, Property 5: Codec 三项核对分开计数
#[test]
fn a_mismatch_planted_in_one_item_moves_that_items_count_alone() {
    if common::missing(&[SPAMAP_BASELINE, DESVIR, ATTLIB]) {
        return;
    }
    let session = Session::open(SPAMAP_BASELINE, SessionSelector::Exact(SPAMAP_BASELINE_SESSION))
        .expect("the baseline lists a SPAMAP table");
    let rules = RuleSet::candidates(&candidates::pick());
    let before = session.check(&rules, &session.table);

    // Where each item agrees now, checked one entry or one element at a
    // time, so a mismatch planted there must cost that item exactly one.
    let mut of_element: HashMap<RefNo, Vec<usize>> = HashMap::new();
    for (index, entry) in session.table.iter().enumerate() {
        of_element.entry(entry.refno()).or_default().push(index);
    }
    let alone = |indices: &[usize]| {
        let entries: Vec<SpamapEntry> = indices.iter().map(|&index| session.table[index]).collect();
        session.check(&rules, &entries)
    };
    let key0_agrees: Vec<usize> = (0..session.table.len()).filter(|&index| alone(&[index]).key0.agree == 1).collect();
    let flags_agree: Vec<usize> = (0..session.table.len()).filter(|&index| alone(&[index]).flags.agree == 1).collect();
    let mut k_agrees: Vec<usize> =
        of_element.values().filter(|indices| alone(indices).entries_k.agree == 1).map(|indices| indices[0]).collect();
    k_agrees.sort_unstable();
    assert!(!key0_agrees.is_empty() && !flags_agree.is_empty() && !k_agrees.is_empty(), "each item agrees somewhere");

    let planted = prop_oneof![Just(Planted::Key0), Just(Planted::K), Just(Planted::Flags)];
    let mut runner = TestRunner::default();
    let result = runner.run(&(planted, any::<Index>(), 1u32..1_000), |(planted, at, delta)| {
        let mut table = session.table.clone();
        let target = match planted {
            Planted::Key0 => *at.get(&key0_agrees),
            Planted::K => *at.get(&k_agrees),
            Planted::Flags => *at.get(&flags_agree),
        };
        let entry = &mut table[target];
        match planted {
            Planted::Key0 => entry.key[0] = entry.key[0].wrapping_add(delta),
            Planted::K => entry.key[3] = entry.key[3].wrapping_add(delta),
            Planted::Flags => entry.value[6] ^= delta,
        }
        let after = session.check(&rules, &table);
        prop_assert_eq!((after.authored_elements, after.excluded_elements), (before.authored_elements, before.excluded_elements));
        let moved = match planted {
            Planted::Key0 => 0,
            Planted::K => 1,
            Planted::Flags => 2,
        };
        for (index, ((name, was), (_, is))) in items(&before).into_iter().zip(items(&after)).enumerate() {
            if index != moved {
                prop_assert_eq!(counts(is), counts(was), "{} moved for a mismatch planted in {:?}", name, planted);
                continue;
            }
            prop_assert_eq!(is.agree + 1, was.agree, "{}", name);
            prop_assert_eq!((is.total, is.waiting), (was.total, was.waiting), "{}", name);
            let totals = |item: &Item| item.by_noun.iter().map(|(noun, (_, total))| (noun.clone(), *total)).collect::<Vec<_>>();
            prop_assert_eq!(totals(is), totals(was), "{}", name);
        }
        Ok(())
    });
    if let Err(failure) = result {
        panic!("{failure}");
    }
}
