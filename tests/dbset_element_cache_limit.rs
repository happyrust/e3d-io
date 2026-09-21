//! `DbSet::with_element_cache_limit`: the pool-level element cache with an entry cap.
//!
//! Its own test binary on purpose: `read_stats` is process-global and
//! `dbset_generations.rs` asserts exact open counts after a `reset()`, so a
//! second test opening databases in that process would skew those numbers.
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use e3d_io::db_element::{DbFilePin, DbFileResolver, DbSet};
use e3d_io::RefNo;

struct Pins {
    pins: HashMap<u32, DbFilePin>,
    resolves: Arc<AtomicUsize>,
}
impl DbFileResolver for Pins {
    fn resolve(&self, dbno: u32) -> Option<DbFilePin> {
        self.resolves.fetch_add(1, Ordering::SeqCst);
        self.pins.get(&dbno).cloned()
    }
}

/// One synthetic DESI file holding a single element (same layout as `dbset_generations.rs`).
fn fixture(root: &Path, name: &str, dbno: u32, session: u32, noun: &str) -> (DbFilePin, RefNo) {
    let refno = RefNo::new(dbno | (1 << 13), 42);
    let mut pages = vec![vec![0u8; 256]; 4];
    fn put(page: &mut [u8], at: usize, value: u32) {
        page[at..at + 4].copy_from_slice(&value.to_be_bytes());
    }
    for (at, value) in [
        (0x08, dbno),
        (0x20, e3d_attlib::db1_hash("DESI")),
        (0x28, 1),
        (0x2c, 1),
        (0x30, 1),
        (0x34, 64),
        (0x38, refno.word0),
    ] {
        put(&mut pages[0], at, value);
    }
    for (at, value) in [(0, 3), (4, u32::MAX), (0x0c, session), (0x1c, 2), (0x20, 1)] {
        put(&mut pages[1], at, value);
    }
    for (i, value) in [
        5,
        13_387_743,
        0,
        2,
        2,
        0,
        53,
        refno.word0,
        refno.word1,
        3,
        (20 << 12) | 1,
    ]
    .into_iter()
    .enumerate()
    {
        put(&mut pages[2], i * 4, value);
    }
    put(&mut pages[3], 0, 7);
    for (i, value) in [
        11,
        refno.word0,
        refno.word1,
        e3d_attlib::db1_hash(noun),
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ]
    .into_iter()
    .enumerate()
    {
        put(&mut pages[3], 40 + i * 4, value);
    }
    let file = root.join(name);
    std::fs::write(&file, pages.concat()).unwrap();
    (
        DbFilePin {
            file,
            template: e3d_io::bundled_schema_root().join("e3d31/binaries/desvir.dat"),
            db_type: Some("DESI".into()),
            sesno: None,
        },
        refno,
    )
}

/// `with_element_cache_limit`：池级元素缓存装满就整张清（全池、跨库合计），再装下一个；
/// `0` = 不限，与 `with_element_cache` 同一形态；`fork_without` 带过去的缓存条目按现状重数。
#[test]
fn the_element_cache_limit_clears_the_whole_pool_when_full_and_zero_means_unbounded() {
    let root = tempfile::tempdir().unwrap();
    let (first, target) = fixture(root.path(), "first", 1000, 1, "SITE");
    let (other, other_ref) = fixture(root.path(), "other", 2000, 7, "EQUI");
    let resolver = || {
        Box::new(Pins {
            pins: HashMap::from([(1000, first.clone()), (2000, other.clone())]),
            resolves: Arc::new(AtomicUsize::new(0)),
        })
    };
    let attlib = || Arc::new(e3d_attlib::AttlibData::default());

    let capped = Arc::new(DbSet::with_resolver(attlib(), resolver()).with_element_cache_limit(1));
    assert_eq!(capped.element_cache_limit(), 1);
    assert_eq!(capped.element(target).element_type().unwrap(), "SITE");
    assert_eq!(capped.cached_element_count(), 1);
    // 第 2 个元素：先整张清、再装它——池里只剩它。
    assert_eq!(capped.element(other_ref).element_type().unwrap(), "EQUI");
    assert_eq!(capped.cached_element_count(), 1, "满则清，不是 LRU");
    assert_eq!(capped.element(other_ref).element_type().unwrap(), "EQUI");
    assert_eq!(capped.cached_element_count(), 1, "命中不再装、也不再清");
    assert_eq!(capped.element(target).element_type().unwrap(), "SITE");
    assert_eq!(capped.cached_element_count(), 1);

    // 上限 2：两个都装得下，第三次读是命中。
    let two = Arc::new(DbSet::with_resolver(attlib(), resolver()).with_element_cache_limit(2));
    assert_eq!(two.element(target).element_type().unwrap(), "SITE");
    assert_eq!(two.element(other_ref).element_type().unwrap(), "EQUI");
    assert_eq!(two.element(target).element_type().unwrap(), "SITE");
    assert_eq!(two.cached_element_count(), 2);
    // fork 带走 2000 那一库的条目，计数按留下来的重数；再装 1000 的元素回到 2 条、不清。
    let forked = Arc::new(
        two.fork_without(&HashSet::from([1000]), resolver())
            .unwrap(),
    );
    assert_eq!(forked.element_cache_limit(), 2);
    assert_eq!(forked.cached_element_count(), 1);
    assert_eq!(forked.element(target).element_type().unwrap(), "SITE");
    assert_eq!(
        forked.cached_element_count(),
        2,
        "留下来的那条没被当成多出来的"
    );

    let unbounded =
        Arc::new(DbSet::with_resolver(attlib(), resolver()).with_element_cache_limit(0));
    assert_eq!(unbounded.element_cache_limit(), 0);
    assert_eq!(unbounded.element(target).element_type().unwrap(), "SITE");
    assert_eq!(unbounded.element(other_ref).element_type().unwrap(), "EQUI");
    assert_eq!(unbounded.cached_element_count(), 2, "0 = 不限");

    let plain = Arc::new(DbSet::with_resolver(attlib(), resolver()));
    assert_eq!(plain.element_cache_limit(), 0, "没开缓存也报 0");
    assert_eq!(plain.element(target).element_type().unwrap(), "SITE");
    assert_eq!(plain.cached_element_count(), 0);

    // 观测：`element_cache_stats` 查时现算——条数与 `cached_element_count` 同一口径，字节是估算
    // （每条至少一个 `ParsedElement` 的内联大小），上限跟着报；没开缓存全是 0。
    let stats = unbounded.element_cache_stats();
    assert_eq!(
        (stats.parsed_entries, stats.extraction_entries, stats.limit),
        (2, 0, 0),
        "{stats:?}"
    );
    assert!(
        stats.approx_bytes >= 2 * std::mem::size_of::<e3d_io::record::element::ParsedElement>(),
        "{stats:?}"
    );
    let one = capped.element_cache_stats();
    assert_eq!((one.parsed_entries, one.limit), (1, 1), "{one:?}");
    assert!(
        one.approx_bytes < stats.approx_bytes,
        "一条比两条少: {one:?} vs {stats:?}"
    );
    assert_eq!(
        plain.element_cache_stats(),
        e3d_io::ElementCacheStats::default()
    );
}
