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
//!
//! Two shapes of explicit value have their own setters ([`crate::record::hunk`]
//! has the layouts, read out of `core.dll`). A **string list** (storage type
//! 19) is `[n] { [i][words][bytes][packed text] }*` under a header of
//! `19 << 26 | words` with no separate count word
//! ([`RecordDraft::set_explicit_text_list_ascii`]). A **hunk** (types 20–23,
//! the out-of-record form of an int array, real array, text or string list)
//! is a seven-word directory entry in the stream — `[hash][type << 26 | 5]
//! [1][page][packed][count][words]` — with the value itself in a kind-3 block
//! chain elsewhere. A draft holds such a value as a [`HunkDraft`] until the
//! record is placed: the directory entry carries address `(0, 0)` in the
//! draft, and the placer ([`crate::write::stage_element_session`]'s
//! `place_record`) lays the hunk down first, as `4SCEA` does, and patches the
//! address in ([`RecordDraft::explicit_stream_words_with_hunks`]). A hunk
//! entry read back from disk keeps its address and is re-emitted as it stands,
//! so a rewrite of the record leaves the value where it is — a page once
//! written is never changed, so the address stays good.

use crate::record::block::{block_at, BlockKind, HEADER_BYTES};
use crate::record::hunk::{self, HunkRef, TEXT_LIST_TYPE};
use crate::refno::RefNo;

/// Words of header before the first implicit attribute.
const HEADER_WORDS: usize = 11;

/// Bit 29 of the format word: the wide record layout, the one every sampled
/// element uses.
const WIDE_LAYOUT_FLAG: u32 = 0x2000_0000;

/// The explicit-stream type code of a type-16 reference attribute: the
/// storage type shifted by ten, as every explicit field's header states it
/// (`crate::record::explicit` decodes `0x4000` as a RefNo).
pub const REFERENCE_TYPE_CODE: u16 = 16 << 10;

/// The explicit-stream type code of a type-17 reference array (`HCREF`,
/// `TCREF`, `MDSYSF`, `CONNRE`, …): data is a count word then that many
/// `(word0, word1)` pairs — the 2 754 such fields in the corpus all state
/// `0x4400`, and four of them keep more pairs than the count says (a slot
/// sized for the descriptor's capacity), so the count, not the length, says
/// how many are set.
pub const REFERENCE_ARRAY_TYPE_CODE: u16 = 17 << 10;

/// The explicit-stream type code of a type-14 keyed attribute (`TYPEX`):
/// data is a count word then the elements.
pub const KEYED_TYPE_CODE: u16 = 14 << 10;

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
    #[error(
        "explicit attribute {hash:#x}: {words} data words is more than an inline entry's \
         length field holds ({max}); store it as a hunk"
    )]
    ExplicitTooLong { hash: u32, words: usize, max: usize },
    #[error("storage type {storage_type} is not a hunk type (20, 21, 22 or 23)")]
    NotAHunkType { storage_type: u16 },
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
    /// Out-of-record values set on this draft and not yet placed. Each has a
    /// directory entry in `explicit_stream` whose address reads `(0, 0)`
    /// until the placer fills it in.
    hunks: Vec<HunkDraft>,
}

/// A value to be written out of the record, in a hunk (storage type 20–23):
/// what the placer lays down as a kind-3 block chain and points the
/// attribute's directory entry at. See [`crate::record::hunk`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HunkDraft {
    pub hash: u32,
    /// 20 (ints), 21 (reals), 22 (text) or 23 (string list).
    pub hunk_type: u16,
    /// The element count the directory entry states — what `core.dll`'s
    /// caller passed `4SCEA` as the value's length.
    pub count: u32,
    /// The hunk's payload, exactly the words the chain carries.
    pub payload: Vec<u32>,
}

