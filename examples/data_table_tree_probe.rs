//! Research item R1: what is the `(5, 7618377)` tree, and where is its root
//! stated on disk?
//!
//! P0 (`docs/evidence/2026-08-30-e3d-io-data-page-header.md`) established that
//! every database holds `(5, 7618377)` pages, that word0=5 means "a B+tree
//! page" and word1 is the tree's identity, and that the *refno index* tree
//! (word1 = 13387743) references none of them — so 7618377 is a separate tree
//! whose root the refno index never names. core.dll takes the data-table root
//! from its in-memory control block at +20/+24, eight bytes past the main-index
//! root at +12/+16; on a session page the index root sits at 0x1C/0x20 and the
//! "claim" pointer at 0x24/0x28 — the same "+8 bytes" step. So the leading
//! hypothesis is: the session page's claim word *is* the 7618377-tree root.
//!
//! This probe settles it against the corpus, asking:
//!
//! 1. Which stated on-disk pointer equals the live 7618377 root? Every word of
//!    the latest session page and of the descriptor (page 0) is followed as a
//!    page address and checked against the set of `(5, 7618377)` pages.
//! 2. Structurally, how many 7618377 roots does a file hold (live + dead CoW
//!    copies), and does the pointer from (1) land on one of them?
//! 3. Descending the live root: how tall is the tree, how many nodes/leaves,
//!    and — the identity question — what do its leaf values point at (element
//!    pages? tree pages?), and how do its keys relate to the refno index's
//!    live key set (equal / subset / superset / disjoint)?
//!
//! `cargo run --release --example data_table_tree_probe -- [dir] [max MB]`

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use e3d_io::engine::{ReadOnlyEngine, ScanTier};
use e3d_io::index::IndexCursor;
use e3d_io::page::cache::PageCache;
use e3d_io::page::PageId;
use e3d_io::session::DbView;

const AMS_DIR: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";

const PAGE_TREE: u32 = 5;
const TABLE_DATA: u32 = 7_618_377;

/// Header dwords of a B+tree page, then key/value widths — same layout the
/// refno index uses, read here without the noun check so any tree decodes.
const HEADER_DWORDS: usize = 7;
const SENTINEL: u32 = 0x8000_0001;

/// How many leading words of the session page / descriptor to test as pointers.
const PROBE_WORDS: usize = 24;

#[derive(Default)]
struct Tally {
    files: usize,
    /// Files that hold at least one `(5, 7618377)` page.
    files_with_data_tree: usize,

    /// Per session-page word slot: files where that word, followed as a page
    /// address, lands on a `(5, 7618377)` page.
    session_word_hits: Vec<usize>,
    /// Same for descriptor (page 0) words.
    descriptor_word_hits: Vec<usize>,
    /// Files where the named session field lands on a `(5, 7618377)` page.
    claim_is_data_page: usize,
    end_is_data_page: usize,
    index_root_is_data_page: usize,
    /// Files where the claim pointer is the live root and descends cleanly.
    claim_descends: usize,

    /// Distribution of how many structural roots (a 7618377 page no other
    /// 7618377 internal page names as a child) a file holds.
    structural_roots: BTreeMap<usize, usize>,
    /// Files where the claim page is among the structural roots.
    claim_is_structural_root: usize,

    /// Live-tree shape, summed for an average; and extremes.
    live_trees: usize,
    live_height_sum: u64,
    live_nodes_sum: u64,
    live_leaves_sum: u64,
    live_entries_sum: u64,
    live_height_min: u32,
    live_height_max: u32,

    /// Value semantics: dword0 is expected 0 (not a page pointer); dword1 is
    /// expected to be a session number in `[1, latest_sesno]`.
    value0_nonzero: u64,
    value1_over_latest_sesno: u64,
    value1_max: u32,
    files_value1_reaches_latest: usize,

    /// Noun classification: is the key subset always a union of *complete*
    /// noun-classes (every live element of a noun, or none)?
    files_clean_noun_union: usize,
    /// Per noun: in how many files it is fully-in the tree vs fully-out, and
    /// whether it was ever split (partial) — a partial breaks the clean rule.
    noun_fully_in: BTreeMap<u32, usize>,
    noun_fully_out: BTreeMap<u32, usize>,
    noun_partial: BTreeMap<u32, usize>,

