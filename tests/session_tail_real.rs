//! The session page's `USER` field and its six closing words, on real databases.
//!
//! `core.dll` addresses session-page fields by attribute name: `USER` is a
//! word-counted text at +0x78/+0x7C (this crate used to call it `machine` and
//! read it without the count), and words 505..510 are `RUNUM`, `MEMEXT`,
//! `MEMPPS`, `BASESN`, `REASON`, `LINKPS`. These tests pin what every retained
//! session of the sample databases states, so a layout the reader has wrong
//! shows up here rather than in a downstream log line.
//!
//! The offline copy `ams1112_0001` is the anchor: it is not written to.
//! The installed E3D 2.1 / 3.1 AvevaMarineSample files are read when present;
//! `ams8000_0001` of 3.1 is live (E3D and a running service write it), so it
//! is only held to the invariants, never to a number.

use e3d_io::session::{walk_chain, DbView, SessionPage};

mod common;

const E3D31_AMS7000: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7000_0001";
const E3D31_AMS8000: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";
const E3D21_AMS8000: &str = r"D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams8000_0001";

/// How a retained session sits on the one retained behind it, read off
/// `BASESN` alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stacking {
    /// `BASESN` is the retained session right behind: a plain SAVEWORK on an
    /// unmerged stretch of the chain.
    Direct,
    /// `BASESN` is newer than the retained session behind: the sessions in
    /// between were folded away by MERGE CHANGES.
    OverAGap,
    /// `BASESN` is *older* than the retained session behind: the state was
    /// taken back to an earlier session (BACKTRACK) and saved on as a new one.
    /// This is the shape a downstream keying caches on "session number only
    /// goes up" has to see.
    Reverted,
    /// `BASESN` is 0: based on nothing (E3D 3.1's `Saving Meta Data` pages).
    Unbased,
}

impl Stacking {
    fn of(newer: &SessionPage, older: &SessionPage) -> Self {
        let base = newer.tail.expect("full page").base_sesno;
        if base == 0 {
            Self::Unbased
        } else if base == older.sesno {
            Self::Direct
        } else if base > older.sesno {
            Self::OverAGap
        } else {
            Self::Reverted
        }
    }
}

/// `(direct, over a gap, reverted, unbased)` over every retained pair.
fn stacking_census(chain: &[SessionPage]) -> (usize, usize, usize, usize) {
    chain.windows(2).fold((0, 0, 0, 0), |(d, g, r, u), pair| {
        match Stacking::of(&pair[0], &pair[1]) {
            Stacking::Direct => (d + 1, g, r, u),
            Stacking::OverAGap => (d, g + 1, r, u),
            Stacking::Reverted => (d, g, r + 1, u),
            Stacking::Unbased => (d, g, r, u + 1),
        }
    })
}

/// Every retained session page, newest first.
fn retained_sessions(path: &str) -> Vec<SessionPage> {
    let mut view = DbView::open(path).expect("open the database at its newest session");
    let descriptor = view.descriptor().clone();
    walk_chain(view.pages_mut(), &descriptor).expect("walk the session chain")
}

/// What holds on every retained session of every sampled database.
fn assert_session_invariants(path: &str, chain: &[SessionPage]) {
    assert!(
        !chain.is_empty(),
        "{path}: a database retains at least one session"
    );
    for (index, session) in chain.iter().enumerate() {
        let where_ = format!("{path} session {} (page {:?})", session.sesno, session.page);

        // USER: word-counted, so it can never be longer than the nine words
        // that fit before the comment's length word, and a written session
        // always names who wrote it.
        assert!(!session.user.is_empty(), "{where_}: USER is empty");
        assert!(
            session.user.len() <= 9 * 4,
            "{where_}: USER {:?} is longer than its nine words",
            session.user
        );
        assert!(
            session.user.bytes().all(|b| b.is_ascii_graphic()),
            "{where_}: USER {:?} is not a bare user name",
            session.user
        );

        // Every sampled page is 2048 bytes, so the tail is always there.
        let tail = session
            .tail
            .unwrap_or_else(|| panic!("{where_}: a full session page states its tail"));

        // BASESN names the session this one was written on top of, so it is
        // strictly older than this session; 0 means "based on none". It is
        // *not* bounded below by the retained session behind it: MERGE CHANGES
        // folds sessions away (BASESN then names a number the chain no longer
        // holds), and a BACKTRACK writes a new session on top of an old state
        // (`ams8000_0001` of 3.1 has session 243 based on 239 with 242
        // retained behind it). See [`Stacking::of`].
        if tail.base_sesno != 0 {
            assert!(
                tail.base_sesno < session.sesno,
                "{where_}: BASESN {} is not older than the session",
                tail.base_sesno
            );
        }
        let _ = index;

        // REASON is a small message-catalogue code (message 43 / 460 + code),
        // not a page number or a hash.
        assert!(
            tail.reason < 100,
            "{where_}: REASON {} is not a reason code",
            tail.reason
        );
    }
}

