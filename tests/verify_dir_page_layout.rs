//! Slice 7 step 2 verification: dump dir-page (page 1) words [0..16] of
//! attlib.dat and cross-reference with the diagnostic ATGTDF page locator
//! to confirm the IDA-derived ATTOPE table layout:
//!
//! ```text
//! dir_page[0] = system_atgtdf_start
//! dir_page[1] = (system block end / count? — TBD)
//! dir_page[2] = system_attr_start
//! dir_page[3] = system_atnain_start
//! dir_page[4] = uda_atgtdf_start
//! dir_page[5] = (uda block end / count? — TBD)
//! dir_page[6] = uda_attr_start
//! dir_page[7] = uda_atnain_start
//! ```

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

mod common;

const ATTLIB_PATH: &str = common::ATTLIB;
const PAGE_SIZE: usize = 2048;

fn r32(buf: &[u8], off: usize) -> u32 {
    u32::from_be_bytes(buf[off..off + 4].try_into().unwrap())
}

#[test]
fn dump_dir_page_words() {
    if common::missing(&[ATTLIB_PATH]) {
        return;
    }

    let mut f = File::open(ATTLIB_PATH).unwrap();
    let mut page0 = vec![0u8; PAGE_SIZE];
    f.seek(SeekFrom::Start(0)).unwrap();
    f.read_exact(&mut page0).unwrap();
    let mut page1 = vec![0u8; PAGE_SIZE];
    f.seek(SeekFrom::Start(PAGE_SIZE as u64)).unwrap();
    f.read_exact(&mut page1).unwrap();

    println!("\n=== attlib.dat page 0 [first 16 words] ===");
    for i in 0..16 {
        let w = r32(&page0, i * 4);
        println!("  word[{:>2}] = 0x{:08X} = {}", i, w, w);
    }

    println!("\n=== attlib.dat page 1 (dir page) [first 16 words] ===");
    for i in 0..16 {
        let w = r32(&page1, i * 4);
        println!("  word[{:>2}] = 0x{:08X} = {}", i, w, w);
    }

    // Hypothesis from ATTOPE (sub_55F4290):
    let dir = |i: usize| r32(&page1, i * 4) as usize;
    let sys_atgtdf = dir(0);
    let sys_block_b = dir(1);
    let sys_attr = dir(2);
    let sys_atnain = dir(3);
    let uda_atgtdf = dir(4);
    let uda_block_b = dir(5);
    let uda_attr = dir(6);
    let uda_atnain = dir(7);

    println!("\n=== Inferred table starts (IDA: ATTOPE @ 0x55F4290) ===");
    println!("  system ATGTDF = dir[0] = {sys_atgtdf}");
    println!("  system ?slot1 = dir[1] = {sys_block_b}");
    println!("  system ATTR   = dir[2] = {sys_attr}");
    println!("  system ATNAIN = dir[3] = {sys_atnain}");
    println!("  uda    ATGTDF = dir[4] = {uda_atgtdf}");
    println!("  uda    ?slot5 = dir[5] = {uda_block_b}");
    println!("  uda    ATTR   = dir[6] = {uda_attr}");
    println!("  uda    ATNAIN = dir[7] = {uda_atnain}");

    // Sanity: ATGTDF start should be one of the diagnostic candidates
    // (page 1740 or page 2). Print a hint.
    println!("\n[hint] find_atgtdf_pages reports ATGTDF-shaped pages at 2");
    println!("       (63-run) and 2167 (93-run) for the pack's attlib.");
}
