//! SPAMAP_Codec under rule sets drawn at random (Property 6): whichever
//! candidates stand in and whatever the box — on the edge of what a word or
//! Key0 holds or past it, with a tied or a flat short axis — `encode` refuses
//! the element on a coordinate that really is out of reach, or names exactly
//! the pending rules its walk met, or gives every entry; never some of them.
//!
//! The expectation is worked out here, step by step in the design's order
//! (`.kiro/specs/write-path-phase-3/design.md`, SPAMAP_Codec), from the same
//! candidate functions — not from the codec's own bookkeeping.

use std::collections::BTreeSet;

use e3d_attlib::db1_hash;
use e3d_io::core_rules::{candidate, CandidatePick, RuleId, RuleSet};
use e3d_io::index::SpamapEntry;
use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::spatial::candidates;
use e3d_io::spatial::codec::{CodecError, Encoded, SpamapCodec};
use e3d_io::spatial::{Axis, EntryBoxes, SpatialState, UndecodableAttr, WorldAabb};
use e3d_io::RefNo;
use proptest::prelude::*;

/// A candidate, shown in a failing case by its name.
struct Named<F: 'static> {
    name: &'static str,
    function: F,
}

impl<F> std::fmt::Debug for Named<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}

// The candidates of `src/spatial/candidates.rs`, and stand-ins that answer
// otherwise where the codec has something to decide: unsigned words, the
// first of tied axes, a flat axis with a bucket of its own, a bucket rule and
// a flags rule that stop short, a second entry with a box of its own, an
// input nothing has.

static ROUNDING: [Named<candidate::Rounding>; 2] = [
    Named { name: "round_to_nearest", function: candidates::round_to_nearest },
    Named { name: "round_outward", function: candidates::round_outward },
];

static WORD_SIGN: [Named<candidate::WordSign>; 2] = [
    Named { name: "SIGNED_WORDS", function: candidates::SIGNED_WORDS },
    Named {
        name: "unsigned_words",
        function: candidate::WordSign { encode: encode_unsigned, decode: decode_unsigned, range: (0, u32::MAX as i64) },
    },
];

fn encode_unsigned(value: i64) -> Option<u32> {
    u32::try_from(value).ok()
}

fn decode_unsigned(word: u32) -> i64 {
    i64::from(word)
}

static SHORT_AXIS: [Named<candidate::ShortAxis>; 2] = [
    Named { name: "last_shortest_axis", function: candidates::last_shortest_axis },
    Named { name: "first_shortest_axis", function: first_shortest_axis },
];

fn first_shortest_axis(extents: [i64; 3]) -> Axis {
    let least = extents.iter().copied().min().expect("three extents");
    Axis::ALL[extents.iter().position(|&extent| extent == least).expect("the least is among them")]
}

static ZERO_EXTENT: [Named<candidate::ZeroExtent>; 2] = [
    Named { name: "zero_extent_by_bucket_rule", function: candidates::zero_extent_by_bucket_rule },
    Named { name: "flat_in_bucket_1", function: flat_in_bucket_1 },
];

fn flat_in_bucket_1(_axis: Axis) -> Option<u8> {
    Some(1)
}

static BUCKET: [Named<candidate::Bucket>; 2] = [
    Named { name: "bucket_by_extent", function: candidates::bucket_by_extent },
    Named { name: "buckets_below_20_m", function: buckets_below_20_m },
];

/// The corpus' boundaries, and nothing said from 20 m up.
fn buckets_below_20_m(extent: i64) -> Option<u8> {
    if extent < 200_000 {
        candidates::bucket_by_extent(extent)
    } else {
        None
    }
}

static KEY0: [Named<candidate::Key0>; 2] = [
    Named { name: "key0_in_signed_word", function: candidates::key0_in_signed_word },
    Named { name: "key0_in_unsigned_word", function: key0_in_unsigned_word },
];

fn key0_in_unsigned_word(axis: Axis, bucket: u8, min: i64) -> Result<u32, (i64, i64)> {
    let base = (6 * axis as i64 + i64::from(bucket)) * 100_000_000;
    u32::try_from(base + min).map_err(|_| (-base, i64::from(u32::MAX) - base))
}

