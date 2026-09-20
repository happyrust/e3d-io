//! The keyed tables under the element write path (`docs/plans/2026-09-20-dabacon-write-plan.md`
//! §2 阶段 2 第 4 条 / §7 2026-09-20 反向表), on `ams8000_0001`: `TYPEX` and the
//! reverse-reference tables (`SPRE`, `ISPE`, `SPCREF`, …) follow the records
//! the session writes the way `core.dll`'s `sub_5AB91B0` / `sub_5AB3620` keep
//! them — a new element of a `TYPEX`-listing noun gets its field and key, a
//! type-16 reference set files `[target, self]` under the attribute's name
//! (creating the table when the directory has none), a change moves the edge,
//! a delete withdraws every key the element made. Everything is checked back
//! through the read side (`index::reference`, `session::tables`), and a write
//! without templates leaves every table exactly where it was.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::write_fixture::{assert_appended_contiguous, assert_prefix_unchanged, page_word, DbCopy, AMS8000};
use common::DESVIR;
use e3d_attlib::{db1_dehash, db1_hash};
use e3d_io::engine::ScanTier;
use e3d_io::index::reference::ReferenceTable;
use e3d_io::record::element::ParsedElement;
use e3d_io::record::explicit::ExplicitValue;
use e3d_io::record::template::{NounTemplate, TemplateProvider};
use e3d_io::session::{DbView, SessionDate, SessionSelector};
use e3d_io::write::record::{KEYED_TYPE_CODE, REFERENCE_TYPE_CODE};
use e3d_io::write::{
    reference_value, set_reference, table_keys, typex_table_hash, write_element_session, write_element_session_with,
    ElementWrite, RecordDraft, SessionClock, SessionRequest, WriteOptions,
};
use e3d_io::{ReadOnlyEngine, RefNo};

const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;
const PAGE0_COMMIT_WORDS: &[usize] = &[9, 10, 11];

type Key = [u32; 4];

fn clock() -> SessionClock {
    SessionClock::fixed(
        SessionDate { year: 2026, month: 9, day: 20, hour: 22, minute: 0, second: 0, millisecond: 0 },
        SessionDate { year: 2026, month: 9, day: 20, hour: 14, minute: 0, second: 0, millisecond: 0 },
    )
}

fn templates() -> TemplateProvider {
    TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir")
}

fn template(provider: &mut TemplateProvider, noun_hash: u32) -> NounTemplate {
    provider
        .template_for(noun_hash)
        .expect("load template")
        .unwrap_or_else(|| panic!("desvir has no template for {:?}", db1_dehash(noun_hash)))
        .clone()
}

fn name(hash: u32) -> String {
    db1_dehash(hash).unwrap_or_else(|| format!("#{hash}"))
}

/// Every keyed table the session names — the 4-dword-key, valueless ones —
/// as its full key set, read through the reverse-reference reader (`TYPEX`
/// decodes under it too: same page shape). `NAME` and `SPAMAP` are skipped
/// by their header.
fn keyed_tables(view: &mut DbView) -> BTreeMap<u32, BTreeSet<Key>> {
    let mut out = BTreeMap::new();
    for table in view.tables().to_vec() {
        let page = view.pages_mut().get(table.root).expect("root page").to_vec();
        let word = |i: usize| u32::from_be_bytes(page[i * 4..i * 4 + 4].try_into().unwrap());
        if word(3) != 4 || word(4) != 0 {
            continue;
        }
        let keys: BTreeSet<Key> = ReferenceTable::open(view, table.name_hash)
            .expect("open keyed table")
            .iter()
            .map(|entry| {
                let entry = entry.expect("walk keyed table");
                [entry.target.word0, entry.target.word1, entry.referrer.word0, entry.referrer.word1]
            })
            .collect();
        out.insert(table.name_hash, keys);
    }
    out
}

