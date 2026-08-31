//! What gen-model's generation chain asks for, and what this crate answers.
//!
//! Reconnaissance for `docs/plans/2026-08-30-e3d-io-gen-model-gap.md` in
//! gen-model. Each subcommand answers one question the generation chain
//! raises, on real databases, and prints counts rather than opinions.
//!
//! ```text
//! cargo run --release --example genmodel_gap_probe -- session  [db]
//! cargo run --release --example genmodel_gap_probe -- crossdb  [db] [sample]
//! cargo run --release --example genmodel_gap_probe -- children [db] [sample]
//! cargo run --release --example genmodel_gap_probe -- deep     [db] [root] [noun]
//! cargo run --release --example genmodel_gap_probe -- perf     [db] [sample]
//! cargo run --release --example genmodel_gap_probe -- uda      [design_db]
//! cargo run --release --example genmodel_gap_probe -- expr     [catalogue_db] [sample]
//! ```

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use e3d_io::index::IndexCursor;
use e3d_io::page::PageId;
use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::record::element::ParsedElement;
use e3d_io::record::read_record;
use e3d_io::record::template::TemplateProvider;
use e3d_io::refno::RefNo;
use e3d_io::session::{DbView, SessionSelector};
use e3d_io::uda_catalog::UdaCatalog;
use e3d_io::ReadOnlyEngine;

const AMS_DIR: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";
const DESIGN_DB: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";
const CATALOGUE_DB: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams6100_0001";
const DICTIONARY_DB: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5100_0001";
const ATTLIB: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all\attlib.dat";
const DESVIR: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all\desvir.dat";
const DICVIR: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all\dicvir.dat";
const CATVIR: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all\catvir.dat";
const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;

fn main() {
    let mut args = std::env::args().skip(1);
    let what = args.next().unwrap_or_else(|| "session".to_string());
    let rest: Vec<String> = args.collect();
    match what.as_str() {
        "session" => session(arg(&rest, 0, DESIGN_DB)),
        "crossdb" => crossdb(arg(&rest, 0, DESIGN_DB), num(&rest, 1, usize::MAX)),
        "children" => children(arg(&rest, 0, DESIGN_DB), num(&rest, 1, usize::MAX)),
        "deep" => deep(arg(&rest, 0, DESIGN_DB), rest.get(1), rest.get(2)),
        "perf" => perf(arg(&rest, 0, DESIGN_DB), num(&rest, 1, 200)),
        "uda" => uda(arg(&rest, 0, DESIGN_DB)),
        "expr" => expr(arg(&rest, 0, CATALOGUE_DB), num(&rest, 1, 4000)),
        other => println!("unknown subcommand {other}"),
    }
}

fn arg(rest: &[String], at: usize, fallback: &str) -> PathBuf {
    PathBuf::from(
        rest.get(at)
            .cloned()
            .unwrap_or_else(|| fallback.to_string()),
    )
}

fn num(rest: &[String], at: usize, fallback: usize) -> usize {
    rest.get(at)
        .and_then(|value| value.parse().ok())
        .unwrap_or(fallback)
}

/// Every RefNo the selected session indexes, with the address of its record.
fn live_keys(view: &mut DbView) -> Vec<(RefNo, PageId, u32)> {
    let root = view.index_root();
    let mut out = Vec::new();
    let mut cursor = IndexCursor::at_root(view.pages_mut(), root);
    for found in cursor.walk() {
        let found = found.expect("the live tree walks");
        out.push((found.refno, found.loc.page, found.loc.offset_words));
    }
    out
}

// ---------------------------------------------------------------- session pin

