use super::DbDescriptor;
use crate::error::E3dReaderError;

impl DbDescriptor {
    pub const PAGE_INDEX: u32 = 0;

    /// Parse descriptor from page 0 raw bytes.
    ///
    /// Layout verified against pdmsdb_engine_v2 HeaderView::from_bytes:
    /// - +0x04 (4):  version
    /// - +0x08 (8):  db_num
    /// - +0x28 (40): latest_ses_pgno
    /// - +0x2C (44): ext_no
    /// - +0x30 (48): session_page_no
    /// - +0x34 (52): page_size
    /// - +0x38 (56): primary_extent_ref
    ///
    /// +0x38 was read as a stored page count until a census showed it is the
    /// `RefNo::word0` of the elements stored in the database's first extent:
    /// it decodes back to `db_mark` through `RefNo::dbno` with an extent index
    /// of 1, in all 448 databases of `AvevaMarineSample\ams000`. It is a refno
    /// prefix, not a length — `ams7330_0001` states 15522 and is 644 pages
    /// long.
    ///
    /// Byte order: BIG-ENDIAN (Fortran legacy).
    pub fn parse(data: &[u8]) -> Result<Self, E3dReaderError> {
        if data.len() < 60 {
            return Err(E3dReaderError::InvalidPage {
                page_no: 0,
                detail: format!("descriptor page too small: {} bytes", data.len()),
            });
        }
        let r = |off: usize| u32::from_be_bytes(data[off..off + 4].try_into().unwrap());
        Ok(Self {
            bucket_type_id: r(0x00),
            page_size: r(0x34).saturating_mul(4), // stored in 4-byte words, convert to bytes
            db_mark: r(0x08),
            session_page_no: r(0x30),
            primary_extent_ref: r(0x38),
        })
    }
}