static ENTRIES_K: [Named<candidate::EntriesK>; 3] = [
    Named { name: "one_entry", function: candidates::one_entry },
    Named { name: "second_entry", function: second_entry },
    Named { name: "needs_more", function: needs_more },
];

/// k = 0 with the element's box and k = 1 with one of its own, by the refno:
/// the same box, the box flat on z, or the box moved past every word on x.
fn second_entry(refno: RefNo, _state: &dyn SpatialState, aabb: [i64; 6]) -> Result<EntryBoxes, UndecodableAttr> {
    let mut second = aabb;
    match refno.word1 % 3 {
        0 => {}
        1 => second[5] = second[2],
        _ => {
            second[0] += 5_000_000_000;
            second[3] += 5_000_000_000;
        }
    }
    Ok(EntryBoxes::Entries(vec![(0, aabb), (1, second)]))
}

fn needs_more(_refno: RefNo, _state: &dyn SpatialState, _aabb: [i64; 6]) -> Result<EntryBoxes, UndecodableAttr> {
    Ok(EntryBoxes::MissingInput("the box of entry 1"))
}

static FLAGS: [Named<candidate::Flags>; 2] = [
    Named { name: "flags_from_obst", function: candidates::flags_from_obst },
    Named { name: "first_entry_flags", function: first_entry_flags },
];

fn first_entry_flags(refno: RefNo, state: &dyn SpatialState, k: u32) -> Result<Option<u32>, UndecodableAttr> {
    if k == 0 {
        candidates::flags_from_obst(refno, state, k)
    } else {
        Ok(None)
    }
}

/// The candidate each rule takes; `None` leaves it pending.
#[derive(Debug, Clone)]
struct Pick {
    rounding: Option<&'static Named<candidate::Rounding>>,
    word_sign: Option<&'static Named<candidate::WordSign>>,
    short_axis: Option<&'static Named<candidate::ShortAxis>>,
    zero_extent: Option<&'static Named<candidate::ZeroExtent>>,
    bucket: Option<&'static Named<candidate::Bucket>>,
    key0: Option<&'static Named<candidate::Key0>>,
    entries_k: Option<&'static Named<candidate::EntriesK>>,
    flags: Option<&'static Named<candidate::Flags>>,
}

impl Pick {
    fn rule_set(&self) -> RuleSet {
        let mut pick = CandidatePick::default();
        pick.rounding = self.rounding.map(|rule| rule.function);
        pick.word_sign = self.word_sign.map(|rule| rule.function);
        pick.short_axis = self.short_axis.map(|rule| rule.function);
        pick.zero_extent = self.zero_extent.map(|rule| rule.function);
        pick.bucket = self.bucket.map(|rule| rule.function);
        pick.key0 = self.key0.map(|rule| rule.function);
        pick.entries_k = self.entries_k.map(|rule| rule.function);
        pick.flags = self.flags.map(|rule| rule.function);
        RuleSet::candidates(&pick)
    }
}

fn any_of<F: 'static>(choices: &'static [Named<F>]) -> impl Strategy<Value = &'static Named<F>> {
    prop::sample::select(choices.iter().collect::<Vec<_>>())
}

/// A candidate, or pending one time in five — never when `whole`, so rule
/// sets that can encode at all are common.
fn choose<F: 'static>(
    candidate: impl Strategy<Value = &'static Named<F>> + 'static,
    whole: bool,
) -> BoxedStrategy<Option<&'static Named<F>>> {
    if whole {
        candidate.prop_map(Some).boxed()
    } else {
        prop::option::weighted(0.8, candidate).boxed()
    }
}