/// ADR-053 Q3 pins reads at `applied_sesno`. Which entry points can do that?
fn session(db: PathBuf) {
    let mut latest = DbView::open(&db).expect("open at the newest session");
    let sessions = latest.walk_sessions().expect("walk the session chain");
    println!("db={}", db.display());
    println!("sessions={}", sessions.len());
    println!(
        "newest sesno={} index_root={:?}",
        latest.selected_session(),
        latest.index_root()
    );

    // A DbView can be pinned. Take a session about a third of the way back and
    // show that it descends a different tree with a different key set.
    let older = sessions
        .get(sessions.len() / 3)
        .map(|session| session.sesno)
        .unwrap_or_else(|| latest.selected_session());
    let mut pinned =
        DbView::open_at(&db, SessionSelector::Exact(older)).expect("open at an older session");
    println!(
        "pinned  sesno={} index_root={:?} sessions_read={}",
        pinned.selected_session(),
        pinned.index_root(),
        pinned.sessions_read()
    );

    let newest_keys: HashSet<RefNo> = live_keys(&mut latest).into_iter().map(|k| k.0).collect();
    let pinned_keys: HashSet<RefNo> = live_keys(&mut pinned).into_iter().map(|k| k.0).collect();
    println!(
        "keys newest={} pinned={} only-in-newest={} only-in-pinned={}",
        newest_keys.len(),
        pinned_keys.len(),
        newest_keys.difference(&pinned_keys).count(),
        pinned_keys.difference(&newest_keys).count()
    );

    // Reading one element as of the pinned session, without ReadOnlyEngine.
    let sample = live_keys(&mut pinned).into_iter().next();
    match sample {
        Some((_, page, offset_words)) => {
            let record = read_record(pinned.pages_mut(), page, offset_words)
                .expect("read a record at the pinned session");
            println!(
                "pinned read ok: {} noun=0x{:08X} bytes={} blocks={}",
                record.refno,
                record.noun_hash,
                record.bytes.len(),
                record.blocks
            );
        }
        None => println!("pinned session indexes nothing"),
    }

    // The high-level API takes no session.
    let engine = ReadOnlyEngine::open(&db).expect("open the engine");
    println!(
        "ReadOnlyEngine::open selects sesno={} (no open_at exists on it)",
        engine.session().sesno
    );
}

// ------------------------------------------------------------ cross-database

