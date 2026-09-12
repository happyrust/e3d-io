//! ATNLOG — element-record byte decoder.
//!
//! Reverse-engineered from IDA `sub_55BC98B` (called via `sub_55BC8DC` at
//! `0x55BC8DC` in `core.dll`; MTR trace name `ATNLOG`). The decode formula
//! recovered in goal `e3d31-attribute-parsing` Slice 7 step 6 is:
//!
//! ```text
//! value_word = page_cache[(page - 1) * 512 + (word_offset - 1) + atgtdf_index]
//! ```
//!
//! Where:
//! * `(page, word_offset)` come from the element's ATGTIX entry, and
//! * `atgtdf_index` is the 1-based position of the attribute hash in the
//!   ATGTDF table.
//!
//! In Rust we already have the element's bytes via
//! [`crate::record::element::ParsedElement`], so the page/offset
//! indirection collapses to a direct read inside `ParsedElement.implicit_data`.
//!
//! See `goals/e3d31-attribute-parsing/SLICE4_IMPLEMENTATION_PLAN.md` for
//! the full design rationale.

use crate::record::attrs;
use crate::record::element::ParsedElement;
use e3d_attlib::{exmap_dispatch, AttrDefault, ExpType};

/// A single attribute slot value as read out of an element record.
#[derive(Debug, Clone, Copy)]
pub struct AtnlogSlot {
    pub raw_word: u32,
}

/// Decoded attribute value, dispatched on the EXMAP `DB_Attribute::type`
/// code (see `docs/ida-3.1-attributes.md` §3 — corrected semantics).
#[derive(Debug, Clone)]
pub enum AttrValue {
    /// `type==3` scalar — encoded as `1`/`0` in the slot.
    Bool(bool),
    /// `type==1` scalar (Integer).
    Int(i32),
    /// `type==2` scalar (Real). Read using the Fortran word-swap
    /// `f64` convention.
    Real(f64),
    /// `type==5` scalar — `RefNo` (2 words).
    DbRef(crate::refno::RefNo),
    /// `type==4`/`6`/`11` scalar (Text-like) — currently surfaced as the
    /// raw word; full string materialisation is a follow-up.
    Text(u32),
    /// `type==7` scalar (Position) — 3 doubles laid out as 6 words.
    Position([f64; 3]),
    /// `type==8` scalar (Direction) — 3 doubles.
    Direction([f64; 3]),
    /// `type==9` scalar (Orientation) — 9 doubles.
    Orientation([f64; 9]),
    /// `type==1` array (1-word i32 elements).
    IntArray(Vec<i32>),
    /// `type==2` array (2-word f64 elements, Fortran word-swap).
    RealArray(Vec<f64>),
    /// `type==3` array (1-word bool elements; per IDA ATNLOG case 1,
    /// stored as `1`=true / `0`=false).
    BoolArray(Vec<bool>),
    /// `type==5` array (2-word `RefNo` elements).
    DbRefArray(Vec<crate::refno::RefNo>),
    /// `type==6`/`11` array of text refs (1-word each).
    TextArray(Vec<u32>),
    /// Raw word, kept for types we don't yet decode.
    Raw(u32),
}

/// SYNO ("synonym") chain attribute hash, used by IDA ATNLOG when a
/// slot reads as `0` (`raw_word == 0`). The reader retries the lookup
/// against the SYNO attribute, which (if present) points to an
/// extension element holding the actual value.
///
/// Resolved from IDA `unk_5DAEB9C` via [`e3d_attlib::db1_dehash`]:
/// `db1_dehash(0x000CC7D2) == "SYNO"`. See progress.jsonl entry
/// `slice_4_step_5_richer_element_decoded`.
///
/// Following the chain requires (a) finding `SYNO`'s ATGTDF position
/// in the noun's attribute list, (b) reading the slot value as a
/// reference, and (c) recursing into the referenced element. Currently
/// only the hash constant is exposed; the walker itself is a follow-up.
pub const SYNO_CHAIN_HASH: u32 = 0x000CC7D2;

/// Returned by [`read_slot`] when the raw word is `-1` (= `0xFFFFFFFF`):
/// the IDA ATNLOG reader interprets this as "use the default value
/// pool" (`dword_6C21390[dword_6C21200[atgtdf_index - 1] - 1]`).
///
/// When the attlib exposes a default value for this hash (via
/// [`e3d_attlib::AttlibData::set_attr_default`]),
/// [`decode_attribute_by_hash`] returns the materialised default;
/// otherwise the generic decoder surfaces the sentinel as an
/// [`AttrValue::Raw`] / `Int(-1)` / equivalent for the attr's `ExpType`.
pub const RAW_USE_DEFAULT: u32 = 0xFFFFFFFF;

