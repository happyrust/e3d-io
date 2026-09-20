//! Stage 1 of the write plan (`docs/plans/2026-09-20-dabacon-write-plan.md`
//! §2 阶段 1): an empty session appended to a copy of a sample database must
//! read back through every reader this crate has, and must have touched
//! nothing but page 0's commit words.
//!
//! The fixtures are the installed E3D 3.1 samples; a machine without them
//! reports a skip. The session number of the sample is read, not assumed —
//! `ams8000_0001` was at 281 when the plan was written and at 640 a day later.

mod common;

use common::write_fixture::{
    assert_appended_contiguous, assert_prefix_unchanged, page_word, DbCopy, AMS5100, AMS8000,
};
use e3d_io::engine::ScanTier;
use e3d_io::page::PageId;
use e3d_io::session::{DbView, SessionDate, SessionSelector};
use e3d_io::write::{
    stage_empty_session, write_empty_session, SessionClock, SessionRequest, WriteError,
    E3DIO_VERSION_TAG,
};
use e3d_io::ReadOnlyEngine;

/// Page 0 words a commit may change: `TMPLVN` (9, only when positive),
/// `LSPGNO` (10) and its extent (11).
const PAGE0_COMMIT_WORDS: &[usize] = &[9, 10, 11];

fn fixed_clock() -> SessionClock {
    let local = SessionDate {
        year: 2026,
        month: 9,
        day: 20,
        hour: 14,
        minute: 25,
        second: 20,
        millisecond: 0,
    };
    let utc = SessionDate {
        year: 2026,
        month: 9,
        day: 20,
        hour: 6,
        minute: 25,
        second: 20,
        millisecond: 919,
    };
    SessionClock::fixed(local, utc)
}

/// What the readers say about a file, gathered the same way before and after.
struct Snapshot {
    sesno: u32,
    page: PageId,
    end: PageId,
    index_root: PageId,
    claim: PageId,
    reserved_10: u32,
    tables: Vec<(u32, PageId)>,
    chain: Vec<u32>,
    latest_ses_pgno: u32,
    element_count: usize,
    /// The first ten elements in key order and where their records live.
    heads: Vec<(e3d_io::RefNo, u32, u32)>,
}

fn snapshot(path: &std::path::Path) -> Snapshot {
    let mut view = DbView::open(path).expect("open with DbView");
    let session = view.session().clone();
    let chain: Vec<u32> = view
        .walk_sessions()
        .expect("walk the session chain")
        .iter()
        .map(|s| s.sesno)
        .collect();
    let latest_ses_pgno = view.descriptor().latest_ses_pgno;
    drop(view);

    let mut engine = ReadOnlyEngine::open(path).expect("open with ReadOnlyEngine");
    let mut element_count = 0usize;
    let mut heads = Vec::new();
    for element in engine.scan_elements(ScanTier::Header).expect("scan") {
        let element = element.expect("scanned element");
        if heads.len() < 10 {
            heads.push((element.refno, element.page_no, element.offset_words));
        }
        element_count += 1;
    }

    Snapshot {
        sesno: session.sesno,
        page: session.page,
        end: session.end,
        index_root: session.index_root,
        claim: session.claim,
        reserved_10: session.reserved_10,
        tables: session
            .tables
            .iter()
            .map(|t| (t.name_hash, t.root))
            .collect(),
        chain,
        latest_ses_pgno,
        element_count,
        heads,
    }
}