/// The largest data-word count an inline explicit entry can state here: this
/// module's field walk and the stream reader ([`crate::record::explicit`])
/// split the header word into two 16-bit halves. `core.dll`'s field is 26
/// bits, but it moves anything past [`hunk::HUNK_THRESHOLD_WORDS`] (`0x2000`)
/// out of the record, so no inline entry it writes comes near this.
pub const MAX_INLINE_DATA_WORDS: usize = 0xFFFF;

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
            hunks: Vec::new(),
        })
    }

    /// Decode a gathered record ([`crate::record::read_record`] output) back
    /// into a draft, so an existing element can be re-emitted with changes.
    ///
    /// The bytes are one contiguous record — implicit region, then the member
    /// blocks, then the explicit blocks — so this walks the same way
    /// [`crate::record::explicit::parse_tail`] does, keeping the member RefNos
    /// and the explicit stream bytes rather than decoding the attributes. A
    /// hunk directory entry in the stream keeps the address it has; the value
    /// it points at is not read and stays where it is.
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

    /// Move an existing member to directly after `after`, or to the front
    /// when `after` is `None` — the list edit a `REORDER` makes. Returns
    /// whether the move happened; it does not when either member is missing
    /// or `after` is the member itself.
    pub fn move_member_after(&mut self, member: RefNo, after: Option<RefNo>) -> bool {
        let Some(current) = self.members.iter().position(|m| *m == member) else {
            return false;
        };
        if after.is_some_and(|anchor| anchor == member || !self.members.contains(&anchor)) {
            return false;
        }
        self.members.remove(current);
        let slot = after
            .and_then(|anchor| self.members.iter().position(|m| *m == anchor))
            .map_or(0, |anchor| anchor + 1);
        self.members.insert(slot, member);
        true
    }

    pub fn members(&self) -> &[RefNo] {
        &self.members
    }

    /// The whole implicit region, header included.
    pub fn implicit(&self) -> &[u32] {
        &self.implicit
    }

    /// The explicit attribute stream as big-endian words — the payload of the
    /// explicit tail block, before any block header.
    pub fn explicit_stream_words(&self) -> Vec<u32> {
        stream_words(&self.explicit_stream)
    }

    /// Whether two records carry the same content — noun, owner, members, the
    /// implicit region past its header (the header words are addresses and the
    /// format word, recomputed on encode), the explicit attributes as fields
    /// (so a difference in stream padding does not count) and any hunk values
    /// waiting to be placed. A hunk value newly set on one side is a pending
    /// [`HunkDraft`] with a `(0, 0)` directory address, so it never compares
    /// equal to a placed one — the record is rewritten, as it must be.
    ///
    /// This is what INDEX de-duplication (`sub_5B07080`, A3a) tests: a merged
    /// record equal to the committed one is not written, and its index leaf
    /// keeps pointing at the record already on disk.
    pub fn content_eq(&self, other: &RecordDraft) -> bool {
        self.noun_hash == other.noun_hash
            && self.owner == other.owner
            && self.members == other.members
            && self.implicit.get(HEADER_WORDS..) == other.implicit.get(HEADER_WORDS..)
            && self.parse_explicit_fields() == other.parse_explicit_fields()
            && self.hunks == other.hunks
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
        let data = ascii_text_data(text)?;
        let mut fields = self.parse_explicit_fields();
        let Some(field) = fields.iter_mut().find(|(field_hash, _, _)| *field_hash == hash) else {
            return Ok(false);
        };
        field.2 = data;
        self.explicit_stream = rebuild_explicit_stream(&fields);
        Ok(true)
    }

    /// Add a text attribute the element does not carry, or replace it if it
    /// does — the way an element gains a `NAME` it never had. `type_code` is
    /// the stored type (`0x3C00` for a `FORMAT 1` text, the shape `NAME`
    /// takes). ASCII only, like [`Self::set_explicit_text_ascii`].
    pub fn add_explicit_text_ascii(
        &mut self,
        hash: u32,
        type_code: u16,
        text: &str,
    ) -> Result<(), RecordEncodeError> {
        let data = ascii_text_data(text)?;
        let mut fields = self.parse_explicit_fields();
        match fields.iter_mut().find(|(field_hash, _, _)| *field_hash == hash) {
            Some(field) => {
                field.1 = type_code;
                field.2 = data;
            }
            None => fields.push((hash, type_code, data)),
        }
        self.explicit_stream = rebuild_explicit_stream(&fields);
        Ok(())
    }

    /// The explicit attribute keyed by `hash`, as its stored type code and
    /// data words, or `None` when the element does not carry it.
    pub fn explicit_words(&self, hash: u32) -> Option<(u16, Vec<u32>)> {
        self.parse_explicit_fields()
            .into_iter()
            .find(|(field_hash, _, _)| *field_hash == hash)
            .map(|(_, type_code, data)| (type_code, data))
    }

    /// Set an explicit attribute's stored words verbatim: replaced in place
    /// when the element carries it, appended to the stream when it does not.
    pub fn set_explicit_words(&mut self, hash: u32, type_code: u16, data: Vec<u32>) {
        self.upsert_explicit(hash, type_code, data, false);
    }

    /// [`Self::set_explicit_words`], but a field the element lacks goes to the
    /// **front** of the stream — where `core.dll` puts `TYPEX`, which it sets
    /// before anything else at element creation (every `TYPEX`-carrying record
    /// of `ams8000_0001` opens its stream with it).
    pub fn set_explicit_words_first(&mut self, hash: u32, type_code: u16, data: Vec<u32>) {
        self.upsert_explicit(hash, type_code, data, true);
    }

    /// Set a reference attribute that lives in the explicit stream: type code
    /// [`REFERENCE_TYPE_CODE`], the two RefNo words as data.
    pub fn set_explicit_refno(&mut self, hash: u32, value: RefNo) {
        self.set_explicit_words(hash, REFERENCE_TYPE_CODE, vec![value.word0, value.word1]);
    }

    /// Set a string-list attribute (storage type 19) inline: the
    /// `[n] { [i][words][bytes][packed text] }*` buffer `db_cxx_put_str_vector`
    /// builds, under type code `19 << 10` with the buffer's own length as the
    /// entry's data-word count and no separate count word — `4SCEA`'s `type ==
    /// 19` arm ([`crate::record::hunk`]). ASCII only, like the text setters.
    /// An empty list unsets the attribute — `db_cxx_put_str_vector` passes an
    /// empty vector to `4SCEA` as a value of length 0, which deletes the entry
    /// and writes nothing. A list past [`MAX_INLINE_DATA_WORDS`] is refused;
    /// use [`Self::set_text_list_hunk_ascii`] for it (`core.dll` moves
    /// anything past `0x2000` words out of the record).
    pub fn set_explicit_text_list_ascii<I, S>(&mut self, hash: u32, items: I) -> Result<(), RecordEncodeError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let (data, _count) = text_list_data_ascii(items)?;
        if data[0] == 0 {
            self.remove_explicit(hash);
            return Ok(());
        }
        self.set_explicit_words_checked(hash, TEXT_LIST_TYPE << 10, data)
    }

    /// Set an attribute as a hunk (storage type 20–23) from its payload words:
    /// the seven-word directory entry goes into the stream with address
    /// `(0, 0)` and the value waits as a [`HunkDraft`] for the placer. `count`
    /// is the element count the entry states. Any inline value the attribute
    /// had is replaced.
    pub fn set_hunk_words(
        &mut self,
        hash: u32,
        hunk_type: u16,
        count: u32,
        payload: Vec<u32>,
    ) -> Result<(), RecordEncodeError> {
        if !hunk::is_hunk_type(hunk_type) {
            return Err(RecordEncodeError::NotAHunkType { storage_type: hunk_type });
        }
        let entry = HunkRef {
            hunk_type,
            page_no: 0,
            packed: 0,
            count,
            words: payload.len() as u32,
        };
        self.upsert_explicit(hash, entry.type_code(), entry.to_words().to_vec(), false);
        self.hunks.push(HunkDraft {
            hash,
            hunk_type,
            count,
            payload,
        });
        Ok(())
    }

    /// Set an int array as a type-20 hunk: one word per value, `count` the
    /// number of values.
    pub fn set_int_array_hunk(&mut self, hash: u32, values: &[i32]) -> Result<(), RecordEncodeError> {
        let payload = values.iter().map(|value| *value as u32).collect();
        self.set_hunk_words(hash, 20, values.len() as u32, payload)
    }

    /// Set a real array as a type-21 hunk: each value two Fortran-ordered
    /// words, as an inline real array stores them ([`put_real_wide`]),
    /// `count` the number of values.
    pub fn set_real_array_hunk(&mut self, hash: u32, values: &[f64]) -> Result<(), RecordEncodeError> {
        let mut payload = vec![0u32; values.len() * 2];
        for (index, value) in values.iter().enumerate() {
            put_real_wide(&mut payload, index * 2, *value);
        }
        self.set_hunk_words(hash, 21, values.len() as u32, payload)
    }

    /// Set a text as a type-22 hunk: the bytes four to a word, NUL-padded,
    /// `count` the byte length — the packed text an inline `0x2800` entry
    /// carries after its count word, here with the count in the directory.
    /// ASCII only, like the inline text setters.
    pub fn set_text_hunk_ascii(&mut self, hash: u32, text: &str) -> Result<(), RecordEncodeError> {
        let mut data = ascii_text_data(text)?;
        let count = data.remove(0);
        self.set_hunk_words(hash, 22, count, data)
    }

    /// Set a string list as a type-23 hunk: the same buffer
    /// [`Self::set_explicit_text_list_ascii`] would store inline, with the
    /// element count `db_cxx_put_str_vector` reports for it
    /// (`1 + Σ(4 × words_i + 3)`) in the directory. An empty list unsets the
    /// attribute, as inline.
    pub fn set_text_list_hunk_ascii<I, S>(&mut self, hash: u32, items: I) -> Result<(), RecordEncodeError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let (data, count) = text_list_data_ascii(items)?;
        if data[0] == 0 {
            self.remove_explicit(hash);
            return Ok(());
        }
        self.set_hunk_words(hash, 23, count, data)
    }

    /// The hunk values set on this draft and not yet placed.
    pub fn pending_hunks(&self) -> &[HunkDraft] {
        &self.hunks
    }

    /// The hunk directory entry of the explicit attribute keyed by `hash`, or
    /// `None` when the element does not carry it or it is not a hunk. A
    /// pending hunk's entry reads address `(0, 0)`.
    pub fn hunk_ref(&self, hash: u32) -> Option<HunkRef> {
        self.explicit_words(hash)
            .and_then(|(type_code, data)| HunkRef::from_words(type_code, &data))
    }

    /// The explicit stream as words with each pending hunk's directory entry
    /// pointed at `addresses` — `(hash, page, packed)` per hunk, the placer's
    /// answer for where its first block landed. A hunk not in the list keeps
    /// `(0, 0)`.
    pub fn explicit_stream_words_with_hunks(&self, addresses: &[(u32, u32, u32)]) -> Vec<u32> {
        let mut fields = self.parse_explicit_fields();
        for (hash, page, packed) in addresses {
            let Some(field) = fields.iter_mut().find(|(field_hash, _, _)| field_hash == hash) else {
                continue;
            };
            if let Some(entry) = HunkRef::from_words(field.1, &field.2) {
                field.2 = HunkRef { page_no: *page, packed: *packed, ..entry }.to_words().to_vec();
            }
        }
        stream_words(&rebuild_explicit_stream(&fields))
    }

    /// Remove an explicit attribute. Returns whether it was there. A pending
    /// hunk value of the attribute goes with it.
    pub fn remove_explicit(&mut self, hash: u32) -> bool {
        self.hunks.retain(|hunk| hunk.hash != hash);
        let mut fields = self.parse_explicit_fields();
        let before = fields.len();
        fields.retain(|(field_hash, _, _)| *field_hash != hash);
        if fields.len() == before {
            return false;
        }
        self.explicit_stream = rebuild_explicit_stream(&fields);
        true
    }

    /// [`Self::set_explicit_words`] with the inline length limit enforced.
    pub(crate) fn set_explicit_words_checked(
        &mut self,
        hash: u32,
        type_code: u16,
        data: Vec<u32>,
    ) -> Result<(), RecordEncodeError> {
        if data.len() > MAX_INLINE_DATA_WORDS {
            return Err(RecordEncodeError::ExplicitTooLong {
                hash,
                words: data.len(),
                max: MAX_INLINE_DATA_WORDS,
            });
        }
        self.upsert_explicit(hash, type_code, data, false);
        Ok(())
    }

    /// Replace or add a field. Whatever value the attribute had is gone,
    /// a pending hunk included; [`Self::set_hunk_words`] re-registers its own.
    fn upsert_explicit(&mut self, hash: u32, type_code: u16, data: Vec<u32>, at_front: bool) {
        self.hunks.retain(|hunk| hunk.hash != hash);
        let mut fields = self.parse_explicit_fields();
        match fields.iter_mut().find(|(field_hash, _, _)| *field_hash == hash) {
            Some(field) => {
                field.1 = type_code;
                field.2 = data;
            }
            None if at_front => fields.insert(0, (hash, type_code, data)),
            None => fields.push((hash, type_code, data)),
        }
        self.explicit_stream = rebuild_explicit_stream(&fields);
    }

    /// The explicit stream as `(hash, type_code, data_words)` fields.
    fn parse_explicit_fields(&self) -> Vec<(u32, u16, Vec<u32>)> {
        let words = stream_words(&self.explicit_stream);
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
    /// the member block), both on the same page. Pending hunks are not placed
    /// by this single-page layout — their directory entries keep address
    /// `(0, 0)`; the session placer is what lays a record with hunks down.
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
        words[10] = wide_format_word(member_words, explicit_words);

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

/// Word 10 of a record header, the storage-format word, in the wide layout
/// every element uses: `0x20000000 | member_words | (explicit_stream_words <<
/// 14)`. `member_words` is `members × 2`, `explicit_stream_words` the explicit
/// stream's word count — the same fields whether the record is laid on one
/// page or split across several, so the placer and [`RecordDraft::encode`]
/// compute it here.
pub fn wide_format_word(member_words: usize, explicit_stream_words: usize) -> u32 {
    WIDE_LAYOUT_FLAG | member_words as u32 | ((explicit_stream_words as u32) << 14)
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
/// A3b rule for a plain `SAVEWORK` (extract flush gives the committed side:
/// [`MergeMode::ExtractFlush`] through [`three_way_with_mode`]).
///
/// The member merge is A3b.4's ([`merge_members`]): `latest`'s member list
/// with the writer's own adds, removes and moves — what [`member_list_diff`]
/// finds between `base` and `working` — replayed onto it in order, an added
/// or moved member landing right after the nearest member that precedes it
/// in the writer's list and still survives in the merged one, the "preceding
/// shared sibling" anchor `sub_5B20CA0` uses. Members the writer did not
/// touch keep the committed order, so two writers adding members to one
/// owner both land, and a member both sides moved sits where the writer put
/// it. This is [`three_way_with`] at [`MemberMerge::MemberByMember`]; a noun
/// whose member list is a reference list takes [`MemberMerge::WholeList`].
pub fn three_way(
    base: &RecordDraft,
    latest: &RecordDraft,
    working: &RecordDraft,
) -> Result<RecordDraft, RecordEncodeError> {
    three_way_with(base, latest, working, MemberMerge::MemberByMember)
}

/// How a three-way merge reconciles an element's member list — the two arms
/// of `core.dll`'s `sub_5B224B0` on the noun's member-list `kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MemberMerge {
    /// `kind != 2`: replay the writer's adds, removes and moves onto the
    /// committed list, each after its preceding shared sibling
    /// (`sub_5B20CA0`, [`merge_members`]). The default, and what every noun
    /// whose members are its own children gets.
    #[default]
    MemberByMember,
    /// `kind == 2`: the list is one attribute. If the writer changed it at
    /// all, the writer's whole list replaces the committed one
    /// (`sub_5B1F660`), so adds and removes the committed side made
    /// meanwhile are lost — the same "writer wins" a clash on any other
    /// attribute gets; if the writer left it alone, the committed list
    /// stands. This is the arm for nouns whose members are references to
    /// elements owned elsewhere — `GROU`, and `MDB`/`USLI` in the system
    /// database ([`crate::record::template::MemberListKind::Referenced`]).
    WholeList,
}

/// Which way a same-thing clash breaks in a three-way merge — the plain
/// `SAVEWORK` the corpus is written by, or the extract-flush variant.
///
/// Both compose changes to different attributes the same way; they differ only
/// when the writer and the committed session changed the *same* attribute,
/// owner, or whole member list relative to the base:
///
/// * [`PlainSaveWork`](MergeMode::PlainSaveWork) — `core.dll`'s `a6 = 0`
///   `SAVEWORK` — lets the writer's value win (`sub_5B0D950` writes the working
///   value with no base↔latest check; A3b.2).
/// * [`ExtractFlush`](MergeMode::ExtractFlush) — the fourth claims-manager slot
///   (`DXSWK2`, extract flush) — keeps the committed value: `sub_5B0D950`,
///   `sub_5B1F660` and `sub_5B211C0` each first compare base↔latest and, when
///   the committed session changed it too, skip the writer's change. Flushing
///   an extract back to the master must not clobber what the master gained
///   meanwhile, so on a clash the master (latest) wins.
///
/// No installed database is an extract, so this is exercised by constructed
/// three-view scenarios rather than against E3D-written extract data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MergeMode {
    /// The writer wins a clash (plain `SAVEWORK`).
    #[default]
    PlainSaveWork,
    /// The committed side wins a clash (extract flush, the fourth slot).
    ExtractFlush,
}

