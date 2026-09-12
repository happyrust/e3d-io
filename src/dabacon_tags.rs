//! Project-level aggregation of the DAB engine tags persisted on session pages.
//!
//! A tag identifies the database engine that wrote one retained session. It is
//! deliberately not mapped to an E3D release: a project can retain sessions
//! written by several engine builds, and an untouched database in a newer
//! project tree can still end in an older tag.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::E3dReaderError;
use crate::schema::{project_database_paths, SchemaError};
use crate::session::DbView;

/// One retained session and the DAB engine tag it stores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDabaconTag {
    pub session_number: u32,
    pub session_page: u32,
    pub dabacon_version_tag: Option<String>,
}

/// Latest and historical DAB tags for one primary database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseDabaconTagReport {
    pub database: PathBuf,
    pub dbnum: u32,
    pub db_type_hash: u32,
    pub db_type_name: Option<String>,
    pub latest_session_number: u32,
    pub latest_dabacon_version_tag: Option<String>,
    /// Retained sessions in newest-first chain order.
    pub retained_sessions: Vec<SessionDabaconTag>,
    /// Session count per non-empty historical tag.
    pub historical_tag_session_counts: BTreeMap<String, usize>,
    pub untagged_retained_sessions: usize,
}

/// A descriptor-shaped candidate whose session history could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectDabaconTagIssue {
    pub database: PathBuf,
    pub detail: String,
}

/// Deterministic project-wide DAB tag inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectDabaconTagReport {
    pub root: PathBuf,
    /// Primary database files found from descriptor bytes, including issues.
    pub discovered_databases: usize,
    pub databases: Vec<DatabaseDabaconTagReport>,
    pub issues: Vec<ProjectDabaconTagIssue>,
    /// Number of readable databases whose latest session has each tag.
    pub latest_tag_database_counts: BTreeMap<String, usize>,
    /// Number of retained sessions carrying each tag.
    pub historical_tag_session_counts: BTreeMap<String, usize>,
    /// Number of readable databases whose retained history contains each tag.
    pub historical_tag_database_counts: BTreeMap<String, usize>,
    pub latest_untagged_databases: usize,
    pub historical_untagged_databases: usize,
    pub historical_untagged_sessions: usize,
}

/// Scan every primary database under `project_root` and aggregate the DAB tags
/// on its latest and retained historical sessions.
///
/// Passing a database file instead inventories that one file.
///
/// The report is not a cross-file transaction: each database is frozen at the
/// latest-session pointer observed when that file is opened.
///
/// Candidate discovery reads only descriptor prefixes and ignores files with
/// extensions and secondary extents. A broken database becomes an entry in
/// [`ProjectDabaconTagReport::issues`] so one unreadable file does not hide the
/// rest of the project. Directory traversal failures remain an error because
/// the candidate set itself would be incomplete.
pub fn project_dabacon_tags(
    project_root: impl AsRef<Path>,
) -> Result<ProjectDabaconTagReport, SchemaError> {
    let root = project_root.as_ref();
    let candidates = project_database_paths(root)?;
    let discovered_databases = candidates.len();
    let mut databases = Vec::new();
    let mut issues = Vec::new();
    let mut latest_tag_database_counts = BTreeMap::new();
    let mut historical_tag_session_counts = BTreeMap::new();
    let mut historical_tag_database_counts = BTreeMap::new();
    let mut latest_untagged_databases = 0usize;
    let mut historical_untagged_databases = 0usize;
    let mut historical_untagged_sessions = 0usize;

    for path in candidates {
        let database = match database_dabacon_tags(&path) {
            Ok(database) => database,
            Err(error) => {
                issues.push(ProjectDabaconTagIssue {
                    database: path,
                    detail: error.to_string(),
                });
                continue;
            }
        };

        match &database.latest_dabacon_version_tag {
            Some(tag) => *latest_tag_database_counts.entry(tag.clone()).or_insert(0) += 1,
            None => latest_untagged_databases += 1,
        }
        if database.untagged_retained_sessions != 0 {
            historical_untagged_databases += 1;
        }
        historical_untagged_sessions += database.untagged_retained_sessions;
        for (tag, sessions) in &database.historical_tag_session_counts {
            *historical_tag_session_counts
                .entry(tag.clone())
                .or_insert(0) += *sessions;
            *historical_tag_database_counts
                .entry(tag.clone())
                .or_insert(0) += 1;
        }
        databases.push(database);
    }

    Ok(ProjectDabaconTagReport {
        root: root.to_path_buf(),
        discovered_databases,
        databases,
        issues,
        latest_tag_database_counts,
        historical_tag_session_counts,
        historical_tag_database_counts,
        latest_untagged_databases,
        historical_untagged_databases,
        historical_untagged_sessions,
    })
}

fn database_dabacon_tags(path: &Path) -> Result<DatabaseDabaconTagReport, E3dReaderError> {
    let mut view = DbView::open(path)?;
    let dbnum = view.descriptor().db_mark;
    let db_type_hash = view.descriptor().db_type_hash;
    let db_type_name = view.descriptor().db_type_name();
    let sessions = view.walk_sessions()?;
    let latest = sessions.first().ok_or_else(|| {
        E3dReaderError::Other(format!(
            "{}: session chain contains no session",
            path.display()
        ))
    })?;
    let latest_session_number = latest.sesno;
    let latest_dabacon_version_tag = latest.dabacon_version_tag.clone();
    let mut historical_tag_session_counts = BTreeMap::new();
    let mut untagged_retained_sessions = 0usize;
    let retained_sessions: Vec<SessionDabaconTag> = sessions
        .into_iter()
        .map(|session| {
            match &session.dabacon_version_tag {
                Some(tag) => {
                    *historical_tag_session_counts
                        .entry(tag.clone())
                        .or_insert(0) += 1;
                }
                None => untagged_retained_sessions += 1,
            }
            SessionDabaconTag {
                session_number: session.sesno,
                session_page: session.page.page,
                dabacon_version_tag: session.dabacon_version_tag,
            }
        })
        .collect();

    // Keep this assertion local to construction rather than making callers
    // rediscover whether the per-database count map is internally coherent.
    let tagged_sessions: usize = historical_tag_session_counts.values().sum();
    debug_assert_eq!(
        tagged_sessions + untagged_retained_sessions,
        retained_sessions.len()
    );

    Ok(DatabaseDabaconTagReport {
        database: path.to_path_buf(),
        dbnum,
        db_type_hash,
        db_type_name,
        latest_session_number,
        latest_dabacon_version_tag,
        retained_sessions,
        historical_tag_session_counts,
        untagged_retained_sessions,
    })
}
