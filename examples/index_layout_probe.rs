//! Decide, from real database files alone, how a dabacon index page is laid out.
//!
//! Four shipped implementations disagree about two things and no existing test
//! can separate them:
//!
//! * **How many entries a page holds** — scan until the first all-zero 16-byte
//!   slot, or derive it from the dword at `0x18`
//!   (`count = (page_dwords - 7 - free) / stride`). core.dll does not clear
//!   slots that leave the live region, so the two disagree exactly when a page
//!   carries stale residue.
//! * **How an entry is stored** — AoS (`refno.hi, refno.lo, page_no, packed`
//!   repeated) or SoA (a run of keys followed by a run of values), which one
//!   IDA read of `sub_5AFFCB0` suggests.
//!
//! The probe reads bytes only. It never calls this crate's index code, so it
//! cannot inherit the reading it is meant to judge. Every number it prints is
//! a structural invariant that a wrong layout breaks: keys ascend inside a
//! page, a leaf's keys fall inside the range its ancestors routed to it, and
//! one refno appears once in the live tree.
//!
//! Run: `cargo run --release --example index_layout_probe -- <db-path>...`

use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const INDEX_NOUN: u32 = 0x00CC_47DF;
const INDEX_PAGE_TYPE: u32 = 5;
const HEADER_DWORDS: usize = 7;
const ENTRY_DWORDS: usize = 4;
const SENTINEL: u32 = 0x8000_0001;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
struct Key {
    hi: u32,
    lo: u32,
}

impl Key {
    fn is_sentinel(self) -> bool {
        self.hi == SENTINEL && self.lo == SENTINEL
    }
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    key: Key,
    page_no: u32,
    packed: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Counting {
    /// Stop at the first 16-byte slot that is entirely zero.
    ScanToZero,
    /// `(page_dwords - 7 - free_dwords) / 4`, with `free_dwords` at `0x18`.
    FreeDwords,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Layout {
    /// `[key0 val0][key1 val1]...`
    Aos,
    /// `[key0 key1 ...][val0 val1 ...]`
    Soa,
}

/// What an internal entry's key says about the subtree it points at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Separator {
    /// Key is the greatest key in the subtree: child covers `(prev, key]`.
    SubtreeMax,
    /// Key is the least key in the subtree: child covers `[key, next)`.
    SubtreeMin,
}

fn be32(page: &[u8], byte_off: usize) -> u32 {
    u32::from_be_bytes(page[byte_off..byte_off + 4].try_into().unwrap())
}

struct Db {
    file: File,
    page_size: usize,
    latest_ses_pgno: u32,
    page_count: u32,
}

impl Db {
    fn open(path: &Path) -> std::io::Result<Self> {
        let mut file = File::open(path)?;
        let len = file.metadata()?.len();
        let mut header = [0u8; 0x40];
        file.read_exact(&mut header)?;

        // 0x34 holds the page size in 4-byte words, not bytes. Reading it as
        // bytes is the defect that trapped 17 of 490 production files.
        let page_size = be32(&header, 0x34) as usize * 4;
        let latest_ses_pgno = be32(&header, 0x28);

        Ok(Self {
            file,
            page_size,
            latest_ses_pgno,
            page_count: (len / page_size.max(1) as u64) as u32,
        })
    }

    fn read_page(&mut self, page_no: u32) -> std::io::Result<Vec<u8>> {
        let mut buf = vec![0u8; self.page_size];
        self.file
            .seek(SeekFrom::Start(page_no as u64 * self.page_size as u64))?;
        self.file.read_exact(&mut buf)?;
        Ok(buf)
    }

