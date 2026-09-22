//! Values stored out of the record, in a *hunk* (storage types 20–23), and the
//! string-list layout of storage type 19 that type 23 moves out there.
//!
//! `4SCEA` (`sub_5AB7B90`) writes most explicit values into the record's own
//! attribute stream. Four array types have an out-of-record form it switches
//! to for a large value — 7 → 20 (ints), 6 → 21 (reals), 10 → 22 (text) and
//! 19 → 23 (string list); `4GCEA` (`sub_5AB5BC0`, line 44) treats each pair as
//! one type. The stream then carries a seven-word *directory entry* in the
//! attribute's place and the value itself lives in a chain of *hunk blocks*
//! wherever the writer's output page had room:
//!
//! ```text
//! directory entry (in the explicit stream)
//!   [hash][type << 26 | 5][1][hunk_page][hunk_packed][count][words]
//!
//! hunk block (kind 3; `sub_5AEC110` with a3 == 3, header 7 words)
//!   [3:u16][words:u16][refno.word0][refno.word1][cont_pgno][cont_packed][hash][type] payload…
//! ```
//!
//! The entry's five data words are the shape of a one-element type-11 array
//! (`1.0 / dbl_6453DF8[11]` = 4 words, so its count word reads 1): the hunk's
//! `(page, packed)` address in the encoding an index leaf value uses, then the
//! value's element count (`*a5` as the caller gave it) and its length in words.
//! The reader (`sub_5AB4CF0`, its `type in 20..=23` arm) takes the length from
//! word 6 for type 23 and from `count / dbl_6453DF8[type]` otherwise, then
//! `sub_5AEA500(kind 3)` walks the chain — kind checked, `words - 7` payload
//! words per block, following `cont` while words are still owed — and hands
//! the payload back as if it had been inline. The block header names the
//! element the value belongs to (words 1/2, as a member or explicit block does),
//! the attribute and the hunk type (words 5/6). A block fills to the page
//! boundary and states where the rest continues, exactly as a member or
//! explicit block does (`sub_5AE9DF0` with a minimum of nine words: header
//! plus two). A value of no words has address `(0, 0)` and no block.
//!
//! A hunk block is **not** a tail block of the record: [`super::block::block_at`]
//! does not know kind 3, so a record walk that runs into one stops — which is
//! right, a hunk is reached only through its directory entry.
//!
//! **Storage type 19** is a list of strings, the on-disk form of the
//! `std::vector<std::string>` that `db_cxx_put_str_vector` (routine 640,
//! `sub_5AAA8B0` → `sub_5AD5060`) takes:
//!
//! ```text
//! [item_count] { [index][words][bytes][packed text × words] }*
//! ```
//!
//! — a 0-based item index, the item's packed word count and its byte length
//! ahead of each string, the text four characters to a word, NUL-padded. The
//! whole buffer is the explicit entry's data (`4SCEA`'s `type == 19` arm writes
//! it verbatim under a header of `words | 19 << 26`, with no separate count
//! word; the reader's `case 19` copies `words` words back from the entry's
//! third word) and, as type 23, the hunk's payload. The element count
//! `sub_5AD5060` reports for it is `1 + Σ(4 × words_i + 3)`, which is what a
//! type-23 directory entry carries in its count word.
//!
//! No installed E3D 3.1 database sets a type-19 value, and the one hunk E3D
//! itself wrote in the corpus is `DBMDAT 24384/26333`'s `DBMVAL` in
//! `ams8000_0001` — a type-20 array of 5 709 ints whose directory entry, block
//! header and continuation match what this crate writes word for word
//! (`docs/plans/2026-09-21-write-path-next-steps.md` §7, S4) — so everything
//! here is read out of `core.dll` and exercised against that hunk and against
//! records this crate writes.

use crate::page::cache::PageCache;
use crate::page::PageId;
use crate::record::block::Address;
use crate::record::explicit::{ExplicitAttribute, ExplicitValue};
use crate::record::text::{decode_as, TextFormat};
use crate::refno::RefNo;

