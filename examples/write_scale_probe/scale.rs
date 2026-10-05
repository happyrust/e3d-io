//! Mixed batch probe. Fixture setup and read-side verification are not timed.
use std::{collections::HashSet, error::Error, fs, path::Path, time::Instant};
use e3d_attlib::{db1_hash, AttlibData};
use e3d_io::{ReadOnlyEngine, RefNo};
use e3d_io::record::{descriptor::DescriptorValue, element::ParsedElement, template::TemplateProvider};
use e3d_io::session::{DbView, SessionDate, SessionSelector};
use e3d_io::spatial::{SpatialPolicy, aabb::{ChainComposer, FrameComposer, FrameOutcome, WorldFrame}, state::ReadState};
use e3d_io::write::{Transaction, SessionRequest, SessionClock, WriteOptions, ElementWrite, PageFile, write_element_session_with};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const DESVIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schema/e3d31/binaries/desvir.dat");
const ATTLIB: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schema/e3d31/binaries/attlib.dat");

fn request() -> SessionRequest<'static> {
    let date = SessionDate { year: 2026, month: 10, day: 5, hour: 0, minute: 0, second: 0, millisecond: 0 };
    SessionRequest { user: "e3dio", comment: "fixed mixed scale probe", clock: Some(SessionClock::fixed(date, date)) }
}
fn hash(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
fn elapsed(start: Instant) -> f64 { start.elapsed().as_secs_f64() * 1000. }
fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if n.is_multiple_of(2) { (values[n/2-1] + values[n/2]) / 2. } else { values[n/2] }
}

fn aabb(frame: WorldFrame, pos: [f64; 3], xlen: f64) -> [f64; 6] {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for x in [-xlen/2., xlen/2.] {
        for y in [-200., 200.] { for z in [-250., 250.] {
            let p = frame.point([pos[0]+x, pos[1]+y, pos[2]+z]);
            for axis in 0..3 { min[axis] = min[axis].min(p[axis]); max[axis] = max[axis].max(p[axis]); }
        } }
    }
    [min[0], min[1], min[2], max[0], max[1], max[2]]
}

// Native process high-water working set, including setup. Not a per-stage delta.
#[cfg(windows)]
fn peak_rss() -> Result<u64> {
    #[repr(C)]
    struct Counters { cb: u32, faults: u32, peak: usize, working: usize, pool_peak: usize, pool: usize, nonpaged_peak: usize, nonpaged: usize, pagefile: usize, pagefile_peak: usize }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn K32GetProcessMemoryInfo(process: *mut std::ffi::c_void, counters: *mut Counters, size: u32) -> i32;
    }
    let mut counters = Counters { cb: std::mem::size_of::<Counters>() as u32, faults: 0, peak: 0, working: 0, pool_peak: 0, pool: 0, nonpaged_peak: 0, nonpaged: 0, pagefile: 0, pagefile_peak: 0 };
    // SAFETY: the current-process pseudo handle is valid; counters has the native layout and size.
    if unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(counters.peak as u64)
}
#[cfg(not(windows))]
fn peak_rss() -> Result<u64> { Err("peak RSS is currently implemented for Windows only".into()) }

