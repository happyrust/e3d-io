//! Encoding an element record: the inverse of [`crate::record::read_record`]
//! plus [`crate::record::element::ParsedElement`].
//!
//! A record is an eleven-word header, an implicit region, and two optional
//! tail blocks — the member list and the explicit attribute stream — each in
//! the `[kind][words][owner][cont]` shape [`crate::record::block`] reads. The
//! reader gathers a record by following the two `(page, packed)` addresses in
//! the header (words 6/7 for the explicit stream, 8/9 for the members) and
//! then walking each block's continuation, so a writer is free to place the
//! blocks wherever it likes as long as those addresses point at them. This
//! module places them contiguously right after the implicit region — implicit,
//! then the member block, then the explicit block — which is the order the
//! reader splices a scattered record into anyway, so a record this module
//! writes reads back through the same path a `core.dll`-written one does.
//!
//! Word 10 of the header is the storage-format word. Read as
//! `0x20000000 | member_words | (explicit_stream_words << 14)` it matches every
//! one of the 6 639 elements of `ams8000_0001` (the low fourteen bits are the
//! member list's word count, `members × 2`; bits 14..28 are the explicit
//! stream's word count, `Σ(2 + data_words)`; bit 29 is the wide-layout flag,
//! set on all of them). The reader does not consult it — it follows the two
//! addresses instead — but E3D does, so it is written the way E3D writes it.
//!
//! Values are laid into the implicit region in the wide form (bit 29), the one
//! every sampled element uses: a real at a descriptor's wide word offset is two
//! Fortran-ordered words ([`put_real_wide`]), and a real array is a count word
//! then that many reals ([`put_real_vec_wide`]).

use crate::record::block::{block_at, BlockKind, HEADER_BYTES};
use crate::refno::RefNo;

/// Words of header before the first implicit attribute.
const HEADER_WORDS: usize = 11;

/// Bit 29 of the format word: the wide record layout, the one every sampled
/// element uses.
const WIDE_LAYOUT_FLAG: u32 = 0x2000_0000;

/// Block header words: `[kind:u16 | words:u16][owner.0][owner.1][cont.0][cont.1]`.
const BLOCK_HEADER_WORDS: usize = HEADER_BYTES / 4;

/// Why a record could not be built or placed.
#[derive(Debug, thiserror::Error)]
pub enum RecordEncodeError {
    #[error("a record needs at least {HEADER_WORDS} implicit words, got {got}")]
    ImplicitTooShort { got: usize },
    #[error("the explicit stream is {len} bytes, not a whole number of 4-byte words")]
    ExplicitNotDwordAligned { len: usize },
    #[error("an explicit text may only be set to ASCII for now, got {text:?}")]
    ExplicitTextNotAscii { text: String },
    #[error("{refno}: gathered record is {len} bytes, too short for an {HEADER_WORDS}-word header")]
    RecordTooShort { refno: RefNo, len: usize },
    #[error("{refno}: implicit length {words} words runs past the {len}-byte record")]
    ImplicitOverrun { refno: RefNo, words: u32, len: usize },
    #[error(
        "a record of {words} words does not fit on a {page_words}-word page \
         (stage 2 does not split a record across pages)"
    )]
    RecordTooLong { words: usize, page_words: usize },
}

/// One element record to be written, in a form that is easy to change before
/// it is placed on a page.
#[derive(Debug, Clone)]
pub struct RecordDraft {
    pub refno: RefNo,
    pub noun_hash: u32,
    pub owner: RefNo,
    /// The whole implicit region including the eleven-word header. The header
    /// words this module owns — 0 (length), 1/2 (refno), 3 (noun), 4/5
    /// (owner), 6/7 and 8/9 (the block addresses) and 10 (the format word) —
    /// are overwritten on [`RecordDraft::encode`]; the rest is written as it
    /// stands.
    implicit: Vec<u32>,
    members: Vec<RefNo>,
    /// The explicit attribute stream, verbatim, without any block header.
    explicit_stream: Vec<u8>,
}

