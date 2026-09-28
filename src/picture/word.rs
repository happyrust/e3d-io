//! The 8-byte H-list word a GW picture file is made of, and 1-based views over
//! runs of them.
//!
//! Wigwam keeps picture data in Fortran H-lists whose unit is one 8-byte word
//! made of two 4-byte halves `u0`, `u1`. `FHDBWN` writes each half big-endian
//! and keeps their order, so on disk a word is `u0.to_be_bytes() ++
//! u1.to_be_bytes()`. Three readings share that one layout
//! (`docs/evidence/2026-09-28-gw-picture-format.md` §1):
//!
//! * integer (`HPUTI1`): `u0 = 0`, `u1` = the signed 32-bit value;
//! * real (`HPUTRA`): the 8 bytes are one IEEE double with `u0` the low
//!   dword and `u1` the high dword — `f64::from_bits(u1 << 32 | u0)`;
//! * header (element / group / primitive): `u0 = kind << 27 | payload`,
//!   `u1 = length | extra << 16`.
//!
//! Nothing here interprets a word; a caller that knows what a position holds
//! picks the reading. Round-tripping goes through the same three constructors,
//! so a word read as a real and written back as one is the same 8 bytes.

use super::PictureError;

/// One 8-byte word: two 4-byte units, each stored big-endian.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Word {
    pub u0: u32,
    pub u1: u32,
}

impl Word {
    /// Bytes one word occupies on disk.
    pub const BYTES: usize = 8;

    /// The word whose 8 bytes are `bytes`, each half read big-endian.
    pub fn from_be(bytes: &[u8; 8]) -> Self {
        Self {
            u0: u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            u1: u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        }
    }

    /// The 8 bytes `FHDBWN` writes for this word.
    pub fn to_be(self) -> [u8; 8] {
        let mut out = [0u8; 8];
        out[..4].copy_from_slice(&self.u0.to_be_bytes());
        out[4..].copy_from_slice(&self.u1.to_be_bytes());
        out
    }

    /// An integer word (`HPUTI1`): `u0 = 0`, `u1` = the value.
    pub const fn int(value: i32) -> Self {
        Self {
            u0: 0,
            u1: value as u32,
        }
    }

    /// A real word (`HPUTRA`): the double's bits split low / high.
    pub fn real(value: f64) -> Self {
        let bits = value.to_bits();
        Self {
            u0: bits as u32,
            u1: (bits >> 32) as u32,
        }
    }

    /// A header word from its four fields.
    pub const fn header(kind: u8, payload: u32, length: u16, extra: u16) -> Self {
        Self {
            u0: ((kind as u32) << 27) | (payload & 0x07FF_FFFF),
            u1: (length as u32) | ((extra as u32) << 16),
        }
    }

    /// A word from its two raw halves.
    pub const fn units(u0: u32, u1: u32) -> Self {
        Self { u0, u1 }
    }

    /// The integer reading: `u1` as a signed 32-bit value.
    pub const fn as_int(self) -> i32 {
        self.u1 as i32
    }

    /// The real reading: `u1 << 32 | u0` as IEEE double bits.
    pub fn as_real(self) -> f64 {
        f64::from_bits(((self.u1 as u64) << 32) | self.u0 as u64)
    }

    /// Header reading: the 5-bit kind in the top of `u0` (14 = segment,
    /// 15 = group, 1–10 = a primitive).
    pub const fn kind(self) -> u8 {
        (self.u0 >> 27) as u8
    }

    /// Header reading: the low 27 bits of `u0` (segment flags, group id, or a
    /// primitive's pen).
    pub const fn payload(self) -> u32 {
        self.u0 & 0x07FF_FFFF
    }

    /// Header reading: the low 16 bits of `u1` — the block's length in words,
    /// header included.
    pub const fn length(self) -> u16 {
        (self.u1 & 0xFFFF) as u16
    }

    /// Header reading: the high 16 bits of `u1` (a primitive's colour number).
    pub const fn extra(self) -> u16 {
        (self.u1 >> 16) as u16
    }
}

impl std::fmt::Debug for Word {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Word({:#010x}, {:#010x})", self.u0, self.u1)
    }
}

/// A 1-based view over consecutive words, the way the Fortran H-list routines
/// address them (`HGETI1(list, i)` is word `i`, counting from 1).
///
/// The view remembers the record it starts at so an out-of-range read names a
/// place in the file rather than a byte offset.
#[derive(Clone, Copy)]
pub struct Words<'a> {
    data: &'a [u8],
    rec: u32,
}

impl<'a> Words<'a> {
    /// A view whose word 1 is the first 8 bytes of `data`, said to start at
    /// record `rec` (1-based; 0 when the caller has no record to name).
    pub fn new(data: &'a [u8], rec: u32) -> Self {
        Self { data, rec }
    }

    /// The 1-based record this view starts at.
    pub fn rec(&self) -> u32 {
        self.rec
    }

    /// Whole words available from word 1 to the end of the data.
    pub fn len(&self) -> usize {
        self.data.len() / Word::BYTES
    }