/// The block kind of a hunk block, in the high half of its first word.
pub const HUNK_KIND: u16 = 3;

/// Bytes of a hunk block's header: the five tail-block words then the
/// attribute hash and the hunk type.
pub const HUNK_HEADER_BYTES: usize = 28;

/// Words of a hunk block's header.
pub const HUNK_HEADER_WORDS: usize = HUNK_HEADER_BYTES / 4;

/// The smallest hunk block `sub_5AEC110` lays down: its header and two
/// payload words (`sub_5AE9DF0`'s minimum, its `a6 = 9`).
pub const HUNK_MIN_WORDS: usize = HUNK_HEADER_WORDS + 2;

/// Data words of a hunk directory entry: `[1][page][packed][count][words]`.
pub const DIRECTORY_WORDS: usize = 5;

/// The first data word of a directory entry — one element of type 11, four
/// words wide.
pub const DIRECTORY_COUNT_WORD: u32 = 1;

/// Storage type of a string list stored inline.
pub const TEXT_LIST_TYPE: u16 = 19;

/// The hunk form of an inline storage type, `4SCEA`'s upgrade table:
/// 6 → 21, 7 → 20, 10 → 22, 19 → 23. `None` for a type that has no hunk form.
pub fn hunk_type_of(inline_type: u16) -> Option<u16> {
    Some(match inline_type {
        6 => 21,
        7 => 20,
        10 => 22,
        19 => 23,
        _ => return None,
    })
}

/// The inline storage type a hunk type is the out-of-record form of — the
/// inverse of [`hunk_type_of`].
pub fn inline_type_of(hunk_type: u16) -> Option<u16> {
    Some(match hunk_type {
        20 => 7,
        21 => 6,
        22 => 10,
        23 => 19,
        _ => return None,
    })
}

/// Whether `storage_type` is one of the four hunk types.
pub const fn is_hunk_type(storage_type: u16) -> bool {
    matches!(storage_type, 20..=23)
}

/// Words a hunk payload of `count` elements of `hunk_type` occupies, by
/// `core.dll`'s width table: one per int (20), two per real (21), a quarter
/// per character (22, rounded up). Type 23 states its own length, so `None`.
pub fn payload_words(hunk_type: u16, count: u32) -> Option<u32> {
    Some(match hunk_type {
        20 => count,
        21 => count * 2,
        22 => count.div_ceil(4),
        _ => return None,
    })
}

/// The value `4SCEA` moves a large array to when the hunk machinery is armed:
/// `0x2000` words (1024 when the descriptor's logical count is 0, or when
/// bit 0 of its flags asks for the lower bound). Whether `core.dll` actually
/// upgrades depends on its runtime state (`DB_Attribute::initialised` on the
/// hunk registry), so a writer chooses inline or hunk explicitly; this is the
/// size past which `core.dll` itself would stop writing inline.
pub const HUNK_THRESHOLD_WORDS: usize = 0x2000;

/// A hunk directory entry as the explicit stream carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HunkRef {
    /// 20, 21, 22 or 23.
    pub hunk_type: u16,
    /// The first block's page, or 0 with `packed` 0 when the value has no
    /// words and no block.
    pub page_no: u32,
    /// `ext | word_offset << 13`, as an index leaf value.
    pub packed: u32,
    /// The value's element count, as the writer stated it.
    pub count: u32,
    /// The payload's length in words.
    pub words: u32,
}

impl HunkRef {
    /// Read a directory entry out of an explicit field's stored type code and
    /// data words. `None` unless the type is 20..=23 and the data is the
    /// five-word directory shape.
    pub fn from_words(type_code: u16, data: &[u32]) -> Option<Self> {
        let hunk_type = type_code >> 10;
        if !is_hunk_type(hunk_type) || data.len() != DIRECTORY_WORDS {
            return None;
        }
        Some(Self {
            hunk_type,
            page_no: data[1],
            packed: data[2],
            count: data[3],
            words: data[4],
        })
    }