pub fn run() -> Result<()> {
    let mut source = None;
    let mut counts = vec![10_000usize, 100_000];
    let mut runs = 3usize;
    let mut policy = SpatialPolicy::Require;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = args.next().ok_or_else(|| format!("missing value for {arg}"))?;
        match arg.as_str() {
            "--source" => source = Some(value),
            "--n" => counts = value.split(',').map(str::parse).collect::<std::result::Result<_, _>>()?,
            "--runs" => runs = value.parse()?,
            "--spatial" => policy = match value.as_str() { "require" => SpatialPolicy::Require, "off" => SpatialPolicy::Off, _ => return Err("--spatial requires require|off".into()) },
            _ => return Err(format!("unknown argument {arg}").into()),
        }
    }
    let source = source.ok_or("usage: write_scale_probe --source DB [--n 10000,100000] [--runs 3] [--spatial require|off]")?;
    if runs < 3 || counts.is_empty() || counts.contains(&0) { return Err("require runs >=3 and positive sizes".into()); }
    let before = fs::read(&source)?;
    let source_hash = hash(&before);
    let dir = tempfile::tempdir()?;
    let prepared = dir.path().join("prepared.db");
    fs::write(&prepared, &before)?;
    let mut templates = TemplateProvider::open(DESVIR, Some(0x000B_0692))?;
    let mut reader = ReadOnlyEngine::open(&prepared)?;
    let (writes, fixtures) = super::batch::build(&mut reader, &mut templates, 1100)?;
    let owner = match writes.last().ok_or("empty fixture")? { ElementWrite::Update(d) => d.refno, _ => return Err("missing owner update".into()) };
    drop(reader);
    write_element_session_with(&prepared, &request(), &writes, &[], WriteOptions { templates: Some(&mut templates) })?;
    drop(writes);
    let mut reader = ReadOnlyEngine::open(&prepared)?;
    let record = reader.find_element(owner)?.ok_or("missing owner")?;
    let base_members = ParsedElement::parse_as(&record.raw_bytes, reader.text_format())?.members;
    drop(reader);
    let rules = e3d_io::core_rules::RuleSet::confirmed();
    let state = ReadState::open(&prepared, SessionSelector::Latest, AttlibData::parse_file(ATTLIB)?, TemplateProvider::open(DESVIR, Some(0x000B_0692))?)?;
    let frame = match ChainComposer::new(rules).world_frame(&state, owner)? {
        FrameOutcome::Frame(frame) => frame,
        other => return Err(format!("owner world frame unavailable: {other:?}").into()),
    };
    drop(state);
    let mut sizes = Vec::new();
    for n in counts {
        let mut results = Vec::new();
        for run in 0..runs {
            let copy = dir.path().join(format!("mixed-{n}-{run}.db"));
            fs::copy(&prepared, &copy)?;
            let warm = fs::read(&copy)?; std::hint::black_box(&warm); drop(warm);
            eprintln!("PROBE n={n} run={} open", run+1);
            let start = Instant::now();
            let mut tx = Transaction::open(&copy, &mut templates)?.with_spatial(policy);
            let open_ms = elapsed(start);
            let format = tx.text_format();
            let start = Instant::now();
            let mut inserted = Vec::with_capacity(n);
            for i in 0..n {
                let pos = [1000.+(i%1000) as f64, 2000.+(i/1000) as f64, 3000.];
                let r = tx.insert(owner, db1_hash("BOX"), None, |draft, template| {
                    for (name, value) in [("POS", DescriptorValue::RealArray(pos.to_vec())), ("XLEN", DescriptorValue::Real(300.)), ("YLEN", DescriptorValue::Real(400.)), ("ZLEN", DescriptorValue::Real(500.))] {
                        draft.set_attribute(template, db1_hash(name), &value, format)?;
                    }
                    Ok(())
                })?;
                if policy == SpatialPolicy::Require { tx.set_aabb(r, aabb(frame, pos, 300.))?; }
                inserted.push(r);
            }
            for (i, &r) in fixtures[..1000].iter().enumerate() {
                tx.update(r, |draft, template| { draft.set_attribute(template, db1_hash("XLEN"), &DescriptorValue::Real(301.), format)?; Ok(()) })?;
                if policy == SpatialPolicy::Require { tx.set_aabb(r, aabb(frame, [1000.+i as f64, 2000., 3000.], 301.))?; }
            }
            for &r in &fixtures[1000..] { tx.delete(r)?; }
            let build_ms = elapsed(start);
            eprintln!("PROBE n={n} run={} stage build_ms={build_ms:.3}", run+1);
            let start = Instant::now();
            let staged = tx.stage(&request())?;
            let stage_ms = elapsed(start);
            let appended = staged.appended_pages().to_vec();
            let start = Instant::now();
            let report = staged.commit()?;
            let commit_ms = elapsed(start);
            let peak_rss_bytes = peak_rss()?;
            let pages = page_counts(&copy, &appended)?;
            let deleted: HashSet<_> = fixtures[1000..].iter().copied().collect();
            let mut expected: Vec<_> = base_members.iter().copied().filter(|r| !deleted.contains(r)).collect();
            expected.extend_from_slice(&inserted);
            verify(&copy, owner, &expected, &inserted, &fixtures, policy, frame)?;
            let bytes = fs::read(&copy)?;
            results.push(json!({ "open_ms":open_ms,"build_ms":build_ms,"stage_ms":stage_ms,"commit_ms":commit_ms,"peak_rss_bytes":peak_rss_bytes,"pages":pages,"sesno":report.written.sesno,"sha256":hash(&bytes),"len":bytes.len(),"members_verified":expected.len() }));
            eprintln!("PROBE n={n} run={} verified stage_ms={stage_ms:.3} commit_ms={commit_ms:.3}", run+1);
            fs::remove_file(&copy)?;
        }
        let med = |field: &str| median(results.iter().map(|r| r[field].as_f64().expect("numeric timing")).collect());
        let combined = median(results.iter().map(|r| r["stage_ms"].as_f64().unwrap()+r["commit_ms"].as_f64().unwrap()).collect());
        sizes.push(json!({"n":n,"median_ms":{"open":med("open_ms"),"build":med("build_ms"),"stage":med("stage_ms"),"commit":med("commit_ms"),"stage_commit":combined},"runs":results}));
    }
    if hash(&fs::read(&source)?) != source_hash { return Err("source changed".into()); }
    let small = sizes.iter().find(|s| s["n"] == 10_000);
    let large = sizes.iter().find(|s| s["n"] == 100_000);
    let ratio = small.zip(large).map(|(a,b)| b["median_ms"]["stage"].as_f64().unwrap()/a["median_ms"]["stage"].as_f64().unwrap());
    let time_pass = if policy == SpatialPolicy::Require { small.map(|s| s["median_ms"]["stage_commit"].as_f64().unwrap() <= 10_000.) } else { None };
    let page_pass = small.map(|s| s["runs"].as_array().unwrap().iter().all(|r| {
        let p = &r["pages"];
        (p["total"].as_u64().unwrap()-p["spamap"].as_u64().unwrap())*2 <= p["record"].as_u64().unwrap()*3 && p["spamap"].as_u64().unwrap() <= 2*(11_000usize.div_ceil(p["leaf_capacity"].as_u64().unwrap() as usize)) as u64 + p["spamap_height"].as_u64().unwrap()
    }));
    let gates = json!({"stage_100k_over_10k":ratio,"ratio_le_15":ratio.map(|r|r<=15.),"require_10k_le_10s":time_pass,"page_bounds_10k":page_pass});
    println!("{}", serde_json::to_string_pretty(&json!({"source":source,"source_sha256":source_hash,"source_len":before.len(),"policy":format!("{policy:?}"),"profile":if cfg!(debug_assertions){"debug"}else{"release"},"peak_rss_scope":"process high water including fixture and earlier runs","gates":gates,"sizes":sizes}))?);
    if ratio.is_some_and(|r| r>15.) || time_pass == Some(false) || page_pass == Some(false) { return Err("scale performance/page gate failed; measurements retained above".into()); }
    Ok(())
}