/// Translate an [`AttrDefault`] from the attlib's default pool into the
/// corresponding [`AttrValue`] surfaced by the byte decoder. Used by
/// [`decode_attribute_by_hash`] when the storage slot reads
/// [`RAW_USE_DEFAULT`] AND the attlib has a known default for the
/// attribute hash.
pub fn attr_default_to_value(default: AttrDefault) -> AttrValue {
    match default {
        AttrDefault::Int(v) => AttrValue::Int(v),
        AttrDefault::Real(v) => AttrValue::Real(v),
        AttrDefault::Bool(v) => AttrValue::Bool(v),
        AttrDefault::Raw(w) => AttrValue::Raw(w),
    }
}

/// Default header-word count for an element record before the first
/// attribute slot. Header = `impl_len` (1w) + refno (2w) + noun_hash (1w)
/// + owner (2w) = 6 words.
pub const DEFAULT_HEADER_WORDS: usize = 6;

/// Compute the byte offset within `parsed.implicit_data` for the slot at
/// the given 1-based ATGTDF position. Returns `None` if the offset is
/// beyond the record.
pub fn slot_byte_offset(
    parsed: &ParsedElement,
    atgtdf_position_1based: u32,
    header_word_count: usize,
) -> Option<usize> {
    if atgtdf_position_1based == 0 {
        return None;
    }
    let word_idx = header_word_count + (atgtdf_position_1based as usize - 1);
    let byte_off = word_idx * 4;
    if byte_off + 4 > parsed.implicit_data.len() {
        return None;
    }
    Some(byte_off)
}

/// Read the raw slot word for an attribute.
pub fn read_slot(
    parsed: &ParsedElement,
    atgtdf_position_1based: u32,
    header_word_count: usize,
) -> Option<AtnlogSlot> {
    let byte_off = slot_byte_offset(parsed, atgtdf_position_1based, header_word_count)?;
    let bytes = &parsed.implicit_data[byte_off..byte_off + 4];
    let raw_word = u32::from_be_bytes(bytes.try_into().unwrap());
    Some(AtnlogSlot { raw_word })
}

/// Decode an attribute value using the EXMAP-aware dispatch.
///
/// `data_type_code` (1..12) and `size` are from the ATGTDF entry;
/// `ityp` is `DB_Attribute::ityp` (which we don't always have from
/// attlib alone — pass 0 if unknown); `is_uda` is `hash > 0x171FAD39`.
///
/// Routes through [`e3d_attlib::exmap_dispatch`] to obtain the
/// semantic [`ExpType`] and then dispatches each branch to the right
/// byte-reader. Multi-word values consume consecutive slots starting
/// at `atgtdf_position_1based`.
pub fn decode_value_with_meta(
    parsed: &ParsedElement,
    atgtdf_position_1based: u32,
    data_type_code: u32,
    size: u32,
    ityp: u32,
    is_uda: bool,
    header_word_count: usize,
) -> Option<AttrValue> {
    let expt = exmap_dispatch(data_type_code, size.max(1), ityp, is_uda)?;
    decode_value_by_exptype(parsed, atgtdf_position_1based, expt, header_word_count)
}

/// Decode the slot already classified as a specific `ExpType`.
pub fn decode_value_by_exptype(
    parsed: &ParsedElement,
    atgtdf_position_1based: u32,
    expt: ExpType,
    header_word_count: usize,
) -> Option<AttrValue> {
    let word_idx = header_word_count + (atgtdf_position_1based.checked_sub(1)? as usize);
    let word_off_u32 = word_idx as u32;
    match expt {
        ExpType::Boolean => {
            let slot = read_slot(parsed, atgtdf_position_1based, header_word_count)?;
            Some(AttrValue::Bool(slot.raw_word == 1))
        }
        ExpType::Integer => {
            attrs::read_int(&parsed.implicit_data, word_off_u32).map(AttrValue::Int)
        }
        ExpType::Real | ExpType::RealScalar => {
            attrs::read_f64(&parsed.implicit_data, word_off_u32).map(AttrValue::Real)
        }
        ExpType::Text => read_slot(parsed, atgtdf_position_1based, header_word_count)
            .map(|s| AttrValue::Text(s.raw_word)),
        ExpType::DbRef => {
            attrs::read_refno(&parsed.implicit_data, word_off_u32).map(AttrValue::DbRef)
        }
        ExpType::Position => {
            attrs::read_pos_f64(&parsed.implicit_data, word_off_u32).map(AttrValue::Position)
        }
        ExpType::Direction => {
            attrs::read_pos_f64(&parsed.implicit_data, word_off_u32).map(AttrValue::Direction)
        }
        ExpType::Orientation => {
            let mut arr = [0f64; 9];
            for i in 0..9 {
                arr[i] = attrs::read_f64(&parsed.implicit_data, word_off_u32 + i as u32 * 2)?;
            }
            Some(AttrValue::Orientation(arr))
        }
        ExpType::IntArray => decode_int_array(parsed, atgtdf_position_1based, header_word_count),
        ExpType::RealArray => decode_real_array(parsed, atgtdf_position_1based, header_word_count),
        ExpType::BooleanArray => {
            decode_bool_array(parsed, atgtdf_position_1based, header_word_count)
        }
        ExpType::DbRefArray => decode_ref_array(parsed, atgtdf_position_1based, header_word_count),
        ExpType::TextArray => decode_text_array(parsed, atgtdf_position_1based, header_word_count),
        ExpType::Blob | ExpType::Unknown(_) => {
            read_slot(parsed, atgtdf_position_1based, header_word_count)
                .map(|s| AttrValue::Raw(s.raw_word))
        }
    }
}

