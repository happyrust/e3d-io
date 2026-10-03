//! SPAMAP_Codec (`.kiro/specs/write-path-phase-3/design.md`, SPAMAP_Codec;
//! requirement 4): a World_AABB and the element it belongs to, as the SPAMAP
//! entries E3D_Core would write for them.
//!
//! The codec knows the order of the steps, not how each one goes: every step
//! is a rule of [`crate::core_rules`], taken from the rule set the codec was
//! made with. A step whose rule is pending is noted and the walk goes on, so
//! the answer names every pending rule the element needs — and then no entry
//! comes out at all, never part of one.
//!
//! The steps: round to 0.1 mm (`spamap.rounding`); check each word can hold its
//! value (`spamap.word_sign`); the short axis (`spamap.short_axis`); a short
//! axis of extent 0 (`spamap.zero_extent`, consulted only when the extent is
//! known to be 0); the bucket (`spamap.bucket`); Key0 (`spamap.key0`); the
//! entries and their k (`spamap.entries_k`); Spatial_Flags (`spamap.flags`).
//! Every entry's Key0 comes from its own six coordinates.
//!
//! The codec does not look at Spatial_Policy and does not decide what a
//! pending rule means for a batch; its caller does.

use super::{Axis, EntryBoxes, SpatialState, UndecodableAttr, WorldAabb};
use crate::core_rules::{Implementation, RuleId, RuleSet, Slot};
use crate::index::SpamapEntry;
use crate::refno::RefNo;

/// The codec under one rule set.
#[derive(Debug, Clone, Copy)]
pub struct SpamapCodec<'r> {
    rules: &'r RuleSet,
}

/// What one element encodes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Encoded {
    /// The entries in key order, and the rules a candidate stood in for.
    Entries { entries: Vec<SpamapEntry>, relied_on: Vec<RuleId> },
    /// Every pending rule the element needs, in [`RuleId`] order.
    NeedsUnconfirmed(Vec<RuleId>),
    /// An input the entries need that nothing supplies (requirement 6.15).
    MissingInput(&'static str),
}

/// Why an element cannot be encoded at all, whatever the rules.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CodecError {
    #[error("{refno}: {value_mm} mm on the {axis} axis is outside what SPAMAP holds ({range:?} in 0.1 mm)")]
    OutOfRange { refno: RefNo, axis: Axis, value_mm: f64, range: (i64, i64) },
    #[error(transparent)]
    Undecodable(#[from] UndecodableAttr),
}

/// The rules one encoding consulted: those pending, and those a candidate
/// answered for.
#[derive(Default)]
struct Walk {
    pending: Vec<RuleId>,
    relied_on: Vec<RuleId>,
}

impl Walk {
    fn take<T: Implementation<C>, C: Copy>(&mut self, id: RuleId, slot: &Slot<T, C>) -> Option<C> {
        match slot {
            Slot::Pending => {
                if !self.pending.contains(&id) {
                    self.pending.push(id);
                }
                None
            }
            Slot::Confirmed(confirmed) => Some(confirmed.function()),
            Slot::Candidate(candidate) => {
                if !self.relied_on.contains(&id) {
                    self.relied_on.push(id);
                }
                Some(*candidate)
            }
        }
    }

    fn need(&mut self, id: RuleId) {
        if !self.pending.contains(&id) {
            self.pending.push(id);
        }
    }

    fn sorted(mut ids: Vec<RuleId>) -> Vec<RuleId> {
        ids.sort();
        ids
    }
}

/// Key0 could not be represented: the mins that could be.
struct Key0Overflow {
    axis: Axis,
    min: i64,
    range: (i64, i64),
}

impl<'r> SpamapCodec<'r> {
    pub fn new(rules: &'r RuleSet) -> Self {
        Self { rules }
    }

