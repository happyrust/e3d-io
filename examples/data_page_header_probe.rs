//! What is in the header of a page that element data lives on?
//!
//! core.dll refuses to read a data page whose second word is not 7618377
//! (`sub_5AFB660`), the exact check it applies to index pages with 13387743
//! (`sub_5AEE4E0`) — and `record::read_record` applies no such check. Before
//! it grows one, this probe asks the corpus three questions:
//!
//! 1. What `(word0, word1)` pairs do page headers hold, over every readable
//!    page of every file?
//! 2. For every page a live record's parts touch — the page its index entry
//!    names, and every page its block chains continue onto — is the header
//!    always `(5, 7618377)`?
//! 3. Which session-page words vary across the corpus? (The data-table root
//!    core.dll takes from its in-memory control block at +20/+24 has no known
//!    file-side field; a word that varies per file is a candidate.)
//!
//! `cargo run --release --example data_page_header_probe -- [dir] [max MB]`

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use e3d_io::index::IndexCursor;
use e3d_io::page::cache::PageCache;
use e3d_io::page::PageId;
use e3d_io::record::block::block_at;
use e3d_io::refno::RefNo;
use e3d_io::session::DbView;

const AMS_DIR: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";

const PAGE_KIND_TABLE: u32 = 5;
const PAGE_KIND_SESSION: u32 = 3;
const TABLE_INDEX: u32 = 13_387_743;
const TABLE_DATA: u32 = 7_618_377;

/// How many words of a session page to census.
const SESSION_WORDS: usize = 16;

#[derive(Default)]
struct Tally {
    files: usize,
    pages: u64,
    /// Every page's `(word0, word1)`, word1 bucketed to keep the map small.
    census: BTreeMap<(u32, Word1), u64>,
    /// Files that hold at least one page of the given class.
    census_files: BTreeMap<(u32, Word1), BTreeSet<String>>,
    unreadable_pages: u64,

    records: u64,
    /// Distinct pages holding any part of a live record, per header pair.
    part_pages: BTreeMap<(u32, Word1), u64>,
    /// Part pages whose header is not `(5, TABLE_DATA)`.
    violations: Vec<String>,
    /// `word2` of every distinct data page a live record touches.
    data_word2: BTreeMap<u32, u64>,
    /// Data pages by census vs data pages the live tree reaches.
    data_pages_total: u64,
    data_pages_live: u64,
    chain_dangling: u64,

    /// word1 values on `(5, Other)` pages — candidate further table ids.
    other_table_ids: BTreeMap<u32, u64>,
    /// On live element pages (word0 = 7): the two halves of word1.
    element_w1_hi: BTreeMap<u32, u64>,
    element_w1_lo_min: u32,
    element_w1_lo_max: u32,
    /// The smallest byte offset any index entry names, per file — how much
    /// page header an element page keeps before its first record.
    min_entry_offset: BTreeMap<u32, u64>,

    /// Distinct values seen in each of the first words of session pages.
    session_slots: Vec<BTreeSet<u32>>,
    session_pages: u64,
}

/// `word1` carries a table id on table pages and arbitrary bits elsewhere;
/// bucket the arbitrary ones so the census stays readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Word1 {
    Index,
    Data,
    AllOnes,
    Zero,
    Other,
}

impl Word1 {
    fn of(word: u32) -> Self {
        match word {
            TABLE_INDEX => Self::Index,
            TABLE_DATA => Self::Data,
            u32::MAX => Self::AllOnes,
            0 => Self::Zero,
            _ => Self::Other,
        }
    }
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| AMS_DIR.to_string());
    let max_mb: u64 = std::env::args()
        .nth(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(25);

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
    total.session_slots = vec![BTreeSet::new(); SESSION_WORDS];
    total.element_w1_lo_min = u32::MAX;
    for path in &files {
        probe(path, &mut total);
    }

    println!(
        "files={} pages={} unreadable={}",
        total.files, total.pages, total.unreadable_pages
    );
    println!("\npage header census (word0, word1-bucket) -> pages / files:");
    for (key, pages) in &total.census {
        let files = total.census_files.get(key).map_or(0, BTreeSet::len);
        println!("  {:?} -> {pages} pages in {files} files", key);
    }

    println!(
        "\nlive records: {} (chain dangling: {})",
        total.records, total.chain_dangling
    );
    println!("pages holding live record parts, by header:");
    for (key, pages) in &total.part_pages {
        println!("  {:?} -> {pages} pages", key);
    }
    println!(
        "(5, 7618377) pages: {} in census, {} referenced by the live tree",
        total.data_pages_total, total.data_pages_live,
    );
    println!(
        "\n(5, Other) word1 values — further table ids? ({} distinct):",
        total.other_table_ids.len()
    );
    for (id, pages) in total.other_table_ids.iter().take(30) {
        println!("  {id} ({id:#x}) -> {pages} pages");
    }

    println!("\nlive element pages (word0 = 7):");
    println!("  word1 high half tally: {:?}", total.element_w1_hi);
    println!(
        "  word1 low half range: {}..={}",
        total.element_w1_lo_min, total.element_w1_lo_max
    );
    let mut w2: Vec<(u32, u64)> = total.data_word2.into_iter().collect();
    w2.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    println!(
        "  word2: {} distinct, top: {:?}",
        w2.len(),
        &w2[..w2.len().min(8)]
    );
    let offsets: Vec<(u32, u64)> = total
        .min_entry_offset
        .iter()
        .take(8)
        .map(|(k, v)| (*k, *v))
        .collect();
    println!("  smallest record byte offsets on a page: {offsets:?}");

    println!("\nviolations (live record part on a non-(5,DATA) page):");
    if total.violations.is_empty() {
        println!("  none");
    }
    for line in total.violations.iter().take(40) {
        println!("  {line}");
    }

    println!(
        "\nsession pages: {} — distinct values per word (first {SESSION_WORDS} words):",
        total.session_pages
    );
    for (slot, values) in total.session_slots.iter().enumerate() {
        let shown: Vec<String> = values.iter().take(6).map(|v| format!("{v:#x}")).collect();
        println!(
            "  word{slot:2}: {:5} distinct  e.g. [{}]{}",
            values.len(),
            shown.join(", "),
            if values.len() > 6 { " …" } else { "" }
        );
    }
}