fn picks() -> impl Strategy<Value = Pick> {
    any::<bool>().prop_flat_map(|whole| {
        (
            choose(any_of(&ROUNDING), whole),
            // Unsigned words refuse every box with a coordinate below zero.
            choose(prop_oneof![3 => Just(&WORD_SIGN[0]), 1 => Just(&WORD_SIGN[1])], whole),
            choose(any_of(&SHORT_AXIS), whole),
            choose(any_of(&ZERO_EXTENT), whole),
            choose(any_of(&BUCKET), whole),
            choose(any_of(&KEY0), whole),
            choose(any_of(&ENTRIES_K), whole),
            choose(any_of(&FLAGS), whole),
        )
            .prop_map(|(rounding, word_sign, short_axis, zero_extent, bucket, key0, entries_k, flags)| Pick {
                rounding,
                word_sign,
                short_axis,
                zero_extent,
                bucket,
                key0,
                entries_k,
                flags,
            })
    })
}

/// One element, in memory, with an `OBST` or without.
#[derive(Debug, Clone)]
struct Element {
    refno: RefNo,
    noun: &'static str,
    obst: Option<DescriptorValue>,
}

impl SpatialState for Element {
    fn noun(&self, refno: RefNo) -> Option<u32> {
        (refno == self.refno).then(|| db1_hash(self.noun))
    }
    fn owner(&self, _refno: RefNo) -> Option<RefNo> {
        None
    }
    fn attr(&self, refno: RefNo, hash: u32) -> Result<Option<DescriptorValue>, UndecodableAttr> {
        Ok(if refno == self.refno && hash == db1_hash("OBST") { self.obst.clone() } else { None })
    }
}

fn elements() -> impl Strategy<Value = Element> {
    (
        24_384u32..24_390,
        any::<u32>(),
        prop_oneof![3 => Just("BOX"), 2 => Just("CYLI"), 1 => Just("EQUI")],
        prop_oneof![
            1 => Just(None),
            3 => Just(Some(DescriptorValue::Word { raw: 2, text: None })),
            1 => Just(Some(DescriptorValue::Int(0))),
        ],
    )
        .prop_map(|(word0, word1, noun, obst)| Element { refno: RefNo::new(word0, word1), noun, obst })
}

/// A multiple of 0.05 mm, so roundings meet halves.
fn twentieths(n: i64) -> f64 {
    n as f64 * 0.05
}

/// An extent in mm: flat, small, on a corpus bucket boundary or a hair
/// below one, up to 20 m, or up to 500 m.
fn extent_mm() -> impl Strategy<Value = f64> {
    prop_oneof![
        2 => Just(0.0),
        2 => (0i64..=2_000).prop_map(twentieths),
        1 => prop::sample::select(vec![59.95, 60.0, 199.95, 200.0, 599.95, 600.0, 1_999.95, 2_000.0]),
        3 => (0i64..=400_000).prop_map(twentieths),
        1 => (0i64..=10_000_000).prop_map(twentieths),
    ]
}

/// A min in mm within 2 mm of where Key0 = N × 10^8 + min stops being a
/// non-negative signed word for some N, or of where a signed or an unsigned
/// word stops holding a coordinate; or far past all of them.
fn edge_mm() -> impl Strategy<Value = f64> {
    let key0_edge = (1i64..=17, any::<bool>()).prop_map(|(n, upper)| {
        let min = if upper { i64::from(i32::MAX) - n * 100_000_000 } else { -n * 100_000_000 };
        min as f64 / 10.0
    });
    let word_edge = prop::sample::select(vec![
        i64::from(i32::MIN) as f64 / 10.0,
        0.0,
        i64::from(i32::MAX) as f64 / 10.0,
        i64::from(u32::MAX) as f64 / 10.0,
    ]);
    prop_oneof![
        3 => (key0_edge, -40i64..=40).prop_map(|(edge, off)| edge + twentieths(off)),
        2 => (word_edge, -40i64..=40).prop_map(|(edge, off)| edge + twentieths(off)),
        1 => prop::sample::select(vec![-1e9, 1e9]),
    ]
}

