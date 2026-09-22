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
//! With `--group` it appends a **second** session after the box: a `GPWL`
//! under the `WORL` and a `GROU` under that, whose members are *references* —
//! the ZONE and the new BOX, both owned elsewhere — the way E3D's own groups
//! hold elements (the noun's template says so: header word 3 is 2, see
//! `MemberListKind::Referenced`). That session asks E3D a second question: does
//! it accept a member list whose members do not name this element as owner?
//! Keeping it a separate session tells the two questions apart if E3D balks.
//!
//! With `--hunk` it appends two more sessions asking the third and fourth
//! questions — does E3D read a value stored **out of the record** (a *hunk*,
//! storage types 20–23, `e3d_io::record::hunk`), and does it read a **string
//! list** (storage type 19)? The first of them puts two `EQUI`s under the ZONE
//! whose `DESC` (text), `DESP` (real array) and `UUIDIA` (int array) are hunks
//! — 120 characters (E3D's own `DESC` limit; the template's 480 is the UTF-8
//! byte capacity, and 480 characters earn (2,170) "Attribute DESC too long"),
//! the descriptor's full 100 reals, and 4 ints — so that with two elements in
//! one session at least one chain runs across a page boundary (the second
//! element's hunks start wherever the first element's record ended); the bin
//! reads the chains back and says which;
//! the second puts a `FIXING` under the ZONE with `ROKEYS` as an inline type-19
//! list and `ROVALS` as the same list moved out into a type-23 hunk. Every one
//! of those attributes is one the noun's template declares with no implicit
//! slot, so E3D's `4GCEA` reads it from the explicit stream — the only place a
//! hunk entry can be met — and is one E3D's `Q` actually exposes (the
//! template's `RATM`/`UNIPAR` get "does not have attribute", and `USRWEI`
//! reads as unset however it is stored, so they prove nothing). Two sessions
//! again, so a balk at the string list does not muddy the hunk question.
//!
//! With `--text` it appends a fifth session asking whether E3D reads text
//! outside ASCII the way this crate stores it under the header's `FORMAT`
//! (`e3d_io::record::text::encode_as`, plan S5): the BOX gets a Chinese
//! `NAME` — record and `NAME` table key — the ZONE a Chinese `DESC` with an
//! escaped run inside plain text, and the session a Chinese comment.
//!
//! All sessions are written with the schema at hand (`WriteOptions`), so the
//! keyed tables follow: a `BOX` lists no `TYPEX` in its template and files no
//! key, while the `GPWL`, `GROU`, `EQUI` and `FIXING` get their `TYPEX` field
//! and key the way E3D's own elements carry them, and the `EQUI` its claim.
//!
//! It writes the database **in place**, so point it at a copy (or a backed-up
//! project DB); the write is an append plus the page-0 commit, so restoring the
//! original bytes undoes it. It prints the session numbers, the RefNos and
//! owners it wrote, and the geometry, so the values can be checked in E3D.

use std::process::ExitCode;

use e3d_io::engine::ScanTier;
use e3d_io::record::template::{MemberListKind, NounTemplate, TemplateProvider};
use e3d_io::write::record::{put_real_vec_wide, put_real_wide};
use e3d_io::write::{write_element_session_with, ElementWrite, RecordDraft, SessionRequest, WriteOptions};
use e3d_io::{ReadOnlyEngine, RefNo};

const DEFAULT_DESVIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schema/e3d31/binaries/desvir.dat");
const USAGE: &str = "usage: e3d-write-box <database> [desvir.dat] [--group] [--hunk] [--text]";

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

    let report = write_element_session_with(
        &db,
        &SessionRequest {
            user: "e3dio",
            comment: "e3d-io write-path gate: one BOX",
            clock: None,
        },
        &[ElementWrite::Insert(box_draft), ElementWrite::Update(zone_draft)],
        &[],
        WriteOptions {
            templates: Some(&mut provider),
        },
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
    if hunk {
        write_hunks(&db, &mut provider, zone_refno, fresh)?;
    }
    if text {
        write_text(&db, &mut provider, zone_refno, fresh)?;
    }
    Ok(())
}

/// The noun template for `noun`, or why there is none.
fn template_of(provider: &mut TemplateProvider, noun: &str) -> Result<NounTemplate, String> {
    provider
        .template_for(e3d_attlib::db1_hash(noun))
        .map_err(|e| e.to_string())?
        .cloned()
        .ok_or_else(|| format!("the schema has no {noun} template"))
}

/// Check that `template` declares `name` with no implicit slot and a declared
/// count of at least `count`, so E3D reads the value from the explicit stream
/// and the value keeps under the descriptor's size. Returns the attribute hash.
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

/// The values one gate EQUI carries: a text of 120 characters, a real array
/// of the descriptor's full 100, and four ints. 120 is E3D's own limit for
/// `DESC` — the template's 480 is the UTF-8 byte capacity behind it, and a
/// 480-character value is refused by `Q DESC` with (2,170) "Attribute DESC too
/// long"; 65 and 120 read back. With two such elements in one session the
/// second element's hunks start wherever the first element's record ended, so
/// at least one chain runs across a page boundary.
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
        // 120 characters with the position spelled out every five, so a cut or
        // a shifted word shows where it happened: "… (type 22) @070 @075 …".
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

