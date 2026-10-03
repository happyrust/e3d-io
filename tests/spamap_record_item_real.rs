//! SPAMAP_Record_Item on the installed Corpus (requirements 5.5, 5.11): an
//! element E3D wrote that has entries carries, in its record's type 14
//! `SPAMAP` attribute, the Key0 of each entry at the place its k says, every
//! other place holding a word core writes for an empty slot
//! (`docs/evidence/2026-10-03-spamap-record-item.md`). Printed per noun: the
//! elements with entries, how many carry the item, how many items match.

mod common;

use std::collections::BTreeMap;

use common::corpus::{CORPUS, SPAMAP_BASELINE, SPAMAP_BASELINE_SESSION};
use common::spamap_session::Session;
use common::{ATTLIB, DESVIR};
use e3d_attlib::{db1_dehash, db1_hash};
use e3d_io::index::SpamapEntry;
use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::session::SessionSelector;
use e3d_io::spatial::SpatialState;
use e3d_io::RefNo;

/// What core's type 14 code leaves in a slot no entry holds: 0x80000001 when
/// the item is made, 0x80000000 is the other word it skips as empty.
const EMPTY: [u32; 2] = [0x8000_0001, 0x8000_0000];

fn words(value: DescriptorValue) -> Option<Vec<u32>> {
    match value {
        DescriptorValue::IntArray(values) => Some(values.into_iter().map(|value| value as u32).collect()),
        DescriptorValue::WordArray(values) => Some(values),
        DescriptorValue::Int(value) => Some(vec![value as u32]),
        DescriptorValue::Word { raw, .. } => Some(vec![raw]),
        _ => None,
    }
}

/// Whether the item holds each entry's Key0 at its k and nothing but empty
/// words elsewhere.
fn holds(item: &[u32], entries: &[SpamapEntry]) -> bool {
    entries.iter().all(|entry| item.get(entry.k() as usize) == Some(&entry.key[0]))
        && item.iter().enumerate().all(|(k, word)| entries.iter().any(|entry| entry.k() as usize == k) || EMPTY.contains(word))
}

#[test]
fn every_element_with_entries_carries_their_key0s_by_k() {
    if common::missing(&[DESVIR, ATTLIB, SPAMAP_BASELINE]) || common::missing(CORPUS) {
        return;
    }
    let spamap = db1_hash("SPAMAP");
    let sessions = std::iter::once((SPAMAP_BASELINE, SessionSelector::Exact(SPAMAP_BASELINE_SESSION)))
        .chain(CORPUS.iter().map(|path| (*path, SessionSelector::Latest)));
    for (path, at) in sessions {
        let Some(session) = Session::open(path, at) else { continue };
        let mut of_element: BTreeMap<(u32, u32), Vec<SpamapEntry>> = BTreeMap::new();
        for entry in &session.table {
            let refno = entry.refno();
            of_element.entry((refno.word0, refno.word1)).or_default().push(*entry);
        }
        // noun -> (elements with entries, of them carrying the item, of those holding the entries)
        let mut by_noun: BTreeMap<String, (u64, u64, u64)> = BTreeMap::new();
        let mut wrong = Vec::new();
        for (&(word0, word1), entries) in &of_element {
            let refno = RefNo { word0, word1 };
            if !session.e3d_wrote(refno) {
                continue;
            }
            let noun = session.state.noun(refno).map_or_else(String::new, |noun| db1_dehash(noun).unwrap_or_else(|| format!("#{noun}")));
            let tally = by_noun.entry(noun.clone()).or_default();
            tally.0 += 1;
            let item = session.state.attr(refno, spamap).expect("the SPAMAP attribute decodes").and_then(words);
            let Some(item) = item else {
                wrong.push(format!("{noun} {word0}/{word1}: no item"));
                continue;
            };
            tally.1 += 1;
            if holds(&item, entries) {
                tally.2 += 1;
            } else {
                let by_k: Vec<(u32, u32)> = entries.iter().map(|entry| (entry.k(), entry.key[0])).collect();
                wrong.push(format!("{noun} {word0}/{word1}: item {item:?}, entries (k, Key0) {by_k:?}"));
            }
        }
        let (with, carrying, holding) = by_noun.values().fold((0, 0, 0), |sum, tally| (sum.0 + tally.0, sum.1 + tally.1, sum.2 + tally.2));
        eprintln!("{path} session {}: {carrying} / {with} elements with entries carry the item, {holding} hold their entries", session.sesno);
        for (noun, (with, carrying, holding)) in &by_noun {
            eprintln!("    {noun:<8} {carrying} / {with} carry, {holding} hold");
        }
        wrong.truncate(10);
        assert!(wrong.is_empty(), "{path} session {}: {wrong:#?}", session.sesno);
    }
}
