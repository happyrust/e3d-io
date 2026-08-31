//! Pick real elements to spot-check against E3D TTY `ATTLIST` output, from the
//! tree the session names.
//!
//! The point of the sample is coverage of the *decode paths*, not of the model:
//! one element per `(noun, layout)` bucket, so that every distinct
//! `(compact|wide) x noun` shape a library actually contains gets one witness.
//! Layout is the axis because `evidence/20260826-record-layout-census` measured
//! `ams7000_0001` at 41.5% compact records, while every element verified
//! against TTY so far has been wide; a sample drawn without regard to layout
//! draws almost entirely wide records and leaves the compact column — where the
//! descriptor's two offset lists diverge — untested.
//!
//! What changed is where the candidates come from. The sample this replaces,
//! `evidence/20260826-tty-sampling/sample.tsv`, took the leaf entries of every
//! index page in the file. The index is copy-on-write, so every session leaves
//! its tree behind and nothing points at the old ones: on the 429-database
//! corpus that rule yields 940 694 entries against the 789 831 the live trees
//! hold, so 16% of its candidates were addresses in a tree no session can
//! reach. 77 of its 211 rows are elements since deleted and 4 more record a
//! copy the current session has superseded — measured by
//! `tests/sampled_elements_resolve.rs`, which still reads the old list.
//!
//! Candidates here come from [`IndexCursor`] descending the root the session
//! page names, which is the descent `find_element` answers from. An element
//! this picks is an element the current session can look up, and
//! `tests/live_tree_sample_resolves.rs` is the gate that says so.
//!
//! Two passes, because dropping a noun and being unable to cover one are
//! different facts:
//!
//! 1. **The three libraries the old list covers.** Same scope, so the two
//!    lists' noun coverage is comparable at all.
//! 2. **Every other library in the project**, consulted only for the
//!    `(noun, layout)` buckets the old list has and the first pass could not
//!    fill. A bucket still empty after this one is a shape that no live tree in
//!    the project holds — which is a measurement, not a gap in the sample.
//!
//! Writes the TSV itself rather than printing it, so reproducing the evidence
//! is one command and no shell decides the encoding:
//!
//! `cargo run --release --example tty_sample_live_tree -- [ams dir] [out tsv]`

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use e3d_attlib::db1_dehash;
use e3d_io::index::{IndexCursor, IndexEntry};
use e3d_io::record::element::ParsedElement;
use e3d_io::record::read_element_record;
use e3d_io::record::template::{NounTemplate, StorageLayout};
use e3d_io::refno::RefNo;
use e3d_io::session::DbView;

const AMS_DIR: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";
const OUT_TSV: &str = "evidence/2026-08-30-tty-sampling/sample.tsv";
const OLD_TSV: &str = "evidence/20260826-tty-sampling/sample.tsv";

/// The databases the 2026-08-26 sample covers, and why this list is fixed
/// rather than "whatever is in the directory": the two lists are only
/// comparable on noun coverage if the first pass draws from the same libraries.
const CORE_DBS: &[&str] = &["ams7000_0001", "ams8000_0001", "ams1112_0001"];

/// Word 10 of a record header is its storage format word — see `HEADER_WORDS`
/// in `src/record/mod.rs`, the eleven words a record carries before its first
/// attribute.
const FORMAT_WORD_INDEX: usize = 10;

const COMPACT: usize = 0;
const WIDE: usize = 1;
const LAYOUTS: [&str; 2] = ["compact", "wide"];

type Bucket = (String, usize);

/// Live elements per `(noun, layout)` across the first pass.
///
/// This is what turns "the new list does not cover noun X" from a gap into a
/// fact: a bucket the walk never filled is a bucket those libraries do not
/// hold, and the count says so rather than the sampler's silence.
type Census = BTreeMap<String, [usize; 2]>;

/// One row of the sample: the tab-separated shape the 2026-08-26 list uses, so
/// a consumer of the old file can swap in the new one unchanged.
struct Row {
    db: String,
    order: usize,
    noun: String,
    layout: usize,
    implicit_words: i32,
    refno: RefNo,
}

