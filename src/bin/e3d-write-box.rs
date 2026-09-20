//! Append one session to a dabacon DESIGN database: a new `BOX` under the
//! first `ZONE`, added to that ZONE's member list. This is the fixture for the
//! E3D manual gate (`docs/plans/2026-09-20-dabacon-write-plan.md` §2 阶段 2 用例
//! A / §7 Q8) — write with this crate, then open the database in AVEVA E3D 3.1
//! Design and `Q SESSIONS` / `Q ATT` the box to see whether E3D reads back what
//! was written.
//!
//! ```text
//! e3d-write-box <database> [desvir.dat]
//! ```
//!
//! It writes the database **in place**, so point it at a copy (or a backed-up
//! project DB); the write is an append plus the page-0 commit, so restoring the
//! original bytes undoes it. It prints the session number, the box's RefNo and
//! owner, and the geometry it wrote, so the values can be checked in E3D.

use std::process::ExitCode;

use e3d_io::engine::ScanTier;
use e3d_io::record::template::{NounTemplate, TemplateProvider};
use e3d_io::write::record::{put_real_vec_wide, put_real_wide};
use e3d_io::write::{write_element_session, ElementWrite, RecordDraft, SessionRequest};
use e3d_io::{ReadOnlyEngine, RefNo};

const DEFAULT_DESVIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schema/e3d31/binaries/desvir.dat");

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

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let db = args.next().ok_or("usage: e3d-write-box <database> [desvir.dat]")?;
    let desvir = args.next().unwrap_or_else(|| DEFAULT_DESVIR.to_string());

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
    let zone_draft = {
        let mut engine = ReadOnlyEngine::open(&db).map_err(|e| e.to_string())?;
        let view = engine
            .find_element(zone_refno)
            .map_err(|e| e.to_string())?
            .ok_or("the ZONE vanished between scans")?;
        let mut draft = RecordDraft::from_record_bytes(&view.raw_bytes).map_err(|e| e.to_string())?;
        draft.push_member(fresh);
        draft
    };

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
