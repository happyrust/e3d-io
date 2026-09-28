//! A whole GW picture file: records, the header list, the name and attribute
//! blocks, and the picture table.
//!
//! The file is a run of 128-byte records; every position the format stores is
//! a 1-based record number (`docs/evidence/2026-09-28-gw-picture-format.md`
//! §1–§2). Record 1 is the header list of 21 words: 15 version characters one
//! per word (`GVRSN` + `" PC 06"`), then record / length pairs for the name
//! block, the attribute table and the picture table. Everything else is found
//! from those six words — the file is never scanned to its end, because a
//! re-saved picture that shrank leaves its old tail in place (334 of the 881
//! Plant files) and the last record of every block ends in heap garbage.

use std::fs;
use std::path::{Path, PathBuf};

use crate::refno::RefNo;

use super::directory::Picture;
use super::word::{Word, Words};
use super::PictureError;

/// Bytes per record (`HWDBIR`: 32 four-byte units).
pub const RECORD_BYTES: usize = 128;

/// Words in the header list at record 1.
pub const HEADER_WORDS: usize = 21;

/// The header list: version string and the three block locations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// 15 characters, e.g. `"GW6A04    PC 06"`: 9 of `GVRSN` then `" PC 06"`.
    pub version: String,
    /// 1-based record of the name block.
    pub name_rec: i32,
    /// Words in the name block (one character each).
    pub name_len: i32,
    /// 1-based record of the attribute table.
    pub attr_rec: i32,
    /// Words in the attribute table (0 or 132 on every sample).
    pub attr_len: i32,
    /// 1-based record of the picture table.
    pub pic_rec: i32,
    /// Words in the picture table (8 per picture).
    pub pic_len: i32,
}

impl Header {
    /// The 21 words `GSVPI0` writes for this header, for round-tripping.
    pub fn encode(&self) -> Vec<Word> {
        let mut out: Vec<Word> = self.version.bytes().map(|c| Word::int(c as i32)).collect();
        out.extend(
            [
                self.name_rec,
                self.name_len,
                self.attr_rec,
                self.attr_len,
                self.pic_rec,
                self.pic_len,
            ]
            .map(Word::int),
        );
        out
    }
}

/// The name block: `"<w0> <w1> <EXFI> 0 <PVNO>"`, one character per word.
///
/// `w0/w1` is the refno of the owning SHEE (or OVER), `EXFI` the extract
/// database number and `PVNO` the sheet's picture version — the same numbers
/// the file name `M<w0>-<w1>-<EXFI>-<PVNO>` spells. The fields are parsed
/// when the text has that shape and left `None` otherwise; the text itself
/// is always kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PictureName {
    /// The block's characters with trailing blanks removed.
    pub text: String,
    /// Words the block occupies (`Header::name_len`), trailing blanks included.
    pub len: i32,
    /// `(w0, w1)` as a refno, when the text parses.
    pub owner: Option<RefNo>,
    /// Extract database number, when the text parses.
    pub exfi: Option<i32>,
    /// Picture version number, when the text parses.
    pub pvno: Option<i32>,
}

impl PictureName {
    fn parse(text: String, len: i32) -> Self {
        let fields: Vec<i64> = text
            .split_whitespace()
            .map(|f| f.parse::<i64>())
            .collect::<Result<_, _>>()
            .unwrap_or_default();
        let (owner, exfi, pvno) = match fields.as_slice() {
            [w0, w1, exfi, _zero, pvno] => (
                Some(RefNo::new(*w0 as u32, *w1 as u32)),
                i32::try_from(*exfi).ok(),
                i32::try_from(*pvno).ok(),
            ),
            _ => (None, None, None),
        };
        Self {
            text,
            len,
            owner,
            exfi,
            pvno,
        }
    }

    /// The block's words as written: the characters, then blanks to `len`.
    pub fn encode(&self) -> Vec<Word> {
        let mut out: Vec<Word> = self.text.bytes().map(|c| Word::int(c as i32)).collect();
        out.resize(self.len.max(0) as usize, Word::int(b' ' as i32));
        out
    }
}

/// One GW picture file, read whole.
#[derive(Debug)]
pub struct PictureFile {
    path: PathBuf,
    bytes: Vec<u8>,
    header: Header,
    name: PictureName,
    attrs: Vec<Word>,
    pictures: Vec<Picture>,
}

impl PictureFile {
    /// Read `path` and decode every block the header points at, directories
    /// and segments included.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PictureError> {
        let path = path.as_ref();
        let bytes = fs::read(path).map_err(|source| PictureError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_bytes(path.to_path_buf(), bytes)
    }

