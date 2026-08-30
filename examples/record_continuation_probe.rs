//! Is a record's tail chained by a stated address rather than by adjacency?
//!
//! A tail block looks like `[flag:u16][words:u16][ref0][ref1][pgno][packed]`
//! followed by payload. The two words after the RefNo read zero on most blocks
//! and the shipped reader treats them as padding; on BRAN 24383/85432 of
//! `ams7999_0001` they read 18010 and 0x2001, which is `(page 18010, byte 4)`
//! in exactly the encoding an index leaf uses — and that is where the rest of
//! that member list is.
//!
//! `cargo run --release --example record_continuation_probe -- [dir] [max MB]`

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use e3d_io::index::{IndexCursor, IndexEntry};
use e3d_io::page::PageId;
use e3d_io::session::DbView;

const AMS_DIR: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";

#[derive(Default)]
struct Tally {
    files: usize,
    records: usize,
    blocks: usize,
    /// Blocks whose two words after the RefNo are not both zero.
    chained: usize,
    /// Chained blocks whose stated address holds a block of the same element.
    chain_lands: usize,
    chain_missed: Vec<String>,
    /// Chained blocks where the block ends exactly at the page boundary.
    chain_at_page_end: usize,
    /// Chained blocks whose stated address is the very next page at byte 4.
    chain_next_page_byte4: usize,
    /// Records whose whole tail is contiguous.
    contiguous: usize,
    blocks_per_record: BTreeMap<usize, usize>,
    packed_flags: BTreeMap<u32, usize>,
    /// Records whose implicit region alone runs past the page it starts on.
    implicit_overflows: usize,
    /// Blocks whose declared length runs past the page they start on.
    block_overflows: usize,
    /// Smallest block, in words.
    min_block_words: usize,
    /// Blocks ending at the page boundary that state no continuation.
    unchained_at_page_end: usize,
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| AMS_DIR.to_string());
    let max_mb: u64 = std::env::args()
        .nth(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(200);

    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("sample directory reads")
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let metadata = entry.metadata().ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let sampled = name.starts_with("ams")
                && name.len() > 5
                && name.as_bytes()[name.len() - 5] == b'_'
                && name[name.len() - 4..].bytes().all(|b| b.is_ascii_digit())
                && name[3..name.len() - 5].bytes().all(|b| b.is_ascii_digit());
            let sized = metadata.len() < max_mb * 1024 * 1024 && metadata.len() > 20 * 1024;
            (metadata.is_file() && sampled && sized).then(|| entry.path())
        })
        .collect();
    files.sort();

    let mut total = Tally::default();
    for path in &files {
        probe(path, &mut total);
    }

    println!("files={} records={}", total.files, total.records);
    println!("tail blocks: {}", total.blocks);
    println!("  with a stated continuation: {}", total.chained);
    println!(
        "    landing on a block of the same element: {}",
        total.chain_lands
    );
    println!(
        "    ending exactly at the page boundary:    {}",
        total.chain_at_page_end
    );
    println!(
        "    stating the next page at byte 4:        {}",
        total.chain_next_page_byte4
    );
    println!(
        "  low 12 bits of the stated packed word: {:?}",
        total.packed_flags
    );
    println!(
        "  ending at the page boundary with no continuation: {}",
        total.unchained_at_page_end
    );
    println!("  smallest block: {} words", total.min_block_words);
    println!(
        "implicit regions running past their page: {}",
        total.implicit_overflows
    );
    println!(
        "blocks running past their page:           {}",
        total.block_overflows
    );
    println!("records with a contiguous tail: {}", total.contiguous);
    println!("blocks per record: {:?}", total.blocks_per_record);
    for miss in total.chain_missed.iter().take(20) {
        println!("  missed: {miss}");
    }
}

fn probe(path: &Path, total: &mut Tally) {
    let Ok(mut view) = DbView::open(path) else {
        return;
    };
    let entries: Vec<IndexEntry> = match IndexCursor::new(&mut view)
        .walk()
        .map(|record| record.map(|record| record.entry()))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("{}: {error}", path.display());
            return;
        }
    };
    total.files += 1;

    let ext = view.index_root().ext;
    let page_bytes = view.pages().page_size() as usize;
    let pages = view.pages_mut();

    for entry in &entries {
        total.records += 1;
        let mut at = (
            PageId::new(ext, entry.page_no),
            entry.offset_words as usize * 2,
        );
        let mut blocks = 0usize;
        let mut contiguous = true;

        let Ok(page) = pages.get(at.0) else { continue };
        let Some(impl_words) = word(page, at.1) else {
            continue;
        };
        let (Some(ref0), Some(ref1)) = (word(page, at.1 + 4), word(page, at.1 + 8)) else {
            continue;
        };
        if !(6..=16 * 1024).contains(&impl_words) {
            continue;
        }
        if at.1 + impl_words as usize * 4 > page_bytes {
            total.implicit_overflows += 1;
        }
        at.1 += impl_words as usize * 4;

        loop {
            let Ok(page) = pages.get(at.0) else { break };
            let Some(header) = word(page, at.1) else {
                break;
            };
            let flag = header >> 16;
            let words = (header & 0xFFFF) as usize;
            if (flag != 1 && flag != 2)
                || words < 5
                || word(page, at.1 + 4) != Some(ref0)
                || word(page, at.1 + 8) != Some(ref1)
            {
                break;
            }
            blocks += 1;
            total.blocks += 1;
            total.min_block_words = if total.min_block_words == 0 {
                words
            } else {
                total.min_block_words.min(words)
            };

            let next_page = word(page, at.1 + 12).unwrap_or(0);
            let next_packed = word(page, at.1 + 16).unwrap_or(0);
            let end = at.1 + words * 4;
            if end > page_bytes {
                total.block_overflows += 1;
            }

            if next_page == 0 && next_packed == 0 {
                if end == page_bytes {
                    total.unchained_at_page_end += 1;
                }
                at.1 = end;
                continue;
            }

            total.chained += 1;
            *total.packed_flags.entry(next_packed & 0xFFF).or_default() += 1;
            if end == page_bytes {
                total.chain_at_page_end += 1;
            }
            let target = PageId::new(at.0.ext, next_page);
            let target_byte = (next_packed >> 12) as usize * 2;
            if next_page == at.0.page + 1 && target_byte == 4 {
                total.chain_next_page_byte4 += 1;
            }

            let lands = pages
                .get(target)
                .ok()
                .and_then(|page| {
                    let header = word(page, target_byte)?;
                    let flag = header >> 16;
                    Some(
                        (flag == 1 || flag == 2)
                            && word(page, target_byte + 4) == Some(ref0)
                            && word(page, target_byte + 8) == Some(ref1),
                    )
                })
                .unwrap_or(false);
            if lands {
                total.chain_lands += 1;
            } else if total.chain_missed.len() < 20 {
                total.chain_missed.push(format!(
                    "{} {ref0}/{ref1} block at {} byte {} states page {next_page} packed {next_packed:#x}",
                    path.file_name().unwrap().to_string_lossy(),
                    at.0,
                    at.1
                ));
            }
            contiguous = false;
            at = (target, target_byte);
        }

        if contiguous {
            total.contiguous += 1;
        }
        *total.blocks_per_record.entry(blocks.min(9)).or_default() += 1;
    }
}

fn word(page: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(page.get(at..at + 4)?.try_into().ok()?))
}