fn roots(view: &DbView) -> BTreeMap<u32, u32> {
    view.tables().iter().map(|t| (t.name_hash, t.root.page)).collect()
}

fn draft_of(engine: &mut ReadOnlyEngine, refno: RefNo) -> RecordDraft {
    let view = engine.find_element(refno).expect("lookup").unwrap_or_else(|| panic!("{refno} is indexed"));
    RecordDraft::from_record_bytes(&view.raw_bytes).expect("draft")
}

/// The first element of `noun`, the largest word1 in use, and the count.
fn survey(engine: &mut ReadOnlyEngine, noun: &str) -> (RefNo, u32, usize) {
    let wanted = db1_hash(noun);
    let mut first = None;
    let mut max_word1 = 0;
    let mut count = 0;
    for element in engine.scan_elements(ScanTier::Header).expect("scan") {
        let element = element.expect("header");
        count += 1;
        max_word1 = max_word1.max(element.refno.word1);
        if first.is_none() && element.noun_hash == wanted {
            first = Some(element.refno);
        }
    }
    (first.unwrap_or_else(|| panic!("the sample has a {noun}")), max_word1, count)
}

fn tally(path: &std::path::Path, from: u32, to: u32) -> (u64, u64, u64) {
    let mut view = DbView::open(path).expect("open");
    let diff = view
        .diff_sessions(SessionSelector::Exact(from), SessionSelector::Exact(to))
        .expect("diff");
    let t = diff.tally();
    (t.deleted, t.inserted, t.modified)
}

/// The file before a write is unchanged up to that write's previous end (bar
/// page 0's commit words), and the new pages follow contiguously.
fn assert_bytes(before: &[u8], copy: &DbCopy, report: &e3d_io::write::CommitReport) {
    let bytes = copy.current_bytes();
    let page_bytes = 4 * page_word(before, 2048, 0, 13) as usize;
    let old_last = report.previous.end.page as usize;
    let new_end = *report.appended_pages.iter().max().unwrap() as usize;
    assert_prefix_unchanged(before, &bytes, page_bytes, old_last, PAGE0_COMMIT_WORDS);
    assert_appended_contiguous(&bytes, page_bytes, old_last, new_end, &[3, 5, 7]);
}

