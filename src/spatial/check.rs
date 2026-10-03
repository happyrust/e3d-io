//! Codec_Check (`.kiro/specs/write-path-phase-3`, requirements 4.10, 4.11,
//! 4.15, 4.16, 6.9): the codec held against the entries E3D_Core wrote, in
//! three items counted apart, so a miss on one is no miss on another — and
//! the built-in AABB held against them whole.
//!
//! - Key0: every entry of the table, from its own six coordinate words.
//! - Entry count and k: every element E3D wrote that has entries, given the
//!   box of its first entry.
//! - Spatial_Flags: every entry of an element E3D wrote.
//! - Built_In_AABB: every BOX or CYLI E3D wrote that has entries, encoded
//!   from the box the built-in AABB computes, word for word.
//!
//! Apart from them, [`check_coverage`] holds `spamap.coverage` against every
//! element E3D wrote, entries or none (Property 7).
//!
//! Counting only; `e3d-descriptor spamap-check` and the corpus tests call the
//! same functions.

use std::collections::{BTreeMap, HashMap};

use e3d_attlib::db1_dehash;

use super::aabb::{AabbOutcome, BuiltInAabb};
use super::codec::{Encoded, EntriesOf, SpamapCodec};
use super::state::ReadState;
use super::{SpatialState, Walk, WorldAabb};
use crate::core_rules::{RuleId, RuleSet};
use crate::index::SpamapEntry;
use crate::refno::RefNo;
use crate::session::SessionPage;

/// How many misses an item lists in full.
pub const LISTED: usize = 100;

/// One item's count.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Item {
    pub agree: u64,
    pub total: u64,
    /// Of the misses, those for want of a pending rule or of an input e3d-io
    /// cannot have (requirement 6.15): under the confirmed rules D7's
    /// fallback, not a counterexample. `agree + waiting = total` is the
    /// item's whole criterion once its rules are confirmed.
    pub waiting: u64,
    /// The first [`LISTED`] misses, in key order.
    pub misses: Vec<Miss>,
    /// `(agree, total)` by noun; empty for Key0, which is checked on words.
    pub by_noun: BTreeMap<String, (u64, u64)>,
}

/// One disagreement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Miss {
    /// What the table holds: the entry, or every entry of the element.
    pub stored: Vec<SpamapEntry>,
    /// What the codec made of it: a Key0, the k of each entry, flags, or the
    /// eleven words of each entry the built-in box encodes to; empty when it
    /// made nothing.
    pub codec: Vec<u32>,
    /// Why it made nothing: the pending rules by name, or the input it
    /// lacked. Empty when it made something else.
    pub reasons: Vec<String>,
}

/// The items of one table.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Checked {
    pub key0: Item,
    pub entries_k: Item,
    pub flags: Item,
    pub built_in_aabb: Item,
    /// Elements E3D wrote that have entries, and of them how many the
    /// exclusion list took out of the count and flags items.
    pub authored_elements: u64,
    pub excluded_elements: u64,
}

/// Why the codec made nothing for a case.
enum Unmade {
    /// It waits on these pending rules.
    Pending(Vec<RuleId>),
    /// A confirmed rule needs this input, which e3d-io cannot have.
    Missing(&'static str),
    /// Something no rule would change.
    Other(String),
}

impl Item {
    /// Count one case: whether what the codec made agrees and what it made,
    /// or why it made nothing.
    fn count(&mut self, noun: Option<&str>, made: Result<(bool, Vec<u32>), Unmade>, stored: impl FnOnce() -> Vec<SpamapEntry>) {
        let agreed = matches!(made, Ok((true, _)));
        self.total += 1;
        if agreed {
            self.agree += 1;
        } else {
            if matches!(made, Err(Unmade::Pending(_) | Unmade::Missing(_))) {
                self.waiting += 1;
            }
            if self.misses.len() < LISTED {
                let (codec, reasons) = match made {
                    Ok((_, made)) => (made, Vec::new()),
                    Err(Unmade::Pending(pending)) => (Vec::new(), names(&pending)),
                    Err(Unmade::Missing(input)) => (Vec::new(), vec![format!("missing input: {input}")]),
                    Err(Unmade::Other(why)) => (Vec::new(), vec![why]),
                };
                self.misses.push(Miss { stored: stored(), codec, reasons });
            }
        }
        if let Some(noun) = noun {
            let tally = self.by_noun.entry(noun.to_owned()).or_default();
            tally.1 += 1;
            if agreed {
                tally.0 += 1;
            }
        }
    }
}

/// Who wrote each page: the USER of the session whose region holds it.
#[derive(Debug, Clone)]
pub struct Authors {
    /// `(last page the session wrote, its USER)`, by page.
    ends: Vec<(u32, String)>,
}

impl Authors {
    pub fn from_sessions(sessions: &[SessionPage]) -> Self {
        let mut ends: Vec<(u32, String)> = sessions.iter().map(|session| (session.end.page, session.user.clone())).collect();
        ends.sort();
        Self { ends }
    }

