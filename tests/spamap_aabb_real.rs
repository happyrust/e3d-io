//! Built_In_AABB on the installed Corpus (Property 8): every BOX and CYLI with
//! entries on SPAMAP_Baseline and, written by E3D, on each Corpus database's
//! newest session — its World_AABB computed through `ReadState`, encoded by
//! the codec, held against the stored entries word for word. The native BOX
//! `24384/24777` is among them.
//!
//! The `aabb.*` rules are confirmed
//! (`docs/evidence/2026-10-03-spamap-aabb-members.md`): under them every
//! such element agrees word for word or waits on an input e3d-io cannot have
//! (requirement 6.15) — the whole criterion, with no tolerance for
//! requirement 6.16 to record. Beside it the test prints what each
//! alternative candidate, standing in for its confirmed rule, makes of the
//! Corpus, and every miss with how far each word is off.

mod common;

use std::collections::{BTreeMap, HashMap};

use common::corpus::{CORPUS, SPAMAP_BASELINE, SPAMAP_BASELINE_SESSION};
use common::spamap_session::Session;
use common::{ATTLIB, DESVIR};
use e3d_attlib::{db1_dehash, db1_hash};
use e3d_io::core_rules::{CandidatePick, RuleSet};
use e3d_io::index::SpamapEntry;
use e3d_io::session::SessionSelector;
use e3d_io::spatial::aabb::{AabbOutcome, BuiltInAabb, ChainComposer, BUILT_IN_MEMBER_NOUNS};
use e3d_io::spatial::codec::{Encoded, SpamapCodec};
use e3d_io::spatial::noun_fields::NounFields;
use e3d_io::spatial::SpatialState;
use e3d_io::spatial::{candidates, rules};
use e3d_io::RefNo;

const NATIVE_BOX: RefNo = RefNo { word0: 24_384, word1: 24_777 };

/// A BOX or CYLI E3D wrote that has entries: what is stored, and what the
/// rules make of it.
struct Element {
    refno: RefNo,
    noun: String,
    /// In key order.
    stored: Vec<SpamapEntry>,
    made: Result<Vec<SpamapEntry>, String>,
}

impl Element {
    fn agrees(&self) -> bool {
        self.made.as_ref().is_ok_and(|made| *made == self.stored)
    }
}

/// Every BOX and CYLI E3D wrote that has entries, in table order, under `rules`.
fn built_in_elements(session: &Session, rules: &RuleSet) -> Vec<Element> {
    let composer = ChainComposer::new(rules);
    let built_in = BuiltInAabb::new(rules, &composer);
    let codec = SpamapCodec::new(rules);
    let mut stored: Vec<(RefNo, Vec<SpamapEntry>)> = Vec::new();
    let mut at: HashMap<RefNo, usize> = HashMap::new();
    for entry in &session.table {
        let index = *at.entry(entry.refno()).or_insert_with(|| {
            stored.push((entry.refno(), Vec::new()));
            stored.len() - 1
        });
        stored[index].1.push(*entry);
    }
    let mut elements = Vec::new();
    for (refno, mut stored) in stored {
        if !session.e3d_wrote(refno) {
            continue;
        }
        let made = match built_in.compute(&session.state, refno) {
            Ok(AabbOutcome::NotBuiltIn) => continue,
            Ok(AabbOutcome::Aabb(aabb)) => match codec.encode(refno, &session.state, &aabb) {
                Ok(Encoded::Entries { entries, .. }) => Ok(entries),
                other => Err(format!("{other:?}")),
            },
            other => Err(format!("{other:?}")),
        };
        stored.sort_unstable();
        let noun = session.state.noun(refno).and_then(db1_dehash).unwrap_or_default();
        elements.push(Element { refno, noun, stored, made });
    }
    elements
}

/// How far each stored word is from the one made, as signed 32-bit words:
/// Key0 and the six coordinates in 0.1 mm, then flags.
fn off_by(stored: &SpamapEntry, made: &SpamapEntry) -> [i64; 8] {
    let signed = |word: u32| i64::from(word as i32);
    let mut off = [0; 8];
    off[0] = i64::from(stored.key0()) - i64::from(made.key0());
    for word in 0..7 {
        off[word + 1] = signed(stored.value[word]) - signed(made.value[word]);
    }
    off
}

/// A miss in a line: why nothing was made, the entry counts, or how far
/// each word is off.
fn describe(element: &Element) -> String {
    let head = format!("{} {}", element.refno, element.noun);
    match &element.made {
        Err(why) => format!("{head}: {why}"),
        Ok(made) if made.len() != element.stored.len() => {
            format!("{head}: {} entries stored, {} made", element.stored.len(), made.len())
        }
        Ok(made) => {
            let offs: Vec<String> = element.stored.iter().zip(made).map(|(stored, made)| format!("{:?}", off_by(stored, made))).collect();
            format!("{head}: stored − made [Key0, minx, miny, minz, maxx, maxy, maxz, flags] = {}", offs.join(" "))
        }
    }
}