    /// Key-set relationship between the 7618377 tree and the live refno index.
    rel_equal: usize,
    rel_data_subset_of_index: usize,
    rel_index_subset_of_data: usize,
    rel_disjoint: usize,
    rel_overlap: usize,
    /// Summed sizes for a feel of scale.
    data_keys_sum: u64,
    index_keys_sum: u64,
    /// Of 7618377 keys, how many are NOT live refno-index keys (summed), and
    /// how many live index keys are missing from the 7618377 tree (summed).
    data_only_keys_sum: u64,
    index_only_keys_sum: u64,

    errors: Vec<String>,
}

fn noun_name(hash: u32) -> String {
    e3d_attlib::db1_dehash(hash).unwrap_or_else(|| format!("0x{hash:08X}"))
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
    total.session_word_hits = vec![0; PROBE_WORDS];
    total.descriptor_word_hits = vec![0; PROBE_WORDS];
    total.live_height_min = u32::MAX;
    for path in &files {
        probe(path, &mut total);
    }

    report(&total);
}

fn report(total: &Tally) {
    println!(
        "files={} with_data_tree={}",
        total.files, total.files_with_data_tree
    );

    println!("\nnamed session fields that land on a (5,7618377) page:");
    println!(
        "  claim (0x24)      -> {}/{}",
        total.claim_is_data_page, total.files_with_data_tree
    );
    println!(
        "  end (0x14)        -> {}/{}",
        total.end_is_data_page, total.files_with_data_tree
    );
    println!(
        "  index_root (0x1C) -> {}/{}",
        total.index_root_is_data_page, total.files_with_data_tree
    );
    println!(
        "  claim descends as a clean tree -> {}/{}",
        total.claim_descends, total.files_with_data_tree
    );
    println!(
        "  claim is a structural root      -> {}/{}",
        total.claim_is_structural_root, total.files_with_data_tree
    );

    println!("\nsession-page word slots hitting a (5,7618377) page (word: files):");
    for (slot, hits) in total.session_word_hits.iter().enumerate() {
        if *hits > 0 {
            println!("  word{slot:2} (0x{:02X}) -> {hits} files", slot * 4);
        }
    }
    println!("descriptor (page 0) word slots hitting a (5,7618377) page:");
    for (slot, hits) in total.descriptor_word_hits.iter().enumerate() {
        if *hits > 0 {
            println!("  word{slot:2} (0x{:02X}) -> {hits} files", slot * 4);
        }
    }

    println!("\nstructural roots per file (roots: files):");
    for (roots, files) in &total.structural_roots {
        println!("  {roots} -> {files} files");
    }

    if total.live_trees > 0 {
        let n = total.live_trees as u64;
        println!("\nlive 7618377 tree shape over {} files:", total.live_trees);
        println!(
            "  height: min {} / avg {:.2} / max {}",
            total.live_height_min,
            total.live_height_sum as f64 / n as f64,
            total.live_height_max
        );
        println!(
            "  nodes/file avg  : {:.1}",
            total.live_nodes_sum as f64 / n as f64
        );
        println!(
            "  leaves/file avg : {:.1}",
            total.live_leaves_sum as f64 / n as f64
        );
        println!(
            "  entries/file avg: {:.1} (total {})",
            total.live_entries_sum as f64 / n as f64,
            total.live_entries_sum
        );
    }

    println!("\n7618377 leaf value semantics (value = (dword0, dword1)):");
    println!("  entries with dword0 != 0 : {}", total.value0_nonzero);
    println!(
        "  files with dword1 > latest sesno : {}",
        total.value1_over_latest_sesno
    );
    println!(
        "  files where max dword1 == latest sesno : {}/{}",
        total.files_value1_reaches_latest, total.live_trees
    );
    println!("  max dword1 seen anywhere : {}", total.value1_max);

    println!("\n7618377 keys vs live refno-index keys:");
    println!("  equal              : {}", total.rel_equal);
    println!("  7618377 ⊂ index    : {}", total.rel_data_subset_of_index);
    println!("  index ⊂ 7618377    : {}", total.rel_index_subset_of_data);
    println!("  overlap (neither)  : {}", total.rel_overlap);
    println!("  disjoint           : {}", total.rel_disjoint);
    println!(
        "  keys summed: 7618377 {} / index {}",
        total.data_keys_sum, total.index_keys_sum
    );
    println!(
        "  7618377-only keys summed: {} ; index-only keys summed: {}",
        total.data_only_keys_sum, total.index_only_keys_sum
    );

    println!("\nnoun classification (is the subset a union of complete noun-classes?):");
    println!(
        "  files where every noun is fully-in or fully-out (no split): {}/{}",
        total.files_clean_noun_union, total.live_trees
    );
    let partial_nouns: usize = total.noun_partial.len();
    println!("  nouns ever split (partial) across corpus: {partial_nouns}");
    if partial_nouns > 0 {
        for (noun, files) in total.noun_partial.iter().take(20) {
            println!("    {} split in {files} files", noun_name(*noun));
        }
    }
    println!("  nouns always/ever fully-IN the tree (noun: files-in / files-out):");
    let mut in_rows: Vec<(u32, usize, usize)> = total
        .noun_fully_in
        .iter()
        .map(|(n, f)| (*n, *f, total.noun_fully_out.get(n).copied().unwrap_or(0)))
        .collect();
    in_rows.sort_by_key(|(_, fin, _)| std::cmp::Reverse(*fin));
    for (noun, fin, fout) in in_rows.iter().take(40) {
        println!("    {:<10} in {fin:>4} / out {fout:>4}", noun_name(*noun));
    }
    println!("  nouns that are ONLY ever fully-out (never in the tree):");
    let mut out_only: Vec<(u32, usize)> = total
        .noun_fully_out
        .iter()
        .filter(|(n, _)| !total.noun_fully_in.contains_key(*n))
        .map(|(n, f)| (*n, *f))
        .collect();
    out_only.sort_by_key(|(_, f)| std::cmp::Reverse(*f));
    for (noun, files) in out_only.iter().take(40) {
        println!("    {:<10} out {files:>4}", noun_name(*noun));
    }

    if !total.errors.is_empty() {
        println!("\nerrors ({}):", total.errors.len());
        for line in total.errors.iter().take(30) {
            println!("  {line}");
        }
    }
}