fn page_counts(path: &Path, pages: &[u32]) -> Result<Value> {
    let mut file = PageFile::open(path)?;
    let mut counts = [0usize; 7];
    let mut height = 0;
    for &page in pages {
        let bytes = file.read_page(page)?;
        let word = |i: usize| u32::from_be_bytes(bytes[i*4..i*4+4].try_into().expect("word"));
        let category = match word(0) {
            7 => 0,
            3 => 6,
            5 => match word(1) {
                hash if hash == db1_hash("INDEX") => 1,
                hash if hash == db1_hash("CLAIM") => 2,
                hash if hash == db1_hash("TYPEX") => 3,
                e3d_io::index::spamap::SPAMAP_NAME_HASH => 4,
                _ => 5,
            },
            _ => 5,
        };
        counts[category] += 1;
        if category == 4 { height = height.max(word(2)+1); }
    }
    Ok(json!({"record":counts[0],"index":counts[1],"claim":counts[2],"typex":counts[3],"spamap":counts[4],"other":counts[5],"session":counts[6],"total":pages.len(),"spamap_height":height,"leaf_capacity":(file.page_size().words()-7)/11}))
}

#[allow(clippy::too_many_arguments)]
fn verify(path: &Path, owner: RefNo, expected: &[RefNo], inserted: &[RefNo], fixtures: &[RefNo], policy: SpatialPolicy, frame: WorldFrame) -> Result<()> {
    let mut reader = ReadOnlyEngine::open(path)?;
    let record = reader.find_element(owner)?.ok_or("owner missing after commit")?;
    let members = ParsedElement::parse_as(&record.raw_bytes, reader.text_format())?.members;
    if members != expected { return Err("complete owner member order differs".into()); }
    for &r in inserted.iter().chain(&fixtures[..1000]) {
        let record = reader.find_element(r)?.ok_or("BOX missing")?;
        let parsed = ParsedElement::parse_as(&record.raw_bytes, reader.text_format())?;
        if parsed.owner != owner || parsed.refno != r || parsed.noun_hash != db1_hash("BOX") { return Err("BOX identity/owner differs".into()); }
    }
    for &r in &fixtures[1000..] { if reader.find_element(r)?.is_some() { return Err("deleted BOX remains indexed".into()); } }
    let state = ReadState::open(path, reader.session().sesno, AttlibData::parse_file(ATTLIB)?, TemplateProvider::open(DESVIR, Some(0x000B_0692))?)?;
    for &r in &fixtures[..1000] {
        if e3d_io::spatial::SpatialState::attr(&state, r, db1_hash("XLEN"))? != Some(DescriptorValue::Real(301.)) { return Err("updated XLEN differs".into()); }
    }
    if policy == SpatialPolicy::Require {
        let mut view = DbView::open(path)?;
        let e3d_io::index::spamap::SpamapRead::Table { table, .. } = e3d_io::index::spamap::read_spamap(&mut view)? else { return Err("SPAMAP missing".into()); };
        let codec = e3d_io::spatial::codec::SpamapCodec::new(e3d_io::core_rules::RuleSet::confirmed());
        for (r, bounds) in inserted.iter().enumerate().map(|(i,&r)| (r,aabb(frame,[1000.+(i%1000) as f64,2000.+(i/1000) as f64,3000.],300.))).chain(fixtures[..1000].iter().enumerate().map(|(i,&r)| (r,aabb(frame,[1000.+i as f64,2000.,3000.],301.)))) {
            let entries: Vec<_> = table.for_refno(r).collect();
            if entries.len() != 1 { return Err(format!("expected one SPAMAP entry for {r}").into()); }
            let decoded = codec.decode_words(&entries[0].aabb_words()).map_err(|e| format!("unconfirmed decode: {e:?}"))?;
            if decoded != bounds.map(|v| (v*10.).round() as i64) { return Err(format!("SPAMAP bounds differ for {r}").into()); }
        }
        for &r in &fixtures[1000..] { if table.for_refno(r).next().is_some() { return Err("deleted BOX remains in SPAMAP".into()); } }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotated_box_bounds_cover_all_corners() {
        let frame = WorldFrame { origin_mm: [10.,20.,30.], rotation: [[0.,-1.,0.],[1.,0.,0.],[0.,0.,1.]] };
        assert_eq!(aabb(frame, [0.;3], 300.), [-190.,-130.,-220.,210.,170.,280.]);
        assert_eq!(median(vec![4.,1.,2.,3.]), 2.5);
    }
}
