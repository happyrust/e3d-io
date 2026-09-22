//! The write-mode sentinel lock (`docs/plans/2026-09-21-write-path-next-steps.md`
//! S7): a session write takes an exclusive lock on the one byte `core.dll` locks
//! while a database is in write mode, so a file another writer — this crate or a
//! real E3D 3.1 — is mid-write on is refused at once and left untouched. This is
//! the Rust-level stand-in for the TTY gate's question 17 (E3D holding the copy
//! in write mode), which needs a live E3D; the lock, its byte and its release
//! are the same either way.
//!
//! Windows only: `LockFileEx` is what `core.dll` uses, and the lock is a no-op
//! elsewhere (the commit's page-0 re-read still catches a writer that finished).
#![cfg(windows)]

mod common;

use common::write_fixture::{DbCopy, AMS8000};
use e3d_io::session::DbView;
use e3d_io::write::{write_empty_session, PageFile, SessionRequest};
use e3d_io::write::{SessionClock, WriteError};
use e3d_io::session::SessionDate;

fn clock() -> SessionClock {
    SessionClock::fixed(
        SessionDate { year: 2026, month: 9, day: 22, hour: 16, minute: 0, second: 0, millisecond: 0 },
        SessionDate { year: 2026, month: 9, day: 22, hour: 8, minute: 0, second: 0, millisecond: 0 },
    )
}

#[test]
fn a_write_is_refused_while_the_file_is_in_write_mode_and_succeeds_once_it_is_free() {
    if common::missing(&[AMS8000]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let request = SessionRequest { user: "e3dio", comment: "e3d-io S7 lock", clock: Some(clock()) };

    let base_sesno = DbView::open(&copy.path).unwrap().session().sesno;

    // Another writer (stand-in for E3D in write mode) holds the sentinel.
    let mut holder = PageFile::open(&copy.path).expect("open holder");
    holder.lock_for_write().expect("holder takes the write-mode lock");

    // A write now is refused at once, and not a byte changes.
    match write_empty_session(&copy.path, &request) {
        Err(WriteError::Locked { .. }) => {}
        other => panic!("a write into a file in write mode must be refused as Locked, got {other:?}"),
    }
    assert_eq!(copy.current_bytes(), copy.original, "a refused write changes no bytes");
    assert_eq!(
        DbView::open(&copy.path).unwrap().session().sesno,
        base_sesno,
        "the session is still the base one"
    );

    // The writer leaves write mode; the lock is free and the write goes through.
    drop(holder);
    let report = write_empty_session(&copy.path, &request).expect("the write succeeds once the lock is free");
    assert_eq!(report.written.sesno, base_sesno + 1, "the empty session committed");
    assert_ne!(copy.current_bytes(), copy.original, "and the file moved on");
}

/// Two staged sessions cannot be open on one file at once: the second is
/// refused while the first holds the lock, and goes through after it commits.
#[test]
fn staging_holds_the_lock_until_it_commits() {
    if common::missing(&[AMS8000]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let request = SessionRequest { user: "e3dio", comment: "e3d-io S7 staged lock", clock: Some(clock()) };

    let staged = e3d_io::write::stage_empty_session(&copy.path, &request).expect("stage the first");
    // A second write while the first is staged (uncommitted) is refused.
    match write_empty_session(&copy.path, &request) {
        Err(WriteError::Locked { .. }) => {}
        other => panic!("a staged session holds the lock, so a second write is Locked, got {other:?}"),
    }
    // Commit the first (releasing the lock), then the second write goes through.
    let base = staged.previous().sesno;
    staged.commit().expect("commit the first");
    let report = write_empty_session(&copy.path, &request).expect("the second write after the first commits");
    assert_eq!(report.written.sesno, base + 2, "two sessions committed in turn");
}
