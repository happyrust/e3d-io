//! Pick real elements to spot-check against E3D TTY `ATTLIST` output.
//!
//! The point of the sample is coverage of the *decode paths*, not of the model:
//! one element per `(noun, layout)` bucket, so that every distinct
//! `(compact|wide) x noun` shape a library actually contains gets one witness.
//!
//! Why layout is the axis: `evidence/20260826-record-layout-census` measured
//! `ams7000_0001` at 41.5% compact records, while every element verified against
//! TTY so far has been wide. A sample drawn without regard to layout would draw
//! almost entirely wide records and leave the compact column — where the
//! descriptor's two offset lists diverge — untested.
//!
//! Run with:
//! `$env:E3D_PICK_DB='<db>'; cargo test --test tty_sample_pick -- --ignored --nocapture`

use std::collections::BTreeMap;
use std::env;
use std::path::Path;

use e3d_attlib::db1_dehash;
use e3d_io::index::{IndexEntry, IndexPageView};
use e3d_io::meta::constants::DATA_PAGE_INDEX;
use e3d_io::page::cache::PageCache;
use e3d_io::page::io::PageReader;
use e3d_io::page::PageHeader;
use e3d_io::record::element::ParsedElement;
use e3d_io::record::read_element_record;
use e3d_io::record::template::WIDE_LAYOUT_FLAG;

const DEFAULT_DB: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7000_0001";
const FORMAT_WORD_INDEX: usize = 10;

fn format_word(parsed: &ParsedElement) -> Option<u32> {
    let start = FORMAT_WORD_INDEX * 4;
    Some(u32::from_be_bytes(
        parsed
            .implicit_data
            .get(start..start + 4)?
            .try_into()
            .ok()?,
    ))
}

fn leaf_entries(cache: &mut PageCache, page_count: u32) -> Vec<IndexEntry> {
    let mut entries = Vec::new();
    for page_no in 0..page_count {
        let Ok(data) = cache.get_page(page_no) else {
            continue;
        };
        let Some(header) = PageHeader::parse(data) else {
            continue;
        };
        if header.type_id != DATA_PAGE_INDEX {
            continue;
        }
        let Ok(view) = IndexPageView::parse(data) else {
            continue;
        };
        if !view.is_leaf() {
            continue;
        }
        entries.extend(
            view.entries
                .iter()
                .filter(|entry| !entry.is_start_marker())
                .copied(),
        );
    }
    entries.sort_by_key(|entry| (entry.page_no, entry.offset_words));
    entries.dedup_by_key(|entry| (entry.page_no, entry.offset_words));
    entries
}

#[test]
#[ignore = "requires an installed E3D 3.1 AMS database"]
fn pick_tty_sample_elements() {
    let db = env::var("E3D_PICK_DB").unwrap_or_else(|_| DEFAULT_DB.into());
    let per_group: usize = env::var("E3D_PICK_PER_GROUP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    if !Path::new(&db).exists() {
        eprintln!("SKIP: {db} not present");
        return;
    }

    let reader = PageReader::open(&db).expect("open database");
    let page_count = reader.physical_page_count();
    let mut cache = PageCache::new(reader, 4096);
    let entries = leaf_entries(&mut cache, page_count);

    // (noun_hash, is_wide) -> chosen refnos
    let mut picked: BTreeMap<(u32, bool), Vec<(u32, u32, i32)>> = BTreeMap::new();

    for entry in &entries {
        let Ok(view) = read_element_record(&mut cache, e3d_io::page::PageId::PRIMARY, entry) else {
            continue;
        };
        let Ok(parsed) = ParsedElement::parse(&view.raw_bytes) else {
            continue;
        };
        // Same self-证明 rule as the census: an index entry that does not land on
        // its own element's header is stale, and its "noun" is header noise.
        if parsed.refno != entry.refno || db1_dehash(parsed.noun_hash).is_none() {
            continue;
        }
        let Some(word) = format_word(&parsed) else {
            continue;
        };
        let bucket = picked
            .entry((parsed.noun_hash, word & WIDE_LAYOUT_FLAG != 0))
            .or_default();
        if bucket.len() < per_group {
            bucket.push((
                parsed.refno.word0,
                parsed.refno.word1,
                parsed.impl_len_words,
            ));
        }
    }

    let db_stem = Path::new(&db)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("db");

    println!("# db\tnoun\tlayout\timplicit_words\trefno");
    let mut rows = 0usize;
    for ((hash, is_wide), refnos) in &picked {
        let noun = db1_dehash(*hash).unwrap_or_else(|| format!("0x{hash:08X}"));
        let layout = if *is_wide { "wide" } else { "compact" };
        for (w0, w1, words) in refnos {
            println!("{db_stem}\t{noun}\t{layout}\t{words}\t{w0}/{w1}");
            rows += 1;
        }
    }
    eprintln!("picked {rows} elements across {} buckets", picked.len());
    assert!(rows > 0, "expected to pick at least one element");
}