/// Owners and reference-valued attributes that leave this database.
fn crossdb(db: PathBuf, sample: usize) {
    let dir = db.parent().map(Path::to_path_buf).unwrap_or_default();
    let present: BTreeSet<u32> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(|entry| dbnum_of_file(&entry.ok()?.file_name().to_string_lossy()))
                .collect()
        })
        .unwrap_or_default();

    let mut view = DbView::open(&db).expect("open at the newest session");
    let own_db = view.descriptor().db_mark;
    let keys = live_keys(&mut view);
    let total = keys.len().min(sample);
    println!("db={} db_mark={} keys={}", db.display(), own_db, keys.len());
    println!(
        "sibling database files in {}: {}",
        dir.display(),
        present.len()
    );

    let mut owners_out = 0usize;
    let mut owners_out_missing_file = 0usize;
    let mut owner_targets: BTreeMap<u32, usize> = BTreeMap::new();
    let mut refs_out: BTreeMap<u32, usize> = BTreeMap::new();
    let mut refs_out_missing_file = 0usize;
    let mut examples: Vec<String> = Vec::new();
    let mut nouns_with_outbound: BTreeMap<u32, usize> = BTreeMap::new();

    for (refno, page, offset_words) in keys.into_iter().take(total) {
        let Ok(record) = read_record(view.pages_mut(), page, offset_words) else {
            continue;
        };
        let self_db = refno.dbno();
        if record.owner.is_valid() && record.owner.dbno() != self_db {
            owners_out += 1;
            *owner_targets.entry(record.owner.dbno()).or_default() += 1;
            if !present.contains(&record.owner.dbno()) {
                owners_out_missing_file += 1;
            }
            if examples.len() < 8 {
                examples.push(format!(
                    "owner out: {} (noun 0x{:08X}) -> {} in db {}",
                    refno,
                    record.noun_hash,
                    record.owner,
                    record.owner.dbno()
                ));
            }
        }

        // Reference-valued words anywhere in the implicit region. Reading the
        // whole region rather than named attributes keeps this independent of
        // the descriptor tables, which is the point: it measures how much of
        // the graph leaves the file, not how well one schema is loaded.
        let Ok(parsed) = ParsedElement::parse(&record.bytes) else {
            continue;
        };
        let mut left = false;
        for pair in parsed.implicit_data[24..].chunks_exact(8) {
            let candidate = RefNo::new(
                u32::from_be_bytes(pair[0..4].try_into().unwrap()),
                u32::from_be_bytes(pair[4..8].try_into().unwrap()),
            );
            if !plausible_refno(candidate) || candidate.dbno() == self_db {
                continue;
            }
            *refs_out.entry(candidate.dbno()).or_default() += 1;
            if !present.contains(&candidate.dbno()) {
                refs_out_missing_file += 1;
            }
            left = true;
        }
        if left {
            *nouns_with_outbound.entry(record.noun_hash).or_default() += 1;
        }
    }

    println!("records examined: {total}");
    println!(
        "owner leaves this database: {owners_out} (target file absent: {owners_out_missing_file})"
    );
    println!("  owner target databases: {owner_targets:?}");
    let refs_total: usize = refs_out.values().sum();
    println!("implicit reference words leaving this database: {refs_total} (target file absent: {refs_out_missing_file})");
    println!("  top targets: {:?}", top(&refs_out, 12));
    println!(
        "nouns with outbound references: {}",
        nouns_with_outbound.len()
    );
    println!("  top nouns: {:?}", top_nouns(&nouns_with_outbound, 12));
    for line in examples {
        println!("  {line}");
    }

    // Same question through the descriptor tables, so the outbound references
    // arrive with their attribute names. The word scan above cannot tell a
    // reference from two integers that happen to look like one.
    if !Path::new(ATTLIB).exists() || !Path::new(DESVIR).exists() {
        println!("attlib/desvir absent: skipping the named-attribute pass");
        return;
    }
    let attlib = e3d_attlib::AttlibData::parse_file(ATTLIB).expect("parse attlib");
    let mut provider =
        TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("open desvir");
    let mut engine = ReadOnlyEngine::open(&db).expect("open the engine");
    let engine_keys = engine.indexed_refnos().expect("walk the live tree");
    let mut named_out: BTreeMap<String, usize> = BTreeMap::new();
    let mut named_in: BTreeMap<String, usize> = BTreeMap::new();
    let mut named_examples: BTreeMap<String, String> = BTreeMap::new();
    for (refno, _, _) in engine_keys.iter().take(total) {
        let Ok(extraction) =
            engine.extract_element_with_descriptors(*refno, &attlib, &mut provider)
        else {
            continue;
        };
        for attribute in &extraction.attributes {
            let targets: Vec<RefNo> = match &attribute.value {
                Some(DescriptorValue::RefNo(target)) => vec![*target],
                Some(DescriptorValue::RefNoArray(list)) => list.clone(),
                _ => continue,
            };
            for target in targets {
                if !target.is_valid() {
                    continue;
                }
                if target.dbno() == own_db {
                    *named_in.entry(attribute.name.clone()).or_default() += 1;
                } else {
                    *named_out.entry(attribute.name.clone()).or_default() += 1;
                    named_examples
                        .entry(attribute.name.clone())
                        .or_insert(format!("{refno} -> {target} (db {})", target.dbno()));
                }
            }
        }
    }
    println!("-- named reference attributes --");
    println!(
        "pointing inside this database: {}",
        named_in.values().sum::<usize>()
    );
    println!(
        "pointing at another database:  {}",
        named_out.values().sum::<usize>()
    );
    let mut out: Vec<(&String, &usize)> = named_out.iter().collect();
    out.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
    for (name, count) in out.into_iter().take(16) {
        println!(
            "  {name:<12} {count:>6}   e.g. {}",
            named_examples.get(name).map(String::as_str).unwrap_or("")
        );
    }
    println!("NOTE e3d-io opens one file; nothing here resolves a target in another file.");
}

