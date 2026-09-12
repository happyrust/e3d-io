//! L1: the session chain, and opening the database as of one session.
//!
//! The index is copy-on-write — a page, once written, is never modified — so
//! **every session page carries the index root as it stood at that session**.
//! Opening "at session N" is therefore not a filter applied after the fact: it
//! is the choice of which root to descend from. That is also what makes
//! two-root differencing possible later.
//!
//! Two rules hold this module together:
//!
//! * `open` and `open_at` are one implementation. `open` is `open_at` asking
//!   for the chain tail, so there is no separate "latest" path that could drift
//!   from the "historical" one.
//! * A session that is not there is an `Err` naming it. The reader this
//!   replaces answered `None` for a missing session, a malformed session page
//!   and an unreadable header alike — and one level up, `None` reads as "that
//!   refno does not exist".

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::error::{E3dReaderError, SessionError};
use crate::meta::DbDescriptor;
use crate::page::cache::PageCache;
use crate::page::io::PageReader;
use crate::page::{IoStats, PageId, PageType};
use crate::record::text::TextFormat;

pub mod delta;
pub mod diff;

pub use delta::{
    AttributeRowChange, ElementDelta, ExplicitChange, ImplicitWordChange, OwnerChange,
};
pub use diff::{ChangeKind, ChangedElement, ChangedElements, SessionDiff};

/// Byte offsets inside a session page, all big-endian words.
///
/// `core.dll` addresses a session page by attribute hash: `sub_5AF8740` is the
/// integer table (hash → word), `sub_5AF83D0` the text / array table (IDB
/// `plant3/core.dll.i64`, 2026-09-12; the 2.1 `core.dll` has the same words
/// but no `UDATE` / `UTCSES`). The full map, with the name each word is
/// served under and what this crate reads it as:
///
/// | offset | word | `core.dll` | this crate |
/// |---|---|---|---|
/// | +0x00 | 0 | — | page type (3 = session) |
/// | +0x04 / +0x08 | 1 / 2 | `PREVSP` — page / extent of the previous session page | [`SessionPage::previous`]; `0xFFFFFFFF` or 0 ends the chain |
/// | +0x0C | 3 | `SESNO` | [`SessionPage::sesno`] |
/// | +0x10 | 4 | `OWNSES` — the owning extract's session this one derives from | [`SessionPage::reserved_10`], verbatim; `0xFFFFFFFF` on every sampled page, which are all master files |
/// | +0x14 / +0x18 | 5 / 6 | `LASTSP` — last page the session wrote (`EXFLSZ`, the extent file size in pages, is this + 1) | [`SessionPage::end`] |
/// | +0x1C / +0x20 | 7 / 8 | `INDEX` — root of the RefNo B+tree as of this session | [`SessionPage::index_root`] |
/// | +0x24 / +0x28 | 9 / 10 | `CLAIM` — root of the claim tree | [`SessionPage::claim`], carried and not walked |
/// | +0x2C | 11 | `PRISES` — primary session | *not read* |
/// | +0x30 / +0x34.. | 12 / 13..16 | `DATE` — word count (4) and `year, month, day×24+hour, minute×60+second`, local clock | [`SessionPage::date`]; the raw words on [`SessionPage::stamp_words`] |
/// | +0x50.. | 20.. | `TRANSA` — an array the text table serves from word 20; its length rule is not decoded | *not read* |
/// | +0x64 / +0x68.. | 25 / 26..29 | `UDATE` — the same four words in UTC, fourth in milliseconds; `UTCSES` is "the count is 4" (3.1 only) | [`SessionPage::utc_date`] |
/// | +0x78 / +0x7C.. | 30 / 31..39 | `USER` — word count and text | [`SessionPage::user`], decoded under the header's `FORMAT` |
/// | +0xA0 / +0xA4.. | 40 / 41.. | `DESC` — word count and text | [`SessionPage::comment`], decoded under the header's `FORMAT` |
/// | +0x31C / +0x320.. | 199 / 200.. | table directory: count, then `(name_hash, root_pgno, root_extno)` triples | [`SessionPage::tables`] |
/// | +0x7DC | 503..504 | eight bytes of the `DBVRSN` engine tag after its `DAB` prefix | [`SessionPage::dabacon_version_tag`] |
/// | +0x7E4 | 505 | `RUNUM` | [`SessionTail::runum`] |
/// | +0x7E8 | 506 | `MEMEXT` | [`SessionTail::memext`] |
/// | +0x7EC | 507 | `MEMPPS` | [`SessionTail::mempps`] |
/// | +0x7F0 | 508 | `BASESN` — the session this one was based on; 0 = none | [`SessionTail::base_sesno`] |
/// | +0x7F4 | 509 | `REASON` — code of the message (group 43, 460 + code) `DB_DB::reason` shows | [`SessionTail::reason`] |
/// | +0x7F8 | 510 | `LINKPS` | [`SessionTail::linkps`] |
///
/// Three names in the integer table are not stored words but derived on the
/// spot: `SPGNO` is the page's own number, `PRVSES` is word 3 of the page
/// `PREVSP` names, and `FRSTSP` — the first page this session wrote — is the
/// previous page's `LASTSP` + 1, or this page's own `LASTSP` when there is no
/// previous session.
mod at {
    pub const PAGE_TYPE: usize = 0x00;
    pub const LAST_SES_PGNO: usize = 0x04;
    pub const LAST_SES_EXTNO: usize = 0x08;
    pub const SESNO: usize = 0x0C;
    pub const RESERVED_10: usize = 0x10;
    pub const END_PGNO: usize = 0x14;
    pub const END_EXTNO: usize = 0x18;
    pub const INDEX_ROOT_PGNO: usize = 0x1C;
    pub const INDEX_ROOT_EXTNO: usize = 0x20;
    pub const CLAIM_PGNO: usize = 0x24;
    pub const CLAIM_EXTNO: usize = 0x28;
    /// `DATE`: word count at +0x30 (always 4 when stated), the four words at
    /// +0x34 — `year, month, day * 24 + hour, minute * 60 + second`, local
    /// wall clock. `core.dll` reads it through `DB_DB::GetSessionDateTime` →
    /// `DB_DateTime::set(year, month, hours, milliseconds, kind)`, which
    /// splits the third word with `DB_TimeSpan::fromHours` (`days()` →
    /// `wDay`, `hours()` → `wHour`) and the fourth, scaled to milliseconds,
    /// with `fromMilliSeconds` (`minutes()`, `seconds()`, `milliseconds()`).
    pub const DATE_LEN: usize = 0x30;
    pub const DATE: usize = 0x34;
    pub const DATE_WORDS: usize = 4;
    /// `UDATE`: the same four words in UTC with the fourth in
    /// **milliseconds**, count at +0x64 (`UTCSES` is that count being 4) and
    /// words at +0x68. E3D 3.1 writes it; 2.1 states a count of 0.
    pub const UDATE_LEN: usize = 0x64;
    pub const UDATE: usize = 0x68;
    /// First word of the raw stamp region kept on [`super::SessionPage::stamp_words`]:
    /// the four `DATE` words and the word after them.
    pub const STAMP: usize = DATE;
    pub const STAMP_WORDS: usize = 5;
    /// `USER` length, counted in 4-byte words — the same shape as the comment.
    /// `core.dll` serves this field as the session attribute `USER`
    /// (`sub_5AF83D0`: count at word 30, text from word 31).
    pub const USER_LEN: usize = 0x78;
    pub const USER_TEXT: usize = 0x7C;
    /// Words 31..39 are all the room `USER` has before `DESC`'s length word at
    /// +0xA0; a longer stated count is clamped rather than read into the comment.
    pub const USER_MAX_WORDS: usize = (COMMENT_LEN - USER_TEXT) / 4;
    /// Comment length, counted in 4-byte words. `core.dll` calls this `DESC`.
    pub const COMMENT_LEN: usize = 0xA0;
    pub const COMMENT_TEXT: usize = 0xA4;
    /// How many named tables the directory that follows declares.
    pub const TABLE_COUNT: usize = 0x31C;
    /// `(name_hash, root_pgno, root_extno)` triples, [`TABLE_COUNT`] of them.
    pub const TABLE_DIR: usize = 0x320;
    pub const TABLE_ENTRY_BYTES: usize = 12;
    /// Eight bytes copied from `DBVRSN`, beginning after its `DAB` prefix.
    pub const DABACON_VERSION_TAG: usize = 0x7DC;
    pub const DABACON_VERSION_TAG_BYTES: usize = 8;
    /// The six bookkeeping words that close a 2048-byte session page, right
    /// behind the DAB tag: words 505..510. Named after the session attributes
    /// `core.dll`'s `sub_5AF8740` serves them under.
    pub const RUNUM: usize = 0x7E4;
    pub const MEMEXT: usize = 0x7E8;
    pub const MEMPPS: usize = 0x7EC;
    pub const BASESN: usize = 0x7F0;
    pub const REASON: usize = 0x7F4;
    pub const LINKPS: usize = 0x7F8;
    /// A page must reach past `LINKPS` to state a [`super::SessionTail`].
    pub const TAIL_END: usize = LINKPS + 4;
    /// Everything through the comment length word must be present.
    pub const MIN_BYTES: usize = COMMENT_TEXT;
}

