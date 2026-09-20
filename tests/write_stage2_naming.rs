//! Stage 2 of the write plan (`docs/plans/2026-09-20-dabacon-write-plan.md` §2
//! 阶段 2 用例 D) on `ams8000_0001`: renaming an element rewrites its `NAME`
//! attribute and maintains the `NAME` table — the variable-key B+tree — so the
//! new name is found by one descent, the old name is gone, and the table still
//! agrees with the elements. A second element taking a name already in use is
//! refused as conflict 561, and the file is left untouched.

mod common;

use std::collections::BTreeMap;

use common::write_fixture::{assert_appended_contiguous, assert_prefix_unchanged, page_word, DbCopy, AMS8000};
use e3d_attlib::db1_hash;
use e3d_io::engine::ScanTier;
use e3d_io::index::{encode_name, name_table_hash};
use e3d_io::record::element::ParsedElement;
use e3d_io::record::explicit::ExplicitValue;
use e3d_io::session::{DbView, SessionDate, SessionSelector};
use e3d_io::write::{
    write_element_session, ElementWrite, NamedCowError, NamedEdit, NamedEditKind, RecordDraft,
    SessionClock, SessionRequest, WriteError,
};
use e3d_io::{ReadOnlyEngine, RefNo};

const PAGE0_COMMIT_WORDS: &[usize] = &[9, 10, 11];

fn fixed_clock() -> SessionClock {
    SessionClock::fixed(
        SessionDate { year: 2026, month: 9, day: 20, hour: 16, minute: 0, second: 0, millisecond: 0 },
        SessionDate { year: 2026, month: 9, day: 20, hour: 8, minute: 0, second: 0, millisecond: 0 },
    )
}

/// The element's stored `NAME` — with its leading `/`, the way the table keys
/// it — read straight off the record's explicit stream.
fn stored_name(engine: &mut ReadOnlyEngine, refno: RefNo) -> Option<String> {
    let name_hash = db1_hash("NAME");
    let text = engine.text_format();
    let view = engine.find_element(refno).ok()??;
    let parsed = ParsedElement::parse_as(&view.raw_bytes, text).ok()?;
    parsed.explicit_attributes.iter().find_map(|a| match &a.value {
        ExplicitValue::Text(t) if a.hash == name_hash => Some(t.clone()),
        _ => None,
    })
}

/// The `(name, refno)` map the table walk yields.
fn table_map(engine: &mut ReadOnlyEngine) -> BTreeMap<String, RefNo> {
    let mut table = engine.name_table().expect("a design database keeps a NAME table");
    table.iter().map(|e| e.expect("a readable page")).map(|e| (e.name(), e.refno)).collect()
}

/// The `(name, refno)` map read off the live elements, keyed the way the table
/// does (leading `/`).
fn element_name_map(engine: &mut ReadOnlyEngine) -> BTreeMap<String, RefNo> {
    let mut out = BTreeMap::new();
    for element in engine.scan_elements(ScanTier::Named).expect("scan") {
        let element = element.expect("named element");
        if let Some(name) = element.name {
            let key = if name.starts_with('/') { name } else { format!("/{name}") };
            out.insert(key, element.refno);
        }
    }
    out
}

