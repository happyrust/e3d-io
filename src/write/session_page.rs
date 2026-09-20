//! Building a session page: the inverse of [`SessionPage::parse_as`].
//!
//! Every word this module writes is read back by `crate::session` through the
//! same offsets (`session::at`), and every word it does not understand is
//! carried over from the previous session page byte for byte. That is the
//! stage-1 rule of the write plan (`docs/plans/2026-09-20-dabacon-write-plan.md`
//! §2 阶段 1): the words whose meaning is pinned — `PREVSP`, `SESNO`,
//! `LASTSP`, `DATE`, `UDATE`, `USER`, `DESC`, `DBVRSN`, `BASESN`, `REASON` —
//! are set; `OWNSES`, `PRISES`, `TRANSA`, `RUNUM`, `MEMEXT`, `MEMPPS`,
//! `LINKPS`, the table directory and both tree roots are copied from the page
//! being superseded, which is also what `core.dll` does for a session that
//! changed no tree (A3.3 shared-mode refresh writes the roots it has).
//!
//! What `core.dll` writes, word by word, is tabulated in the write-path note
//! §2 and in the `core.dll` map on [`crate::session`].

use crate::page::{PageId, PageType};
use crate::record::text::TextFormat;
use crate::session::{at, SessionDate, SessionPage};

use super::pages::{be_word, put_be_word};
use super::WriteError;

/// The eight bytes written where `core.dll` puts its `DBVRSN` engine tag
/// (`2_35_23o` on the 3.1 samples), so a session e3d-io wrote is
/// recognisable as such from the page alone. Reads back as
/// [`SessionPage::dabacon_version_tag`] `"e3dio001"`.
pub const E3DIO_VERSION_TAG: &[u8; 8] = b"e3dio001";

/// `PRISES` (+0x2C), which `core.dll` writes as 1 for a plain `SAVEWORK`
/// (A6: 调用方 a12，普通 SAVEWORK 传 1); every sampled page states 1.
const PRISES_PLAIN_SAVEWORK: u32 = 1;

/// `REASON` (+0x7F4) of a plain `SAVEWORK`: 0. `Saving Meta Data` sessions
/// state 15; nothing this crate writes is one of those.
const REASON_PLAIN_SAVEWORK: u32 = 0;

/// The two clocks a session page carries: the writing machine's wall clock
/// (`DATE`) and the same instant in UTC (`UDATE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionClock {
    pub local: SessionDate,
    pub utc: SessionDate,
}

impl SessionClock {
    /// Now, with the local clock `zone_offset_seconds` east of UTC. The crate
    /// carries no time-zone table; the caller usually hands over the offset
    /// the previous session page states ([`SessionPage::zone_offset_seconds`]),
    /// which is what the next session page of the same site would carry.
    pub fn now(zone_offset_seconds: i64) -> Self {
        let since_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let utc_seconds = since_epoch.as_secs() as i64;
        let millis = since_epoch.subsec_millis();
        Self {
            local: SessionDate::from_unix_seconds(utc_seconds + zone_offset_seconds, 0),
            utc: SessionDate::from_unix_seconds(utc_seconds, millis),
        }
    }

    /// A fixed instant, for tests that want a page whose bytes are stable.
    pub fn fixed(local: SessionDate, utc: SessionDate) -> Self {
        Self { local, utc }
    }
}