/// A word pair that could be an element reference rather than two numbers.
fn plausible_refno(candidate: RefNo) -> bool {
    candidate.is_valid()
        && candidate.word1 > 0
        && candidate.word1 < 10_000_000
        && candidate.dbno() > 0
        && candidate.dbno() < 300_000
        && candidate.extent() > 0
        && candidate.extent() < 64
}

fn dbnum_of_file(name: &str) -> Option<u32> {
    let bytes = name.as_bytes();
    if !name.starts_with("ams") || bytes.len() < 9 || bytes[bytes.len() - 5] != b'_' {
        return None;
    }
    name[3..name.len() - 5].parse().ok()
}

// ------------------------------------------------------------- children order

/// Is the member list the children order, and is it complete?
fn children(db: PathBuf, sample: usize) {
    let mut view = DbView::open(&db).expect("open at the newest session");
    let keys = live_keys(&mut view);
    let indexed: HashSet<RefNo> = keys.iter().map(|k| k.0).collect();
    let take = keys.len().min(sample);

    let mut members_of: HashMap<RefNo, Vec<RefNo>> = HashMap::new();
    let mut owner_of: HashMap<RefNo, RefNo> = HashMap::new();
    let mut parsed_ok = 0usize;
    for (refno, page, offset_words) in keys.iter().take(take) {
        let Ok(record) = read_record(view.pages_mut(), *page, *offset_words) else {
            continue;
        };
        let Ok(parsed) = ParsedElement::parse(&record.bytes) else {
            continue;
        };
        parsed_ok += 1;
        owner_of.insert(*refno, parsed.owner);
        if !parsed.members.is_empty() {
            members_of.insert(*refno, parsed.members);
        }
    }

    // Rebuild children from the owner back-pointers, the way a reader that
    // grouped an index walk by owner would, and compare with the stated list.
    let mut by_owner: HashMap<RefNo, Vec<RefNo>> = HashMap::new();
    for (child, owner) in &owner_of {
        by_owner.entry(*owner).or_default().push(*child);
    }
    for list in by_owner.values_mut() {
        list.sort_by_key(|refno| (refno.word0, refno.word1));
    }

    let mut same_set_same_order = 0usize;
    let mut same_set_other_order = 0usize;
    let mut different_set = 0usize;
    let mut members_not_indexed = 0usize;
    let mut members_owned_by_someone_else = 0usize;
    let mut duplicated_members = 0usize;
    let mut order_examples: Vec<String> = Vec::new();

    for (parent, stated) in &members_of {
        for member in stated {
            if !indexed.contains(member) {
                members_not_indexed += 1;
            } else if owner_of.get(member).is_some_and(|owner| owner != parent) {
                members_owned_by_someone_else += 1;
            }
        }
        let mut seen = HashSet::new();
        duplicated_members += stated.iter().filter(|m| !seen.insert(**m)).count();

        let empty = Vec::new();
        let rebuilt = by_owner.get(parent).unwrap_or(&empty);
        let stated_set: BTreeSet<_> = stated.iter().map(|r| (r.word0, r.word1)).collect();
        let rebuilt_set: BTreeSet<_> = rebuilt.iter().map(|r| (r.word0, r.word1)).collect();
        if stated_set != rebuilt_set {
            different_set += 1;
            continue;
        }
        if stated.as_slice() == rebuilt.as_slice() {
            same_set_same_order += 1;
        } else {
            same_set_other_order += 1;
            if order_examples.len() < 6 {
                order_examples.push(format!(
                    "{parent}: stated {:?} vs refno-sorted {:?}",
                    stated.iter().take(6).map(|r| r.word1).collect::<Vec<_>>(),
                    rebuilt.iter().take(6).map(|r| r.word1).collect::<Vec<_>>()
                ));
            }
        }
    }

    println!(
        "db={} keys={} parsed={}",
        db.display(),
        keys.len(),
        parsed_ok
    );
    println!("elements stating a member list: {}", members_of.len());
    println!("member RefNos the session does not index: {members_not_indexed}");
    println!("members whose own owner is another element: {members_owned_by_someone_else}");
    println!("members listed twice by the same parent: {duplicated_members}");
    println!("stated list == owner-grouped set, same order:  {same_set_same_order}");
    println!("stated list == owner-grouped set, other order: {same_set_other_order}");
    println!("stated list != owner-grouped set:              {different_set}");
    for line in order_examples {
        println!("  {line}");
    }
}