fn probe(path: &Path, total: &mut Tally) {
    let Ok(mut view) = DbView::open(path) else {
        return;
    };
    let name = path.file_name().unwrap().to_string_lossy().into_owned();

    // Live refno-index key set, for the identity comparison at the end.
    let index_keys: BTreeSet<(u32, u32)> = match IndexCursor::new(&mut view)
        .walk()
        .map(|record| record.map(|record| (record.refno.word0, record.refno.word1)))
        .collect::<Result<BTreeSet<_>, _>>()
    {
        Ok(keys) => keys,
        Err(_) => return,
    };
    total.files += 1;

    let ext = view.index_root().ext;
    let session = view.session().clone();
    let claim = session.claim;
    let end = session.end;
    let index_root = session.index_root;
    let latest_sesno = session.sesno;
    let pages = view.pages_mut();

    // Enumerate every (5, 7618377) page in the file.
    let count = pages.physical_page_count();
    let mut data_pages: BTreeSet<u32> = BTreeSet::new();
    for page_no in 1..count {
        if let Ok(data) = pages.get(PageId::new(ext, page_no)) {
            if word(data, 0) == Some(PAGE_TREE) && word(data, 4) == Some(TABLE_DATA) {
                data_pages.insert(page_no);
            }
        }
    }
    if data_pages.is_empty() {
        return;
    }
    total.files_with_data_tree += 1;

    // Structural rooting: a 7618377 page that no other 7618377 internal page
    // lists as a child.
    let mut referenced: BTreeSet<u32> = BTreeSet::new();
    for &page_no in &data_pages {
        if let Ok(data) = pages.get(PageId::new(ext, page_no)) {
            if let Some(node) = decode_relaxed(data) {
                if node.level > 0 {
                    for (_, child_page, _) in &node.entries {
                        if data_pages.contains(child_page) {
                            referenced.insert(*child_page);
                        }
                    }
                }
            }
        }
    }
    let structural_roots: Vec<u32> = data_pages.difference(&referenced).copied().collect();
    *total
        .structural_roots
        .entry(structural_roots.len())
        .or_default() += 1;

    // Which stated pointer lands on a data page? Test named fields, then sweep
    // every leading word of the session page and the descriptor.
    let lands_on_data = |pages: &mut PageCache, pgno: u32| -> bool {
        pgno != 0
            && pages
                .get(PageId::new(ext, pgno))
                .ok()
                .map(|d| word(d, 0) == Some(PAGE_TREE) && word(d, 4) == Some(TABLE_DATA))
                .unwrap_or(false)
    };

    if lands_on_data(pages, claim.page) {
        total.claim_is_data_page += 1;
    }
    if lands_on_data(pages, end.page) {
        total.end_is_data_page += 1;
    }
    if lands_on_data(pages, index_root.page) {
        total.index_root_is_data_page += 1;
    }
    if structural_roots.contains(&claim.page) {
        total.claim_is_structural_root += 1;
    }

    if let Ok(session_data) = pages.get(session.page).map(<[u8]>::to_vec) {
        for slot in 0..PROBE_WORDS {
            if let Some(v) = word(&session_data, slot * 4) {
                if lands_on_data(pages, v) {
                    total.session_word_hits[slot] += 1;
                }
            }
        }
    }
    if let Ok(desc_data) = pages.get(PageId::new(ext, 0)).map(<[u8]>::to_vec) {
        for slot in 0..PROBE_WORDS {
            if let Some(v) = word(&desc_data, slot * 4) {
                if lands_on_data(pages, v) {
                    total.descriptor_word_hits[slot] += 1;
                }
            }
        }
    }

    // Descend the live tree from the claim pointer and characterise it.
    if lands_on_data(pages, claim.page) {
        match descend(pages, ext, claim.page, count) {
            Ok(tree) => {
                total.claim_descends += 1;
                total.live_trees += 1;
                total.live_height_sum += tree.height as u64;
                total.live_height_min = total.live_height_min.min(tree.height);
                total.live_height_max = total.live_height_max.max(tree.height);
                total.live_nodes_sum += tree.nodes as u64;
                total.live_leaves_sum += tree.leaves as u64;
                total.live_entries_sum += tree.keys.len() as u64;

                // Value semantics: dword0 == 0 always; dword1 a session number.
                total.value0_nonzero += tree.value0_nonzero;
                total.value1_max = total.value1_max.max(tree.value1_max);
                if tree.value1_max > latest_sesno {
                    total.value1_over_latest_sesno += 1;
                }
                if tree.value1_max == latest_sesno {
                    total.files_value1_reaches_latest += 1;
                }

                let data_keys = &tree.keys;
                let data_only = data_keys.difference(&index_keys).count();
                let index_only = index_keys.difference(data_keys).count();
                total.data_keys_sum += data_keys.len() as u64;
                total.index_keys_sum += index_keys.len() as u64;
                total.data_only_keys_sum += data_only as u64;
                total.index_only_keys_sum += index_only as u64;
                match (data_only, index_only) {
                    (0, 0) => total.rel_equal += 1,
                    (0, _) => total.rel_data_subset_of_index += 1,
                    (_, 0) => total.rel_index_subset_of_data += 1,
                    (_, _) => {
                        if data_keys.is_disjoint(&index_keys) {
                            total.rel_disjoint += 1;
                        } else {
                            total.rel_overlap += 1;
                        }
                    }
                }

                classify_nouns(path, data_keys, total);
            }
            Err(reason) if total.errors.len() < 200 => {
                total
                    .errors
                    .push(format!("{name}: descend failed: {reason}"));
            }
            Err(_) => {}
        }
    }
}