    fn page_dwords(&self) -> usize {
        self.page_size / 4
    }
}

fn is_index_page(page: &[u8]) -> bool {
    page.len() >= HEADER_DWORDS * 4 && be32(page, 4) == INDEX_NOUN
}

/// How many entries the page declares under `counting`, clamped to capacity.
fn entry_count(page: &[u8], counting: Counting) -> usize {
    let page_dwords = page.len() / 4;
    let capacity = page_dwords.saturating_sub(HEADER_DWORDS) / ENTRY_DWORDS;
    match counting {
        Counting::FreeDwords => {
            let free = be32(page, 0x18) as usize;
            page_dwords
                .saturating_sub(HEADER_DWORDS)
                .saturating_sub(free)
                / ENTRY_DWORDS
        }
        Counting::ScanToZero => (0..capacity)
            .take_while(|i| {
                let at = HEADER_DWORDS * 4 + i * ENTRY_DWORDS * 4;
                page[at..at + ENTRY_DWORDS * 4].iter().any(|&b| b != 0)
            })
            .count(),
    }
    .min(capacity)
}

fn read_entries(page: &[u8], count: usize, layout: Layout) -> Vec<Entry> {
    let base = HEADER_DWORDS * 4;
    (0..count)
        .map(|i| match layout {
            Layout::Aos => {
                let at = base + i * ENTRY_DWORDS * 4;
                Entry {
                    key: Key {
                        hi: be32(page, at),
                        lo: be32(page, at + 4),
                    },
                    page_no: be32(page, at + 8),
                    packed: be32(page, at + 12),
                }
            }
            Layout::Soa => {
                let key_at = base + i * 8;
                let val_at = base + count * 8 + i * 8;
                Entry {
                    key: Key {
                        hi: be32(page, key_at),
                        lo: be32(page, key_at + 4),
                    },
                    page_no: be32(page, val_at),
                    packed: be32(page, val_at + 4),
                }
            }
        })
        .collect()
}

#[derive(Clone, Copy)]
enum Bound {
    Unbounded,
    Inclusive(Key),
    Exclusive(Key),
}

#[derive(Clone, Copy)]
struct Range {
    lo: Bound,
    hi: Bound,
}

impl Range {
    fn unbounded() -> Self {
        Self {
            lo: Bound::Unbounded,
            hi: Bound::Unbounded,
        }
    }