/// A session page marks the chain head by pointing nowhere.
///
/// `ams8000_0001` page 4 is session 1 and states `0xFFFFFFFF`. Page 0 is the
/// descriptor and can never be a session page, so 0 ends the chain too.
const NO_PREVIOUS: [u32; 2] = [u32::MAX, 0];

/// One named B+tree the session page points at.
///
/// A dabacon file holds more than the refno index. Two trees have dedicated
/// pointers — the index at +0x1C and `CLAIM` at +0x24 — and the rest are listed
/// in a directory at +0x31C, each named by the base-27 hash of its name and
/// rooted at a `(pgno, extno)` pair. Across the 543-file corpus every one of
/// the 2071 directory entries points at a type-5 page whose own second header
/// word repeats the name hash, and every stated extent is 1.
///
/// The names that turn up are `NAME`, `TYPEX`, `SPAMAP`, the UDA tables — and,
/// one table per reference attribute, `SPRE` / `CATR` / `PRTREF` / `GOBREF` /
/// … : those are the reverse-reference tables core.dll searches through
/// `DB_RefTableIterator::startSearch(dbno, attId, targetRef, handle)`, where
/// `attId` picks the table and `targetRef` is the key. See
/// [`crate::index::reference`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableRoot {
    /// Base-27 hash of the table's name, the same hash the table's own pages
    /// carry in their second header word.
    pub name_hash: u32,
    pub root: PageId,
}

impl TableRoot {
    /// The table's name, when the hash decodes as a base-27 name.
    pub fn name(&self) -> Option<String> {
        e3d_attlib::db1_dehash(self.name_hash)
    }
}

/// One session page: what the database looked like when that session closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPage {
    /// Where this page lives.
    pub page: PageId,
    /// Session number, +0x0C.
    pub sesno: u32,
    /// The session before this one, or `None` at the head of the chain.
    pub previous: Option<PageId>,
    /// +0x10 verbatim. Reads `0xFFFFFFFF` on every sampled session page, so
    /// whatever it is, it is not a page number. Carried only because
    /// [`SessionInfo::start_page_no`] has been reporting it for years.
    pub reserved_10: u32,
    /// Last page the session wrote, +0x14 / +0x18.
    pub end: PageId,
    /// **Root of the index as of this session**, +0x1C / +0x20.
    pub index_root: PageId,
    /// Claim page, +0x24 / +0x28.
    pub claim: PageId,
    /// **The other trees as of this session**, +0x31C count and +0x320 entries.
    ///
    /// Copy-on-write like the index root, so these roots are session-pinned
    /// too: a view opened at session N sees that session's reference tables.
    pub tables: Vec<TableRoot>,
    /// User who wrote the session, +0x7C, length at +0x78 counted in 4-byte
    /// words — `core.dll`'s session attribute `USER`.
    ///
    /// This field was called `machine` and read as NUL-padded ASCII up to the
    /// comment. It is the name E3D recorded for whoever saved the session —
    /// `dpc`, `80404`, `happyrust`, `YW`, but also host-shaped logins such as
    /// `pdms-bj-c103-47` on the sampled databases — and the word at +0x78
    /// states exactly its length in words, as the comment's does at +0xA0
    /// (`dpc` → 1, `80404` → 2, `happyrust` → 3).
    pub user: String,
    /// Session comment, +0xA4, length at +0xA0 counted in 4-byte words.
    /// `core.dll` calls it `DESC`.
    pub comment: String,
    /// When the session was written, local wall clock of the machine that
    /// wrote it — `core.dll`'s `DATE` (+0x30 count, +0x34 words). `None` when
    /// the page does not state exactly four words there, which is the
    /// condition `DB_DB::GetSessionDateTime` puts on it too.
    pub date: Option<SessionDate>,
    /// The same instant in UTC with milliseconds — `UDATE` (+0x64 count,
    /// +0x68 words), which E3D 3.1 writes and 2.1 does not. `core.dll`
    /// prefers it over `date` whenever it is stated and the project's
    /// `outputUTCSessionDateTime` switch is on (`DB_DB::canGetUTCDateTimeForSession`).
    pub utc_date: Option<SessionDate>,
    /// Words +0x34..+0x48 verbatim: the four `DATE` words and the word after
    /// them. Kept for callers that decode the stamp themselves; new code
    /// should read [`Self::date`] / [`Self::utc_date`].
    pub stamp_words: [u32; at::STAMP_WORDS],
    /// DAB database-engine build tag at +0x7DC, when the complete session page
    /// is available.
    ///
    /// `core.dll` writes eight bytes from `DBVRSN` into every new session:
    /// observed examples include `2_28_19_` and `2_35_23o`. This identifies
    /// the engine that wrote this session, not the E3D release that should be
    /// used to select a SchemaPack.
    pub dabacon_version_tag: Option<String>,
    /// The six bookkeeping words at +0x7E4..+0x7F8, when the complete session
    /// page is available. See [`SessionTail`].
    pub tail: Option<SessionTail>,
}

/// A session's timestamp as the page stores it: civil fields, no zone.
///
/// Decoded the way `core.dll` decodes the four `DATE` / `UDATE` words
/// (`DB_DB::GetSessionDateTime` → `DB_DateTime::set(year, month, hours,
/// milliseconds, kind)`):
///
/// | word | holds | split |
/// |---|---|---|
/// | 0 | year | as is |
/// | 1 | month | as is |
/// | 2 | `day * 24 + hour` | `DB_TimeSpan::fromHours(w).days()` / `.hours()` |
/// | 3 | `DATE`: `minute * 60 + second`; `UDATE`: the same in milliseconds | `fromMilliSeconds(ms).minutes()` / `.seconds()` / `.milliseconds()` |
///
/// `ams8000_0001` of E3D 3.1, session 444: `DATE` `2026, 9, 216, 1325` is
/// 2026-09-09 00:22:05 local and `UDATE` `2026, 9, 208, 1325919` is
/// 2026-09-08 16:22:05.919 UTC — eight hours apart, as the machine's zone
/// says, with the same seconds. `core.dll` itself drops `UDATE`'s
/// milliseconds (`fromMilliSeconds(w).totalSeconds()` before the split);
/// this type keeps them.
///
/// Which zone the fields are in is the page's business: [`SessionPage::date`]
/// is the writing machine's local clock, [`SessionPage::utc_date`] is UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionDate {
    pub year: u32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    /// Only `UDATE` carries these; `DATE` always states 0.
    pub millisecond: u32,
}