    /// The directory entry of a parsed explicit attribute, which the stream
    /// decoder hands over as five raw words.
    pub fn from_explicit(attribute: &ExplicitAttribute) -> Option<Self> {
        match &attribute.value {
            ExplicitValue::RawWords(words) => Self::from_words(attribute.type_code, words),
            _ => None,
        }
    }

    /// The five data words of the entry.
    pub fn to_words(&self) -> [u32; DIRECTORY_WORDS] {
        [DIRECTORY_COUNT_WORD, self.page_no, self.packed, self.count, self.words]
    }

    /// The explicit-stream type code the entry is stored under.
    pub fn type_code(&self) -> u16 {
        self.hunk_type << 10
    }

    /// Where the first block is, or `None` for a value of no words.
    pub fn address(&self) -> Option<Address> {
        let mut bytes = [0u8; 8];
        bytes[..4].copy_from_slice(&self.page_no.to_be_bytes());
        bytes[4..].copy_from_slice(&self.packed.to_be_bytes());
        Address::at(&bytes, 0)
    }
}

/// One hunk block's header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HunkBlock {
    /// Bytes of the whole block, header included.
    pub total_bytes: usize,
    pub continues_at: Option<Address>,
    /// The attribute the value belongs to.
    pub hash: u32,
    /// 20..=23.
    pub hunk_type: u16,
}

impl HunkBlock {
    /// Byte range of the payload within the page the block sits on.
    pub fn payload(&self, at: usize) -> std::ops::Range<usize> {
        at + HUNK_HEADER_BYTES..at + self.total_bytes
    }
}

/// The hunk block at `at`, if one belonging to `owner` starts there — the
/// kind-3 counterpart of [`super::block::block_at`], and the only place a hunk
/// header is read.
pub fn hunk_block_at(data: &[u8], at: usize, owner: RefNo) -> Option<HunkBlock> {
    let header = data.get(at..at + HUNK_HEADER_BYTES)?;
    if u16::from_be_bytes(header[0..2].try_into().ok()?) != HUNK_KIND {
        return None;
    }
    let words = u16::from_be_bytes(header[2..4].try_into().ok()?) as usize;
    if words < HUNK_HEADER_WORDS {
        return None;
    }
    if RefNo::new(be32(header, 4)?, be32(header, 8)?) != owner {
        return None;
    }
    Some(HunkBlock {
        total_bytes: words * 4,
        continues_at: Address::at(header, 12),
        hash: be32(header, 20)?,
        hunk_type: be32(header, 24)? as u16,
    })
}