/// Is the key subset a union of *complete* noun-classes? Build refno -> noun
/// over the live index (a header-tier scan), then for every noun tally how many
/// of its live elements are in the tree. A noun is fully-in, fully-out, or
/// (the rule-breaker) partial.
fn classify_nouns(path: &Path, tree_keys: &BTreeSet<(u32, u32)>, total: &mut Tally) {
    let Ok(mut engine) = ReadOnlyEngine::open(path) else {
        return;
    };
    let Ok(scan) = engine.scan_elements(ScanTier::Header) else {
        return;
    };
    // noun -> (total live, in tree)
    let mut per_noun: BTreeMap<u32, (u64, u64)> = BTreeMap::new();
    for element in scan {
        let Ok(element) = element else { continue };
        let key = (element.refno.word0, element.refno.word1);
        let slot = per_noun.entry(element.noun_hash).or_default();
        slot.0 += 1;
        if tree_keys.contains(&key) {
            slot.1 += 1;
        }
    }

    let mut clean = true;
    for (noun, (live, in_tree)) in &per_noun {
        if *in_tree == 0 {
            *total.noun_fully_out.entry(*noun).or_default() += 1;
        } else if in_tree == live {
            *total.noun_fully_in.entry(*noun).or_default() += 1;
        } else {
            *total.noun_partial.entry(*noun).or_default() += 1;
            clean = false;
        }
    }
    if clean {
        total.files_clean_noun_union += 1;
    }
}

struct Tree {
    height: u32,
    nodes: usize,
    leaves: usize,
    keys: BTreeSet<(u32, u32)>,
    /// `(refno, value_dword1)` per leaf entry — value_dword0 is always 0.
    key_values: Vec<((u32, u32), u32)>,
    value0_nonzero: u64,
    value1_max: u32,
}