/// A new ZONE under the first SITE, with `ISPE` (type 16, implicit-resident)
/// and `SPCREF` (type 16, explicit-resident, no table in the sample yet) set:
/// the write gives it `TYPEX`, files its key, files both edges — building the
/// `SPCREF` table and listing it behind the eight the sample has — and every
/// other table, page for page, stays.
#[test]
fn a_new_zone_files_its_typex_key_and_its_reference_edges_and_builds_a_missing_table() {
    if common::missing(&[AMS8000, DESVIR]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let mut provider = templates();
    let zone_hash = db1_hash("ZONE");
    let (ispe, spcref) = (db1_hash("ISPE"), db1_hash("SPCREF"));
    let zone_template = template(&mut provider, zone_hash);
    assert!(
        zone_template.attrs.iter().any(|a| a.hash == typex_table_hash() && a.storage_type_code == 14),
        "ZONE's template lists TYPEX"
    );
    for (hash, offset_is_implicit) in [(ispe, true), (spcref, false)] {
        let attr = zone_template.attrs.iter().find(|a| a.hash == hash).expect("ZONE lists the attribute");
        assert_eq!(attr.storage_type_code, 16, "{} is a type-16 reference", name(hash));
        assert_eq!(attr.wide.location.word_offset() != 0, offset_is_implicit, "{} residence", name(hash));
    }

    let (before, roots_before, base_sesno) = {
        let mut view = DbView::open(&copy.path).expect("open");
        (keyed_tables(&mut view), roots(&view), view.session().sesno)
    };
    assert_eq!(roots_before.len(), 8, "the sample names eight tables");
    assert!(before.contains_key(&ispe) && !before.contains_key(&spcref), "ISPE has a table, SPCREF has none yet");
    // A real specification the sample already files ISPE edges under, and a
    // fabricated catalogue refno for SPCREF (targets live in other databases).
    let ispe_target = {
        let first = before[&ispe].iter().next().expect("ISPE has edges");
        RefNo::new(first[0], first[1])
    };
    let spcref_target = RefNo::new(13_244, 424_242);

    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
    let (site, max_word1, count_before) = survey(&mut engine, "SITE");
    let zone = RefNo::new(site.word0, max_word1 + 3000);
    let mut zone_draft = {
        let mut implicit = vec![0u32; 11];
        implicit.extend_from_slice(&zone_template.defaults.implicit);
        RecordDraft::new(zone, zone_hash, site, implicit, vec![], vec![]).expect("zone draft")
    };
    assert!(set_reference(&mut zone_draft, &zone_template, ispe, ispe_target));
    assert!(set_reference(&mut zone_draft, &zone_template, spcref, spcref_target));
    assert_eq!(reference_value(&zone_draft, &zone_template, ispe), Some(ispe_target));
    assert_eq!(reference_value(&zone_draft, &zone_template, spcref), Some(spcref_target));
    assert!(zone_draft.explicit_words(typex_table_hash()).is_none(), "the caller did not add TYPEX");
    let mut site_draft = draft_of(&mut engine, site);
    site_draft.push_member(zone);
    drop(engine);

    let report = write_element_session_with(
        &copy.path,
        &SessionRequest { user: "e3dio", comment: "e3d-io tables: a zone with ISPE and SPCREF", clock: Some(clock()) },
        &[ElementWrite::Insert(zone_draft), ElementWrite::Update(site_draft)],
        &[],
        WriteOptions { templates: Some(&mut provider) },
    )
    .expect("write");
    assert_eq!(report.written.sesno, base_sesno + 1);
    assert_eq!(tally(&copy.path, base_sesno, base_sesno + 1), (0, 1, 1));

    // --- the directory: nine tables, SPCREF appended, a fresh leaf root ---
    let mut view = DbView::open(&copy.path).expect("reopen");
    let roots_after = roots(&view);
    assert_eq!(view.tables().len(), 9, "SPCREF was added to the directory");
    assert_eq!(view.tables()[8].name_hash, spcref, "behind the existing eight");
    let spcref_root = view.table_root(spcref).expect("SPCREF root");
    {
        let page = view.pages_mut().get(spcref_root).expect("root").to_vec();
        let word = |i: usize| u32::from_be_bytes(page[i * 4..i * 4 + 4].try_into().unwrap());
        assert_eq!(
            [word(0), word(1), word(2), word(3), word(4), word(5)],
            [5, spcref, 0, 4, 0, 0],
            "page type 5, the name hash, a leaf, key 4, no value, flags 0 — sub_5AFF6D0's header"
        );
    }
    for (table, root) in &roots_before {
        if *table == typex_table_hash() || *table == ispe {
            assert_ne!(roots_after[table], *root, "{} was rewritten", name(*table));
        } else {
            assert_eq!(roots_after[table], *root, "{} was not touched, not even copied", name(*table));
        }
    }

    // --- the keys: exactly the three new ones, nothing else moved ---
    let after = keyed_tables(&mut view);
    let typex_key: Key = [zone_hash, zone.word0, zone.word1, 0];
    let ispe_key: Key = [ispe_target.word0, ispe_target.word1, zone.word0, zone.word1];
    let spcref_key: Key = [spcref_target.word0, spcref_target.word1, zone.word0, zone.word1];
    for (table, keys) in &before {
        let mut expected = keys.clone();
        if *table == typex_table_hash() {
            expected.insert(typex_key);
        }
        if *table == ispe {
            expected.insert(ispe_key);
        }
        assert_eq!(&after[table], &expected, "{} keys", name(*table));
    }
    assert_eq!(after[&spcref], BTreeSet::from([spcref_key]), "the new table holds the one edge");

    // --- through the reader's own question ---
    let referrers = ReferenceTable::open(&mut view, spcref).unwrap().referrers_of(spcref_target).unwrap();
    assert_eq!(referrers, vec![zone]);
    let referrers = ReferenceTable::open(&mut view, ispe).unwrap().referrers_of(ispe_target).unwrap();
    assert!(referrers.contains(&zone), "the zone is among {ispe_target}'s ISPE referrers");
    drop(view);

    // --- the record: TYPEX first in the stream, the two references where set ---
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open written");
    let record = engine.find_element(zone).expect("lookup").expect("indexed");
    let parsed = ParsedElement::parse_as(&record.raw_bytes, engine.text_format()).expect("parse");
    assert_eq!(parsed.explicit_attributes[0].hash, typex_table_hash(), "TYPEX opens the explicit stream");
    assert_eq!(parsed.explicit_attributes[0].type_code, KEYED_TYPE_CODE);
    assert_eq!(parsed.explicit_attributes[0].value, ExplicitValue::RawWords(vec![1, zone_hash]));
    let spcref_field = parsed.explicit_attributes.iter().find(|a| a.hash == spcref).expect("SPCREF stored");
    assert_eq!(spcref_field.type_code, REFERENCE_TYPE_CODE);
    assert_eq!(spcref_field.value, ExplicitValue::RefNo(spcref_target));
    let written = RecordDraft::from_record_bytes(&record.raw_bytes).unwrap();
    assert_eq!(reference_value(&written, &zone_template, ispe), Some(ispe_target));
    assert_eq!(
        table_keys(&written, &zone_template),
        BTreeSet::from([(typex_table_hash(), typex_key), (ispe, ispe_key), (spcref, spcref_key)])
    );
    let (_, _, count_after) = survey(&mut engine, "SITE");
    assert_eq!(count_after, count_before + 1);

    assert_bytes(&copy.original, &copy, &report);
}

/// Take the first `SPRE` edge `[A, R]`: pointing `R`'s `SPRE` at `B` moves the
/// edge from `A` to `B` and touches no other table; deleting `R` then
/// withdraws every key `R` made — its `SPRE` edge and its `TYPEX` key among
/// them — and nothing that belongs to any other element.
#[test]
fn changing_a_reference_moves_its_edge_and_deleting_the_element_withdraws_all_its_keys() {
    if common::missing(&[AMS8000, DESVIR]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let mut provider = templates();
    let spre = db1_hash("SPRE");

    let (before, roots_before, sesno) = {
        let mut view = DbView::open(&copy.path).expect("open");
        (keyed_tables(&mut view), roots(&view), view.session().sesno)
    };
    let spre_keys = &before[&spre];
    let first = spre_keys.iter().next().expect("SPRE has edges");
    let (a, r) = (RefNo::new(first[0], first[1]), RefNo::new(first[2], first[3]));
    let last = spre_keys.iter().next_back().unwrap();
    let mut b = RefNo::new(last[0], last[1]);
    if b == a {
        b = RefNo::new(a.word0, a.word1 + 1);
    }

    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
    let r_draft = draft_of(&mut engine, r);
    let r_template = template(&mut provider, r_draft.noun_hash);
    assert_eq!(reference_value(&r_draft, &r_template, spre), Some(a), "the edge says what the record says");
    let r_keys_before = table_keys(&r_draft, &r_template);
    assert!(r_keys_before.contains(&(spre, *first)));
    let r_has_typex = r_keys_before.iter().any(|(table, _)| *table == typex_table_hash());
    // Every key R makes is in its table now.
    for (table, key) in &r_keys_before {
        assert!(before[table].contains(key), "{} holds {key:?} before", name(*table));
    }
    let owner = r_draft.owner;
    drop(engine);

    // --- S+1: R's SPRE goes from A to B ---
    let mut changed = r_draft.clone();
    assert!(set_reference(&mut changed, &r_template, spre, b));
    let report = write_element_session_with(
        &copy.path,
        &SessionRequest { user: "e3dio", comment: "e3d-io tables: move an SPRE edge", clock: Some(clock()) },
        &[ElementWrite::Update(changed)],
        &[],
        WriteOptions { templates: Some(&mut provider) },
    )
    .expect("write S+1");
    assert_eq!(tally(&copy.path, sesno, sesno + 1), (0, 0, 1));
    assert_bytes(&copy.original, &copy, &report);
    let bytes_after1 = copy.current_bytes();

    let mut view = DbView::open(&copy.path).expect("reopen");
    let after1 = keyed_tables(&mut view);
    let roots_after1 = roots(&view);
    let moved: Key = [b.word0, b.word1, r.word0, r.word1];
    for (table, keys) in &before {
        let mut expected = keys.clone();
        if *table == spre {
            expected.remove(first);
            expected.insert(moved);
        }
        assert_eq!(&after1[table], &expected, "{} keys after the change", name(*table));
        if *table != spre {
            assert_eq!(roots_after1[table], roots_before[table], "{} was not even copied", name(*table));
        }
    }
    let mut table = ReferenceTable::open(&mut view, spre).unwrap();
    assert!(!table.referrers_of(a).unwrap().contains(&r), "{r} no longer references {a}");
    assert!(table.referrers_of(b).unwrap().contains(&r), "{r} now references {b}");
    drop(view);

    // --- S+2: R is deleted (and taken out of its owner's members) ---
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
    let mut owner_draft = draft_of(&mut engine, owner);
    assert!(owner_draft.remove_member(r), "{owner} lists {r}");
    drop(engine);
    let report = write_element_session_with(
        &copy.path,
        &SessionRequest { user: "e3dio", comment: "e3d-io tables: delete a referrer", clock: Some(clock()) },
        &[ElementWrite::Delete(r), ElementWrite::Update(owner_draft)],
        &[],
        WriteOptions { templates: Some(&mut provider) },
    )
    .expect("write S+2");
    assert_eq!(tally(&copy.path, sesno + 1, sesno + 2), (1, 0, 1));
    assert_bytes(&bytes_after1, &copy, &report);

    let mut view = DbView::open(&copy.path).expect("reopen");
    let after2 = keyed_tables(&mut view);
    let mut withdrawn = 0usize;
    for (table, keys) in &after1 {
        let expected: BTreeSet<Key> = keys
            .iter()
            .copied()
            .filter(|key| {
                let mine = if *table == typex_table_hash() {
                    (key[1], key[2]) == (r.word0, r.word1)
                } else {
                    (key[2], key[3]) == (r.word0, r.word1)
                };
                withdrawn += usize::from(mine);
                !mine
            })
            .collect();
        assert_eq!(&after2[table], &expected, "{} keys after the delete", name(*table));
    }
    let expected_withdrawn = 1 + usize::from(r_has_typex);
    assert!(
        withdrawn >= expected_withdrawn,
        "R made at least {expected_withdrawn} keys (its SPRE edge{}) and every one went",
        if r_has_typex { " and its TYPEX key" } else { "" }
    );
    assert!(!ReferenceTable::open(&mut view, spre).unwrap().referrers_of(b).unwrap().contains(&r));
    drop(view);
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
    assert!(engine.find_element(r).unwrap().is_none(), "{r} is gone from the index");
}

/// A `BOX` — whose template lists no `TYPEX` — gets neither field nor key,
/// like the 45 boxes of the sample; and a write **without** templates, even of
/// a ZONE with a reference set, leaves every table page where it was.
#[test]
fn a_noun_without_typex_gets_no_key_and_a_write_without_templates_moves_no_table() {
    if common::missing(&[AMS8000, DESVIR]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let mut provider = templates();
    let (box_hash, zone_hash) = (db1_hash("BOX"), db1_hash("ZONE"));
    let box_template = template(&mut provider, box_hash);
    assert!(!box_template.attrs.iter().any(|a| a.hash == typex_table_hash()), "BOX lists no TYPEX");
    let zone_template = template(&mut provider, zone_hash);

    let (before, roots_before, sesno) = {
        let mut view = DbView::open(&copy.path).expect("open");
        (keyed_tables(&mut view), roots(&view), view.session().sesno)
    };
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
    let (zone, max_word1, _) = survey(&mut engine, "ZONE");
    let (site, _, _) = survey(&mut engine, "SITE");
    let new_box = RefNo::new(zone.word0, max_word1 + 4000);
    let new_zone = RefNo::new(zone.word0, max_word1 + 4001);

    // --- S+1, with templates: a BOX ---
    let box_draft = {
        let mut implicit = vec![0u32; 11];
        implicit.extend_from_slice(&box_template.defaults.implicit);
        RecordDraft::new(new_box, box_hash, zone, implicit, vec![], vec![]).unwrap()
    };
    let mut zone_draft = draft_of(&mut engine, zone);
    zone_draft.push_member(new_box);
    drop(engine);
    write_element_session_with(
        &copy.path,
        &SessionRequest { user: "e3dio", comment: "e3d-io tables: a box", clock: Some(clock()) },
        &[ElementWrite::Insert(box_draft), ElementWrite::Update(zone_draft)],
        &[],
        WriteOptions { templates: Some(&mut provider) },
    )
    .expect("write S+1");
    assert_eq!(tally(&copy.path, sesno, sesno + 1), (0, 1, 1));
    {
        let mut view = DbView::open(&copy.path).expect("reopen");
        assert_eq!(roots(&view), roots_before, "no table was copied for a BOX");
        assert_eq!(keyed_tables(&mut view), before);
        drop(view);
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let record = engine.find_element(new_box).unwrap().expect("box indexed");
        let parsed = ParsedElement::parse_as(&record.raw_bytes, engine.text_format()).unwrap();
        assert!(!parsed.explicit_attributes.iter().any(|a| a.hash == typex_table_hash()), "no TYPEX on a BOX");
    }

    // --- S+2, without templates: a ZONE with ISPE set, tables untouched ---
    let ispe_target = RefNo::new(13_245, 918_008);
    let mut zone2 = {
        let mut implicit = vec![0u32; 11];
        implicit.extend_from_slice(&zone_template.defaults.implicit);
        RecordDraft::new(new_zone, zone_hash, site, implicit, vec![], vec![]).unwrap()
    };
    assert!(set_reference(&mut zone2, &zone_template, db1_hash("ISPE"), ispe_target));
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
    let mut site_draft = draft_of(&mut engine, site);
    site_draft.push_member(new_zone);
    drop(engine);
    write_element_session(
        &copy.path,
        &SessionRequest { user: "e3dio", comment: "e3d-io tables: a zone, no templates", clock: Some(clock()) },
        &[ElementWrite::Insert(zone2), ElementWrite::Update(site_draft)],
        &[],
    )
    .expect("write S+2");
    assert_eq!(tally(&copy.path, sesno + 1, sesno + 2), (0, 1, 1));
    let mut view = DbView::open(&copy.path).expect("reopen");
    assert_eq!(roots(&view), roots_before, "without templates every table root is carried over");
    assert_eq!(keyed_tables(&mut view), before);
    drop(view);
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
    let record = engine.find_element(new_zone).unwrap().expect("zone indexed");
    let parsed = ParsedElement::parse_as(&record.raw_bytes, engine.text_format()).unwrap();
    assert!(
        !parsed.explicit_attributes.iter().any(|a| a.hash == typex_table_hash()),
        "without templates the writer does not know ZONE lists TYPEX"
    );
}
