pub mod constants;
pub mod descriptor;

use crate::error::E3dReaderError;

use nom::number::complete::be_u32;
use nom::IResult;

/// Database descriptor parsed from page 0.
///
/// Field offsets and the evidence for each are in [`DbDescriptor::parse`].
#[derive(Debug, Clone)]
pub struct DbDescriptor {
    /// Page size in **bytes**. Derived, not stored: the header counts words.
    pub page_size: u32,
    /// Page size exactly as the header states it, in 4-byte words.
    pub page_size_words: u32,
    /// `db_num` at +0x08. Named `db_mark` since before the field was
    /// identified; the two are the same number.
    pub db_mark: u32,
    /// Noun of page 0, at +0x04.
    pub noun: u32,
    /// db1 hash of the database type name (`DESI`/`CATA`/…), at +0x20.
    /// [`DbDescriptor::db_type_name`] turns it back into the name.
    pub db_type_hash: u32,
    /// Signed template version at +0x24.
    ///
    /// `core.dll` compares its absolute value with word 3 of the matching
    /// `*vir.dat` template header when opening a database. It is a schema
    /// compatibility value, not an E3D product release.
    pub template_version: i32,
    /// Page of the **newest** session, at +0x28. This is the chain tail that
    /// `open` starts from.
    pub latest_ses_pgno: u32,
    /// Extent number of this file, at +0x2C.
    pub ext_no: u32,
    /// Page of the **oldest** session, at +0x30 — not the current one. Kept
    /// under its original name because callers outside this module read it.
    pub session_page_no: u32,
    /// `RefNo::word0` of the database's first extent, i.e. `db_mark + 0x2000`.
    pub primary_extent_ref: u32,
    /// Page type word at +0x00, 0 for a descriptor page. Named
    /// `bucket_type_id` before the page header layout was known.
    pub bucket_type_id: u32,
    /// Packed ASCII `CRINFO` from +0x40/+0x44.
    ///
    /// This is the database creator banner (user, time, OS, product and build).
    /// It survives later opens and upgrades, so it describes creation rather
    /// than the client that currently reads or last wrote the file.
    pub creation_info: Option<String>,
    /// `FORMAT` at +0x154 when the caller supplied the complete descriptor
    /// page. Short prefix probes leave it as `None`.
    ///
    /// The writer uses this value while converting packed text fields. It is
    /// not an E3D product release.
    pub header_format: Option<u32>,
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
    let declared_entries =
        table_end_word.saturating_sub(FileInfo::TABLE_START_WORD) / FileInfo::WORDS_PER_ENTRY;
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

    /// Page 0 of `ams8000_0001`, word for word.
    fn ams8000_page0() -> Vec<u8> {
        page1(&[
            0,
            2,
            8000,
            1,
            1,
            0,
            u32::MAX,
            0,
            722_578,
            u32::MAX,
            8897,
            1,
            4,
            512,
            16_192,
        ])
    }

    #[test]
    fn descriptor_reads_both_session_pointers_apart() {
        let desc = DbDescriptor::parse(&ams8000_page0()).unwrap();
        assert_eq!(desc.db_mark, 8000);
        assert_eq!(desc.noun, 2);
        assert_eq!(desc.db_type_hash, 722_578);
        assert_eq!(desc.db_type_name().as_deref(), Some("DESI"));
        assert_eq!(desc.template_version, -1);
        assert_eq!(desc.ext_no, 1);
        assert_eq!(desc.page_size_words, 512);
        assert_eq!(desc.page_size, 2048);
        assert_eq!(desc.primary_extent_ref, 16_192);
        // +0x28 is the newest session, +0x30 the oldest. Swapping them hands
        // back the database as it was at session 1.
        assert_eq!(desc.latest_ses_pgno, 8897);
        assert_eq!(desc.session_page_no, 4);
        assert_eq!(
            desc.latest_session_page(),
            crate::page::PageId::new(1, 8897)
        );
        assert_eq!(desc.first_session_page(), crate::page::PageId::new(1, 4));
    }

    #[test]
    fn descriptor_exposes_creator_template_and_format_as_diagnostics() {
        let mut page = ams8000_page0();
        let creator = b"dpc using AVEVA Administration Admin Mk1.4.0.3";
        let words = creator.len().div_ceil(4);
        page[0x40..0x44].copy_from_slice(&(words as u32).to_be_bytes());
        page[0x44..0x44 + creator.len()].copy_from_slice(creator);
        page[0x154..0x158].copy_from_slice(&1u32.to_be_bytes());

        let desc = DbDescriptor::parse(&page).unwrap();
        assert_eq!(desc.template_version, -1);
        assert_eq!(
            desc.creation_info.as_deref(),
            Some("dpc using AVEVA Administration Admin Mk1.4.0.3")
        );
        assert_eq!(desc.header_format, Some(1));
    }

    #[test]
    fn descriptor_prefix_probe_keeps_extended_diagnostics_optional() {
        let page = ams8000_page0();
        let desc = DbDescriptor::parse(&page[..0x3C]).unwrap();
        assert_eq!(desc.template_version, -1);
        assert_eq!(desc.creation_info, None);
        assert_eq!(desc.header_format, None);
    }

    #[test]
    fn descriptor_page_size_is_words_and_is_validated() {
        let mut page = ams8000_page0();
        page[0x34..0x38].copy_from_slice(&700u32.to_be_bytes());
        let err = DbDescriptor::parse(&page).unwrap_err();
        assert!(
            matches!(err, E3dReaderError::InvalidPage { page_no: 0, .. }),
            "{err}"
        );
    }

    /// Page 0 states its own type, and the parser now holds it to that: a
    /// session page handed in as a descriptor is refused, not reinterpreted.
    #[test]
    fn descriptor_page_type_is_validated() {
        let mut page = ams8000_page0();
        page[0x00..0x04].copy_from_slice(&3u32.to_be_bytes());
        let err = DbDescriptor::parse(&page).unwrap_err();
        assert!(
            matches!(err, E3dReaderError::InvalidPage { page_no: 0, .. }),
            "{err}"
        );
        assert!(err.to_string().contains("page type 3"), "{err}");
    }

    /// **Reverting `extent()` to the fallback turns this red.** An `ext_no`
    /// past `u16::MAX` used to become the primary extent without a word.
    #[test]
    fn descriptor_extent_past_u16_is_refused() {
        let mut page = ams8000_page0();
        page[0x2C..0x30].copy_from_slice(&0x1_0000u32.to_be_bytes());
        let err = DbDescriptor::parse(&page).unwrap_err();
        assert!(
            matches!(err, E3dReaderError::InvalidPage { page_no: 0, .. }),
            "{err}"
        );
        assert!(err.to_string().contains("65536"), "{err}");
    }

    #[test]
    fn extent_refs_lists_primary_first() {
        let mut page0 = ams8000_page0();
        page0[0x38..0x3C].copy_from_slice(&16_192u32.to_be_bytes());
        let descriptor = DbDescriptor::parse(&page0).unwrap();
        let info = FileInfo::parse_page1(&page1(&[1, u32::MAX, 5, 24_384, 26_261, 0])).unwrap();
        assert_eq!(info.extent_refs(&descriptor), vec![16_192, 24_384]);
    }
}