/// Which kind of miss: nothing made, another entry count, or the largest
/// coordinate word off, in 0.1 mm.
fn kind(element: &Element) -> &'static str {
    let Ok(made) = &element.made else { return "nothing made" };
    if made.len() != element.stored.len() {
        return "entry count";
    }
    let worst = element.stored.iter().zip(made).flat_map(|(stored, made)| off_by(stored, made)[1..7].to_vec()).map(i64::abs).max();
    match worst.unwrap_or(0) {
        0 => "key or flags only",
        1 => "a coordinate off by 1",
        2..=9 => "a coordinate off by 2–9",
        10..=99 => "a coordinate off by 10–99",
        _ => "a coordinate off by 100 or more",
    }
}

/// The confirmed rules as a pick, with `change` made: for
/// `RuleSet::candidates_alone`, one alternative standing in for its rule.
fn alternative(change: impl FnOnce(&mut CandidatePick)) -> CandidatePick {
    let mut pick = candidates::pick();
    pick.rounding = Some(rules::round_half_away_from_zero);
    pick.word_sign = Some(rules::SIGNED_WORDS);
    pick.short_axis = Some(rules::least_axis_last_of_ties);
    pick.zero_extent = Some(rules::leave_zero_extent_to_the_bucket);
    pick.bucket = Some(rules::bucket_by_mapkey);
    pick.key0 = Some(rules::key0_by_mapky2);
    pick.entries_k = Some(rules::entries_by_mpbox);
    pick.flags = Some(rules::flags_by_mpbox);
    pick.coverage = Some(rules::coverage_by_imap);
    change(&mut pick);
    pick
}

/// `aabb.members` takes a member's pose from `GETRAM` only for a noun whose
/// FRMC (`IFCOMP`) is 0, as every member noun the built-in AABB boxes has.
#[test]
fn the_member_nouns_have_a_frmc_of_0() {
    if common::missing(&[ATTLIB]) {
        return;
    }
    let fields = NounFields::parse_file(ATTLIB).expect("attlib.dat");
    for noun in BUILT_IN_MEMBER_NOUNS {
        let frmc = fields.field(db1_hash(noun), db1_hash("FRMC")).map(|value| value.word());
        assert_eq!(frmc, Some(0), "{noun}");
    }
}

// Feature: write-path-phase-3, Property 8: Built_In_AABB 与盘上逐字相同
#[test]
fn every_built_in_element_is_held_against_its_stored_entries() {
    if common::missing(&[DESVIR, ATTLIB]) || common::missing(CORPUS) {
        return;
    }
    let confirmed_rules = RuleSet::confirmed();
    let alternatives = [
        ("members not taken in", alternative(|pick| pick.members = Some(|_, _| false))),
        ("ORI turned y, x, then z", alternative(|pick| pick.ori_form = Some(candidates::ori_degrees_y_then_x_then_z))),
        ("CYLI by its own box", alternative(|pick| pick.cyli_aabb = Some(candidates::cylinder_by_its_own_box))),
        ("sizes as their magnitude", alternative(|pick| pick.degenerate = Some(candidates::size_as_its_magnitude))),
    ];
    let sessions = std::iter::once((SPAMAP_BASELINE, SessionSelector::Exact(SPAMAP_BASELINE_SESSION)))
        .chain(CORPUS.iter().map(|path| (*path, SessionSelector::Latest)));
    let mut native_box = None;
    for (path, at) in sessions {
        let Some(session) = Session::open(path, at) else {
            eprintln!("{path}: no SPAMAP table");
            continue;
        };
        let elements = built_in_elements(&session, confirmed_rules);
        let confirmed = session.check(confirmed_rules, &session.table).built_in_aabb;
        assert_eq!(confirmed.total, elements.len() as u64, "{path}: Codec_Check counts every BOX and CYLI E3D wrote");
        assert_eq!(
            confirmed.agree + confirmed.waiting,
            confirmed.total,
            "{path} session {}: under the confirmed rules a built-in box agrees word for word or waits on an input \
             e3d-io cannot have: {:?}",
            session.sesno,
            confirmed.misses
        );
        let agree = elements.iter().filter(|element| element.agrees()).count();
        assert_eq!(confirmed.agree, agree as u64, "{path}: Codec_Check and this walk");
        if (path, session.sesno) == (SPAMAP_BASELINE, SPAMAP_BASELINE_SESSION) {
            native_box = elements.iter().find(|element| element.refno == NATIVE_BOX).map(|element| element.agrees());
        }

        let mut by_noun: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
        let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
        for element in &elements {
            let tally = by_noun.entry(&element.noun).or_default();
            tally.1 += 1;
            if element.agrees() {
                tally.0 += 1;
            } else {
                *kinds.entry(kind(element)).or_default() += 1;
            }
        }
        eprintln!(
            "{path} session {}: confirmed {agree} / {} ({by_noun:?}); misses {kinds:?}",
            session.sesno,
            elements.len()
        );
        for (name, pick) in &alternatives {
            let agree =
                built_in_elements(&session, &RuleSet::candidates_alone(pick)).iter().filter(|element| element.agrees()).count();
            eprintln!("    with {name}: {agree} / {}", elements.len());
        }
        for element in elements.iter().filter(|element| !element.agrees()) {
            eprintln!("    miss {}", describe(element));
        }
    }
    let native_box = native_box.expect("the native BOX 24384/24777 is one of the baseline's built-in elements");
    assert!(native_box, "the native BOX 24384/24777 under the confirmed rules, word for word");
}