/// How many pages each of a written element's hunk chains runs over, read
/// back through the directory entries — the placer's answer for whether the
/// E3D check exercises a chain that continues onto another page.
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
            let page = cache
                .get(PageId::primary(address.page_no))
                .map_err(|e| e.to_string())?;
            let block = hunk_block_at(page, address.byte_offset(), refno)
                .ok_or_else(|| format!("{refno}.{name}: no hunk block at page {} byte {}", address.page_no, address.byte_offset()))?;
            pages += 1;
            next = block.continues_at;
        }
        out.push((name, entry.hunk_type, entry.words, pages));
    }
    Ok(out)
}

/// The third and fourth sessions: two EQUIs whose DESC, DESP and UUIDIA are
/// hunks, then a FIXING whose ROKEYS is an inline string list and ROVALS the
/// same list as a hunk. All under the ZONE, each appended to its member list.
fn write_hunks(db: &str, provider: &mut TemplateProvider, zone_refno: RefNo, box_refno: RefNo) -> Result<(), String> {
    // --- session: hunks on two EQUIs ---
    // DESC (text → 22), DESP (reals → 21) and UUIDIA (ints → 20). These three
    // are declared on EQUI with no implicit slot and are ones E3D's `Q`
    // exposes; RATM and UNIPAR are in the template too but E3D answers "does
    // not have attribute", and USRWEI reads as unset however it is stored.
    let equi_template = template_of(provider, "EQUI")?;
    let values = [HunkValues::for_element(1), HunkValues::for_element(2)];
    let desc = explicit_only(&equi_template, "DESC", values[0].desc.len())?;
    let desp = explicit_only(&equi_template, "DESP", values[0].desp.len())?;
    let uuidia = explicit_only(&equi_template, "UUIDIA", values[0].uuidia.len())?;

    let equis = [
        RefNo::new(box_refno.word0, box_refno.word1 + 3),
        RefNo::new(box_refno.word0, box_refno.word1 + 4),
    ];
    let mut elements = Vec::new();
    for (equi, value) in equis.iter().zip(&values) {
        let mut draft = fresh_element(&equi_template, *equi, zone_refno, vec![])?;
        draft.set_text_hunk_ascii(desc, &value.desc).map_err(|e| e.to_string())?;
        draft.set_real_array_hunk(desp, &value.desp).map_err(|e| e.to_string())?;
        draft.set_int_array_hunk(uuidia, &value.uuidia).map_err(|e| e.to_string())?;
        elements.push(ElementWrite::Insert(draft));
    }
    let mut zone_draft = draft_of(db, zone_refno, "the ZONE")?;
    for equi in &equis {
        zone_draft.push_member(*equi);
    }
    elements.push(ElementWrite::Update(zone_draft));

    let report = write_element_session_with(
        db,
        &SessionRequest {
            user: "e3dio",
            comment: "e3d-io write-path gate: two EQUIs with DESC/DESP/UUIDIA as hunks (types 22/21/20)",
            clock: None,
        },
        &elements,
        &[],
        WriteOptions {
            templates: Some(provider),
        },
    )
    .map_err(|e| e.to_string())?;
    let hunk_sesno = report.written.sesno;

    println!();
    println!("wrote session {hunk_sesno} to {db}");
    println!("  previous session : {}", report.previous.sesno);
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
    println!("  appended pages   : {:?}", report.appended_pages);

    // --- session: a string list, inline and as a hunk, on a FIXING ---
    let fixing_template = template_of(provider, "FIXING")?;
    let keys = ["K1", "K2"];
    let vals = ["V1", "V2"];
    // The descriptor's count is 20 for these; a two-item list of two-character
    // strings reports 1 + 2 × (4 × 1 + 3) = 15, under it.
    let rokeys = explicit_only(&fixing_template, "ROKEYS", 15)?;
    let rovals = explicit_only(&fixing_template, "ROVALS", 15)?;
    let fixing = RefNo::new(box_refno.word0, box_refno.word1 + 5);
    let mut fixing_draft = fresh_element(&fixing_template, fixing, zone_refno, vec![])?;
    fixing_draft.set_explicit_text_list_ascii(rokeys, keys).map_err(|e| e.to_string())?;
    fixing_draft.set_text_list_hunk_ascii(rovals, vals).map_err(|e| e.to_string())?;
    let mut zone_draft = draft_of(db, zone_refno, "the ZONE")?;
    zone_draft.push_member(fixing);

    let report = write_element_session_with(
        db,
        &SessionRequest {
            user: "e3dio",
            comment: "e3d-io write-path gate: FIXING with ROKEYS inline (type 19) and ROVALS as a hunk (type 23)",
            clock: None,
        },
        &[ElementWrite::Insert(fixing_draft), ElementWrite::Update(zone_draft)],
        &[],
        WriteOptions {
            templates: Some(provider),
        },
    )
    .map_err(|e| e.to_string())?;
    let list_sesno = report.written.sesno;

    println!();
    println!("wrote session {list_sesno} to {db}");
    println!("  previous session : {}", report.previous.sesno);
    println!("  new FIXING       : {fixing} (owner ZONE {zone_refno}, appended to its member list)");
    println!("  ROKEYS (inline 19): {keys:?}");
    println!("  ROVALS (hunk 23)  : {vals:?}");
    println!("  appended pages   : {:?}", report.appended_pages);
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

    let report = write_element_session_with(
        db,
        &SessionRequest {
            user: "e3dio",
            comment: "e3d-io write-path gate: GPWL + GROU with reference members",
            clock: None,
        },
        &[ElementWrite::Insert(gpwl_draft), ElementWrite::Insert(grou_draft), ElementWrite::Update(world_draft)],
        &[],
        WriteOptions {
            templates: Some(provider),
        },
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

/// The `NAME` the `--text` session gives the BOX and the `DESC` it gives the
/// ZONE: text outside ASCII, stored as the header's `FORMAT` says
/// (`e3d_io::record::text::encode_as`). The `DESC` opens and closes an
/// escaped run inside plain text on a `FORMAT 0` file.
pub const TEXT_BOX_NAME: &str = "/E3DIO-盒子";
pub const TEXT_ZONE_DESC: &str = "反应堆厂房 -3.320m~±0.00m";
pub const TEXT_COMMENT: &str = "e3d-io 写路径门：中文 NAME / DESC / 会话注释";

/// The fifth session: the BOX gets a Chinese `NAME` (record and `NAME` table
/// key alike), the ZONE a Chinese `DESC`, and the session a Chinese comment —
/// every one through the header's `FORMAT`, so on a `FORMAT 1` file they are
/// UTF-8 as they are and on any other the project code page in `&~ … &`
/// runs. E3D's `Q NAME` / `Q DESC` / `Q SESSION` then say whether it reads
/// them as the characters they are.
fn write_text(db: &str, provider: &mut TemplateProvider, zone_refno: RefNo, box_refno: RefNo) -> Result<(), String> {
    use e3d_io::index::encode_name_as;
    use e3d_io::record::descriptor::DescriptorValue;
    use e3d_io::write::{NamedEdit, NamedEditKind};

    let format = {
        let mut engine = ReadOnlyEngine::open(db).map_err(|e| e.to_string())?;
        if engine.find_by_name(TEXT_BOX_NAME).map_err(|e| e.to_string())?.is_some() {
            return Err(format!("{TEXT_BOX_NAME} is already in use"));
        }
        engine.text_format()
    };
    let box_template = template_of(provider, "BOX")?;
    let zone_template = template_of(provider, "ZONE")?;
    let name_hash = e3d_attlib::db1_hash("NAME");
    let desc_hash = e3d_attlib::db1_hash("DESC");

    let mut box_draft = draft_of(db, box_refno, "the BOX")?;
    box_draft
        .set_attribute(&box_template, name_hash, &DescriptorValue::Text(TEXT_BOX_NAME.into()), format)
        .map_err(|e| e.to_string())?;
    let mut zone_draft = draft_of(db, zone_refno, "the ZONE")?;
    zone_draft
        .set_attribute(&zone_template, desc_hash, &DescriptorValue::Text(TEXT_ZONE_DESC.into()), format)
        .map_err(|e| e.to_string())?;
    let key = encode_name_as(TEXT_BOX_NAME, format).map_err(|e| e.to_string())?;
    let name_edits = [NamedEdit { key: key.clone(), refno: box_refno, kind: NamedEditKind::Insert }];

    let report = write_element_session_with(
        db,
        &SessionRequest { user: "e3dio", comment: TEXT_COMMENT, clock: None },
        &[ElementWrite::Update(box_draft), ElementWrite::Update(zone_draft)],
        &name_edits,
        WriteOptions { templates: Some(provider) },
    )
    .map_err(|e| e.to_string())?;

    println!();
    println!("wrote session {} to {db}", report.written.sesno);
    println!("  previous session : {}", report.previous.sesno);
    println!("  text format      : {format:?}");
    println!("  BOX NAME         : {TEXT_BOX_NAME} (key bytes {:?})", key.text());
    println!("  ZONE DESC        : {TEXT_ZONE_DESC}");
    println!("  comment          : {TEXT_COMMENT}");
    println!("  appended pages   : {:?}", report.appended_pages);
    println!();
    println!("In E3D 3.1 Design, on this database's project:");
    println!("  ={}/{}   Q NAME             -> {TEXT_BOX_NAME}", box_refno.word0, box_refno.word1);
    println!("  ={}/{}   Q DESC             -> {TEXT_ZONE_DESC}", zone_refno.word0, zone_refno.word1);
    println!("  Q SESSIONS                 -> session {} commented {TEXT_COMMENT}", report.written.sesno);
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
