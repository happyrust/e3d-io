//! Which claim-tree edits a session's element writes imply
//! (`docs/plans/2026-09-20-dabacon-write-plan.md` §7 CLAIM; `core.dll`'s
//! `sub_5B0C760`, A3a.6).
//!
//! The claim tree keys **primary** elements — the ones whose noun template's
//! payload word 4 is set ([`crate::record::template::NounTemplate::is_primary`])
//! — to `(owner, session)`. `core.dll` keeps it in step with a save by diffing
//! the committed and new index and, for each element that changed, touching the
//! claim of its nearest primary: a new primary gets a claim `(0, new_session)`,
//! a deleted primary loses it, and anything else — a primary whose record was
//! rewritten, or a non-primary whose primary ancestor was — bumps that
//! primary's session to the new one (`sub_5B059C0` walks the owner chain to the
//! primary; `sub_5B06C40` is the "did it actually change" gate we take a write
//! at face value for). Only an *existing* claim is bumped: a primary the tree
//! has no claim for is left alone (the claims manager's 534), which is why a
//! non-primary touch inserts nothing.
//!
//! This module turns the elements a session writes into that set of edits. It
//! is given each written element's noun and owner and two lookups — whether a
//! noun is primary, and the noun/owner of an element it must walk up through —
//! so it needs neither the schema nor the file directly and is tested on
//! synthetic hierarchies.

use std::collections::BTreeMap;

use crate::refno::RefNo;

use super::claim_cow::{ClaimEdit, ClaimEditKind};

/// How an element was written this session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchKind {
    Insert,
    Update,
    Delete,
}

/// One element a session wrote, with what its record states about itself.
#[derive(Debug, Clone, Copy)]
pub struct Touched {
    pub refno: RefNo,
    pub noun_hash: u32,
    pub owner: RefNo,
    pub kind: TouchKind,
}

/// What the claim of one primary should become.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    /// A new primary: file `(0, session)`.
    Create,
    /// A deleted primary: withdraw its claim.
    Remove,
    /// An existing primary touched: bump its session (no-op if unclaimed).
    Refresh,
}

impl Op {
    /// Precedence when one primary is reached more than one way: a primary
    /// deleted outright wins over anything, and its own creation wins over a
    /// mere refresh reached through a descendant.
    fn rank(self) -> u8 {
        match self {
            Op::Remove => 2,
            Op::Create => 1,
            Op::Refresh => 0,
        }
    }
}

/// The claim edits `touched` implies for session `new_sesno`.
///
/// `is_primary(noun)` says whether a noun is a primary; a noun neither lookup
/// knows is treated as non-primary, so the walk climbs past it. `ancestor(ref)`
/// gives the noun and owner of an element not in `touched` — the file's view of
/// an owner the walk has to climb through; `None` ends the climb (the root, or
/// an owner the file does not hold). The walk is bounded by the number of
/// elements it may pass through so a cyclic owner chain cannot loop forever.
pub fn plan_claim_edits(
    touched: &[Touched],
    new_sesno: u32,
    mut is_primary: impl FnMut(u32) -> bool,
    mut ancestor: impl FnMut(RefNo) -> Option<(u32, RefNo)>,
    max_climb: usize,
) -> Vec<ClaimEdit> {
    // What each written element states about itself, for the owner climb.
    let known: BTreeMap<(u32, u32), (u32, RefNo)> = touched
        .iter()
        .map(|t| ((t.refno.word0, t.refno.word1), (t.noun_hash, t.owner)))
        .collect();

    let mut ops: BTreeMap<(u32, u32), (RefNo, Op)> = BTreeMap::new();
    let mut record = |refno: RefNo, op: Op| {
        ops.entry((refno.word0, refno.word1))
            .and_modify(|(_, existing)| {
                if op.rank() > existing.rank() {
                    *existing = op;
                }
            })
            .or_insert((refno, op));
    };

    for t in touched {
        let itself_primary = is_primary(t.noun_hash);
        if itself_primary {
            match t.kind {
                TouchKind::Insert => record(t.refno, Op::Create),
                TouchKind::Delete => record(t.refno, Op::Remove),
                TouchKind::Update => record(t.refno, Op::Refresh),
            }
            continue;
        }
        // A non-primary element bumps the session of its nearest primary
        // ancestor, if it has one.
        if let Some(primary) = climb_to_primary(t.owner, &known, &mut is_primary, &mut ancestor, max_climb) {
            record(primary, Op::Refresh);
        }
    }

    ops.into_values()
        .map(|(refno, op)| ClaimEdit {
            refno,
            kind: match op {
                Op::Create => ClaimEditKind::Set { owner: 0, session: new_sesno },
                Op::Refresh => ClaimEditKind::Refresh { session: new_sesno },
                Op::Remove => ClaimEditKind::Remove,
            },
        })
        .collect()
}

