use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

mod common;

const ATTLIB_PATH: &str = common::ATTLIB;
const PAGE_SIZE: usize = 2048;
const DELIMITER: u32 = 0xFFFFFFFF;

fn r32(data: &[u8], off: usize) -> u32 {
    u32::from_be_bytes(data[off..off + 4].try_into().unwrap())
}

#[test]
fn scan_attlib_pages() {
    if common::missing(&[ATTLIB_PATH]) {
        return;
    }

    let mut file = File::open(ATTLIB_PATH).unwrap();
    let file_len = file.metadata().unwrap().len();
    let total_pages = file_len as usize / PAGE_SIZE;

    println!("\n=== Attlib.dat Scan ===");
    println!(
        "  File size: {} bytes, {} pages of {} bytes",
        file_len, total_pages, PAGE_SIZE
    );

    let mut buf = vec![0u8; PAGE_SIZE];
    file.seek(SeekFrom::Start(0)).unwrap();
    file.read_exact(&mut buf).unwrap();

    println!("\n  Page 0 header (Fortran chars):");
    let mut header_str = String::new();
    for i in (0..80).step_by(4) {
        let ch = r32(&buf, i);
        if ch >= 32 && ch < 127 {
            header_str.push(ch as u8 as char);
        }
    }
    println!("    '{}'", header_str.trim());

    let mut table_names = Vec::new();
    for page_no in 0..total_pages.min(20) {
        file.seek(SeekFrom::Start(page_no as u64 * PAGE_SIZE as u64))
            .unwrap();
        file.read_exact(&mut buf).unwrap();

        let first_word = r32(&buf, 0);
        let mut has_delim = false;
        for off in (0..PAGE_SIZE - 4).step_by(4) {
            if r32(&buf, off) == DELIMITER {
                has_delim = true;
                break;
            }
        }

        if page_no < 5 || has_delim {
            let mut ascii = String::new();
            for i in (0..40.min(PAGE_SIZE)).step_by(4) {
                let ch = r32(&buf, i);
                if ch >= 32 && ch < 127 {
                    ascii.push(ch as u8 as char);
                } else if ch == 0 {
                    ascii.push('.');
                } else {
                    ascii.push('?');
                }
            }
            println!(
                "  page {:>3}: first=0x{:08X} delim={} ascii='{}'",
                page_no,
                first_word,
                has_delim,
                ascii.trim()
            );
        }
    }

    println!("\n  Looking for noun-hash table (ATNAIN)...");
    for page_no in 0..total_pages {
        file.seek(SeekFrom::Start(page_no as u64 * PAGE_SIZE as u64))
            .unwrap();
        file.read_exact(&mut buf).unwrap();

        let mut atnain_str = String::new();
        for i in (0..24.min(PAGE_SIZE)).step_by(4) {
            let ch = r32(&buf, i);
            if ch >= 32 && ch < 127 {
                atnain_str.push(ch as u8 as char);
            }
        }
        if atnain_str.contains("ATNAIN") || atnain_str.contains("ATAINT") {
            println!("  Found '{}' at page {}", atnain_str.trim(), page_no);
            table_names.push((page_no, atnain_str.trim().to_string()));
        }
    }

    if table_names.is_empty() {
        println!("  No ATNAIN/ATAINT tables found by string scan");
    }
}
