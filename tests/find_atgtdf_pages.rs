//! Slice 7 step 0 — diagnostic: find the actual ATGTDF page chain in
//! attlib.dat using a structural pattern stronger than the existing
//! `guess_atgtdf_start` heuristic.
//!
//! The IDA reference (sub_55F53B8) expects each record to be (hash, type,
//! kind) where:
//!   - hash ∈ [0x81BF2, 0x171FAD39]      (base-27 PDMS hash range)
//!   - type ∈ [1, 12]                     (EXMAP-recognized DB_Attribute::type)
//!   - kind ∈ [1, 2]                      (1 = scalar; 2 = sized/array)
//!
//! Pages where 5+ consecutive triples match this pattern are very likely
//! ATGTDF; the existing parser fails because it accepts pages where only
//! `word[0]` (hash) lands in range without checking type/kind.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

mod common;

const ATTLIB_PATH: &str = common::ATTLIB;
const PAGE_SIZE: usize = 2048;
const HASH_MIN: u32 = 0x0008_1BF2;
const HASH_MAX: u32 = 0x171F_AD39;

fn r32(buf: &[u8], off: usize) -> u32 {
    u32::from_be_bytes(buf[off..off + 4].try_into().unwrap())
}

#[test]
fn locate_atgtdf_pages() {
    if common::missing(&[ATTLIB_PATH]) {
        return;
    }

    let mut file = File::open(ATTLIB_PATH).expect("open attlib.dat");
    let file_len = file.metadata().unwrap().len();
    let total_pages = (file_len as usize) / PAGE_SIZE;

    println!("\n=== ATGTDF page locator ===");
    println!("attlib.dat: {} bytes, {} pages", file_len, total_pages);

    let mut buf = vec![0u8; PAGE_SIZE];
    let mut candidate_pages: Vec<(usize, u32)> = Vec::new();

    for page in 0..total_pages {
        file.seek(SeekFrom::Start((page * PAGE_SIZE) as u64))
            .unwrap();
        file.read_exact(&mut buf).unwrap();

        // Scan within the page for ATGTDF-shaped triples
        let mut max_run = 0u32;
        let mut run = 0u32;
        let mut i = 0;
        while i + 12 <= PAGE_SIZE {
            let hash = r32(&buf, i);
            let ty = r32(&buf, i + 4);
            let kind = r32(&buf, i + 8);
            let ok = (HASH_MIN..=HASH_MAX).contains(&hash)
                && (1..=12).contains(&ty)
                && (kind == 1 || kind == 2);
            if ok {
                run += 1;
                if run > max_run {
                    max_run = run;
                }
                let step = if kind == 2 {
                    if ty == 4 {
                        // variable: this run will likely break here; just step
                        // 12 bytes and let the run reset on the next mismatch.
                        12
                    } else {
                        16
                    }
                } else {
                    12
                };
                i += step;
            } else {
                run = 0;
                i += 4;
            }
        }
        if max_run >= 5 {
            candidate_pages.push((page, max_run));
        }
    }

    candidate_pages.sort_by(|a, b| b.1.cmp(&a.1));
    println!(
        "\nFound {} candidate pages with max_run >= 5",
        candidate_pages.len()
    );
    for (page, run) in candidate_pages.iter().take(30) {
        println!("  page {:>4}: max_run = {}", page, run);
    }

    // For the top candidate, decode and print the first ~10 records.
    if let Some(&(page, _run)) = candidate_pages.first() {
        file.seek(SeekFrom::Start((page * PAGE_SIZE) as u64))
            .unwrap();
        file.read_exact(&mut buf).unwrap();
        println!("\n=== Top candidate page {} — first records ===", page);
        let mut i = 0usize;
        let mut idx = 0usize;
        while idx < 15 && i + 12 <= PAGE_SIZE {
            let hash = r32(&buf, i);
            if !(HASH_MIN..=HASH_MAX).contains(&hash) {
                break;
            }
            let ty = r32(&buf, i + 4);
            let kind = r32(&buf, i + 8);
            if !((1..=12).contains(&ty) && (kind == 1 || kind == 2)) {
                break;
            }
            let mut extra = String::new();
            let step = if kind == 2 {
                if ty == 4 {
                    if i + 16 <= PAGE_SIZE {
                        let count = r32(&buf, i + 12);
                        extra = format!(" array-count={count}");
                        16 + count.saturating_mul(4) as usize
                    } else {
                        12
                    }
                } else {
                    if i + 16 <= PAGE_SIZE {
                        let size = r32(&buf, i + 12);
                        extra = format!(" size={size}");
                    }
                    16
                }
            } else {
                12
            };
            println!(
                "  rec[{:>2}] @+{:<5} hash=0x{:08X} type={:>2} kind={}{}",
                idx, i, hash, ty, kind, extra
            );
            i += step;
            idx += 1;
        }
    }

    assert!(
        !candidate_pages.is_empty(),
        "should find at least one ATGTDF-shaped page"
    );
}