// --------------------------------------------------- deep filtered traversal

/// Walk a subtree by member list, filtering on noun, and price it.
fn deep(db: PathBuf, root: Option<&String>, noun: Option<&String>) {
    let mut view = DbView::open(&db).expect("open at the newest session");
    let keys = live_keys(&mut view);

    let start = match root.and_then(|value| parse_refno(value)) {
        Some(refno) => refno,
        // No root given: take the element with the most members, which is the
        // shape a generation root has.
        None => {
            let mut best = (keys[0].0, 0usize);
            for (refno, page, offset_words) in keys.iter() {
                let Ok(record) = read_record(view.pages_mut(), *page, *offset_words) else {
                    continue;
                };
                let Ok(parsed) = ParsedElement::parse(&record.bytes) else {
                    continue;
                };
                if parsed.members.len() > best.1 {
                    best = (*refno, parsed.members.len());
                }
            }
            println!("root not given, using {} with {} members", best.0, best.1);
            best.0
        }
    };
    let want = noun.map(|name| e3d_attlib::db1_hash(name));

    let address: HashMap<RefNo, (PageId, u32)> = keys
        .iter()
        .map(|(refno, page, offset)| (*refno, (*page, *offset)))
        .collect();

    let before = view.stats();
    let started = Instant::now();
    let mut queue = vec![start];
    let mut visited = HashSet::new();
    let mut records = 0usize;
    let mut matched = 0usize;
    let mut off_database = 0usize;
    let mut not_indexed = 0usize;
    let mut depth_of: HashMap<RefNo, u32> = HashMap::from([(start, 0)]);
    let mut deepest = 0u32;

    while let Some(refno) = queue.pop() {
        if !visited.insert(refno) {
            continue;
        }
        let depth = depth_of.get(&refno).copied().unwrap_or(0);
        deepest = deepest.max(depth);
        let Some((page, offset)) = address.get(&refno).copied() else {
            if refno.dbno() != start.dbno() {
                off_database += 1;
            } else {
                not_indexed += 1;
            }
            continue;
        };
        let Ok(record) = read_record(view.pages_mut(), page, offset) else {
            continue;
        };
        records += 1;
        if want.is_none_or(|hash| hash == record.noun_hash) {
            matched += 1;
        }
        let Ok(parsed) = ParsedElement::parse(&record.bytes) else {
            continue;
        };
        for member in parsed.members {
            depth_of.entry(member).or_insert(depth + 1);
            queue.push(member);
        }
    }

    let elapsed = started.elapsed();
    let after = view.stats();
    println!("db={} root={start}", db.display());
    println!(
        "noun filter: {}",
        noun.map(String::as_str).unwrap_or("(none)")
    );
    println!("records read: {records} matching: {matched} depth: {deepest}");
    println!("members off this database: {off_database}");
    println!("members this session does not index: {not_indexed}");
    println!(
        "pages read: {} cache hits: {} misses: {}",
        after.pages_read - before.pages_read,
        after.cache_hits - before.cache_hits,
        after.cache_misses - before.cache_misses
    );
    println!(
        "elapsed: {:.1} ms ({:.1} us/record)",
        elapsed.as_secs_f64() * 1e3,
        elapsed.as_secs_f64() * 1e6 / records.max(1) as f64
    );
    println!("NOTE the filter runs after the read: a noun-filtered walk still reads every record.");
}

fn parse_refno(text: &str) -> Option<RefNo> {
    let (a, b) = text.split_once('/')?;
    Some(RefNo::new(a.trim().parse().ok()?, b.trim().parse().ok()?))
}

// -------------------------------------------------------------- performance