/// Everything the builder needs beyond the previous page's bytes.
#[derive(Debug, Clone)]
pub struct NewSession<'a> {
    /// Where the new page will live; goes into nothing on the page itself
    /// but names the `PageId` the parsed result reports.
    pub page: PageId,
    /// Session number: the previous session's + 1.
    pub sesno: u32,
    /// `LASTSP`: the last page this session wrote (the session page itself
    /// when nothing else was written).
    pub end: PageId,
    /// `INDEX` root (words 7/8): the new tree's root when this session changed
    /// it, or the previous session's root carried over when it did not.
    pub index_root: PageId,
    /// Table-directory roots to rewrite: `(name_hash, new_root)`. The `NAME`
    /// table moves here when a name was written, a reverse-reference table or
    /// `TYPEX` when a key was filed or withdrawn; every other table is carried
    /// over. A hash the directory does not list is ignored.
    pub table_overrides: &'a [(u32, PageId)],
    /// Tables the directory does not list yet, appended behind the existing
    /// entries: `(name_hash, root)` — a reverse-reference table created for
    /// an attribute's first non-null value, as `sub_5AFF6D0` does (the
    /// directory grows in creation order; `ams8000_0001` lists `HSTU`, `SPRE`,
    /// `LSTU`, `CATR`, `ISPE` behind `NAME`, `TYPEX`, `SPAMAP`).
    pub table_additions: &'a [(u32, PageId)],
    /// `BASESN`: the session this one was written on top of.
    pub base_sesno: u32,
    /// `USER`, ASCII only in stage 1; at most nine words (36 bytes).
    pub user: &'a str,
    /// `DESC`, ASCII only in stage 1; at most 158 words before the table
    /// directory at +0x31C.
    pub comment: &'a str,
    pub clock: SessionClock,
    /// The header's `FORMAT`, deciding how text is stored. Stage 1 accepts
    /// ASCII under either format, which both store as the bytes themselves.
    pub text: TextFormat,
}

/// Words between `DESC`'s text and the table directory: how long a comment
/// may be.
const COMMENT_MAX_WORDS: usize = (at::TABLE_COUNT - at::COMMENT_TEXT) / 4;

/// Build the new session page from the page it supersedes.
///
/// `previous` is the full previous session page (the one page 0's `LSPGNO`
/// names); `previous_id` is where it lives, which becomes the new page's
/// `PREVSP`. The result is one page of the same length.
pub fn build_session_page(
    previous: &[u8],
    previous_id: PageId,
    new: &NewSession<'_>,
) -> Result<Vec<u8>, WriteError> {
    if previous.len() < at::TAIL_END {
        return Err(WriteError::PreviousSessionPageTooShort {
            page: previous_id,
            len: previous.len(),
            need: at::TAIL_END,
        });
    }
    if PageType::from_raw(be_word(previous, at::PAGE_TYPE)) != PageType::Session {
        return Err(WriteError::NotASessionPage {
            page: previous_id,
            page_type: be_word(previous, at::PAGE_TYPE),
        });
    }

    let mut page = previous.to_vec();

    // Chain: this page points back at the one it supersedes.
    put_be_word(&mut page, at::LAST_SES_PGNO, previous_id.page);
    put_be_word(&mut page, at::LAST_SES_EXTNO, u32::from(previous_id.ext));
    put_be_word(&mut page, at::SESNO, new.sesno);
    // OWNSES (+0x10) is carried over: 0xFFFFFFFF on every master file sampled.
    put_be_word(&mut page, at::END_PGNO, new.end.page);
    put_be_word(&mut page, at::END_EXTNO, u32::from(new.end.ext));
    // INDEX root (+0x1C/+0x20): the new tree when the session changed it, the
    // old one carried over otherwise. CLAIM root (+0x24/+0x28) is always
    // carried over — nothing this crate writes touches a claim tree.
    put_be_word(&mut page, at::INDEX_ROOT_PGNO, new.index_root.page);
    put_be_word(&mut page, at::INDEX_ROOT_EXTNO, u32::from(new.index_root.ext));
    put_be_word(&mut page, at::PRISES, PRISES_PLAIN_SAVEWORK);

    write_date(&mut page, at::DATE_LEN, at::DATE, new.clock.local, false);
    write_date(&mut page, at::UDATE_LEN, at::UDATE, new.clock.utc, true);

    write_text(
        &mut page,
        at::USER_LEN,
        at::USER_TEXT,
        at::USER_MAX_WORDS,
        new.user,
        new.text,
        "USER",
    )?;
    write_text(
        &mut page,
        at::COMMENT_LEN,
        at::COMMENT_TEXT,
        COMMENT_MAX_WORDS,
        new.comment,
        new.text,
        "DESC",
    )?;

    // Table directory (+0x31C..) carried over, save any root the caller
    // moved: find the triple by its name hash and rewrite its page/extent.
    let table_count = be_word(&page, at::TABLE_COUNT) as usize;
    for (name_hash, new_root) in new.table_overrides {
        for slot in 0..table_count {
            let entry = at::TABLE_DIR + slot * at::TABLE_ENTRY_BYTES;
            if entry + at::TABLE_ENTRY_BYTES > page.len() {
                break;
            }
            if be_word(&page, entry) == *name_hash {
                put_be_word(&mut page, entry + 4, new_root.page);
                put_be_word(&mut page, entry + 8, u32::from(new_root.ext));
            }
        }
    }
    // New tables go behind the existing triples. The directory ends where the
    // DAB tag begins; a page that cannot hold another triple refuses rather
    // than overwriting the tag.
    let room = (at::DABACON_VERSION_TAG - at::TABLE_DIR) / at::TABLE_ENTRY_BYTES;
    let mut count = table_count;
    for (name_hash, root) in new.table_additions {
        if count >= room {
            return Err(WriteError::TableDirectoryFull { count: count as u32, room: room as u32 });
        }
        let entry = at::TABLE_DIR + count * at::TABLE_ENTRY_BYTES;
        put_be_word(&mut page, entry, *name_hash);
        put_be_word(&mut page, entry + 4, root.page);
        put_be_word(&mut page, entry + 8, u32::from(root.ext));
        count += 1;
    }
    put_be_word(&mut page, at::TABLE_COUNT, count as u32);

    page[at::DABACON_VERSION_TAG..at::DABACON_VERSION_TAG + at::DABACON_VERSION_TAG_BYTES]
        .copy_from_slice(E3DIO_VERSION_TAG);
    // RUNUM / MEMEXT / MEMPPS / LINKPS carried over; the two pinned tail words set.
    put_be_word(&mut page, at::BASESN, new.base_sesno);
    put_be_word(&mut page, at::REASON, REASON_PLAIN_SAVEWORK);

    // Read it back through the parser before handing it out: a page this
    // module cannot parse is not a session page whatever its bytes say.
    SessionPage::parse_as(new.page, &page, new.text).map_err(WriteError::Session)?;
    Ok(page)
}

