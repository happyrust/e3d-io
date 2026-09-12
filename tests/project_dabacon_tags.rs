use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use e3d_io::project_dabacon_tags;

const WORDS: u32 = 512;
const PAGE_BYTES: usize = WORDS as usize * 4;

#[derive(Clone, Copy)]
struct Session<'a> {
    page: u32,
    sesno: u32,
    previous: Option<u32>,
    tag: Option<&'a str>,
}

fn put(page: &mut [u8], at: usize, value: u32) {
    page[at..at + 4].copy_from_slice(&value.to_be_bytes());
}

fn database(
    root: &Path,
    name: &str,
    dbno: u32,
    db_type: &str,
    sessions: &[Session<'_>],
) -> PathBuf {
    let pages = sessions.iter().map(|session| session.page).max().unwrap() + 1;
    let mut file = vec![0u8; pages as usize * PAGE_BYTES];
    let page0 = &mut file[..PAGE_BYTES];
    put(page0, 0x00, 0);
    put(page0, 0x04, 2);
    put(page0, 0x08, dbno);
    put(page0, 0x20, e3d_attlib::db1_hash(db_type));
    put(page0, 0x28, sessions[0].page);
    put(page0, 0x2C, 1);
    put(page0, 0x30, sessions.last().unwrap().page);
    put(page0, 0x34, WORDS);
    put(page0, 0x38, dbno + 0x2000);

    for session in sessions {
        let start = session.page as usize * PAGE_BYTES;
        let page = &mut file[start..start + PAGE_BYTES];
        put(page, 0x00, 3);
        put(page, 0x04, session.previous.unwrap_or(u32::MAX));
        put(page, 0x08, 1);
        put(page, 0x0C, session.sesno);
        put(page, 0x10, u32::MAX);
        put(page, 0x14, session.page);
        put(page, 0x18, 1);
        put(page, 0x1C, session.page);
        put(page, 0x20, 1);
        put(page, 0x24, session.page);
        put(page, 0x28, 1);
        if let Some(tag) = session.tag {
            assert_eq!(tag.len(), 8);
            page[0x7DC..0x7E4].copy_from_slice(tag.as_bytes());
        }
    }

    let path = root.join(name);
    let mut output = File::create(&path).unwrap();
    output.write_all(&file).unwrap();
    output.flush().unwrap();
    path
}

fn broken_database(root: &Path, name: &str, dbno: u32) {
    let mut file = vec![0u8; 2 * PAGE_BYTES];
    let page0 = &mut file[..PAGE_BYTES];
    put(page0, 0x00, 0);
    put(page0, 0x04, 2);
    put(page0, 0x08, dbno);
    put(page0, 0x20, e3d_attlib::db1_hash("DESI"));
    put(page0, 0x28, 99);
    put(page0, 0x2C, 1);
    put(page0, 0x30, 99);
    put(page0, 0x34, WORDS);
    put(page0, 0x38, dbno + 0x2000);
    std::fs::write(root.join(name), file).unwrap();
}

#[test]
fn project_report_aggregates_latest_and_retained_session_tags() {
    let root = tempfile::tempdir().unwrap();
    let design_path = database(
        root.path(),
        "tst1000_0001",
        1000,
        "DESI",
        &[
            Session {
                page: 4,
                sesno: 3,
                previous: Some(3),
                tag: Some("2_35_23o"),
            },
            Session {
                page: 3,
                sesno: 2,
                previous: Some(2),
                tag: Some("2_28_19_"),
            },
            Session {
                page: 2,
                sesno: 1,
                previous: None,
                tag: None,
            },
        ],
    );
    database(
        root.path(),
        "tst2000_0001",
        2000,
        "CATA",
        &[
            Session {
                page: 3,
                sesno: 4,
                previous: Some(2),
                tag: Some("2_28_19_"),
            },
            Session {
                page: 2,
                sesno: 1,
                previous: None,
                tag: Some("2_27_2_n"),
            },
        ],
    );
    database(
        root.path(),
        "tst2500_0001",
        2500,
        "MISC",
        &[Session {
            page: 2,
            sesno: 1,
            previous: None,
            tag: None,
        }],
    );
    broken_database(root.path(), "tst3000_0001", 3000);
    std::fs::copy(&design_path, root.path().join("ignored.copy")).unwrap();
    let mut secondary = std::fs::read(&design_path).unwrap();
    put(&mut secondary[..PAGE_BYTES], 0x2C, 2);
    std::fs::write(root.path().join("tst1000_0002"), secondary).unwrap();

    let report = project_dabacon_tags(root.path()).expect("aggregate project tags");

    assert_eq!(report.discovered_databases, 4);
    assert_eq!(report.databases.len(), 3);
    assert_eq!(report.issues.len(), 1);
    assert_eq!(report.latest_tag_database_counts["2_35_23o"], 1);
    assert_eq!(report.latest_tag_database_counts["2_28_19_"], 1);
    assert_eq!(report.historical_tag_session_counts["2_35_23o"], 1);
    assert_eq!(report.historical_tag_session_counts["2_28_19_"], 2);
    assert_eq!(report.historical_tag_session_counts["2_27_2_n"], 1);
    assert_eq!(report.historical_tag_database_counts["2_28_19_"], 2);
    assert_eq!(report.latest_untagged_databases, 1);
    assert_eq!(report.historical_untagged_databases, 2);
    assert_eq!(report.historical_untagged_sessions, 2);
    assert_eq!(
        report
            .databases
            .iter()
            .map(|database| {
                database
                    .database
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>(),
        ["tst1000_0001", "tst2000_0001", "tst2500_0001"]
    );
    assert_eq!(
        report.issues[0].database.file_name().unwrap(),
        "tst3000_0001"
    );

    let design = report
        .databases
        .iter()
        .find(|database| database.dbnum == 1000)
        .unwrap();
    assert_eq!(design.db_type_name.as_deref(), Some("DESI"));
    assert_eq!(design.latest_session_number, 3);
    assert_eq!(
        design.latest_dabacon_version_tag.as_deref(),
        Some("2_35_23o")
    );
    assert_eq!(design.retained_sessions.len(), 3);
    assert_eq!(design.retained_sessions[1].session_number, 2);
    assert_eq!(
        design.retained_sessions[1].dabacon_version_tag.as_deref(),
        Some("2_28_19_")
    );
    assert_eq!(design.untagged_retained_sessions, 1);

    let single = project_dabacon_tags(&design_path).expect("aggregate one explicit database");
    assert_eq!(single.discovered_databases, 1);
    assert_eq!(single.databases.len(), 1);
    assert!(single.issues.is_empty());
}

#[test]
fn installed_project_report_covers_every_primary_database() {
    let root = Path::new(r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000");
    if !root.is_dir() {
        eprintln!("skipped: installed project is absent");
        return;
    }

    let report = project_dabacon_tags(root).expect("aggregate installed project tags");
    assert!(report.discovered_databases >= 400, "{report:#?}");
    assert!(report.databases.len() >= 400, "{report:#?}");
    assert!(
        report
            .latest_tag_database_counts
            .keys()
            .any(|tag| tag.starts_with("2_35_")),
        "{:?}",
        report.latest_tag_database_counts
    );
    assert!(
        report
            .historical_tag_session_counts
            .keys()
            .any(|tag| tag.starts_with("2_28_")),
        "{:?}",
        report.historical_tag_session_counts
    );
}
