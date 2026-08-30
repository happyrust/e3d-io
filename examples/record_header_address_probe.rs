//! Does an element header state where its member list and explicit stream are?
//!
//! Words 6/7 and 8/9 of a record read as `(page_no, packed)` pairs, the same
//! encoding an index leaf value and a block continuation use. On 13292/122 of
//! `ams5100_0001` — the ROOM_NO UDA definition — words 6/7 read
//! `(60, 0x2001)`, and page 60 byte 4 is where its explicit stream is. There
//! are 24 bytes left on the page the record ends on, too few for a block, and
//! nothing in the record's own bytes points forward.
//!
//! `cargo run --release --example record_header_address_probe -- [dir] [max MB]`

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use e3d_io::index::{IndexCursor, IndexEntry};
use e3d_io::page::cache::PageCache;
use e3d_io::page::PageId;
use e3d_io::record::block::{block_at, BlockKind};
use e3d_io::refno::RefNo;
use e3d_io::session::DbView;

const AMS_DIR: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";

#[derive(Default)]
struct Tally {
    files: usize,
    records: usize,
    /// Records where the two walks name exactly the same blocks.
    agree: usize,
    /// Records the stated addresses reach blocks that adjacency does not.
    stated_finds_more: usize,
    /// Records adjacency reaches blocks the stated addresses do not.
    adjacent_finds_more: usize,
    /// A stated address that holds no block of this element.
    dangling: usize,
    /// A stated address whose block is not the kind the slot implies.
    wrong_kind: usize,
    stated_blocks: usize,
    adjacent_blocks: usize,
    slots_used: BTreeMap<&'static str, usize>,
    examples: Vec<String>,
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
    println!(
        "blocks reached from the header addresses: {}",
        total.stated_blocks
    );
    println!(
        "blocks reached by adjacency:              {}",
        total.adjacent_blocks
    );
    println!("records where the two agree:      {}", total.agree);
    println!(
        "  the header reaches more:        {}",
        total.stated_finds_more
    );
    println!(
        "  adjacency reaches more:         {}",
        total.adjacent_finds_more
    );
    println!(
        "stated addresses holding no block of the element: {}",
        total.dangling
    );
    println!(
        "stated addresses holding the other kind:          {}",
        total.wrong_kind
    );
    println!("header slots in use: {:?}", total.slots_used);
    for line in total.examples.iter().take(20) {
        println!("  {line}");
    }
}

type Found = BTreeSet<(u16, u32, usize)>;

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
        Err(_) => return,
    };
    total.files += 1;

    let ext = view.index_root().ext;
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let pages = view.pages_mut();

    for entry in &entries {
        let page = PageId::new(ext, entry.page_no);
        let at = entry.offset_words as usize * 2;
        let Ok(data) = pages.get(page) else { continue };
        let Some(impl_words) = word(data, at) else {
            continue;
        };
        if impl_words < 11 {
            continue;
        }
        let refno = RefNo::new(word(data, at + 4).unwrap(), word(data, at + 8).unwrap());
        let explicit = slot(data, at + 24);
        let members = slot(data, at + 32);
        if at + impl_words as usize * 4 > data.len() {
            continue;
        }
        total.records += 1;

        let key = match (explicit.is_some(), members.is_some()) {
            (false, false) => "neither",
            (true, false) => "explicit only",
            (false, true) => "members only",
            (true, true) => "both",
        };
        *total.slots_used.entry(key).or_default() += 1;

        let mut stated = Found::new();
        for (kind, start) in [
            (BlockKind::Explicit, explicit),
            (BlockKind::Members, members),
        ] {
            let Some(start) = start else { continue };
            follow(
                pages,
                page.ext,
                start,
                refno,
                kind,
                &mut stated,
                total,
                &name,
            );
        }

        let mut adjacent = Found::new();
        let mut cursor = (page, at + impl_words as usize * 4);
        loop {
            let Ok(data) = pages.get(cursor.0) else { break };
            let Some(block) = block_at(data, cursor.1, refno) else {
                break;
            };
            if cursor.1 + block.total_bytes > data.len() {
                break;
            }
            adjacent.insert((cursor.0.ext, cursor.0.page, cursor.1));
            match block.continues_at {
                Some(next) => {
                    cursor = (PageId::new(cursor.0.ext, next.page_no), next.byte_offset());
                    if adjacent.contains(&(cursor.0.ext, cursor.0.page, cursor.1)) {
                        break;
                    }
                }
                None => cursor.1 += block.total_bytes,
            }
        }

        total.stated_blocks += stated.len();
        total.adjacent_blocks += adjacent.len();
        if stated == adjacent {
            total.agree += 1;
        } else {
            if !stated.is_subset(&adjacent) {
                total.stated_finds_more += 1;
            }
            if !adjacent.is_subset(&stated) {
                total.adjacent_finds_more += 1;
                if total.examples.len() < 20 {
                    total.examples.push(format!(
                        "{name} {refno}: adjacency has {:?}, the header has {:?}",
                        adjacent, stated
                    ));
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn follow(
    pages: &mut PageCache,
    ext: u16,
    start: (u32, usize),
    refno: RefNo,
    kind: BlockKind,
    found: &mut Found,
    total: &mut Tally,
    name: &str,
) {
    let mut at = (PageId::new(ext, start.0), start.1);
    loop {
        if !found.insert((at.0.ext, at.0.page, at.1)) {
            return;
        }
        let Ok(data) = pages.get(at.0) else {
            total.dangling += 1;
            return;
        };
        let Some(block) = block_at(data, at.1, refno) else {
            total.dangling += 1;
            if total.examples.len() < 20 {
                total.examples.push(format!(
                    "{name} {refno}: {kind:?} slot points at {} byte {}, no block there",
                    at.0, at.1
                ));
            }
            return;
        };
        if block.kind != kind {
            total.wrong_kind += 1;
        }
        let Some(next) = block.continues_at else {
            return;
        };
        at = (PageId::new(ext, next.page_no), next.byte_offset());
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
