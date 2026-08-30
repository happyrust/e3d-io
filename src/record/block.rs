//! What a tail block is, decoded in one place.
//!
//! An element's members and its explicit attributes arrive in blocks that
//! follow its implicit region:
//!
//! ```text
//! [kind:u16][words:u16][owner.word0][owner.word1][cont_pgno][cont_packed] payload…
//! ```
//!
//! Twenty bytes of header and `words * 4 - 20` of payload. The two words after
//! the owner are the address the block continues at, in the same
//! `(page_no, packed)` encoding an index leaf value uses, and they are zero on
//! a block that does not continue.
//!
//! They were previously read as "a reserved pair, usually zero" and skipped
//! only when they were. That is why a member list came back with a phantom
//! member — BRAN 24383/85432 of `ams7999_0001` carries `18010 / 0x2001` there,
//! which was reported as member 18010/8193 — and why the explicit decoder
//! carried a fallback that tried the stream at two different offsets.
//!
//! Across 438 sample databases and 3 148 449 blocks, 158 526 state a
//! continuation. Every one of them ends exactly at the page boundary, and
//! every one lands on a block of the same element. 5 444 blocks also end
//! exactly at a page boundary and state no continuation: their record ends
//! there. Ending at the boundary is therefore not what says a block continues
//! — the stated address is.

use crate::refno::RefNo;

/// Header bytes before a block's payload.
pub const HEADER_BYTES: usize = 20;

/// Smallest block the sample corpus holds is 7 words; the header alone needs 5.
pub const MIN_WORDS: usize = HEADER_BYTES / 4;

/// What a block carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// A run of the owner's member RefNos.
    Members,
    /// A run of the owner's explicit attribute stream.
    Explicit,
}

impl BlockKind {
    fn from_raw(value: u16) -> Option<Self> {
        match value {
            2 => Some(Self::Members),
            1 => Some(Self::Explicit),
            _ => None,
        }
    }
}

/// Where a part of a record lives: a `(page_no, packed)` pair.
///
/// The same encoding an index leaf value uses. `offset_words` counts 2-byte
/// words from the top of the page and [`Address::byte_offset`] is the only
/// conversion. Both slots in a record header and the continuation field of
/// every block are written this way, which is why they are read here rather
/// than three times over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Address {
    pub page_no: u32,
    pub offset_words: u32,
    /// Low 12 bits of the packed word. Reads 1 on all 158 526 block
    /// continuations in the corpus, exactly as it does on an index leaf value,
    /// and takes no part in addressing.
    pub flag: u16,
}

impl Address {
    /// The pair at `offset`, or `None` when both words are zero, which is how
    /// "there is nothing here" is written.
    pub fn at(data: &[u8], offset: usize) -> Option<Self> {
        let page_no = be32(data, offset)?;
        let packed = be32(data, offset + 4)?;
        (page_no != 0 || packed != 0).then_some(Self {
            page_no,
            offset_words: packed >> 12,
            flag: (packed & 0xFFF) as u16,
        })
    }

    pub const fn byte_offset(&self) -> usize {
        self.offset_words as usize * 2
    }
}

/// One tail block belonging to a known element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    /// Bytes of the whole block, header included.
    pub total_bytes: usize,
    pub continues_at: Option<Address>,
}

impl Block {
    /// Byte range of this block's payload within the page it sits on.
    pub fn payload(&self, at: usize) -> std::ops::Range<usize> {
        at + HEADER_BYTES..at + self.total_bytes
    }
}

/// The block at `at`, if one belonging to `owner` starts there.
///
/// **The only place a block header is read.** The record reader uses it to
/// learn how far a record reaches and the tail parser uses it to find the
/// payloads, so the two cannot disagree about where one element's bytes stop
/// and the next element's begin.
///
/// `None` means "no block of this element starts here", which is how a record
/// ends: the words that follow are the next record's header, or the page's
/// unused tail. Refusing a block whose owner is somebody else is what keeps
/// the walk inside one element.
pub fn block_at(data: &[u8], at: usize, owner: RefNo) -> Option<Block> {
    let header = data.get(at..at + HEADER_BYTES)?;
    let kind = BlockKind::from_raw(u16::from_be_bytes(header[0..2].try_into().ok()?))?;
    let words = u16::from_be_bytes(header[2..4].try_into().ok()?) as usize;
    if words < MIN_WORDS {
        return None;
    }
    if RefNo::new(be32(header, 4)?, be32(header, 8)?) != owner {
        return None;
    }

    Some(Block {
        kind,
        total_bytes: words * 4,
        continues_at: Address::at(header, 12),
    })
}

fn be32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(kind: u16, words: u16, owner: RefNo, cont: (u32, u32)) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&kind.to_be_bytes());
        out.extend_from_slice(&words.to_be_bytes());
        out.extend_from_slice(&owner.word0.to_be_bytes());
        out.extend_from_slice(&owner.word1.to_be_bytes());
        out.extend_from_slice(&cont.0.to_be_bytes());
        out.extend_from_slice(&cont.1.to_be_bytes());
        out.resize(words as usize * 4, 0);
        out
    }

    #[test]
    fn a_block_states_its_kind_length_and_owner() {
        let owner = RefNo::new(24383, 85432);
        let decoded = block_at(&block(2, 8, owner, (0, 0)), 0, owner).expect("a member block");
        assert_eq!(decoded.kind, BlockKind::Members);
        assert_eq!(decoded.total_bytes, 32);
        assert_eq!(decoded.continues_at, None);
        assert_eq!(decoded.payload(0), 20..32);
    }

    /// The two words after the owner are BRAN 24383/85432's own, from
    /// `ams7999_0001` page 18009: the rest of its member list is at page 18010,
    /// byte 4.
    #[test]
    fn a_block_that_continues_states_where() {
        let owner = RefNo::new(24383, 85432);
        let decoded =
            block_at(&block(2, 8, owner, (18010, 0x2001)), 0, owner).expect("a member block");
        let cont = decoded.continues_at.expect("it continues");
        assert_eq!(cont.page_no, 18010);
        assert_eq!(cont.byte_offset(), 4);
        assert_eq!(cont.flag, 1);
    }

    #[test]
    fn a_block_owned_by_another_element_is_not_this_one() {
        let owner = RefNo::new(24383, 85432);
        let other = RefNo::new(24383, 85433);
        assert!(block_at(&block(1, 7, other, (0, 0)), 0, owner).is_none());
    }

    #[test]
    fn a_word_that_is_not_a_block_header_ends_the_walk() {
        let owner = RefNo::new(10, 20);
        // The next record's header: its first word is an implicit length.
        let mut data = vec![0u8; 40];
        data[0..4].copy_from_slice(&86u32.to_be_bytes());
        data[4..8].copy_from_slice(&10u32.to_be_bytes());
        data[8..12].copy_from_slice(&20u32.to_be_bytes());
        assert!(block_at(&data, 0, owner).is_none());

        // A block too short to hold its own header.
        assert!(block_at(&block(1, 4, owner, (0, 0)), 0, owner).is_none());
    }
}
