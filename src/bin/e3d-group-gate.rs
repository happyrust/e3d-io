//! The group write gate's write half: create the GPWL / GPSET / GPITEM chain and a
//! `SCOSEL` rules set on a copy of the APS sample, so a real E3D TTY session can be asked
//! whether it reads them back.
//!
//! It goes through the facade (`Transaction::insert_gpwl` and friends) rather than
//! `e3d-write`, for one reason the CLI cannot work around: `GPREF` is not in E3D's
//! dictionary (`noun_attr_fields.json` has no `GPITEM` entry at all), so the CLI's
//! `attrs` path refuses it — `Transaction::set_explicit_refno` is the only way to write it,
//! and that is what `insert_gpset_member` calls. `SCOSEL` is the same story from the other
//! direction: it is a type-20 hunk, which the CLI can write, but the words have to come from
//! somewhere that is not a re-encode until the encoder is verified.
//!
//! ```text
//! cargo run --release --bin e3d-group-gate -- \
//!     --db <copy>/aps000/aps250218_0001 --out gate.json [--cross-db 2013286676/1251]
//! ```
//!
//! It prints the refnos it minted as JSON on stdout (and to `--out`), so the caller can
//! render them into the TTY macro. Nothing here decides whether the result is correct: the
//! TTY session is what says that.

use std::path::{Path, PathBuf};

use e3d_attlib::db1_hash;
use e3d_io::engine::ScanTier;
use e3d_io::record::template::TemplateProvider;
use e3d_io::session::SessionDate;
use e3d_io::write::{
    MemberPosition, SessionClock, SessionRequest, Transaction,
};
use e3d_io::{ReadOnlyEngine, RefNo};

const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;

fn clock() -> SessionClock {
    SessionClock::fixed(
        SessionDate { year: 2026, month: 11, day: 8, hour: 10, minute: 0, second: 0, millisecond: 0 },
        SessionDate { year: 2026, month: 11, day: 8, hour: 2, minute: 0, second: 0, millisecond: 0 },
    )
}

fn usage() -> ! {
    eprintln!(
        "e3d-group-gate --db <database file> [--out <json>] [--cross-db <word0/word1>]\n\
         \x20          [--target <word0/word1>] [--scosel <comma or space separated ints>]\n\
         \n\
         Writes a GPWL under the database's WORL, two GPSETs under it, a GPITEM in each\n\
         (GPREF to a design element, and optionally to one in another database), and — with\n\
         --scosel — a SCOSEL rules set on the second GPSET. `--target` names the design\n\
         element the same-db GPITEM points at, for a library that holds none of its own.\n\
         Pass the words of a real sample rather than an encode() output until the encoder\n\
         is verified."
    );
    std::process::exit(2)
}

/// `--scosel` words: integers separated by commas and/or whitespace.
fn parse_words(text: &str) -> Result<Vec<i32>, String> {
    text.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|token| !token.is_empty())
        .map(|token| {
            token
                .parse::<i32>()
                .map_err(|_| format!("{token:?} is not an integer"))
        })
        .collect()
}

fn main() {
    let mut db: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut cross: Option<RefNo> = None;
    let mut target: Option<RefNo> = None;
    let mut scosel: Option<Vec<i32>> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--db" => db = args.next().map(PathBuf::from),
            "--out" => out = args.next().map(PathBuf::from),
            "--scosel" => {
                let Some(text) = args.next() else { usage() };
                match parse_words(&text) {
                    Ok(words) if !words.is_empty() => scosel = Some(words),
                    Ok(_) => {
                        eprintln!("--scosel: no integers");
                        usage()
                    }
                    Err(error) => {
                        eprintln!("--scosel: {error}");
                        usage()
                    }
                }
            }
            "--cross-db" => {
                let Some(text) = args.next() else { usage() };
                let Some((a, b)) = text.split_once('/') else { usage() };
                let (Ok(word0), Ok(word1)) = (a.trim().parse(), b.trim().parse()) else {
                    usage()
                };
                cross = Some(RefNo::new(word0, word1));
            }
            "--target" => {
                let Some(text) = args.next() else { usage() };
                let Some((a, b)) = text.split_once('/') else { usage() };
                let (Ok(word0), Ok(word1)) = (a.trim().parse(), b.trim().parse()) else {
                    usage()
                };
                target = Some(RefNo::new(word0, word1));
            }
            "-h" | "--help" => usage(),
            other => {
                eprintln!("unknown argument {other:?}");
                usage()
            }
        }
    }
    let Some(db) = db else { usage() };
    if let Err(error) = run(&db, out.as_deref(), cross, scosel.as_deref(), target) {
        eprintln!("e3d-group-gate: {error}");
        std::process::exit(1);
    }
}