/// [`three_way`] with the member-list arm chosen by the caller, who knows the
/// noun's [`MemberListKind`](crate::record::template::MemberListKind). The
/// clash tie-break is plain `SAVEWORK` ([`MergeMode::PlainSaveWork`]).
pub fn three_way_with(
    base: &RecordDraft,
    latest: &RecordDraft,
    working: &RecordDraft,
    member_merge: MemberMerge,
) -> Result<RecordDraft, RecordEncodeError> {
    three_way_with_mode(base, latest, working, member_merge, MergeMode::PlainSaveWork)
}

/// Where a member element stands in the two committed views a merge consults
/// — what `sub_5B20CA0` looks up about a member the writer added (`st 1`)
/// before it lets the addition through.
///
/// The lookups are `4GTE` on the member itself in the base and in the latest
/// session, and its `OWNREF` (record header words 4/5) where it is indexed;
/// an element the session does not index (`4GTE` error 10) reads as owner
/// `(0, 0)`. Whether the addition goes through is
/// [`MemberStanding::blocks_addition`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemberStanding {
    /// The member's owner in the base session; `None` when the base session
    /// does not index the element.
    pub base_owner: Option<RefNo>,
    /// The member's owner in the latest session; `None` when the latest
    /// session does not index the element.
    pub latest_owner: Option<RefNo>,
}

impl MemberStanding {
    /// Whether `sub_5B20CA0` leaves the writer's addition of this member out
    /// of the merged list.
    ///
    /// Under plain `SAVEWORK` a member the base session indexes and the latest
    /// does not — the committed side deleted the element — is not added. Under
    /// extract flush (the fourth slot, `a13`) the test is the member's
    /// `OWNREF` in base against latest, an unindexed element reading as
    /// `(0, 0)`: a member the committed side re-parented (or deleted, or
    /// created) since the base is not added, so the list agrees with the
    /// owner `sub_5B211C0` keeps for it. Neither test looks at the writer's
    /// own view.
    pub fn blocks_addition(self, mode: MergeMode) -> bool {
        match mode {
            MergeMode::PlainSaveWork => self.base_owner.is_some() && self.latest_owner.is_none(),
            MergeMode::ExtractFlush => {
                let unindexed = RefNo::new(0, 0);
                self.base_owner.unwrap_or(unindexed) != self.latest_owner.unwrap_or(unindexed)
            }
        }
    }
}

/// [`three_way_with`] with the clash tie-break chosen too: [`MergeMode`] says
/// whether the writer or the committed side wins where both changed the same
/// attribute, owner or whole member list relative to `base`.
///
/// Nothing is known here about where the member elements themselves stand, so
/// every member the writer added is let through; [`three_way_with_standing`]
/// takes that knowledge.
pub fn three_way_with_mode(
    base: &RecordDraft,
    latest: &RecordDraft,
    working: &RecordDraft,
    member_merge: MemberMerge,
    mode: MergeMode,
) -> Result<RecordDraft, RecordEncodeError> {
    three_way_with_standing(base, latest, working, member_merge, mode, |_| None)
}

