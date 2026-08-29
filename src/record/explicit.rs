//! Decoder for the member and explicit-attribute blocks following an element's
//! declared implicit region.

use crate::refno::RefNo;

const MEMBER_FLAG: u16 = 2;
const EXPLICIT_FLAG: u16 = 1;

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
    /// End of this element within the raw compound record. Bytes after this
    /// offset may be child element records returned by the same Dabacon slot.
    pub record_end: usize,
}

pub fn parse_tail(raw: &[u8], start: usize, refno: RefNo) -> TailData {
    let mut out = TailData {
        record_end: start,
        ..TailData::default()
    };
    let mut cursor = start;

    // One loop over every block the element owns, dispatching on the flag,
    // rather than "one member block, then explicit blocks". A member list can
    // arrive in more than one block: BRAN 24383/85432 in the AMS sample has
    // two (8 words then 7) followed by its explicit stream, and taking only
    // the first left the loop staring at a member block where it wanted an
    // explicit one — so it stopped and reported the element as storing no
    // explicit attributes at all, `NAME` and its UDA value included.
    //
    // `matching_block` already refuses any block whose owner is not this
    // element, which is what keeps the walk from running into the next
    // element's record.
    let mut member_payload = Vec::new();
    let mut explicit_payload = Vec::new();
    loop {
        cursor = skip_markers(raw, cursor);
        let Some((flag, end)) = matching_block(raw, cursor, refno) else {
            break;
        };
        match flag {
            // A member list splits the same way an attribute stream does, and
            // it splits mid-RefNo: on this BRAN the first block ends with the
            // word 24383 and the second begins with 85434, which together are
            // member 24383/85434. Concatenate first, decode after.
            //
            // The two words after the 12-byte header are not part of the list.
            // They are usually zero, and the reader used to skip them only
            // then — but the first block here carries 18010/8193 there, which
            // is not a member (`old-parse-pdms-db` reads two members for this
            // element, not three) and was being reported as one.
            MEMBER_FLAG => {
                member_payload.extend_from_slice(raw.get(cursor + 20..end).unwrap_or_default())
            }
            // Dabacon may split one attribute stream across several flag-1
            // continuation blocks, including between an attribute header and
            // its value. Concatenate their payloads before decoding.
            _ => {
                let mut payload = &raw[cursor + 12..end];
                if payload
                    .get(..8)
                    .is_some_and(|reserved| reserved.iter().all(|value| *value == 0))
                {
                    payload = &payload[8..];
                }
                explicit_payload.extend_from_slice(payload);
            }
        }
        cursor = end;
    }
    parse_members(&member_payload, &mut out.members);
    parse_explicit_payload(&explicit_payload, &mut out.attributes);
    out.record_end = cursor;
    out
}

fn matching_block(raw: &[u8], cursor: usize, refno: RefNo) -> Option<(u16, usize)> {
    let header = raw.get(cursor..cursor + 12)?;
    let flag = u16::from_be_bytes(header[0..2].try_into().ok()?);
    if flag != MEMBER_FLAG && flag != EXPLICIT_FLAG {
        return None;
    }
    let words = u16::from_be_bytes(header[2..4].try_into().ok()?) as usize;
    let bytes = words.checked_mul(4)?;
    if bytes < 12 {
        return None;
    }
    let end = cursor.checked_add(bytes)?;
    if end > raw.len() {
        return None;
    }
    let owner = RefNo::new(read_u32(header, 4)?, read_u32(header, 8)?);
    (owner == refno).then_some((flag, end))
}

fn skip_markers(raw: &[u8], mut cursor: usize) -> usize {
    while let Some(word) = raw.get(cursor..cursor + 4).and_then(|b| read_u32(b, 0)) {
        if word != 0 && word != 7 {
            break;
        }
        cursor += 4;
    }
    cursor
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

fn parse_explicit_payload(payload: &[u8], out: &mut Vec<ExplicitAttribute>) {
    let starts = if payload.get(..8).is_some_and(|b| b.iter().all(|v| *v == 0)) {
        [Some(8usize), Some(0usize)]
    } else {
        [Some(0usize), Some(8usize)]
    };
    for start in starts.into_iter().flatten() {
        let mut parsed = Vec::new();
        if parse_attribute_stream(payload, start, &mut parsed) {
            out.extend(parsed);
            return;
        }
    }
}

fn parse_attribute_stream(
    payload: &[u8],
    mut cursor: usize,
    out: &mut Vec<ExplicitAttribute>,
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
        let Some(value) = decode_value(type_code, data) else {
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

fn decode_value(type_code: u16, data: &[u8]) -> Option<ExplicitValue> {
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
            ExplicitValue::Text(crate::record::text::decode(bytes))
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

    #[test]
    fn parses_reserved_prefix_and_scalar_attributes() {
        let refno = RefNo::new(10, 20);
        let mut block = vec![0, 1, 0, 0];
        block.extend_from_slice(&10u32.to_be_bytes());
        block.extend_from_slice(&20u32.to_be_bytes());
        block.extend_from_slice(&[0; 8]);
        block.extend_from_slice(&0x000A_AFCAu32.to_be_bytes());
        block.extend_from_slice(&0x1400u16.to_be_bytes());
        block.extend_from_slice(&1u16.to_be_bytes());
        block.extend_from_slice(&0u32.to_be_bytes());
        let words = (block.len() / 4) as u16;
        block[2..4].copy_from_slice(&words.to_be_bytes());
        let tail = parse_tail(&block, 0, refno);
        assert_eq!(tail.attributes.len(), 1);
        assert_eq!(tail.attributes[0].value, ExplicitValue::Bool(false));
    }

    #[test]
    fn parses_attribute_split_between_explicit_continuation_blocks() {
        let refno = RefNo::new(10, 20);
        let mut first = vec![0, 1, 0, 7];
        first.extend_from_slice(&10u32.to_be_bytes());
        first.extend_from_slice(&20u32.to_be_bytes());
        first.extend_from_slice(&[0; 8]);
        first.extend_from_slice(&0x000A_FA16u32.to_be_bytes());
        first.extend_from_slice(&0x2800u16.to_be_bytes());
        first.extend_from_slice(&2u16.to_be_bytes());

        let mut raw = first;
        raw.extend_from_slice(&7u32.to_be_bytes());
        let second_start = raw.len();
        raw.extend_from_slice(&[0, 1, 0, 7]);
        raw.extend_from_slice(&10u32.to_be_bytes());
        raw.extend_from_slice(&20u32.to_be_bytes());
        raw.extend_from_slice(&[0; 8]);
        raw.extend_from_slice(&4u32.to_be_bytes());
        raw.extend_from_slice(b"NONE");
        assert_eq!(raw.len() - second_start, 28);

        let tail = parse_tail(&raw, 0, refno);
        assert_eq!(tail.attributes.len(), 1);
        assert_eq!(tail.attributes[0].value, ExplicitValue::Text("NONE".into()));
    }
}
