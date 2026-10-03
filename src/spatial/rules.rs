//! Confirmed rules of the SPAMAP codec (`.kiro/specs/write-path-phase-3`,
//! group 7): each function does what the E3D_Core routine its Evidence_Doc
//! names does, and reaches the codec only as a confirmed slot of
//! `RuleSet::confirmed` in `src/core_rules.rs`.
//!
//! Evidence: `docs/evidence/2026-10-03-spamap-encoding.md` — Core3D.dll's
//! `maplib/MAPINS`, `MAPKEY` and `MAPKY2`, core.dll's table code.

use super::Axis;
use crate::core_rules::candidate;

// --- spamap.rounding ---------------------------------------------------------------

/// Each of min and max to the nearest 0.1 mm, halves away from zero. Core3D
/// `maplib/MAPINS` takes `f_ldnint_val(10 × (coordinate − datum × 10^8))`,
/// Fortran DNINT, and the datum is 0 on every axis while [`SIGNED_WORDS`]
/// holds the coordinates.
pub fn round_half_away_from_zero(min_mm: f64, max_mm: f64) -> (i64, i64) {
    ((min_mm * 10.0).round() as i64, (max_mm * 10.0).round() as i64)
}

// --- spamap.word_sign --------------------------------------------------------------

/// The largest coordinate magnitude, in 0.1 mm, an entry holds with a datum of
/// 0. `MAPINS` gives an axis a datum once its bound of larger magnitude reaches
/// 10^8 mm; this side of 10^9 (0.1 mm) that cannot have happened.
pub const WITHIN_DATUM: i64 = 999_999_999;

/// Signed 32-bit words: `MAPINS` stores INTEGER*4 coordinates, `MAPKY2` keeps
/// Key0 within [5 × 10^7, 1.85 × 10^9], and core.dll compares key words
/// signed. A coordinate past [`WITHIN_DATUM`] would take a datum, which the
/// entry does not hold, so it is out of reach.
pub const SIGNED_WORDS: candidate::WordSign =
    candidate::WordSign { encode: encode_signed, decode: decode_signed, range: (-WITHIN_DATUM, WITHIN_DATUM) };

fn encode_signed(value: i64) -> Option<u32> {
    (-WITHIN_DATUM..=WITHIN_DATUM).contains(&value).then_some(value as i32 as u32)
}

fn decode_signed(word: u32) -> i64 {
    i64::from(word as i32)
}

// --- spamap.short_axis and spamap.bucket ---------------------------------------------

/// The short-axis extent, in 0.1 mm, below which buckets 1 to 5 end.
const BUCKET_CEILINGS: [i64; 5] = [600, 2_000, 6_000, 20_000, 200_000];

/// The least extent, in 0.1 mm, from which `MAPKEY` takes bucket 6 on x.
const BUCKET_6_FLOOR: i64 = 200_000;

/// `MAPKEY` on the rounded box: z when its extent is at or below both others,
/// else y when at or below both others, else x — the last of tied axes; and x
/// whenever that extent is 20 m or more.
pub fn least_axis_last_of_ties(extents: [i64; 3]) -> Axis {
    let [x, y, z] = extents;
    let axis = if z <= x && z <= y {
        Axis::Z
    } else if y <= x && y <= z {
        Axis::Y
    } else {
        Axis::X
    };
    if extents[axis as usize] >= BUCKET_6_FLOOR {
        Axis::X
    } else {
        axis
    }
}

/// An extent of 0 is nothing apart: `MAPKEY` buckets it like any under 600.
pub fn leave_zero_extent_to_the_bucket(_axis: Axis) -> Option<u8> {
    None
}

/// `MAPKEY`: below 600, 2 000, 6 000, 20 000 and 200 000 (0.1 mm) buckets 1 to
/// 5, a boundary in the upper bucket; from 200 000 bucket 6.
pub fn bucket_by_mapkey(extent: i64) -> Option<u8> {
    if extent < 0 {
        return None;
    }
    let below = BUCKET_CEILINGS.iter().position(|&ceiling| extent < ceiling);
    Some(below.map_or(6, |index| index as u8 + 1))
}