    /// The USER of the first session whose last page is at or past `page`.
    pub fn user_of(&self, page: u32) -> Option<&str> {
        let at = self.ends.partition_point(|(end, _)| *end < page);
        self.ends.get(at).map(|(_, user)| user.as_str())
    }

    /// Whether E3D wrote the element as `state` has it (E3D_Authored_Element):
    /// its record is in a session whose USER is not [`E3DIO_USER`].
    pub fn e3d_wrote(&self, state: &ReadState, refno: RefNo) -> bool {
        state
            .record_page(refno)
            .and_then(|page| self.user_of(page))
            .is_some_and(|user| !user.eq_ignore_ascii_case(E3DIO_USER))
    }
}

/// The USER e3d-io's own sessions carry; anything else counts as E3D's.
pub const E3DIO_USER: &str = "e3dio";

/// Run the items over `table`, the entries of one session in key order.
/// `authored` says whether E3D wrote an element (its latest record is in a
/// session whose USER is not [`E3DIO_USER`]); `excluded` is the exclusion
/// list of requirement 4.17.
pub fn check(
    table: &[SpamapEntry],
    state: &dyn SpatialState,
    codec: &SpamapCodec<'_>,
    built_in: &BuiltInAabb<'_>,
    authored: &dyn Fn(RefNo) -> bool,
    excluded: &dyn Fn(RefNo) -> bool,
) -> Checked {
    let mut checked = Checked::default();
    let noun_of = |refno: RefNo| -> String {
        match state.noun(refno) {
            Some(noun) => db1_dehash(noun).unwrap_or_else(|| format!("#{noun}")),
            None => "(no element)".to_owned(),
        }
    };

    for entry in table {
        let made = match codec.key0_of(&entry.aabb_words()) {
            Ok(Some(key0)) => Ok((key0 == entry.key0(), vec![key0])),
            Ok(None) => Err(Unmade::Other("Key0 cannot be represented".to_owned())),
            Err(pending) => Err(Unmade::Pending(pending)),
        };
        checked.key0.count(None, made, || vec![*entry]);
    }

    let mut elements: Vec<(RefNo, Vec<SpamapEntry>)> = Vec::new();
    let mut at: HashMap<RefNo, usize> = HashMap::new();
    for entry in table {
        let index = *at.entry(entry.refno()).or_insert_with(|| {
            elements.push((entry.refno(), Vec::new()));
            elements.len() - 1
        });
        elements[index].1.push(*entry);
    }
    let mut counted: HashMap<RefNo, String> = HashMap::new();
    for (refno, stored) in &elements {
        if !authored(*refno) {
            continue;
        }
        checked.authored_elements += 1;
        if excluded(*refno) {
            checked.excluded_elements += 1;
            continue;
        }
        let noun = noun_of(*refno);
        let mut stored_k: Vec<u32> = stored.iter().map(SpamapEntry::k).collect();
        stored_k.sort_unstable();
        let made = codec_ks(codec, *refno, state, &stored[0]).map(|ks| (ks == stored_k, ks));
        checked.entries_k.count(Some(&noun), made, || stored.clone());
        if let Some(made) = built_in_entries(codec, built_in, *refno, state) {
            let made = made.map(|entries| {
                let mut stored = stored.clone();
                stored.sort_unstable();
                (entries == stored, entries.iter().flat_map(|entry| entry.key.into_iter().chain(entry.value)).collect())
            });
            checked.built_in_aabb.count(Some(&noun), made, || stored.clone());
        }
        counted.insert(*refno, noun);
    }

    for entry in table {
        let Some(noun) = counted.get(&entry.refno()) else { continue };
        let made = match codec.flags_of(entry.refno(), state, entry.k()) {
            Ok(Ok(flags)) => Ok((flags == entry.flags(), vec![flags])),
            Ok(Err(pending)) => Err(Unmade::Pending(pending)),
            Err(undecodable) => Err(Unmade::Other(undecodable.to_string())),
        };
        checked.flags.count(Some(noun), made, || vec![*entry]);
    }
    checked
}

/// Property 7 (requirement 5.7) over `elements`, the elements of the session
/// `table` belongs to: each one E3D wrote has entries exactly when
/// `spamap.coverage` says E3D gives it entries. The rule's word is `1` or `0`
/// in a miss; a rule pending, or unsaid for the element, is waiting (D7).
pub fn check_coverage(
    elements: &[RefNo],
    table: &[SpamapEntry],
    state: &dyn SpatialState,
    rules: &RuleSet,
    authored: &dyn Fn(RefNo) -> bool,
) -> Item {
    let mut stored: HashMap<RefNo, Vec<SpamapEntry>> = HashMap::new();
    for entry in table {
        stored.entry(entry.refno()).or_default().push(*entry);
    }
    let mut item = Item::default();
    for &refno in elements {
        if !authored(refno) {
            continue;
        }
        let entries = stored.get(&refno);
        let mut walk = Walk::default();
        let made = match walk.take(RuleId::SpamapCoverage, &rules.coverage) {
            None => Err(Unmade::Pending(walk.pending)),
            Some(coverage) => match coverage(refno, state) {
                Ok(Some(gives)) => Ok((gives == entries.is_some(), vec![u32::from(gives)])),
                Ok(None) => Err(Unmade::Pending(vec![RuleId::SpamapCoverage])),
                Err(undecodable) => Err(Unmade::Other(undecodable.to_string())),
            },
        };
        let noun = match state.noun(refno) {
            Some(noun) => db1_dehash(noun).unwrap_or_else(|| format!("#{noun}")),
            None => "(no element)".to_owned(),
        };
        item.count(Some(&noun), made, || entries.cloned().unwrap_or_default());
    }
    item
}

/// The k of each entry the codec gives `refno` for the box of `first`, in
/// order, or why it gives none.
fn codec_ks(codec: &SpamapCodec<'_>, refno: RefNo, state: &dyn SpatialState, first: &SpamapEntry) -> Result<Vec<u32>, Unmade> {
    let words = codec.decode_words(&first.aabb_words()).map_err(Unmade::Pending)?;
    let mm = words.map(|word| word as f64 / 10.0);
    let aabb = WorldAabb::new([mm[0], mm[1], mm[2]], [mm[3], mm[4], mm[5]])
        .map_err(|invalid| Unmade::Other(format!("the stored box is not one: {invalid}")))?;
    match codec.entries_of(refno, state, &aabb) {
        Ok(EntriesOf::Entries(boxes)) => {
            let mut ks: Vec<u32> = boxes.iter().map(|(k, _)| *k).collect();
            ks.sort_unstable();
            Ok(ks)
        }
        Ok(EntriesOf::NeedsUnconfirmed(pending)) => Err(Unmade::Pending(pending)),
        Ok(EntriesOf::MissingInput(input)) => Err(Unmade::Missing(input)),
        Err(error) => Err(Unmade::Other(error.to_string())),
    }
}

/// The entries, in key order, the codec encodes from the box the built-in
/// AABB computes for `refno`, or why there are none; `None` when the noun
/// is not built in.
fn built_in_entries(
    codec: &SpamapCodec<'_>,
    built_in: &BuiltInAabb<'_>,
    refno: RefNo,
    state: &dyn SpatialState,
) -> Option<Result<Vec<SpamapEntry>, Unmade>> {
    let aabb = match built_in.compute(state, refno) {
        Ok(AabbOutcome::Aabb(aabb)) => aabb,
        Ok(AabbOutcome::NotBuiltIn) => return None,
        Ok(AabbOutcome::NeedsUnconfirmed(pending)) => return Some(Err(Unmade::Pending(pending))),
        Ok(AabbOutcome::MissingInput(input)) => return Some(Err(Unmade::Missing(input))),
        Ok(AabbOutcome::NoWorld) => return Some(Err(Unmade::Other("the owner chain does not reach a WORL".to_owned()))),
        Err(undecodable) => return Some(Err(Unmade::Other(undecodable.to_string()))),
    };
    Some(match codec.encode(refno, state, &aabb) {
        Ok(Encoded::Entries { entries, .. }) => Ok(entries),
        Ok(Encoded::NeedsUnconfirmed(pending)) => Err(Unmade::Pending(pending)),
        Ok(Encoded::MissingInput(input)) => Err(Unmade::Missing(input)),
        Err(error) => Err(Unmade::Other(error.to_string())),
    })
}

fn names(ids: &[RuleId]) -> Vec<String> {
    ids.iter().map(|id| format!("unconfirmed:{}", id.name())).collect()
}
