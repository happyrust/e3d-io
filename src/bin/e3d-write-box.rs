//! Append one session to a dabacon DESIGN database: a new `BOX` under the
//! first `ZONE`, added to that ZONE's member list. This was the fixture for the
//! E3D manual gate (`docs/plans/2026-09-20-dabacon-write-plan.md` §2 阶段 2 用例
//! A / §7 Q8) — write with this crate, then open the database in AVEVA E3D 3.1
//! Design and `Q SESSIONS` / `Q ATT` the box to see whether E3D reads back what
//! was written. Since plan T1d the gate (`tools/e3d/write_gate.ps1`) writes the
//! same sessions through `e3d-write apply` and the instruction files in
//! `tools/e3d/write_gate_ops/`; this bin is kept as the hand-written form of
//! them and still runs.
//!
//! ```text
//! e3d-write-box <database> [desvir.dat] [--group] [--hunk] [--text]
//! ```
//!
//! Every session is written through [`e3d_io::write::Transaction`] (S6): the
//! facade mints each new element's refno in the owner's extent (one past the
//! highest seen there), adds it to the owner's member list, files any `NAME` in
//! the `NAME` table, and keeps `TYPEX`, the reverse-reference tables and the
//! claim tree. So this bin no longer hands out refnos, threads `NamedEdit`s or
//! edits owner lists by hand — the three chores it used to do inline. The
//! refnos it prints are the ones the facade chose (`max + 1`, not the old
//! `max + 5000`); the gate reads them from this output, so the values move but
//! the questions do not.
//!
//! With `--group` it appends a **second** session after the box: a `GPWL`
//! under the `WORL` and a `GROU` under that, whose members are *references* —
//! the ZONE and the new BOX, both owned elsewhere — the way E3D's own groups
//! hold elements (`GROU`'s template says its member list is by reference, so
//! the facade leaves it to us). That session asks E3D whether it accepts a
//! member list whose members do not name this element as owner.
//!
//! With `--hunk` it appends two more sessions asking the third and fourth
//! questions — does E3D read a value stored **out of the record** (a *hunk*,
//! storage types 20–23), and a **string list** (storage type 19)? The first
//! puts two `EQUI`s under the ZONE whose `DESC` / `DESP` / `UUIDIA` are hunks
//! (120 characters, 100 reals, 4 ints), so with two elements in one session at
//! least one chain runs across a page boundary; the second puts a `FIXING`
//! under the ZONE with `ROKEYS` inline (type 19) and `ROVALS` as a type-23 hunk.
//!
//! With `--move` it appends two sessions asking whether E3D reads a
//! re-parented element (`Transaction::move_to`, E3D's `INCLUDE`), a reordered
//! member list (`reorder`) and an edited reference list
//! (`add_reference_member` / `remove_reference_member`; needs `--group` for
//! the GROU): three named BOXes go under the second ZONE, then one is moved to
//! the third ZONE, one is put before another, and the group gains one of them
//! and loses another (plan T1a).
//!
//! With `--big` it appends one session: a new ZONE under the first ZONE's SITE
//! with 8 200 BOXes — more than the format word's fourteen-bit member count
//! holds, so the count is written saturated (`0x3FFF`) and E3D has to measure
//! the member chain (plan §1.3 #11; the 20 128-member ZONE E3D listed as 3 744).
//!
//! With `--strings` it appends one session written **by attribute name from
//! text** (`Transaction::set`, plan T1b) under E3D's dictionary
//! (`schema/e3d31/catalog/noun_attr_fields.json`): an EQUI whose `DESC` /
//! `POS` / `FUNC` / `PURP` and a FIXING whose `ROKEYS` / `BUIL` / `NUMB` /
//! `CREF` (a `/NAME` reference) are all given as strings, after the dictionary
//! has refused a 121-character `DESC`, a four-element `POS` and `RATM`. The
//! question is whether E3D prints back exactly what was typed.
//!
//! With `--text` it appends a last session: the BOX gets a Chinese `NAME`
//! (record and `NAME` table key, through `Transaction::rename`), the ZONE a
//! Chinese `DESC`, and the session a Chinese comment — each stored the way the
//! header's `FORMAT` says (`e3d_io::record::text::encode_as`, plan S5). It goes
//! last so `Q SESSION` shows its comment.
//!
//! `--merge-from <sesno>` is a different run (plan T1c): none of the sessions
//! above, just one transaction opened at session `<sesno>` — a base the
//! database has since moved past, E3D itself having saved in between — that
//! inserts a BOX under the first ZONE and sets the second ZONE's `FUNC`, and
//! commits: a **merge** onto the newest session (`BASESN` = `<sesno>`), the
//! inserted refno re-minted if the sessions in between took it. The question
//! is whether E3D then reads both what it wrote and what was merged in.
//!
//! It writes the database **in place**, so point it at a copy (or a backed-up
//! project DB); the write is an append plus the page-0 commit, so restoring the
//! original bytes undoes it. It prints the session numbers, the RefNos and
//! owners it wrote, and the geometry, so the values can be checked in E3D.

use std::process::ExitCode;

use e3d_io::engine::ScanTier;
use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::record::template::{MemberListKind, NounTemplate, TemplateProvider};
use e3d_io::record::text::TextFormat;
use e3d_io::session::SessionSelector;
use e3d_io::write::{Dictionary, DictionaryError, MemberPosition, RecordDraft, SessionRequest, Transaction, TransactionError};
use e3d_io::{ReadOnlyEngine, RefNo};

const DEFAULT_DESVIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schema/e3d31/binaries/desvir.dat");
/// E3D's dictionary export the schema pack carries, for `--strings`.
const DICTIONARY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schema/e3d31/catalog/noun_attr_fields.json");
const USAGE: &str = "usage: e3d-write-box <database> [desvir.dat] [--group] [--hunk] [--move] [--big] [--strings] [--text] | --merge-from <sesno>";

/// The `NAME` the `--text` session gives the BOX and the `DESC` it gives the
/// ZONE: text outside ASCII, stored as the header's `FORMAT` says.
pub const TEXT_BOX_NAME: &str = "/E3DIO-盒子";
pub const TEXT_ZONE_DESC: &str = "反应堆厂房 -3.320m~±0.00m";
pub const TEXT_COMMENT: &str = "e3d-io 写路径门：中文 NAME / DESC / 会话注释";