    /// True when no whole word is available.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Word `i` (1-based), or an error naming the record and word when it
    /// lies past the end of the data.
    pub fn get(&self, i: usize) -> Result<Word, PictureError> {
        let start = i
            .checked_sub(1)
            .and_then(|k| k.checked_mul(Word::BYTES))
            .ok_or(PictureError::ShortFile {
                rec: self.rec,
                word: i,
            })?;
        let bytes: &[u8; 8] = self
            .data
            .get(start..start + Word::BYTES)
            .and_then(|s| s.try_into().ok())
            .ok_or(PictureError::ShortFile {
                rec: self.rec,
                word: i,
            })?;
        Ok(Word::from_be(bytes))
    }

    /// Word `i` read as an integer.
    pub fn int(&self, i: usize) -> Result<i32, PictureError> {
        self.get(i).map(Word::as_int)
    }

    /// Word `i` read as a real.
    pub fn real(&self, i: usize) -> Result<f64, PictureError> {
        self.get(i).map(Word::as_real)
    }

    /// The bytes of words `i .. i + n` (1-based, `n` words).
    pub fn raw(&self, i: usize, n: usize) -> Result<&'a [u8], PictureError> {
        let start = i
            .checked_sub(1)
            .and_then(|k| k.checked_mul(Word::BYTES))
            .ok_or(PictureError::ShortFile {
                rec: self.rec,
                word: i,
            })?;
        let end = n
            .checked_mul(Word::BYTES)
            .and_then(|len| start.checked_add(len))
            .ok_or(PictureError::ShortFile {
                rec: self.rec,
                word: i + n,
            })?;
        self.data.get(start..end).ok_or(PictureError::ShortFile {
            rec: self.rec,
            word: i + n.saturating_sub(1),
        })
    }

    /// Words `i .. i + n` decoded, in order.
    pub fn slice(&self, i: usize, n: usize) -> Result<Vec<Word>, PictureError> {
        let bytes = self.raw(i, n)?;
        Ok(bytes
            .as_chunks::<{ Word::BYTES }>()
            .0
            .iter()
            .map(Word::from_be)
            .collect())
    }

    /// The same data seen from word `i` onward: the new view's word 1 is
    /// this view's word `i`. Record bookkeeping is kept for messages only.
    pub fn from_word(&self, i: usize) -> Result<Words<'a>, PictureError> {
        let start = i
            .checked_sub(1)
            .and_then(|k| k.checked_mul(Word::BYTES))
            .filter(|start| *start <= self.data.len())
            .ok_or(PictureError::ShortFile {
                rec: self.rec,
                word: i,
            })?;
        Ok(Words {
            data: &self.data[start..],
            rec: self.rec,
        })
    }
}

/// Serialise `words` back to the bytes they came from.
pub fn to_bytes(words: &[Word]) -> Vec<u8> {
    let mut out = Vec::with_capacity(words.len() * Word::BYTES);
    for word in words {
        out.extend_from_slice(&word.to_be());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_readings_round_trip_through_their_constructors() {
        let int = Word::int(-11);
        assert_eq!(int.u0, 0);
        assert_eq!(int.as_int(), -11);
        assert_eq!(Word::from_be(&int.to_be()), int);

        let real = Word::real(-297.0);
        assert_eq!(real.as_real(), -297.0);
        assert_eq!(Word::from_be(&real.to_be()), real);
        // 1.0 is 0x3FF0_0000_0000_0000: the high dword carries all of it.
        assert_eq!(Word::real(1.0), Word::units(0, 0x3FF0_0000));

        let header = Word::header(14, 0x10f, 11, 0);
        assert_eq!(header.kind(), 14);
        assert_eq!(header.payload(), 0x10f);
        assert_eq!(header.length(), 11);
        assert_eq!(header.extra(), 0);
        assert_eq!(header.u0, 0x7000_010f);

        // A primitive header: pen 8107 | kind 3 << 27, length 11 | colour 1 << 16.
        let prim = Word::header(3, 8107, 11, 1);
        assert_eq!(prim.u0, (3 << 27) | 8107);
        assert_eq!(prim.u1, 11 | (1 << 16));
    }

    #[test]
    fn the_on_disk_order_is_two_big_endian_halves() {
        let bytes = [0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xf5];
        let word = Word::from_be(&bytes);
        assert_eq!(word.as_int(), -11);
        assert_eq!(word.to_be(), bytes);
    }

    #[test]
    fn views_are_one_based_and_report_the_record_they_start_at() {
        let mut data = Vec::new();
        data.extend_from_slice(&Word::int(7).to_be());
        data.extend_from_slice(&Word::real(2.5).to_be());
        data.extend_from_slice(&[0xAA; 3]); // a torn tail is not a word
        let words = Words::new(&data, 318);
        assert_eq!(words.len(), 2);
        assert_eq!(words.int(1).unwrap(), 7);
        assert_eq!(words.real(2).unwrap(), 2.5);
        assert_eq!(words.raw(1, 2).unwrap().len(), 16);
        assert_eq!(
            words.slice(1, 2).unwrap(),
            vec![Word::int(7), Word::real(2.5)]
        );
        match words.get(3) {
            Err(PictureError::ShortFile { rec: 318, word: 3 }) => {}
            other => panic!("expected ShortFile at record 318 word 3, got {other:?}"),
        }
        assert!(words.get(0).is_err());
        assert!(words.raw(2, 2).is_err());
        let tail = words.from_word(2).unwrap();
        assert_eq!(tail.len(), 1);
        assert_eq!(tail.real(1).unwrap(), 2.5);
        assert_eq!(to_bytes(&[Word::int(7), Word::real(2.5)]), data[..16]);
    }
}