/// What one element costs, cold and hot, with and without attribute decoding.
fn perf(db: PathBuf, sample: usize) {
    let attlib_present = Path::new(ATTLIB).exists() && Path::new(DESVIR).exists();

    // Cold: a fresh view per element, so the page cache starts empty and the
    // session chain is re-read. This is the shape of "open, read one element".
    let mut view = DbView::open(&db).expect("open at the newest session");
    let keys = live_keys(&mut view);
    let picks: Vec<(RefNo, PageId, u32)> = keys
        .iter()
        .step_by((keys.len() / sample.max(1)).max(1))
        .take(sample)
        .copied()
        .collect();
    drop(view);

    let started = Instant::now();
    let mut cold_pages = 0u64;
    for (_, page, offset) in &picks {
        let mut fresh = DbView::open(&db).expect("reopen");
        let _ = read_record(fresh.pages_mut(), *page, *offset).expect("read");
        cold_pages += fresh.stats().pages_read;
    }
    let cold = started.elapsed();

    // Hot: one view, cache warm, records read through the index each time.
    let mut view = DbView::open(&db).expect("open at the newest session");
    let root = view.index_root();
    let started = Instant::now();
    for (refno, _, _) in &picks {
        let mut cursor = IndexCursor::at_root(view.pages_mut(), root);
        let found = cursor.seek(*refno).expect("seek").found.expect("indexed");
        let (page, offset) = (found.loc.page, found.loc.offset_words);
        let _ = read_record(view.pages_mut(), page, offset).expect("read");
    }
    let hot = started.elapsed();

    println!(
        "db={} sampled={} of {}",
        db.display(),
        picks.len(),
        keys.len()
    );
    println!(
        "cold (reopen the file per element, index bypassed): {:.3} ms/element, {:.1} pages/element",
        cold.as_secs_f64() * 1e3 / picks.len() as f64,
        cold_pages as f64 / picks.len() as f64
    );
    println!(
        "hot  (one view, seek + read):                       {:.3} ms/element",
        hot.as_secs_f64() * 1e3 / picks.len() as f64
    );

    if !attlib_present {
        println!("attlib/desvir absent: skipping the decoded-attribute timing");
        return;
    }
    let attlib = e3d_attlib::AttlibData::parse_file(ATTLIB).expect("parse attlib");
    let mut provider =
        TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("open desvir");
    let mut engine = ReadOnlyEngine::open(&db).expect("open the engine");
    let started = Instant::now();
    let mut decoded = 0usize;
    let mut attributes = 0usize;
    let mut failures: BTreeMap<String, usize> = BTreeMap::new();
    for (refno, _, _) in &picks {
        match engine.extract_element_with_descriptors(*refno, &attlib, &mut provider) {
            Ok(extraction) => {
                decoded += 1;
                attributes += extraction.attributes.len();
            }
            Err(error) => *failures.entry(short(&error.to_string())).or_default() += 1,
        }
    }
    let full = started.elapsed();
    println!(
        "hot  (seek + read + decode every descriptor):       {:.3} ms/element",
        full.as_secs_f64() * 1e3 / picks.len() as f64
    );
    println!(
        "decoded {decoded}/{} elements, {:.1} attributes each",
        picks.len(),
        attributes as f64 / decoded.max(1) as f64
    );
    for (reason, count) in failures {
        println!("  extraction failed {count}x: {reason}");
    }
}

fn short(text: &str) -> String {
    text.chars().take(90).collect()
}

// ---------------------------------------------------------------------- UDA