/// The names of the three BOXes the `--move` sessions play with, so E3D's
/// `Q MEM` lists them by name wherever they land.
pub const MOVE_BOX_NAMES: [&str; 3] = ["/E3DIO-MOVE-A", "/E3DIO-MOVE-B", "/E3DIO-MOVE-C"];

/// The `--big` ZONE's name and member count: past 8 191 the format word's
/// member count no longer fits its fourteen bits.
pub const BIG_ZONE_NAME: &str = "/E3DIO-BIG-ZONE";
pub const BIG_ZONE_MEMBERS: u32 = 8_200;

/// What the `--strings` session types, attribute by attribute, exactly as
/// `Transaction::set` receives it — the gate compares E3D's print-out with
/// these.
pub const STR_EQUI_NAME: &str = "/E3DIO-STR-EQUI";
pub const STR_EQUI_DESC: &str = "e3d-io T1b: DESC typed as text and set by name through Transaction::set";
pub const STR_EQUI_POS: &str = "1234 2345 3456";
pub const STR_EQUI_FUNC: &str = "STRING-PATH";
pub const STR_EQUI_PURP: &str = "PIPE";
pub const STR_FIXING_NAME: &str = "/E3DIO-STR-FIXING";
pub const STR_FIXING_ROKEYS: &str = "S1|S2|S3";
pub const STR_FIXING_NUMB: &str = "42";

/// What the `--merge-from` transaction writes: a BOX under the first ZONE and
/// the second ZONE's `FUNC`.
pub const MERGE_BOX_NAME: &str = "/E3DIO-MERGE-BOX";
pub const MERGE_BOX_XLEN: &str = "555";
pub const MERGE_ZONE_FUNC: &str = "MERGED-BY-E3DIO";
pub const MERGE_COMMENT: &str = "e3d-io write-path gate: merged onto a session E3D saved after this transaction's base";

/// The database's header `FORMAT` reading, for text values.
fn text_format(db: &str) -> Result<TextFormat, String> {
    let engine = ReadOnlyEngine::open(db).map_err(|e| format!("open {db}: {e}"))?;
    Ok(TextFormat::from_header(engine.descriptor().header_format))
}

/// The first `n` ZONEs in the database, in index order; the first owns the box.
fn zones(db: &str, n: usize) -> Result<Vec<RefNo>, String> {
    let mut engine = ReadOnlyEngine::open(db).map_err(|e| format!("open {db}: {e}"))?;
    let zone_hash = e3d_attlib::db1_hash("ZONE");
    let found: Vec<RefNo> = engine
        .scan_elements(ScanTier::Header)
        .map_err(|e| e.to_string())?
        .map(|e| e.map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|e| e.noun_hash == zone_hash)
        .map(|e| e.refno)
        .take(n)
        .collect();
    if found.len() < n {
        return Err(format!("the database indexes {} ZONE(s); {n} wanted", found.len()));
    }
    Ok(found)
}

/// The first ZONE in the database, to own the box.
fn first_zone(db: &str) -> Result<RefNo, String> {
    Ok(zones(db, 1)?[0])
}

/// An element's owner and member list as stored.
fn owner_and_members(db: &str, refno: RefNo) -> Result<(RefNo, Vec<RefNo>), String> {
    let mut engine = ReadOnlyEngine::open(db).map_err(|e| format!("open {db}: {e}"))?;
    let view = engine
        .find_element(refno)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("{refno} is not indexed"))?;
    let parsed = e3d_io::record::element::ParsedElement::parse_as(&view.raw_bytes, engine.text_format())
        .map_err(|e| e.to_string())?;
    Ok((view.header().owner, parsed.members))
}

/// Refuse to reuse a name the database already has.
fn name_is_free(db: &str, name: &str) -> Result<(), String> {
    let mut engine = ReadOnlyEngine::open(db).map_err(|e| e.to_string())?;
    if engine.find_by_name(name).map_err(|e| e.to_string())?.is_some() {
        return Err(format!("{name} is already in use"));
    }
    Ok(())
}

/// The database's WORL element.
fn world_refno(db: &str) -> Result<RefNo, String> {
    let mut engine = ReadOnlyEngine::open(db).map_err(|e| format!("open {db}: {e}"))?;
    Ok(engine
        .world()
        .map_err(|e| e.to_string())?
        .ok_or("the database indexes no WORL")?
        .header()
        .refno)
}

/// Set POS/XLEN/YLEN/ZLEN on a fresh BOX draft.
fn build_box(draft: &mut RecordDraft, template: &NounTemplate, format: TextFormat) -> Result<(), e3d_io::write::TransactionError> {
    let hash = |name: &str| e3d_attlib::db1_hash(name);
    draft.set_attribute(template, hash("POS"), &DescriptorValue::RealArray(vec![1000.0, 2000.0, 3000.0]), format)?;
    draft.set_attribute(template, hash("XLEN"), &DescriptorValue::Real(300.0), format)?;
    draft.set_attribute(template, hash("YLEN"), &DescriptorValue::Real(400.0), format)?;
    draft.set_attribute(template, hash("ZLEN"), &DescriptorValue::Real(500.0), format)?;
    Ok(())
}