    /// Decode a file already in memory; `path` is kept for messages.
    pub fn from_bytes(path: PathBuf, bytes: Vec<u8>) -> Result<Self, PictureError> {
        let head = Words::new(&bytes, 1);
        let mut version = String::with_capacity(15);
        for i in 1..=15 {
            let c = head.int(i).map_err(|_| PictureError::NotAPicture {
                path: path.clone(),
                found: format!("{} bytes, shorter than the 21-word header", bytes.len()),
            })?;
            version.push((c & 0xFF) as u8 as char);
        }
        if !version.starts_with("GW") {
            return Err(PictureError::NotAPicture {
                path,
                found: format!("header version {version:?}"),
            });
        }
        let header = Header {
            version,
            name_rec: head.int(16)?,
            name_len: head.int(17)?,
            attr_rec: head.int(18)?,
            attr_len: head.int(19)?,
            pic_rec: head.int(20)?,
            pic_len: head.int(21)?,
        };

        let words_at = |rec: i32| record_words(&bytes, rec);

        let name_words = words_at(header.name_rec)?;
        let mut text = String::new();
        for i in 1..=header.name_len.max(0) as usize {
            text.push((name_words.int(i)? & 0xFF) as u8 as char);
        }
        let name = PictureName::parse(text.trim_end().to_string(), header.name_len);

        let attrs = if header.attr_len > 0 {
            words_at(header.attr_rec)?.slice(1, header.attr_len as usize)?
        } else {
            Vec::new()
        };

        let mut pictures = Vec::new();
        if header.pic_len > 0 {
            let table = words_at(header.pic_rec)?;
            for k in 0..(header.pic_len as usize / Picture::TABLE_WORDS) {
                let entry = table.from_word(1 + k * Picture::TABLE_WORDS)?;
                pictures.push(Picture::parse(entry, &words_at)?);
            }
        }

        Ok(Self {
            path,
            bytes,
            header,
            name,
            attrs,
            pictures,
        })
    }

    /// Where the file was read from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The whole file.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Whole 128-byte records in the file.
    pub fn records(&self) -> usize {
        self.bytes.len() / RECORD_BYTES
    }

    /// The header list at record 1.
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// The name block.
    pub fn name(&self) -> &PictureName {
        &self.name
    }

    /// The attribute table's words, uninterpreted.
    pub fn attrs(&self) -> &[Word] {
        &self.attrs
    }

    /// The pictures in table order.
    pub fn pictures(&self) -> &[Picture] {
        &self.pictures
    }

    /// The words starting at 1-based record `rec`, running to the end of the
    /// file.
    pub fn words_at(&self, rec: i32) -> Result<Words<'_>, PictureError> {
        record_words(&self.bytes, rec)
    }

    /// The last record any block or element occupies, from the header's
    /// positions and the directories' entries. Records past it are the
    /// residue of an earlier, longer save.
    pub fn last_used_record(&self) -> i32 {
        let end = |rec: i32, words: i32| -> i32 {
            if words <= 0 {
                return rec;
            }
            rec + ((words as usize * Word::BYTES).saturating_sub(1) / RECORD_BYTES) as i32
        };
        let h = &self.header;
        let mut last = end(1, HEADER_WORDS as i32)
            .max(end(h.name_rec, h.name_len))
            .max(end(h.attr_rec, h.attr_len))
            .max(end(h.pic_rec, h.pic_len));
        for picture in &self.pictures {
            last = last.max(end(picture.dir_rec, picture.dir_len));
            for entry in &picture.entries {
                last = last.max(end(entry.rec(), entry.nwords()));
            }
        }
        last
    }
}

/// The words from 1-based record `rec` to the end of `bytes`.
fn record_words(bytes: &[u8], rec: i32) -> Result<Words<'_>, PictureError> {
    let records = bytes.len() / RECORD_BYTES;
    let index = usize::try_from(rec)
        .ok()
        .filter(|r| (1..=records).contains(r))
        .ok_or(PictureError::BadRecord { rec, records })?;
    Ok(Words::new(&bytes[(index - 1) * RECORD_BYTES..], rec as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_block_parses_the_five_numbers_and_keeps_the_text() {
        let name = PictureName::parse("2013286677 4350 10000 0 11".to_string(), 32);
        assert_eq!(name.owner, Some(RefNo::new(2013286677, 4350)));
        assert_eq!(name.exfi, Some(10000));
        assert_eq!(name.pvno, Some(11));
        let words = name.encode();
        assert_eq!(words.len(), 32);
        assert_eq!(words[0], Word::int(b'2' as i32));
        assert_eq!(words[31], Word::int(b' ' as i32));

        let odd = PictureName::parse("hello".to_string(), 5);
        assert_eq!(odd.owner, None);
        assert_eq!(odd.text, "hello");
    }

    #[test]
    fn a_file_that_does_not_start_with_gw_is_refused() {
        let bytes = vec![0u8; 256];
        let err = PictureFile::from_bytes(PathBuf::from("x"), bytes).unwrap_err();
        assert!(matches!(err, PictureError::NotAPicture { .. }), "{err}");
        let err = PictureFile::from_bytes(PathBuf::from("x"), vec![0u8; 10]).unwrap_err();
        assert!(matches!(err, PictureError::NotAPicture { .. }), "{err}");
    }

    #[test]
    fn the_header_encodes_back_to_its_twenty_one_words() {
        let header = Header {
            version: "GW6A04    PC 06".to_string(),
            name_rec: 3,
            name_len: 32,
            attr_rec: 5,
            attr_len: 132,
            pic_rec: 14,
            pic_len: 8,
        };
        let words = header.encode();
        assert_eq!(words.len(), HEADER_WORDS);
        assert_eq!(words[0], Word::int(b'G' as i32));
        assert_eq!(words[14], Word::int(b'6' as i32));
        assert_eq!(words[15], Word::int(3));
        assert_eq!(words[20], Word::int(8));
    }
}