/// [`three_way_with_mode`] told, through `standing`, where each member element
/// stands in the base and latest sessions, so a member the writer added can be
/// gated the way `sub_5B20CA0` gates it before inserting (`st 1`): not added
/// when the committed side deleted the element, nor — under extract flush —
/// when it changed the element's owner ([`MemberStanding::blocks_addition`]).
/// `standing` answering `None` for a member means nothing is known about it
/// and it is added. Only the member-by-member arm consults it; a whole
/// reference list is one attribute and carries no such gate.
pub fn three_way_with_standing(
    base: &RecordDraft,
    latest: &RecordDraft,
    working: &RecordDraft,
    member_merge: MemberMerge,
    mode: MergeMode,
    mut standing: impl FnMut(RefNo) -> Option<MemberStanding>,
) -> Result<RecordDraft, RecordEncodeError> {
    let flush = mode == MergeMode::ExtractFlush;

    // Implicit region: latest's, with the words the writer changed overlaid.
    // The header words (0..11) are recomputed on encode, so only attribute
    // words matter. Under extract flush a word the committed session also
    // changed keeps the committed value.
    let mut implicit = latest.implicit.clone();
    let common = implicit
        .len()
        .min(base.implicit.len())
        .min(working.implicit.len());
    for index in HEADER_WORDS..common {
        let writer_changed = working.implicit[index] != base.implicit[index];
        let latest_changed = latest.implicit[index] != base.implicit[index];
        if writer_changed && !(flush && latest_changed) {
            implicit[index] = working.implicit[index];
        }
    }

    // Explicit attributes, keyed by hash: latest's, with the writer's changes
    // (added, replaced or removed relative to base) applied. Under extract
    // flush an attribute the committed session also changed keeps its value.
    let base_fields = base.parse_explicit_fields();
    let working_fields = working.parse_explicit_fields();
    let latest_fields = latest.parse_explicit_fields();
    let mut merged_fields = latest_fields.clone();
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
        if flush && value_of(&latest_fields, hash) != in_base {
            continue; // the committed session changed it too — it wins
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

    // Members (A3b.4): by default the committed list with the writer's own
    // adds, removes and moves replayed onto it, each anchored after its
    // preceding shared sibling — an addition first passing `sub_5B20CA0`'s
    // gate on where the member element itself stands; a reference list is one
    // attribute instead — the writer's whole list if the writer changed it,
    // else the committed one, and under extract flush the committed list when
    // it changed the list too.
    let members = match member_merge {
        MemberMerge::MemberByMember => {
            merge_members_with(&base.members, &latest.members, &working.members, |member| {
                standing(member).is_some_and(|standing| standing.blocks_addition(mode))
            })
        }
        MemberMerge::WholeList if working.members != base.members => {
            if flush && latest.members != base.members {
                latest.members.clone()
            } else {
                working.members.clone()
            }
        }
        MemberMerge::WholeList => latest.members.clone(),
    };

    // Owner: the writer's when it moved the element, unless extract flush and
    // the committed session moved it too (`sub_5B211C0`), where latest wins.
    let owner = if working.owner != base.owner && !(flush && latest.owner != base.owner) {
        working.owner
    } else {
        latest.owner
    };

    let mut merged = RecordDraft::new(working.refno, latest.noun_hash, owner, implicit, members, explicit_stream)?;
    // A pending hunk travels with the directory entry that names it: the
    // writer's where the merge took the writer's field, the committed side's
    // where that field stood (only a draft still in memory has one; a record
    // read from disk carries placed entries and no pending hunks).
    for (source, fields) in [(working, &working_fields), (latest, &latest_fields)] {
        for pending in &source.hunks {
            let taken = value_of(&merged_fields, pending.hash) == value_of(fields, pending.hash);
            if taken && !merged.hunks.iter().any(|hunk| hunk.hash == pending.hash) {
                merged.hunks.push(pending.clone());
            }
        }
    }
    Ok(merged)
}

/// One step of the ordered member-list diff `sub_5B1FF60` yields between a
/// writer's base list and its working list — what the writer did to one
/// member. `at` is the member's position in the working list, which is where
/// [`merge_members`] takes its anchor from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberEdit {
    /// `st 1`: in the working list, not in the base one — the writer added it.
    Added { member: RefNo, at: usize },
    /// `st 3`: in the base list, not in the working one — the writer removed
    /// it.
    Removed { member: RefNo },
    /// `st 2`: in both lists at different places — the writer moved it.
    Moved { member: RefNo, at: usize },
}

/// `sub_5B1FF60`: walk the writer's base and working member lists together
/// and say, member by member, what the writer added, removed and moved.
///
/// Two cursors advance in step while the lists agree. A base member missing
/// from the working list is a removal, a working member missing from the base
/// list an addition. When both lists hold the members under the cursors but
/// in different places, one of them moved, and `core.dll` picks which by
/// distance: the base member's displacement (its working position against
/// the base cursor) against the working member's (its base position against
/// the working cursor). The further-displaced one is the move — a base member
/// is deferred until the working cursor reaches it, a working member is
/// emitted at once — so `[p q r s]` → `[q r s p]` is one move of `p`, not
/// three. A move is emitted where the working cursor stands, so the edits
/// come out in the writer's order and a moved member's anchor is the member
/// before it in the writer's list.
pub fn member_list_diff(base: &[RefNo], working: &[RefNo]) -> Vec<MemberEdit> {
    let position_in = |list: &[RefNo], member: RefNo| list.iter().position(|m| *m == member);
    let mut edits = Vec::new();
    // The two scratch lists: moves already emitted from the working side (the
    // base cursor skips them) and moves deferred from the base side (emitted
    // when the working cursor reaches them).
    let mut emitted: Vec<RefNo> = Vec::new();
    let mut deferred: Vec<RefNo> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < base.len() && j < working.len() {
        let (a, b) = (base[i], working[j]);
        if a == b {
            i += 1;
            j += 1;
            continue;
        }
        let Some(a_in_working) = position_in(working, a) else {
            edits.push(MemberEdit::Removed { member: a });
            i += 1;
            continue;
        };
        let Some(b_in_base) = position_in(base, b) else {
            edits.push(MemberEdit::Added { member: b, at: j });
            j += 1;
            continue;
        };
        // Both present, in different places: one of them moved.
        if emitted.contains(&a) {
            i += 1;
            continue;
        }
        if let Some(slot) = deferred.iter().position(|m| *m == b) {
            deferred.swap_remove(slot);
            emitted.push(b);
            edits.push(MemberEdit::Moved { member: b, at: j });
            j += 1;
            continue;
        }
        if a_in_working.abs_diff(i) > b_in_base.abs_diff(j) {
            deferred.push(a);
            i += 1;
        } else {
            emitted.push(b);
            edits.push(MemberEdit::Moved { member: b, at: j });
            j += 1;
        }
    }
    // One side ran out. Base leftovers are removals unless they were placed
    // from the working side already; working leftovers are moves if they were
    // deferred (or are in the base list at all), additions otherwise.
    for &a in &base[i..] {
        if !emitted.contains(&a) && position_in(working, a).is_none() {
            edits.push(MemberEdit::Removed { member: a });
        }
    }
    for (offset, &b) in working[j..].iter().enumerate() {
        let at = j + offset;
        if deferred.contains(&b) || position_in(base, b).is_some() {
            edits.push(MemberEdit::Moved { member: b, at });
        } else {
            edits.push(MemberEdit::Added { member: b, at });
        }
    }
    edits
}