fn round_trip(source: &str) {
    let copy = DbCopy::of(source);
    let before = snapshot(&copy.path);
    assert_eq!(before.chain[0], before.sesno, "chain starts at the newest session");

    let report = write_empty_session(
        &copy.path,
        &SessionRequest {
            user: "e3dio",
            comment: "e3d-io stage1 empty session",
            clock: Some(fixed_clock()),
        },
    )
    .expect("write an empty session");

    let expected_page = PageId::primary(before.end.page + 1);
    assert_eq!(report.previous.sesno, before.sesno);
    assert_eq!(report.previous.page, before.page);
    assert_eq!(report.written.sesno, before.sesno + 1);
    assert_eq!(report.written.page, expected_page);
    assert_eq!(report.written.end, expected_page);
    assert_eq!(report.appended_pages, vec![expected_page.page]);

    // --- the session page, through the session reader ---
    let mut view = DbView::open(&copy.path).expect("reopen the written copy");
    let session = view.session().clone();
    assert_eq!(session.sesno, before.sesno + 1);
    assert_eq!(session.page, expected_page);
    assert_eq!(session.previous, Some(before.page));
    assert_eq!(session.end, expected_page);
    assert_eq!(session.index_root, before.index_root, "index root carried over");
    assert_eq!(session.claim, before.claim, "claim root carried over");
    assert_eq!(session.reserved_10, before.reserved_10, "OWNSES carried over");
    let tables: Vec<(u32, PageId)> = session.tables.iter().map(|t| (t.name_hash, t.root)).collect();
    assert_eq!(tables, before.tables, "table directory carried over");
    assert_eq!(session.user, "e3dio");
    assert_eq!(session.comment, "e3d-io stage1 empty session");
    assert_eq!(session.date, Some(fixed_clock().local));
    assert_eq!(session.utc_date, Some(fixed_clock().utc));
    assert_eq!(session.zone_offset_seconds(), Some(8 * 3600));
    assert_eq!(
        session.dabacon_version_tag.as_deref(),
        Some(std::str::from_utf8(E3DIO_VERSION_TAG).unwrap())
    );
    let tail = session.tail.expect("a 2048-byte page states its tail");
    assert_eq!(tail.base_sesno, before.sesno, "BASESN names the session written on");
    assert_eq!(tail.reason, 0, "a plain SAVEWORK states reason 0");
    assert_eq!(view.descriptor().latest_ses_pgno, expected_page.page);

    // --- the chain ---
    let chain: Vec<u32> = view
        .walk_sessions()
        .expect("walk the chain")
        .iter()
        .map(|s| s.sesno)
        .collect();
    assert_eq!(chain.len(), before.chain.len() + 1);
    assert_eq!(chain[0], before.sesno + 1);
    assert_eq!(&chain[1..], &before.chain[..], "the old chain hangs unchanged behind the new page");
    assert!(chain.windows(2).all(|w| w[0] > w[1]), "sesno strictly decreasing along the chain");

    // --- the range diff: nothing changed ---
    let diff = view
        .diff_sessions(
            SessionSelector::Exact(before.sesno),
            SessionSelector::Exact(before.sesno + 1),
        )
        .expect("diff the two sessions");
    let tally = diff.tally();
    assert_eq!((tally.deleted, tally.inserted, tally.modified), (0, 0, 0));
    assert!(diff.is_empty());
    drop(view);

    // --- the elements ---
    let after = snapshot(&copy.path);
    assert_eq!(after.element_count, before.element_count);
    assert_eq!(after.heads, before.heads, "records live where they lived");

    // --- the bytes ---
    let bytes = copy.current_bytes();
    let page_bytes = 4 * page_word(&copy.original, 2048, 0, 13) as usize;
    assert_prefix_unchanged(
        &copy.original,
        &bytes,
        page_bytes,
        before.end.page as usize,
        PAGE0_COMMIT_WORDS,
    );
    assert_eq!(page_word(&bytes, page_bytes, 0, 10), expected_page.page);
    assert_eq!(page_word(&bytes, page_bytes, 0, 11), 1);
    assert_appended_contiguous(
        &bytes,
        page_bytes,
        before.end.page as usize,
        expected_page.page as usize,
        &[3],
    );
}

#[test]
fn scaffold_names_the_changed_word() {
    // Four 64-byte pages, so page 0 has a word 10 to allow.
    let page_bytes = 64;
    let before: Vec<u8> = (0..=255u8).collect();
    assert_prefix_unchanged(&before, &before, page_bytes, 3, &[]);

    let mut after = before.clone();
    after[2 * 64 + 8] ^= 0xFF; // page 2, word 2
    let caught = std::panic::catch_unwind(|| {
        assert_prefix_unchanged(&before, &after, page_bytes, 3, &[]);
    });
    let payload = caught.expect_err("a changed word must fail");
    let message = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    assert!(message.contains("page 2 word 2"), "{message}");

    let mut page0 = before.clone();
    page0[10 * 4] ^= 0xFF; // page 0, word 10
    assert_prefix_unchanged(&before, &page0, page_bytes, 3, &[10]);
    assert!(std::panic::catch_unwind(|| {
        assert_prefix_unchanged(&before, &page0, page_bytes, 3, &[9]);
    })
    .is_err());

    let mut appended = before.clone();
    appended.extend_from_slice(&[0, 0, 0, 3]);
    appended.extend_from_slice(&[0u8; 60]);
    assert_appended_contiguous(&appended, page_bytes, 3, 4, &[3]);
    assert!(std::panic::catch_unwind(|| assert_appended_contiguous(&appended, page_bytes, 3, 4, &[5])).is_err());
}