/// Do the Dictionary UDAs materialise onto Design elements, and does OLDKEY
/// matter in practice?
fn uda(db: PathBuf) {
    for path in [ATTLIB, DICVIR, DESVIR, DICTIONARY_DB] {
        if !Path::new(path).exists() {
            println!("missing fixture {path}");
            return;
        }
    }
    let attlib = e3d_attlib::AttlibData::parse_file(ATTLIB).expect("parse attlib");
    let started = Instant::now();
    let catalog = UdaCatalog::read(DICTIONARY_DB, DICVIR, &attlib).expect("read Dictionary UDAs");
    let read_ms = started.elapsed().as_secs_f64() * 1e3;
    println!(
        "dictionary={DICTIONARY_DB} definitions={} issues={} read={read_ms:.0} ms",
        catalog.definitions().len(),
        catalog.issues().len()
    );
    let with_old = catalog
        .definitions()
        .iter()
        .filter(|d| d.old_key.is_some())
        .count();
    let pseudo = catalog.definitions().iter().filter(|d| d.is_pseudo).count();
    let dynamic = catalog
        .definitions()
        .iter()
        .filter(|d| d.dynamic_name)
        .count();
    println!("definitions carrying OLDKEY: {with_old}, pseudo: {pseudo}, dynamic name: {dynamic}");

    let mut provider =
        TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("open desvir");
    let mut engine = ReadOnlyEngine::open(&db).expect("open the design database");
    let keys = engine.indexed_refnos().expect("walk the live tree");
    println!("design={} keys={}", db.display(), keys.len());

    let mut elements_with_uda = 0usize;
    let mut materialised = 0usize;
    let mut via_old_key = 0usize;
    let mut extract_failed = 0usize;
    let mut per_noun: BTreeMap<u32, usize> = BTreeMap::new();
    let old_keys: HashSet<u32> = catalog
        .definitions()
        .iter()
        .filter_map(|d| d.old_key)
        .collect();

    for (refno, _, _) in keys.iter().step_by((keys.len() / 3000).max(1)) {
        let Ok(view) = engine.find_element(*refno) else {
            continue;
        };
        let Some(view) = view else { continue };
        let Ok(parsed) = ParsedElement::parse(&view.raw_bytes) else {
            continue;
        };
        let carries_old = parsed
            .explicit_attributes
            .iter()
            .any(|attribute| old_keys.contains(&attribute.hash));
        let Ok(mut extraction) =
            engine.extract_element_with_descriptors(*refno, &attlib, &mut provider)
        else {
            extract_failed += 1;
            continue;
        };
        let added = catalog.materialize_for_element(&mut extraction, &parsed.explicit_attributes);
        if added > 0 {
            elements_with_uda += 1;
            materialised += added;
            *per_noun.entry(extraction.noun_hash).or_default() += 1;
            if carries_old {
                via_old_key += 1;
            }
        }
    }
    println!("sampled elements with at least one UDA: {elements_with_uda}");
    println!("UDA values materialised: {materialised}");
    println!("sampled elements storing a value under an OLDKEY hash: {via_old_key}");
    println!("descriptor extraction failed on: {extract_failed}");
    println!("nouns carrying UDAs: {:?}", top_nouns(&per_noun, 12));
}

// -------------------------------------------------------- catalogue expressions

