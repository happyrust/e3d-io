//! `DATE` / `UDATE` on every retained session of the sample databases.
//!
//! The split is `core.dll`'s (`DB_DB::GetSessionDateTime` →
//! `DB_DateTime::set(year, month, hours, milliseconds, kind)`; see
//! `SessionDate`). What a real file adds is the check that the two clocks a
//! 3.1 page carries agree with each other: the local `DATE` and the UTC
//! `UDATE` of one session must be the same instant seen from two zones — same
//! seconds, and one offset for the whole file, a whole number of quarter
//! hours. A wrong split of the third or fourth word would break that on
//! hundreds of pages at once; a right one cannot fake it.
//!
//! `ams1112_0001` is the offline anchor and gets a number pinned. The
//! installed AvevaMarineSample files are read when present; `ams8000_0001`
//! of 3.1 is live and is held to the invariants only.

use e3d_io::session::{walk_chain, DbView, SessionPage};

mod common;

const E3D31_AMS7000: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7000_0001";
const E3D31_AMS8000: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";
const E3D21_AMS8000: &str = r"D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams8000_0001";

fn retained_sessions(path: &str) -> Vec<SessionPage> {
    let mut view = DbView::open(path).expect("open the database at its newest session");
    let descriptor = view.descriptor().clone();
    walk_chain(view.pages_mut(), &descriptor).expect("walk the session chain")
}

/// What every retained session of every sampled database states.
///
/// Returns `(sessions with UDATE, the file's zone offsets, chronology
/// inversions newest-first)`.
fn assert_date_invariants(path: &str, chain: &[SessionPage]) -> (usize, Vec<i64>, usize) {
    assert!(
        !chain.is_empty(),
        "{path}: a database retains at least one session"
    );
    let mut with_utc = 0;
    let mut offsets: Vec<i64> = Vec::new();
    for session in chain {
        let where_ = format!("{path} session {} (page {:?})", session.sesno, session.page);

        let date = session
            .date
            .unwrap_or_else(|| panic!("{where_}: every written session states DATE"));
        assert!(
            date.is_plausible(),
            "{where_}: DATE {date} does not read as a calendar date (raw {:?})",
            session.stamp_words
        );
        assert_eq!(date.millisecond, 0, "{where_}: DATE has no milliseconds");

        if let Some(utc) = session.utc_date {
            with_utc += 1;
            assert!(
                utc.is_plausible(),
                "{where_}: UDATE {utc} does not read as a date"
            );
            // Same instant, two zones: seconds agree, and the offset is a
            // whole number of quarter hours within the range zones span.
            assert_eq!(
                utc.second, date.second,
                "{where_}: DATE {date} and UDATE {utc} disagree on the second"
            );
            let offset = session.zone_offset_seconds().expect("both clocks stated");
            assert_eq!(
                offset % 900,
                0,
                "{where_}: DATE {date} − UDATE {utc} = {offset}s is not a zone offset"
            );
            assert!(
                offset.abs() <= 14 * 3600,
                "{where_}: DATE {date} − UDATE {utc} = {offset}s is outside any zone"
            );
            if !offsets.contains(&offset) {
                offsets.push(offset);
            }
        }
    }

    // Newest first: each session was written after the one behind it. A
    // machine whose clock went backwards between two saves is a fact about
    // that machine, so this is counted and reported rather than asserted.
    let inversions = chain
        .windows(2)
        .filter(|pair| {
            let (newer, older) = (
                pair[0].date.expect("checked"),
                pair[1].date.expect("checked"),
            );
            newer.unix_seconds_as_utc() < older.unix_seconds_as_utc()
        })
        .count();
    (with_utc, offsets, inversions)
}