/// The database's `WORL` and one design element under it — the anchor and the same-db
/// target.
///
/// "Design element" excludes the system nouns a bare library carries (`WORL`, `MNUM`) and
/// the group nouns this gate writes itself (`GP*`). A library that holds nothing but those
/// has no target to point a `GPREF` at — `aps250204_0001` is exactly that case, which is
/// why `--target` exists: the target may live in another database, and E3D rejects a
/// `GPREF` to a system noun with `is a reference to an illegal element`.
fn anchors(path: &Path) -> Result<(RefNo, Option<RefNo>), String> {
    let mut engine =
        ReadOnlyEngine::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let world_hash = db1_hash("WORL");
    let mnum_hash = db1_hash("MNUM");
    let mut world = None;
    let mut here = None;
    for element in engine
        .scan_elements(ScanTier::Header)
        .map_err(|e| format!("scan: {e}"))?
    {
        let Ok(element) = element else { continue };
        if world.is_none() && element.noun_hash == world_hash {
            world = Some(element.refno);
            continue;
        }
        if element.noun_hash == mnum_hash {
            continue;
        }
        let noun = e3d_attlib::db1_dehash(element.noun_hash).unwrap_or_default();
        if noun.is_empty() || noun == "WORL" || noun == "MNUM" || noun.starts_with("GP") {
            continue;
        }
        if here.is_none() && element.refno.word0 == world.map(|w: RefNo| w.word0).unwrap_or(0) {
            here = Some(element.refno);
        }
    }
    match world {
        Some(world) => Ok((world, here)),
        None => Err("the database indexes no WORL".to_string()),
    }
}