#[test]
fn renaming_elements_moves_them_in_the_name_table() {
    if common::missing(&[AMS8000]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let name_hash = db1_hash("NAME");

    // Three named elements, their current names, and the base session facts.
    let (renames, base_sesno, old_end): (Vec<(RefNo, String, String)>, u32, u32) = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let mut named: Vec<RefNo> = Vec::new();
        for element in engine.scan_elements(ScanTier::Named).expect("scan") {
            let element = element.expect("element");
            if element.name.is_some() && named.len() < 3 {
                named.push(element.refno);
            }
        }
        assert_eq!(named.len(), 3, "the sample has at least three named elements");
        let renames: Vec<(RefNo, String, String)> = named
            .into_iter()
            .enumerate()
            .map(|(index, refno)| {
                let old = stored_name(&mut engine, refno).expect("a stored NAME");
                (refno, old, format!("/E3DIO-RENAMED-{index}"))
            })
            .collect();
        // The new names must be free.
        for (_, _, new) in &renames {
            assert!(engine.find_by_name(new).expect("lookup").is_none(), "{new} must be unused");
        }
        let view = DbView::open(&copy.path).unwrap();
        (renames, view.session().sesno, view.session().end.page)
    };
    let table_before = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        table_map(&mut engine)
    };

    // Build the element updates (NAME attribute) and the table edits.
    let mut elements: Vec<ElementWrite> = Vec::new();
    let mut name_edits: Vec<NamedEdit> = Vec::new();
    {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        for (refno, old, new) in &renames {
            let view = engine.find_element(*refno).expect("read").expect("exists");
            let mut draft = RecordDraft::from_record_bytes(&view.raw_bytes).expect("draft");
            assert!(draft.set_explicit_text_ascii(name_hash, new).expect("ascii"), "{refno} carries a NAME");
            elements.push(ElementWrite::Update(draft));
            name_edits.push(NamedEdit { key: encode_name(old), refno: *refno, kind: NamedEditKind::Delete });
            name_edits.push(NamedEdit { key: encode_name(new), refno: *refno, kind: NamedEditKind::Insert });
        }
    }

    let report = write_element_session(
        &copy.path,
        &SessionRequest { user: "e3dio", comment: "e3d-io stage2 rename", clock: Some(fixed_clock()) },
        &elements,
        &name_edits,
    )
    .expect("write the rename session");
    let new_end = *report.appended_pages.iter().max().unwrap();

    // --- the diff: three modifies ---
    let mut view = DbView::open(&copy.path).expect("reopen");
    let diff = view
        .diff_sessions(SessionSelector::Exact(base_sesno), SessionSelector::Exact(base_sesno + 1))
        .expect("diff");
    assert_eq!((diff.tally().deleted, diff.tally().inserted, diff.tally().modified), (0, 0, 3));
    drop(view);

    // --- the table: new names found, old names gone, everything consistent ---
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open written");
    for (refno, old, new) in &renames {
        assert_eq!(engine.find_by_name(new).expect("lookup new"), Some(*refno), "{new} names {refno}");
        assert_eq!(engine.find_by_name(old).expect("lookup old"), None, "{old} is gone");
        assert_eq!(stored_name(&mut engine, *refno).as_deref(), Some(new.as_str()), "the record's NAME is the new one");
    }
    let table_after = table_map(&mut engine);
    let elements_after = element_name_map(&mut engine);
    assert_eq!(table_after, elements_after, "the table and the elements still agree name for name");
    assert_eq!(table_after.len(), table_before.len(), "a rename neither gains nor loses a name");
    for (_, old, new) in &renames {
        assert!(!table_after.contains_key(old), "{old} left the table");
        assert!(table_after.contains_key(new), "{new} joined it");
    }

    // --- the bytes ---
    let bytes = copy.current_bytes();
    let page_bytes = 4 * page_word(&copy.original, 2048, 0, 13) as usize;
    assert_prefix_unchanged(&copy.original, &bytes, page_bytes, old_end as usize, PAGE0_COMMIT_WORDS);
    assert_appended_contiguous(&bytes, page_bytes, old_end as usize, new_end as usize, &[3, 5, 7]);
}

#[test]
fn a_name_already_taken_is_refused_and_nothing_is_written() {
    if common::missing(&[AMS8000]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);

    // Two distinct named elements: X will try to take Y's name.
    let (x, y_name, y) = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let mut named: Vec<RefNo> = Vec::new();
        for element in engine.scan_elements(ScanTier::Named).expect("scan") {
            let element = element.expect("element");
            if element.name.is_some() && named.len() < 2 {
                named.push(element.refno);
            }
        }
        let x = named[0];
        let y = named[1];
        let y_name = stored_name(&mut engine, y).expect("Y has a NAME");
        (x, y_name, y)
    };

    // Insert Y's name pointing at X: the table already holds it, naming Y.
    let name_edits = vec![NamedEdit {
        key: encode_name(&y_name),
        refno: x,
        kind: NamedEditKind::Insert,
    }];
    let result = write_element_session(
        &copy.path,
        &SessionRequest { user: "e3dio", comment: "duplicate name", clock: Some(fixed_clock()) },
        &[],
        &name_edits,
    );
    match result {
        Err(WriteError::NameTable(NamedCowError::Duplicate { existing, .. })) => {
            assert_eq!(existing, y, "the name still names Y");
        }
        other => panic!("a duplicate name must be refused, got {other:?}"),
    }
    assert_eq!(copy.current_bytes(), copy.original, "a refused write changes no bytes");
    let _ = name_table_hash();
}
