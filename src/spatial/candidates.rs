//! Candidate rules for the SPAMAP codec (`.kiro/specs/write-path-phase-3`,
//! task 5.2): assumptions about E3D_Core, each with what it rests on, for
//! Codec_Check to hold against the corpus. They reach a rule set only through
//! `RuleSet::candidates`; nothing that writes a database names this module.
//!
//! What they rest on: the SPAMAP census
//! (`docs/evidence/2026-10-03-spamap-census.md`) and the 28 269 entries of
//! the four corpus databases with a table
//! (`docs/plans/2026-10-03-write-path-phase-3-milestone-b-plan.md` §2).

use std::collections::HashMap;
use std::sync::OnceLock;

use e3d_attlib::db1_hash;

use super::{Axis, EntryBoxes, SpatialState, UndecodableAttr};
use crate::core_rules::{candidate, CandidatePick};
use crate::record::descriptor::DescriptorValue;
use crate::refno::RefNo;

/// One candidate per rule: what Codec_Check runs by default.
pub fn pick() -> CandidatePick {
    CandidatePick {
        rounding: Some(round_to_nearest),
        word_sign: Some(SIGNED_WORDS),
        short_axis: Some(last_shortest_axis),
        zero_extent: Some(zero_extent_by_bucket_rule),
        bucket: Some(bucket_by_extent),
        key0: Some(key0_in_signed_word),
        entries_k: Some(one_entry),
        flags: Some(flags_from_obst),
        coverage: Some(coverage_seen_in_corpus),
    }
}

// --- spamap.rounding ---------------------------------------------------------------

/// Each of min and max to the nearest 0.1 mm, halves away from zero. Nothing
/// rests on it yet: every corpus entry is already in 0.1 mm, and the native
/// BOX `24384/24777` (milestone B plan §3.3) is the first sample that can tell.
pub fn round_to_nearest(min_mm: f64, max_mm: f64) -> (i64, i64) {
    ((min_mm * 10.0).round() as i64, (max_mm * 10.0).round() as i64)
}

/// Min down and max up to the 0.1 mm grid, so the box never shrinks. A hair
/// of tolerance keeps a value already on the grid (8495.000000000001) on it.
pub fn round_outward(min_mm: f64, max_mm: f64) -> (i64, i64) {
    const ON_GRID: f64 = 1e-6;
    ((min_mm * 10.0 + ON_GRID).floor() as i64, (max_mm * 10.0 - ON_GRID).ceil() as i64)
}

// --- spamap.word_sign --------------------------------------------------------------

/// Coordinates are signed 32-bit words: negative mins are stored in two's
/// complement (the native BOX's minx `4294937282` is −30014), and Key0 =
/// N × 10^8 + signed min reads 28 269 of 28 269 entries that way.
pub const SIGNED_WORDS: candidate::WordSign =
    candidate::WordSign { encode: encode_signed, decode: decode_signed, range: (i32::MIN as i64, i32::MAX as i64) };

fn encode_signed(value: i64) -> Option<u32> {
    i32::try_from(value).ok().map(|value| value as u32)
}

fn decode_signed(word: u32) -> i64 {
    i64::from(word as i32)
}

// --- spamap.short_axis -------------------------------------------------------------

/// The axis of least extent in 0.1 mm; among equal ones the last, z before y
/// before x. Key0's axis is a shortest one in 28 269 of 28 269 entries, and
/// the last of the tied ones in all 6 236 ties.
pub fn last_shortest_axis(extents: [i64; 3]) -> Axis {
    let least = extents.iter().copied().min().expect("three extents");
    let last = extents.iter().rposition(|&extent| extent == least).expect("the least is among them");
    Axis::ALL[last]
}

// --- spamap.zero_extent ------------------------------------------------------------

