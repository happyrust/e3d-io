//! Deterministic immutable-view and first-open regression, no installed E3D needed.
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Barrier,
};

use e3d_io::db_element::{DbElementError, DbFilePin, DbFileResolver, DbSet};
use e3d_io::{read_stats, RefNo};

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

#[test]
fn concurrent_open_and_dbnum_generation_forks_preserve_frozen_readers() {
    let root = tempfile::tempdir().unwrap();
    let (first, target) = fixture(root.path(), "first", 1000, 1, "SITE");
    let (second, _) = fixture(root.path(), "second", 1000, 2, "ZONE");
    let (other, other_ref) = fixture(root.path(), "other", 2000, 7, "EQUI");
    let resolves = Arc::new(AtomicUsize::new(0));
    let resolver = |pin: DbFilePin| {
        Box::new(Pins {
            pins: HashMap::from([(1000, pin), (2000, other.clone())]),
            resolves: resolves.clone(),
        })
    };
    let original = Arc::new(
        DbSet::with_resolver(
            Arc::new(e3d_attlib::AttlibData::default()),
            resolver(first.clone()),
        )
        .with_element_cache(),
    );
    read_stats::reset();
    let lazy = original.element(target);
    assert_eq!(lazy.db_no(), 1000);
    assert_eq!(read_stats::snapshot().database_opens, 0);
    let barrier = Arc::new(Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let set = original.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                assert_eq!(set.element(target).element_type().unwrap(), "SITE");
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(resolves.load(Ordering::SeqCst), 1);
    assert_eq!(read_stats::snapshot().database_opens, 1);
    let retained = original.element(other_ref).parsed().unwrap();
    let before_fork = read_stats::snapshot().database_opens;
    let changed = HashSet::from([1000]);
    let next = Arc::new(original.fork_without(&changed, resolver(second)).unwrap());
    assert_eq!(next.db_numbers(), vec![2000]);
    assert_eq!(read_stats::snapshot().database_opens, before_fork);
    assert!(Arc::ptr_eq(
        &retained,
        &next.element(other_ref).parsed().unwrap()
    ));
    assert_eq!(next.element(target).element_type().unwrap(), "ZONE");
    assert_eq!(next.pinned_sesno(1000), Some(2));
    assert_eq!(original.element(target).element_type().unwrap(), "SITE");
    assert_eq!(original.pinned_sesno(1000), Some(1));
    assert_eq!(read_stats::snapshot().database_opens, before_fork + 1);
    assert_eq!(next.cached_element_count(), 2);
    let restored = Arc::new(
        next.fork_without(&changed, resolver(first.clone()))
            .unwrap(),
    );
    assert_eq!(restored.element(target).element_type().unwrap(), "SITE");
    assert_eq!(next.element(target).element_type().unwrap(), "ZONE");
    assert!(Arc::ptr_eq(
        &retained,
        &restored.element(other_ref).parsed().unwrap()
    ));
    assert_eq!(read_stats::snapshot().scan_calls, 0);
    assert_eq!(read_stats::snapshot().scanned_elements, 0);

    let mut wrong_type = first.clone();
    wrong_type.db_type = Some("CATA".into());
    let mismatch = DbSet::with_resolver(
        Arc::new(e3d_attlib::AttlibData::default()),
        resolver(wrong_type),
    );
    assert!(matches!(
        mismatch.element_exists(target),
        Err(DbElementError::DbTypeMismatch { .. })
    ));
    let mismatch = DbSet::with_resolver(
        Arc::new(e3d_attlib::AttlibData::default()),
        Box::new(Pins {
            pins: HashMap::from([(1001, first)]),
            resolves,
        }),
    );
    assert!(matches!(
        mismatch.element_exists(RefNo::new(1001 | (1 << 13), 42)),
        Err(DbElementError::DbNumberMismatch {
            expected: 1001,
            actual: 1000,
            ..
        })
    ));
}

#[test]
#[ignore = "requires AMS source databases"]
fn ams_fork_retains_catalogue_extraction_and_reloads_only_design_db() {
    let schema = e3d_io::bundled_schema_root().join("e3d31/binaries");
    let project = Path::new("D:/AVEVA/Projects/E3D3.1/AvevaMarineSample/ams000");
    let design = DbFilePin {
        file: project.join("ams8000_0001"),
        template: schema.join("desvir.dat"),
        db_type: Some("DESI".into()),
        sesno: None,
    };
    let catalogue = DbFilePin {
        file: project.join("ams5052_0001"),
        template: schema.join("catvir.dat"),
        db_type: Some("CATA".into()),
        sesno: None,
    };
    let catalogue_world = e3d_io::ReadOnlyEngine::open(&catalogue.file)
        .unwrap()
        .descriptor()
        .world_refno();
    let resolver = || {
        Box::new(Pins {
            pins: HashMap::from([(8000, design.clone()), (5052, catalogue.clone())]),
            resolves: Arc::new(AtomicUsize::new(0)),
        })
    };
    let attlib = e3d_attlib::AttlibData::parse_file(schema.join("attlib.dat")).unwrap();
    let old = Arc::new(DbSet::with_resolver(Arc::new(attlib), resolver()).with_element_cache());
    read_stats::reset();
    let branch = RefNo::new(24384, 23257);
    let old_branch = old.element(branch).extraction().unwrap();
    let old_catalogue = old.element(catalogue_world).extraction().unwrap();
    let before = read_stats::snapshot();
    let next = Arc::new(
        old.fork_without(&HashSet::from([8000]), resolver())
            .unwrap(),
    );
    assert!(Arc::ptr_eq(
        &old_catalogue,
        &next.element(catalogue_world).extraction().unwrap()
    ));
    assert_eq!(read_stats::snapshot().database_opens, before.database_opens);
    let fresh_branch = next.element(branch).extraction().unwrap();
    assert!(!Arc::ptr_eq(&old_branch, &fresh_branch));
    assert_eq!(old_branch.attributes.len(), fresh_branch.attributes.len());
    assert!(Arc::ptr_eq(
        &old_branch,
        &old.element(branch).extraction().unwrap()
    ));
    let after = read_stats::snapshot();
    assert_eq!(after.database_opens, before.database_opens + 1);
    assert_eq!(after.scan_calls, 0);
    eprintln!(
        "AMS fork: catalogue Arc retained, old branch Arc retained, design opens delta=1, {}",
        after.summary()
    );
}