fn run() -> Result<(), String> {
    let mut db = None;
    let mut desvir = None;
    let mut group = false;
    let mut hunk = false;
    let mut moves = false;
    let mut big = false;
    let mut strings = false;
    let mut text = false;
    let mut merge_from: Option<u32> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--group" => group = true,
            "--hunk" => hunk = true,
            "--move" => moves = true,
            "--big" => big = true,
            "--strings" => strings = true,
            "--text" => text = true,
            "--merge-from" => {
                let sesno = args.next().ok_or_else(|| format!("--merge-from wants a session number\n{USAGE}"))?;
                merge_from = Some(sesno.parse().map_err(|_| format!("--merge-from {sesno}: not a session number\n{USAGE}"))?);
            }
            other if other.starts_with("--") => return Err(format!("unknown flag {other}\n{USAGE}")),
            _ if db.is_none() => db = Some(arg),
            _ if desvir.is_none() => desvir = Some(arg),
            _ => return Err(USAGE.to_string()),
        }
    }
    let db = db.ok_or(USAGE)?;
    let desvir = desvir.unwrap_or_else(|| DEFAULT_DESVIR.to_string());
    let format = text_format(&db)?;
    let zone_refno = first_zone(&db)?;
    let box_hash = e3d_attlib::db1_hash("BOX");
    let mut provider = TemplateProvider::open(&desvir, None).map_err(|e| format!("open {desvir}: {e}"))?;

    if let Some(base) = merge_from {
        return write_merge(&db, &mut provider, base);
    }

    // --- session 1: the box, through the facade ---
    let (box_refno, sesno, previous, appended) = {
        let mut tx = Transaction::open(&db, &mut provider).map_err(|e| e.to_string())?;
        let box_refno = tx
            .insert(zone_refno, box_hash, None, |draft, template| build_box(draft, template, format))
            .map_err(|e| e.to_string())?;
        let report = tx.commit(&session("e3d-io write-path gate: one BOX")).map_err(|e| e.to_string())?;
        (box_refno, report.written.sesno, report.previous.sesno, report.appended_pages)
    };

    println!("wrote session {sesno} to {db}");
    println!("  previous session : {previous}");
    println!("  new BOX          : {box_refno} (word0={}, word1={})", box_refno.word0, box_refno.word1);
    println!("  owner ZONE       : {zone_refno}");
    println!("  POS              : 1000 2000 3000");
    println!("  XLEN/YLEN/ZLEN   : 300 / 400 / 500");
    println!("  appended pages   : {appended:?}");
    println!();
    println!("In E3D 3.1 Design, on this database's project:");
    println!("  Q SESSIONS                 -> the newest should be session {sesno} by user e3dio");
    println!("  Q ATT /{box_refno}                  -> POS/XLEN/YLEN/ZLEN as above, OWNER the ZONE");

    let mut grou_refno = None;
    if group {
        grou_refno = Some(write_group(&db, &mut provider, zone_refno, box_refno)?);
    }
    if hunk {
        write_hunks(&db, &mut provider, format, zone_refno, box_refno)?;
    }
    if moves {
        write_moves(&db, &mut provider, format, grou_refno)?;
    }
    if big {
        write_big(&db, &mut provider, format, zone_refno)?;
    }
    if strings {
        write_strings(&db, &mut provider, zone_refno)?;
    }
    // Last, so `Q SESSION` (the newest session) shows the Chinese comment.
    if text {
        write_text(&db, &mut provider, format, zone_refno, box_refno)?;
    }
    Ok(())
}

/// A session request with the given comment and no fixed clock.
fn session(comment: &str) -> SessionRequest<'_> {
    SessionRequest { user: "e3dio", comment, clock: None }
}

/// The noun template for `noun`, cloned so the provider is free for the tx.
fn template_of(provider: &mut TemplateProvider, noun: &str) -> Result<NounTemplate, String> {
    provider
        .template_for(e3d_attlib::db1_hash(noun))
        .map_err(|e| e.to_string())?
        .cloned()
        .ok_or_else(|| format!("the schema has no {noun} template"))
}

/// The attribute hash of `name` on `template`, checked to have no implicit slot
/// (so E3D reads it from the explicit stream, where a hunk entry can be met) and
/// a declared count of at least `count`.
fn explicit_only(template: &NounTemplate, name: &str, count: usize) -> Result<u32, String> {
    let attr = template
        .attrs
        .iter()
        .find(|a| {
            e3d_attlib::lookup_system_name(a.hash) == Some(name)
                || e3d_attlib::db1_dehash(a.hash).as_deref() == Some(name)
        })
        .ok_or_else(|| format!("the {} template does not declare {name}", template_name(template)))?;
    if attr.wide.location.word_offset() != 0 {
        return Err(format!(
            "{}.{name} has an implicit slot (wide word {}); E3D would read that, not the explicit entry",
            template_name(template),
            attr.wide.location.word_offset()
        ));
    }
    if (attr.logical_count as usize) < count {
        return Err(format!(
            "{}.{name} declares {} elements, the gate wants to write {count}",
            template_name(template),
            attr.logical_count
        ));
    }
    Ok(attr.hash)
}

fn template_name(template: &NounTemplate) -> String {
    e3d_attlib::db1_dehash(template.noun_hash).unwrap_or_else(|| format!("{:#x}", template.noun_hash))
}

/// The values one gate EQUI carries: a text of 120 characters, a real array of
/// the descriptor's full 100, and four ints.
struct HunkValues {
    desc: String,
    desp: Vec<f64>,
    uuidia: [i32; 4],
}

/// E3D's `DESC` length limit in characters (its dictionary's, not the
/// template's byte capacity).
const DESC_CHARS: usize = 120;

impl HunkValues {
    fn for_element(n: u32) -> Self {
        let head = format!("e3d-io hunk gate EQUI {n}: DESC stored out of the record (type 22) ");
        let mut desc = head.clone();
        while desc.len() < DESC_CHARS {
            let marker = format!("@{:03} ", desc.len());
            desc.push_str(&marker);
        }
        desc.truncate(DESC_CHARS);
        Self {
            desc,
            desp: (1..=100).map(|i| f64::from(n) * 1000.0 + f64::from(i) + 0.25).collect(),
            uuidia: [n as i32 * 10 + 1, n as i32 * 10 + 2, n as i32 * 10 + 3, n as i32 * 10 + 4],
        }
    }
}

/// How many pages each of a written element's hunk chains runs over, read back
/// through the directory entries.
fn hunk_spans(db: &str, refno: RefNo) -> Result<Vec<(String, u16, u32, usize)>, String> {
    use e3d_io::page::cache::PageCache;
    use e3d_io::page::io::PageReader;
    use e3d_io::page::PageId;
    use e3d_io::record::element::ParsedElement;
    use e3d_io::record::hunk::{hunk_block_at, HunkRef};

    let mut engine = ReadOnlyEngine::open(db).map_err(|e| e.to_string())?;
    let view = engine
        .find_element(refno)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("{refno} is not indexed after the write"))?;
    let parsed = ParsedElement::parse_as(&view.raw_bytes, engine.text_format()).map_err(|e| e.to_string())?;
    let mut cache = PageCache::new(PageReader::open(db).map_err(|e| e.to_string())?, 16);
    let mut out = Vec::new();
    for attribute in &parsed.explicit_attributes {
        let Some(entry) = HunkRef::from_explicit(attribute) else { continue };
        let name = e3d_attlib::lookup_system_name(attribute.hash)
            .map(str::to_string)
            .or_else(|| e3d_attlib::db1_dehash(attribute.hash))
            .unwrap_or_else(|| format!("{:#x}", attribute.hash));
        let mut pages = 0usize;
        let mut next = entry.address();
        while let Some(address) = next {
            let page = cache.get(PageId::primary(address.page_no)).map_err(|e| e.to_string())?;
            let block = hunk_block_at(page, address.byte_offset(), refno)
                .ok_or_else(|| format!("{refno}.{name}: no hunk block at page {} byte {}", address.page_no, address.byte_offset()))?;
            pages += 1;
            next = block.continues_at;
        }
        out.push((name, entry.hunk_type, entry.words, pages));
    }
    Ok(out)
}