/// How much of a catalogue reads as a number, and how much as an expression?
fn expr(db: PathBuf, sample: usize) {
    if !Path::new(ATTLIB).exists() || !Path::new(CATVIR).exists() {
        println!("missing attlib or catvir");
        return;
    }
    let attlib = e3d_attlib::AttlibData::parse_file(ATTLIB).expect("parse attlib");
    let mut provider = TemplateProvider::open(CATVIR, None).expect("open catvir");
    let mut engine = ReadOnlyEngine::open(&db).expect("open the catalogue database");
    let keys = engine.indexed_refnos().expect("walk the live tree");
    let step = (keys.len() / sample.max(1)).max(1);

    let mut examined = 0usize;
    let mut raw_words = 0usize;
    let mut by_renderer: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut raw_words_unrendered = 0usize;
    let mut explicit_pml = 0usize;
    let mut numeric = 0usize;
    let mut unset = 0usize;
    let mut extract_failed: BTreeMap<String, usize> = BTreeMap::new();
    let mut rendered_examples: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();
    let mut unrendered_examples: BTreeSet<String> = BTreeSet::new();

    for (refno, _, _) in keys.iter().step_by(step).take(sample) {
        let extraction =
            match engine.extract_element_with_descriptors(*refno, &attlib, &mut provider) {
                Ok(extraction) => extraction,
                Err(error) => {
                    *extract_failed.entry(short(&error.to_string())).or_default() += 1;
                    continue;
                }
            };
        examined += 1;
        for attribute in &extraction.attributes {
            match &attribute.value {
                Some(DescriptorValue::RawWords(words)) => {
                    raw_words += 1;
                    match render_by_shape(words) {
                        Some((renderer, text)) => {
                            *by_renderer.entry(renderer).or_default() += 1;
                            let bucket = rendered_examples.entry(renderer).or_default();
                            if bucket.len() < 10 {
                                bucket.insert(format!("{}: {text}", attribute.name));
                            }
                        }
                        None => {
                            raw_words_unrendered += 1;
                            if unrendered_examples.len() < 20 {
                                unrendered_examples.insert(format!(
                                    "{}: {:?}",
                                    attribute.name,
                                    &words[..words.len().min(6)]
                                ));
                            }
                        }
                    }
                }
                Some(DescriptorValue::Text(text)) if text.contains("ATTRIB") => explicit_pml += 1,
                Some(DescriptorValue::Real(_)) | Some(DescriptorValue::Int(_)) => numeric += 1,
                Some(DescriptorValue::Unset) | None => unset += 1,
                _ => {}
            }
        }
    }

    println!(
        "db={} keys={} examined={examined}",
        db.display(),
        keys.len()
    );
    println!("attribute values that are numbers: {numeric}, unset: {unset}");
    println!("attribute values that are raw word tuples: {raw_words}");
    for (renderer, count) in &by_renderer {
        println!("  rendered by {renderer:<16} {count}");
    }
    println!("  rendered by nothing:      {raw_words_unrendered}");
    println!("explicit text values holding a PML formula: {explicit_pml}");
    for (reason, count) in extract_failed {
        println!("  extraction failed {count}x: {reason}");
    }
    println!("-- rendered forms a consumer would have to evaluate --");
    for (renderer, lines) in &rendered_examples {
        for line in lines {
            println!("  [{renderer}] {line}");
        }
    }
    println!("-- word tuples nothing renders --");
    for line in unrendered_examples {
        println!("  {line}");
    }
    println!("NOTE render() returns E3D display text, not a number: nothing here evaluates.");
}

/// The dispatcher `tty.rs` keeps private, spelled out here so the count is
/// against everything the crate can already render rather than against two of
/// the five renderers.
fn render_by_shape(words: &[u32]) -> Option<(&'static str, String)> {
    if let Some(text) = e3d_io::record::catalogue_expr::render(words) {
        return Some(("catalogue_expr", text));
    }
    if let Some(text) = e3d_io::record::axis_spec::render(words) {
        return Some(("axis_spec", text));
    }
    if let Some(text) = e3d_io::record::direction_spec::render(words) {
        return Some(("direction_spec", text));
    }
    if let Some(text) = e3d_io::record::point_list::render(words) {
        return Some(("point_list", text));
    }
    let signed: Vec<i32> = words.iter().map(|word| *word as i32).collect();
    if let Some(text) = e3d_io::record::catalogue_pml::render(&signed) {
        return Some(("catalogue_pml", text));
    }
    e3d_io::record::text::packed(words).map(|text| ("text", text))
}

// -------------------------------------------------------------------- helpers

fn top(counts: &BTreeMap<u32, usize>, n: usize) -> Vec<(u32, usize)> {
    let mut items: Vec<(u32, usize)> = counts.iter().map(|(k, v)| (*k, *v)).collect();
    items.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    items.truncate(n);
    items
}

fn top_nouns(counts: &BTreeMap<u32, usize>, n: usize) -> Vec<(String, usize)> {
    top(counts, n)
        .into_iter()
        .map(|(hash, count)| {
            (
                e3d_attlib::db1_dehash(hash).unwrap_or_else(|| format!("0x{hash:08X}")),
                count,
            )
        })
        .collect()
}

#[allow(dead_code)]
fn ams_dir() -> &'static str {
    AMS_DIR
}