/// A placed record: its bytes and where they were placed.
#[derive(Debug, Clone)]
pub struct PlacedRecord {
    pub refno: RefNo,
    pub page_no: u32,
    /// 4-byte word offset of the record on its page.
    pub word_offset: u32,
    pub bytes: Vec<u8>,
}

impl RecordDraft {
    /// A record from its parts. `implicit` is the whole implicit region
    /// (header included); `explicit_stream` is the raw attribute stream with no
    /// block header, and must be a whole number of words.
    pub fn new(
        refno: RefNo,
        noun_hash: u32,
        owner: RefNo,
        implicit: Vec<u32>,
        members: Vec<RefNo>,
        explicit_stream: Vec<u8>,
    ) -> Result<Self, RecordEncodeError> {
        if implicit.len() < HEADER_WORDS {
            return Err(RecordEncodeError::ImplicitTooShort { got: implicit.len() });
        }
        if !explicit_stream.len().is_multiple_of(4) {
            return Err(RecordEncodeError::ExplicitNotDwordAligned {
                len: explicit_stream.len(),
            });
        }
        Ok(Self {
            refno,
            noun_hash,
            owner,
            implicit,
            members,
            explicit_stream,
        })
    }

    /// Decode a gathered record ([`crate::record::read_record`] output) back
    /// into a draft, so an existing element can be re-emitted with changes.
    ///
    /// The bytes are one contiguous record — implicit region, then the member
    /// blocks, then the explicit blocks — so this walks the same way
    /// [`crate::record::explicit::parse_tail`] does, keeping the member RefNos
    /// and the explicit stream bytes rather than decoding the attributes.
    pub fn from_record_bytes(bytes: &[u8]) -> Result<Self, RecordEncodeError> {
        let word = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
        if bytes.len() < HEADER_WORDS * 4 {
            return Err(RecordEncodeError::RecordTooShort {
                refno: RefNo::new(0, 0),
                len: bytes.len(),
            });
        }
        let impl_len = word(0) as usize;
        let refno = RefNo::new(word(4), word(8));
        let noun_hash = word(12);
        let owner = RefNo::new(word(16), word(20));
        if impl_len < HEADER_WORDS || impl_len * 4 > bytes.len() {
            return Err(RecordEncodeError::ImplicitOverrun {
                refno,
                words: impl_len as u32,
                len: bytes.len(),
            });
        }
        let implicit: Vec<u32> = (0..impl_len).map(|i| word(i * 4)).collect();

        let mut member_bytes: Vec<u8> = Vec::new();
        let mut explicit_stream: Vec<u8> = Vec::new();
        let mut at = impl_len * 4;
        while let Some(block) = block_at(bytes, at, refno) {
            let Some(payload) = bytes.get(block.payload(at)) else {
                break;
            };
            match block.kind {
                BlockKind::Members => member_bytes.extend_from_slice(payload),
                BlockKind::Explicit => explicit_stream.extend_from_slice(payload),
            }
            at += block.total_bytes;
        }

        let mut members = Vec::new();
        let mut cursor = 0;
        while let Some(pair) = member_bytes.get(cursor..cursor + 8) {
            let member = RefNo::new(
                u32::from_be_bytes(pair[0..4].try_into().unwrap()),
                u32::from_be_bytes(pair[4..8].try_into().unwrap()),
            );
            if member.word0 == 0 && member.word1 == 0 {
                break;
            }
            members.push(member);
            cursor += 8;
        }

        Self::new(refno, noun_hash, owner, implicit, members, explicit_stream)
    }

    /// Append a member. The word-10 count and the block grow with it.
    pub fn push_member(&mut self, member: RefNo) {
        self.members.push(member);
    }

    /// Remove a member, keeping the rest in order. Returns whether it was
    /// there.
    pub fn remove_member(&mut self, member: RefNo) -> bool {
        let before = self.members.len();
        self.members.retain(|m| *m != member);
        self.members.len() != before
    }

