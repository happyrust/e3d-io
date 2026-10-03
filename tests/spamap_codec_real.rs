//! Codec_Check on the installed Corpus (Property 5): Key0, entry count and k,
//! and Spatial_Flags counted apart on SPAMAP_Baseline and on every Corpus
//! database's newest session, and a mismatch planted in one item moving that
//! item's count alone. With them the coverage reconciliation (Property 7):
//! every element E3D wrote, entries or none, against `spamap.coverage`.
//!
//! Until the codec's rules are confirmed, what the candidates make of the
//! Corpus is evidence data: printed, not judged. What is judged now is that
//! under the confirmed rules every miss waits on a pending rule (D7). Once
//! the rules are confirmed that same assertion is the whole criterion — the
//! only misses left may be the exclusion list's (requirement 4.17).

mod common;

use std::collections::HashMap;

use common::corpus::{CORPUS, SPAMAP_BASELINE, SPAMAP_BASELINE_SESSION};
use common::spamap_session::Session;
use common::{ATTLIB, DESVIR};
use e3d_attlib::{db1_dehash, db1_hash};
use e3d_io::core_rules::RuleSet;
use e3d_io::index::SpamapEntry;
use e3d_io::session::SessionSelector;
use e3d_io::spatial::check::{Checked, Item, Miss};
use e3d_io::spatial::{candidates, rules};
use e3d_io::RefNo;
use proptest::prelude::*;
use proptest::sample::Index;
use proptest::test_runner::TestRunner;

/// The three codec items; Built_In_AABB is Property 8's.
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

// Feature: write-path-phase-3, Property 7: 覆盖对账
//
// Every element E3D wrote in every Corpus database's newest session has
// entries exactly when `spamap.coverage` says E3D gives it entries; the only
// misses left wait on the elements the rule leaves unsaid (D7). A database
// whose session lists no SPAMAP table has no entries, so every noun it holds
// must be one the rule never records — judged by noun, the catalogue
// template's elements not being decodable with the design one.
#[test]
fn every_element_e3d_wrote_has_entries_exactly_when_coverage_says_so() {
    if common::missing(&[DESVIR, ATTLIB]) || common::missing(CORPUS) {
        return;
    }
    for path in CORPUS {
        let Some(session) = Session::open(path, SessionSelector::Latest) else {
            let mut engine = e3d_io::ReadOnlyEngine::open_at(path, SessionSelector::Latest).expect("open the engine");
            let mut nouns: Vec<u32> = engine
                .scan_elements(e3d_io::engine::ScanTier::Header)
                .expect("walk the refno index")
                .map(|element| element.expect("read an indexed element's header").noun_hash)
                .collect();
            nouns.sort_unstable();
            nouns.dedup();
            let recorded: Vec<String> = nouns
                .iter()
                .filter(|&&noun| matches!(rules::imap_of(noun), 1 | 5 | 7) || rules::HNGT.iter().any(|name| db1_hash(name) == noun))
                .map(|&noun| db1_dehash(noun).unwrap_or_else(|| format!("#{noun}")))
                .collect();
            assert!(recorded.is_empty(), "{path} has no SPAMAP table and holds nouns E3D records: {recorded:?}");
            eprintln!("{path}: no SPAMAP table; none of its {} nouns is recorded", nouns.len());
            continue;
        };
        let coverage = session.coverage(RuleSet::confirmed());
        assert!(session.state.failures().is_empty(), "{path}: {:?}", session.state.failures());
        let decided: Vec<_> = coverage.misses.iter().filter(|miss| !waits(miss)).collect();
        assert_eq!(
            coverage.agree + coverage.waiting,
            coverage.total,
            "{path} session {}: coverage has misses that wait on no rule, as {decided:?}",
            session.sesno
        );
        let waiting: Vec<String> = coverage
            .by_noun
            .iter()
            .filter(|(_, (agree, total))| agree != total)
            .map(|(noun, (agree, total))| format!("{noun} {agree} / {total}"))
            .collect();
        eprintln!(
            "{path} session {}: coverage {} / {} ({} waiting: {})",
            session.sesno,
            coverage.agree,
            coverage.total,
            coverage.waiting,
            waiting.join(", ")
        );
    }
}
