//! R1 identity dump: descend one database's 7618377 tree (rooted at the session
//! page's claim word, 0x24 — see `data_table_tree_probe`) and show, in raw
//! detail, what it maps and to what. The whole-corpus probe established the
//! tree is refno-keyed and its keys are a strict subset of the live refno
//! index, but every leaf value's first dword reads as page 0 — so the value is
//! not a record pointer. This dump prints the raw values and correlates the key
//! subset against element nouns, ownership and names to say what the subset is.
//!
//! `cargo run --release --example data_table_tree_dump -- [file] [sample N]`

use std::collections::{BTreeMap, BTreeSet};

use e3d_io::engine::{ReadOnlyEngine, ScanTier};
use e3d_io::page::cache::PageCache;
use e3d_io::page::PageId;
use e3d_io::session::DbView;

const DEFAULT_FILE: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";
const PAGE_TREE: u32 = 5;
const HEADER_DWORDS: usize = 7;
const SENTINEL: u32 = 0x8000_0001;

fn main() {
    let file = std::env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_FILE.to_string());
    let sample: usize = std::env::args()
        .nth(2)
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);

    // Live element facts: refno -> (noun_hash, owner, has_name).
    let mut facts: BTreeMap<(u32, u32), (u32, (u32, u32), bool)> = BTreeMap::new();
    let mut live_keys: BTreeSet<(u32, u32)> = BTreeSet::new();
    let mut owners_referenced: BTreeSet<(u32, u32)> = BTreeSet::new();
    {
        let mut engine = ReadOnlyEngine::open(&file).expect("open engine");
        let scan = engine.scan_elements(ScanTier::Named).expect("scan");
        for element in scan {
            let element = element.expect("element");
            let key = (element.refno.word0, element.refno.word1);
            live_keys.insert(key);
            owners_referenced.insert((element.owner.word0, element.owner.word1));
            facts.insert(
                key,
                (
                    element.noun_hash,
                    (element.owner.word0, element.owner.word1),
                    element.name.is_some(),
                ),
            );
        }
    }
    // Elements that are themselves someone's owner AND exist in this DB.
    let owners_in_db: BTreeSet<(u32, u32)> = owners_referenced
        .intersection(&live_keys)
        .copied()
        .collect();

    // Descend the 7618377 tree from the claim pointer.
    let mut view = DbView::open(&file).expect("open view");
    let claim = view.session().claim;
    let ext = claim.ext;
    let page_limit = view.pages_mut().physical_page_count();
    let pages = view.pages_mut();

    let root_data = pages.get(claim).expect("claim page");
    let root_hdr = (word(root_data, 0), word(root_data, 4));
    println!("file: {file}");
    println!(
        "claim (data-tree root) = {claim}  header=({:?},{:?})",
        root_hdr.0, root_hdr.1
    );
    let root_node = decode_relaxed(root_data).expect("root decodes");
    println!(
        "root level={} entries={} key_dwords={} value_dwords={}",
        root_node.level,
        root_node.entries.len(),
        root_node.key_dwords,
        root_node.value_dwords
    );

    let mut entries: Vec<((u32, u32), u32, u32)> = Vec::new();
    walk(pages, ext, claim.page, page_limit, &mut entries);

    println!("\ntotal leaf entries: {}", entries.len());
    println!("\nfirst {sample} entries  [key.w0/key.w1 dbno=?  ->  val0  val1(hex)  noun/name]:");
    for ((k0, k1), v0, v1) in entries.iter().take(sample) {
        let dbno = (k0 & 0x1FFF) | ((k0 >> 13) & 0x3E000);
        let (noun, named) = match facts.get(&(*k0, *k1)) {
            Some((n, _, named)) => (noun_name(*n), if *named { "named" } else { "-" }),
            None => ("<not-in-live-index>".into(), "?"),
        };
        println!("  {k0}/{k1} dbno={dbno}  -> v0={v0} v1=0x{v1:08X} ({v1})  {noun} {named}");
    }

    // Value distributions.
    let mut v0_tally: BTreeMap<u32, u64> = BTreeMap::new();
    let mut v1_tally: BTreeMap<u32, u64> = BTreeMap::new();
    for (_, v0, v1) in &entries {
        *v0_tally.entry(*v0).or_default() += 1;
        *v1_tally.entry(*v1).or_default() += 1;
    }
    println!(
        "\nvalue dword0: {} distinct, min={} max={}",
        v0_tally.len(),
        v0_tally.keys().next().copied().unwrap_or(0),
        v0_tally.keys().next_back().copied().unwrap_or(0)
    );
    print_top("  dword0 top", &v0_tally, 8);
    println!(
        "value dword1: {} distinct, min={} max={}",
        v1_tally.len(),
        v1_tally.keys().next().copied().unwrap_or(0),
        v1_tally.keys().next_back().copied().unwrap_or(0)
    );
    print_top("  dword1 top", &v1_tally, 8);

    // Identity of the key subset.
    let tree_keys: BTreeSet<(u32, u32)> = entries.iter().map(|(k, _, _)| *k).collect();
    println!("\nsubset identity:");
    println!(
        "  tree keys {} ; live index {} ; owners-in-db {}",
        tree_keys.len(),
        live_keys.len(),
        owners_in_db.len()
    );
    let named_count = tree_keys
        .iter()
        .filter(|k| facts.get(*k).map(|f| f.2).unwrap_or(false))
        .count();
    println!("  of tree keys, named: {named_count}");
    println!(
        "  tree == owners-in-db? {}   tree ⊆ owners? {}   owners ⊆ tree? {}",
        tree_keys == owners_in_db,
        tree_keys.is_subset(&owners_in_db),
        owners_in_db.is_subset(&tree_keys)
    );
    let in_owners = tree_keys.intersection(&owners_in_db).count();
    println!(
        "  |tree ∩ owners-in-db| = {in_owners}  (tree-only {}, owners-only {})",
        tree_keys.difference(&owners_in_db).count(),
        owners_in_db.difference(&tree_keys).count()
    );

    // Noun make-up of the subset vs the whole DB.
    let mut subset_nouns: BTreeMap<u32, u64> = BTreeMap::new();
    for k in &tree_keys {
        if let Some((n, _, _)) = facts.get(k) {
            *subset_nouns.entry(*n).or_default() += 1;
        }
    }
    let mut all_nouns: BTreeMap<u32, u64> = BTreeMap::new();
    for (n, _, _) in facts.values() {
        *all_nouns.entry(*n).or_default() += 1;
    }
    println!("\nnoun make-up (subset count / whole-db count) — top by subset:");
    let mut rows: Vec<(u32, u64, u64)> = subset_nouns
        .iter()
        .map(|(n, c)| (*n, *c, all_nouns.get(n).copied().unwrap_or(0)))
        .collect();
    rows.sort_by_key(|(_, c, _)| std::cmp::Reverse(*c));
    for (n, sub, all) in rows.iter().take(20) {
        println!("  {:<10} {sub:>6} / {all:>6}", noun_name(*n));
    }
    let subset_noun_kinds = subset_nouns.len();
    let all_noun_kinds = all_nouns.len();
    println!("  noun kinds: subset {subset_noun_kinds} / whole-db {all_noun_kinds}");

    // Are all elements of the subset-present nouns fully in the subset?
    let fully_covered: Vec<u32> = subset_nouns
        .iter()
        .filter(|(n, c)| all_nouns.get(*n) == Some(*c))
        .map(|(n, _)| *n)
        .collect();
    println!(
        "  nouns whose every live element is in the tree: {} of {subset_noun_kinds}",
        fully_covered.len()
    );

    // Nouns entirely absent from the tree — what the tree deliberately excludes.
    let mut out_nouns: Vec<(u32, u64)> = all_nouns
        .iter()
        .filter(|(n, _)| !subset_nouns.contains_key(*n))
        .map(|(n, c)| (*n, *c))
        .collect();
    out_nouns.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    println!("\nnouns entirely EXCLUDED from the tree (name / live count):");
    for (n, c) in out_nouns.iter().take(30) {
        println!("  {:<10} {c:>6}", noun_name(*n));
    }
}