/// The second session: a GPWL under the WORL, and under it a GROU whose members
/// are references to the ZONE and the new BOX. Returns the GROU's refno.
fn write_group(db: &str, provider: &mut TemplateProvider, zone_refno: RefNo, box_refno: RefNo) -> Result<RefNo, String> {
    let grou_template = template_of(provider, "GROU")?;
    if grou_template.member_list_kind != MemberListKind::Referenced {
        return Err(format!(
            "the schema's GROU template says its member list is {:?}, not a reference list",
            grou_template.member_list_kind
        ));
    }
    let world = world_refno(db)?;
    let gpwl_hash = e3d_attlib::db1_hash("GPWL");
    let grou_hash = e3d_attlib::db1_hash("GROU");

    let (gpwl, grou, sesno, previous, appended) = {
        let mut tx = Transaction::open(db, &mut *provider).map_err(|e| e.to_string())?;
        let gpwl = tx.insert(world, gpwl_hash, None, |_, _| Ok(())).map_err(|e| e.to_string())?;
        // GROU's members are references (owned elsewhere): the facade leaves a
        // reference list to us, so we set them on the draft.
        let grou = tx
            .insert(gpwl, grou_hash, None, |draft, _| {
                draft.push_member(zone_refno);
                draft.push_member(box_refno);
                Ok(())
            })
            .map_err(|e| e.to_string())?;
        let report = tx
            .commit(&session("e3d-io write-path gate: GPWL + GROU with reference members"))
            .map_err(|e| e.to_string())?;
        (gpwl, grou, report.written.sesno, report.previous.sesno, report.appended_pages)
    };

    println!();
    println!("wrote session {sesno} to {db}");
    println!("  previous session : {previous}");
    println!("  new GPWL         : {gpwl} (owner WORL {world}, appended to its member list)");
    println!("  new GROU         : {grou} (owner GPWL {gpwl})");
    println!("  GROU members     : {zone_refno} (the ZONE) and {box_refno} (the BOX) — references, owned elsewhere");
    println!("  appended pages   : {appended:?}");
    println!();
    println!("In E3D 3.1 Design, on this database's project:");
    println!("  Q SESSIONS                 -> the newest should be session {sesno} by user e3dio");
    println!("  ={}/{}   Q ATT   Q MEM      -> GROU under GPWL {gpwl}; members the ZONE and the BOX", grou.word0, grou.word1);
    println!("  ={}/{}   Q OWNER            -> the BOX still says OWNER the ZONE, not the group", box_refno.word0, box_refno.word1);
    println!("  ={}/{}   Q MEM              -> the WORL lists the GPWL behind its old members", world.word0, world.word1);
    println!("PASS = E3D navigates to the GROU, lists both reference members, and the BOX keeps its ZONE owner.");
    Ok(grou)
}

/// The `--move` sessions (plan T1a). First: three named BOXes A, B, C under
/// the second ZONE, and — with `--group` — A and B added to the GROU's
/// reference list. Then, in one transaction: A is moved under the third ZONE
/// (`move_to`, E3D's `INCLUDE`), C is put before B in the second ZONE's list
/// (`reorder`), and B is taken out of the group's references.
fn write_moves(db: &str, provider: &mut TemplateProvider, format: TextFormat, grou: Option<RefNo>) -> Result<(), String> {
    let all = zones(db, 3)?;
    let (zone2, zone3) = (all[1], all[2]);
    for name in MOVE_BOX_NAMES {
        name_is_free(db, name)?;
    }
    let box_hash = e3d_attlib::db1_hash("BOX");

    // --- session: the three boxes, and two of them referenced by the group ---
    let (boxes, sesno, previous, appended) = {
        let mut tx = Transaction::open(db, &mut *provider).map_err(|e| e.to_string())?;
        let mut boxes = Vec::new();
        for (n, name) in MOVE_BOX_NAMES.iter().enumerate() {
            let refno = tx
                .insert(zone2, box_hash, Some(name), |draft, template| {
                    build_box(draft, template, format)?;
                    let offset = 1000.0 * (n as f64 + 1.0);
                    draft.set_attribute(
                        template,
                        e3d_attlib::db1_hash("POS"),
                        &DescriptorValue::RealArray(vec![offset, 2000.0, 3000.0]),
                        format,
                    )?;
                    Ok(())
                })
                .map_err(|e| format!("{name}: {e}"))?;
            boxes.push(refno);
        }
        if let Some(grou) = grou {
            tx.add_reference_member(grou, boxes[0], MemberPosition::Last).map_err(|e| e.to_string())?;
            tx.add_reference_member(grou, boxes[1], MemberPosition::Last).map_err(|e| e.to_string())?;
        }
        let report = tx
            .commit(&session("e3d-io write-path gate: three BOXes under the second ZONE, two referenced by the GROU"))
            .map_err(|e| e.to_string())?;
        (boxes, report.written.sesno, report.previous.sesno, report.appended_pages)
    };
    println!();
    println!("wrote session {sesno} to {db}");
    println!("  previous session : {previous}");
    for (refno, name) in boxes.iter().zip(MOVE_BOX_NAMES) {
        println!("  new BOX {}       : {refno} ({name}, owner ZONE {zone2})", &name[name.len() - 1..]);
    }
    println!("  owner ZONE 2     : {zone2}");
    println!("  owner ZONE 3     : {zone3}");
    if let Some(grou) = grou {
        println!("  GROU {grou} references: {} and {}", boxes[0], boxes[1]);
    }
    println!("  appended pages   : {appended:?}");

    // --- session: A -> ZONE 3 (INCLUDE), C before B (REORDER), B out of the group ---
    let (sesno, previous, appended) = {
        let mut tx = Transaction::open(db, &mut *provider).map_err(|e| e.to_string())?;
        tx.move_to(boxes[0], zone3, MemberPosition::Last).map_err(|e| format!("move A: {e}"))?;
        tx.reorder(boxes[2], MemberPosition::Before(boxes[1])).map_err(|e| format!("reorder C: {e}"))?;
        if let Some(grou) = grou {
            tx.remove_reference_member(grou, boxes[1]).map_err(|e| format!("unreference B: {e}"))?;
        }
        let report = tx
            .commit(&session("e3d-io write-path gate: move A to the third ZONE, C before B, B out of the GROU"))
            .map_err(|e| e.to_string())?;
        (report.written.sesno, report.previous.sesno, report.appended_pages)
    };
    // Read back through this crate before E3D gets to look.
    let (a_owner, _) = owner_and_members(db, boxes[0])?;
    let (_, zone2_members) = owner_and_members(db, zone2)?;
    let (_, zone3_members) = owner_and_members(db, zone3)?;
    if a_owner != zone3 || zone3_members.last() != Some(&boxes[0]) || zone2_members.contains(&boxes[0]) {
        return Err(format!("move read-back: A owner {a_owner}, ZONE 3 last {:?}", zone3_members.last()));
    }
    let (c_at, b_at) = (
        zone2_members.iter().position(|m| *m == boxes[2]),
        zone2_members.iter().position(|m| *m == boxes[1]),
    );
    if !matches!((c_at, b_at), (Some(c), Some(b)) if c + 1 == b) {
        return Err(format!("reorder read-back: C at {c_at:?}, B at {b_at:?}"));
    }
    println!();
    println!("wrote session {sesno} to {db}");
    println!("  previous session : {previous}");
    println!("  moved            : {} {} -> owner ZONE {zone3}, last member", boxes[0], MOVE_BOX_NAMES[0]);
    println!("  reordered        : {} {} before {} {}", boxes[2], MOVE_BOX_NAMES[2], boxes[1], MOVE_BOX_NAMES[1]);
    if let Some(grou) = grou {
        let (_, grou_members) = owner_and_members(db, grou)?;
        if !grou_members.contains(&boxes[0]) || grou_members.contains(&boxes[1]) {
            return Err(format!("reference read-back: GROU members {grou_members:?}"));
        }
        println!("  unreferenced     : {} {} out of GROU {grou} (which keeps {})", boxes[1], MOVE_BOX_NAMES[1], boxes[0]);
    }
    println!("  appended pages   : {appended:?}");
    println!();
    println!("In E3D 3.1 Design, on this database's project:");
    println!("  ={}/{}   Q OWNER            -> ZONE {zone3}", boxes[0].word0, boxes[0].word1);
    println!("  ={}/{}   Q MEM              -> {} before {}, no {}", zone2.word0, zone2.word1, MOVE_BOX_NAMES[2], MOVE_BOX_NAMES[1], MOVE_BOX_NAMES[0]);
    println!("  ={}/{}   Q MEM              -> {} last", zone3.word0, zone3.word1, MOVE_BOX_NAMES[0]);
    if let Some(grou) = grou {
        println!("  ={}/{}   Q MEM              -> {} listed, {} not", grou.word0, grou.word1, MOVE_BOX_NAMES[0], MOVE_BOX_NAMES[1]);
    }
    Ok(())
}