/// Which buckets a pass is allowed to fill.
enum Want<'a> {
    /// Everything the library holds: the first pass.
    Everything,
    /// Only these, and only while they are still empty: the top-up pass.
    Only(&'a BTreeSet<Bucket>),
}

#[derive(Default)]
struct Tally {
    sesno: u32,
    indexed: usize,
    usable: usize,
    /// Live records per layout. `evidence/20260826-record-layout-census` put
    /// `ams7000_0001` at 41.5% compact reading every page in the file; this is
    /// the same split over the tree the session names.
    layouts: [usize; 2],
    /// The record at the address the leaf gives states a different RefNo. On a
    /// live tree this should be nobody: `tests/record_l3_contract.rs` puts it
    /// at 0 of 789 831 across the corpus.
    disowned: usize,
    /// The noun hash is outside the DB1 alphabet, so there is no name to type
    /// into TTY. Counted, not dropped quietly.
    unnameable: BTreeSet<u32>,
    unreadable: usize,
    unparsable: usize,
    headerless: usize,
    notes: Vec<String>,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().unwrap_or_else(|| AMS_DIR.to_string()));
    let out = PathBuf::from(args.next().unwrap_or_else(|| OUT_TSV.to_string()));
    let started = Instant::now();

    let old = read_old_list();
    let mut rows: Vec<Row> = Vec::new();
    let mut census = Census::new();
    let mut order = 0usize;
    let mut failed = false;

    for db in CORE_DBS {
        let path = dir.join(db);
        if !path.exists() {
            eprintln!("FAIL {db}: not present at {}", path.display());
            failed = true;
            continue;
        }
        match sample(&path, db, order, &Want::Everything, &mut rows, &mut census) {
            Ok(tally) => report(db, &tally),
            Err(error) => {
                eprintln!("FAIL {db}: {error}");
                failed = true;
            }
        }
        order += 1;
    }

    let filled: BTreeSet<Bucket> = rows
        .iter()
        .map(|row| (row.noun.clone(), row.layout))
        .collect();
    let mut wanted: BTreeSet<Bucket> = old.buckets.difference(&filled).cloned().collect();
    eprintln!(
        "--- pass 2: {} buckets the old list has and the first pass could not fill ---",
        wanted.len()
    );

    let mut consulted = 0usize;
    let mut unopenable: Vec<String> = Vec::new();
    for path in other_databases(&dir) {
        if wanted.is_empty() {
            break;
        }
        let db = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let before = rows.len();
        let mut ignored = Census::new();
        match sample(
            &path,
            &db,
            order,
            &Want::Only(&wanted),
            &mut rows,
            &mut ignored,
        ) {
            Ok(_) => consulted += 1,
            // A library that will not open cannot witness anything, and there
            // are 443 of them: name it and go on, rather than stop the run or
            // pretend the bucket was checked.
            Err(error) => {
                unopenable.push(format!("{db}: {error}"));
                continue;
            }
        }
        if rows.len() > before {
            for row in &rows[before..] {
                wanted.remove(&(row.noun.clone(), row.layout));
                eprintln!(
                    "  {} {} witnessed by {db} {}",
                    row.noun, LAYOUTS[row.layout], row.refno
                );
            }
            order += 1;
        }
    }
    eprintln!(
        "consulted {consulted} further libraries; {} could not be opened; {} buckets still unwitnessed",
        unopenable.len(),
        wanted.len()
    );
    for line in unopenable.iter().take(10) {
        eprintln!("  {line}");
    }

    if rows.is_empty() {
        eprintln!("no rows picked; nothing written to {}", out.display());
        std::process::exit(1);
    }

    rows.sort_by(|a, b| {
        a.order
            .cmp(&b.order)
            .then_with(|| a.noun.cmp(&b.noun))
            .then_with(|| a.layout.cmp(&b.layout))
            .then_with(|| (a.refno.word0, a.refno.word1).cmp(&(b.refno.word0, b.refno.word1)))
    });
    write_tsv(&out, &rows);
    eprintln!("wrote {} rows to {}", rows.len(), out.display());

    compare(&old, &rows, &census, &wanted);
    audit_old_list(&dir, &old);
    eprintln!("done in {:.1}s", started.elapsed().as_secs_f64());

    if failed {
        std::process::exit(1);
    }
}