/// Read the count word at `atgtdf_position` and validate it as a
/// reasonable array length. Returns `(count, first_value_word_index)`.
///
/// Per IDA ATNLOG case 4: the slot at `atgtdf_position_1based` holds
/// the array length; the values follow contiguously starting at the
/// next word. Each value's word width depends on the element type.
fn array_count_and_start(
    parsed: &ParsedElement,
    atgtdf_position_1based: u32,
    header_word_count: usize,
) -> Option<(usize, u32)> {
    let count_slot = read_slot(parsed, atgtdf_position_1based, header_word_count)?;
    let count = count_slot.raw_word;
    // A defaulted / empty / sentinel array. Treat as length-0.
    if count == 0 || count == RAW_USE_DEFAULT {
        let first = header_word_count as u32 + atgtdf_position_1based;
        return Some((0, first));
    }
    // Sanity-cap: a single element record's implicit area is at most a
    // few kilobytes; anything beyond a few hundred entries is almost
    // certainly mis-typed.
    if count > 4096 {
        return None;
    }
    let first = header_word_count as u32 + atgtdf_position_1based;
    Some((count as usize, first))
}

fn decode_int_array(
    parsed: &ParsedElement,
    atgtdf_position_1based: u32,
    header_word_count: usize,
) -> Option<AttrValue> {
    let (count, first_word) =
        array_count_and_start(parsed, atgtdf_position_1based, header_word_count)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let v = attrs::read_int(&parsed.implicit_data, first_word + i as u32)?;
        out.push(v);
    }
    Some(AttrValue::IntArray(out))
}

fn decode_real_array(
    parsed: &ParsedElement,
    atgtdf_position_1based: u32,
    header_word_count: usize,
) -> Option<AttrValue> {
    let (count, first_word) =
        array_count_and_start(parsed, atgtdf_position_1based, header_word_count)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        // Each f64 takes 2 consecutive words.
        let v = attrs::read_f64(&parsed.implicit_data, first_word + (i as u32) * 2)?;
        out.push(v);
    }
    Some(AttrValue::RealArray(out))
}

fn decode_bool_array(
    parsed: &ParsedElement,
    atgtdf_position_1based: u32,
    header_word_count: usize,
) -> Option<AttrValue> {
    let (count, first_word) =
        array_count_and_start(parsed, atgtdf_position_1based, header_word_count)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let word_off = first_word + i as u32;
        let byte_off = word_off as usize * 4;
        if byte_off + 4 > parsed.implicit_data.len() {
            return None;
        }
        let w = u32::from_be_bytes(
            parsed.implicit_data[byte_off..byte_off + 4]
                .try_into()
                .unwrap(),
        );
        out.push(w == 1);
    }
    Some(AttrValue::BoolArray(out))
}

fn decode_ref_array(
    parsed: &ParsedElement,
    atgtdf_position_1based: u32,
    header_word_count: usize,
) -> Option<AttrValue> {
    let (count, first_word) =
        array_count_and_start(parsed, atgtdf_position_1based, header_word_count)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        // Each RefNo takes 2 consecutive words.
        let v = attrs::read_refno(&parsed.implicit_data, first_word + (i as u32) * 2)?;
        out.push(v);
    }
    Some(AttrValue::DbRefArray(out))
}

