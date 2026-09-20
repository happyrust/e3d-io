//! Append one session to a dabacon DESIGN database: a new `BOX` under the
//! first `ZONE`, added to that ZONE's member list. This is the fixture for the
//! E3D manual gate (`docs/plans/2026-09-20-dabacon-write-plan.md` §2 阶段 2 用例
//! A / §7 Q8) — write with this crate, then open the database in AVEVA E3D 3.1
//! Design and `Q SESSIONS` / `Q ATT` the box to see whether E3D reads back what
//! was written.
//!
//! ```text
//! e3d-write-box <database> [desvir.dat] [--group]
//! ```
//!
//! With `--group` it appends a **second** session after the box: a `GPWL`
//! under the `WORL` and a `GROU` under that, whose members are *references* —
//! the ZONE and the new BOX, both owned elsewhere — the way E3D's own groups
//! hold elements (the noun's template says so: header word 3 is 2, see
//! `MemberListKind::Referenced`). That session asks E3D a second question: does
//! it accept a member list whose members do not name this element as owner?
//! Keeping it a separate session tells the two questions apart if E3D balks.
//!
//! It writes the database **in place**, so point it at a copy (or a backed-up
//! project DB); the write is an append plus the page-0 commit, so restoring the
//! original bytes undoes it. It prints the session numbers, the RefNos and
//! owners it wrote, and the geometry, so the values can be checked in E3D.

use std::process::ExitCode;

use e3d_io::engine::ScanTier;
use e3d_io::record::template::{MemberListKind, NounTemplate, TemplateProvider};
use e3d_io::write::record::{put_real_vec_wide, put_real_wide};
use e3d_io::write::{write_element_session, ElementWrite, RecordDraft, SessionRequest};
use e3d_io::{ReadOnlyEngine, RefNo};

const DEFAULT_DESVIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schema/e3d31/binaries/desvir.dat");
const USAGE: &str = "usage: e3d-write-box <database> [desvir.dat] [--group]";

fn wide_offset(template: &NounTemplate, name: &str) -> Option<usize> {
    template
        .attrs
        .iter()
        .find(|a| {
            e3d_attlib::lookup_system_name(a.hash) == Some(name)
                || e3d_attlib::db1_dehash(a.hash).as_deref() == Some(name)
        })
        .map(|a| a.wide.location.word_offset() as usize)
}

/// A fresh element of `template`'s noun under `owner`, carrying the noun's
/// default implicit record and the given members.
fn fresh_element(template: &NounTemplate, refno: RefNo, owner: RefNo, members: Vec<RefNo>) -> Result<RecordDraft, String> {
    let mut implicit = vec![0u32; 11];
    implicit.extend_from_slice(&template.defaults.implicit);
    RecordDraft::new(refno, template.noun_hash, owner, implicit, members, vec![]).map_err(|e| e.to_string())
}

/// Read an element back into a draft so it can be re-emitted with a change.
fn draft_of(db: &str, refno: RefNo, what: &str) -> Result<RecordDraft, String> {
    let mut engine = ReadOnlyEngine::open(db).map_err(|e| e.to_string())?;
    let view = engine
        .find_element(refno)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("{what} {refno} is not indexed"))?;
    RecordDraft::from_record_bytes(&view.raw_bytes).map_err(|e| e.to_string())
}

fn run() -> Result<(), String> {
    let mut db = None;
    let mut desvir = None;
    let mut group = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--group" => group = true,
            other if other.starts_with("--") => return Err(format!("unknown flag {other}\n{USAGE}")),
            _ if db.is_none() => db = Some(arg),
            _ if desvir.is_none() => desvir = Some(arg),
            _ => return Err(USAGE.to_string()),
        }
    }
    let db = db.ok_or(USAGE)?;
    let desvir = desvir.unwrap_or_else(|| DEFAULT_DESVIR.to_string());

    // Find the first ZONE to own the box, and a free RefNo well past the last.
    let (zone_refno, fresh) = {
        let mut engine = ReadOnlyEngine::open(&db).map_err(|e| format!("open {db}: {e}"))?;
        let zone_hash = e3d_attlib::db1_hash("ZONE");
        let mut zone = None;
        let mut max_word1 = 0u32;
        let mut word0 = 24_384;
        for element in engine.scan_elements(ScanTier::Header).map_err(|e| e.to_string())? {
            let element = element.map_err(|e| e.to_string())?;
            max_word1 = max_word1.max(element.refno.word1);
            if zone.is_none() && element.noun_hash == zone_hash {
                zone = Some(element.refno);
                word0 = element.refno.word0;
            }
        }
        let zone = zone.ok_or("the database indexes no ZONE to own a BOX")?;
        (zone, RefNo::new(word0, max_word1 + 5000))
    };

    // Build the box from the BOX template's defaults, with a distinctive size.
    let pos = [1000.0, 2000.0, 3000.0];
    let dims = [300.0, 400.0, 500.0];
    let box_hash = e3d_attlib::db1_hash("BOX");
    let mut provider = TemplateProvider::open(&desvir, None).map_err(|e| format!("open {desvir}: {e}"))?;
    let template = provider
        .template_for(box_hash)
        .map_err(|e| e.to_string())?
        .ok_or("the schema has no BOX template")?
        .clone();
    let mut implicit = vec![0u32; 11];
    implicit.extend_from_slice(&template.defaults.implicit);
    if let Some(offset) = wide_offset(&template, "POS") {
        put_real_vec_wide(&mut implicit, offset, &pos);
    }
    for (name, value) in [("XLEN", dims[0]), ("YLEN", dims[1]), ("ZLEN", dims[2])] {
        if let Some(offset) = wide_offset(&template, name) {
            put_real_wide(&mut implicit, offset, value);
        }
    }
    let box_draft = RecordDraft::new(fresh, box_hash, zone_refno, implicit, vec![], vec![])
        .map_err(|e| e.to_string())?;

    // Add the box to the ZONE's member list.
    let mut zone_draft = draft_of(&db, zone_refno, "the ZONE")?;
    zone_draft.push_member(fresh);

    let report = write_element_session(
        &db,
        &SessionRequest {
            user: "e3dio",
            comment: "e3d-io write-path gate: one BOX",
            clock: None,
        },
        &[ElementWrite::Insert(box_draft), ElementWrite::Update(zone_draft)],
        &[],
    )
    .map_err(|e| e.to_string())?;

    println!("wrote session {} to {db}", report.written.sesno);
    println!("  previous session : {}", report.previous.sesno);
    println!("  new BOX          : {} (word0={}, word1={})", fresh, fresh.word0, fresh.word1);
    println!("  owner ZONE       : {zone_refno}");
    println!("  POS              : {} {} {}", pos[0], pos[1], pos[2]);
    println!("  XLEN/YLEN/ZLEN   : {} / {} / {}", dims[0], dims[1], dims[2]);
    println!("  appended pages   : {:?}", report.appended_pages);
    println!();
    println!("In E3D 3.1 Design, on this database's project:");
    println!("  Q SESSIONS                 -> the newest should be session {} by user e3dio", report.written.sesno);
    println!("  Q ATT /{}                  -> POS/XLEN/YLEN/ZLEN as above, OWNER the ZONE", fresh);

    if group {
        write_group(&db, &mut provider, zone_refno, fresh)?;
    }
    Ok(())
}