/// The `--big` session (plan §1.3 #11): a new ZONE under the first ZONE's SITE
/// with [`BIG_ZONE_MEMBERS`] BOXes, so its member block is 16 400 words and the
/// format word's fourteen-bit member count is written saturated at `0x3FFF`.
/// E3D then has to measure the chain; before the fix it read `16 400 & 0x3FFF`
/// = 17 words = 8 members.
fn write_big(db: &str, provider: &mut TemplateProvider, format: TextFormat, zone_refno: RefNo) -> Result<(), String> {
    name_is_free(db, BIG_ZONE_NAME)?;
    let (site, _) = owner_and_members(db, zone_refno)?;
    let box_hash = e3d_attlib::db1_hash("BOX");
    let zone_hash = e3d_attlib::db1_hash("ZONE");
    let started = std::time::Instant::now();
    let (big_zone, sesno, previous, appended) = {
        let mut tx = Transaction::open(db, &mut *provider).map_err(|e| e.to_string())?;
        let big_zone = tx.insert(site, zone_hash, Some(BIG_ZONE_NAME), |_, _| Ok(())).map_err(|e| e.to_string())?;
        for n in 0..BIG_ZONE_MEMBERS {
            tx.insert(big_zone, box_hash, None, |draft, template| {
                build_box(draft, template, format)?;
                draft.set_attribute(
                    template,
                    e3d_attlib::db1_hash("POS"),
                    &DescriptorValue::RealArray(vec![f64::from(n) * 1000.0, 0.0, 0.0]),
                    format,
                )?;
                Ok(())
            })
            .map_err(|e| format!("box {n}: {e}"))?;
        }
        let report = tx
            .commit(&session("e3d-io write-path gate: a ZONE with 8200 BOXes (format word member count saturated)"))
            .map_err(|e| e.to_string())?;
        (big_zone, report.written.sesno, report.previous.sesno, report.appended_pages)
    };
    let elapsed = started.elapsed();
    let (_, members) = owner_and_members(db, big_zone)?;
    if members.len() != BIG_ZONE_MEMBERS as usize {
        return Err(format!("big ZONE read-back: {} members, {BIG_ZONE_MEMBERS} written", members.len()));
    }
    let word10 = {
        let mut engine = ReadOnlyEngine::open(db).map_err(|e| e.to_string())?;
        let view = engine.find_element(big_zone).map_err(|e| e.to_string())?.ok_or("big ZONE not indexed")?;
        u32::from_be_bytes(view.raw_bytes[40..44].try_into().unwrap())
    };
    if word10 & 0x3FFF != 0x3FFF {
        return Err(format!("big ZONE format word {word10:#010x}: the member count did not saturate"));
    }
    println!();
    println!("wrote session {sesno} to {db}");
    println!("  previous session : {previous}");
    println!("  new ZONE big     : {big_zone} ({BIG_ZONE_NAME}, owner SITE {site})");
    println!("  big members      : {} BOXes, {} .. {}", members.len(), members[0], members[members.len() - 1]);
    println!("  format word      : {word10:#010x} (member count saturated at 0x3FFF, explicit count {})", (word10 >> 14) & 0x3FFF);
    println!("  appended pages   : {} pages, {elapsed:.1?}", appended.len());
    println!();
    println!("In E3D 3.1 Design, on this database's project:");
    println!("  ={}/{}   Q MEM              -> {} BOX lines (8 before the format word was saturated)", big_zone.word0, big_zone.word1, members.len());
    Ok(())
}