fn print_top(label: &str, tally: &BTreeMap<u32, u64>, n: usize) {
    let mut v: Vec<(u32, u64)> = tally.iter().map(|(k, c)| (*k, *c)).collect();
    v.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    let shown: Vec<String> = v
        .iter()
        .take(n)
        .map(|(k, c)| format!("{k}(x{c})"))
        .collect();
    println!("{label}: {}", shown.join(", "));
}

fn noun_name(hash: u32) -> String {
    e3d_attlib::db1_dehash(hash).unwrap_or_else(|| format!("0x{hash:08X}"))
}

fn walk(
    pages: &mut PageCache,
    ext: u16,
    root: u32,
    limit: u32,
    out: &mut Vec<((u32, u32), u32, u32)>,
) {
    let mut visited: BTreeSet<u32> = BTreeSet::new();
    let mut stack = vec![root];
    while let Some(page_no) = stack.pop() {
        if !visited.insert(page_no) || visited.len() as u32 > limit {
            continue;
        }
        let Ok(data) = pages.get(PageId::new(ext, page_no)) else {
            continue;
        };
        let Some(node) = decode_relaxed(data) else {
            continue;
        };
        if node.level == 0 {
            for (key, v0, v1) in node.entries {
                if key.0 == SENTINEL && key.1 == SENTINEL {
                    continue;
                }
                out.push((key, v0, v1));
            }
        } else {
            for (_, child, _) in node.entries {
                stack.push(child);
            }
        }
    }
}

struct RelaxedNode {
    level: u32,
    key_dwords: u32,
    value_dwords: u32,
    entries: Vec<((u32, u32), u32, u32)>,
}

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
        entries.push((
            (word(page, key_at)?, word(page, key_at + 4)?),
            word(page, value_at)?,
            word(page, value_at + 4)?,
        ));
    }
    Some(RelaxedNode {
        level,
        key_dwords: key_dwords as u32,
        value_dwords: value_dwords as u32,
        entries,
    })
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