/// Nothing special: an extent of 0 is below the first bucket boundary like
/// any small one. `ams251142_0001`'s 138 flat entries and `ams7999_0001`'s
/// point entries all sit in bucket 1.
pub fn zero_extent_by_bucket_rule(_axis: Axis) -> Option<u8> {
    None
}

// --- spamap.bucket -----------------------------------------------------------------

/// The short-axis extent in 0.1 mm against 600 / 2 000 / 6 000 / 20 000, each
/// boundary in the upper bucket. The first three have corpus samples on the
/// boundary landing above it; the fourth is only known to lie in
/// (19 800, 21 200].
pub fn bucket_by_extent(extent: i64) -> Option<u8> {
    match extent {
        ..0 => None,
        0..600 => Some(1),
        600..2_000 => Some(2),
        2_000..6_000 => Some(3),
        6_000..20_000 => Some(4),
        _ => Some(5),
    }
}

// --- spamap.key0 -------------------------------------------------------------------

/// `N × 10^8 + min` with `N = 6 × axis + bucket`, held in a non-negative
/// signed 32-bit word.
pub fn key0_in_signed_word(axis: Axis, bucket: u8, min: i64) -> Result<u32, (i64, i64)> {
    const STEP: i64 = 100_000_000;
    let base = (6 * axis as i64 + i64::from(bucket)) * STEP;
    let key0 = base + min;
    if (0..=i64::from(i32::MAX)).contains(&key0) {
        Ok(key0 as u32)
    } else {
        Err((-base, i64::from(i32::MAX) - base))
    }
}

// --- spamap.entries_k --------------------------------------------------------------

/// One entry, k = 0, with the element's own box. Wrong for the elements with
/// two entries — 118 in `ams8000_0001`, 2 634 in `ams7999_0001` — whose k = 1
/// entry has a box of its own; Codec_Check counts them.
pub fn one_entry(_refno: RefNo, _state: &dyn SpatialState, aabb: [i64; 6]) -> Result<EntryBoxes, UndecodableAttr> {
    Ok(EntryBoxes::Entries(vec![(0, aabb)]))
}

// --- spamap.flags ------------------------------------------------------------------

/// A BOX's or CYLI's flags are its `OBST`: every BOX and CYLI entry in the
/// corpus has flags 2, and the native BOX's `OBST` is 2. Other nouns are not
/// covered — pipe components have no `OBST` and flags 0, 2, 3, 6.
pub fn flags_from_obst(refno: RefNo, state: &dyn SpatialState, _k: u32) -> Result<Option<u32>, UndecodableAttr> {
    let Some(noun) = state.noun(refno) else { return Ok(None) };
    if noun != db1_hash("BOX") && noun != db1_hash("CYLI") {
        return Ok(None);
    }
    Ok(match state.attr(refno, db1_hash("OBST"))? {
        Some(DescriptorValue::Word { raw, .. }) => Some(raw),
        Some(DescriptorValue::Int(value)) => Some(value as u32),
        _ => None,
    })
}

// --- spamap.coverage ---------------------------------------------------------------

/// Geometric where every corpus element of the noun has an entry, not where
/// none has, unsaid where some have (BRAN, PJOI, SCOJ: something besides the
/// noun decides) or the corpus has no element of the noun.
pub fn coverage_seen_in_corpus(noun: u32) -> Option<bool> {
    static SEEN: OnceLock<HashMap<u32, bool>> = OnceLock::new();
    SEEN.get_or_init(|| {
        let geometric = GEOMETRIC_IN_CORPUS.iter().map(|noun| (db1_hash(noun), true));
        let not = NOT_GEOMETRIC_IN_CORPUS.iter().map(|noun| (db1_hash(noun), false));
        geometric.chain(not).collect()
    })
    .get(&noun)
    .copied()
}