/// The third and fourth sessions: two EQUIs whose DESC/DESP/UUIDIA are hunks,
/// then a FIXING whose ROKEYS is an inline string list and ROVALS the same list
/// as a hunk. All under the ZONE, each added to its member list by the facade.
fn write_hunks(
    db: &str,
    provider: &mut TemplateProvider,
    _format: TextFormat,
    zone_refno: RefNo,
    _box_refno: RefNo,
) -> Result<(), String> {
    // --- session: hunks on two EQUIs ---
    let equi_template = template_of(provider, "EQUI")?;
    let values = [HunkValues::for_element(1), HunkValues::for_element(2)];
    let desc = explicit_only(&equi_template, "DESC", values[0].desc.len())?;
    let desp = explicit_only(&equi_template, "DESP", values[0].desp.len())?;
    let uuidia = explicit_only(&equi_template, "UUIDIA", values[0].uuidia.len())?;
    let equi_hash = e3d_attlib::db1_hash("EQUI");

    let (equis, hunk_sesno, hunk_prev, hunk_appended) = {
        let mut tx = Transaction::open(db, &mut *provider).map_err(|e| e.to_string())?;
        let mut equis = Vec::new();
        for value in &values {
            let desc_text = value.desc.clone();
            let desp_reals = value.desp.clone();
            let uuidia_ints = value.uuidia;
            let equi = tx
                .insert(zone_refno, equi_hash, None, |draft, _| {
                    draft.set_text_hunk_ascii(desc, &desc_text)?;
                    draft.set_real_array_hunk(desp, &desp_reals)?;
                    draft.set_int_array_hunk(uuidia, &uuidia_ints)?;
                    Ok(())
                })
                .map_err(|e| e.to_string())?;
            equis.push(equi);
        }
        let report = tx
            .commit(&session("e3d-io write-path gate: two EQUIs with DESC/DESP/UUIDIA as hunks (types 22/21/20)"))
            .map_err(|e| e.to_string())?;
        (equis, report.written.sesno, report.previous.sesno, report.appended_pages)
    };

    println!();
    println!("wrote session {hunk_sesno} to {db}");
    println!("  previous session : {hunk_prev}");
    let mut spanning = 0usize;
    for (n, (equi, value)) in equis.iter().zip(&values).enumerate() {
        println!("  new EQUI {}       : {equi} (owner ZONE {zone_refno}, appended to its member list)", n + 1);
        println!("    DESC   (hunk 22): {} characters: {:?}", value.desc.len(), value.desc);
        println!(
            "    DESP   (hunk 21): {} reals, {} {} … {}",
            value.desp.len(),
            value.desp[0],
            value.desp[1],
            value.desp[value.desp.len() - 1]
        );
        println!("    UUIDIA (hunk 20): {:?}", value.uuidia);
        for (name, hunk_type, words, pages) in hunk_spans(db, *equi)? {
            println!("    on disk: {name} type {hunk_type}, {words} words in a chain of {pages} page(s)");
            if pages > 1 {
                spanning += 1;
            }
        }
    }
    println!("  hunk chains that cross a page boundary: {spanning}");
    if spanning == 0 {
        println!("  (warning: no chain spans a page; the continuation link goes unexercised by this run)");
    }
    println!("  appended pages   : {hunk_appended:?}");

    // --- session: a string list, inline and as a hunk, on a FIXING ---
    let fixing_template = template_of(provider, "FIXING")?;
    let keys = ["K1", "K2"];
    let vals = ["V1", "V2"];
    let rokeys = explicit_only(&fixing_template, "ROKEYS", 15)?;
    let rovals = explicit_only(&fixing_template, "ROVALS", 15)?;
    let fixing_hash = e3d_attlib::db1_hash("FIXING");

    let (fixing, list_sesno, list_prev, list_appended) = {
        let mut tx = Transaction::open(db, &mut *provider).map_err(|e| e.to_string())?;
        let fixing = tx
            .insert(zone_refno, fixing_hash, None, |draft, _| {
                draft.set_explicit_text_list_ascii(rokeys, keys)?;
                draft.set_text_list_hunk_ascii(rovals, vals)?;
                Ok(())
            })
            .map_err(|e| e.to_string())?;
        let report = tx
            .commit(&session("e3d-io write-path gate: FIXING with ROKEYS inline (type 19) and ROVALS as a hunk (type 23)"))
            .map_err(|e| e.to_string())?;
        (fixing, report.written.sesno, report.previous.sesno, report.appended_pages)
    };

    println!();
    println!("wrote session {list_sesno} to {db}");
    println!("  previous session : {list_prev}");
    println!("  new FIXING       : {fixing} (owner ZONE {zone_refno}, appended to its member list)");
    println!("  ROKEYS (inline 19): {keys:?}");
    println!("  ROVALS (hunk 23)  : {vals:?}");
    println!("  appended pages   : {list_appended:?}");
    println!();
    println!("In E3D 3.1 Design, on this database's project:");
    println!("  Q SESSION                  -> the last saved session should be {list_sesno} by user e3dio");
    for equi in &equis {
        println!("  ={}/{}   Q ATT             -> an EQUI under the ZONE; then:", equi.word0, equi.word1);
        println!("             Q DESC           -> the 120-character text above, markers in step");
        println!("             Q DESP           -> the hundred reals above");
        println!("             Q UUIDIA         -> the four ints above");
    }
    println!("  ={}/{}   Q ATT             -> the FIXING under the ZONE; then:", fixing.word0, fixing.word1);
    println!("             Q ROKEYS         -> the two keys above (a string list stored inline)");
    println!("             Q ROVALS         -> the two values above (the same list stored as a hunk)");
    println!("PASS (session {hunk_sesno}) = E3D shows DESC, DESP and UUIDIA of both EQUIs with the values above — it followed the hunk chains, one of them across pages.");
    println!("PASS (session {list_sesno}) = E3D shows both lists — it read a type-19 list inline and out of a hunk.");
    println!("If E3D balks, note which session: a hunk problem shows on the EQUIs, a string-list problem on the FIXING.");
    Ok(())
}

