//! Warm-cache open and mixed staging measurements on disposable copies.
mod batch;
mod scale;
use std::{error::Error, fs, time::Instant};
use e3d_io::index::IndexCursor;
use e3d_io::session::DbView;
use e3d_io::record::template::TemplateProvider;
use e3d_io::spatial::SpatialPolicy;
use e3d_io::write::Transaction;
use sha2::{Digest, Sha256};

fn main() -> Result<(), Box<dyn Error>> {
    if std::env::args().nth(1).as_deref() != Some("--open-only") {
        return scale::run();
    }
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--open-only") {
        return Err("usage: write_scale_probe --open-only DB [--runs 3]".into());
    }
    let source = args.next().ok_or("missing DB")?;
    let mut runs = 3usize;
    while let Some(arg) = args.next() {
        if arg != "--runs" { return Err(format!("unknown argument {arg}").into()); }
        runs = args.next().ok_or("missing runs")?.parse()?;
    }
    if runs < 3 { return Err("at least 3 runs required".into()); }
    let before = fs::read(&source)?;
    let hash = |bytes: &[u8]| format!("{:x}", Sha256::digest(bytes));
    let source_hash = hash(&before);
    let dir = tempfile::tempdir()?;
    let copy = dir.path().join("open-probe.db");
    fs::write(&copy, &before)?;
    let mut view = DbView::open(&copy)?;
    let owner = IndexCursor::new(&mut view).walk().next().ok_or("empty index")??.refno;
    drop(view);
    let desvir = concat!(env!("CARGO_MANIFEST_DIR"), "/schema/e3d31/binaries/desvir.dat");
    let mut templates = TemplateProvider::open(desvir, Some(0x000B_0692))?;
    let mut results = Vec::new();
    for _ in 0..runs {
        // Warm the complete copy outside the measured interval.
        let warm = fs::read(&copy)?; std::hint::black_box(&warm);
        let start = Instant::now();
        let mut tx = Transaction::open(&copy, &mut templates)?.with_spatial(SpatialPolicy::Off);
        let open_ms = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        let refno = tx.insert(owner, e3d_attlib::db1_hash("EQUI"), None, |_, _| Ok(()))?;
        let first_allocation_ms = start.elapsed().as_secs_f64() * 1000.;
        results.push(serde_json::json!({"open_ms":open_ms,"first_allocation_ms":first_allocation_ms,"refno":refno.to_string(),"fast":tx.allocation_basis().open_fast_path}));
        drop(tx);
    }
    let mut times: Vec<_> = results.iter().map(|r| r["open_ms"].as_f64().unwrap()).collect();
    times.sort_by(f64::total_cmp);
    let median = if runs.is_multiple_of(2) { (times[runs/2-1]+times[runs/2])/2. } else { times[runs/2] };
    if hash(&fs::read(&source)?) != source_hash || fs::read(&copy)? != before { return Err("source or copy changed".into()); }
    println!("{}", serde_json::to_string_pretty(&serde_json::json!({"source":source,"source_sha256":source_hash,"source_len":before.len(),"runs":results,"open_median_ms":median,"open_gate_pass":median<=1000.,"profile":if cfg!(debug_assertions){"debug"}else{"release"}}))?);
    if median > 1000. { return Err("open median exceeded 1 s".into()); }
    Ok(())
}