#[test]
fn empty_session_round_trips_on_ams8000() {
    if common::missing(&[AMS8000]) {
        return;
    }
    round_trip(AMS8000);
}

#[test]
fn empty_session_round_trips_on_ams5100() {
    if common::missing(&[AMS5100]) {
        return;
    }
    round_trip(AMS5100);
}

#[test]
fn staged_but_uncommitted_file_still_reads_as_the_old_session() {
    if common::missing(&[AMS5100]) {
        return;
    }
    let copy = DbCopy::of(AMS5100);
    let before = snapshot(&copy.path);

    let staged = stage_empty_session(
        &copy.path,
        &SessionRequest {
            user: "e3dio",
            comment: "crash before page 0",
            clock: Some(fixed_clock()),
        },
    )
    .expect("stage");
    assert_eq!(staged.previous().sesno, before.sesno);
    assert_eq!(staged.staged().sesno, before.sesno + 1);

    // The new page is on disk, page 0 is not: a crash here leaves this file.
    let page_bytes = staged.page_size().bytes();
    let bytes = copy.current_bytes();
    assert_eq!(bytes.len(), copy.original.len() + page_bytes, "exactly one page appended");
    assert_prefix_unchanged(&copy.original, &bytes, page_bytes, before.end.page as usize, &[]);
    let mid = snapshot(&copy.path);
    assert_eq!(mid.sesno, before.sesno, "readers still see the old session");
    assert_eq!(mid.chain, before.chain);
    assert_eq!(mid.latest_ses_pgno, before.latest_ses_pgno);
    assert_eq!(mid.element_count, before.element_count);

    // Committing the same staging finishes the job.
    let report = staged.commit().expect("commit");
    assert_eq!(report.written.sesno, before.sesno + 1);
    let after = snapshot(&copy.path);
    assert_eq!(after.sesno, before.sesno + 1);
    assert_eq!(after.chain.len(), before.chain.len() + 1);
}

#[test]
fn rollback_restores_the_file_byte_for_byte() {
    if common::missing(&[AMS5100]) {
        return;
    }
    let copy = DbCopy::of(AMS5100);
    let staged = stage_empty_session(
        &copy.path,
        &SessionRequest {
            user: "e3dio",
            comment: "rolled back",
            clock: Some(fixed_clock()),
        },
    )
    .expect("stage");
    assert_ne!(copy.current_bytes().len(), copy.original.len());
    staged.rollback().expect("rollback");
    assert_eq!(copy.current_bytes(), copy.original);
}

#[test]
fn commit_refuses_when_page_0_moved_under_it() {
    if common::missing(&[AMS5100]) {
        return;
    }
    let copy = DbCopy::of(AMS5100);
    let before = snapshot(&copy.path);
    let staged = stage_empty_session(
        &copy.path,
        &SessionRequest {
            user: "e3dio",
            comment: "second writer",
            clock: Some(fixed_clock()),
        },
    )
    .expect("stage");

    // Another writer commits first: page 0's LSPGNO names a different page.
    let page_bytes = staged.page_size().bytes();
    let mut bytes = copy.current_bytes();
    let other = before.end.page + 7;
    bytes[0x28..0x2C].copy_from_slice(&other.to_be_bytes());
    std::fs::write(&copy.path, &bytes).expect("simulate the other writer");

    match staged.commit() {
        Err(WriteError::Moved {
            expected, found, ..
        }) => {
            assert_eq!(expected, before.page.page);
            assert_eq!(found, other);
        }
        other => panic!("commit must refuse, got {other:?}"),
    }
    // Nothing of ours reached page 0.
    let after = std::fs::read(&copy.path).unwrap();
    assert_eq!(page_word(&after, page_bytes, 0, 10), other);
}