/// The `--strings` session (plan T1b): every value typed as text and set by
/// attribute name through `Transaction::set`, under E3D's dictionary. An EQUI
/// `/E3DIO-STR-EQUI` under the first ZONE gets `DESC` / `POS` / `FUNC` /
/// `PURP`; a FIXING `/E3DIO-STR-FIXING` gets `ROKEYS` (a `|` list), `BUIL`
/// (`true`), `NUMB` and `CREF` — the ZONE, by its `/NAME` when it has one.
/// Before the values go in, the dictionary is shown refusing a 121-character
/// `DESC`, a four-element `POS` and `RATM`, none of which reaches the record.
fn write_strings(db: &str, provider: &mut TemplateProvider, zone_refno: RefNo) -> Result<(), String> {
    for name in [STR_EQUI_NAME, STR_FIXING_NAME] {
        name_is_free(db, name)?;
    }
    let dictionary = Dictionary::read(DICTIONARY).map_err(|e| e.to_string())?;
    // The ZONE's stored name, for a `/NAME` reference; its refno otherwise.
    let zone_name = {
        let mut engine = ReadOnlyEngine::open(db).map_err(|e| e.to_string())?;
        let view = engine.find_element(zone_refno).map_err(|e| e.to_string())?.ok_or("the ZONE is not indexed")?;
        let parsed = e3d_io::record::element::ParsedElement::parse_as(&view.raw_bytes, engine.text_format())
            .map_err(|e| e.to_string())?;
        e3d_io::element_name::stored_name(&parsed)
    };
    let cref_text = zone_name.clone().unwrap_or_else(|| format!("={}/{}", zone_refno.word0, zone_refno.word1));

    let (equi, fixing, refused, sesno, previous, appended) = {
        let mut tx = Transaction::open(db, &mut *provider).map_err(|e| e.to_string())?.with_dictionary(&dictionary);
        let equi = tx
            .insert(zone_refno, e3d_attlib::db1_hash("EQUI"), Some(STR_EQUI_NAME), |_, _| Ok(()))
            .map_err(|e| e.to_string())?;
        // What the dictionary refuses, before anything is written.
        let mut refused = Vec::new();
        let long_desc = "x".repeat(121);
        for (attr, text) in [("DESC", long_desc.as_str()), ("POS", "1 2 3 4"), ("RATM", "1")] {
            match tx.set(equi, attr, text) {
                Err(TransactionError::Dictionary(error @ DictionaryError::TextTooLong { .. }))
                | Err(TransactionError::Dictionary(error @ DictionaryError::TooManyElements { .. }))
                | Err(TransactionError::Dictionary(error @ DictionaryError::NotInDictionary { .. })) => {
                    refused.push(format!("{attr} = {:?}: {error}", if text.len() > 24 { &text[..24] } else { text }));
                }
                Ok(()) => return Err(format!("the dictionary let {attr} = {text:?} through")),
                Err(other) => return Err(format!("{attr} = {text:?}: {other}, not a dictionary refusal")),
            }
        }
        let set = |tx: &mut Transaction<'_>, refno: RefNo, attr: &str, text: &str| {
            tx.set(refno, attr, text).map_err(|e| format!("{attr} = {text:?}: {e}"))
        };
        set(&mut tx, equi, "DESC", STR_EQUI_DESC)?;
        set(&mut tx, equi, "POS", STR_EQUI_POS)?;
        set(&mut tx, equi, "FUNC", STR_EQUI_FUNC)?;
        set(&mut tx, equi, "PURP", STR_EQUI_PURP)?;
        let fixing = tx
            .insert(zone_refno, e3d_attlib::db1_hash("FIXING"), Some(STR_FIXING_NAME), |_, _| Ok(()))
            .map_err(|e| e.to_string())?;
        set(&mut tx, fixing, "ROKEYS", STR_FIXING_ROKEYS)?;
        set(&mut tx, fixing, "BUIL", "true")?;
        set(&mut tx, fixing, "NUMB", STR_FIXING_NUMB)?;
        set(&mut tx, fixing, "CREF", &cref_text)?;
        let report = tx
            .commit(&session("e3d-io write-path gate: an EQUI and a FIXING set by attribute name from text (Transaction::set)"))
            .map_err(|e| e.to_string())?;
        (equi, fixing, refused, report.written.sesno, report.previous.sesno, report.appended_pages)
    };
    // Read back through this crate before E3D gets to look: the `/NAME`
    // reference resolved to the ZONE's refno, wherever the descriptor keeps it.
    {
        let fixing_template = template_of(provider, "FIXING")?;
        let cref = fixing_template
            .attrs
            .iter()
            .find(|attr| attr.hash == e3d_attlib::db1_hash("CREF"))
            .ok_or("the FIXING template has no CREF")?;
        let mut engine = ReadOnlyEngine::open(db).map_err(|e| e.to_string())?;
        let view = engine.find_element(fixing).map_err(|e| e.to_string())?.ok_or("the FIXING is not indexed")?;
        let draft = RecordDraft::from_record_bytes(&view.raw_bytes).map_err(|e| e.to_string())?;
        let offset = cref.wide.location.word_offset() as usize;
        let words: Vec<u32> = if offset != 0 {
            draft.implicit().get(offset..offset + 2).map(<[u32]>::to_vec).unwrap_or_default()
        } else {
            draft.explicit_words(cref.hash).map(|(_, words)| words).unwrap_or_default()
        };
        if words != [zone_refno.word0, zone_refno.word1] {
            return Err(format!("CREF read-back: {words:?}, wanted {zone_refno}"));
        }
    }
    println!();
    println!("wrote session {sesno} to {db}");
    println!("  previous session : {previous}");
    println!("  dictionary       : {} attributes from {DICTIONARY}", dictionary.len());
    for line in &refused {
        println!("  refused          : {line}");
    }
    println!("  new EQUI str     : {equi} ({STR_EQUI_NAME}, owner ZONE {zone_refno})");
    println!("    DESC = {STR_EQUI_DESC:?}");
    println!("    POS  = {STR_EQUI_POS:?}   FUNC = {STR_EQUI_FUNC:?}   PURP = {STR_EQUI_PURP:?}");
    println!("  new FIXING str   : {fixing} ({STR_FIXING_NAME}, owner ZONE {zone_refno})");
    println!("    ROKEYS = {STR_FIXING_ROKEYS:?}   BUIL = \"true\"   NUMB = {STR_FIXING_NUMB:?}");
    println!("    CREF = {cref_text:?} -> {zone_refno}");
    println!("  CREF text        : {cref_text}");
    println!("  appended pages   : {appended:?}");
    println!();
    println!("In E3D 3.1 Design, on this database's project:");
    println!("  ={}/{}   Q DESC / Q POS / Q FUNC / Q PURP -> the texts above, verbatim", equi.word0, equi.word1);
    println!("  ={}/{}   Q ROKEYS / Q BUIL / Q NUMB / Q CREF -> S1 S2 S3, true, 42, the ZONE", fixing.word0, fixing.word1);
    Ok(())
}