/// `DATE` / `UDATE`: count word 4, then `year, month, day*24+hour,
/// minute*60+second` — the fourth in milliseconds for `UDATE`.
fn write_date(page: &mut [u8], len_at: usize, words_at: usize, date: SessionDate, millis: bool) {
    put_be_word(page, len_at, at::DATE_WORDS as u32);
    let fourth = date.minute * 60 + date.second;
    let fourth = if millis {
        fourth * 1000 + date.millisecond
    } else {
        fourth
    };
    let words = [date.year, date.month, date.day * 24 + date.hour, fourth];
    for (i, word) in words.iter().enumerate() {
        put_be_word(page, words_at + i * 4, *word);
    }
}

/// A word-counted text field, NUL-padded to the word, exactly as `USER` and
/// `DESC` are stored (`session::word_text` reads them back so). The field's
/// room is cleared first so a shorter text leaves no tail of the old one.
fn write_text(
    page: &mut [u8],
    len_at: usize,
    text_at: usize,
    max_words: usize,
    text: &str,
    format: TextFormat,
    field: &'static str,
) -> Result<(), WriteError> {
    if !text.is_ascii() || text.bytes().any(|b| b < 0x20 || b == 0x7F) {
        return Err(WriteError::TextNotAscii {
            field,
            text: text.to_owned(),
        });
    }
    // Under both formats ASCII is stored as itself: `FORMAT 1` is UTF-8 as it
    // lies, and the escaped form only wraps bytes outside seven-bit ASCII.
    let _ = format;
    let bytes = text.as_bytes();
    let words = bytes.len().div_ceil(4);
    if words > max_words {
        return Err(WriteError::TextTooLong {
            field,
            len: bytes.len(),
            max: max_words * 4,
        });
    }
    page[text_at..text_at + max_words * 4].fill(0);
    page[text_at..text_at + bytes.len()].copy_from_slice(bytes);
    put_be_word(page, len_at, words as u32);
    Ok(())
}

