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

    /// Whether two records carry the same content — noun, owner, members, the
    /// implicit region past its header (the header words are addresses and the
    /// format word, recomputed on encode) and the explicit attributes as
    /// fields (so a difference in stream padding does not count).
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
/// The member merge is A3b.4's ([`merge_members`]): `latest`'s member list
/// with the writer's own adds, removes and moves — what [`member_list_diff`]
/// finds between `base` and `working` — replayed onto it in order, an added
/// or moved member landing right after the nearest member that precedes it
/// in the writer's list and still survives in the merged one, the "preceding
/// shared sibling" anchor `sub_5B20CA0` uses. Members the writer did not
/// touch keep the committed order, so two writers adding members to one
/// owner both land, and a member both sides moved sits where the writer put
/// it.
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

    // Members (A3b.4): the committed list with the writer's own adds, removes
    // and moves replayed onto it, each anchored after its preceding shared
    // sibling.
    let members = merge_members(&base.members, &latest.members, &working.members);

    let owner = if working.owner != base.owner {
        working.owner
    } else {
        latest.owner
    };

    RecordDraft::new(working.refno, latest.noun_hash, owner, implicit, members, explicit_stream)
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
pub fn merge_members(base: &[RefNo], latest: &[RefNo], working: &[RefNo]) -> Vec<RefNo> {
    let mut merged = latest.to_vec();
    for edit in member_list_diff(base, working) {
        match edit {
            MemberEdit::Removed { member } => merged.retain(|m| *m != member),
            MemberEdit::Added { member, at } => {
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
}