fn run(
    db: &Path,
    out: Option<&Path>,
    cross: Option<RefNo>,
    scosel: Option<&[i32]>,
    target: Option<RefNo>,
) -> Result<(), String> {
    let (world, here) = anchors(db)?;
    // `--target` wins: the database being written need not hold a design element at all
    // (`aps250204` holds only WORL / MNUM / GPWL / GPSET), and the element a `GPREF`
    // points at may live in a sibling library.
    let here = match (target, here) {
        (Some(target), _) => target,
        (None, Some(here)) => here,
        (None, None) => {
            return Err(format!(
                "{} holds no design element to point a GPREF at; pass --target <word0/word1> \
                 from the library that does",
                db.display()
            ))
        }
    };
    let desvir = std::env::var_os("E3DIO_DESVIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| e3d_io::bundled_schema_root().join("e3d31").join("binaries").join("desvir.dat"));
    let mut provider = TemplateProvider::open(&desvir, Some(DESIGN_TEMPLATE_TYPE))
        .map_err(|e| format!("desvir {}: {e}", desvir.display()))?;

    let request = SessionRequest {
        user: "e3dio",
        comment: "e3d-io group gate: GPWL / GPSET / GPITEM / SCOSEL",
        clock: Some(clock()),
    };
    let mut tx = Transaction::open(db, &mut provider)
        .map_err(|e| format!("open {}: {e}", db.display()))?
        .with_spatial(e3d_io::spatial::SpatialPolicy::Off);

    let gpwl = tx.insert_gpwl(world, "/E3DIO-GPWL").map_err(|e| format!("gpwl: {e}"))?;
    let sets = tx
        .insert_gpset(gpwl, "/E3DIO-GPWL/ITEMS")
        .map_err(|e| format!("gpset items: {e}"))?;
    let rules = tx
        .insert_gpset(gpwl, "/E3DIO-GPWL/RULES")
        .map_err(|e| format!("gpset rules: {e}"))?;

    let here_item = tx
        .insert_gpset_member(sets, here, MemberPosition::Last)
        .map_err(|e| format!("member here: {e}"))?;
    let cross_item = match cross {
        Some(target) => Some(
            tx.insert_gpset_member(sets, target, MemberPosition::Last)
                .map_err(|e| format!("member cross-db: {e}"))?,
        ),
        None => None,
    };
    // The rules set holds one explicit member too, so the gate can tell "E3D expanded the
    // GPITEMs" from "E3D evaluated the SCOSEL": they are different member lists.
    let rules_item = tx
        .insert_gpset_member(rules, here, MemberPosition::Last)
        .map_err(|e| format!("member in rules set: {e}"))?;
    if let Some(words) = scosel {
        tx.set_gpset_rules(rules, words)
            .map_err(|e| format!("SCOSEL: {e}"))?;
    }

    let report = tx.commit(&request).map_err(|e| format!("commit: {e}"))?;

    // `RefNo`'s `Display` is `word0/word1` -- the raw encoded page word. E3D's navigation
    // syntax `=A/B` takes the *database number* in `A` (`=24384/22403` in the existing
    // macros), so the gate has to print `dbno/word1` or every navigation is "undefined
    // reference". Both halves are kept: the JSON carries the E3D form under `e3d_ref` and
    // the raw pair under `ref`.
    let e3d_ref = |r: RefNo| format!("{}/{}", r.dbno(), r.word1);
    let gpitem_here = here_item.item();
    let gpitem_cross = cross_item.map(|member| member.item());
    let gpitem_rules = rules_item.item();
    let json = format!(
        "{{\n  \"db\": {db:?},\n  \"world\": \"{world}\",\n  \"gpwl\": \"{gpwl_e3d}\",\n  \
         \"gpset_items\": \"{sets_e3d}\",\n  \"gpset_rules\": \"{rules_e3d}\",\n  \
         \"gpitem_here\": \"{gpitem_here_e3d}\",\n  \"gpitem_cross\": {cross_item},\n  \
         \"gpitem_rules\": \"{gpitem_rules_e3d}\",\n  \"target_here\": \"{here_e3d}\",\n  \
         \"target_cross\": {cross},\n  \"scosel_words\": {scosel_words},\n  \
         \"ref\": {{\"world\": \"{world_raw}\", \"gpwl\": \"{gpwl_raw}\", \
         \"gpset_items\": \"{sets_raw}\", \"gpset_rules\": \"{rules_raw}\", \
         \"gpitem_here\": \"{gpitem_here_raw}\", \"gpitem_rules\": \"{gpitem_rules_raw}\", \
         \"target_here\": \"{here_raw}\"}},\n  \"sesno\": {sesno}\n}}\n",
        db = format!("{:?}", db.display().to_string()),
        world = e3d_ref(world),
        gpwl_e3d = e3d_ref(gpwl),
        sets_e3d = e3d_ref(sets),
        rules_e3d = e3d_ref(rules),
        gpitem_here_e3d = e3d_ref(gpitem_here),
        gpitem_rules_e3d = e3d_ref(gpitem_rules),
        here_e3d = e3d_ref(here),
        cross_item = match gpitem_cross {
            Some(item) => format!("\"{}\"", e3d_ref(item)),
            None => "null".to_string(),
        },
        cross = match cross {
            Some(target) => format!("\"{}\"", e3d_ref(target)),
            None => "null".to_string(),
        },
        world_raw = world,
        gpwl_raw = gpwl,
        sets_raw = sets,
        rules_raw = rules,
        gpitem_here_raw = gpitem_here,
        gpitem_rules_raw = gpitem_rules,
        here_raw = here,
        sesno = report.written.sesno,
        scosel_words = match scosel {
            Some(words) => words.len().to_string(),
            None => "null".to_string(),
        },
    );
    print!("{json}");
    if let Some(out) = out {
        std::fs::write(out, &json).map_err(|e| format!("write {}: {e}", out.display()))?;
    }
    Ok(())
}