    pub fn members(&self) -> &[RefNo] {
        &self.members
    }

    /// The whole implicit region, header included.
    pub fn implicit(&self) -> &[u32] {
        &self.implicit
    }

    /// Set the value at wide word offset `offset`: the header-relative word,
    /// the way a descriptor's `wide.location.word_offset()` states it.
    pub fn implicit_word_mut(&mut self, offset: usize) -> Option<&mut u32> {
        self.implicit.get_mut(offset)
    }

    /// Overwrite the real at wide word offset `offset` (two words, Fortran
    /// order). Panics if the offset runs past the implicit region.
    pub fn set_real_wide(&mut self, offset: usize, value: f64) {
        put_real_wide(&mut self.implicit, offset, value);
    }

    /// Replace the value of the explicit text attribute keyed by `hash` with
    /// `text`, keeping its type code. Returns whether such an attribute was
    /// there. ASCII only for now — under either header `FORMAT` it stores as
    /// its own bytes, the same rule the session-page writer keeps.
    pub fn set_explicit_text_ascii(
        &mut self,
        hash: u32,
        text: &str,
    ) -> Result<bool, RecordEncodeError> {
        if !text.is_ascii() {
            return Err(RecordEncodeError::ExplicitTextNotAscii {
                text: text.to_owned(),
            });
        }
        let mut fields = self.parse_explicit_fields();
        let Some(field) = fields.iter_mut().find(|(field_hash, _, _)| *field_hash == hash) else {
            return Ok(false);
        };
        let bytes = text.as_bytes();
        let mut data = Vec::with_capacity(1 + bytes.len().div_ceil(4));
        data.push(bytes.len() as u32);
        for chunk in bytes.chunks(4) {
            let mut word = [0u8; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            data.push(u32::from_be_bytes(word));
        }
        field.2 = data;
        self.explicit_stream = rebuild_explicit_stream(&fields);
        Ok(true)
    }

    /// The explicit stream as `(hash, type_code, data_words)` fields.
    fn parse_explicit_fields(&self) -> Vec<(u32, u16, Vec<u32>)> {
        let words: Vec<u32> = self
            .explicit_stream
            .chunks_exact(4)
            .map(|w| u32::from_be_bytes(w.try_into().unwrap()))
            .collect();
        let mut fields = Vec::new();
        let mut cursor = 0;
        while cursor + 2 <= words.len() {
            let hash = words[cursor];
            if hash == 0 {
                break;
            }
            let type_code = (words[cursor + 1] >> 16) as u16;
            let data_words = (words[cursor + 1] & 0xFFFF) as usize;
            let Some(data) = words.get(cursor + 2..cursor + 2 + data_words) else {
                break;
            };
            fields.push((hash, type_code, data.to_vec()));
            cursor += 2 + data_words;
        }
        fields
    }

    /// The whole record's length in words: implicit region plus both blocks.
    pub fn encoded_len_words(&self) -> usize {
        self.implicit.len() + self.member_block_words() + self.explicit_block_words()
    }

    fn member_block_words(&self) -> usize {
        if self.members.is_empty() {
            0
        } else {
            BLOCK_HEADER_WORDS + self.members.len() * 2
        }
    }

    fn explicit_block_words(&self) -> usize {
        if self.explicit_stream.is_empty() {
            0
        } else {
            BLOCK_HEADER_WORDS + self.explicit_stream.len() / 4
        }
    }

    /// Lay the record out at `word_offset` on `page`, in extent `ext`.
    ///
    /// The header's two block addresses are set to point at the member block
    /// (right after the implicit region) and the explicit block (right after
    /// the member block), both on the same page.
    pub fn encode(&self, page: u32, word_offset: u32, ext: u16) -> PlacedRecord {
        let mut words = self.implicit.clone();
        let member_words = self.members.len() * 2;
        let explicit_words = self.explicit_stream.len() / 4;

        let members_at = word_offset + self.implicit.len() as u32;
        let explicit_at = members_at + self.member_block_words() as u32;

        words[0] = self.implicit.len() as u32;
        words[1] = self.refno.word0;
        words[2] = self.refno.word1;
        words[3] = self.noun_hash;
        words[4] = self.owner.word0;
        words[5] = self.owner.word1;
        if self.explicit_stream.is_empty() {
            words[6] = 0;
            words[7] = 0;
        } else {
            words[6] = page;
            words[7] = pack_offset(explicit_at, ext);
        }
        if self.members.is_empty() {
            words[8] = 0;
            words[9] = 0;
        } else {
            words[8] = page;
            words[9] = pack_offset(members_at, ext);
        }
        words[10] = WIDE_LAYOUT_FLAG | member_words as u32 | ((explicit_words as u32) << 14);

        let mut bytes = Vec::with_capacity(self.encoded_len_words() * 4);
        for word in &words {
            bytes.extend_from_slice(&word.to_be_bytes());
        }
        if !self.members.is_empty() {
            push_block_header(&mut bytes, BlockKind::Members, self.member_block_words(), self.refno);
            for member in &self.members {
                bytes.extend_from_slice(&member.word0.to_be_bytes());
                bytes.extend_from_slice(&member.word1.to_be_bytes());
            }
        }
        if !self.explicit_stream.is_empty() {
            push_block_header(
                &mut bytes,
                BlockKind::Explicit,
                self.explicit_block_words(),
                self.refno,
            );
            bytes.extend_from_slice(&self.explicit_stream);
        }

        PlacedRecord {
            refno: self.refno,
            page_no: page,
            word_offset,
            bytes,
        }
    }
}

/// The packed second word of a `(page, packed)` address: `ext | (offset << 13)`
/// with the offset in 4-byte words, exactly as `sub_5AE9DF0` writes it and the
/// index and block readers unpack it.
pub fn pack_offset(word_offset: u32, ext: u16) -> u32 {
    u32::from(ext) | (word_offset << 13)
}

/// Attribute-level three-way merge (`docs/plans/2026-09-20-dabacon-write-path-algorithms.md`
/// A3b): rebase the writer's change onto the committed record.
///
/// `base` is the record the writer opened, `latest` the committed one, and
/// `working` the writer's edited record. The result starts from `latest` and
/// takes only what the writer actually changed (`base` → `working`): an
/// implicit word, an explicit attribute or a member the writer left as it was
/// keeps `latest`'s value, so a change to one attribute and a change to
/// another compose. Where both changed the same thing the writer wins — the
/// A3b rule for a plain `SAVEWORK` (`extract flush` gives the committed side;
/// this crate does not do extract flush).
///
/// The member merge is the simple set delta — `latest`'s members plus the ones
/// the writer added, less the ones it removed — not yet the anchor-ordered
/// reconciliation of A3b.4.
pub fn three_way(
    base: &RecordDraft,
    latest: &RecordDraft,
    working: &RecordDraft,
) -> Result<RecordDraft, RecordEncodeError> {
    // Implicit region: latest's, with the words the writer changed overlaid.
    // The header words (0..11) are recomputed on encode, so only attribute
    // words matter.
    let mut implicit = latest.implicit.clone();
    let common = implicit
        .len()
        .min(base.implicit.len())
        .min(working.implicit.len());
    for index in HEADER_WORDS..common {
        if working.implicit[index] != base.implicit[index] {
            implicit[index] = working.implicit[index];
        }
    }

    // Explicit attributes, keyed by hash: latest's, with the writer's changes
    // (added, replaced or removed relative to base) applied.
    let base_fields = base.parse_explicit_fields();
    let working_fields = working.parse_explicit_fields();
    let mut merged_fields = latest.parse_explicit_fields();
    let value_of = |fields: &[(u32, u16, Vec<u32>)], hash: u32| {
        fields.iter().find(|f| f.0 == hash).map(|f| (f.1, f.2.clone()))
    };
    let mut touched: Vec<u32> = base_fields
        .iter()
        .map(|f| f.0)
        .chain(working_fields.iter().map(|f| f.0))
        .collect();
    touched.sort_unstable();
    touched.dedup();
    for hash in touched {
        let in_base = value_of(&base_fields, hash);
        let in_working = value_of(&working_fields, hash);
        if in_base == in_working {
            continue; // the writer did not change this attribute
        }
        match in_working {
            Some((type_code, data)) => match merged_fields.iter_mut().find(|f| f.0 == hash) {
                Some(slot) => {
                    slot.1 = type_code;
                    slot.2 = data;
                }
                None => merged_fields.push((hash, type_code, data)),
            },
            None => merged_fields.retain(|f| f.0 != hash),
        }
    }
    let explicit_stream = rebuild_explicit_stream(&merged_fields);

    // Members: latest's, plus the writer's additions, less its removals.
    let mut members = latest.members.clone();
    for member in &working.members {
        if !base.members.contains(member) && !members.contains(member) {
            members.push(*member);
        }
    }
    members.retain(|member| working.members.contains(member) || !base.members.contains(member));

    let owner = if working.owner != base.owner {
        working.owner
    } else {
        latest.owner
    };

    RecordDraft::new(working.refno, latest.noun_hash, owner, implicit, members, explicit_stream)
}

/// Serialise explicit fields back into a stream: `[hash][type_code << 16 |
/// data_words][data…]` per entry, the inverse of the reader's walk.
fn rebuild_explicit_stream(fields: &[(u32, u16, Vec<u32>)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (hash, type_code, data) in fields {
        bytes.extend_from_slice(&hash.to_be_bytes());
        let header = (u32::from(*type_code) << 16) | data.len() as u32;
        bytes.extend_from_slice(&header.to_be_bytes());
        for word in data {
            bytes.extend_from_slice(&word.to_be_bytes());
        }
    }
    bytes
}

fn push_block_header(bytes: &mut Vec<u8>, kind: BlockKind, words: usize, owner: RefNo) {
    let kind_word: u16 = match kind {
        BlockKind::Members => 2,
        BlockKind::Explicit => 1,
    };
    bytes.extend_from_slice(&kind_word.to_be_bytes());
    bytes.extend_from_slice(&(words as u16).to_be_bytes());
    bytes.extend_from_slice(&owner.word0.to_be_bytes());
    bytes.extend_from_slice(&owner.word1.to_be_bytes());
    // Continuation address: none, the block is whole.
    bytes.extend_from_slice(&0u32.to_be_bytes());
    bytes.extend_from_slice(&0u32.to_be_bytes());
}

/// Write a real at wide word offset `offset` of the implicit region: two words
/// in the Fortran order [`crate::record::attrs::read_f64`] reads — low half
/// first, high half second.
pub fn put_real_wide(words: &mut [u32], offset: usize, value: f64) {
    let bits = value.to_bits();
    words[offset] = bits as u32;
    words[offset + 1] = (bits >> 32) as u32;
}

/// Write a real array at wide word offset `offset`: a count word, then each
/// value as two Fortran-ordered words. This is the wide form of a
/// `logical_count > 1` real descriptor such as `POS`.
pub fn put_real_vec_wide(words: &mut [u32], offset: usize, values: &[f64]) {
    words[offset] = values.len() as u32;
    for (index, value) in values.iter().enumerate() {
        put_real_wide(words, offset + 1 + index * 2, *value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::element::ParsedElement;
    use crate::record::explicit::ExplicitValue;
    use crate::record::read_record;
    use crate::page::cache::PageCache;
    use crate::page::io::PageReader;
    use crate::page::PageId;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    const PAGE_WORDS: usize = 128;

    /// An implicit region: an eleven-word header (its owned words are set on
    /// encode) followed by `attrs` attribute words.
    fn implicit(attrs: &[u32]) -> Vec<u32> {
        let mut words = vec![0u32; HEADER_WORDS];
        words.extend_from_slice(attrs);
        words
    }

    #[test]
    fn a_real_round_trips_through_the_fortran_word_order() {
        let mut words = vec![0u32; 8];
        put_real_wide(&mut words, 2, 310.0);
        let mut bytes = Vec::new();
        for word in &words {
            bytes.extend_from_slice(&word.to_be_bytes());
        }
        assert_eq!(crate::record::attrs::read_f64(&bytes, 2), Some(310.0));

        put_real_vec_wide(&mut words, 0, &[1.5, 2.5, 3.5]);
        assert_eq!(words[0], 3);
        let mut bytes = Vec::new();
        for word in &words {
            bytes.extend_from_slice(&word.to_be_bytes());
        }
        assert_eq!(crate::record::attrs::read_f64(&bytes, 1), Some(1.5));
        assert_eq!(crate::record::attrs::read_f64(&bytes, 3), Some(2.5));
        assert_eq!(crate::record::attrs::read_f64(&bytes, 5), Some(3.5));
    }

    /// Write a record to a page, read it back through the record reader, and
    /// check every part survives — the header, a real attribute, the members
    /// and the explicit stream.
    #[test]
    fn a_written_record_reads_back_through_the_reader() {
        let refno = RefNo::new(24_384, 30_000);
        let owner = RefNo::new(24_384, 24_932);
        let mut words = implicit(&[0u32; 26]);
        // A real at wide offset 29, the BOX XLEN slot.
        put_real_wide(&mut words, 29, 310.0);
        // An explicit stream: one integer attribute, hash 0xAB, type 0x0C00.
        let mut stream = Vec::new();
        stream.extend_from_slice(&0xABu32.to_be_bytes());
        stream.extend_from_slice(&0x0C00u16.to_be_bytes());
        stream.extend_from_slice(&1u16.to_be_bytes());
        stream.extend_from_slice(&7u32.to_be_bytes());
        let members = vec![RefNo::new(24_384, 30_001), RefNo::new(24_384, 30_002)];

        let draft = RecordDraft::new(refno, 0x861E0, owner, words, members.clone(), stream).unwrap();
        let placed = draft.encode(1, 1, PageId::PRIMARY);

        // Lay the record on a two-page file and read it.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("ams1000_0001");
        let mut file = File::create(&path).unwrap();
        let mut page0 = vec![0u8; PAGE_WORDS * 4];
        page0[0x34..0x38].copy_from_slice(&(PAGE_WORDS as u32).to_be_bytes());
        file.write_all(&page0).unwrap();
        let mut page1 = vec![0u8; PAGE_WORDS * 4];
        page1[0..4].copy_from_slice(&7u32.to_be_bytes());
        page1[4..4 + placed.bytes.len()].copy_from_slice(&placed.bytes);
        file.write_all(&page1).unwrap();
        file.flush().unwrap();

        let reader = PageReader::open(&path).unwrap();
        let mut cache = PageCache::new(reader, 4);
        let record = read_record(&mut cache, PageId::primary(1), placed.word_offset * 2).unwrap();
        let parsed = ParsedElement::parse(&record.bytes).unwrap();

        assert_eq!(parsed.refno, refno);
        assert_eq!(parsed.noun_hash, 0x861E0);
        assert_eq!(parsed.owner, owner);
        assert_eq!(parsed.members, members);
        assert_eq!(parsed.explicit_attributes.len(), 1);
        assert_eq!(parsed.explicit_attributes[0].hash, 0xAB);
        assert_eq!(parsed.explicit_attributes[0].value, ExplicitValue::Integer(7));
        assert_eq!(
            crate::record::attrs::read_f64(&parsed.implicit_data, 29),
            Some(310.0)
        );
    }

    /// Attribute-level three-way: changes to different attributes compose, a
    /// change to the same one is the writer's, and an attribute only the
    /// committed side touched is kept.
    #[test]
    fn three_way_composes_disjoint_changes_and_prefers_the_writer_on_a_clash() {
        let refno = RefNo::new(1, 10);
        let owner = RefNo::new(1, 1);
        let explicit = |hash: u32, value: u32| {
            let mut s = Vec::new();
            s.extend_from_slice(&hash.to_be_bytes());
            s.extend_from_slice(&((0x0C00u32 << 16) | 1).to_be_bytes());
            s.extend_from_slice(&value.to_be_bytes());
            s
        };

        let base = RecordDraft::new(refno, 0x861E0, owner, implicit(&[10, 20, 30]), vec![], explicit(0xAA, 1)).unwrap();
        // Committed side changed implicit word 12 and explicit 0xAA, and added 0xBB.
        let mut latest_words = implicit(&[10, 20, 30]);
        latest_words[12] = 120;
        let mut latest_stream = explicit(0xAA, 2);
        latest_stream.extend_from_slice(&explicit(0xBB, 9));
        let latest = RecordDraft::new(refno, 0x861E0, owner, latest_words, vec![], latest_stream).unwrap();
        // Writer changed implicit word 13 and explicit 0xAA to its own value.
        let mut working_words = implicit(&[10, 20, 30]);
        working_words[13] = 130;
        let working = RecordDraft::new(refno, 0x861E0, owner, working_words, vec![], explicit(0xAA, 7)).unwrap();

        let merged = three_way(&base, &latest, &working).unwrap();
        assert_eq!(merged.implicit()[11], 10, "unchanged word keeps base/latest");
        assert_eq!(merged.implicit()[12], 120, "committed-only change kept");
        assert_eq!(merged.implicit()[13], 130, "writer-only change applied");

        let fields = merged.parse_explicit_fields();
        let value = |hash: u32| fields.iter().find(|f| f.0 == hash).map(|f| f.2[0]);
        assert_eq!(value(0xAA), Some(7), "both changed 0xAA: the writer wins");
        assert_eq!(value(0xBB), Some(9), "committed-only attribute is kept");
    }

    /// A record decoded from its own bytes and re-encoded is byte-for-byte the
    /// same, so an element can be modified by decode, change, re-encode.
    #[test]
    fn decode_then_encode_is_the_identity() {
        let refno = RefNo::new(24_384, 47);
        let owner = RefNo::new(24_384, 46);
        let mut stream = Vec::new();
        stream.extend_from_slice(&0x9C18Eu32.to_be_bytes());
        stream.extend_from_slice(&0x3C00u16.to_be_bytes());
        stream.extend_from_slice(&1u16.to_be_bytes());
        stream.extend_from_slice(&0x2F463100u32.to_be_bytes());
        let draft = RecordDraft::new(
            refno,
            0x640493,
            owner,
            implicit(&[1, 2, 3, 4]),
            vec![RefNo::new(24_384, 48)],
            stream,
        )
        .unwrap();
        let placed = draft.encode(9, 5, PageId::PRIMARY);

        let redraft = RecordDraft::from_record_bytes(&placed.bytes).unwrap();
        let replaced = redraft.encode(9, 5, PageId::PRIMARY);
        assert_eq!(placed.bytes, replaced.bytes);

        // Adding a member grows the record by one RefNo and bumps word 10.
        let mut grown = RecordDraft::from_record_bytes(&placed.bytes).unwrap();
        grown.push_member(RefNo::new(24_384, 49));
        assert_eq!(grown.members().len(), 2);
        let grown = grown.encode(9, 5, PageId::PRIMARY);
        let member_words = u32::from_be_bytes(grown.bytes[40..44].try_into().unwrap()) & 0x3FFF;
        assert_eq!(member_words, 4);
    }
}