/// Why a hunk could not be read.
#[derive(Debug, Clone, thiserror::Error)]
pub enum HunkError {
    #[error("{0}")]
    Page(#[from] super::RecordError),
    #[error("{refno}: the hunk is said to be at {page} byte {at}, and no hunk block of this element is there")]
    Missing { refno: RefNo, page: PageId, at: usize },
    #[error("{refno}: the hunk block at {page} byte {at} declares {bytes} bytes, running {over} past the end of the page")]
    OverflowsPage {
        refno: RefNo,
        page: PageId,
        at: usize,
        bytes: usize,
        over: usize,
    },
    #[error("{refno}: the hunk chain comes back to {page} byte {at}")]
    Cycle { refno: RefNo, page: PageId, at: usize },
    #[error("{refno}: the hunk directory promises {wanted} words and the chain ends after {got}")]
    Truncated { refno: RefNo, wanted: u32, got: usize },
}

/// Read the words a directory entry points at, following the chain the way
/// `sub_5AEA500` does: every block's payload up to the `words` still owed,
/// stopping when they are all in hand. An entry of no words is an empty value
/// and no page is read.
pub fn read_hunk(
    pages: &mut PageCache,
    ext: u16,
    refno: RefNo,
    hunk: &HunkRef,
) -> Result<Vec<u32>, HunkError> {
    let wanted = hunk.words as usize;
    let mut out = Vec::with_capacity(wanted);
    if wanted == 0 {
        return Ok(out);
    }
    let Some(from) = hunk.address() else {
        return Err(HunkError::Truncated { refno, wanted: hunk.words, got: 0 });
    };
    let mut page_id = PageId::new(ext, from.page_no);
    let mut at = from.byte_offset();
    let mut visited = std::collections::HashSet::new();
    loop {
        if !visited.insert((page_id, at)) {
            return Err(HunkError::Cycle { refno, page: page_id, at });
        }
        let page = super::page_of(pages, page_id)?;
        let Some(block) = hunk_block_at(page, at, refno) else {
            return Err(HunkError::Missing { refno, page: page_id, at });
        };
        let end = at + block.total_bytes;
        if end > page.len() {
            return Err(HunkError::OverflowsPage {
                refno,
                page: page_id,
                at,
                bytes: block.total_bytes,
                over: end - page.len(),
            });
        }
        let payload = &page[block.payload(at)];
        for word in payload.as_chunks::<4>().0 {
            if out.len() == wanted {
                break;
            }
            out.push(u32::from_be_bytes(*word));
        }
        if out.len() == wanted {
            return Ok(out);
        }
        let Some(next) = block.continues_at else {
            return Err(HunkError::Truncated { refno, wanted: hunk.words, got: out.len() });
        };
        page_id = PageId::new(ext, next.page_no);
        at = next.byte_offset();
    }
}

/// Decode a type-19/23 value — `[n] { [i][words][bytes][packed…] }*` — into its
/// strings, each read the way the database's header says text is stored.
/// `None` when the words do not hold that shape.
pub fn decode_text_list(words: &[u32], text: TextFormat) -> Option<Vec<String>> {
    let (&count, mut rest) = words.split_first()?;
    let mut items = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let (head, tail) = rest.split_first_chunk::<3>()?;
        let [_index, item_words, bytes] = *head;
        let item_words = item_words as usize;
        let packed = tail.get(..item_words)?;
        rest = &tail[item_words..];
        let raw: Vec<u8> = packed
            .iter()
            .flat_map(|word| word.to_be_bytes())
            .take((bytes as usize).min(item_words * 4))
            .collect();
        items.push(decode_as(&raw, text));
    }
    Some(items)
}

/// Decode a hunk's payload as the value its inline type would be: 20 → ints,
/// 21 → reals (two Fortran-ordered words each, as an inline real array), 22 →
/// text (`count` characters), 23 → a string list ([`decode_text_list`]; a
/// payload that is not list-shaped stays [`ExplicitValue::RawWords`]).
pub fn decode_hunk_value(hunk: &HunkRef, payload: &[u32], text: TextFormat) -> ExplicitValue {
    match hunk.hunk_type {
        20 => ExplicitValue::IntArray(payload.iter().map(|word| *word as i32).collect()),
        21 => ExplicitValue::RealArray(
            payload
                .as_chunks::<2>()
                .0
                .iter()
                .map(|[low, high]| f64::from_bits((u64::from(*high) << 32) | u64::from(*low)))
                .collect(),
        ),
        22 => {
            let bytes: Vec<u8> = payload
                .iter()
                .flat_map(|word| word.to_be_bytes())
                .take(hunk.count as usize)
                .collect();
            ExplicitValue::Text(decode_as(&bytes, text))
        }
        23 => match decode_text_list(payload, text) {
            Some(items) => ExplicitValue::TextList(items),
            None => ExplicitValue::RawWords(payload.to_vec()),
        },
        _ => ExplicitValue::RawWords(payload.to_vec()),
    }
}