/// A3b.4's member merge, `sub_5B20CA0`: the committed (`latest`) member list
/// with the writer's edits — [`member_list_diff`] of `base` against
/// `working` — replayed onto it in order.
///
/// A removal takes the member out of the list if it is still there. An
/// addition goes in right after its anchor, the nearest member before it in
/// the writer's list that the merged list holds (the preceding shared
/// sibling), or at the front when none does; a member already present is
/// left alone. A move takes the member out of wherever the committed list
/// has it and puts it back after the same kind of anchor, so a member both
/// sides moved ends up where the writer put it, and a move whose anchor the
/// committed side removed falls back to the sibling before that. A moved
/// member the committed side took out of the list is not put back — the
/// writer only moved it, the other side removed it (inferred; `sub_5B21C10`
/// looks the member up in the merged table before it shifts anything).
/// Members the writer did not touch keep the committed order, so the
/// committed side's own adds, removes and moves survive.
///
/// Every addition is let through; [`merge_members_with`] takes the gate
/// `sub_5B20CA0` puts before one.
pub fn merge_members(base: &[RefNo], latest: &[RefNo], working: &[RefNo]) -> Vec<RefNo> {
    merge_members_with(base, latest, working, |_| false)
}

/// [`merge_members`] with `sub_5B20CA0`'s gate on each addition (`st 1`):
/// before a member the writer added is looked for in the merged list and
/// inserted, `blocked(member)` is asked, and a blocked member is left out —
/// `core.dll` asks it of where the member element itself stands in the base
/// and latest sessions ([`MemberStanding::blocks_addition`]). Removals and
/// moves carry no gate.
pub fn merge_members_with(
    base: &[RefNo],
    latest: &[RefNo],
    working: &[RefNo],
    mut blocked: impl FnMut(RefNo) -> bool,
) -> Vec<RefNo> {
    let mut merged = latest.to_vec();
    for edit in member_list_diff(base, working) {
        match edit {
            MemberEdit::Removed { member } => merged.retain(|m| *m != member),
            MemberEdit::Added { member, at } => {
                if blocked(member) {
                    continue;
                }
                if !merged.contains(&member) {
                    let slot = anchor_slot(&merged, working, at);
                    merged.insert(slot, member);
                }
            }
            MemberEdit::Moved { member, at } => {
                if let Some(current) = merged.iter().position(|m| *m == member) {
                    merged.remove(current);
                    let slot = anchor_slot(&merged, working, at);
                    merged.insert(slot, member);
                }
            }
        }
    }
    merged
}

/// Where the member at `at` in the writer's list lands in `merged`: right
/// after the nearest member before it in the writer's list that `merged`
/// holds, or at the front when none does.
fn anchor_slot(merged: &[RefNo], working: &[RefNo], at: usize) -> usize {
    working[..at]
        .iter()
        .rev()
        .find_map(|earlier| merged.iter().position(|m| m == earlier))
        .map_or(0, |anchor| anchor + 1)
}

/// An explicit text value's data words: a count word, then the bytes four to
/// a word, NUL-padded — the `[len][packed text]` a `0x3C00`/`0x2800` reader
/// decodes.
/// An explicit stream's bytes as its big-endian words. The stream is whole
/// words — every entry states its length in words — so the remainder is
/// empty; a trailing partial word, were there one, is left out the way
/// `chunks_exact(4)` left it out.
fn stream_words(stream: &[u8]) -> Vec<u32> {
    let (words, _rest) = stream.as_chunks::<4>();
    words.iter().map(|word| u32::from_be_bytes(*word)).collect()
}

fn ascii_text_data(text: &str) -> Result<Vec<u32>, RecordEncodeError> {
    if !text.is_ascii() {
        return Err(RecordEncodeError::ExplicitTextNotAscii {
            text: text.to_owned(),
        });
    }
    let bytes = text.as_bytes();
    let mut data = Vec::with_capacity(1 + bytes.len().div_ceil(4));
    data.push(bytes.len() as u32);
    for chunk in bytes.chunks(4) {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        data.push(u32::from_be_bytes(word));
    }
    Ok(data)
}