    fn contains(self, key: Key) -> bool {
        let above_lo = match self.lo {
            Bound::Unbounded => true,
            Bound::Inclusive(bound) => key >= bound,
            Bound::Exclusive(bound) => key > bound,
        };
        let below_hi = match self.hi {
            Bound::Unbounded => true,
            Bound::Inclusive(bound) => key <= bound,
            Bound::Exclusive(bound) => key < bound,
        };
        above_lo && below_hi
    }
}

#[derive(Default, Debug)]
struct Walk {
    pages_read: usize,
    index_pages: usize,
    leaf_entries: usize,
    sentinel_leaf_entries: usize,
    distinct_refnos: usize,
    duplicate_refnos: usize,
    /// Pages whose keys are not strictly ascending. A wrong layout shreds this
    /// first: garbage does not come out sorted.
    unsorted_pages: usize,
    /// Leaf keys outside the range the descent path routed to that leaf. A
    /// point lookup can never reach these; an enumeration that lacks a range
    /// filter reports them as live.
    out_of_range_leaf_entries: usize,
    /// Internal pages holding the same key on more than one child pointer.
    duplicate_child_pointers: usize,
    /// Child pages that could not be read or were not index pages.
    unreadable_child_pages: usize,
    /// Pages whose level did not decrease on descent (cycle guard).
    level_anomalies: usize,
    /// `page_type` values other than 5.
    bad_page_types: usize,
}

fn walk(db: &mut Db, root: u32, counting: Counting, layout: Layout, sep: Separator) -> Walk {
    let mut stats = Walk::default();
    let mut seen_refnos: HashSet<Key> = HashSet::new();
    let mut visited: HashSet<u32> = HashSet::new();
    let mut stack: Vec<(u32, u32, Range)> = vec![(root, u32::MAX, Range::unbounded())];

    while let Some((page_no, level_bound, range)) = stack.pop() {
        if !visited.insert(page_no) {
            continue;
        }
        let Ok(page) = db.read_page(page_no) else {
            stats.unreadable_child_pages += 1;
            continue;
        };
        stats.pages_read += 1;
        if !is_index_page(&page) {
            stats.unreadable_child_pages += 1;
            continue;
        }
        stats.index_pages += 1;
        if be32(&page, 0) != INDEX_PAGE_TYPE {
            stats.bad_page_types += 1;
        }

        let level = be32(&page, 8);
        if level >= level_bound {
            stats.level_anomalies += 1;
            continue;
        }

        let count = entry_count(&page, counting);
        let entries = read_entries(&page, count, layout);

        let mut previous: Option<Key> = None;
        let mut ascending = true;
        for entry in entries.iter().filter(|e| !e.key.is_sentinel()) {
            if previous.is_some_and(|prev| entry.key <= prev) {
                ascending = false;
            }
            previous = Some(entry.key);
        }
        if !ascending {
            stats.unsorted_pages += 1;
        }

        if level == 0 {
            for entry in &entries {
                if entry.key.is_sentinel() {
                    stats.sentinel_leaf_entries += 1;
                    continue;
                }
                stats.leaf_entries += 1;
                if !range.contains(entry.key) {
                    stats.out_of_range_leaf_entries += 1;
                }
                if !seen_refnos.insert(entry.key) {
                    stats.duplicate_refnos += 1;
                }
            }
            continue;
        }

        // Internal page: the sentinel is the left-most child; every other entry
        // bounds the subtree it points at, on whichever side `sep` says.
        let mut child_keys: HashSet<Key> = HashSet::new();
        let normals: Vec<&Entry> = entries
            .iter()
            .filter(|entry| {
                if entry.key.is_sentinel() {
                    return false;
                }
                if !child_keys.insert(entry.key) {
                    stats.duplicate_child_pointers += 1;
                    return false;
                }
                true
            })
            .collect();

        for (index, entry) in normals.iter().enumerate() {
            let child = match sep {
                Separator::SubtreeMax => Range {
                    lo: index
                        .checked_sub(1)
                        .map(|prev| Bound::Exclusive(normals[prev].key))
                        .unwrap_or(range.lo),
                    hi: Bound::Inclusive(entry.key),
                },
                Separator::SubtreeMin => Range {
                    lo: Bound::Inclusive(entry.key),
                    hi: normals
                        .get(index + 1)
                        .map(|next| Bound::Exclusive(next.key))
                        .unwrap_or(range.hi),
                },
            };
            stack.push((entry.page_no, level, child));
        }

        if let Some(sentinel) = entries.iter().find(|e| e.key.is_sentinel()) {
            let hi = match (sep, normals.first()) {
                (Separator::SubtreeMax, Some(first)) => Bound::Exclusive(first.key),
                (Separator::SubtreeMin, Some(first)) => Bound::Exclusive(first.key),
                (_, None) => range.hi,
            };
            stack.push((sentinel.page_no, level, Range { lo: range.lo, hi }));
        }
    }

    stats.distinct_refnos = seen_refnos.len();
    stats
}

/// Does the dword at `0x18` behave like a free-space counter on every index
/// page in the file? If it is really `pfno`, most pages will fail this.
fn survey_header_field(db: &mut Db) -> (usize, usize, usize) {
    let page_dwords = db.page_dwords();
    let usable = page_dwords.saturating_sub(HEADER_DWORDS);
    let (mut index_pages, mut plausible_free, mut exact_multiple) = (0, 0, 0);
    for page_no in 0..db.page_count {
        let Ok(page) = db.read_page(page_no) else {
            continue;
        };
        if !is_index_page(&page) {
            continue;
        }
        index_pages += 1;
        let value = be32(&page, 0x18) as usize;
        if value <= usable {
            plausible_free += 1;
            if (usable - value) % ENTRY_DWORDS == 0 {
                exact_multiple += 1;
            }
        }
    }
    (index_pages, plausible_free, exact_multiple)
}

fn report(label: &str, stats: &Walk) {
    println!(
        "    {label:<36} pages={:<5} leaf_entries={:<7} distinct={:<7} dup={:<6} unsorted_pages={:<4} out_of_range={:<6} dup_children={:<4} level_anom={:<4} bad_type={:<4} unreadable={}",
        stats.pages_read,
        stats.leaf_entries,
        stats.distinct_refnos,
        stats.duplicate_refnos,
        stats.unsorted_pages,
        stats.out_of_range_leaf_entries,
        stats.duplicate_child_pointers,
        stats.level_anomalies,
        stats.bad_page_types,
        stats.unreadable_child_pages,
    );
}

fn probe(path: &Path) -> std::io::Result<()> {
    let mut db = Db::open(path)?;
    let session = db.read_page(db.latest_ses_pgno)?;
    let session_type = be32(&session, 0);
    let sesno = be32(&session, 0x0C);
    let root = be32(&session, 0x1C);

    println!("\n=== {} ===", path.display());
    println!(
        "  page_size={} bytes ({} words)  pages={}  latest_ses_pgno={}  session_page_type={}  sesno={}  index_root={}",
        db.page_size,
        db.page_size / 4,
        db.page_count,
        db.latest_ses_pgno,
        session_type,
        sesno,
        root
    );

    let (index_pages, plausible, exact) = survey_header_field(&mut db);
    println!(
        "  dword@0x18 over {index_pages} index pages: within_free_range={plausible} and_exact_entry_multiple={exact}"
    );

    for layout in [Layout::Aos, Layout::Soa] {
        for counting in [Counting::FreeDwords, Counting::ScanToZero] {
            for sep in [Separator::SubtreeMax, Separator::SubtreeMin] {
                let stats = walk(&mut db, root, counting, layout, sep);
                report(&format!("{layout:?}/{counting:?}/{sep:?}"), &stats);
            }
        }
    }
    Ok(())
}

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: index_layout_probe <db-path>...");
        std::process::exit(2);
    }
    for path in paths {
        if let Err(error) = probe(Path::new(&path)) {
            eprintln!("{path}: {error}");
        }
    }
}