// --- spamap.key0 -------------------------------------------------------------------

/// `MAPKEY` / `MAPKY2`: the short axis' min, remainder by 5 × 10^7 (truncated,
/// as Fortran MOD), held within ±5 × 10^7, plus 10^8 × (6 × axis + bucket).
/// Every min is represented; only a bucket no rule gives could overflow.
pub fn key0_by_mapky2(axis: Axis, bucket: u8, min: i64) -> Result<u32, (i64, i64)> {
    const STEP: i64 = 100_000_000;
    const WRAP: i64 = 50_000_000;
    let key0 = (min % WRAP).clamp(-WRAP, WRAP) + STEP * (6 * axis as i64 + i64::from(bucket));
    match i32::try_from(key0) {
        Ok(key0) if key0 >= 0 => Ok(key0 as u32),
        _ => Err((1, 0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ties_go_to_the_last_axis_and_20_m_goes_to_x() {
        assert_eq!(least_axis_last_of_ties([5, 3, 9]), Axis::Y);
        assert_eq!(least_axis_last_of_ties([3, 3, 9]), Axis::Y);
        assert_eq!(least_axis_last_of_ties([3, 9, 3]), Axis::Z);
        assert_eq!(least_axis_last_of_ties([3, 3, 3]), Axis::Z);
        assert_eq!(least_axis_last_of_ties([199_999, 300_000, 400_000]), Axis::X);
        assert_eq!(least_axis_last_of_ties([300_000, 250_000, 200_000]), Axis::X);
    }

    #[test]
    fn buckets_end_where_mapkey_ends_them() {
        let buckets: Vec<Option<u8>> = [-1, 0, 599, 600, 1_999, 2_000, 5_999, 6_000, 19_999, 20_000, 199_999, 200_000]
            .into_iter()
            .map(bucket_by_mapkey)
            .collect();
        assert_eq!(
            buckets,
            [None, Some(1), Some(1), Some(2), Some(2), Some(3), Some(3), Some(4), Some(4), Some(5), Some(5), Some(6)]
        );
    }

    /// Requirement 4.20's entry, the native BOX's negative min, the 2026-09-22
    /// evidence's `24384/2869` (N 16), and a min past 5 km.
    #[test]
    fn key0_is_mapky2s() {
        assert_eq!(key0_by_mapky2(Axis::X, 3, 8_495), Ok(300_008_495));
        assert_eq!(key0_by_mapky2(Axis::X, 3, -30_014), Ok(299_969_986));
        assert_eq!(key0_by_mapky2(Axis::Z, 4, 20_129), Ok(1_600_020_129));
        assert_eq!(key0_by_mapky2(Axis::Z, 5, 50_000_123), Ok(1_700_000_123));
        assert_eq!(key0_by_mapky2(Axis::Z, 5, -50_000_123), Ok(1_699_999_877));
        assert_eq!(key0_by_mapky2(Axis::X, 6, -49_999_999), Ok(550_000_001));
    }

    #[test]
    fn halves_round_away_from_zero() {
        assert_eq!(round_half_away_from_zero(849.5, 1150.5), (8_495, 11_505));
        assert_eq!(round_half_away_from_zero(-0.05, 0.05), (-1, 1));
        assert_eq!(round_half_away_from_zero(-0.04, 0.04), (0, 0));
    }

    #[test]
    fn words_hold_what_a_datum_of_0_holds() {
        assert_eq!((SIGNED_WORDS.encode)(-30_014), Some(4_294_937_282));
        assert_eq!((SIGNED_WORDS.decode)(4_294_937_282), -30_014);
        assert_eq!((SIGNED_WORDS.encode)(WITHIN_DATUM), Some(999_999_999));
        assert_eq!((SIGNED_WORDS.encode)(WITHIN_DATUM + 1), None);
        assert_eq!((SIGNED_WORDS.encode)(-WITHIN_DATUM - 1), None);
    }
}