/// A string list's stored words and the element count `core.dll` reports for
/// it: `[n] { [i][words][bytes][packed text × words] }*` as `sub_5AD5060`
/// builds it — item index from 0, the item's packed word count, its byte
/// length, then the bytes four to a word NUL-padded — and the running total
/// `1 + Σ(4 × words_i + 3)` it hands `4SCEA` as the value's length (what a
/// type-23 directory entry's count word carries). ASCII only, like
/// `ascii_text_data`: under either header `FORMAT` the bytes store as they
/// are.
pub fn text_list_data_ascii<I, S>(items: I) -> Result<(Vec<u32>, u32), RecordEncodeError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut data = vec![0u32];
    let mut count: u32 = 1;
    let mut items_seen: u32 = 0;
    for (index, item) in items.into_iter().enumerate() {
        let text = item.as_ref();
        if !text.is_ascii() {
            return Err(RecordEncodeError::ExplicitTextNotAscii {
                text: text.to_owned(),
            });
        }
        let bytes = text.as_bytes();
        let words = bytes.len().div_ceil(4);
        data.push(index as u32);
        data.push(words as u32);
        data.push(bytes.len() as u32);
        for chunk in bytes.chunks(4) {
            let mut word = [0u8; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            data.push(u32::from_be_bytes(word));
        }
        count += 4 * words as u32 + 3;
        items_seen += 1;
    }
    data[0] = items_seen;
    Ok((data, count))
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

    /// The same three views under extract flush: disjoint changes still
    /// compose, but where the writer and the committed side changed the same
    /// attribute the committed value is kept — the opposite tie-break.
    #[test]
    fn extract_flush_keeps_the_committed_value_on_a_clash() {
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
        let mut latest_words = implicit(&[10, 20, 30]);
        latest_words[12] = 120;
        let latest = RecordDraft::new(refno, 0x861E0, owner, latest_words, vec![], explicit(0xAA, 2)).unwrap();
        let mut working_words = implicit(&[10, 20, 30]);
        working_words[12] = 121; // clashes with the committed word-12 change
        working_words[13] = 130; // disjoint
        let working = RecordDraft::new(refno, 0x861E0, owner, working_words, vec![], explicit(0xAA, 7)).unwrap();

        let merged = three_way_with_mode(&base, &latest, &working, MemberMerge::MemberByMember, MergeMode::ExtractFlush).unwrap();
        assert_eq!(merged.implicit()[12], 120, "both changed word 12: extract flush keeps the committed value");
        assert_eq!(merged.implicit()[13], 130, "the disjoint writer change still composes");
        let fields = merged.parse_explicit_fields();
        assert_eq!(
            fields.iter().find(|f| f.0 == 0xAA).map(|f| f.2[0]),
            Some(2),
            "both changed 0xAA: extract flush keeps the committed value"
        );
    }

    /// Extract flush on owner and on a whole (reference) member list: the
    /// committed side wins each when it changed it too, and the writer still
    /// wins where the committed side left it at the base.
    #[test]
    fn extract_flush_keeps_the_committed_owner_and_whole_list() {
        let refno = RefNo::new(1, 10);
        let m = |n| RefNo::new(1, n);
        let draft = |owner: RefNo, members: Vec<RefNo>| {
            RecordDraft::new(refno, 0x861E0, owner, implicit(&[0]), members, vec![]).unwrap()
        };
        let base = draft(m(1), vec![m(5), m(6)]);
        // Committed side moved the element to owner 2 and changed the list.
        let latest = draft(m(2), vec![m(5), m(6), m(7)]);
        // Writer moved it to owner 3 and changed the list to its own.
        let working = draft(m(3), vec![m(5), m(8)]);

        let flushed = three_way_with_mode(&base, &latest, &working, MemberMerge::WholeList, MergeMode::ExtractFlush).unwrap();
        assert_eq!(flushed.owner, m(2), "both moved owner: extract flush keeps the committed owner");
        assert_eq!(flushed.members(), &[m(5), m(6), m(7)], "both changed the reference list: committed list kept");

        // Plain SAVEWORK on the same views: the writer wins both.
        let plain = three_way_with(&base, &latest, &working, MemberMerge::WholeList).unwrap();
        assert_eq!(plain.owner, m(3), "plain SAVEWORK: the writer's owner wins");
        assert_eq!(plain.members(), &[m(5), m(8)], "plain SAVEWORK: the writer's whole list wins");

        // Committed side left the owner and list at the base: the writer wins
        // even under extract flush, because there is no clash.
        let latest_untouched = draft(m(1), vec![m(5), m(6)]);
        let flushed2 = three_way_with_mode(&base, &latest_untouched, &working, MemberMerge::WholeList, MergeMode::ExtractFlush).unwrap();
        assert_eq!(flushed2.owner, m(3), "no committed owner change: the writer's owner applies");
        assert_eq!(flushed2.members(), &[m(5), m(8)], "no committed list change: the writer's list applies");
    }

    /// Member three-way: both sides added a member to one owner, and the
    /// writer's lands after its preceding shared sibling; a member the writer
    /// removed is gone even though the committed side kept it.
    #[test]
    fn three_way_anchors_added_members_and_honours_removals() {
        let refno = RefNo::new(1, 10);
        let owner = RefNo::new(1, 1);
        let m = |n| RefNo::new(1, n);
        let draft = |members: Vec<RefNo>| {
            RecordDraft::new(refno, 0x861E0, owner, implicit(&[0]), members, vec![]).unwrap()
        };

        // base: a b c. Committed side added X after a and removed nothing.
        let base = draft(vec![m(1), m(2), m(3)]);
        let latest = draft(vec![m(1), m(90), m(2), m(3)]);
        // Writer added Y after b and removed c.
        let working = draft(vec![m(1), m(2), m(20), m(3)]);
        let mut working = working;
        working.remove_member(m(3));
        // working is now a b Y.

        let merged = three_way(&base, &latest, &working).unwrap();
        // X stays (committed), Y lands after b, c is gone (writer removed it).
        assert_eq!(
            merged.members(),
            &[m(1), m(90), m(2), m(20)],
            "committed insert kept, writer insert anchored after b, writer delete honoured"
        );
    }

    /// `sub_5B20CA0`'s gate on an addition (`st 1`), on where the member
    /// element stands in base and latest: plain `SAVEWORK` leaves out a member
    /// the committed side deleted; extract flush also leaves out one whose
    /// owner the committed side changed; a member nothing is known about, or
    /// one both sessions index under the same owner, goes in. Removals and
    /// moves are not gated.
    #[test]
    fn an_added_member_is_gated_on_where_it_stands_in_base_and_latest() {
        let m = |n| RefNo::new(1, n);
        let (kept, deleted, moved, fresh) = (m(20), m(21), m(22), m(23));
        let owner_p = m(1);
        let owner_q = m(2);
        let standing_of = |member: RefNo| -> Option<MemberStanding> {
            match member {
                // Indexed by both, same owner.
                x if x == kept => Some(MemberStanding { base_owner: Some(owner_p), latest_owner: Some(owner_p) }),
                // The committed side deleted it since the base.
                x if x == deleted => Some(MemberStanding { base_owner: Some(owner_p), latest_owner: None }),
                // The committed side moved it from P to Q since the base.
                x if x == moved => Some(MemberStanding { base_owner: Some(owner_p), latest_owner: Some(owner_q) }),
                // Nothing known (the writer's own new element, say).
                _ => None,
            }
        };

        // The gate itself.
        assert!(!standing_of(kept).unwrap().blocks_addition(MergeMode::PlainSaveWork));
        assert!(!standing_of(kept).unwrap().blocks_addition(MergeMode::ExtractFlush));
        assert!(standing_of(deleted).unwrap().blocks_addition(MergeMode::PlainSaveWork), "deleted under me: not added");
        assert!(standing_of(deleted).unwrap().blocks_addition(MergeMode::ExtractFlush), "deleted reads as owner (0,0) ≠ P");
        assert!(!standing_of(moved).unwrap().blocks_addition(MergeMode::PlainSaveWork), "plain SAVEWORK does not look at OWNREF");
        assert!(standing_of(moved).unwrap().blocks_addition(MergeMode::ExtractFlush), "extract flush: owner P ≠ Q, not added");
        // An element indexed by neither session (both read (0,0)) passes even
        // under extract flush.
        let unindexed = MemberStanding { base_owner: None, latest_owner: None };
        assert!(!unindexed.blocks_addition(MergeMode::ExtractFlush));
        // One the committed side created since the base (base (0,0), latest
        // owner Q) is blocked under extract flush, not under plain SAVEWORK.
        let created = MemberStanding { base_owner: None, latest_owner: Some(owner_q) };
        assert!(created.blocks_addition(MergeMode::ExtractFlush));
        assert!(!created.blocks_addition(MergeMode::PlainSaveWork));

        // Through the member merge: base `a b`, committed side unchanged; the
        // writer appends kept, deleted, moved, fresh and removes b.
        let (a, b) = (m(1), m(2));
        let base = [a, b];
        let latest = [a, b];
        let working = [a, kept, deleted, moved, fresh];
        let plain = merge_members_with(&base, &latest, &working, |member| {
            standing_of(member).is_some_and(|s| s.blocks_addition(MergeMode::PlainSaveWork))
        });
        assert_eq!(plain, vec![a, kept, moved, fresh], "plain: the deleted member stays out, b removed, the rest anchored in order");
        let flushed = merge_members_with(&base, &latest, &working, |member| {
            standing_of(member).is_some_and(|s| s.blocks_addition(MergeMode::ExtractFlush))
        });
        assert_eq!(flushed, vec![a, kept, fresh], "extract flush: the re-parented member stays out too");
        assert_eq!(merge_members(&base, &latest, &working), vec![a, kept, deleted, moved, fresh], "ungated: everything in");

        // A move of a member whose owner changed is not gated: base `a b c`,
        // writer moves c to the front, c re-parented in latest.
        let c = moved;
        let reordered = merge_members_with(&[a, b, c], &[a, b, c], &[c, a, b], |member| {
            assert_ne!(member, c, "moves are not asked about");
            standing_of(member).is_some_and(|s| s.blocks_addition(MergeMode::ExtractFlush))
        });
        assert_eq!(reordered, vec![c, a, b]);

        // Through the record-level three-way, the gate follows the mode.
        let refno = RefNo::new(1, 10);
        let draft = |members: Vec<RefNo>| {
            RecordDraft::new(refno, 0x861E0, owner_p, implicit(&[0]), members, vec![]).unwrap()
        };
        let (base_d, latest_d, working_d) = (draft(vec![a, b]), draft(vec![a, b]), draft(vec![a, b, deleted, moved]));
        let plain = three_way_with_standing(&base_d, &latest_d, &working_d, MemberMerge::MemberByMember, MergeMode::PlainSaveWork, standing_of).unwrap();
        assert_eq!(plain.members(), &[a, b, moved]);
        let flushed = three_way_with_standing(&base_d, &latest_d, &working_d, MemberMerge::MemberByMember, MergeMode::ExtractFlush, standing_of).unwrap();
        assert_eq!(flushed.members(), &[a, b]);
        // The whole-list arm is one attribute and has no gate.
        let whole = three_way_with_standing(&base_d, &latest_d, &working_d, MemberMerge::WholeList, MergeMode::ExtractFlush, standing_of).unwrap();
        assert_eq!(whole.members(), &[a, b, deleted, moved]);
        // Without standing, nothing is gated (the `three_way_with_mode` path).
        let ungated = three_way_with_mode(&base_d, &latest_d, &working_d, MemberMerge::MemberByMember, MergeMode::ExtractFlush).unwrap();
        assert_eq!(ungated.members(), &[a, b, deleted, moved]);
    }

    /// `sub_5B1FF60`'s classification: a member that changed place is one
    /// move of the further-displaced member, not a cascade; adds and removes
    /// come out where the cursors meet them.
    #[test]
    fn member_list_diff_classifies_adds_removes_and_moves() {
        let m = |n| RefNo::new(1, n);
        let (p, q, r, s, x) = (m(1), m(2), m(3), m(4), m(9));

        // p moved to the end: one move, deferred from the base side and
        // emitted where the working cursor reaches it.
        assert_eq!(
            member_list_diff(&[p, q, r, s], &[q, r, s, p]),
            vec![MemberEdit::Moved { member: p, at: 3 }]
        );
        // r moved to the front: one move, emitted at once from the working side.
        assert_eq!(
            member_list_diff(&[p, q, r, s], &[r, p, q, s]),
            vec![MemberEdit::Moved { member: r, at: 0 }]
        );
        // Reversed ends: r comes forward, p goes back, q stays.
        assert_eq!(
            member_list_diff(&[p, q, r], &[r, q, p]),
            vec![MemberEdit::Moved { member: r, at: 0 }, MemberEdit::Moved { member: p, at: 2 }]
        );
        // q replaced by x in place: a removal then an addition at q's slot.
        assert_eq!(
            member_list_diff(&[p, q, r], &[p, x, r]),
            vec![MemberEdit::Removed { member: q }, MemberEdit::Added { member: x, at: 1 }]
        );
        // Unchanged lists have nothing to say.
        assert!(member_list_diff(&[p, q], &[p, q]).is_empty());
    }

    /// Member three-way, the move case (`st 2`): the writer's moved member
    /// lands right after its anchor in the committed list, and the committed
    /// side's own insert keeps its place relative to the members around it.
    #[test]
    fn three_way_reorders_a_moved_member_after_its_anchor() {
        let refno = RefNo::new(1, 10);
        let owner = RefNo::new(1, 1);
        let m = |n| RefNo::new(1, n);
        let (a, b, c, d, x) = (m(1), m(2), m(3), m(4), m(90));
        let draft = |members: Vec<RefNo>| {
            RecordDraft::new(refno, 0x861E0, owner, implicit(&[0]), members, vec![]).unwrap()
        };

        // base: a b c d. Committed side inserted X after a. Writer moved d
        // to sit right after a.
        let base = draft(vec![a, b, c, d]);
        let latest = draft(vec![a, x, b, c, d]);
        let mut working = base.clone();
        assert!(working.move_member_after(d, Some(a)));
        assert_eq!(working.members(), &[a, d, b, c]);

        let merged = three_way(&base, &latest, &working).unwrap();
        assert_eq!(merged.members(), &[a, d, x, b, c], "d follows its anchor a; X keeps its committed place");
    }

    /// Both sides moved the same member: the writer's placement wins, the
    /// A3b rule for a plain SAVEWORK.
    #[test]
    fn three_way_lets_the_writer_place_a_member_both_sides_moved() {
        let refno = RefNo::new(1, 10);
        let owner = RefNo::new(1, 1);
        let m = |n| RefNo::new(1, n);
        let (a, b, c, d) = (m(1), m(2), m(3), m(4));
        let draft = |members: Vec<RefNo>| {
            RecordDraft::new(refno, 0x861E0, owner, implicit(&[0]), members, vec![]).unwrap()
        };

        // base: a b c d. Committed side moved d to the front; writer moved d after b.
        let base = draft(vec![a, b, c, d]);
        let latest = draft(vec![d, a, b, c]);
        let working = draft(vec![a, b, d, c]);

        let merged = three_way(&base, &latest, &working).unwrap();
        assert_eq!(merged.members(), &[a, b, d, c], "the writer's placement of d wins");
    }

    /// A move whose anchor the committed side removed falls back to the
    /// sibling before it; a moved member the committed side took out of the
    /// list is not put back.
    #[test]
    fn three_way_move_falls_back_to_the_previous_shared_sibling() {
        let refno = RefNo::new(1, 10);
        let owner = RefNo::new(1, 1);
        let m = |n| RefNo::new(1, n);
        let (a, b, c, d) = (m(1), m(2), m(3), m(4));
        let draft = |members: Vec<RefNo>| {
            RecordDraft::new(refno, 0x861E0, owner, implicit(&[0]), members, vec![]).unwrap()
        };

        // Writer moved a after c; committed side removed c → a lands after b.
        let base = draft(vec![a, b, c, d]);
        let latest = draft(vec![a, b, d]);
        let working = draft(vec![b, c, a, d]);
        let merged = three_way(&base, &latest, &working).unwrap();
        assert_eq!(merged.members(), &[b, a, d], "anchor c is gone, a falls back behind b");

        // Writer moved c to the front; committed side removed c → it stays gone.
        let base = draft(vec![a, b, c]);
        let latest = draft(vec![a, b]);
        let working = draft(vec![c, a, b]);
        let merged = three_way(&base, &latest, &working).unwrap();
        assert_eq!(merged.members(), &[a, b], "a move does not resurrect a member the other side removed");
    }

    /// The `kind == 2` arm: a reference list is one attribute — the writer's
    /// whole list if the writer touched it, the committed list otherwise —
    /// where the member-by-member arm would compose both sides.
    #[test]
    fn three_way_whole_list_takes_the_writers_list_when_it_changed() {
        let refno = RefNo::new(1, 10);
        let owner = RefNo::new(1, 1);
        let m = |n| RefNo::new(1, n);
        let (a, b, c, d) = (m(1), m(2), m(3), m(4));
        let draft = |members: Vec<RefNo>| {
            RecordDraft::new(refno, 0x861E0, owner, implicit(&[0]), members, vec![]).unwrap()
        };

        // base a b; committed side added c; writer added d.
        let base = draft(vec![a, b]);
        let latest = draft(vec![a, b, c]);
        let working = draft(vec![a, b, d]);
        let whole = three_way_with(&base, &latest, &working, MemberMerge::WholeList).unwrap();
        assert_eq!(whole.members(), &[a, b, d], "the writer's list replaces the committed one; c is lost");
        let by_member = three_way_with(&base, &latest, &working, MemberMerge::MemberByMember).unwrap();
        assert_eq!(
            by_member.members(),
            &[a, b, d, c],
            "member by member, both additions land — the writer's right after its anchor b"
        );

        // The writer left the list alone: the committed list stands.
        let untouched = three_way_with(&base, &latest, &base, MemberMerge::WholeList).unwrap();
        assert_eq!(untouched.members(), &[a, b, c], "an unchanged list keeps the committed side's additions");
    }

    /// `move_member_after` is the REORDER edit: after an anchor, to the
    /// front, and a no-op when a member is missing or the anchor is itself.
    #[test]
    fn move_member_after_reorders_in_place() {
        let m = |n| RefNo::new(1, n);
        let (a, b, c) = (m(1), m(2), m(3));
        let mut draft = RecordDraft::new(m(10), 0x861E0, m(0), implicit(&[0]), vec![a, b, c], vec![]).unwrap();

        assert!(draft.move_member_after(a, Some(c)));
        assert_eq!(draft.members(), &[b, c, a]);
        assert!(draft.move_member_after(c, None));
        assert_eq!(draft.members(), &[c, b, a]);
        assert!(!draft.move_member_after(m(99), Some(a)), "missing member");
        assert!(!draft.move_member_after(a, Some(m(99))), "missing anchor");
        assert!(!draft.move_member_after(a, Some(a)), "a member cannot anchor itself");
        assert_eq!(draft.members(), &[c, b, a], "a refused move changes nothing");
    }

    /// `content_eq` sees past the header and explicit-stream padding, and
    /// catches a real difference.
    #[test]
    fn content_eq_normalises_header_and_padding() {
        let refno = RefNo::new(1, 10);
        let owner = RefNo::new(1, 1);
        let mut stream = Vec::new();
        stream.extend_from_slice(&0xAAu32.to_be_bytes());
        stream.extend_from_slice(&((0x0C00u32 << 16) | 1).to_be_bytes());
        stream.extend_from_slice(&7u32.to_be_bytes());
        let a = RecordDraft::new(refno, 0x861E0, owner, implicit(&[1, 2, 3]), vec![RefNo::new(1, 5)], stream.clone()).unwrap();

        // Same content, read back from a's own bytes: equal.
        let placed = a.encode(9, 5, PageId::PRIMARY);
        let round_trip = RecordDraft::from_record_bytes(&placed.bytes).unwrap();
        assert!(a.content_eq(&round_trip), "a record equals its own round trip");

        // A different attribute value is not equal.
        let mut other_stream = stream.clone();
        other_stream[8..12].copy_from_slice(&8u32.to_be_bytes());
        let different = RecordDraft::new(refno, 0x861E0, owner, implicit(&[1, 2, 3]), vec![RefNo::new(1, 5)], other_stream).unwrap();
        assert!(!a.content_eq(&different), "a changed attribute is not equal");
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

    /// The type-19 buffer `sub_5AD5060` builds — `[n] {[i][words][bytes]
    /// [packed]}*` — and the running length it reports, `1 + Σ(4·words + 3)`.
    #[test]
    fn a_text_list_encodes_as_db_cxx_put_str_vector_builds_it() {
        let (words, count) = text_list_data_ascii(["AB", "CDEFG"]).unwrap();
        assert_eq!(
            words,
            vec![2, 0, 1, 2, 0x4142_0000, 1, 2, 5, 0x4344_4546, 0x4700_0000]
        );
        assert_eq!(count, 1 + (4 + 3) + (8 + 3));
        assert_eq!(
            crate::record::hunk::decode_text_list(&words, crate::record::text::TextFormat::Utf8),
            Some(vec!["AB".to_string(), "CDEFG".to_string()])
        );
        // An empty string is an item of no words.
        let (words, count) = text_list_data_ascii([""]).unwrap();
        assert_eq!(words, vec![1, 0, 0, 0]);
        assert_eq!(count, 4);
        assert!(text_list_data_ascii(["é"]).is_err());
    }

    /// An inline string list goes under `19 << 10` with the buffer's own length
    /// and no count word; an empty list unsets the attribute.
    #[test]
    fn an_inline_text_list_is_a_type_19_entry() {
        let refno = RefNo::new(1, 10);
        let mut draft = RecordDraft::new(refno, 0x861E0, RefNo::new(1, 1), implicit(&[0]), vec![], vec![]).unwrap();
        draft.set_explicit_text_list_ascii(0x1234, ["AB", "CDEFG"]).unwrap();
        let (type_code, data) = draft.explicit_words(0x1234).expect("the list is there");
        assert_eq!(type_code, 19 << 10);
        assert_eq!(data.len(), 10);
        assert_eq!(draft.explicit_stream_words()[1], (19 << 26) | 10, "header: type 19, ten words, no count word");
        assert!(draft.pending_hunks().is_empty());

        draft.set_explicit_text_list_ascii(0x1234, Vec::<&str>::new()).unwrap();
        assert_eq!(draft.explicit_words(0x1234), None, "an empty list deletes the entry");
    }

    /// A hunk value is a seven-word directory entry with address `(0, 0)` and
    /// a pending payload; the placer's addresses patch into the stream; an
    /// inline value set over it, or its removal, drops the pending hunk.
    #[test]
    fn a_hunk_is_a_directory_entry_and_a_pending_payload_until_placed() {
        let refno = RefNo::new(1, 10);
        let mut draft = RecordDraft::new(refno, 0x861E0, RefNo::new(1, 1), implicit(&[0]), vec![], vec![]).unwrap();
        draft.set_int_array_hunk(0xA1, &[1, -2, 3]).unwrap();
        draft.set_real_array_hunk(0xA2, &[1.5, 2.5]).unwrap();
        draft.set_text_hunk_ascii(0xA3, "HELLO").unwrap();
        draft.set_text_list_hunk_ascii(0xA4, ["AB", "CDEFG"]).unwrap();
        assert_eq!(draft.set_hunk_words(0xA5, 7, 1, vec![1]).unwrap_err().to_string(), RecordEncodeError::NotAHunkType { storage_type: 7 }.to_string());

        let ints = draft.hunk_ref(0xA1).expect("a directory entry");
        assert_eq!((ints.hunk_type, ints.page_no, ints.packed, ints.count, ints.words), (20, 0, 0, 3, 3));
        let reals = draft.hunk_ref(0xA2).unwrap();
        assert_eq!((reals.hunk_type, reals.count, reals.words), (21, 2, 4));
        let text = draft.hunk_ref(0xA3).unwrap();
        assert_eq!((text.hunk_type, text.count, text.words), (22, 5, 2));
        let list = draft.hunk_ref(0xA4).unwrap();
        assert_eq!((list.hunk_type, list.count, list.words), (23, 19, 10));
        assert_eq!(draft.explicit_words(0xA4).unwrap().1, vec![1, 0, 0, 19, 10]);
        assert_eq!(draft.explicit_stream_words()[1], (20 << 26) | 5, "header: type 20, five words");

        let pending = draft.pending_hunks();
        assert_eq!(pending.len(), 4);
        assert_eq!(pending[0].payload, vec![1, 0xFFFF_FFFE, 3]);
        let mut reals_payload = vec![0u32; 4];
        put_real_wide(&mut reals_payload, 0, 1.5);
        put_real_wide(&mut reals_payload, 2, 2.5);
        assert_eq!(pending[1].payload, reals_payload);
        assert_eq!(pending[2].payload, vec![0x4845_4C4C, 0x4F00_0000]);
        assert_eq!(pending[3].payload.len(), 10);

        // The placer's answer lands in the directory; a hunk it did not place keeps (0, 0).
        let words = draft.explicit_stream_words_with_hunks(&[(0xA1, 700, (9 << 13) | 1), (0xA3, 701, (1 << 13) | 1)]);
        let mut patched = RecordDraft::new(refno, 0x861E0, RefNo::new(1, 1), implicit(&[0]), vec![], vec![]).unwrap();
        patched.explicit_stream = words.iter().flat_map(|w| w.to_be_bytes()).collect();
        assert_eq!((patched.hunk_ref(0xA1).unwrap().page_no, patched.hunk_ref(0xA1).unwrap().packed), (700, (9 << 13) | 1));
        assert_eq!((patched.hunk_ref(0xA3).unwrap().page_no, patched.hunk_ref(0xA3).unwrap().packed), (701, (1 << 13) | 1));
        assert_eq!((patched.hunk_ref(0xA2).unwrap().page_no, patched.hunk_ref(0xA2).unwrap().packed), (0, 0));
        assert_eq!(patched.hunk_ref(0xA1).unwrap().count, 3, "count and words are untouched by the patch");

        // An inline value over a hunk attribute drops the pending payload; so does removal.
        draft.set_explicit_words(0xA1, 0x1C00, vec![1, 5]);
        assert!(draft.hunk_ref(0xA1).is_none());
        assert_eq!(draft.pending_hunks().len(), 3);
        assert!(draft.remove_explicit(0xA2));
        assert_eq!(draft.pending_hunks().iter().map(|h| h.hash).collect::<Vec<_>>(), vec![0xA3, 0xA4]);
        // Setting a hunk again replaces the earlier pending one, not adds to it.
        draft.set_text_hunk_ascii(0xA3, "BYE").unwrap();
        assert_eq!(draft.pending_hunks().len(), 2);
        assert_eq!(draft.pending_hunks()[1].hash, 0xA3);
        assert_eq!(draft.hunk_ref(0xA3).unwrap().count, 3);

        // A placed entry read back has no pending hunk and is re-emitted as it stands.
        let placed = patched.encode(9, 5, PageId::PRIMARY);
        let redraft = RecordDraft::from_record_bytes(&placed.bytes).unwrap();
        assert!(redraft.pending_hunks().is_empty());
        assert_eq!(redraft.hunk_ref(0xA1).unwrap().page_no, 700);
        assert!(patched.content_eq(&redraft));
    }

    /// A three-way merge carries the writer's pending hunk with the field it
    /// took, and not when the committed side's value stands.
    #[test]
    fn three_way_carries_a_pending_hunk_with_its_field() {
        let refno = RefNo::new(1, 10);
        let owner = RefNo::new(1, 1);
        let base = RecordDraft::new(refno, 0x861E0, owner, implicit(&[10]), vec![], vec![]).unwrap();
        let latest = base.clone();
        let mut working = base.clone();
        working.set_int_array_hunk(0xA1, &[4, 5, 6]).unwrap();

        let merged = three_way(&base, &latest, &working).unwrap();
        assert_eq!(merged.pending_hunks(), working.pending_hunks());
        assert_eq!(merged.hunk_ref(0xA1).unwrap().words, 3);
        assert!(!merged.content_eq(&latest), "a new hunk value is a change");
        assert!(merged.content_eq(&working));

        // Under extract flush, a clash on the attribute keeps the committed side and drops the writer's hunk.
        let mut latest_changed = base.clone();
        latest_changed.set_explicit_words(0xA1, 0x1C00, vec![1, 9]);
        let merged = three_way_with_mode(&base, &latest_changed, &working, MemberMerge::MemberByMember, MergeMode::ExtractFlush).unwrap();
        assert!(merged.pending_hunks().is_empty());
        assert_eq!(merged.explicit_words(0xA1), Some((0x1C00, vec![1, 9])));
    }
}