/// How the fourth date word is scaled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FourthWord {
    Seconds,
    Milliseconds,
}

impl SessionDate {
    /// Split the four stored words the way `DB_DateTime::set` does. No range
    /// check: the page's statement is handed over, and [`Self::is_plausible`]
    /// says whether it reads as a calendar date.
    fn from_words(words: [u32; at::DATE_WORDS], fourth: FourthWord) -> Self {
        let [year, month, hours, fourth_word] = words;
        let milliseconds = match fourth {
            FourthWord::Seconds => u64::from(fourth_word) * 1000,
            FourthWord::Milliseconds => u64::from(fourth_word),
        };
        Self {
            year,
            month,
            day: hours / 24,
            hour: hours % 24,
            minute: (milliseconds / 60_000) as u32,
            second: ((milliseconds / 1000) % 60) as u32,
            millisecond: (milliseconds % 1000) as u32,
        }
    }

    /// Read `DATE` (`fourth` = seconds) or `UDATE` (`fourth` = milliseconds)
    /// from a page: the count word must state exactly four words, as
    /// `DB_DB::GetSessionDateTime` requires (`(end - begin) & !3 == 0x10`).
    fn read(data: &[u8], len_at: usize, words_at: usize, fourth: FourthWord) -> Option<Self> {
        let r = |off: usize| -> Option<u32> {
            Some(u32::from_be_bytes(data.get(off..off + 4)?.try_into().ok()?))
        };
        if r(len_at)? != at::DATE_WORDS as u32 {
            return None;
        }
        let mut words = [0u32; at::DATE_WORDS];
        for (i, word) in words.iter_mut().enumerate() {
            *word = r(words_at + i * 4)?;
        }
        Some(Self::from_words(words, fourth))
    }

    /// Whether the fields read as a calendar date and time of day. A page
    /// that states `DATE` as four zero words, or a count of 4 over garbage,
    /// parses but is not plausible.
    pub fn is_plausible(&self) -> bool {
        (1..=12).contains(&self.month)
            && (1..=31).contains(&self.day)
            && self.hour < 24
            && self.minute < 60
            && self.second < 60
            && self.millisecond < 1000
            && (1970..=9999).contains(&self.year)
    }

    /// Seconds since 1970-01-01T00:00:00 **treating the fields as UTC**.
    ///
    /// Exact for [`SessionPage::utc_date`]. For [`SessionPage::date`] it is
    /// the local wall clock re-labelled as UTC, so the difference between the
    /// two is the writing machine's zone offset — 28 800 on every 3.1 sample,
    /// which is what [`SessionPage::zone_offset_seconds`] reports.
    pub fn unix_seconds_as_utc(&self) -> i64 {
        days_from_civil(i64::from(self.year), self.month, self.day) * 86_400
            + i64::from(self.hour) * 3600
            + i64::from(self.minute) * 60
            + i64::from(self.second)
    }
}