/// Replace every hunk directory entry in `parsed`'s explicit stream with the
/// value it points at — the chain read through [`read_hunk`] and decoded by
/// [`decode_hunk_value`] under the record's own text format — the way
/// `sub_5AB4CF0`'s `type in 20..=23` arm hands a hunk back to `4GCEA` as if
/// it had been inline. The entry keeps its `type_code` (`20..=23 << 10`) and
/// `data_words` (5), so a caller can still tell a value that lived out of the
/// record from one that did not; only `value` changes. Returns how many
/// entries were resolved. Nothing is changed when a chain cannot be read: the
/// first unreadable hunk is the error and the entries before it stay resolved.
///
/// The stream decoder cannot do this itself — it has the record's bytes and
/// no pages — which is why a freshly parsed record carries the five raw
/// words. [`crate::ReadOnlyEngine::extract_element_with_descriptors`] and the
/// `DbElement` facade call this before decoding by descriptor; a caller that
/// parses records itself calls it when it wants the values rather than the
/// addresses (a writer re-emitting a record wants the addresses).
pub fn resolve_hunks(
    pages: &mut PageCache,
    ext: u16,
    parsed: &mut crate::record::element::ParsedElement,
) -> Result<usize, HunkError> {
    let refno = parsed.refno;
    let text = parsed.text_format;
    let mut resolved = 0;
    for attribute in &mut parsed.explicit_attributes {
        let Some(entry) = HunkRef::from_explicit(attribute) else {
            continue;
        };
        let payload = read_hunk(pages, ext, refno, &entry)?;
        attribute.value = decode_hunk_value(&entry, &payload, text);
        resolved += 1;
    }
    Ok(resolved)
}