fn decode_text_array(
    parsed: &ParsedElement,
    atgtdf_position_1based: u32,
    header_word_count: usize,
) -> Option<AttrValue> {
    let (count, first_word) =
        array_count_and_start(parsed, atgtdf_position_1based, header_word_count)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let word_off = first_word + i as u32;
        let byte_off = word_off as usize * 4;
        if byte_off + 4 > parsed.implicit_data.len() {
            return None;
        }
        let w = u32::from_be_bytes(
            parsed.implicit_data[byte_off..byte_off + 4]
                .try_into()
                .unwrap(),
        );
        out.push(w);
    }
    Some(AttrValue::TextArray(out))
}

/// High-level API: look up an attribute by hash, locate it in the
/// element's slot area, dispatch via EXMAP, and decode.
///
/// Returns `Ok(Some(value))` on success, `Ok(None)` when the slot
/// reads as `0` AND the SYNO chain cannot resolve (or is disabled),
/// `Err(LookupError)` when the attribute isn't in the attlib at all.
///
/// When `follow_syno_chain == true` and the direct slot returns `0`,
/// this helper recursively looks up the SYNO ([`SYNO_CHAIN_HASH`])
/// attribute on the same element. If SYNO resolves to a non-zero
/// `RefNo`, that ref is returned as [`AttrValue::DbRef`] — the actual
/// recursion into the referenced element is left to the caller (since
/// it needs the engine to load that element).
pub fn decode_attribute_by_hash(
    parsed: &ParsedElement,
    attlib: &e3d_attlib::AttlibData,
    attr_hash: u32,
    follow_syno_chain: bool,
    header_word_count: usize,
) -> Result<Option<AttrValue>, LookupError> {
    let def = attlib
        .attr_def(attr_hash)
        .ok_or(LookupError::NotInAttlib(attr_hash))?;
    let position = attlib
        .atgtdf_position(attr_hash)
        .ok_or(LookupError::NotInAttlib(attr_hash))?;

    // The position from the helper is 1-based and fits in u32 for our purposes.
    let position_u32 =
        u32::try_from(position).map_err(|_| LookupError::PositionTooLarge(position))?;

    // Peek at the raw slot first: if it's the RAW_USE_DEFAULT sentinel
    // AND the attlib has a known default for this attr, the default
    // pool entry takes precedence over the generic byte decoder.
    if let Some(default) = def.default_value {
        if let Some(slot) = read_slot(parsed, position_u32, header_word_count) {
            if slot.raw_word == RAW_USE_DEFAULT {
                return Ok(Some(attr_default_to_value(default)));
            }
        }
    }

    let is_uda = !e3d_attlib::is_system_hash(attr_hash);
    let value = decode_value_with_meta(
        parsed,
        position_u32,
        def.data_type,
        def.size,
        0,
        is_uda,
        header_word_count,
    );

    // Direct lookup succeeded with non-zero value.
    if let Some(v) = &value {
        if !is_zero_or_default(v) {
            return Ok(value);
        }
    }

    if !follow_syno_chain {
        return Ok(value);
    }

    // Try SYNO fallback. SYNO is a system attribute; if absent from
    // the attlib, just return the original (likely zero) value.
    let syno_pos = match attlib.atgtdf_position(SYNO_CHAIN_HASH) {
        Some(p) => p,
        None => return Ok(value),
    };
    let syno_def = match attlib.attr_def(SYNO_CHAIN_HASH) {
        Some(d) => d,
        None => return Ok(value),
    };
    let syno_pos_u32 =
        u32::try_from(syno_pos).map_err(|_| LookupError::PositionTooLarge(syno_pos))?;
    let syno_value = decode_value_with_meta(
        parsed,
        syno_pos_u32,
        syno_def.data_type,
        syno_def.size,
        0,
        false,
        header_word_count,
    );

    // Surface the SYNO chain pointer to the caller (they can recurse).
    match syno_value {
        Some(AttrValue::DbRef(r)) if r.is_valid() => Ok(Some(AttrValue::DbRef(r))),
        // SYNO present but zero → no chain; return the original zero.
        _ => Ok(value),
    }
}

#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum LookupError {
    #[error("attribute hash 0x{0:08X} not present in attlib ATGTDF table")]
    NotInAttlib(u32),
    #[error("attribute position {0} exceeds u32 range (corrupt attlib?)")]
    PositionTooLarge(usize),
}