/// Walk one database's live tree and fill the buckets it is allowed to.
fn sample(
    path: &Path,
    db: &str,
    order: usize,
    want: &Want<'_>,
    rows: &mut Vec<Row>,
    census: &mut Census,
) -> Result<Tally, String> {
    let mut view = DbView::open(path).map_err(|error| format!("open: {error}"))?;
    let sesno = view.session().sesno;
    let entries: Vec<IndexEntry> = IndexCursor::new(&mut view)
        .walk()
        .map(|found| found.map(|found| found.entry()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("walk the session {sesno} tree: {error}"))?;

    let mut tally = Tally {
        sesno,
        indexed: entries.len(),
        ..Tally::default()
    };
    // (noun, layout) -> rows this database has already contributed.
    let mut picked: BTreeSet<Bucket> = BTreeSet::new();
    let root_ext = view.index_root().ext;
    let pages = view.pages_mut();

    for entry in &entries {
        let record = match read_element_record(pages, root_ext, entry) {
            Ok(record) => record,
            Err(error) => {
                tally.unreadable += 1;
                note(&mut tally, format!("{} unreadable: {error}", entry.refno));
                continue;
            }
        };
        let parsed = match ParsedElement::parse(&record.raw_bytes) {
            Ok(parsed) => parsed,
            Err(error) => {
                tally.unparsable += 1;
                note(&mut tally, format!("{} unparsable: {error}", entry.refno));
                continue;
            }
        };
        if parsed.refno != entry.refno {
            tally.disowned += 1;
            note(
                &mut tally,
                format!(
                    "{} is addressed by the leaf but states {}",
                    entry.refno, parsed.refno
                ),
            );
            continue;
        }
        let Some(word) = format_word(&parsed) else {
            tally.headerless += 1;
            note(
                &mut tally,
                format!(
                    "{}: {} implicit words, too few to carry the format word",
                    entry.refno, parsed.impl_len_words
                ),
            );
            continue;
        };
        let Some(noun) = db1_dehash(parsed.noun_hash) else {
            tally.unnameable.insert(parsed.noun_hash);
            continue;
        };

        let layout = slot(NounTemplate::layout_from_format_word(word));
        tally.usable += 1;
        tally.layouts[layout] += 1;
        census.entry(noun.clone()).or_insert([0, 0])[layout] += 1;

        let bucket = (noun, layout);
        let eligible = match want {
            Want::Everything => true,
            Want::Only(wanted) => wanted.contains(&bucket),
        };
        if eligible && picked.insert(bucket.clone()) {
            rows.push(Row {
                db: db.to_string(),
                order,
                noun: bucket.0,
                layout,
                implicit_words: parsed.impl_len_words,
                refno: parsed.refno,
            });
        }
    }

    Ok(tally)
}

/// The project's databases other than the three the first pass reads, in name
/// order so the top-up picks the same witness on every run.
fn other_databases(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        eprintln!("cannot list {}", dir.display());
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let named = name.starts_with("ams")
                && name.len() > 5
                && name.as_bytes()[name.len() - 5] == b'_'
                && name[name.len() - 4..].bytes().all(|b| b.is_ascii_digit())
                && name[3..name.len() - 5].bytes().all(|b| b.is_ascii_digit());
            let core = CORE_DBS.contains(&name.as_str());
            (entry.metadata().ok()?.is_file() && named && !core).then(|| entry.path())
        })
        .collect();
    files.sort();
    files
}

