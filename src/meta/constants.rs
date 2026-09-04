/// E3D 3.1 magic constants recovered from IDA decompile.
///
/// WARNING: These values are specific to E3D 3.1.
/// E3D 2.10 uses different values (e.g. 7618321 instead of 7618377).
///
/// The release-bound ones are also carried on [`crate::schema::E3D31`], which
/// is where a second release would add its own; the values live here so the
/// read path can reach them without a schema pack being open.

/// The tree identity `sub_5AFB660` checks a type-5 page's second word
/// against.
///
/// NOT the element data pages: element records live on pages whose first
/// word is 7 (`crate::page::PageType::ElementData`), and no live record part
/// touches a `(5, 7618377)` page anywhere in the 429-file corpus — every
/// file holds some, and the live index tree references none of them
/// (`docs/evidence/2026-08-30-e3d-io-data-page-header.md`).
///
/// Research item R1 is now closed
/// (`docs/evidence/2026-08-30-e3d-io-data-table-tree-7618377.md`): this is a
/// copy-on-write refno-keyed B+tree rooted at the session page's 0x24 word
/// (the field this crate currently names `SessionPage::claim`), mapping each
/// **primary element** (the name-bearing, navigable hierarchy nouns —
/// SITE/ZONE/EQUI/PIPE/BRAN/STRU/… — every live one of them, and none of the
/// component/primitive/detail nouns beneath them) to `(0, session_number)`.
/// 429/429 files partition cleanly by noun with no split. `sub_5AFB660` reads it.
pub const DATA_PAGE_MAIN: u32 = 7_618_377;

/// The RefNo index tree's identity: the second word of its type-5 pages.
/// Evidence: sub_5AEB6B0: sub_5B01340(dbno, 13387743, ...), corpus-verified
/// on 17 295 pages.
pub const DATA_PAGE_INDEX: u32 = 13_387_743;

/// Property ID: page count query.
/// Evidence: sub_5AF8230: if (a3 == 223355190)
pub const PROP_PAGE_COUNT: u32 = 223_355_190;

/// Property ID: page size.
/// Evidence: E3D_数据库文件架构完整分析.md descriptor offset +20
pub const PROP_PAGE_SIZE: u32 = 107_487_840;

/// Property ID: database mark.
/// Evidence: doc descriptor offset +32
pub const PROP_DB_MARK: u32 = 223_453_451;

/// Property ID: session page number.
/// Evidence: doc descriptor offset +40
pub const PROP_SESSION_PAGE: u32 = 223_594_302;

/// Property ID: total pages (alternate location in descriptor).
/// Evidence: doc descriptor offset +364
pub const PROP_TOTAL_PAGES_ALT: u32 = 11_438_445;

/// Property ID: bucket type identifier.
/// Evidence: doc descriptor offset +16
pub const PROP_BUCKET_TYPE: u32 = 66_156_832;

// There is deliberately no default page size here.
//
// The constant that stood at this spot was 4096, and every sampled database
// states 2048 — so the fail-open it backed did not merely guess, it guessed
// exactly double, and a file that landed on it read every pair of pages as
// one. The page size now comes from the file or the open fails:
// `PageSize::from_header_words`.

/// Database files use BIG-ENDIAN byte order (Fortran legacy convention).
/// All integer fields are read with from_be_bytes, not from_le_bytes.
pub const FILE_BYTE_ORDER: &str = "big-endian";

/// Minimum valid database file size (at least descriptor + file_info pages).
pub const MIN_FILE_SIZE: u64 = 256;
