//! Census of per-record storage layout, keyed off the format word's bit 29.
//!
//! Cross-tabulates `layout x declared implicit word count` for every indexed
//! element in a database, so that a noun showing two implicit lengths in one
//! library can be checked against the compact/wide split rather than guessed
//! at from record length alone.
//!
//! Run with:
//! `cargo test --test record_layout_census -- --ignored --nocapture`

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

/// The format word lives at word 10 of the record, counting from the element
/// header rather than from the first attribute slot.
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
fn census_record_layout_by_format_word() {
    let db = env::var("E3D_CENSUS_DB").unwrap_or_else(|_| DEFAULT_DB.into());
    if !Path::new(&db).exists() {
        eprintln!("SKIP: {db} not present");
        return;
    }

    let reader = PageReader::open(&db).expect("open database");
    let page_count = reader.physical_page_count();
    let mut cache = PageCache::new(reader, 4096);

    let entries = leaf_entries(&mut cache, page_count);
    println!(
        "db={db}\npages={page_count} indexed_entries={}",
        entries.len()
    );

    // noun_hash -> (is_wide, implicit_words) -> count
    let mut census: BTreeMap<u32, BTreeMap<(bool, i32), usize>> = BTreeMap::new();
    let mut decoded = 0usize;
    let mut unreadable = 0usize;
    let mut refno_mismatch = 0usize;
    let mut no_format_word = 0usize;

    for entry in &entries {
        let Ok(view) = read_element_record(&mut cache, e3d_io::page::PageId::PRIMARY, entry) else {
            unreadable += 1;
            continue;
        };
        let Ok(parsed) = ParsedElement::parse(&view.raw_bytes) else {
            unreadable += 1;
            continue;
        };
        // An index entry that does not land on its own element's header is a
        // stale or foreign-bucket pointer; counting it would mix header words
        // into the implicit-length tally.
        if parsed.refno != entry.refno || db1_dehash(parsed.noun_hash).is_none() {
            refno_mismatch += 1;
            continue;
        }
        let Some(word) = format_word(&parsed) else {
            no_format_word += 1;
            continue;
        };
        decoded += 1;
        let is_wide = word & WIDE_LAYOUT_FLAG != 0;
        *census
            .entry(parsed.noun_hash)
            .or_default()
            .entry((is_wide, parsed.impl_len_words))
            .or_default() += 1;
    }

    let wide_total: usize = census
        .values()
        .flat_map(|by_shape| by_shape.iter())
        .filter(|((is_wide, _), _)| *is_wide)
        .map(|(_, count)| *count)
        .sum();
    let compact_total = decoded - wide_total;

    println!(
        "decoded={decoded} unreadable={unreadable} refno_mismatch={refno_mismatch} \
         no_format_word={no_format_word}\n\
         WIDE(bit29=1)={wide_total} COMPACT(bit29=0)={compact_total}"
    );

    // Any noun whose records do not all agree on one (layout, length) shape.
    let mut split_nouns: Vec<(String, u32, &BTreeMap<(bool, i32), usize>)> = census
        .iter()
        .filter(|(_, by_shape)| by_shape.len() > 1)
        .map(|(&hash, by_shape)| {
            (
                db1_dehash(hash).unwrap_or_else(|| format!("0x{hash:08X}")),
                hash,
                by_shape,
            )
        })
        .collect();
    split_nouns.sort_by_key(|(name, _, _)| name.clone());

    println!(
        "\nnouns={} nouns_with_multiple_shapes={}",
        census.len(),
        split_nouns.len()
    );
    for (name, hash, by_shape) in &split_nouns {
        let layouts: Vec<String> = by_shape
            .iter()
            .map(|((is_wide, words), count)| {
                let layout = if *is_wide { "wide" } else { "compact" };
                format!("{layout}/{words}w x{count}")
            })
            .collect();
        let mixed_layout = by_shape.keys().any(|(w, _)| *w) && by_shape.keys().any(|(w, _)| !*w);
        println!(
            "{name:<8} 0x{hash:08X}  {:<8} {}",
            if mixed_layout { "MIXED" } else { "same" },
            layouts.join("  ")
        );
    }

    assert!(decoded > 0, "expected to decode at least one element");
}