    /// The entries of `refno`, whose World_AABB is `aabb`, as `state` has the
    /// element.
    pub fn encode(&self, refno: RefNo, state: &dyn SpatialState, aabb: &WorldAabb) -> Result<Encoded, CodecError> {
        let rules = self.rules;
        let mut walk = Walk::default();

        let rounded = walk.take(RuleId::SpamapRounding, &rules.rounding).map(|round| {
            let mut words = [0i64; 6];
            for axis in Axis::ALL {
                let (min, max) = round(aabb.min_mm()[axis as usize], aabb.max_mm()[axis as usize]);
                words[axis as usize] = min;
                words[axis as usize + 3] = max;
            }
            words
        });
        let word_sign = walk.take(RuleId::SpamapWordSign, &rules.word_sign);
        if let (Some(words), Some(sign)) = (rounded, word_sign) {
            check_range(refno, &words, sign)?;
        }

        // The element's own box decides which key rules it needs, even when
        // the entries rule is pending and no entry box is known.
        self.key0_of_box(rounded, &mut walk).map_err(|overflow| out_of_range(refno, overflow))?;

        let entries_rule = walk.take(RuleId::SpamapEntriesK, &rules.entries_k);
        let flags_rule = walk.take(RuleId::SpamapFlags, &rules.flags);
        let boxes = match (entries_rule, rounded) {
            (Some(entries), Some(words)) => Some(entries(refno, state, words)?),
            _ => None,
        };
        let boxes = match boxes {
            Some(EntryBoxes::MissingInput(input)) => {
                if walk.pending.is_empty() {
                    return Ok(Encoded::MissingInput(input));
                }
                None
            }
            Some(EntryBoxes::Entries(boxes)) => Some(boxes),
            None => None,
        };

        let mut entries = Vec::new();
        for (k, words) in boxes.unwrap_or_default() {
            if let Some(sign) = word_sign {
                check_range(refno, &words, sign)?;
            }
            let key0 = self.key0_of_box(Some(words), &mut walk).map_err(|overflow| out_of_range(refno, overflow))?;
            let flags = match flags_rule {
                Some(flags) => match flags(refno, state, k)? {
                    Some(flags) => Some(flags),
                    None => {
                        walk.need(RuleId::SpamapFlags);
                        None
                    }
                },
                None => None,
            };
            if let (Some(key0), Some(sign), Some(flags)) = (key0, word_sign, flags) {
                let mut value = [0u32; 7];
                for (slot, coordinate) in value.iter_mut().zip(words) {
                    *slot = (sign.encode)(coordinate).expect("the range was checked");
                }
                value[6] = flags;
                entries.push(SpamapEntry { key: [key0, refno.word0, refno.word1, k], value });
            }
        }

        if !walk.pending.is_empty() {
            return Ok(Encoded::NeedsUnconfirmed(Walk::sorted(walk.pending)));
        }
        entries.sort();
        Ok(Encoded::Entries { entries, relied_on: Walk::sorted(walk.relied_on) })
    }

    /// Key0 from an entry's own six stored coordinate words alone
    /// (requirement 4.10): `Ok(None)` when the Key0 rule says it cannot be
    /// represented, `Err` with the pending rules it needs.
    pub fn key0_of(&self, words: &[u32; 6]) -> Result<Option<u32>, Vec<RuleId>> {
        let mut walk = Walk::default();
        let decoded = walk.take(RuleId::SpamapWordSign, &self.rules.word_sign).map(|sign| words.map(sign.decode));
        let key0 = self.key0_of_box(decoded, &mut walk);
        if !walk.pending.is_empty() {
            return Err(Walk::sorted(walk.pending));
        }
        Ok(key0.ok().flatten())
    }

    /// Key0 of a box in 0.1 mm, noting the rules on the way: `Ok(None)` when
    /// the box or a rule is missing.
    fn key0_of_box(&self, words: Option<[i64; 6]>, walk: &mut Walk) -> Result<Option<u32>, Key0Overflow> {
        let rules = self.rules;
        let short_axis = walk.take(RuleId::SpamapShortAxis, &rules.short_axis);
        let extents = words.map(|w| [w[3] - w[0], w[4] - w[1], w[5] - w[2]]);
        let axis = match (short_axis, extents) {
            (Some(short_axis), Some(extents)) => Some(short_axis(extents)),
            _ => None,
        };
        let extent = match (axis, extents) {
            (Some(axis), Some(extents)) => Some(extents[axis as usize]),
            _ => None,
        };
        let zero_bucket = match (axis, extent) {
            (Some(axis), Some(0)) => walk.take(RuleId::SpamapZeroExtent, &rules.zero_extent).map(|rule| rule(axis)),
            _ => None,
        };
        let bucket_rule = walk.take(RuleId::SpamapBucket, &rules.bucket);
        let bucket = match (zero_bucket, bucket_rule, extent) {
            (Some(Some(bucket)), _, _) => Some(bucket),
            (_, Some(bucket_rule), Some(extent)) => match bucket_rule(extent) {
                Some(bucket) => Some(bucket),
                None => {
                    walk.need(RuleId::SpamapBucket);
                    None
                }
            },
            _ => None,
        };
        let key0_rule = walk.take(RuleId::SpamapKey0, &rules.key0);
        match (key0_rule, axis, bucket, words) {
            (Some(key0), Some(axis), Some(bucket), Some(words)) => {
                let min = words[axis as usize];
                key0(axis, bucket, min).map(Some).map_err(|range| Key0Overflow { axis, min, range })
            }
            _ => Ok(None),
        }
    }
}

fn check_range(refno: RefNo, words: &[i64; 6], sign: crate::core_rules::candidate::WordSign) -> Result<(), CodecError> {
    for (index, &coordinate) in words.iter().enumerate() {
        if (sign.encode)(coordinate).is_none() {
            return Err(CodecError::OutOfRange {
                refno,
                axis: Axis::ALL[index % 3],
                value_mm: coordinate as f64 / 10.0,
                range: sign.range,
            });
        }
    }
    Ok(())
}