#[test]
fn the_offline_sample_names_its_user_and_base_session() {
    if common::missing(&[common::TEST_FILE_DB]) {
        return;
    }
    let chain = retained_sessions(common::TEST_FILE_DB);
    assert_session_invariants(common::TEST_FILE_DB, &chain);

    let newest = &chain[0];
    assert_eq!(
        newest.user, "happyrust",
        "the fixture's sessions were saved by happyrust"
    );
    let tail = newest.tail.expect("full page");
    assert_eq!(tail.reason, 0, "a plain SAVEWORK");
    assert_eq!(
        tail.base_sesno + 1,
        newest.sesno,
        "a plain SAVEWORK is based on the session right before it"
    );
    assert_eq!((tail.mempps, tail.linkps), (0, 0));

    // How the retained pairs stack. Reported, not asserted: the fixture's
    // merge and backtrack history is the file's, not ours.
    let census = stacking_census(&chain);
    eprintln!(
        "{}: {} retained sessions, stacking (direct, over a gap, reverted, unbased) = {census:?}, users {:?}",
        common::TEST_FILE_DB,
        chain.len(),
        distinct_users(&chain)
    );
}

#[test]
fn a_merged_chain_bases_sessions_on_numbers_it_no_longer_retains() {
    if common::missing(&[E3D31_AMS7000]) {
        return;
    }
    let chain = retained_sessions(E3D31_AMS7000);
    assert_session_invariants(E3D31_AMS7000, &chain);

    // `ams7000_0001` keeps only 1, 237, 238, 262 and 263 (see
    // `SessionSelector::AtOrBefore`): a gap in the retained numbers is where
    // MERGE CHANGES ran, and the session after the gap was still written on
    // top of the one right before it, which the chain no longer holds.
    let numbers: Vec<u32> = chain.iter().map(|s| s.sesno).collect();
    let has_gap = chain
        .windows(2)
        .any(|pair| pair[0].sesno > pair[1].sesno + 1);
    assert!(
        has_gap,
        "{E3D31_AMS7000}: expected a merged chain, got {numbers:?}"
    );
    let newest = &chain[0];
    let tail = newest.tail.expect("full page");
    assert_eq!(
        tail.base_sesno + 1,
        newest.sesno,
        "the newest SAVEWORK sits on its predecessor"
    );
    eprintln!(
        "{E3D31_AMS7000}: retained {numbers:?}, users {:?}",
        distinct_users(&chain)
    );
}

#[test]
fn e3d_2_1_writes_the_same_layout() {
    if common::missing(&[E3D21_AMS8000]) {
        return;
    }
    let chain = retained_sessions(E3D21_AMS8000);
    assert_session_invariants(E3D21_AMS8000, &chain);
    let newest = &chain[0];
    let tail = newest.tail.expect("full page");
    assert_eq!(tail.reason, 0);
    assert_eq!(tail.base_sesno + 1, newest.sesno);
    eprintln!(
        "{E3D21_AMS8000}: {} retained sessions, users {:?}",
        chain.len(),
        distinct_users(&chain)
    );
}

/// The live 3.1 design database: invariants, plus the two things only a
/// worked-on database shows — `Saving Meta Data` sessions with a non-zero
/// REASON and no base, and sessions written on top of an *older* state than
/// the retained session behind them (a BACKTRACK), which is what BASESN
/// exists to tell apart from a plain SAVEWORK.
#[test]
fn the_live_design_database_holds_the_invariants() {
    if common::missing(&[E3D31_AMS8000]) {
        return;
    }
    let chain = retained_sessions(E3D31_AMS8000);
    assert_session_invariants(E3D31_AMS8000, &chain);

    let mut reasons: Vec<(u32, usize)> = Vec::new();
    for session in &chain {
        let reason = session.tail.expect("full page").reason;
        match reasons.iter_mut().find(|(code, _)| *code == reason) {
            Some((_, count)) => *count += 1,
            None => reasons.push((reason, 1)),
        }
    }
    reasons.sort();

    // Every retained pair, classified; the reverted ones listed with their
    // reason codes so the message-catalogue lookup, when it lands, has
    // numbers to check against.
    let census = stacking_census(&chain);
    let reverted: Vec<(u32, u32, u32, u32)> = chain
        .windows(2)
        .filter(|pair| Stacking::of(&pair[0], &pair[1]) == Stacking::Reverted)
        .map(|pair| {
            let tail = pair[0].tail.expect("full page");
            (pair[0].sesno, tail.base_sesno, pair[1].sesno, tail.reason)
        })
        .collect();
    // A reverted session is the one pattern a session-number-only key misses:
    // the number went up while the content went back. On 2026-09-12 this file
    // carried session 243 based on 239 with 242 retained behind it; the file
    // is live, so that is reported here rather than asserted.
    eprintln!(
        "{E3D31_AMS8000}: {} retained sessions, reasons (code, count) {reasons:?}, \
         stacking (direct, over a gap, reverted, unbased) = {census:?}, \
         reverted (sesno, BASESN, retained behind, reason) {reverted:?}, users {:?}",
        chain.len(),
        distinct_users(&chain)
    );
}

fn distinct_users(chain: &[SessionPage]) -> Vec<&str> {
    let mut users: Vec<&str> = chain.iter().map(|s| s.user.as_str()).collect();
    users.sort_unstable();
    users.dedup();
    users
}