/// Walk up from `start` to the first primary owner, or `None` when the chain
/// runs off the top or past its budget without one.
fn climb_to_primary(
    start: RefNo,
    known: &BTreeMap<(u32, u32), (u32, RefNo)>,
    is_primary: &mut impl FnMut(u32) -> bool,
    ancestor: &mut impl FnMut(RefNo) -> Option<(u32, RefNo)>,
    max_climb: usize,
) -> Option<RefNo> {
    let mut current = start;
    for _ in 0..max_climb {
        if current.word0 == 0 && current.word1 == 0 {
            return None;
        }
        let (noun, owner) = known
            .get(&(current.word0, current.word1))
            .copied()
            .or_else(|| ancestor(current))?;
        if is_primary(noun) {
            return Some(current);
        }
        current = owner;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const ZONE: u32 = 1;
    const BOX: u32 = 2;
    const SITE: u32 = 3;
    const GROU: u32 = 4;

    fn is_primary(noun: u32) -> bool {
        // ZONE and SITE are primary; BOX and GROU are not.
        noun == ZONE || noun == SITE
    }

    fn r(w1: u32) -> RefNo {
        RefNo::new(100, w1)
    }

    #[test]
    fn a_new_primary_gets_a_claim_and_a_new_box_bumps_its_zone() {
        // ZONE 10 is inserted (a new primary), BOX 11 under it is inserted too.
        let touched = vec![
            Touched { refno: r(10), noun_hash: ZONE, owner: r(1), kind: TouchKind::Insert },
            Touched { refno: r(11), noun_hash: BOX, owner: r(10), kind: TouchKind::Insert },
        ];
        let edits = plan_claim_edits(&touched, 7, is_primary, |_| None, 16);
        // ZONE 10: created. BOX 11: not a key; its primary is ZONE 10, already
        // Create, so no separate refresh.
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].refno, r(10));
        assert_eq!(edits[0].kind, ClaimEditKind::Set { owner: 0, session: 7 });
    }

    #[test]
    fn a_box_under_an_existing_zone_bumps_the_zone_only() {
        // BOX 11 inserted under ZONE 10 which is not itself in the batch; the
        // file says 10 is a ZONE owned by SITE 1.
        let touched = vec![Touched { refno: r(11), noun_hash: BOX, owner: r(10), kind: TouchKind::Insert }];
        let file: HashMap<(u32, u32), (u32, RefNo)> = HashMap::from([((100, 10), (ZONE, r(1)))]);
        let edits = plan_claim_edits(&touched, 9, is_primary, |ref_| file.get(&(ref_.word0, ref_.word1)).copied(), 16);
        assert_eq!(edits, vec![ClaimEdit { refno: r(10), kind: ClaimEditKind::Refresh { session: 9 } }]);
    }

    #[test]
    fn a_non_primary_climbs_more_than_one_owner() {
        // GROU 12 under GROU 13 under ZONE 10: climbs two owners to the ZONE.
        let touched = vec![Touched { refno: r(12), noun_hash: GROU, owner: r(13), kind: TouchKind::Insert }];
        let file: HashMap<(u32, u32), (u32, RefNo)> =
            HashMap::from([((100, 13), (GROU, r(10))), ((100, 10), (ZONE, r(1)))]);
        let edits = plan_claim_edits(&touched, 5, is_primary, |ref_| file.get(&(ref_.word0, ref_.word1)).copied(), 16);
        assert_eq!(edits, vec![ClaimEdit { refno: r(10), kind: ClaimEditKind::Refresh { session: 5 } }]);
    }

    #[test]
    fn deleting_a_primary_removes_it_and_a_child_delete_does_not_resurrect_a_refresh() {
        // ZONE 10 deleted, and its BOX 11 deleted with it: the ZONE's Remove
        // wins over the refresh the child would ask for.
        let touched = vec![
            Touched { refno: r(10), noun_hash: ZONE, owner: r(1), kind: TouchKind::Delete },
            Touched { refno: r(11), noun_hash: BOX, owner: r(10), kind: TouchKind::Delete },
        ];
        let edits = plan_claim_edits(&touched, 8, is_primary, |_| None, 16);
        assert_eq!(edits, vec![ClaimEdit { refno: r(10), kind: ClaimEditKind::Remove }]);
    }

    #[test]
    fn updating_a_primary_refreshes_it() {
        let touched = vec![Touched { refno: r(10), noun_hash: ZONE, owner: r(1), kind: TouchKind::Update }];
        let edits = plan_claim_edits(&touched, 4, is_primary, |_| None, 16);
        assert_eq!(edits, vec![ClaimEdit { refno: r(10), kind: ClaimEditKind::Refresh { session: 4 } }]);
    }

    #[test]
    fn a_non_primary_with_no_primary_ancestor_contributes_nothing() {
        // A GROU whose owner chain never reaches a primary (owner not found).
        let touched = vec![Touched { refno: r(12), noun_hash: GROU, owner: r(99), kind: TouchKind::Insert }];
        let edits = plan_claim_edits(&touched, 1, is_primary, |_| None, 16);
        assert!(edits.is_empty());
    }
}