fn out_of_range(refno: RefNo, overflow: Key0Overflow) -> CodecError {
    CodecError::OutOfRange { refno, axis: overflow.axis, value_mm: overflow.min as f64 / 10.0, range: overflow.range }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use e3d_attlib::db1_hash;

    use super::*;
    use crate::core_rules::CandidatePick;
    use crate::record::descriptor::DescriptorValue;
    use crate::spatial::candidates;

    /// One element, in memory.
    struct OneElement {
        refno: RefNo,
        noun: u32,
        attrs: HashMap<u32, DescriptorValue>,
    }

    impl SpatialState for OneElement {
        fn noun(&self, refno: RefNo) -> Option<u32> {
            (refno == self.refno).then_some(self.noun)
        }
        fn owner(&self, _refno: RefNo) -> Option<RefNo> {
            None
        }
        fn attr(&self, refno: RefNo, hash: u32) -> Result<Option<DescriptorValue>, UndecodableAttr> {
            Ok(if refno == self.refno { self.attrs.get(&hash).cloned() } else { None })
        }
    }

    const BOX_26484: RefNo = RefNo { word0: 24_384, word1: 26_484 };

    /// Requirement 4.20's BOX: 301 × 400 × 500 at E 1000 N 2000 U 3000.
    fn box_26484() -> (OneElement, WorldAabb) {
        let state = OneElement {
            refno: BOX_26484,
            noun: db1_hash("BOX"),
            attrs: HashMap::from([(db1_hash("OBST"), DescriptorValue::Word { raw: 2, text: None })]),
        };
        let aabb = WorldAabb::new([849.5, 1800.0, 2750.0], [1150.5, 2200.0, 3250.0]).unwrap();
        (state, aabb)
    }

    #[test]
    fn with_nothing_confirmed_every_rule_on_the_way_is_named_and_no_entry_comes_out() {
        let (state, aabb) = box_26484();
        let encoded = SpamapCodec::new(RuleSet::confirmed()).encode(BOX_26484, &state, &aabb).unwrap();
        assert_eq!(
            encoded,
            Encoded::NeedsUnconfirmed(vec![
                RuleId::SpamapWordSign,
                RuleId::SpamapRounding,
                RuleId::SpamapShortAxis,
                RuleId::SpamapBucket,
                RuleId::SpamapEntriesK,
                RuleId::SpamapFlags,
                RuleId::SpamapKey0,
            ]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect())
        );
    }

    #[test]
    fn the_candidates_encode_requirement_4_20s_entry() {
        let (state, aabb) = box_26484();
        let rules = RuleSet::candidates(&candidates::pick());
        let Encoded::Entries { entries, relied_on } = SpamapCodec::new(&rules).encode(BOX_26484, &state, &aabb).unwrap()
        else {
            panic!("the candidates cover a BOX");
        };
        assert_eq!(
            entries,
            [SpamapEntry {
                key: [300_008_495, 24_384, 26_484, 0],
                value: [8_495, 18_000, 27_500, 11_505, 22_000, 32_500, 2]
            }]
        );
        assert!(!relied_on.contains(&RuleId::SpamapZeroExtent), "the short axis is not flat");
        assert!(relied_on.contains(&RuleId::SpamapFlags));
    }

    #[test]
    fn one_missing_candidate_withholds_every_entry_and_names_it() {
        let (state, aabb) = box_26484();
        let pick = CandidatePick { flags: None, ..candidates::pick() };
        let rules = RuleSet::candidates(&pick);
        assert_eq!(
            SpamapCodec::new(&rules).encode(BOX_26484, &state, &aabb).unwrap(),
            Encoded::NeedsUnconfirmed(vec![RuleId::SpamapFlags])
        );
    }

    #[test]
    fn a_flat_short_axis_consults_the_zero_extent_rule() {
        let (state, _) = box_26484();
        let flat = WorldAabb::new([849.5, 1800.0, 2750.0], [849.5, 2200.0, 3250.0]).unwrap();
        let pick = CandidatePick { zero_extent: None, ..candidates::pick() };
        let rules = RuleSet::candidates(&pick);
        assert_eq!(
            SpamapCodec::new(&rules).encode(BOX_26484, &state, &flat).unwrap(),
            Encoded::NeedsUnconfirmed(vec![RuleId::SpamapZeroExtent])
        );
    }

    #[test]
    fn a_box_beyond_the_words_is_refused_with_its_axis() {
        let (state, _) = box_26484();
        let far = WorldAabb::new([849.5, 1800.0, 2750.0], [1150.5, 2200.0, 300_000_000.0]).unwrap();
        let rules = RuleSet::candidates(&candidates::pick());
        let error = SpamapCodec::new(&rules).encode(BOX_26484, &state, &far).unwrap_err();
        assert!(matches!(error, CodecError::OutOfRange { axis: Axis::Z, .. }), "{error}");
    }

    #[test]
    fn key0_of_stored_words_reads_the_native_box() {
        let rules = RuleSet::candidates(&candidates::pick());
        let words = [4_294_937_282, 76_966, 11_250, 4_294_940_885, 83_113, 16_750];
        assert_eq!(SpamapCodec::new(&rules).key0_of(&words), Ok(Some(299_969_986)));
        assert_eq!(
            SpamapCodec::new(RuleSet::confirmed()).key0_of(&words),
            Err(vec![RuleId::SpamapWordSign, RuleId::SpamapShortAxis, RuleId::SpamapBucket, RuleId::SpamapKey0]
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect())
        );
    }
}