/// Walk a B+tree from `root` in slot order, without the refno-index noun check.
/// Trees are small (a dozen pages), so a stack walk with cycle/level guards is
/// plenty. Leaf values are captured raw: dword0 (expected 0) and dword1.
fn descend(pages: &mut PageCache, ext: u16, root: u32, page_limit: u32) -> Result<Tree, String> {
    let mut keys: BTreeSet<(u32, u32)> = BTreeSet::new();
    let mut key_values: Vec<((u32, u32), u32)> = Vec::new();
    let mut value0_nonzero = 0u64;
    let mut value1_max = 0u32;
    let mut visited: BTreeSet<u32> = BTreeSet::new();
    let mut nodes = 0usize;
    let mut leaves = 0usize;
    let mut height = 0u32;

    // (page_no, expected_level_or_None)
    let mut stack: Vec<(u32, Option<u32>)> = vec![(root, None)];
    while let Some((page_no, expect_level)) = stack.pop() {
        if !visited.insert(page_no) {
            return Err(format!("cycle at page {page_no}"));
        }
        if visited.len() as u32 > page_limit {
            return Err("walk exceeds file page count".into());
        }
        let data = pages
            .get(PageId::new(ext, page_no))
            .map_err(|e| format!("page {page_no} unreadable: {e}"))?;
        let node = decode_relaxed(data).ok_or_else(|| format!("page {page_no} not a tree node"))?;
        if let Some(expected) = expect_level {
            if node.level != expected {
                return Err(format!(
                    "page {page_no} level {} but parent expected {expected}",
                    node.level
                ));
            }
        }
        nodes += 1;
        height = height.max(node.level + 1);
        if node.level == 0 {
            leaves += 1;
            for (refno, value0, value1) in &node.entries {
                if refno.0 == SENTINEL && refno.1 == SENTINEL {
                    continue;
                }
                keys.insert(*refno);
                key_values.push((*refno, *value1));
                if *value0 != 0 {
                    value0_nonzero += 1;
                }
                value1_max = value1_max.max(*value1);
            }
        } else {
            for (_, child_page, _) in &node.entries {
                stack.push((*child_page, Some(node.level - 1)));
            }
        }
    }

    Ok(Tree {
        height,
        nodes,
        leaves,
        keys,
        key_values,
        value0_nonzero,
        value1_max,
    })
}

struct RelaxedNode {
    level: u32,
    /// `((key.w0, key.w1), value_page_no, value_packed)` per live entry.
    entries: Vec<((u32, u32), u32, u32)>,
}

/// [`e3d_io::index::IndexNode::decode`] without the `noun == 13387743` gate, so
/// the 7618377 tree (and any other) decodes with the exact same free_dwords
/// entry-count rule.
fn decode_relaxed(page: &[u8]) -> Option<RelaxedNode> {
    if page.len() < HEADER_DWORDS * 4 || page.len() % 4 != 0 {
        return None;
    }
    if word(page, 0)? != PAGE_TREE {
        return None;
    }
    let level = word(page, 0x08)?;
    let key_dwords = nonzero(word(page, 0x0C)?, 2) as usize;
    let value_dwords = nonzero(word(page, 0x10)?, 2) as usize;
    let free_dwords = word(page, 0x18)? as usize;
    if key_dwords < 2 || value_dwords < 2 {
        return None;
    }
    let usable = page.len() / 4 - HEADER_DWORDS;
    if free_dwords > usable {
        return None;
    }
    let stride = key_dwords + value_dwords;
    let used = usable - free_dwords;
    if stride == 0 || used % stride != 0 {
        return None;
    }
    let count = used / stride;
    let mut entries = Vec::with_capacity(count);
    for slot in 0..count {
        let key_at = (HEADER_DWORDS + slot * stride) * 4;
        let value_at = key_at + key_dwords * 4;
        let k0 = word(page, key_at)?;
        let k1 = word(page, key_at + 4)?;
        let page_no = word(page, value_at)?;
        let packed = word(page, value_at + 4)?;
        entries.push(((k0, k1), page_no, packed));
    }
    Some(RelaxedNode { level, entries })
}

fn nonzero(declared: u32, fallback: u32) -> u32 {
    if declared == 0 {
        fallback
    } else {
        declared
    }
}

fn word(page: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(page.get(at..at + 4)?.try_into().ok()?))
}