/// Nouns every corpus element of which has an entry (census, 2026-10-03).
const GEOMETRIC_IN_CORPUS: &[&str] = &[
    "ATTA", "BEND", "BOX", "CAP", "CONE", "COUP", "CTOR", "CTRAY", "CWBRAN", "CYLI", "DATU", "DISH", "ELBO", "ELCONN",
    "EXTR", "FBLI", "FITT", "FLAN", "FTUB", "GASK", "HELE", "HROD", "INST", "NOZZ", "OLET", "PANE", "PCLA", "PCOM",
    "PFIT", "PNOD", "PYRA", "REDU", "RNODE", "RTOR", "SCLA", "SCTN", "SLCY", "SNOU", "STRT", "TEE", "VALV", "VTWA",
    "WELD",
];

/// Nouns no corpus element of which has an entry (census, 2026-10-03).
const NOT_GEOMETRIC_IN_CORPUS: &[&str] = &[
    "CATA", "CATE", "CONSEQ", "CTBEND", "CTMTRL", "CTRISE", "CTSTRA", "CTTEE", "CWAY", "DATA", "DBMDAT", "DBMETA",
    "DTSE", "EQUI", "FRMW", "HANG", "HATTA", "HVAC", "LNKI", "LNKS", "LOOP", "MNUM", "NBOX", "NCON", "NCYL", "NPYR",
    "NXTR", "PAVE", "PIPE", "PLOO", "POINTR", "RATTA", "REST", "RPATH", "RTFEAT", "SBFR", "SECT", "SITE", "SNOD",
    "SPEC", "SPWL", "STRU", "SUBE", "SUBJ", "SUBS", "TATTA", "TEXT", "TMPL", "VERT", "WORL", "ZONE",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shortest_axis_takes_the_last_of_a_tie() {
        assert_eq!(last_shortest_axis([5, 3, 9]), Axis::Y);
        assert_eq!(last_shortest_axis([3, 3, 9]), Axis::Y);
        assert_eq!(last_shortest_axis([3, 3, 3]), Axis::Z);
    }

    #[test]
    fn bucket_boundaries_belong_to_the_upper_bucket() {
        let buckets: Vec<Option<u8>> =
            [-1, 0, 599, 600, 1_999, 2_000, 5_999, 6_000, 19_999, 20_000].into_iter().map(bucket_by_extent).collect();
        assert_eq!(buckets, [None, Some(1), Some(1), Some(2), Some(2), Some(3), Some(3), Some(4), Some(4), Some(5)]);
    }

    /// Requirement 4.20's entry (x short, bucket 3, minx 8495), the native
    /// BOX's negative min, the evidence's `24384/2869` (N 16), and
    /// requirement 4.12's limit for N = 17.
    #[test]
    fn key0_matches_the_entries_e3d_wrote() {
        assert_eq!(key0_in_signed_word(Axis::X, 3, 8_495), Ok(300_008_495));
        assert_eq!(key0_in_signed_word(Axis::X, 3, -30_014), Ok(299_969_986));
        assert_eq!(key0_in_signed_word(Axis::Z, 4, 20_129), Ok(1_600_020_129));
        assert_eq!(key0_in_signed_word(Axis::Z, 5, 500_000_000), Err((-1_700_000_000, 447_483_647)));
    }

    #[test]
    fn rounding_keeps_grid_values_and_the_outward_candidate_never_shrinks() {
        assert_eq!(round_to_nearest(849.5, 1150.5), (8_495, 11_505));
        assert_eq!(round_outward(849.5, 1150.5), (8_495, 11_505));
        assert_eq!(round_outward(1800.1, 2200.1), (18_001, 22_001));
        assert_eq!(round_outward(-0.04, 0.04), (-1, 1));
    }

    #[test]
    fn signed_words_round_trip() {
        assert_eq!((SIGNED_WORDS.encode)(-30_014), Some(4_294_937_282));
        assert_eq!((SIGNED_WORDS.decode)(4_294_937_282), -30_014);
        assert_eq!((SIGNED_WORDS.encode)(i64::from(i32::MAX) + 1), None);
    }
}