/// Mins within 200 m of the origin, ties between extents common; a third of
/// the boxes have one min moved to an edge, half of those on the short axis,
/// whose min Key0 holds.
fn boxes() -> impl Strategy<Value = WorldAabb> {
    let mins = prop::array::uniform3((-4_000_000i64..=4_000_000).prop_map(twentieths));
    let extents = (prop::array::uniform3(extent_mm()), 0u8..5).prop_map(|(mut extent, tie)| {
        match tie {
            1 => extent[1] = extent[0],
            2 => extent[2] = extent[1],
            3 => extent[2] = extent[0],
            4 => extent = [extent[0]; 3],
            _ => {}
        }
        extent
    });
    let edge = prop::option::weighted(0.33, (prop::option::of(0usize..3), edge_mm()));
    (mins, extents, edge).prop_map(|(mut min, extent, edge)| {
        if let Some((axis, at)) = edge {
            let short = (0..3).rev().min_by(|&a, &b| extent[a].total_cmp(&extent[b])).expect("three axes");
            min[axis.unwrap_or(short)] = at;
        }
        let max = [min[0] + extent[0], min[1] + extent[1], min[2] + extent[2]];
        WorldAabb::new(min, max).expect("finite, and min at or below max")
    })
}

/// A coordinate a rule puts out of reach: its axis, its value in 0.1 mm, and
/// the range the rule allows there.
type OutOfReach = (Axis, i64, (i64, i64));

/// What `encode` must answer.
#[derive(Debug)]
enum Expected {
    /// Every coordinate out of reach for certain; the codec names one.
    OutOfRange(Vec<OutOfReach>),
    NeedsUnconfirmed(Vec<RuleId>),
    MissingInput,
    Entries { entries: Vec<SpamapEntry>, relied_on: Vec<RuleId> },
}

/// The rules the walk consults for every element, whatever its box.
fn consulted_always(pick: &Pick) -> [(RuleId, bool); 7] {
    [
        (RuleId::SpamapRounding, pick.rounding.is_some()),
        (RuleId::SpamapWordSign, pick.word_sign.is_some()),
        (RuleId::SpamapShortAxis, pick.short_axis.is_some()),
        (RuleId::SpamapBucket, pick.bucket.is_some()),
        (RuleId::SpamapKey0, pick.key0.is_some()),
        (RuleId::SpamapEntriesK, pick.entries_k.is_some()),
        (RuleId::SpamapFlags, pick.flags.is_some()),
    ]
}

/// The rules seen on one walk: pending ones, and candidates relied on.
#[derive(Default)]
struct Seen {
    pending: BTreeSet<RuleId>,
    relied_on: BTreeSet<RuleId>,
}

/// Key0 of a box in 0.1 mm, `None` when a rule on the way is pending or does
/// not cover the box. The zero-extent rule is consulted only once the short
/// axis is known to be flat; while it is pending the bucket is unknown.
fn key0_of(pick: &Pick, words: &[i64; 6], seen: &mut Seen) -> Option<Result<u32, OutOfReach>> {
    let extents = [words[3] - words[0], words[4] - words[1], words[5] - words[2]];
    let axis = (pick.short_axis?.function)(extents);
    let extent = extents[axis as usize];
    let mut bucket = None;
    if extent == 0 {
        let Some(rule) = pick.zero_extent else {
            seen.pending.insert(RuleId::SpamapZeroExtent);
            return None;
        };
        seen.relied_on.insert(RuleId::SpamapZeroExtent);
        bucket = (rule.function)(axis);
    }
    let bucket = match bucket {
        Some(bucket) => bucket,
        None => match (pick.bucket?.function)(extent) {
            Some(bucket) => bucket,
            None => {
                seen.pending.insert(RuleId::SpamapBucket);
                return None;
            }
        },
    };
    let min = words[axis as usize];
    Some((pick.key0?.function)(axis, bucket, min).map_err(|range| (axis, min, range)))
}