/// The `--merge-from` run (plan T1c): one transaction opened at `base`, a
/// session the database has moved past — E3D saved in between — that inserts
/// `/E3DIO-MERGE-BOX` under the first ZONE and sets the second ZONE's `FUNC`,
/// and commits as a merge onto the newest session. What the merge did is
/// printed: the base, the session merged onto, any re-minted refno.
fn write_merge(db: &str, provider: &mut TemplateProvider, base: u32) -> Result<(), String> {
    name_is_free(db, MERGE_BOX_NAME)?;
    let all = zones(db, 2)?;
    let (zone1, zone2) = (all[0], all[1]);
    let latest_before = ReadOnlyEngine::open(db).map_err(|e| e.to_string())?.session().sesno;
    if latest_before == base {
        return Err(format!("the newest session is {base}, the base: nothing to merge onto — save something in E3D first"));
    }
    let (_, zone1_before) = owner_and_members(db, zone1)?;

    let (given, commit) = {
        let mut tx = Transaction::open_at(db, SessionSelector::Exact(base), &mut *provider).map_err(|e| e.to_string())?;
        let given = tx
            .insert(zone1, e3d_attlib::db1_hash("BOX"), Some(MERGE_BOX_NAME), |_, _| Ok(()))
            .map_err(|e| e.to_string())?;
        tx.set(given, "XLEN", MERGE_BOX_XLEN).map_err(|e| e.to_string())?;
        tx.set(given, "YLEN", MERGE_BOX_XLEN).map_err(|e| e.to_string())?;
        tx.set(given, "ZLEN", MERGE_BOX_XLEN).map_err(|e| e.to_string())?;
        tx.set(given, "POS", "5000 5000 5000").map_err(|e| e.to_string())?;
        tx.set(zone2, "FUNC", MERGE_ZONE_FUNC).map_err(|e| e.to_string())?;
        let commit = tx.commit_reporting(&session(MERGE_COMMENT)).map_err(|e| e.to_string())?;
        (given, commit)
    };
    let written = commit.reminted.iter().find(|(old, _)| *old == given).map_or(given, |(_, new)| *new);
    // Read back through this crate before E3D gets to look: the box is under
    // the ZONE, the ZONE's list is the committed one plus it, the FUNC is set.
    let (box_owner, _) = owner_and_members(db, written)?;
    let (_, zone1_after) = owner_and_members(db, zone1)?;
    if box_owner != zone1 || !zone1_after.contains(&written) || zone1_after.len() != zone1_before.len() + 1 {
        return Err(format!(
            "merge read-back: box owner {box_owner}, ZONE 1 had {} members, has {}",
            zone1_before.len(),
            zone1_after.len()
        ));
    }
    println!("wrote session {} to {db}", commit.report.written.sesno);
    println!("  previous session : {}", commit.report.previous.sesno);
    println!("  merge base       : {}", commit.base_sesno);
    println!("  merged onto      : {}", commit.merged_onto.map_or("(nothing: written on the base)".to_string(), |s| s.to_string()));
    println!("  refno given      : {given}");
    println!("  new BOX merge    : {written} ({MERGE_BOX_NAME}, owner ZONE {zone1})");
    println!("  re-minted        : {:?}", commit.reminted);
    println!("  retried on Moved : {}", commit.retried_after_move);
    println!("  ZONE 2 FUNC      : {zone2} = {MERGE_ZONE_FUNC}");
    println!("  appended pages   : {:?}", commit.report.appended_pages);
    println!();
    println!("In E3D 3.1 Design, on this database's project:");
    println!("  ={}/{}   Q MEM              -> what E3D saved, then {MERGE_BOX_NAME}", zone1.word0, zone1.word1);
    println!("  ={}/{}   Q NAME / Q OWNER / Q XLEN -> {MERGE_BOX_NAME}, the ZONE, {MERGE_BOX_XLEN}mm", written.word0, written.word1);
    println!("  ={}/{}   Q FUNC            -> {MERGE_ZONE_FUNC}", zone2.word0, zone2.word1);
    Ok(())
}

/// The fifth session: the BOX gets a Chinese `NAME` (record and `NAME` table
/// key alike, via `rename`), the ZONE a Chinese `DESC`, and the session a
/// Chinese comment — every one through the header's `FORMAT`.
fn write_text(
    db: &str,
    provider: &mut TemplateProvider,
    format: TextFormat,
    zone_refno: RefNo,
    box_refno: RefNo,
) -> Result<(), String> {
    {
        let mut engine = ReadOnlyEngine::open(db).map_err(|e| e.to_string())?;
        if engine.find_by_name(TEXT_BOX_NAME).map_err(|e| e.to_string())?.is_some() {
            return Err(format!("{TEXT_BOX_NAME} is already in use"));
        }
    }
    let desc_hash = e3d_attlib::db1_hash("DESC");

    let (sesno, previous, appended) = {
        let mut tx = Transaction::open(db, &mut *provider).map_err(|e| e.to_string())?;
        tx.rename(box_refno, TEXT_BOX_NAME).map_err(|e| e.to_string())?;
        tx.update(zone_refno, |draft, template| {
            draft.set_attribute(template, desc_hash, &DescriptorValue::Text(TEXT_ZONE_DESC.into()), format)?;
            Ok(())
        })
        .map_err(|e| e.to_string())?;
        let report = tx.commit(&session(TEXT_COMMENT)).map_err(|e| e.to_string())?;
        (report.written.sesno, report.previous.sesno, report.appended_pages)
    };

    println!();
    println!("wrote session {sesno} to {db}");
    println!("  previous session : {previous}");
    println!("  text format      : {format:?}");
    println!("  BOX NAME         : {TEXT_BOX_NAME}");
    println!("  ZONE DESC        : {TEXT_ZONE_DESC}");
    println!("  comment          : {TEXT_COMMENT}");
    println!("  appended pages   : {appended:?}");
    println!();
    println!("In E3D 3.1 Design, on this database's project:");
    println!("  ={}/{}   Q NAME             -> {TEXT_BOX_NAME}", box_refno.word0, box_refno.word1);
    println!("  ={}/{}   Q DESC             -> {TEXT_ZONE_DESC}", zone_refno.word0, zone_refno.word1);
    println!("  Q SESSIONS                 -> session {sesno} commented {TEXT_COMMENT}");
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("e3d-write-box: {error}");
            ExitCode::FAILURE
        }
    }
}