#[test]
fn the_offline_sample_dates_its_sessions() {
    if common::missing(&[common::TEST_FILE_DB]) {
        return;
    }
    let chain = retained_sessions(common::TEST_FILE_DB);
    let (with_utc, offsets, inversions) = assert_date_invariants(common::TEST_FILE_DB, &chain);

    // Session 1020, saved by `happyrust`, as the page states it: DATE
    // `2024, 10, 449, 1555` — 449 hours into October is the 18th at 17:00,
    // 1555 seconds into the hour is 25:55.
    let newest = &chain[0];
    assert_eq!(
        newest.date.map(|d| d.to_string()).as_deref(),
        Some("2024-10-18T17:25:55")
    );
    assert_eq!(newest.stamp_words[..4], [2024, 10, 449, 1555]);
    assert_eq!(with_utc, 0, "a 2.1-era file states no UDATE");

    let oldest = chain.last().expect("non-empty");
    eprintln!(
        "{}: {} retained sessions, {} → {}, {inversions} chronology inversions, offsets {offsets:?}",
        common::TEST_FILE_DB,
        chain.len(),
        oldest.date.expect("checked"),
        newest.date.expect("checked"),
    );
}

#[test]
fn e3d_2_1_states_local_dates_only() {
    if common::missing(&[E3D21_AMS8000]) {
        return;
    }
    let chain = retained_sessions(E3D21_AMS8000);
    let (with_utc, offsets, inversions) = assert_date_invariants(E3D21_AMS8000, &chain);
    assert_eq!(with_utc, 0, "{E3D21_AMS8000}: E3D 2.1 writes no UDATE");
    assert!(offsets.is_empty());
    eprintln!(
        "{E3D21_AMS8000}: {} retained sessions, {} → {}, {inversions} chronology inversions",
        chain.len(),
        chain.last().expect("non-empty").date.expect("checked"),
        chain[0].date.expect("checked"),
    );
}

/// `UDATE` is written by the engine that writes the session, not by the file's
/// release: `ams7000_0001` ships with the 3.1 sample but its newest session
/// (263, DAB tag `2_28_19_`) was saved by the older engine and states no
/// `UDATE`; the sessions `2_35_23o` writes into `ams8000_0001` do. So the
/// checks are per session — where both clocks are stated they agree, and a
/// file has one zone — and the correlation with the DAB tag is reported.
#[test]
fn e3d_3_1_states_one_zone_offset_per_file() {
    let mut any_with_utc = 0;
    for path in [E3D31_AMS7000, E3D31_AMS8000] {
        if common::missing(&[path]) {
            continue;
        }
        let chain = retained_sessions(path);
        let (with_utc, offsets, inversions) = assert_date_invariants(path, &chain);
        any_with_utc += with_utc;
        // Every session written in the same place agrees on where that is.
        assert!(
            offsets.len() <= 1,
            "{path}: one zone offset for the file, got {offsets:?}"
        );

        // Which engine tags come with a UTC clock and which without.
        let mut by_tag: Vec<(String, usize, usize)> = Vec::new();
        for session in &chain {
            let tag = session.dabacon_version_tag.clone().unwrap_or_default();
            let slot = match by_tag.iter_mut().find(|(t, _, _)| *t == tag) {
                Some(slot) => slot,
                None => {
                    by_tag.push((tag, 0, 0));
                    by_tag.last_mut().expect("just pushed")
                }
            };
            if session.utc_date.is_some() {
                slot.1 += 1;
            } else {
                slot.2 += 1;
            }
        }
        eprintln!(
            "{path}: {} retained sessions, {with_utc} with UDATE, zone offsets {offsets:?}, \
             {} → {}, {inversions} chronology inversions, (DAB tag, with UDATE, without) {by_tag:?}",
            chain.len(),
            chain.last().expect("non-empty").date.expect("checked"),
            chain[0].date.expect("checked"),
        );
    }
    if !common::missing(&[E3D31_AMS8000]) {
        assert!(
            any_with_utc >= 1,
            "the live 3.1 design database has sessions written by the 3.1 engine"
        );
    }
}