fn expected(pick: &Pick, element: &Element, aabb: &WorldAabb) -> Expected {
    let mut seen = Seen::default();
    for (id, present) in consulted_always(pick) {
        if present {
            seen.relied_on.insert(id);
        } else {
            seen.pending.insert(id);
        }
    }

    let own = pick.rounding.map(|rule| {
        let mut words = [0i64; 6];
        for axis in 0..3 {
            (words[axis], words[axis + 3]) = (rule.function)(aabb.min_mm()[axis], aabb.max_mm()[axis]);
        }
        words
    });
    let entry_boxes = match (pick.entries_k, own) {
        (Some(rule), Some(words)) => Some((rule.function)(element.refno, element, words).expect("decodable")),
        _ => None,
    };
    let boxes = match &entry_boxes {
        Some(EntryBoxes::Entries(boxes)) => boxes.clone(),
        _ => Vec::new(),
    };

    // The element's own box (requirement 4.12), then each entry's.
    let mut out_of_reach = Vec::new();
    let mut entry_key0s = Vec::new();
    for (index, words) in own.iter().chain(boxes.iter().map(|(_, words)| words)).enumerate() {
        if let Some(rule) = pick.word_sign {
            for (at, &coordinate) in words.iter().enumerate() {
                if (rule.function.encode)(coordinate).is_none() {
                    out_of_reach.push((Axis::ALL[at % 3], coordinate, rule.function.range));
                }
            }
        }
        let key0 = key0_of(pick, words, &mut seen);
        if let Some(Err(beyond)) = key0 {
            out_of_reach.push(beyond);
        }
        if index > 0 {
            entry_key0s.push(key0.and_then(Result::ok));
        }
    }
    let mut flags = Vec::new();
    if let Some(rule) = pick.flags {
        for (k, _) in &boxes {
            let made = (rule.function)(element.refno, element, *k).expect("decodable");
            if made.is_none() {
                seen.pending.insert(RuleId::SpamapFlags);
            }
            flags.push(made);
        }
    }

    if !out_of_reach.is_empty() {
        return Expected::OutOfRange(out_of_reach);
    }
    if !seen.pending.is_empty() {
        return Expected::NeedsUnconfirmed(seen.pending.into_iter().collect());
    }
    if let Some(EntryBoxes::MissingInput(_)) = entry_boxes {
        return Expected::MissingInput;
    }
    let sign = pick.word_sign.expect("nothing is pending").function;
    let mut entries: Vec<SpamapEntry> = boxes
        .iter()
        .zip(entry_key0s)
        .zip(flags)
        .map(|(((k, words), key0), flags)| {
            let mut value = [0u32; 7];
            for (slot, &coordinate) in value.iter_mut().zip(words) {
                *slot = (sign.encode)(coordinate).expect("in reach");
            }
            value[6] = flags.expect("covered");
            SpamapEntry { key: [key0.expect("known"), element.refno.word0, element.refno.word1, *k], value }
        })
        .collect();
    entries.sort();
    Expected::Entries { entries, relied_on: seen.relied_on.into_iter().collect() }
}

proptest! {
    // Feature: write-path-phase-3, Property 6: Codec 结果三分
    #[test]
    fn encode_refuses_names_every_pending_rule_or_gives_every_entry(
        pick in picks(),
        element in elements(),
        aabb in boxes(),
    ) {
        let rules = pick.rule_set();
        let encoded = SpamapCodec::new(&rules).encode(element.refno, &element, &aabb);
        match (expected(&pick, &element, &aabb), encoded) {
            (Expected::OutOfRange(reach), Err(CodecError::OutOfRange { refno, axis, value_mm, range })) => {
                prop_assert_eq!(refno, element.refno);
                let value = (value_mm * 10.0).round() as i64;
                prop_assert!(!(range.0..=range.1).contains(&value), "{value} is within {range:?}");
                prop_assert!(
                    reach.contains(&(axis, value, range)),
                    "named {axis} {value} {range:?}, out of reach: {reach:?}"
                );
            }
            (Expected::NeedsUnconfirmed(pending), Ok(Encoded::NeedsUnconfirmed(named))) => {
                prop_assert_eq!(named, pending);
            }
            (Expected::MissingInput, Ok(Encoded::MissingInput(_))) => {}
            (Expected::Entries { entries, relied_on }, Ok(Encoded::Entries { entries: made, relied_on: named })) => {
                prop_assert_eq!(made, entries);
                prop_assert_eq!(named, relied_on);
            }
            (expected, encoded) => prop_assert!(false, "expected {expected:?}, got {encoded:?}"),
        }
    }
}
