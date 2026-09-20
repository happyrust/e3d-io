/// Element reference number: two u32 words encoding database number and element identity.
///
/// Bit layout recovered from E3D 3.1 core.dll (sub_5AEB6B0):
/// - dbno = (word0 & 0x1FFF) | ((word0 >> 13) & 0x3E000)
/// - word1 must be >= 0 (sign bit = 0)
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct RefNo {
    pub word0: u32,
    pub word1: u32,
}

impl RefNo {
    pub const fn new(word0: u32, word1: u32) -> Self {
        Self { word0, word1 }
    }

    pub const fn dbno(&self) -> u32 {
        (self.word0 & 0x1FFF) | ((self.word0 >> 13) & 0x3E000)
    }

    /// Extent index carried in bits 13..25 of `word0`.
    ///
    /// `dbno` is split around this field — bits 0..12 hold its low half and
    /// bits 26..30 its high half — so what sits between them is the extent the
    /// element was allocated in. A database's first extent is index 1, and
    /// each further one adds 0x2000 to `word0`.
    pub const fn extent(&self) -> u32 {
        (self.word0 >> 13) & 0x1FFF
    }

    pub const fn is_valid(&self) -> bool {
        (self.word1 as i32) >= 0 && self.dbno() > 0
    }

    pub fn from_be_bytes(bytes: &[u8; 8]) -> Self {
        Self {
            word0: u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            word1: u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        }
    }

    pub fn to_be_bytes(&self) -> [u8; 8] {
        let mut out = [0u8; 8];
        out[..4].copy_from_slice(&self.word0.to_be_bytes());
        out[4..].copy_from_slice(&self.word1.to_be_bytes());
        out
    }
}

impl std::fmt::Debug for RefNo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RefNo({}/{}, dbno={})",
            self.word0,
            self.word1 as i32,
            self.dbno()
        )
    }
}

impl std::fmt::Display for RefNo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.word0, self.word1 as i32)
    }
}

/// Encoded page address returned by B+tree index search.
///
/// Layout recovered from sub_5AEB6B0 LABEL_87, and confirmed on the write side
/// (`sub_5AE9DF0`: `word1 = ext | (word_offset << 13)`) and in the record
/// copier (`sub_5AE9960`: `page = get_page(word0, word1 & 0x1FFF)`,
/// `record = page + 4 * ((word1 >> 13) & 0xFFF)`):
/// - word0 = page_number
/// - word1 bits 0..=12 (`& 0x1FFF`) = `slot_offset` — misnamed: this is the
///   **extent number** of the page (1 = the primary file, 0 = an unsaved temp
///   page)
/// - word1 bits 13..=24 (`>> 13 & 0xFFF`) = `slot_index` — misnamed: the
///   record's offset from the top of the page in **4-byte words**
///
/// The field names predate that reading and are kept for the callers that
/// spell them. `index::IndexEntry` reads the same word as `offset_words`
/// (2-byte words, `>> 12`) plus `flag`, which coincides with this layout while
/// the extent number is below 4096 — it is 1 on every sample database.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageAddress {
    pub page_number: u32,
    pub slot_offset: u16,
    pub slot_index: u16,
}

impl PageAddress {
    pub fn decode(word0: u32, word1: u32) -> Self {
        Self {
            page_number: word0,
            slot_offset: (word1 & 0x1FFF) as u16,
            slot_index: ((word1 >> 13) & 0xFFF) as u16,
        }
    }

    pub fn encode(&self) -> (u32, u32) {
        let word1 = (self.slot_offset as u32) | ((self.slot_index as u32) << 13);
        (self.page_number, word1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refno_dbno_extraction() {
        let r = RefNo::new(0x0001, 1);
        assert_eq!(r.dbno(), 1);
        assert!(r.is_valid());

        // E3D 2.1 jeuglb capture: word0 carries extent 2 around dbnum 8188.
        let jeu_global = RefNo::new(24_572, 115);
        assert_eq!(jeu_global.dbno(), 8_188);
        assert_eq!(jeu_global.extent(), 2);
        assert!(jeu_global.is_valid());

        let r2 = RefNo::new(0, 0);
        assert_eq!(r2.dbno(), 0);
        assert!(!r2.is_valid());
    }

    #[test]
    fn page_address_roundtrip() {
        let addr = PageAddress {
            page_number: 42,
            slot_offset: 100,
            slot_index: 5,
        };
        let (w0, w1) = addr.encode();
        let decoded = PageAddress::decode(w0, w1);
        assert_eq!(addr, decoded);
    }

    /// `ams251000_0001`: a database number too large to fit the low 13 bits,
    /// so `dbno` is reassembled from both halves around the extent index.
    #[test]
    fn refno_splits_large_dbno_around_extent_index() {
        let primary = RefNo::new(2_013_279_352, 1);
        assert_eq!(primary.dbno(), 251_000);
        assert_eq!(primary.extent(), 1);

        let second = RefNo::new(primary.word0 + 0x2000, 1);
        assert_eq!(second.dbno(), 251_000);
        assert_eq!(second.extent(), 2);
    }

    /// `ams7999_0001` and the two extra extents listed on its page 1.
    #[test]
    fn refno_extent_index_counts_from_one() {
        assert_eq!(RefNo::new(16_191, 1).extent(), 1);
        assert_eq!(RefNo::new(24_383, 1).extent(), 2);
        assert_eq!(RefNo::new(32_575, 1).extent(), 3);
        for word0 in [16_191, 24_383, 32_575] {
            assert_eq!(RefNo::new(word0, 1).dbno(), 7999);
        }
    }

    #[test]
    fn refno_negative_word1_invalid() {
        let r = RefNo::new(1, 0x8000_0000);
        assert!(!r.is_valid());
    }
}