/// Returns true when an `AttrValue` is the byte-pattern equivalent of
/// "slot held 0 or 0xFFFFFFFF" — used to gate SYNO chain fallback.
fn is_zero_or_default(v: &AttrValue) -> bool {
    matches!(
        v,
        AttrValue::Bool(false)
            | AttrValue::Int(0)
            | AttrValue::Text(0)
            | AttrValue::Raw(0)
            | AttrValue::Raw(RAW_USE_DEFAULT)
    ) || matches!(v, AttrValue::Real(r) if *r == 0.0)
        || matches!(v, AttrValue::IntArray(a) if a.is_empty())
        || matches!(v, AttrValue::RealArray(a) if a.is_empty())
        || matches!(v, AttrValue::BoolArray(a) if a.is_empty())
        || matches!(v, AttrValue::DbRefArray(a) if a.is_empty())
        || matches!(v, AttrValue::TextArray(a) if a.is_empty())
}

/// Legacy compatibility: decode using only the ATGTDF `data_type` code
/// (no `size`/`ityp`/`is_uda` context). Treats `size=1`, `ityp=0`,
/// `is_uda=false`. Prefer [`decode_value_with_meta`] in new code.
pub fn decode_value(
    parsed: &ParsedElement,
    atgtdf_position_1based: u32,
    data_type_code: u32,
    header_word_count: usize,
) -> Option<AttrValue> {
    decode_value_with_meta(
        parsed,
        atgtdf_position_1based,
        data_type_code,
        1,
        0,
        false,
        header_word_count,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_record(implicit_words: &[u32]) -> ParsedElement {
        let mut bytes = Vec::with_capacity(implicit_words.len() * 4);
        for &w in implicit_words {
            bytes.extend_from_slice(&w.to_be_bytes());
        }
        ParsedElement {
            impl_len_words: implicit_words.len() as i32,
            refno: crate::refno::RefNo::new(1, 1),
            noun_hash: 0,
            owner: crate::refno::RefNo::new(0, 0),
            prefix_offset: 0,
            implicit_data: bytes,
            members: Vec::new(),
            explicit_attributes: Vec::new(),
            record_end: implicit_words.len() * 4,
            total_size: implicit_words.len() * 4,
            text_format: crate::record::text::TextFormat::Detect,
        }
    }

    #[test]
    fn read_slot_at_header_plus_one() {
        // Make an "implicit data" of 10 words. Position-1 ATGTDF index
        // (= word at offset DEFAULT_HEADER_WORDS + 0 = word index 6).
        let words = [
            0xAAAAAAAA, 0xBBBBBBBB, 0xCCCCCCCC, 0xDDDDDDDD, 0xEEEEEEEE, 0xFFFFFFFF, 0x11111111,
            0x22222222, 0x33333333, 0x44444444,
        ];
        let p = fake_record(&words);
        let slot1 = read_slot(&p, 1, DEFAULT_HEADER_WORDS).unwrap();
        assert_eq!(slot1.raw_word, 0x11111111);
        let slot2 = read_slot(&p, 2, DEFAULT_HEADER_WORDS).unwrap();
        assert_eq!(slot2.raw_word, 0x22222222);
        let slot4 = read_slot(&p, 4, DEFAULT_HEADER_WORDS).unwrap();
        assert_eq!(slot4.raw_word, 0x44444444);
    }

    #[test]
    fn read_slot_out_of_range_returns_none() {
        let words = [0u32; 8];
        let p = fake_record(&words);
        assert!(read_slot(&p, 0, DEFAULT_HEADER_WORDS).is_none());
        // Header=6, slot=3 -> word_index 8 (just past end).
        assert!(read_slot(&p, 3, DEFAULT_HEADER_WORDS).is_none());
    }

    #[test]
    fn decode_bool() {
        // slots: [1, 0, 0xFFFFFFFF, 5] from header onwards
        let words = [0u32, 0, 0, 0, 0, 0, 1, 0, 0xFFFFFFFF, 5];
        let p = fake_record(&words);
        // EXMAP: (type=3, size=1) -> Boolean
        // slot 1 (val=1) -> true
        match decode_value(&p, 1, 3, DEFAULT_HEADER_WORDS).unwrap() {
            AttrValue::Bool(b) => assert!(b, "slot1: raw=1 should be Bool(true)"),
            other => panic!("expected Bool(true), got {other:?}"),
        }
        // slot 2 (val=0) -> false
        match decode_value(&p, 2, 3, DEFAULT_HEADER_WORDS).unwrap() {
            AttrValue::Bool(b) => assert!(!b, "slot2: raw=0 should be Bool(false)"),
            other => panic!("expected Bool(false), got {other:?}"),
        }
        // slot 3 (val=0xFFFFFFFF) -> false (the IDA reference returns 0)
        match decode_value(&p, 3, 3, DEFAULT_HEADER_WORDS).unwrap() {
            AttrValue::Bool(b) => assert!(!b, "slot3: raw=0xFFFFFFFF should be Bool(false)"),
            other => panic!("expected Bool(false), got {other:?}"),
        }
        // slot 4 (val=5) -> false (anything other than 1 is false)
        match decode_value(&p, 4, 3, DEFAULT_HEADER_WORDS).unwrap() {
            AttrValue::Bool(b) => assert!(!b, "slot4: raw=5 should be Bool(false)"),
            other => panic!("expected Bool(false), got {other:?}"),
        }
    }

    #[test]
    fn decode_int_array_count_plus_values() {
        // Header occupies words 0..5; slot N (1-based) at word 5+N.
        // Put count=3 at slot 1 (word 6), then 3 i32 values: 10, 20, 30.
        let words = [
            0u32, 0, 0, 0, 0, 0, /* slot1 */ 3, /* values */ 10, 20, 30, 0, 0,
        ];
        let p = fake_record(&words);
        // EXMAP: type=1 size=4 -> IntArray
        match decode_value_with_meta(&p, 1, 1, 4, 0, false, DEFAULT_HEADER_WORDS).unwrap() {
            AttrValue::IntArray(v) => assert_eq!(v, vec![10, 20, 30]),
            other => panic!("expected IntArray, got {other:?}"),
        }
    }

    #[test]
    fn decode_real_array_two_words_per_element() {
        // Build a record with header (6 words) then count=2 at slot 1,
        // then 2 f64s (each 2 words, Fortran word-swap: lo,hi).
        // f64 1.0 = 0x3FF0000000000000 = hi 0x3FF00000, lo 0x00000000
        //   stored as lo-word-first: [0x00000000, 0x3FF00000]
        // f64 2.0 = 0x4000000000000000 = hi 0x40000000, lo 0x00000000
        //   stored as lo-word-first: [0x00000000, 0x40000000]
        let words = [
            0u32, 0, 0, 0, 0, 0, // header padding
            2, // slot1 count
            0x00000000, 0x3FF00000, // f64 = 1.0 (word-swap)
            0x00000000, 0x40000000, // f64 = 2.0
        ];
        let p = fake_record(&words);
        match decode_value_with_meta(&p, 1, 2, 5, 0, false, DEFAULT_HEADER_WORDS).unwrap() {
            AttrValue::RealArray(v) => {
                assert_eq!(v.len(), 2);
                assert!((v[0] - 1.0).abs() < 1e-12);
                assert!((v[1] - 2.0).abs() < 1e-12);
            }
            other => panic!("expected RealArray, got {other:?}"),
        }
    }

    #[test]
    fn decode_bool_array() {
        // count=4, values 1, 0, 1, 0 -> [true, false, true, false]
        let words = [0u32, 0, 0, 0, 0, 0, /* count */ 4, 1, 0, 1, 0];
        let p = fake_record(&words);
        match decode_value_with_meta(&p, 1, 3, 4, 0, false, DEFAULT_HEADER_WORDS).unwrap() {
            AttrValue::BoolArray(v) => assert_eq!(v, vec![true, false, true, false]),
            other => panic!("expected BoolArray, got {other:?}"),
        }
    }

    #[test]
    fn decode_empty_array_returns_zero_length() {
        // count=0 -> empty array, regardless of subsequent bytes.
        let words = [0u32, 0, 0, 0, 0, 0, /* count */ 0, 0xDEADBEEF];
        let p = fake_record(&words);
        match decode_value_with_meta(&p, 1, 1, 4, 0, false, DEFAULT_HEADER_WORDS).unwrap() {
            AttrValue::IntArray(v) => assert!(v.is_empty()),
            other => panic!("expected empty IntArray, got {other:?}"),
        }
    }

    #[test]
    fn decode_default_sentinel_array_returns_empty() {
        // count = 0xFFFFFFFF means "use default pool" — surface as empty
        // (per RAW_USE_DEFAULT contract, we don't fabricate values).
        let words = [0u32, 0, 0, 0, 0, 0, /* count */ 0xFFFFFFFF, 0xDEADBEEF];
        let p = fake_record(&words);
        match decode_value_with_meta(&p, 1, 1, 4, 0, false, DEFAULT_HEADER_WORDS).unwrap() {
            AttrValue::IntArray(v) => assert!(v.is_empty()),
            other => panic!("expected empty IntArray, got {other:?}"),
        }
    }

    /// Build an attlib snapshot with a tiny pretend ATGTDF table for tests.
    fn fake_attlib(defs: &[(u32, u32, u32, u32)]) -> e3d_attlib::AttlibData {
        let mut a = e3d_attlib::AttlibData::default();
        for &(hash, data_type, kind, size) in defs {
            let def = e3d_attlib::AttrDefEntry::new(hash, data_type, kind, size);
            a.attr_defs.push(def);
            a.attr_def_map.insert(hash, def);
        }
        a
    }

    #[test]
    fn decode_attribute_by_hash_returns_value() {
        // attlib: position 1 = INT attr with hash 0x123, position 2 = SYNO.
        let attlib = fake_attlib(&[
            (0x00000123, 1, 1, 1),      // some int attr (system-hash range)
            (SYNO_CHAIN_HASH, 5, 1, 1), // SYNO as DBREF
        ]);
        // implicit: 6 header words + slot1=42 + slot2_3=refno(7,1)
        let words = [0u32, 0, 0, 0, 0, 0, 42, 0, 0];
        let p = fake_record(&words);
        let v =
            decode_attribute_by_hash(&p, &attlib, 0x00000123, false, DEFAULT_HEADER_WORDS).unwrap();
        match v {
            Some(AttrValue::Int(42)) => {}
            other => panic!("expected Int(42), got {other:?}"),
        }
    }

    #[test]
    fn decode_attribute_by_hash_returns_err_when_hash_missing() {
        let attlib = fake_attlib(&[]);
        let p = fake_record(&[0u32; 10]);
        let err = decode_attribute_by_hash(&p, &attlib, 0xDEADBEEF, false, DEFAULT_HEADER_WORDS);
        assert!(matches!(err, Err(LookupError::NotInAttlib(0xDEADBEEF))));
    }

    #[test]
    fn syno_chain_returns_chain_pointer_on_zero() {
        // Direct attr = Int at position 1, slot reads 0.
        // SYNO at position 2 reads a non-zero RefNo (7, 1).
        let attlib = fake_attlib(&[(0x00000123, 1, 1, 1), (SYNO_CHAIN_HASH, 5, 1, 1)]);
        // 6 header words + slot1=0 (direct miss) + slot2/3 = RefNo(7,1)
        let words = [
            0u32, 0, 0, 0, 0, 0, /* slot1 */ 0, /* slot2 hi */ 7, /* slot3 lo */ 1,
        ];
        let p = fake_record(&words);
        let v =
            decode_attribute_by_hash(&p, &attlib, 0x00000123, true, DEFAULT_HEADER_WORDS).unwrap();
        match v {
            Some(AttrValue::DbRef(r)) => {
                assert_eq!(r.word0, 7);
                assert_eq!(r.word1, 1);
            }
            other => panic!("expected DbRef(7,1), got {other:?}"),
        }
    }

    #[test]
    fn syno_chain_returns_original_zero_when_chain_empty() {
        // Direct attr at position 1, SYNO at position 2.
        // Both slots zero -> the helper must return the original Int(0).
        let attlib = fake_attlib(&[(0x00000123, 1, 1, 1), (SYNO_CHAIN_HASH, 5, 1, 1)]);
        let words = [0u32; 10];
        let p = fake_record(&words);
        let v =
            decode_attribute_by_hash(&p, &attlib, 0x00000123, true, DEFAULT_HEADER_WORDS).unwrap();
        match v {
            Some(AttrValue::Int(0)) => {}
            other => panic!("expected Int(0), got {other:?}"),
        }
    }

    #[test]
    fn syno_chain_disabled_returns_zero_immediately() {
        let attlib = fake_attlib(&[(0x00000123, 1, 1, 1), (SYNO_CHAIN_HASH, 5, 1, 1)]);
        // Even though SYNO would resolve, follow_syno_chain=false keeps original.
        let words = [
            0u32, 0, 0, 0, 0, 0, /* slot1 */ 0, /* slot2 */ 7, /* slot3 */ 1,
        ];
        let p = fake_record(&words);
        let v =
            decode_attribute_by_hash(&p, &attlib, 0x00000123, false, DEFAULT_HEADER_WORDS).unwrap();
        match v {
            Some(AttrValue::Int(0)) => {}
            other => panic!("expected Int(0), got {other:?}"),
        }
    }

    #[test]
    fn syno_chain_hash_dehashes_to_syno() {
        // Sanity: the constant must round-trip via db1_dehash to "SYNO".
        assert_eq!(
            e3d_attlib::db1_dehash(SYNO_CHAIN_HASH).as_deref(),
            Some("SYNO")
        );
    }

    #[test]
    fn decode_via_exmap_routes_correctly() {
        // type=3 size=1 -> Boolean (scalar)
        let words = [0u32, 0, 0, 0, 0, 0, 1, 0, 0, 0];
        let p = fake_record(&words);
        match decode_value_with_meta(&p, 1, 3, 1, 0, false, DEFAULT_HEADER_WORDS).unwrap() {
            AttrValue::Bool(b) => assert!(b),
            other => panic!("expected Bool(true), got {other:?}"),
        }
        // type=3 size=2 -> BooleanArray, count=1 at slot1=word6=1,
        // then value at slot2=word7=0 -> [false].
        match decode_value_with_meta(&p, 1, 3, 2, 0, false, DEFAULT_HEADER_WORDS).unwrap() {
            AttrValue::BoolArray(v) => assert_eq!(v, vec![false]),
            other => panic!("expected BoolArray([false]), got {other:?}"),
        }
        // type=12 -> Blob (raw)
        match decode_value_with_meta(&p, 1, 12, 1, 0, false, DEFAULT_HEADER_WORDS).unwrap() {
            AttrValue::Raw(_) => {} // ok
            other => panic!("expected Raw, got {other:?}"),
        }
    }

    #[test]
    fn raw_use_default_with_attr_default_yields_default_value() {
        // Slot 1 reads RAW_USE_DEFAULT (0xFFFFFFFF); attr has a known
        // default of Int(99) injected via set_attr_default. Decoder
        // must return Int(99), not Int(-1) (the generic decode of the
        // sentinel).
        let mut attlib = fake_attlib(&[(0x00000123, 1, 1, 1)]);
        assert!(attlib.set_attr_default(0x00000123, AttrDefault::Int(99)));
        let words = [0u32, 0, 0, 0, 0, 0, RAW_USE_DEFAULT];
        let p = fake_record(&words);
        let v =
            decode_attribute_by_hash(&p, &attlib, 0x00000123, false, DEFAULT_HEADER_WORDS).unwrap();
        match v {
            Some(AttrValue::Int(99)) => {}
            other => panic!("expected Int(99) from default pool, got {other:?}"),
        }
    }

    #[test]
    fn raw_use_default_without_attr_default_falls_through_to_generic() {
        // No default injected: decoder falls through to the generic
        // path. For an Int (type=1 size=1) slot reading 0xFFFFFFFF,
        // attrs::read_int reads as i32 -> -1.
        let attlib = fake_attlib(&[(0x00000123, 1, 1, 1)]);
        let words = [0u32, 0, 0, 0, 0, 0, RAW_USE_DEFAULT];
        let p = fake_record(&words);
        let v =
            decode_attribute_by_hash(&p, &attlib, 0x00000123, false, DEFAULT_HEADER_WORDS).unwrap();
        match v {
            Some(AttrValue::Int(-1)) => {}
            other => panic!("expected Int(-1) sentinel fall-through, got {other:?}"),
        }
    }

    #[test]
    fn attr_default_with_non_sentinel_slot_uses_actual_slot_value() {
        // Default is injected, but the slot has a real value (not the
        // sentinel). Decoder must NOT substitute the default — it must
        // decode the actual slot bytes.
        let mut attlib = fake_attlib(&[(0x00000123, 1, 1, 1)]);
        attlib.set_attr_default(0x00000123, AttrDefault::Int(99));
        let words = [0u32, 0, 0, 0, 0, 0, /* real value */ 7];
        let p = fake_record(&words);
        let v =
            decode_attribute_by_hash(&p, &attlib, 0x00000123, false, DEFAULT_HEADER_WORDS).unwrap();
        match v {
            Some(AttrValue::Int(7)) => {}
            other => panic!("expected Int(7) from actual slot, got {other:?}"),
        }
    }

    #[test]
    fn attr_default_to_value_translates_all_variants() {
        match attr_default_to_value(AttrDefault::Int(5)) {
            AttrValue::Int(5) => {}
            o => panic!("Int variant: got {o:?}"),
        }
        match attr_default_to_value(AttrDefault::Real(2.5)) {
            AttrValue::Real(r) => assert!((r - 2.5).abs() < 1e-12),
            o => panic!("Real variant: got {o:?}"),
        }
        match attr_default_to_value(AttrDefault::Bool(true)) {
            AttrValue::Bool(true) => {}
            o => panic!("Bool variant: got {o:?}"),
        }
        match attr_default_to_value(AttrDefault::Raw(0xAABBCCDD)) {
            AttrValue::Raw(0xAABBCCDD) => {}
            o => panic!("Raw variant: got {o:?}"),
        }
    }
}