fn probe(path: &Path, total: &mut Tally) {
    let Ok(mut view) = DbView::open(path) else {
        return;
    };
    let entries: Vec<_> = match IndexCursor::new(&mut view)
        .walk()
        .map(|record| record.map(|record| record.entry()))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(entries) => entries,
        Err(_) => return,
    };
    total.files += 1;

    let ext = view.index_root().ext;
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let pages = view.pages_mut();

    // 1. Whole-file census, page 0 (the file header) excluded.
    let count = pages.physical_page_count();
    let mut header_of: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    for page_no in 1..count {
        let Ok(data) = pages.get(PageId::new(ext, page_no)) else {
            total.unreadable_pages += 1;
            continue;
        };
        let (Some(w0), Some(w1)) = (word(data, 0), word(data, 4)) else {
            total.unreadable_pages += 1;
            continue;
        };
        total.pages += 1;
        header_of.insert(page_no, (w0, w1));
        let key = (w0, Word1::of(w1));
        *total.census.entry(key).or_default() += 1;
        total
            .census_files
            .entry(key)
            .or_default()
            .insert(name.clone());
        if (w0, w1) == (PAGE_KIND_TABLE, TABLE_DATA) {
            total.data_pages_total += 1;
        }
        if w0 == PAGE_KIND_TABLE && Word1::of(w1) == Word1::Other {
            *total.other_table_ids.entry(w1).or_default() += 1;
        }
        if w0 == PAGE_KIND_SESSION {
            total.session_pages += 1;
            for slot in 0..SESSION_WORDS {
                if let Some(value) = word(data, slot * 4) {
                    total.session_slots[slot].insert(value);
                }
            }
        }
    }

    // 2. Every page a live record's parts touch.
    let mut part_pages: BTreeSet<u32> = BTreeSet::new();
    for entry in &entries {
        total.records += 1;
        part_pages.insert(entry.page_no);
        let page = PageId::new(ext, entry.page_no);
        let at = entry.offset_words as usize * 2;
        *total.min_entry_offset.entry(at as u32).or_default() += 1;
        let Ok(data) = pages.get(page) else { continue };
        if word(data, at).is_none_or(|words| words < 11) {
            continue;
        }
        let refno = RefNo::new(
            word(data, at + 4).unwrap_or(0),
            word(data, at + 8).unwrap_or(0),
        );
        let explicit = slot(data, at + 24);
        let members = slot(data, at + 32);
        for start in [explicit, members].into_iter().flatten() {
            follow(pages, ext, start, refno, &mut part_pages, total);
        }
    }

    for page_no in &part_pages {
        let Some(&(w0, w1)) = header_of.get(page_no) else {
            total.unreadable_pages += 1;
            continue;
        };
        *total.part_pages.entry((w0, Word1::of(w1))).or_default() += 1;
        if (w0, w1) == (PAGE_KIND_TABLE, TABLE_DATA) {
            total.data_pages_live += 1;
        }
        if w0 == 7 {
            *total.element_w1_hi.entry(w1 >> 16).or_default() += 1;
            let lo = w1 & 0xFFFF;
            total.element_w1_lo_min = total.element_w1_lo_min.min(lo);
            total.element_w1_lo_max = total.element_w1_lo_max.max(lo);
            if let Ok(data) = pages.get(PageId::new(ext, *page_no)) {
                if let Some(w2) = word(data, 8) {
                    *total.data_word2.entry(w2).or_default() += 1;
                }
            }
        } else if total.violations.len() < 200 {
            total
                .violations
                .push(format!("{name} page {page_no}: header ({w0}, {w1})"));
        }
    }
}

/// Walk one block chain, collecting every page it lands on.
fn follow(
    pages: &mut PageCache,
    ext: u16,
    start: (u32, usize),
    refno: RefNo,
    found: &mut BTreeSet<u32>,
    total: &mut Tally,
) {
    let mut at = (start.0, start.1);
    let mut visited: BTreeSet<(u32, usize)> = BTreeSet::new();
    loop {
        if !visited.insert(at) {
            return;
        }
        found.insert(at.0);
        let Ok(data) = pages.get(PageId::new(ext, at.0)) else {
            total.chain_dangling += 1;
            return;
        };
        let Some(block) = block_at(data, at.1, refno) else {
            total.chain_dangling += 1;
            return;
        };
        match block.continues_at {
            Some(next) => at = (next.page_no, next.byte_offset()),
            None => return,
        }
    }
}

fn slot(data: &[u8], at: usize) -> Option<(u32, usize)> {
    let page_no = word(data, at)?;
    let packed = word(data, at + 4)?;
    (page_no != 0 || packed != 0).then_some((page_no, (packed >> 12) as usize * 2))
}

fn word(page: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(page.get(at..at + 4)?.try_into().ok()?))
}