impl SessionDate {
    /// The civil fields of `unix_seconds` **read as UTC**, plus a millisecond
    /// part — the inverse of [`SessionDate::unix_seconds_as_utc`]. A local
    /// clock is produced by adding the zone offset to the seconds first.
    pub fn from_unix_seconds(unix_seconds: i64, millisecond: u32) -> Self {
        let days = unix_seconds.div_euclid(86_400);
        let secs = unix_seconds.rem_euclid(86_400) as u32;
        let (year, month, day) = civil_from_days(days);
        Self {
            year: year as u32,
            month,
            day,
            hour: secs / 3600,
            minute: (secs / 60) % 60,
            second: secs % 60,
            millisecond,
        }
    }
}

/// Howard Hinnant's `civil_from_days`: the proleptic Gregorian date of a day
/// count since 1970-01-01, matching `days_from_civil` on the read side.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_from_march + 2) / 5 + 1) as u32;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_seconds_round_trip_through_civil_fields() {
        // 2026-09-08T16:22:05.919Z, the UDATE of ams8000_0001 session 444.
        let date = SessionDate::from_unix_seconds(1_788_884_525, 919);
        assert_eq!(
            (date.year, date.month, date.day, date.hour, date.minute, date.second),
            (2026, 9, 8, 16, 22, 5)
        );
        assert_eq!(date.millisecond, 919);
        assert_eq!(date.unix_seconds_as_utc(), 1_788_884_525);
        for seconds in [0i64, 951_782_400, 1_709_251_199, 4_102_444_800] {
            assert_eq!(
                SessionDate::from_unix_seconds(seconds, 0).unix_seconds_as_utc(),
                seconds
            );
        }
    }

    #[test]
    fn date_words_read_back_as_written() {
        let mut page = vec![0u8; at::TAIL_END];
        let date = SessionDate {
            year: 2026,
            month: 9,
            day: 20,
            hour: 14,
            minute: 25,
            second: 20,
            millisecond: 0,
        };
        write_date(&mut page, at::DATE_LEN, at::DATE, date, false);
        assert_eq!(be_word(&page, at::DATE_LEN), 4);
        assert_eq!(be_word(&page, at::DATE), 2026);
        assert_eq!(be_word(&page, at::DATE + 8), 20 * 24 + 14);
        assert_eq!(be_word(&page, at::DATE + 12), 25 * 60 + 20);
    }

    #[test]
    fn text_is_word_counted_and_nul_padded() {
        let mut page = vec![0xAAu8; at::TAIL_END];
        write_text(
            &mut page,
            at::USER_LEN,
            at::USER_TEXT,
            at::USER_MAX_WORDS,
            "dpc",
            TextFormat::Utf8,
            "USER",
        )
        .unwrap();
        assert_eq!(be_word(&page, at::USER_LEN), 1);
        assert_eq!(&page[at::USER_TEXT..at::USER_TEXT + 4], b"dpc\0");
        // The rest of the field is cleared, not left as it was.
        assert!(page[at::USER_TEXT + 4..at::USER_TEXT + at::USER_MAX_WORDS * 4]
            .iter()
            .all(|b| *b == 0));
        let too_long = "x".repeat(at::USER_MAX_WORDS * 4 + 1);
        assert!(matches!(
            write_text(
                &mut page,
                at::USER_LEN,
                at::USER_TEXT,
                at::USER_MAX_WORDS,
                &too_long,
                TextFormat::Utf8,
                "USER"
            ),
            Err(WriteError::TextTooLong { field: "USER", .. })
        ));
        assert!(matches!(
            write_text(
                &mut page,
                at::USER_LEN,
                at::USER_TEXT,
                at::USER_MAX_WORDS,
                "反应堆",
                TextFormat::Utf8,
                "USER"
            ),
            Err(WriteError::TextNotAscii { field: "USER", .. })
        ));
    }
}
