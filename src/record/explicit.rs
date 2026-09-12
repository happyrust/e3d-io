//! Decoder for the member and explicit-attribute blocks following an element's
//! declared implicit region.
//!
//! Works on the bytes [`crate::record::read_record`] gathered, which are one
//! contiguous record: a block reached through a continuation has already been
//! spliced in behind the block that named it. So the walk here is contiguous,
//! and a block that says it continues must be followed by one.

use crate::record::block::{block_at, BlockKind};
use crate::record::text::TextFormat;
use crate::refno::RefNo;

#[derive(Debug, Clone, PartialEq)]
pub enum ExplicitValue {
    Integer(i32),
    Real(f64),
    Bool(bool),
    Text(String),
    RefNo(RefNo),
    IntArray(Vec<i32>),
    RealArray(Vec<f64>),
    RefNoArray(Vec<RefNo>),
    RawWords(Vec<u32>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExplicitAttribute {
    pub hash: u32,
    pub type_code: u16,
    pub data_words: u16,
    pub is_uda: bool,
    pub value: ExplicitValue,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TailData {
    pub members: Vec<RefNo>,
    pub attributes: Vec<ExplicitAttribute>,
    /// End of this element in the buffer. With a buffer from
    /// [`crate::record::read_record`] this is the whole of it.
    pub record_end: usize,
    /// The last block walked stated a continuation and nothing followed it.
    ///
    /// On gathered bytes this cannot happen, so it means the caller assembled
    /// the buffer itself and stopped short — the rest of the member list or
    /// attribute stream is missing, and reporting what was found as complete
    /// is how an element came back with no `NAME`.
    pub continues_past_end: bool,
}

/// [`parse_tail_as`] without the header's `FORMAT` word: text is guessed at
/// ([`TextFormat::Detect`]).
pub fn parse_tail(raw: &[u8], start: usize, refno: RefNo) -> TailData {
    parse_tail_as(raw, start, refno, TextFormat::Detect)
}

/// Walk the member and explicit blocks from `start`, decoding text attributes
/// the way the database's header says they are stored.
pub fn parse_tail_as(raw: &[u8], start: usize, refno: RefNo, text: TextFormat) -> TailData {
    let mut out = TailData {
        record_end: start,
        ..TailData::default()
    };
    let mut cursor = start;

    // One loop over every block the element owns, dispatching on its kind,
    // rather than "one member block, then explicit blocks". Both lists split
    // across blocks: BRAN 24383/85432 in the AMS sample has two member blocks
    // (8 words then 7) before its explicit stream, and taking only the first
    // left the loop staring at a member block where it wanted an explicit one
    // — so it stopped and reported the element as storing no explicit
    // attributes at all, `NAME` and its UDA value included.
    //
    // A member list splits mid-RefNo: on that BRAN the first block ends with
    // the word 24383 and the second begins with 85434, which together are
    // member 24383/85434. So concatenate the payloads first and decode after.
    let mut member_payload = Vec::new();
    let mut explicit_payload = Vec::new();
    let mut more_promised = false;
    while let Some(block) = block_at(raw, cursor, refno) {
        let Some(payload) = raw.get(block.payload(cursor)) else {
            more_promised = true;
            break;
        };
        match block.kind {
            BlockKind::Members => member_payload.extend_from_slice(payload),
            BlockKind::Explicit => explicit_payload.extend_from_slice(payload),
        }
        cursor += block.total_bytes;
        more_promised = block.continues_at.is_some();
    }
    parse_members(&member_payload, &mut out.members);
    parse_explicit_payload(&explicit_payload, &mut out.attributes, text);
    out.record_end = cursor;
    out.continues_past_end = more_promised;
    out
}

/// Decode a member list from the concatenated payloads of one element's member
/// blocks. Headers and their two trailing words are already removed.
fn parse_members(payload: &[u8], out: &mut Vec<RefNo>) {
    let block = payload;
    let mut cursor = 0;
    while let Some(bytes) = block.get(cursor..cursor + 8) {
        let item = RefNo::new(read_u32(bytes, 0).unwrap(), read_u32(bytes, 4).unwrap());
        if item.word0 == 0 && item.word1 == 0 {
            break;
        }
        out.push(item);
        cursor += 8;
    }
}

/// The stream starts where the block's payload starts.
///
/// It used to be tried at both 0 and 8, because the payload was taken from
/// byte 12 of the block and the two words that follow the owner were only
/// stepped over when they read zero. They are the block's continuation address
/// ([`crate::record::block`]), always two words, so the payload always begins
/// at byte 20 and the stream always begins at its start.
fn parse_explicit_payload(payload: &[u8], out: &mut Vec<ExplicitAttribute>, text: TextFormat) {
    let mut parsed = Vec::new();
    if parse_attribute_stream(payload, 0, &mut parsed, text) {
        out.extend(parsed);
    }
}

fn parse_attribute_stream(
    payload: &[u8],
    mut cursor: usize,
    out: &mut Vec<ExplicitAttribute>,
    text: TextFormat,
) -> bool {
    let initial = cursor;
    while cursor + 8 <= payload.len() {
        let hash = read_u32(payload, cursor).unwrap();
        if hash == 0 {
            break;
        }
        let type_code = u16::from_be_bytes(payload[cursor + 4..cursor + 6].try_into().unwrap());
        let data_words = u16::from_be_bytes(payload[cursor + 6..cursor + 8].try_into().unwrap());
        let data_len = data_words as usize * 4;
        let data_start = cursor + 8;
        let Some(data) = payload.get(data_start..data_start + data_len) else {
            return false;
        };
        let Some(value) = decode_value(type_code, data, text) else {
            return false;
        };
        out.push(ExplicitAttribute {
            hash,
            type_code,
            data_words,
            is_uda: hash > 0x171F_AD39,
            value,
        });
        cursor = data_start + data_len;
    }
    cursor > initial
}

fn decode_value(type_code: u16, data: &[u8], text: TextFormat) -> Option<ExplicitValue> {
    Some(match type_code {
        0x0C00 => ExplicitValue::Integer(read_u32(data, 0)? as i32),
        0x1400 => ExplicitValue::Bool(read_u32(data, 0)? != 0),
        0x0800 => {
            if data.len() >= 8 {
                ExplicitValue::Real(read_f64(data, 0)?)
            } else {
                ExplicitValue::Integer(read_u32(data, 0)? as i32)
            }
        }
        0x3C00 | 0x2800 => {
            let len = read_u32(data, 0)? as usize;
            let bytes = data.get(4..4 + len)?;
            ExplicitValue::Text(crate::record::text::decode_as(bytes, text))
        }
        0x4000 | 0x1000 => ExplicitValue::RefNo(RefNo::new(read_u32(data, 0)?, read_u32(data, 4)?)),
        0x1C00 => {
            let count = read_u32(data, 0)? as usize;
            let mut values = Vec::with_capacity(count);
            for index in 0..count {
                values.push(read_u32(data, 4 + index * 4)? as i32);
            }
            ExplicitValue::IntArray(values)
        }
        0x1800 => {
            let count = read_u32(data, 0)? as usize;
            let remaining = data.len().saturating_sub(4);
            let mut values = Vec::with_capacity(count);
            if remaining >= count * 8 {
                for index in 0..count {
                    values.push(read_f64(data, 4 + index * 8)?);
                }
            } else if remaining >= count * 4 {
                for index in 0..count {
                    values.push(f32::from_bits(read_u32(data, 4 + index * 4)?) as f64);
                }
            } else {
                return None;
            }
            ExplicitValue::RealArray(values)
        }
        0x2000 => {
            let count = read_u32(data, 0)? as usize;
            let mut values = Vec::with_capacity(count);
            for index in 0..count {
                values.push(RefNo::new(
                    read_u32(data, 4 + index * 8)?,
                    read_u32(data, 8 + index * 8)?,
                ));
            }
            ExplicitValue::RefNoArray(values)
        }
        _ => ExplicitValue::RawWords(
            data.chunks_exact(4)
                .map(|word| u32::from_be_bytes(word.try_into().unwrap()))
                .collect(),
        ),
    })
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        data.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

/// Read a double whose two halves are stored in Fortran order: the word that
/// big-endian IEEE-754 puts first comes second on disk.
///
/// Read straight through as a big-endian u64, `SCTN`'s `Blength` decodes to a
/// run of 5.3e-315 denormals — the signature of a mantissa sitting where an
/// exponent should be. Swapping the halves yields 600, 620, 640, 660 …, which
/// is what E3D's own `Q Blength` reports for the same element. Six of the
/// eight real arrays in the TTY sample confirm the swap value for value; the
/// remaining two are a mixed-type array and one element whose TTY figure is
/// derived rather than stored.
fn read_f64(data: &[u8], offset: usize) -> Option<f64> {
    let low = read_u32(data, offset)?;
    let high = read_u32(data, offset + 4)?;
    Some(f64::from_bits((u64::from(high) << 32) | u64::from(low)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: RefNo = RefNo {
        word0: 10,
        word1: 20,
    };

    /// `[kind][words][owner][continuation]` and then `payload`.
    fn block(kind: u16, owner: RefNo, continues_at: (u32, u32), payload: &[u8]) -> Vec<u8> {
        assert_eq!(payload.len() % 4, 0);
        let mut out = kind.to_be_bytes().to_vec();
        out.extend_from_slice(
            &(((crate::record::block::HEADER_BYTES + payload.len()) / 4) as u16).to_be_bytes(),
        );
        out.extend_from_slice(&owner.word0.to_be_bytes());
        out.extend_from_slice(&owner.word1.to_be_bytes());
        out.extend_from_slice(&continues_at.0.to_be_bytes());
        out.extend_from_slice(&continues_at.1.to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn parses_scalar_attributes_from_the_block_payload() {
        let mut payload = 0x000A_AFCAu32.to_be_bytes().to_vec();
        payload.extend_from_slice(&0x1400u16.to_be_bytes());
        payload.extend_from_slice(&1u16.to_be_bytes());
        payload.extend_from_slice(&0u32.to_be_bytes());

        let tail = parse_tail(&block(1, OWNER, (0, 0), &payload), 0, OWNER);
        assert_eq!(tail.attributes.len(), 1);
        assert_eq!(tail.attributes[0].value, ExplicitValue::Bool(false));
        assert!(!tail.continues_past_end);
    }

    /// A stream that splits between an attribute header and its value.
    ///
    /// The two blocks arrive back to back because
    /// [`crate::record::read_record`] splices a continuation in behind the
    /// block that named it — there is no filler word to step over here, and a
    /// reader that stepped over one would be guessing.
    #[test]
    fn parses_an_attribute_split_between_two_blocks() {
        let mut head = 0x000A_FA16u32.to_be_bytes().to_vec();
        head.extend_from_slice(&0x2800u16.to_be_bytes());
        head.extend_from_slice(&2u16.to_be_bytes());

        let mut tail_payload = 4u32.to_be_bytes().to_vec();
        tail_payload.extend_from_slice(b"NONE");

        let mut raw = block(1, OWNER, (17, (2 << 12) | 1), &head);
        raw.extend_from_slice(&block(1, OWNER, (0, 0), &tail_payload));

        let tail = parse_tail(&raw, 0, OWNER);
        assert_eq!(tail.attributes.len(), 1);
        assert_eq!(tail.attributes[0].value, ExplicitValue::Text("NONE".into()));
        assert_eq!(tail.record_end, raw.len());
        assert!(!tail.continues_past_end);
    }

    /// **Reverting to "stop when the next block is not there" turns this
    /// red.** A block that states a continuation and has none is a buffer that
    /// was cut short, and the attributes found so far are not all of them.
    #[test]
    fn a_promised_continuation_that_is_absent_is_reported() {
        let mut payload = 0x000A_FA16u32.to_be_bytes().to_vec();
        payload.extend_from_slice(&0x2800u16.to_be_bytes());
        payload.extend_from_slice(&2u16.to_be_bytes());

        let tail = parse_tail(&block(1, OWNER, (17, (2 << 12) | 1), &payload), 0, OWNER);
        assert!(tail.continues_past_end);
    }

    /// The continuation words are not payload. Read as members they become
    /// one: BRAN 24383/85432 reported 18010/8193, which is a page address.
    #[test]
    fn the_continuation_words_are_not_members() {
        let mut payload = 10u32.to_be_bytes().to_vec();
        payload.extend_from_slice(&21u32.to_be_bytes());

        let tail = parse_tail(&block(2, OWNER, (18_010, 0x2001), &payload), 0, OWNER);
        assert_eq!(tail.members, vec![RefNo::new(10, 21)]);
    }
}