/// The second session: a GPWL under the WORL, and under it a GROU whose
/// members are references to the ZONE and the new BOX — elements owned
/// elsewhere, as a group's members always are.
fn write_group(db: &str, provider: &mut TemplateProvider, zone_refno: RefNo, box_refno: RefNo) -> Result<(), String> {
    let template = |provider: &mut TemplateProvider, noun: &str| -> Result<NounTemplate, String> {
        provider
            .template_for(e3d_attlib::db1_hash(noun))
            .map_err(|e| e.to_string())?
            .cloned()
            .ok_or_else(|| format!("the schema has no {noun} template"))
    };
    let gpwl_template = template(provider, "GPWL")?;
    let grou_template = template(provider, "GROU")?;
    if grou_template.member_list_kind != MemberListKind::Referenced {
        return Err(format!(
            "the schema's GROU template says its member list is {:?}, not a reference list",
            grou_template.member_list_kind
        ));
    }

    // The WORL owns the GPWL; RefNos follow straight on from the box.
    let mut world_draft = {
        let mut engine = ReadOnlyEngine::open(db).map_err(|e| e.to_string())?;
        let world = engine
            .world()
            .map_err(|e| e.to_string())?
            .ok_or("the database indexes no WORL")?;
        RecordDraft::from_record_bytes(&world.raw_bytes).map_err(|e| e.to_string())?
    };
    let world_refno = world_draft.refno;
    let gpwl = RefNo::new(box_refno.word0, box_refno.word1 + 1);
    let grou = RefNo::new(box_refno.word0, box_refno.word1 + 2);
    world_draft.push_member(gpwl);
    let gpwl_draft = fresh_element(&gpwl_template, gpwl, world_refno, vec![grou])?;
    let members = vec![zone_refno, box_refno];
    let grou_draft = fresh_element(&grou_template, grou, gpwl, members.clone())?;

    let report = write_element_session(
        db,
        &SessionRequest {
            user: "e3dio",
            comment: "e3d-io write-path gate: GPWL + GROU with reference members",
            clock: None,
        },
        &[ElementWrite::Insert(gpwl_draft), ElementWrite::Insert(grou_draft), ElementWrite::Update(world_draft)],
        &[],
    )
    .map_err(|e| e.to_string())?;

    println!();
    println!("wrote session {} to {db}", report.written.sesno);
    println!("  previous session : {}", report.previous.sesno);
    println!("  new GPWL         : {gpwl} (owner WORL {world_refno}, appended to its member list)");
    println!("  new GROU         : {grou} (owner GPWL {gpwl})");
    println!("  GROU members     : {} (the ZONE) and {} (the BOX) — references, owned elsewhere", members[0], members[1]);
    println!("  appended pages   : {:?}", report.appended_pages);
    println!();
    println!("In E3D 3.1 Design, on this database's project:");
    println!("  Q SESSIONS                 -> the newest should be session {} by user e3dio", report.written.sesno);
    println!("  ={}/{}   Q ATT   Q MEM      -> GROU under GPWL {gpwl}; members the ZONE and the BOX", grou.word0, grou.word1);
    println!("  ={}/{}   Q OWNER            -> the BOX still says OWNER the ZONE, not the group", box_refno.word0, box_refno.word1);
    println!("  ={}/{}   Q MEM              -> the WORL lists the GPWL behind its old members", world_refno.word0, world_refno.word1);
    println!("PASS = E3D navigates to the GROU, lists both reference members, and the BOX keeps its ZONE owner.");
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