/// The storage format word, or `None` when the record is too short to carry one
/// — which `read_record` already refuses, so this is belt and braces.
fn format_word(parsed: &ParsedElement) -> Option<u32> {
    let at = FORMAT_WORD_INDEX * 4;
    Some(u32::from_be_bytes(
        parsed.implicit_data.get(at..at + 4)?.try_into().ok()?,
    ))
}

fn slot(layout: StorageLayout) -> usize {
    match layout {
        StorageLayout::Compact => COMPACT,
        StorageLayout::Wide => WIDE,
    }
}

fn note(tally: &mut Tally, line: String) {
    if tally.notes.len() < 20 {
        tally.notes.push(line);
    }
}

fn report(db: &str, tally: &Tally) {
    eprintln!(
        "{db}: session {}, {} indexed, {} usable ({} compact / {} wide), {} unreadable, \
         {} unparsable, {} disowned, {} headerless, {} unnameable nouns",
        tally.sesno,
        tally.indexed,
        tally.usable,
        tally.layouts[COMPACT],
        tally.layouts[WIDE],
        tally.unreadable,
        tally.unparsable,
        tally.disowned,
        tally.headerless,
        tally.unnameable.len(),
    );
    for hash in &tally.unnameable {
        eprintln!("  noun hash outside the DB1 alphabet: {hash:#010X}");
    }
    for line in &tally.notes {
        eprintln!("  {line}");
    }
}

/// Match the 2026-08-26 file in shape: tab separated, no header, CRLF, no BOM.
fn write_tsv(out: &Path, rows: &[Row]) {
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).expect("create the evidence directory");
    }
    let mut text = String::new();
    for row in rows {
        text.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\r\n",
            row.db, row.noun, LAYOUTS[row.layout], row.implicit_words, row.refno
        ));
    }
    fs::write(out, text).expect("write the sample");
}

struct OldRow {
    db: String,
    noun: String,
    layout: usize,
    implicit_words: i32,
    refno: RefNo,
}

struct OldList {
    rows: Vec<OldRow>,
    buckets: BTreeSet<Bucket>,
}

fn read_old_list() -> OldList {
    let mut list = OldList {
        rows: Vec::new(),
        buckets: BTreeSet::new(),
    };
    let Ok(text) = fs::read_to_string(OLD_TSV) else {
        eprintln!("no {OLD_TSV} to compare against; pass 2 has nothing to aim at");
        return list;
    };
    for line in text.lines().filter(|line| !line.starts_with('#')) {
        let cols: Vec<&str> = line.split('\t').collect();
        let [db, noun, layout, words, refno] = cols[..] else {
            continue;
        };
        let (Some(layout), Ok(words), Some(refno)) = (
            LAYOUTS.iter().position(|name| *name == layout),
            words.parse::<i32>(),
            parse_refno(refno),
        ) else {
            continue;
        };
        list.buckets.insert((noun.to_string(), layout));
        list.rows.push(OldRow {
            db: db.to_string(),
            noun: noun.to_string(),
            layout,
            implicit_words: words,
            refno,
        });
    }
    list
}

fn parse_refno(text: &str) -> Option<RefNo> {
    let (word0, word1) = text.split_once('/')?;
    Some(RefNo::new(
        word0.trim().parse().ok()?,
        word1.trim().parse::<i32>().ok()? as u32,
    ))
}