fn be32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hunk_block(words: u16, owner: RefNo, cont: (u32, u32), hash: u32, hunk_type: u32, payload: &[u32]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&HUNK_KIND.to_be_bytes());
        out.extend_from_slice(&words.to_be_bytes());
        out.extend_from_slice(&owner.word0.to_be_bytes());
        out.extend_from_slice(&owner.word1.to_be_bytes());
        out.extend_from_slice(&cont.0.to_be_bytes());
        out.extend_from_slice(&cont.1.to_be_bytes());
        out.extend_from_slice(&hash.to_be_bytes());
        out.extend_from_slice(&hunk_type.to_be_bytes());
        for word in payload {
            out.extend_from_slice(&word.to_be_bytes());
        }
        out
    }

    #[test]
    fn a_directory_entry_is_five_words_under_a_hunk_type() {
        let entry = HunkRef::from_words(23 << 10, &[1, 640, (5 << 13) | 1, 17, 12]).expect("a directory");
        assert_eq!(entry.hunk_type, 23);
        assert_eq!((entry.page_no, entry.packed), (640, (5 << 13) | 1));
        assert_eq!((entry.count, entry.words), (17, 12));
        let address = entry.address().expect("an address");
        assert_eq!((address.page_no, address.byte_offset()), (640, 20));
        assert_eq!(entry.to_words(), [1, 640, (5 << 13) | 1, 17, 12]);
        assert_eq!(entry.type_code(), 0x5C00);

        // Not a hunk: an inline type, or a directory of the wrong length.
        assert!(HunkRef::from_words(19 << 10, &[1, 640, 1, 17, 12]).is_none());
        assert!(HunkRef::from_words(20 << 10, &[1, 640, 1, 17]).is_none());
        // No words, no block: the address is none.
        assert!(HunkRef::from_words(20 << 10, &[1, 0, 0, 0, 0]).unwrap().address().is_none());
    }

    #[test]
    fn a_hunk_block_states_its_owner_attribute_and_type() {
        let owner = RefNo::new(24_384, 90_001);
        let bytes = hunk_block(10, owner, (700, (1 << 13) | 1), 0xABC, 22, &[1, 2, 3]);
        let block = hunk_block_at(&bytes, 0, owner).expect("a hunk block");
        assert_eq!(block.total_bytes, 40);
        assert_eq!(block.payload(0), 28..40);
        assert_eq!((block.hash, block.hunk_type), (0xABC, 22));
        let cont = block.continues_at.expect("continues");
        assert_eq!((cont.page_no, cont.byte_offset()), (700, 4));

        // Another element's block, a tail block, a header too short: not this hunk.
        assert!(hunk_block_at(&bytes, 0, RefNo::new(24_384, 90_002)).is_none());
        let mut tail = bytes.clone();
        tail[0..2].copy_from_slice(&1u16.to_be_bytes());
        assert!(hunk_block_at(&tail, 0, owner).is_none());
        assert!(hunk_block_at(&hunk_block(6, owner, (0, 0), 0xABC, 22, &[]), 0, owner).is_none());
    }

    #[test]
    fn a_text_list_decodes_item_by_item() {
        // "AB" and "CDEFG": [2] {[0][1][2][AB\0\0]} {[1][2][5][CDEF][G\0\0\0]}
        let words = [2, 0, 1, 2, 0x4142_0000, 1, 2, 5, 0x4344_4546, 0x4700_0000];
        assert_eq!(
            decode_text_list(&words, TextFormat::Utf8),
            Some(vec!["AB".to_string(), "CDEFG".to_string()])
        );
        assert_eq!(decode_text_list(&[0], TextFormat::Utf8), Some(vec![]));
        // An item that promises more words than there are is not a list.
        assert_eq!(decode_text_list(&[1, 0, 3, 12, 0x4142_4344], TextFormat::Utf8), None);
        assert_eq!(decode_text_list(&[], TextFormat::Utf8), None);
    }

    #[test]
    fn hunk_payloads_decode_as_their_inline_type_would() {
        let text = TextFormat::Utf8;
        let ints = HunkRef { hunk_type: 20, page_no: 1, packed: 1, count: 3, words: 3 };
        assert_eq!(
            decode_hunk_value(&ints, &[1, 0xFFFF_FFFF, 7], text),
            ExplicitValue::IntArray(vec![1, -1, 7])
        );
        let reals = HunkRef { hunk_type: 21, page_no: 1, packed: 1, count: 1, words: 2 };
        let bits = 2.5f64.to_bits();
        assert_eq!(
            decode_hunk_value(&reals, &[bits as u32, (bits >> 32) as u32], text),
            ExplicitValue::RealArray(vec![2.5])
        );
        let chars = HunkRef { hunk_type: 22, page_no: 1, packed: 1, count: 5, words: 2 };
        assert_eq!(
            decode_hunk_value(&chars, &[0x4845_4C4C, 0x4F00_0000], text),
            ExplicitValue::Text("HELLO".to_string())
        );
        let list = HunkRef { hunk_type: 23, page_no: 1, packed: 1, count: 8, words: 5 };
        assert_eq!(
            decode_hunk_value(&list, &[1, 0, 1, 2, 0x4142_0000], text),
            ExplicitValue::TextList(vec!["AB".to_string()])
        );
        // A type-23 payload that is not list-shaped stays raw.
        assert_eq!(
            decode_hunk_value(&list, &[1, 0, 3, 12, 0x4142_4344], text),
            ExplicitValue::RawWords(vec![1, 0, 3, 12, 0x4142_4344])
        );
    }

    #[test]
    fn the_type_tables_invert_each_other() {
        for inline in [6u16, 7, 10, 19] {
            let hunk = hunk_type_of(inline).unwrap();
            assert!(is_hunk_type(hunk));
            assert_eq!(inline_type_of(hunk), Some(inline));
        }
        assert_eq!(hunk_type_of(3), None);
        assert_eq!(inline_type_of(19), None);
        assert_eq!(payload_words(20, 5), Some(5));
        assert_eq!(payload_words(21, 5), Some(10));
        assert_eq!(payload_words(22, 5), Some(2));
        assert_eq!(payload_words(23, 5), None);
    }
}
