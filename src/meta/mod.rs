pub mod constants;
pub mod descriptor;

use crate::error::E3dReaderError;

use nom::number::complete::be_u32;
use nom::IResult;

/// Database descriptor parsed from page 0.
///
/// Layout from struct_layouts.json "db_descriptor_page"
#[derive(Debug, Clone)]
pub struct DbDescriptor {
    pub page_size: u32,
    pub db_mark: u32,
    pub session_page_no: u32,
    /// `RefNo::word0` of the database's first extent, i.e. `db_mark + 0x2000`.
    pub primary_extent_ref: u32,
    pub bucket_type_id: u32,
}

/// One extent allocation recorded on page 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtentEntry {
    /// `RefNo::word0` carried by every element stored in this extent.
    /// Decodes back to `db_mark` through [`crate::refno::RefNo::dbno`].
    pub ref_word0: u32,
    /// Highest element number handed out inside this extent so far.
    pub high_water: u32,
    pub flags: u32,
}

/// Extent allocation table parsed from page 1.
///
/// The page holds a three-word cursor followed by three-word entries, one per
/// extent allocated *after* the primary one (which lives in the descriptor).
/// It carries no page count and no page size — an E3D database file states its
/// length nowhere, so the file size is the only bound on addressable pages.
///
/// Census over the 448 databases of `AvevaMarineSample\ams000`:
/// `page_flag` is 1 and `signature` is 0xFFFFFFFF everywhere, while
/// `table_end_word` only ever takes the values 2, 5, 8 and 11 — that is
/// `2 + 3n` for n = 0..3 allocated extents.
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub page_flag: u32,
    pub signature: u32,
    /// Index of the first unused word on the page: `2 + 3 * extents.len()`.
    pub table_end_word: u32,
    pub extents: Vec<ExtentEntry>,
}

impl FileInfo {
    pub const PAGE_INDEX: u32 = 1;

    /// Page 1 word 1, 0xFFFFFFFF in all 982 databases of the seven installed
    /// E3D 3.1 projects. This is the only header field that separates a
    /// database from the extensionless plain-text files that sit next to them
    /// in the `*dflts` directories.
    pub const SIGNATURE: u32 = u32::MAX;

    /// Words 0..2 are the page flag, the signature and the table cursor.
    const TABLE_START_WORD: u32 = 2;
    const WORDS_PER_ENTRY: u32 = 3;

    pub fn parse(data: &[u8]) -> Option<Self> {
        parse_file_info(data).ok().map(|(_, fi)| fi)
    }

    pub fn parse_page1(data: &[u8]) -> Result<Self, E3dReaderError> {
        let info = Self::parse(data).ok_or_else(|| E3dReaderError::InvalidPage {
            page_no: Self::PAGE_INDEX,
            detail: "file info page too small".to_string(),
        })?;
        if info.signature != Self::SIGNATURE {
            return Err(E3dReaderError::InvalidPage {
                page_no: Self::PAGE_INDEX,
                detail: format!(
                    "file info signature 0x{:08X}, expected 0x{:08X}: not a database",
                    info.signature,
                    Self::SIGNATURE
                ),
            });
        }
        Ok(info)
    }

    /// Every `RefNo::word0` this database hands out, primary extent first.
    ///
    /// An element physically stored here may carry any of these; the sampled
    /// elements of `ams8000_0001`, for instance, split 57/2 across its two.
    pub fn extent_refs(&self, descriptor: &DbDescriptor) -> Vec<u32> {
        let mut refs = vec![descriptor.primary_extent_ref];
        refs.extend(self.extents.iter().map(|e| e.ref_word0));
        refs
    }
}

