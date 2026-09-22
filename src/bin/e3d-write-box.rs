//! Append one session to a dabacon DESIGN database: a new `BOX` under the
//! first `ZONE`, added to that ZONE's member list. This is the fixture for the
//! E3D manual gate (`docs/plans/2026-09-20-dabacon-write-plan.md` §2 阶段 2 用例
//! A / §7 Q8) — write with this crate, then open the database in AVEVA E3D 3.1
//! Design and `Q SESSIONS` / `Q ATT` the box to see whether E3D reads back what
//! was written.
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
//! With `--text` it appends a fifth session: the BOX gets a Chinese `NAME`
//! (record and `NAME` table key, through `Transaction::rename`), the ZONE a
//! Chinese `DESC`, and the session a Chinese comment — each stored the way the
//! header's `FORMAT` says (`e3d_io::record::text::encode_as`, plan S5).
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
use e3d_io::write::{RecordDraft, SessionRequest, Transaction};
use e3d_io::{ReadOnlyEngine, RefNo};

const DEFAULT_DESVIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schema/e3d31/binaries/desvir.dat");
const USAGE: &str = "usage: e3d-write-box <database> [desvir.dat] [--group] [--hunk] [--text]";

/// The `NAME` the `--text` session gives the BOX and the `DESC` it gives the
/// ZONE: text outside ASCII, stored as the header's `FORMAT` says.
pub const TEXT_BOX_NAME: &str = "/E3DIO-盒子";
pub const TEXT_ZONE_DESC: &str = "反应堆厂房 -3.320m~±0.00m";
pub const TEXT_COMMENT: &str = "e3d-io 写路径门：中文 NAME / DESC / 会话注释";

/// The database's header `FORMAT` reading, for text values.
fn text_format(db: &str) -> Result<TextFormat, String> {
    let engine = ReadOnlyEngine::open(db).map_err(|e| format!("open {db}: {e}"))?;
    Ok(TextFormat::from_header(engine.descriptor().header_format))
}

/// The first ZONE in the database, to own the box.
fn first_zone(db: &str) -> Result<RefNo, String> {
    let mut engine = ReadOnlyEngine::open(db).map_err(|e| format!("open {db}: {e}"))?;
    let zone_hash = e3d_attlib::db1_hash("ZONE");
    engine
        .scan_elements(ScanTier::Header)
        .map_err(|e| e.to_string())?
        .map(|e| e.map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .find(|e| e.noun_hash == zone_hash)
        .map(|e| e.refno)
        .ok_or_else(|| "the database indexes no ZONE to own a BOX".to_string())
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
    let mut text = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--group" => group = true,
            "--hunk" => hunk = true,
            "--text" => text = true,
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

    if group {
        write_group(&db, &mut provider, zone_refno, box_refno)?;
    }
    if hunk {
        write_hunks(&db, &mut provider, format, zone_refno, box_refno)?;
    }
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
/// are references to the ZONE and the new BOX.
fn write_group(db: &str, provider: &mut TemplateProvider, zone_refno: RefNo, box_refno: RefNo) -> Result<(), String> {
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