impl fmt::Display for SessionDate {
    /// `2026-09-08T16:22:05.919`, or without the fraction when it is zero.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )?;
        if self.millisecond != 0 {
            write!(f, ".{:03}", self.millisecond)?;
        }
        Ok(())
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`), for a crate that does not carry a calendar dependency.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_from_march = (i64::from(month) + 9) % 12;
    let day_of_year = (153 * month_from_march + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The six words `core.dll` keeps at the very end of a session page, words
/// 505..510 (+0x7E4..+0x7F8), right behind the DAB version tag.
///
/// `sub_5AF8740` in `core.dll` 3.1 serves them as the session attributes
/// `RUNUM`, `MEMEXT`, `MEMPPS`, `BASESN`, `REASON` and `LINKPS`; 2.1's
/// `core.dll` carries the same attribute hashes. The names are kept as they
/// are because only two of the six have a meaning this crate can state:
///
/// - `BASESN` is the session this one was written on top of. Every plain
///   `SAVEWORK` sampled states the previous session number (`ams7000_0001`
///   263 → 262, `ams1112_0001` 1020 → 1019, E3D 2.1 `ams8000_0001` 19 → 18).
///   After `MERGE CHANGES` it can name a session the chain no longer retains,
///   which is exactly what makes it worth keeping next to `previous`.
/// - `REASON` says why the session exists. `DB_DB::reason(sesno, string&)`
///   renders it through message group 43 as `460 + code`, so the text lives in
///   the AVEVA message catalogue, not in this crate; every plain `SAVEWORK`
///   sampled states 0, and the `Saving Meta Data` session E3D 3.1 writes
///   states 15.
///
/// `RUNUM`, `MEMEXT`, `MEMPPS` and `LINKPS` are handed over verbatim: their
/// names are `core.dll`'s, their meaning is not pinned. Sampled values are
/// `RUNUM` 1..43, `MEMEXT` 0 or 1, the other two 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SessionTail {
    /// `RUNUM`, +0x7E4.
    pub runum: u32,
    /// `MEMEXT`, +0x7E8.
    pub memext: u32,
    /// `MEMPPS`, +0x7EC.
    pub mempps: u32,
    /// `BASESN`, +0x7F0: the session this one was based on; 0 when none.
    pub base_sesno: u32,
    /// `REASON`, +0x7F4: why the session was created, as `core.dll`'s code.
    pub reason: u32,
    /// `LINKPS`, +0x7F8.
    pub linkps: u32,
}

impl SessionTail {
    /// Read the six words, or `None` when the page ends before them.
    fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < at::TAIL_END {
            return None;
        }
        let r = |off: usize| u32::from_be_bytes(data[off..off + 4].try_into().unwrap());
        Some(Self {
            runum: r(at::RUNUM),
            memext: r(at::MEMEXT),
            mempps: r(at::MEMPPS),
            base_sesno: r(at::BASESN),
            reason: r(at::REASON),
            linkps: r(at::LINKPS),
        })
    }
}

impl SessionPage {
    /// [`SessionPage::parse_as`] without the header's `FORMAT` word: `USER`
    /// and `DESC` are guessed at from their bytes ([`TextFormat::Detect`]).
    pub fn parse(page: PageId, data: &[u8]) -> Result<Self, SessionError> {
        Self::parse_as(page, data, TextFormat::Detect)
    }

    /// Parse a session page, rejecting anything that is not one, and decode
    /// its two text fields the way the database's header says text is stored.
    pub fn parse_as(page: PageId, data: &[u8], text: TextFormat) -> Result<Self, SessionError> {
        if data.len() < at::MIN_BYTES {
            return Err(SessionError::PageTooSmall {
                page,
                len: data.len(),
                need: at::MIN_BYTES,
            });
        }
        let r = |off: usize| u32::from_be_bytes(data[off..off + 4].try_into().unwrap());

        let page_type = r(at::PAGE_TYPE);
        if PageType::from_raw(page_type) != PageType::Session {
            return Err(SessionError::NotASessionPage { page, page_type });
        }

        let last_pgno = r(at::LAST_SES_PGNO);
        let previous = if NO_PREVIOUS.contains(&last_pgno) {
            None
        } else {
            Some(address(
                page,
                "previous session",
                r(at::LAST_SES_EXTNO),
                last_pgno,
            )?)
        };

        let mut stamp_words = [0u32; at::STAMP_WORDS];
        for (i, word) in stamp_words.iter_mut().enumerate() {
            *word = r(at::STAMP + i * 4);
        }

        Ok(Self {
            page,
            sesno: r(at::SESNO),
            previous,
            reserved_10: r(at::RESERVED_10),
            end: address(page, "end", r(at::END_EXTNO), r(at::END_PGNO))?,
            index_root: address(
                page,
                "index root",
                r(at::INDEX_ROOT_EXTNO),
                r(at::INDEX_ROOT_PGNO),
            )?,
            claim: address(page, "claim", r(at::CLAIM_EXTNO), r(at::CLAIM_PGNO))?,
            tables: table_directory(page, data)?,
            user: word_text(
                data,
                at::USER_TEXT,
                r(at::USER_LEN),
                at::USER_MAX_WORDS,
                text,
            ),
            comment: comment(data, r(at::COMMENT_LEN), text),
            date: SessionDate::read(data, at::DATE_LEN, at::DATE, FourthWord::Seconds),
            utc_date: SessionDate::read(data, at::UDATE_LEN, at::UDATE, FourthWord::Milliseconds),
            stamp_words,
            dabacon_version_tag: data
                .get(
                    at::DABACON_VERSION_TAG
                        ..at::DABACON_VERSION_TAG + at::DABACON_VERSION_TAG_BYTES,
                )
                .map(ascii_field)
                .map(|tag| tag.trim_end().to_owned())
                .filter(|tag| !tag.is_empty()),
            tail: SessionTail::parse(data),
        })
    }

    /// The writing machine's zone offset in seconds east of UTC, when the page
    /// states both clocks: `date - utc_date`, to the second. `28_800` on every
    /// E3D 3.1 sample written in UTC+8; `None` on 2.1 pages, which have no
    /// `UDATE`.
    pub fn zone_offset_seconds(&self) -> Option<i64> {
        Some(self.date?.unix_seconds_as_utc() - self.utc_date?.unix_seconds_as_utc())
    }

    /// Root of the named table as of this session.
    ///
    /// `name_hash` is the base-27 hash of the table name — `db1_hash("SPRE")`
    /// for the table of everything referencing through `SPRE`.
    pub fn table_root(&self, name_hash: u32) -> Option<PageId> {
        self.tables
            .iter()
            .find(|table| table.name_hash == name_hash)
            .map(|table| table.root)
    }
}

/// The named-table directory at +0x31C.
///
/// A declared count that would run past the page is refused rather than
/// truncated: the count and the entries are the same fact stated twice, and a
/// reader that trims one of them is inventing a directory the file does not
/// state.
///
/// A page that ends before +0x31C has no directory to state — the offset is
/// past the page the file's own header sized. Every dabacon page of the
/// 543-file corpus is 2048 bytes, so this arm belongs to the synthetic
/// 256-byte fixtures in `tests/extent_attach_contract.rs` and not to any real
/// database.
fn table_directory(page: PageId, data: &[u8]) -> Result<Vec<TableRoot>, SessionError> {
    if data.len() < at::TABLE_COUNT + 4 {
        return Ok(Vec::new());
    }
    let r = |off: usize| u32::from_be_bytes(data[off..off + 4].try_into().unwrap());
    let count = r(at::TABLE_COUNT) as usize;
    let need = at::TABLE_DIR + count * at::TABLE_ENTRY_BYTES;
    if need > data.len() {
        return Err(SessionError::TableDirectoryOverflow {
            page,
            count: count as u32,
            need,
            len: data.len(),
        });
    }

    let mut tables = Vec::with_capacity(count);
    for slot in 0..count {
        let at = at::TABLE_DIR + slot * at::TABLE_ENTRY_BYTES;
        tables.push(TableRoot {
            name_hash: r(at),
            root: address(page, "table root", r(at + 8), r(at + 4))?,
        });
    }
    Ok(tables)
}

/// Which session a caller wants.
///
/// `open` passes [`SessionSelector::Latest`] into the same `open_at` that a
/// caller naming a session number reaches, so there is exactly one resolver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionSelector {
    /// The newest session, i.e. the tail of the chain.
    Latest,
    /// One named session. Absent means `Err`, never a fallback to latest.
    Exact(u32),
    /// The newest retained session whose number is `<= n`.
    ///
    /// `MERGE CHANGES` drops sessions from the chain — `ams7000_0001` keeps
    /// only 1, 237, 238, 262 and 263 — so a downstream that recorded "I have
    /// everything up to session 200" cannot ask for 200 exactly. It wants the
    /// diff from the newest session that is not past its watermark, which on
    /// that file is session 1: everything since then is re-reported rather
    /// than the changes in 201..236, which were folded into 237, being lost.
    /// The session actually chosen is always reported back, never silent.
    ///
    /// Resolved as the first session the newest-first walk reaches whose
    /// number is `<= n`, which is the newest such session on a chain whose
    /// numbers descend — every chain of the 453-file corpus does.
    AtOrBefore(u32),
}

impl SessionSelector {
    /// The number the selector names, if it names one.
    pub const fn sesno(self) -> Option<u32> {
        match self {
            Self::Latest => None,
            Self::Exact(sesno) | Self::AtOrBefore(sesno) => Some(sesno),
        }
    }
}

impl From<u32> for SessionSelector {
    fn from(sesno: u32) -> Self {
        Self::Exact(sesno)
    }
}

impl From<SessionSelector> for Option<u32> {
    fn from(selector: SessionSelector) -> Self {
        selector.sesno()
    }
}

/// A database opened as of one session.
///
/// Holds the page source, the file header and the chosen session page. What
/// makes it a *view* rather than a handle is [`DbView::index_root`]: two views
/// of the same file at different sessions descend different trees, and neither
/// is more current than the file says it is.
pub struct DbView {
    pages: PageCache,
    descriptor: DbDescriptor,
    session: SessionPage,
    sessions_read: u32,
    /// Kept for error messages: a session that is not there is named together
    /// with the file it is not in.
    path: PathBuf,
}

/// Names the session and the root, which is what tells two views apart.
impl std::fmt::Debug for DbView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbView")
            .field("db_num", &self.descriptor.db_mark)
            .field("sesno", &self.session.sesno)
            .field("index_root", &self.session.index_root)
            .field("sessions_read", &self.sessions_read)
            .finish_non_exhaustive()
    }
}

impl DbView {
    /// Pages held while reading. Matches `ReadOnlyEngine`'s default.
    pub const DEFAULT_CACHE_PAGES: usize = 256;

    /// Open at the newest session.
    ///
    /// Deliberately nothing but a call to [`DbView::open_at`]: a second
    /// "latest" path is how a reader ends up with two answers to the same
    /// question.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, E3dReaderError> {
        Self::open_at(path, SessionSelector::Latest)
    }

    /// Open as of a named session, or of the newest one.
    ///
    /// `open_at(path, 264)` and `open_at(path, SessionSelector::Latest)` both
    /// work; a `u32` converts into [`SessionSelector::Exact`].
    pub fn open_at(
        path: impl AsRef<Path>,
        at: impl Into<SessionSelector>,
    ) -> Result<Self, E3dReaderError> {
        Self::open_at_with_cache(path, at, Self::DEFAULT_CACHE_PAGES)
    }

    pub fn open_at_with_cache(
        path: impl AsRef<Path>,
        at: impl Into<SessionSelector>,
        cache_pages: usize,
    ) -> Result<Self, E3dReaderError> {
        let path = path.as_ref();
        let mut pages = PageCache::new(PageReader::open(path)?, cache_pages);

        let descriptor = DbDescriptor::parse(pages.get_page(DbDescriptor::PAGE_INDEX)?)?;
        let (session, sessions_read) = resolve(&mut pages, &descriptor, at.into(), path)?;

        Ok(Self {
            pages,
            descriptor,
            session,
            sessions_read,
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What changed between two sessions of this file: see
    /// [`diff::diff_between`], which this delegates to without adding anything.
    ///
    /// Both selectors are resolved on one walk of the chain; `from` older than
    /// `to` is the only accepted order, and either being absent is an error
    /// naming the retained sessions either side of it.
    pub fn diff_sessions(
        &mut self,
        from: impl Into<SessionSelector>,
        to: impl Into<SessionSelector>,
    ) -> Result<SessionDiff, E3dReaderError> {
        diff::diff_between(
            &mut self.pages,
            &self.descriptor,
            &self.path,
            from.into(),
            to.into(),
        )
    }

    /// The same range, one retained session at a time: see [`diff::diff_steps`].
    pub fn diff_steps(
        &mut self,
        from: impl Into<SessionSelector>,
        to: impl Into<SessionSelector>,
    ) -> Result<Vec<SessionDiff>, E3dReaderError> {
        diff::diff_steps(
            &mut self.pages,
            &self.descriptor,
            &self.path,
            from.into(),
            to.into(),
        )
    }

    /// The records behind a diff, read through this view's page cache. See
    /// [`SessionDiff::elements`].
    pub fn changed_elements<'a>(&'a mut self, diff: &'a SessionDiff) -> ChangedElements<'a> {
        diff.elements(&mut self.pages)
    }

    /// The session actually in use — never a guess, never a fallback.
    pub fn selected_session(&self) -> u32 {
        self.session.sesno
    }

    /// Root of the index as of [`DbView::selected_session`].
    pub fn index_root(&self) -> PageId {
        self.session.index_root
    }

    /// The named tables as of [`DbView::selected_session`].
    pub fn tables(&self) -> &[TableRoot] {
        &self.session.tables
    }

    /// Root of one named table as of [`DbView::selected_session`].
    pub fn table_root(&self, name_hash: u32) -> Option<PageId> {
        self.session.table_root(name_hash)
    }

    pub fn session(&self) -> &SessionPage {
        &self.session
    }

    pub fn descriptor(&self) -> &DbDescriptor {
        &self.descriptor
    }

    /// How this database stores text, from the header's `FORMAT` word; see
    /// [`crate::record::text`]. Records read through this view decode under it.
    pub fn text_format(&self) -> TextFormat {
        TextFormat::from_header(self.descriptor.header_format)
    }

    /// Session pages read while opening: 1 for the newest, one more per step
    /// back. Also visible as demand reads in [`DbView::stats`].
    pub fn sessions_read(&self) -> u32 {
        self.sessions_read
    }

    pub fn stats(&self) -> IoStats {
        self.pages.stats()
    }

    pub fn pages(&self) -> &PageCache {
        &self.pages
    }

    pub fn pages_mut(&mut self) -> &mut PageCache {
        &mut self.pages
    }

    /// Every session, newest first. See [`walk_chain`].
    pub fn walk_sessions(&mut self) -> Result<Vec<SessionPage>, E3dReaderError> {
        walk_chain(&mut self.pages, &self.descriptor).map_err(Into::into)
    }
}

/// The chain, one session at a time, newest first.
///
/// The only chain traversal in the crate: [`walk_chain`], [`resolve`] and
/// [`resolve_pair`] all drive one of these, so "which pointer does the chain
/// start from" and "when is a chain broken" each have one answer. It starts
/// from `latest_ses_pgno` (+0x28). The hand-written walk this replaced started
/// from +0x30, which is the *oldest* session, and never followed
/// `last_ses_pgno` — on a 264-session database it returned one entry, for
/// session 1, and reported success.
///
/// A chain cannot be longer than the file has pages, and cannot revisit a
/// page: `last_ses_pgno` pointing at itself used to spin forever. Both are
/// errors here, not silent stops.
///
/// Costs one page read per session, all of them demand reads, so the price
/// shows up in `IoStats::pages_read`.
pub struct ChainWalk<'a> {
    pages: &'a mut PageCache,
    cursor: ChainCursor,
}

impl<'a> ChainWalk<'a> {
    pub fn new(pages: &'a mut PageCache, descriptor: &DbDescriptor) -> Self {
        let cursor = ChainCursor::new(pages, descriptor);
        Self { pages, cursor }
    }

    /// The next session back, or `None` once the chain head has been passed.
    pub fn next_session(&mut self) -> Result<Option<SessionPage>, SessionError> {
        self.cursor.advance(self.pages)
    }

    /// Session pages read so far.
    pub fn steps(&self) -> u32 {
        self.cursor.steps
    }
}

/// A chain walk between two steps, holding no page source.
///
/// [`ChainWalk`] is this plus the pages it reads through. They are apart so
/// that a walk can be **paused**: [`SessionChain`] stops at the session it was
/// asked for and picks the same walk up later from the same place, which a
/// walk borrowing the cache for its whole life cannot be stored to do.
#[derive(Debug, Clone)]
struct ChainCursor {
    next: Option<PageId>,
    seen: HashSet<PageId>,
    steps: u32,
    limit: u32,
    /// The header's `FORMAT` word, under which every page's `USER` and
    /// `DESC` are decoded — the same branch `db_get_file_session_string`
    /// takes before reading either.
    text_format: TextFormat,
}

impl ChainCursor {
    fn new(pages: &PageCache, descriptor: &DbDescriptor) -> Self {
        Self {
            next: Some(descriptor.latest_session_page()),
            seen: HashSet::new(),
            steps: 0,
            limit: pages.physical_page_count().max(1),
            text_format: TextFormat::from_header(descriptor.header_format),
        }
    }

    fn advance(&mut self, pages: &mut PageCache) -> Result<Option<SessionPage>, SessionError> {
        let Some(page) = self.next else {
            return Ok(None);
        };
        if !self.seen.insert(page) {
            return Err(SessionError::ChainCycle {
                revisited: page,
                after: self.steps,
            });
        }
        if self.steps >= self.limit {
            return Err(SessionError::ChainTooLong {
                steps: self.steps,
                limit: self.limit,
            });
        }
        let parsed = read_session(pages, page, self.text_format)?;
        self.next = parsed.previous;
        self.steps += 1;
        Ok(Some(parsed))
    }
}

/// The chain as far as it has been walked, newest first.
///
/// A [`DbView`] and a `ReadOnlyEngine` each keep one for the life of the
/// handle, so **a session page is read and parsed once**. Every resolve used
/// to start again at the newest session: `diff_sessions(250, 264)` on
/// `ams8000_0001` re-read the 32 session pages from 281 back to 250 on every
/// call, and [`diff::diff_steps`] paid for the range twice inside one call —
/// once resolving the two ends, once more listing the sessions between them,
/// and that second walk ran to session 1 rather than stopping at `from`.
///
/// The walk is newest-first and skips nothing, so what is held is always a
/// contiguous prefix of the chain. That is what makes [`SessionChain::between`]
/// free of reads: resolving the older end of a range has already walked past
/// every session in it.
pub struct SessionChain {
    known: Vec<SessionPage>,
    cursor: ChainCursor,
}

impl std::fmt::Debug for SessionChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionChain")
            .field("known", &self.known.len())
            .field("pages_read", &self.cursor.steps)
            .field("complete", &self.is_complete())
            .finish()
    }
}

impl SessionChain {
    /// An unwalked chain of the file `descriptor` describes. Reads nothing.
    pub fn new(pages: &PageCache, descriptor: &DbDescriptor) -> Self {
        Self {
            known: Vec::new(),
            cursor: ChainCursor::new(pages, descriptor),
        }
    }

    /// The sessions walked so far, newest first.
    pub fn known(&self) -> &[SessionPage] {
        &self.known
    }

    /// Session pages read into this chain, each of them exactly once.
    pub fn pages_read(&self) -> u32 {
        self.cursor.steps
    }

    /// Whether the walk has passed the oldest session.
    pub fn is_complete(&self) -> bool {
        self.cursor.next.is_none()
    }

    /// Every session, newest first, walking on from wherever this chain
    /// reaches.
    pub fn all(&mut self, pages: &mut PageCache) -> Result<&[SessionPage], SessionError> {
        while self.step(pages)? {}
        Ok(&self.known)
    }

    /// The walked sessions with `from <= sesno <= to`, oldest first.
    ///
    /// Reads nothing, and is complete for any range whose older end this chain
    /// has already resolved — see the type's own note on the prefix.
    pub fn between(&self, from: u32, to: u32) -> Vec<SessionPage> {
        let mut within: Vec<SessionPage> = self
            .known
            .iter()
            .filter(|session| session.sesno >= from && session.sesno <= to)
            .cloned()
            .collect();
        within.reverse();
        within
    }

    /// One selector against this chain: see [`resolve`].
    pub fn resolve(
        &mut self,
        pages: &mut PageCache,
        want: SessionSelector,
        path: &Path,
    ) -> Result<(SessionPage, u32), SessionError> {
        let ([session], read) = self.resolve_many(pages, [want], path)?;
        Ok((session, read))
    }

    /// Two selectors against this chain, on one walk: see [`resolve_pair`].
    pub fn resolve_pair(
        &mut self,
        pages: &mut PageCache,
        first: SessionSelector,
        second: SessionSelector,
        path: &Path,
    ) -> Result<([SessionPage; 2], u32), SessionError> {
        self.resolve_many(pages, [first, second], path)
    }

    /// Resolve every selector on one newest-first walk, stopping as soon as all
    /// of them have an answer and walking further than the sessions already
    /// held only when those do not answer.
    ///
    /// `Exact(n)` with `n` above the newest session is refused after the first
    /// page, without walking; that is the only shortcut. Anything else that is
    /// not on the chain is reported after the whole chain has been read, with
    /// the oldest session reached and the retained sessions either side of the
    /// request.
    ///
    /// The count returned is the session pages **read from the file during this
    /// call** — 0 when the walk so far already covers the request.
    pub fn resolve_many<const N: usize>(
        &mut self,
        pages: &mut PageCache,
        wants: [SessionSelector; N],
        path: &Path,
    ) -> Result<([SessionPage; N], u32), SessionError> {
        let read_before = self.cursor.steps;
        let mut pending: Vec<Pending> = wants.into_iter().map(Pending::new).collect();

        if self.known.is_empty() {
            self.step(pages)?;
        }
        let newest_sesno = self
            .known
            .first()
            .expect("a chain walk always starts at the descriptor's newest session")
            .sesno;

        for want in &pending {
            if let SessionSelector::Exact(wanted) = want.want {
                if wanted > newest_sesno {
                    return Err(SessionError::NoSuchSession {
                        path: path.to_path_buf(),
                        requested: wanted,
                        newest: newest_sesno,
                        oldest: None,
                        nearest_older: Some(newest_sesno),
                        nearest_newer: None,
                    });
                }
            }
        }

        let settled = |pending: &[Pending]| pending.iter().all(|want| want.found.is_some());
        let mut offered = 0usize;
        let mut oldest_reached = newest_sesno;
        loop {
            while offered < self.known.len() {
                let session = &self.known[offered];
                oldest_reached = session.sesno;
                for want in pending.iter_mut() {
                    want.offer(session);
                }
                offered += 1;
                if settled(&pending) {
                    break;
                }
            }
            if settled(&pending) || !self.step(pages)? {
                break;
            }
        }

        let read = self.cursor.steps - read_before;
        let mut resolved = Vec::with_capacity(N);
        for want in pending {
            match want.found {
                Some(session) => resolved.push(session),
                None => {
                    return Err(SessionError::NoSuchSession {
                        path: path.to_path_buf(),
                        requested: want
                            .want
                            .sesno()
                            .expect("Latest always resolves to the newest session"),
                        newest: newest_sesno,
                        oldest: Some(oldest_reached),
                        nearest_older: want.nearest_older,
                        nearest_newer: want.nearest_newer,
                    })
                }
            }
        }
        let resolved: [SessionPage; N] = resolved
            .try_into()
            .unwrap_or_else(|_| unreachable!("one resolved session per selector"));
        Ok((resolved, read))
    }

    /// One session further back, or `false` once the oldest has been passed.
    fn step(&mut self, pages: &mut PageCache) -> Result<bool, SessionError> {
        match self.cursor.advance(pages)? {
            Some(session) => {
                self.known.push(session);
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

/// Walk the whole chain from the newest session back to the oldest.
///
/// The one-shot form, for a caller holding no [`SessionChain`] of its own.
/// [`DbView::walk_sessions`] and `ReadOnlyEngine::sessions` go through the
/// chain they keep instead, which is the same walk without re-reading the
/// sessions that handle has already parsed.
pub fn walk_chain(
    pages: &mut PageCache,
    descriptor: &DbDescriptor,
) -> Result<Vec<SessionPage>, SessionError> {
    let mut chain = SessionChain::new(pages, descriptor);
    chain.all(pages)?;
    Ok(chain.known)
}

/// The newest session, in one page read.
///
/// The chain head is where the descriptor says it is; walking to find it, as
/// [`walk_chain`] does, costs a page per session and answers the same.
pub fn latest(
    pages: &mut PageCache,
    descriptor: &DbDescriptor,
) -> Result<SessionPage, SessionError> {
    read_session(
        pages,
        descriptor.latest_session_page(),
        TextFormat::from_header(descriptor.header_format),
    )
}

/// The one place a selector becomes a session page.
///
/// The one-shot form of [`SessionChain::resolve`], which is where the resolving
/// is done: this walks a chain that is dropped on return, so a caller asking
/// twice pays twice. Returns the session and how many session pages had to be
/// read to reach it.
pub fn resolve(
    pages: &mut PageCache,
    descriptor: &DbDescriptor,
    want: SessionSelector,
    path: &Path,
) -> Result<(SessionPage, u32), SessionError> {
    let mut chain = SessionChain::new(pages, descriptor);
    chain.resolve(pages, want, path)
}

/// Two selectors, one walk.
///
/// A range diff names two sessions; resolving them one after the other would
/// walk the chain twice and, worse, would be a second resolver. Returns the
/// pages in the order the selectors were given, and the session pages read.
/// One-shot, like [`resolve`]: see [`SessionChain::resolve_pair`] to keep the
/// walk.
pub fn resolve_pair(
    pages: &mut PageCache,
    descriptor: &DbDescriptor,
    first: SessionSelector,
    second: SessionSelector,
    path: &Path,
) -> Result<([SessionPage; 2], u32), SessionError> {
    let mut chain = SessionChain::new(pages, descriptor);
    chain.resolve_pair(pages, first, second, path)
}

/// One selector part-way through a walk.
struct Pending {
    want: SessionSelector,
    found: Option<SessionPage>,
    /// Smallest sesno seen that is above the wanted number.
    nearest_newer: Option<u32>,
    /// Largest sesno seen that is below the wanted number.
    nearest_older: Option<u32>,
}

impl Pending {
    fn new(want: SessionSelector) -> Self {
        Self {
            want,
            found: None,
            nearest_newer: None,
            nearest_older: None,
        }
    }

    /// Offer one session of the walk, newest first.
    fn offer(&mut self, session: &SessionPage) {
        if self.found.is_some() {
            return;
        }
        match self.want {
            SessionSelector::Latest => self.found = Some(session.clone()),
            SessionSelector::Exact(wanted) => {
                if session.sesno == wanted {
                    self.found = Some(session.clone());
                } else {
                    self.note_neighbour(session.sesno, wanted);
                }
            }
            SessionSelector::AtOrBefore(wanted) => {
                if session.sesno <= wanted {
                    self.found = Some(session.clone());
                } else {
                    self.note_neighbour(session.sesno, wanted);
                }
            }
        }
    }

    fn note_neighbour(&mut self, seen: u32, wanted: u32) {
        if seen > wanted {
            self.nearest_newer = Some(self.nearest_newer.map_or(seen, |n| n.min(seen)));
        } else if seen < wanted {
            self.nearest_older = Some(self.nearest_older.map_or(seen, |n| n.max(seen)));
        }
    }
}

fn read_session(
    pages: &mut PageCache,
    page: PageId,
    text_format: TextFormat,
) -> Result<SessionPage, SessionError> {
    let data = pages.get(page).map_err(|source| SessionError::Unreadable {
        page,
        detail: source.to_string(),
    })?;
    SessionPage::parse_as(page, data, text_format)
}

/// A `(pageno, extno)` pair from a session page.
///
/// An extent of 0 means "the same file", which is how every sampled database
/// writes an address inside its own extent. An extent that does not fit in a
/// u16 is refused: falling back to the page's own extent, as this used to,
/// silently redirects the pointer into a different file, and no caller can
/// tell a redirected address from a stated one.
fn address(
    page: PageId,
    field: &'static str,
    extno: u32,
    pageno: u32,
) -> Result<PageId, SessionError> {
    let ext = match extno {
        0 => page.ext,
        stated => u16::try_from(stated).map_err(|_| SessionError::ExtentOutOfRange {
            page,
            field,
            extno,
        })?,
    };
    Ok(PageId::new(ext, pageno))
}

/// NUL-padded printable ASCII, as far as it goes.
fn ascii_field(bytes: &[u8]) -> String {
    bytes
        .iter()
        .copied()
        .take_while(|&b| (0x20..0x7F).contains(&b))
        .map(char::from)
        .collect()
}

/// The comment, whose length word counts 4-byte words rather than bytes.
///
/// `ams8000_0001` states 16 there and holds a 61-character comment NUL-padded
/// to 64 bytes; the byte just past `4 * 16` is NUL on every sampled page.
fn comment(data: &[u8], len_words: u32, text: TextFormat) -> String {
    word_text(data, at::COMMENT_TEXT, len_words, usize::MAX, text)
}

/// A word-counted text field: `len_words` 4-byte words from `text_at`,
/// NUL-padded to the word, as `USER` and `DESC` are both stored.
///
/// The stated count is the field's own statement of its size and is what
/// `core.dll` copies (`sub_5AF83D0` moves exactly that many words); the
/// caller's `max_words` is the room the layout leaves before the next field,
/// so a corrupt count cannot read the neighbouring field as text.
///
/// The bytes are then decoded under the header's `FORMAT` word, as
/// `db_get_file_session_string` decodes them (`sub_5ACECE0` reads `FORMAT`
/// first and branches on it): a Chinese comment is `&~…  &` on a `FORMAT 0`
/// page and raw UTF-8 on a `FORMAT 1` page, and either reads as itself.
/// Control characters are dropped, as the ASCII reading before this dropped
/// everything from the first one on.
fn word_text(
    data: &[u8],
    text_at: usize,
    len_words: u32,
    max_words: usize,
    text: TextFormat,
) -> String {
    let words = (len_words as usize).min(max_words);
    let end = text_at.saturating_add(words * 4).min(data.len());
    match data.get(text_at..end) {
        Some(bytes) => crate::record::text::decode_as(bytes, text)
            .chars()
            .filter(|c| !c.is_control())
            .collect(),
        None => String::new(),
    }
}

/// Session page fields under their pre-identification names.
///
/// Kept only because callers outside this module read them; three of the five
/// names are wrong and each says so on its own line. New code should take
/// [`SessionPage`], which names the fields after what they hold.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    /// **Actually +0x04 `last_ses_pgno`** — the *previous* session's page, not
    /// a mark. Renaming waits for L4.
    ///
    /// Normalised: a chain head reports `0xFFFFFFFF`, which is what every
    /// sampled head writes.
    pub session_mark: u32,
    /// **Actually +0x08 `last_ses_extno`** — the previous session's extent,
    /// not a handle. Renaming waits for L4.
    pub db_handle: u32,
    /// +0x0C `sesno`. This one is right.
    pub session_id: u32,
    /// **Actually +0x10, which reads `0xFFFFFFFF` on every sampled session
    /// page** — it has never held a start page, and nobody had looked at the
    /// value. Renaming waits for L4.
    pub start_page_no: u32,
    /// +0x14 `end_pgno`. This one is right.
    pub end_page_no: u32,
}

impl From<&SessionPage> for SessionInfo {
    fn from(page: &SessionPage) -> Self {
        Self {
            session_mark: page.previous.map_or(u32::MAX, |p| p.page),
            db_handle: page.previous.map_or(0, |p| u32::from(p.ext)),
            session_id: page.sesno,
            start_page_no: page.reserved_10,
            end_page_no: page.end.page,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Session page 8897 of `ams8000_0001`, word for word up to the comment.
    fn ams8000_latest() -> Vec<u8> {
        let mut page = vec![0u8; 2048];
        let words: &[(usize, u32)] = &[
            (0x00, 3),
            (0x04, 8884),
            (0x08, 1),
            (0x0C, 264),
            (0x10, u32::MAX),
            (0x14, 8899),
            (0x18, 1),
            (0x1C, 8890),
            (0x20, 1),
            (0x24, 8898),
            (0x28, 1),
            // DATE and UDATE as page 12439 (session 444) of the same file
            // states them on 2026-09-12: 2026-09-09 00:22:05 local, which is
            // 2026-09-08 16:22:05.919 UTC.
            (0x30, 4),
            (0x34, 2026),
            (0x38, 9),
            (0x3C, 216),
            (0x40, 1325),
            (0x64, 4),
            (0x68, 2026),
            (0x6C, 9),
            (0x70, 208),
            (0x74, 1_325_919),
            (0x78, 1),
            (0xA0, 3),
            // The tail words as the 2026-09-12 page 12439 (session 444) of the
            // same file states them: a `Saving Meta Data` session, reason 15,
            // based on no session.
            (0x7E4, 43),
            (0x7E8, 1),
            (0x7EC, 0),
            (0x7F0, 0),
            (0x7F4, 15),
            (0x7F8, 0),
        ];
        for (off, value) in words {
            page[*off..*off + 4].copy_from_slice(&value.to_be_bytes());
        }
        page[0x7C..0x7F].copy_from_slice(b"dpc");
        page[0xA4..0xAD].copy_from_slice(b"CODEX db8");
        page[0x7DC..0x7E4].copy_from_slice(b"2_35_23o");
        page
    }

    #[test]
    fn a_session_page_carries_its_own_index_root() {
        let parsed = SessionPage::parse(PageId::new(1, 8897), &ams8000_latest()).unwrap();
        assert_eq!(parsed.sesno, 264);
        assert_eq!(parsed.index_root, PageId::new(1, 8890));
        assert_eq!(parsed.previous, Some(PageId::new(1, 8884)));
        assert_eq!(parsed.end, PageId::new(1, 8899));
        assert_eq!(parsed.claim, PageId::new(1, 8898));
        assert_eq!(parsed.user, "dpc");
        assert_eq!(parsed.comment, "CODEX db8");
        assert_eq!(parsed.stamp_words, [2026, 9, 216, 1325, 0]);
        assert_eq!(
            parsed.date,
            Some(SessionDate {
                year: 2026,
                month: 9,
                day: 9,
                hour: 0,
                minute: 22,
                second: 5,
                millisecond: 0,
            })
        );
        assert_eq!(
            parsed.utc_date,
            Some(SessionDate {
                year: 2026,
                month: 9,
                day: 8,
                hour: 16,
                minute: 22,
                second: 5,
                millisecond: 919,
            })
        );
        assert_eq!(parsed.zone_offset_seconds(), Some(8 * 3600));
        assert_eq!(parsed.dabacon_version_tag.as_deref(), Some("2_35_23o"));
        assert_eq!(
            parsed.tail,
            Some(SessionTail {
                runum: 43,
                memext: 1,
                mempps: 0,
                base_sesno: 0,
                reason: 15,
                linkps: 0,
            })
        );
    }

    /// `USER` is word-counted like the comment: the count at +0x78 says how
    /// many words are text, and printable bytes past it belong to whatever
    /// the page left there, not to the user name.
    #[test]
    fn the_user_name_is_read_by_its_length_word() {
        let mut page = ams8000_latest();
        page[0x7C..0x88].copy_from_slice(b"happyrustXYZ");

        page[0x78..0x7C].copy_from_slice(&3u32.to_be_bytes());
        let parsed = SessionPage::parse(PageId::new(1, 8897), &page).unwrap();
        assert_eq!(parsed.user, "happyrustXYZ", "three words are twelve bytes");

        page[0x78..0x7C].copy_from_slice(&2u32.to_be_bytes());
        let parsed = SessionPage::parse(PageId::new(1, 8897), &page).unwrap();
        assert_eq!(parsed.user, "happyrus", "two words stop before the third");

        // A count past the room `USER` has is clamped at the comment's length
        // word, so the comment text never reads as part of the user.
        page[0x78..0x7C].copy_from_slice(&40u32.to_be_bytes());
        let parsed = SessionPage::parse(PageId::new(1, 8897), &page).unwrap();
        assert_eq!(parsed.user, "happyrustXYZ");
        assert_eq!(parsed.comment, "CODEX db8");

        page[0x78..0x7C].copy_from_slice(&0u32.to_be_bytes());
        let parsed = SessionPage::parse(PageId::new(1, 8897), &page).unwrap();
        assert_eq!(parsed.user, "", "a zero count is an empty name");
    }

    /// The date words split the way `DB_DateTime::set(year, month, hours,
    /// milliseconds, kind)` splits them: hours → day and hour, the fourth
    /// word → minute, second and, for `UDATE`, millisecond.
    #[test]
    fn date_words_split_like_db_datetime_set() {
        let local = SessionDate::from_words([2024, 10, 449, 1555], FourthWord::Seconds);
        assert_eq!(local.to_string(), "2024-10-18T17:25:55");
        assert!(local.is_plausible());

        let utc = SessionDate::from_words([2026, 9, 208, 1_325_919], FourthWord::Milliseconds);
        assert_eq!(utc.to_string(), "2026-09-08T16:22:05.919");
        assert_eq!(utc.unix_seconds_as_utc(), 1_788_884_525);

        // A midnight hour is day * 24 exactly, and hour 0.
        let midnight = SessionDate::from_words([2026, 9, 216, 0], FourthWord::Seconds);
        assert_eq!((midnight.day, midnight.hour), (9, 0));
        assert_eq!(midnight.to_string(), "2026-09-09T00:00:00");

        // Four zero words parse but are not a date.
        assert!(!SessionDate::from_words([0; 4], FourthWord::Seconds).is_plausible());
    }

    /// `days_from_civil` against dates whose day numbers are known.
    #[test]
    fn days_from_civil_matches_the_calendar() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(2024, 2, 29), 19_782);
        assert_eq!(days_from_civil(2026, 9, 8), 20_704);
    }

    /// `DATE` and `UDATE` are stated only by a count of exactly four words,
    /// which is the condition `core.dll` puts on them; a 2.1 page states a
    /// `UDATE` count of 0 and has no UTC clock.
    #[test]
    fn a_date_is_only_read_behind_a_count_of_four() {
        let mut page = ams8000_latest();
        page[0x64..0x68].copy_from_slice(&0u32.to_be_bytes());
        let parsed = SessionPage::parse(PageId::new(1, 8897), &page).unwrap();
        assert!(parsed.date.is_some());
        assert_eq!(parsed.utc_date, None, "no UDATE on an E3D 2.1 page");
        assert_eq!(parsed.zone_offset_seconds(), None);

        page[0x30..0x34].copy_from_slice(&5u32.to_be_bytes());
        let parsed = SessionPage::parse(PageId::new(1, 8897), &page).unwrap();
        assert_eq!(parsed.date, None, "a count other than four is not a DATE");
        assert_eq!(
            parsed.stamp_words[0], 2026,
            "the raw words are still handed over"
        );
    }

    /// A page that ends before +0x7FC states no tail, the same way it states
    /// no DAB tag; the fields that are there still parse.
    #[test]
    fn a_short_page_has_no_tail() {
        let page = ams8000_latest();
        let parsed = SessionPage::parse(PageId::new(1, 8897), &page[..0x7DC]).unwrap();
        assert_eq!(parsed.tail, None);
        assert_eq!(parsed.dabacon_version_tag, None);
        assert_eq!(parsed.user, "dpc");
        assert_eq!(parsed.sesno, 264);
    }

    #[test]
    fn the_oldest_session_ends_the_chain() {
        let mut page = ams8000_latest();
        page[0x04..0x08].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(
            SessionPage::parse(PageId::new(1, 4), &page)
                .unwrap()
                .previous,
            None
        );

        page[0x04..0x08].copy_from_slice(&0u32.to_be_bytes());
        assert_eq!(
            SessionPage::parse(PageId::new(1, 4), &page)
                .unwrap()
                .previous,
            None
        );
    }

    #[test]
    fn a_page_that_is_not_a_session_is_refused_not_returned_empty() {
        let mut page = ams8000_latest();
        page[0x00..0x04].copy_from_slice(&5u32.to_be_bytes());
        let err = SessionPage::parse(PageId::new(1, 8897), &page).unwrap_err();
        assert!(
            matches!(err, SessionError::NotASessionPage { page_type: 5, .. }),
            "{err}"
        );

        let err = SessionPage::parse(PageId::new(1, 8897), &[0u8; 32]).unwrap_err();
        assert!(matches!(err, SessionError::PageTooSmall { .. }), "{err}");
    }

    /// **Reverting to the fallback turns this red.** An extent past `u16::MAX`
    /// used to become the page's own extent, so a corrupt pointer read as a
    /// valid address in the wrong file.
    #[test]
    fn an_extent_past_u16_is_refused_not_redirected() {
        let mut page = ams8000_latest();
        page[0x08..0x0C].copy_from_slice(&0x1_0000u32.to_be_bytes());
        let err = SessionPage::parse(PageId::new(1, 8897), &page).unwrap_err();
        assert!(
            matches!(
                err,
                SessionError::ExtentOutOfRange {
                    field: "previous session",
                    extno: 0x1_0000,
                    ..
                }
            ),
            "{err}"
        );

        let mut page = ams8000_latest();
        page[0x20..0x24].copy_from_slice(&u32::MAX.to_be_bytes());
        let err = SessionPage::parse(PageId::new(1, 8897), &page).unwrap_err();
        assert!(
            matches!(
                err,
                SessionError::ExtentOutOfRange {
                    field: "index root",
                    extno: u32::MAX,
                    ..
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn a_u32_reads_as_an_exact_selector() {
        assert_eq!(SessionSelector::from(264u32), SessionSelector::Exact(264));
        assert_eq!(Option::<u32>::from(SessionSelector::Latest), None);
        assert_eq!(Option::<u32>::from(SessionSelector::Exact(7)), Some(7));
    }
}