fn parse_file_info(input: &[u8]) -> IResult<&[u8], FileInfo> {
    use nom::Parser;
    let (input, page_flag) = be_u32.parse(input)?;
    let (input, signature) = be_u32.parse(input)?;
    let (mut rest, table_end_word) = be_u32.parse(input)?;

    // Read only whole entries that the page actually holds, and stop at the
    // first empty slot. A malformed cursor must not fail the open: this field
    // was misread as a page size for a long time and that mistake locked the
    // reader out of 392 of the 448 databases in AvevaMarineSample.
    let declared_entries = table_end_word
        .saturating_sub(FileInfo::TABLE_START_WORD)
        / FileInfo::WORDS_PER_ENTRY;
    let mut extents = Vec::new();
    for _ in 0..declared_entries {
        if rest.len() < (FileInfo::WORDS_PER_ENTRY as usize) * 4 {
            break;
        }
        let (after, ref_word0) = be_u32.parse(rest)?;
        let (after, high_water) = be_u32.parse(after)?;
        let (after, flags) = be_u32.parse(after)?;
        rest = after;
        if ref_word0 == 0 {
            break;
        }
        extents.push(ExtentEntry {
            ref_word0,
            high_water,
            flags,
        });
    }

    Ok((
        rest,
        FileInfo {
            page_flag,
            signature,
            table_end_word,
            extents,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page1(words: &[u32]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for word in words {
            bytes.extend_from_slice(&word.to_be_bytes());
        }
        bytes.resize(2048, 0);
        bytes
    }

    /// `ams5100_0001`: never allocated an extent beyond the primary one.
    #[test]
    fn parse_page1_reads_empty_extent_table() {
        let info = FileInfo::parse_page1(&page1(&[1, u32::MAX, 2])).unwrap();
        assert_eq!(info.page_flag, 1);
        assert_eq!(info.signature, u32::MAX);
        assert_eq!(info.table_end_word, 2);
        assert!(info.extents.is_empty());
    }

    /// `ams7000_0001`: one extra extent. Its cursor word is 5, which the old
    /// reader rejected as a page size because 5 * 256 misses 2048.
    #[test]
    fn parse_page1_reads_single_extent() {
        let info = FileInfo::parse_page1(&page1(&[1, u32::MAX, 5, 23_384, 95, 0])).unwrap();
        assert_eq!(info.table_end_word, 5);
        assert_eq!(
            info.extents,
            vec![ExtentEntry {
                ref_word0: 23_384,
                high_water: 95,
                flags: 0,
            }]
        );
    }

    /// `ams7999_0001`: two extra extents, each 0x2000 above the previous.
    #[test]
    fn parse_page1_reads_multiple_extents() {
        let info =
            FileInfo::parse_page1(&page1(&[1, u32::MAX, 8, 24_383, 101_691, 0, 32_575, 1, 0]))
                .unwrap();
        assert_eq!(info.table_end_word, 8);
        assert_eq!(info.extents.len(), 2);
        assert_eq!(info.extents[0].ref_word0, 24_383);
        assert_eq!(info.extents[1].ref_word0, 32_575);
        assert_eq!(
            info.extents[1].ref_word0 - info.extents[0].ref_word0,
            0x2000
        );
    }

    /// A cursor that does not describe a whole number of entries is data we do
    /// not understand, not a reason to refuse the file.
    #[test]
    fn parse_page1_tolerates_unexpected_cursor() {
        let info = FileInfo::parse_page1(&page1(&[1, u32::MAX, 123, 24_383, 1, 0])).unwrap();
        assert_eq!(info.table_end_word, 123);
        assert_eq!(info.extents.len(), 1);
    }

    #[test]
    fn parse_page1_rejects_short_page() {
        let err = FileInfo::parse_page1(&[0u8; 8]).unwrap_err();
        assert!(matches!(
            err,
            E3dReaderError::InvalidPage { page_no: 1, .. }
        ));
    }

    /// `acpdflts\MDS-REPORT-HEADINGS` is a comment-headed text file that is
    /// extensionless and five pages long, so only the signature keeps it out.
    #[test]
    fn parse_page1_rejects_a_file_that_is_not_a_database() {
        let mut bytes = b"### MANDATORY NUMBER of FIELDS\r\n".repeat(64);
        bytes.resize(4096, b'#');
        let err = FileInfo::parse_page1(&bytes[2048..]).unwrap_err();
        assert!(matches!(
            err,
            E3dReaderError::InvalidPage { page_no: 1, .. }
        ));
    }

    #[test]
    fn extent_refs_lists_primary_first() {
        let descriptor = DbDescriptor {
            page_size: 2048,
            db_mark: 8000,
            session_page_no: 4,
            primary_extent_ref: 16_192,
            bucket_type_id: 0,
        };
        let info = FileInfo::parse_page1(&page1(&[1, u32::MAX, 5, 24_384, 26_261, 0])).unwrap();
        assert_eq!(info.extent_refs(&descriptor), vec![16_192, 24_384]);
    }
}
