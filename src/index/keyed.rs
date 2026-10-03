//! One page of a named table keyed by four dwords — the reverse-reference
//! tables ([`super::reference`], no leaf value) and SPAMAP ([`super::spamap`],
//! a seven-dword leaf value) — decoded by one function, so the two cannot
//! disagree about what a page of such a table is.
//!
//! Header, seven dwords: `page_type`, `name_hash`, `level`, `key_dwords`,
//! `value_dwords`, reserved, `free_dwords`. The declared value width
//! describes the leaf. An internal slot carries its key and then the child's
//! `(pgno, extno)` whatever the leaf holds — the same asymmetry the refno
//! index has, where the leaf value is a record address and the internal value
//! a child address.
//!
//! The refno index's own page reader ([`super::IndexNode`]) does not serve
//! here: it requires the header's second dword to be the refno index's name
//! hash, and it strides every level by `key_dwords + value_dwords`.

/// `page_type` of every B+tree page, reference tables included.
pub const TABLE_PAGE_TYPE: u32 = 5;

/// Every table read here has a four-dword key.
pub(crate) const KEY_DWORDS: u32 = 4;

/// An internal entry is followed by its child's `(pgno, extno)`.
pub(crate) const CHILD_DWORDS: u32 = 2;

pub(crate) const DWORD: usize = 4;
pub(crate) const HEADER_DWORDS: usize = 7;

/// All four words of a sentinel key, the same page-local constant the refno
/// index uses. core.dll matches it as the signed `-2147483647`.
pub(crate) const SENTINEL_WORD: u32 = 0x8000_0001;

/// One slot of a decoded page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyedSlot<const V: usize> {
    /// `None` on the sentinel.
    pub key: Option<[u32; 4]>,
    /// The leaf value as stored; all zero on an internal page.
    pub value: [u32; V],
    /// The child's page number on an internal page; `None` on a leaf.
    pub child: Option<u32>,
}

/// A decoded page of a table whose leaf value is `V` dwords wide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeyedNode<const V: usize> {
    pub level: u32,
    pub slots: Vec<KeyedSlot<V>>,
}

/// Why a page is not a page of the table it was read as. The caller adds the
/// page number and the table's own name for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyedPageError {
    PageNotDwordAligned { len: usize },
    PageTooSmall { len: usize, needed: usize },
    UnexpectedPageType { found: u32 },
    WrongTable { found: u32 },
    /// The key is not four dwords, or a leaf's value is not `V`.
    WrongShape { level: u32, key_dwords: u32, value_dwords: u32 },
    FreeSpaceOutOfRange { free_dwords: u32, usable_dwords: u32 },
    FreeSpaceNotEntryAligned { used_dwords: u64, stride_dwords: u64 },
}

/// Decode one page of the table named `name_hash`, whose leaves carry `V`
/// value dwords. Checks the header and the live region; key order is the
/// caller's, since the tables differ on whether equal keys may follow.
pub(crate) fn decode<const V: usize>(data: &[u8], name_hash: u32) -> Result<KeyedNode<V>, KeyedPageError> {
    if data.len() % DWORD != 0 {
        return Err(KeyedPageError::PageNotDwordAligned { len: data.len() });
    }
    if data.len() < HEADER_DWORDS * DWORD {
        return Err(KeyedPageError::PageTooSmall { len: data.len(), needed: HEADER_DWORDS * DWORD });
    }
    let word = |index: usize| {
        u32::from_be_bytes(data[index * DWORD..index * DWORD + DWORD].try_into().expect("slice is exactly four bytes"))
    };

    let page_type = word(0);
    if page_type != TABLE_PAGE_TYPE {
        return Err(KeyedPageError::UnexpectedPageType { found: page_type });
    }
    let found = word(1);
    if found != name_hash {
        return Err(KeyedPageError::WrongTable { found });
    }

    let level = word(2);
    let key_dwords = word(3);
    let value_dwords = word(4);
    if key_dwords != KEY_DWORDS || (level == 0 && value_dwords as usize != V) {
        return Err(KeyedPageError::WrongShape { level, key_dwords, value_dwords });
    }
    let free_dwords = word(6);

    let usable_dwords = data.len() / DWORD - HEADER_DWORDS;
    if free_dwords as usize > usable_dwords {
        return Err(KeyedPageError::FreeSpaceOutOfRange { free_dwords, usable_dwords: usable_dwords as u32 });
    }
    let stride_dwords = u64::from(key_dwords) + if level == 0 { V as u64 } else { u64::from(CHILD_DWORDS) };
    let used_dwords = usable_dwords as u64 - u64::from(free_dwords);
    if used_dwords % stride_dwords != 0 {
        return Err(KeyedPageError::FreeSpaceNotEntryAligned { used_dwords, stride_dwords });
    }

    let count = (used_dwords / stride_dwords) as usize;
    let mut slots = Vec::with_capacity(count);
    for slot in 0..count {
        let at = HEADER_DWORDS + slot * stride_dwords as usize;
        let key = [word(at), word(at + 1), word(at + 2), word(at + 3)];
        let after_key = at + KEY_DWORDS as usize;
        let mut value = [0u32; V];
        let child = if level == 0 {
            for (index, slot) in value.iter_mut().enumerate() {
                *slot = word(after_key + index);
            }
            None
        } else {
            Some(word(after_key))
        };
        let sentinel = key.iter().all(|&w| w == SENTINEL_WORD);
        slots.push(KeyedSlot { key: (!sentinel).then_some(key), value, child });
    }
    Ok(KeyedNode { level, slots })
}