/// Resolve every row of the old list against the session that answers today.
///
/// The old list records four things per element, and the reason for auditing
/// all four is that "the row still resolves" is not the same as "the row is
/// still true": an element can be live and yet be a different shape from the
/// copy the old sampler read out of a dead tree.
fn audit_old_list(dir: &Path, old: &OldList) {
    if old.rows.is_empty() {
        return;
    }
    let mut by_db: BTreeMap<&str, Vec<&OldRow>> = BTreeMap::new();
    for row in &old.rows {
        by_db.entry(&row.db).or_default().push(row);
    }

    let (mut intact, mut deleted, mut wrong_layout, mut wrong_words, mut wrong_noun) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut examples: Vec<String> = Vec::new();

    for (db, rows) in &by_db {
        let path = dir.join(db);
        let Ok(mut engine) = e3d_io::ReadOnlyEngine::open(&path) else {
            eprintln!("cannot audit {db}: it does not open");
            continue;
        };
        for row in rows {
            let found = match engine.find_element(row.refno) {
                Ok(found) => found,
                Err(error) => {
                    eprintln!("  {db} {} {}: lookup failed: {error}", row.noun, row.refno);
                    continue;
                }
            };
            let Some(view) = found else {
                deleted += 1;
                continue;
            };
            let Ok(parsed) = ParsedElement::parse(&view.raw_bytes) else {
                eprintln!(
                    "  {db} {} {}: live record does not parse",
                    row.noun, row.refno
                );
                continue;
            };
            let noun = db1_dehash(parsed.noun_hash).unwrap_or_default();
            let layout =
                format_word(&parsed).map(|word| slot(NounTemplate::layout_from_format_word(word)));
            let mut agrees = true;
            if noun != row.noun {
                wrong_noun += 1;
                agrees = false;
            }
            if layout != Some(row.layout) {
                wrong_layout += 1;
                agrees = false;
                if examples.len() < 20 {
                    examples.push(format!(
                        "{db} {} {}: recorded {}, the session says {}",
                        row.noun,
                        row.refno,
                        LAYOUTS[row.layout],
                        layout.map_or("no format word", |slot| LAYOUTS[slot])
                    ));
                }
            }
            if parsed.impl_len_words != row.implicit_words {
                wrong_words += 1;
                agrees = false;
            }
            if agrees {
                intact += 1;
            }
        }
    }

    eprintln!("--- the old list against the session that answers today ---");
    eprintln!("rows that still describe their element: {intact}");
    eprintln!("rows whose element the live tree does not hold: {deleted}");
    eprintln!("rows whose layout the live copy contradicts: {wrong_layout}");
    eprintln!("rows whose implicit word count the live copy contradicts: {wrong_words}");
    eprintln!("rows whose noun the live copy contradicts: {wrong_noun}");
    for line in &examples {
        eprintln!("  {line}");
    }
}

/// What the new list covers that the old one did not, and the other way round.
///
/// A bucket the old list has and this one does not is only acceptable with a
/// reason, so this prints one for each: how many live elements of that noun the
/// three core libraries hold, and whether any of the other 440 could witness it.
fn compare(old: &OldList, rows: &[Row], census: &Census, unwitnessed: &BTreeSet<Bucket>) {
    let new_buckets: BTreeSet<Bucket> = rows
        .iter()
        .map(|row| (row.noun.clone(), row.layout))
        .collect();
    let old_nouns: BTreeSet<&String> = old.buckets.iter().map(|(noun, _)| noun).collect();
    let new_nouns: BTreeSet<&String> = new_buckets.iter().map(|(noun, _)| noun).collect();
    let topped_up = rows
        .iter()
        .filter(|row| !CORE_DBS.contains(&row.db.as_str()))
        .count();

    eprintln!("--- against {OLD_TSV} ---");
    eprintln!(
        "rows:    {} old, {} new ({topped_up} from pass 2)",
        old.rows.len(),
        rows.len()
    );
    eprintln!("nouns:   {} old, {} new", old_nouns.len(), new_nouns.len());
    eprintln!(
        "buckets: {} old, {} new, {} of the old ones unwitnessed",
        old.buckets.len(),
        new_buckets.len(),
        unwitnessed.len()
    );

    for (noun, layout) in unwitnessed {
        let live = census.get(noun).copied().unwrap_or([0, 0]);
        eprintln!(
            "  unwitnessed: {noun} {} — the core three hold {} live compact and {} live wide, \
             and no other library in the project holds one either",
            LAYOUTS[*layout], live[COMPACT], live[WIDE]
        );
    }
    for noun in new_nouns.difference(&old_nouns) {
        eprintln!("  noun only in the new list: {noun}");
    }
}
