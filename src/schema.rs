//! Which E3D release a set of schema files came from, and where those files are.
//!
//! Three kinds of input are release-bound and **none of them states its own
//! release**: an installation's `*vir.dat` descriptor files, its `attlib.dat`,
//! and the live catalog export taken beside them. Mixing two releases is
//! therefore silent if those paths are supplied independently. `desvir.dat`
//! from one and `attlib.dat` from another both parse; the descriptors decide
//! the word offsets, so the result is plausible numbers read out of the wrong
//! words rather than an error. `TemplateFile` checks `template_type`, which
//! says `DESI`, not `3.1`.
//!
//! The tree identities do not distinguish 2.1 from 3.1 either: both measured
//! corpora use `(7618377, 13387743)`. IDA shows that the database world's
//! `UPGDNO` is not a product release: `DB_DB::upgdno()` reads it and
//! `DB_UpgradeCtrl::applicableVersion()` compares it with database-migration
//! numbers. The two measured `amssys` files both store `12010401`, and the two
//! `ams8000_0001` files both store `12010104`.
//!
//! The positive product evidence is in the Communication database instead.
//! E3D 3.1's `DB_Communication::updateVersion()` writes
//! `VersionInfo::version()` to the `VERS` attribute of a `COMM` element. Project
//! detection therefore inspects the current indexed `COMM.VERS` values and
//! opens a pack only when one names a measured release. A missing or unfamiliar
//! value is `Unknown`; it is not silently called 2.10.
//!
//! `Saving Meta Data.` is retained as a diagnostic constant only.
//! `DB_DB::setMetaData()` writes `DBMDAT/DBMVAL/DBMEXT` and then uses that text
//! as a session comment; the comment is not a version field and does not
//! participate in selection. Paths, timestamps and file sizes are likewise
//! never evidence.
//!
//! Keeping each release in its own directory under one root is what makes the
//! mix hard to express in the first place:
//!
//! ```text
//! schema/
//!   e3d31/
//!     manifest.json     release identity, magic constants, and the file map
//!     binaries/         desvir.dat, attlib.dat, dicvir.dat, … (optional)
//!     catalog/          noun_layout.json, noun_attr_fields.json
//!     metadata/         compiled parser_metadata.v1.json
//! ```
//!
//! A pack is meant to carry its own binaries — both supported packs hold all
//! 20 `*vir.dat` and `attlib.dat` — so that naming the pack names everything.
//! They are licensed AVEVA installation files; keep the repository within the
//! licensed parties. [`SchemaPack`] still searches because another machine
//! may intentionally point a pack at its own installation.
//!
//! 1. the environment variable the manifest names in `binaries.root_env`,
//! 2. `<pack>/binaries/`,
//! 3. the `binaries.root` the manifest records, if it records one.
//!
//! An environment variable comes first because it is the only one of the three
//! a second machine can set without editing a checked-in file. The cost is
//! that it also shadows a pack that does carry its own copies; an explicit
//! path flag overrides one file, and unsetting the variable overrides all of
//! them.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::E3dReaderError;
use crate::page::{PageHeader, PageId, PageType};
use crate::record::explicit::ExplicitValue;
use crate::refno::RefNo;
use crate::session::DbView;
use crate::{ReadOnlyEngine, ScanTier};

/// One supported E3D release and the database-tree identities it accepts.
///
/// The identities validate the database family; they are not necessarily
/// unique release fingerprints. E3D 2.1 and 3.1 use the same pair in the
/// installed project corpora.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct E3dVersion {
    /// Directory name of the pack, and the id a manifest declares.
    pub id: &'static str,
    /// How the release calls itself.
    pub display: &'static str,
    /// Identity word of the primary-element table's type-5 pages.
    pub data_page_main: u32,
    /// Identity word of the RefNo index tree's type-5 pages.
    pub data_page_index: u32,
}

/// AVEVA Everything3D 2.1. The installation directory calls this release
/// `Everything3D2.10`; existing project roots commonly call it `E3D2.1`.
///
/// The accepted pair was censused over all 828 primary extents under
/// `D:/AVEVA/Projects/E3D2.1`: the selected session's `claim` root is
/// `(5, 7618377)` and its RefNo index root is `(5, 13387743)` in every file.
/// `7618321` exists in the 2.10 `core.dll`, but it is not the live `claim`
/// identity of these databases and therefore cannot identify their release.
pub const E3D210: E3dVersion = E3dVersion {
    id: "e3d210",
    display: "AVEVA Everything3D 2.1 (2.10)",
    data_page_main: 7_618_377,
    data_page_index: 13_387_743,
};

/// AVEVA Everything3D 3.1. The same pair was observed in all 1002 primary
/// extents under `D:/AVEVA/Projects/E3D3.1`.
pub const E3D31: E3dVersion = E3dVersion {
    id: "e3d31",
    display: "AVEVA Everything3D 3.1",
    data_page_main: crate::meta::constants::DATA_PAGE_MAIN,
    data_page_index: crate::meta::constants::DATA_PAGE_INDEX,
};

/// Every release this crate can name.
pub const KNOWN_VERSIONS: &[&E3dVersion] = &[&E3D210, &E3D31];

/// Session comment written by E3D 3.1's binary metadata save.
///
/// Diagnostic only. It is deliberately not used by either version detector.
pub const E3D31_BINARY_SESSION_MARKER: &str = "Saving Meta Data.";

/// Which database field supplied the evidence returned by a probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionEvidenceSource {
    /// A measured product version stored on a current `COMM` element.
    CommunicationVersion,
    /// The database migration number on the world element. Reported for
    /// diagnosis, but not mapped to an E3D product release.
    DatabaseUpgradeNumber,
    /// No version-bearing field was found.
    Unknown,
}

impl VersionEvidenceSource {
    pub fn field_name(self) -> &'static str {
        match self {
            Self::CommunicationVersion => "COMM.VERS",
            Self::DatabaseUpgradeNumber => "WORL.UPGDNO",
            Self::Unknown => "none",
        }
    }
}

/// Version evidence read from one database binary.
///
/// `version == None` is a first-class result: callers deciding a schema must
/// stop or use an explicit manual version rather than guessing from the path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseVersionEvidence {
    pub version: Option<&'static E3dVersion>,
    pub source: VersionEvidenceSource,
    pub raw_value: Option<u32>,
    pub raw_text: Option<String>,
    pub session_page: Option<u32>,
    pub database: PathBuf,
}

/// The release with this id, or `None` when the id names one that has not been
/// measured.
pub fn version_by_id(id: &str) -> Option<&'static E3dVersion> {
    KNOWN_VERSIONS
        .iter()
        .copied()
        .find(|version| version.id.eq_ignore_ascii_case(id))
}

/// Schema version of the manifest format itself, not of the E3D release.
pub const MANIFEST_SCHEMA: &str = "e3d-io-schema-pack/v1";

/// Name of the manifest inside a pack directory.
pub const MANIFEST_FILE: &str = "manifest.json";

#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error("read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{path} is not valid JSON: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("{path} declares schema {found:?}, expected {MANIFEST_SCHEMA:?}")]
    UnknownManifestSchema { path: PathBuf, found: String },

    #[error("{path} is missing the required field {field}")]
    MissingField { path: PathBuf, field: &'static str },

    #[error(
        "{path} declares release {id:?}, which this build has no constants for; \
         measured releases are {known}"
    )]
    UnknownVersion {
        path: PathBuf,
        id: String,
        known: String,
    },

    #[error("{pack} has no {what}: {tried}")]
    FileNotFound {
        pack: PathBuf,
        what: String,
        tried: String,
    },

    #[error("{pack} declares no template file for database type {db_type:?}")]
    NoSuchTemplate { pack: PathBuf, db_type: String },

    #[error("open {path} to detect its E3D release: {source}")]
    DatabaseRead {
        path: PathBuf,
        #[source]
        source: E3dReaderError,
    },

    #[error("{path}: {role} root {page} is not a readable table page: {detail}")]
    VersionProbe {
        path: PathBuf,
        role: &'static str,
        page: PageId,
        detail: String,
    },

    #[error(
        "{path}: database release is unknown (main table {data_page_main}, \
         RefNo index {data_page_index}); measured releases are {known}"
    )]
    UnknownDatabaseVersion {
        path: PathBuf,
        data_page_main: u32,
        data_page_index: u32,
        known: String,
    },

    #[error("{path}: database version evidence is insufficient ({detail})")]
    DatabaseVersionNotDetected { path: PathBuf, detail: String },

    #[error("read project directory {path}: {source}")]
    ProjectRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "{root}: no current COMM.VERS field identified the E3D release \
         ({candidates} candidates checked{detail})"
    )]
    ProjectVersionNotDetected {
        root: PathBuf,
        candidates: usize,
        detail: String,
    },
}

/// Read version evidence from one Dabacon database binary.
///
/// A single file can positively identify a release only when it is a
/// Communication database whose current indexed `COMM` records carry a
/// measured `VERS` value. Other database types intentionally return
/// `version: None`; their `UPGDNO` values are shared by the measured 2.10 and
/// 3.1 samples.
pub fn database_version_evidence(
    database: impl AsRef<Path>,
) -> Result<DatabaseVersionEvidence, SchemaError> {
    let path = database.as_ref();
    let mut view = DbView::open(path).map_err(|source| SchemaError::DatabaseRead {
        path: path.to_path_buf(),
        source,
    })?;
    validate_database_family(path, &mut view)?;

    let session_page = Some(view.session().page.page);
    let db_type = view.descriptor().db_type_hash;
    if db_type != e3d_attlib::db1_hash("COMM") {
        return Ok(DatabaseVersionEvidence {
            version: None,
            source: VersionEvidenceSource::Unknown,
            raw_value: None,
            raw_text: None,
            session_page,
            database: path.to_path_buf(),
        });
    }

    let (upgrade_number, versions) = scan_version_fields(path)?;
    if let Some((version, raw_text)) = classify_communication_versions(&versions) {
        return Ok(DatabaseVersionEvidence {
            version: Some(version),
            source: VersionEvidenceSource::CommunicationVersion,
            raw_value: upgrade_number,
            raw_text: Some(raw_text),
            session_page,
            database: path.to_path_buf(),
        });
    }

    if !versions.is_empty() {
        return Ok(DatabaseVersionEvidence {
            version: None,
            source: VersionEvidenceSource::CommunicationVersion,
            raw_value: upgrade_number,
            raw_text: Some(versions.into_iter().collect::<Vec<_>>().join(", ")),
            session_page,
            database: path.to_path_buf(),
        });
    }

    Ok(DatabaseVersionEvidence {
        version: None,
        source: upgrade_number
            .map(|_| VersionEvidenceSource::DatabaseUpgradeNumber)
            .unwrap_or(VersionEvidenceSource::Unknown),
        raw_value: upgrade_number,
        raw_text: None,
        session_page,
        database: path.to_path_buf(),
    })
}

/// Detect one database's release, refusing to guess when it has no measured
/// `COMM.VERS` value.
pub fn detect_database_version(
    database: impl AsRef<Path>,
) -> Result<&'static E3dVersion, SchemaError> {
    let evidence = database_version_evidence(database)?;
    evidence
        .version
        .ok_or_else(|| SchemaError::DatabaseVersionNotDetected {
            path: evidence.database.clone(),
            detail: evidence_detail(&evidence),
        })
}

fn validate_database_family(path: &Path, view: &mut DbView) -> Result<(), SchemaError> {
    let main_root = view.session().claim;
    let index_root = view.session().index_root;
    let data_page_main = table_identity(view, path, main_root, "primary-element table")?;
    let data_page_index = table_identity(view, path, index_root, "RefNo index")?;

    if !KNOWN_VERSIONS.iter().any(|version| {
        version.data_page_main == data_page_main && version.data_page_index == data_page_index
    }) {
        return Err(SchemaError::UnknownDatabaseVersion {
            path: path.to_path_buf(),
            data_page_main,
            data_page_index,
            known: known_version_pairs(),
        });
    }
    Ok(())
}

/// Read the strongest measured `COMM.VERS` evidence under a project tree.
///
/// Candidate discovery uses descriptor bytes, not names. Only primary
/// Communication databases are opened; backup files with a filename extension
/// are excluded from discovery. If several configured projects share a root,
/// the highest measured release wins.
pub fn project_version_evidence(
    project_root: impl AsRef<Path>,
) -> Result<DatabaseVersionEvidence, SchemaError> {
    let root = project_root.as_ref();
    if root.is_file() {
        return database_version_evidence(root);
    }

    let mut directories = vec![root.to_path_buf()];
    let mut candidates = Vec::new();
    while let Some(directory) = directories.pop() {
        let mut entries = fs::read_dir(&directory)
            .map_err(|source| SchemaError::ProjectRead {
                path: directory.clone(),
                source,
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| SchemaError::ProjectRead {
                path: directory.clone(),
                source,
            })?;
        entries.sort_by_key(|entry| entry.file_name());

        let mut children = Vec::new();
        for entry in entries {
            let file_type = entry
                .file_type()
                .map_err(|source| SchemaError::ProjectRead {
                    path: entry.path(),
                    source,
                })?;
            if file_type.is_dir() {
                children.push(entry.path());
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().is_none()
                && primary_database_type(&path) == Some(e3d_attlib::db1_hash("COMM"))
            {
                candidates.push(path);
            }
        }
        // The stack pops from the end; reverse to preserve lexical traversal.
        directories.extend(children.into_iter().rev());
    }

    candidates.sort();

    let candidate_count = candidates.len();
    let mut e3d210 = None;
    let mut first_failure = None;
    for path in candidates {
        match database_version_evidence(&path) {
            Ok(evidence) if evidence.version == Some(&E3D31) => return Ok(evidence),
            Ok(evidence) if evidence.version == Some(&E3D210) => {
                e3d210.get_or_insert(evidence);
            }
            Ok(evidence) => {
                first_failure.get_or_insert_with(|| {
                    format!(
                        "{}: {}",
                        evidence.database.display(),
                        evidence_detail(&evidence)
                    )
                });
            }
            Err(error @ SchemaError::UnknownDatabaseVersion { .. }) => return Err(error),
            Err(error) => {
                first_failure.get_or_insert_with(|| error.to_string());
            }
        }
    }
    if let Some(evidence) = e3d210 {
        return Ok(evidence);
    }

    Err(SchemaError::ProjectVersionNotDetected {
        root: root.to_path_buf(),
        candidates: candidate_count,
        detail: first_failure
            .map(|failure| format!("; first failure: {failure}"))
            .unwrap_or_default(),
    })
}

/// Detect a project release from current `COMM.VERS` fields.
pub fn detect_project_version(
    project_root: impl AsRef<Path>,
) -> Result<&'static E3dVersion, SchemaError> {
    let evidence = project_version_evidence(project_root)?;
    evidence
        .version
        .ok_or_else(|| SchemaError::ProjectVersionNotDetected {
            root: evidence.database.clone(),
            candidates: 1,
            detail: format!("; {}", evidence_detail(&evidence)),
        })
}

fn scan_version_fields(path: &Path) -> Result<(Option<u32>, BTreeSet<String>), SchemaError> {
    let mut engine = ReadOnlyEngine::open(path).map_err(|source| SchemaError::DatabaseRead {
        path: path.to_path_buf(),
        source,
    })?;
    let world = e3d_attlib::db1_hash("WORL");
    let communication = e3d_attlib::db1_hash("COMM");
    let upgrade_number = e3d_attlib::db1_hash("UPGDNO");
    let product_version = e3d_attlib::db1_hash("VERS");
    let null = RefNo::new(0, 0);
    let mut raw_upgrade = None;
    let mut raw_versions = BTreeSet::new();

    let scan =
        engine
            .scan_elements(ScanTier::Full)
            .map_err(|source| SchemaError::DatabaseRead {
                path: path.to_path_buf(),
                source,
            })?;
    for item in scan {
        let item = item.map_err(|source| SchemaError::DatabaseRead {
            path: path.to_path_buf(),
            source,
        })?;
        let Some(parsed) = item.parsed else {
            continue;
        };
        if item.noun_hash == world && item.owner == null {
            raw_upgrade = parsed
                .explicit_attributes
                .iter()
                .find(|attribute| attribute.hash == upgrade_number)
                .and_then(explicit_u32);
        }
        if item.noun_hash == communication {
            raw_versions.extend(
                parsed
                    .explicit_attributes
                    .iter()
                    .filter(|attribute| attribute.hash == product_version)
                    .filter_map(explicit_version_token),
            );
        }
    }
    Ok((raw_upgrade, raw_versions))
}

fn explicit_u32(attribute: &crate::record::explicit::ExplicitAttribute) -> Option<u32> {
    match &attribute.value {
        ExplicitValue::Integer(value) => u32::try_from(*value).ok(),
        ExplicitValue::RawWords(words) if words.len() == 1 => words.first().copied(),
        _ => None,
    }
}

fn explicit_version_token(
    attribute: &crate::record::explicit::ExplicitAttribute,
) -> Option<String> {
    match &attribute.value {
        ExplicitValue::Text(value) => version_token(value),
        ExplicitValue::RawWords(words) => {
            let bytes = words
                .iter()
                .flat_map(|word| word.to_be_bytes())
                .collect::<Vec<_>>();
            version_token(&String::from_utf8_lossy(&bytes))
        }
        _ => None,
    }
}

fn version_token(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let start = cursor + bytes[cursor..].iter().position(u8::is_ascii_digit)?;
        let end = bytes[start..]
            .iter()
            .position(|byte| !byte.is_ascii_alphanumeric() && !matches!(*byte, b'.' | b'-' | b'_'))
            .map(|offset| start + offset)
            .unwrap_or(bytes.len());
        let token = &value[start..end];
        if token.contains('.') {
            return Some(token.to_string());
        }
        cursor = end.max(start + 1);
    }
    None
}

fn classify_communication_versions(
    versions: &BTreeSet<String>,
) -> Option<(&'static E3dVersion, String)> {
    for version in versions {
        if has_release_prefix(version, "3.1") {
            return Some((&E3D31, version.clone()));
        }
    }
    for version in versions {
        if ["2.1", "2.10", "12.1"]
            .iter()
            .any(|prefix| has_release_prefix(version, prefix))
        {
            return Some((&E3D210, version.clone()));
        }
    }
    None
}

fn has_release_prefix(value: &str, prefix: &str) -> bool {
    value == prefix
        || value
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('.'))
}

fn evidence_detail(evidence: &DatabaseVersionEvidence) -> String {
    format!(
        "field={} raw_value={:?} raw_text={:?} session_page={:?}",
        evidence.source.field_name(),
        evidence.raw_value,
        evidence.raw_text,
        evidence.session_page
    )
}

/// The schema root carried by this checkout/build.
pub fn bundled_schema_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("schema")
}

fn table_identity(
    view: &mut DbView,
    path: &Path,
    page: PageId,
    role: &'static str,
) -> Result<u32, SchemaError> {
    let bytes = view
        .pages_mut()
        .get(page)
        .map_err(|error| SchemaError::VersionProbe {
            path: path.to_path_buf(),
            role,
            page,
            detail: error.to_string(),
        })?;
    let header = PageHeader::parse(bytes).ok_or_else(|| SchemaError::VersionProbe {
        path: path.to_path_buf(),
        role,
        page,
        detail: "page is too short for a page header".to_string(),
    })?;
    if header.page_type != PageType::Tree {
        return Err(SchemaError::VersionProbe {
            path: path.to_path_buf(),
            role,
            page,
            detail: format!("page type is {:?}, expected Tree (5)", header.page_type),
        });
    }
    Ok(header.type_id)
}

/// Read just enough of a file to decide whether it is a primary Dabacon
/// database and, if so, return its database-type hash.
///
/// Project trees contain thousands of non-database files and secondary
/// extents. Rejecting them from the first 60 bytes avoids opening a session
/// chain merely to discover that the file was never a primary database.
fn primary_database_type(path: &Path) -> Option<u32> {
    const DESCRIPTOR_PREFIX: usize = 0x3C;
    let mut file = File::open(path).ok()?;
    let mut prefix = [0u8; DESCRIPTOR_PREFIX];
    file.read_exact(&mut prefix).ok()?;
    let descriptor = crate::meta::DbDescriptor::parse(&prefix).ok()?;
    if descriptor.ext_no != 1 || descriptor.db_type_name().is_none() {
        return None;
    }
    Some(descriptor.db_type_hash)
}

fn known_version_pairs() -> String {
    KNOWN_VERSIONS
        .iter()
        .map(|version| {
            format!(
                "{}=({},{})",
                version.id, version.data_page_main, version.data_page_index
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// One release's schema files, resolved from a pack directory.
#[derive(Debug, Clone)]
pub struct SchemaPack {
    root: PathBuf,
    version: &'static E3dVersion,
    binaries_root: Option<PathBuf>,
    binaries_root_env: Option<String>,
    attlib: String,
    templates: BTreeMap<String, String>,
    catalog: BTreeMap<String, String>,
}

impl SchemaPack {
    /// Open the pack at `pack_dir`, which must hold a [`MANIFEST_FILE`].
    pub fn open(pack_dir: impl AsRef<Path>) -> Result<Self, SchemaError> {
        let root = pack_dir.as_ref().to_path_buf();
        let manifest_path = root.join(MANIFEST_FILE);
        let text = std::fs::read_to_string(&manifest_path).map_err(|source| SchemaError::Io {
            path: manifest_path.clone(),
            source,
        })?;
        let manifest: Value = serde_json::from_str(&text).map_err(|source| SchemaError::Json {
            path: manifest_path.clone(),
            source,
        })?;

        let declared = string_at(&manifest, &["schema"]).unwrap_or_default();
        if declared != MANIFEST_SCHEMA {
            return Err(SchemaError::UnknownManifestSchema {
                path: manifest_path,
                found: declared,
            });
        }

        let id = string_at(&manifest, &["version", "id"]).ok_or(SchemaError::MissingField {
            path: manifest_path.clone(),
            field: "version.id",
        })?;
        let version = version_by_id(&id).ok_or_else(|| SchemaError::UnknownVersion {
            path: manifest_path.clone(),
            id: id.clone(),
            known: KNOWN_VERSIONS
                .iter()
                .map(|version| version.id)
                .collect::<Vec<_>>()
                .join(", "),
        })?;

        let attlib =
            string_at(&manifest, &["binaries", "attlib"]).ok_or(SchemaError::MissingField {
                path: manifest_path.clone(),
                field: "binaries.attlib",
            })?;

        Ok(Self {
            root,
            version,
            binaries_root: string_at(&manifest, &["binaries", "root"]).map(PathBuf::from),
            binaries_root_env: string_at(&manifest, &["binaries", "root_env"]),
            attlib,
            // Database types are matched case-insensitively, so they are
            // folded once here rather than at every lookup. Catalog keys are
            // manifest field names and stay as written.
            templates: string_map(&manifest, &["binaries", "templates"])
                .into_iter()
                .map(|(key, value)| (key.to_ascii_uppercase(), value))
                .collect(),
            catalog: string_map(&manifest, &["catalog"]),
        })
    }

    /// Open `schema_root/<version_id>`, the layout this crate's `schema/`
    /// directory uses.
    pub fn open_version(
        schema_root: impl AsRef<Path>,
        version_id: &str,
    ) -> Result<Self, SchemaError> {
        Self::open(schema_root.as_ref().join(version_id))
    }

    /// Detect one database's project release and open the matching pack below
    /// `schema_root`.
    ///
    /// Product evidence lives in the sibling Communication database, not in a
    /// DESI/SYST file. The parent directory is used only to discover primary
    /// COMM binaries; its spelling is never version evidence.
    pub fn detect_for_database(
        schema_root: impl AsRef<Path>,
        database: impl AsRef<Path>,
    ) -> Result<Self, SchemaError> {
        let database = database.as_ref();
        let context = database.parent().unwrap_or(database);
        let evidence = project_version_evidence(context)?;
        let version = evidence
            .version
            .ok_or_else(|| SchemaError::DatabaseVersionNotDetected {
                path: database.to_path_buf(),
                detail: evidence_detail(&evidence),
            })?;
        Self::open_version(schema_root, version.id)
    }

    /// Detect a project root's release and open the matching pack below
    /// `schema_root`.
    pub fn detect_for_project(
        schema_root: impl AsRef<Path>,
        project_root: impl AsRef<Path>,
    ) -> Result<Self, SchemaError> {
        let version = detect_project_version(project_root)?;
        Self::open_version(schema_root, version.id)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn version(&self) -> &'static E3dVersion {
        self.version
    }

    /// Database types this pack has a `*vir.dat` for, e.g. `DESI`, `CATA`.
    pub fn template_types(&self) -> impl Iterator<Item = &str> {
        self.templates.keys().map(String::as_str)
    }

    /// The `*vir.dat` holding descriptors for one database type.
    pub fn template(&self, db_type: &str) -> Result<PathBuf, SchemaError> {
        let upper = db_type.to_ascii_uppercase();
        let file = self
            .templates
            .get(&upper)
            .ok_or_else(|| SchemaError::NoSuchTemplate {
                pack: self.root.clone(),
                db_type: upper.clone(),
            })?;
        self.binary(file, &format!("{upper} template {file}"))
    }

    pub fn attlib(&self) -> Result<PathBuf, SchemaError> {
        let file = self.attlib.clone();
        self.binary(&file, &format!("attribute library {file}"))
    }

    /// One of the catalog exports, by the key the manifest files it under
    /// (`noun_layout`, `noun_attr_fields`, …). These live inside the pack, so
    /// unlike the binaries they are not searched for.
    pub fn catalog_file(&self, key: &str) -> Option<PathBuf> {
        self.catalog.get(key).map(|file| self.root.join(file))
    }

    pub fn noun_layout(&self) -> Option<PathBuf> {
        self.catalog_file("noun_layout")
    }

    pub fn noun_attr_fields(&self) -> Option<PathBuf> {
        self.catalog_file("noun_attr_fields")
    }

    /// Where a binary would be looked for, in the order they are tried. Public
    /// so a caller reporting a miss can say what it checked.
    pub fn binary_candidates(&self, file: &str) -> Vec<PathBuf> {
        let mut out = Vec::with_capacity(3);
        if let Some(name) = &self.binaries_root_env {
            if let Some(root) = std::env::var_os(name) {
                out.push(PathBuf::from(root).join(file));
            }
        }
        out.push(self.root.join("binaries").join(file));
        if let Some(root) = &self.binaries_root {
            out.push(root.join(file));
        }
        out
    }

    fn binary(&self, file: &str, what: &str) -> Result<PathBuf, SchemaError> {
        let candidates = self.binary_candidates(file);
        candidates
            .iter()
            .find(|path| path.is_file())
            .cloned()
            .ok_or_else(|| SchemaError::FileNotFound {
                pack: self.root.clone(),
                what: what.to_string(),
                tried: candidates
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            })
    }
}

fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut cursor = value;
    for key in path {
        cursor = cursor.get(key)?;
    }
    cursor.as_str().map(str::to_string)
}

fn string_map(value: &Value, path: &[&str]) -> BTreeMap<String, String> {
    let mut cursor = value;
    for key in path {
        match cursor.get(key) {
            Some(next) => cursor = next,
            None => return BTreeMap::new(),
        }
    }
    cursor
        .as_object()
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn pack(manifest: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(MANIFEST_FILE), manifest).unwrap();
        dir
    }

    const MINIMAL: &str = r#"{
        "schema": "e3d-io-schema-pack/v1",
        "version": { "id": "e3d31" },
        "binaries": { "attlib": "attlib.dat", "templates": { "DESI": "desvir.dat" } },
        "catalog": { "noun_layout": "catalog/noun_layout.json" }
    }"#;

    #[test]
    fn open_reads_the_release_and_the_file_map() {
        let dir = pack(MINIMAL);
        let pack = SchemaPack::open(dir.path()).unwrap();
        assert_eq!(pack.version().id, "e3d31");
        assert_eq!(pack.version().data_page_index, 13_387_743);
        assert_eq!(pack.template_types().collect::<Vec<_>>(), vec!["DESI"]);
        assert_eq!(
            pack.noun_layout(),
            Some(dir.path().join("catalog/noun_layout.json"))
        );
    }

    /// A pack that carries its own binaries needs no absolute path anywhere.
    #[test]
    fn a_binary_inside_the_pack_is_found() {
        let dir = pack(MINIMAL);
        fs::create_dir(dir.path().join("binaries")).unwrap();
        fs::write(dir.path().join("binaries/desvir.dat"), [0u8; 4]).unwrap();
        assert_eq!(
            SchemaPack::open(dir.path())
                .unwrap()
                .template("desi")
                .unwrap(),
            dir.path().join("binaries").join("desvir.dat")
        );
    }

    /// The miss names every place it looked, so a wrong pack root is one
    /// message away from being obvious.
    #[test]
    fn a_missing_binary_names_what_was_tried() {
        let dir = pack(MINIMAL);
        let error = SchemaPack::open(dir.path()).unwrap().attlib().unwrap_err();
        let message = error.to_string();
        assert!(message.contains("attlib.dat"), "{message}");
        assert!(message.contains("binaries"), "{message}");
    }

    #[test]
    fn a_database_type_the_pack_does_not_carry_is_named() {
        let dir = pack(MINIMAL);
        let error = SchemaPack::open(dir.path())
            .unwrap()
            .template("CATA")
            .unwrap_err();
        assert!(error.to_string().contains("CATA"), "{error}");
    }

    /// An unmeasured release is refused by name rather than falling back to a
    /// measured release, which is the failure this module exists to prevent.
    #[test]
    fn an_unknown_release_is_refused_and_lists_the_known_ones() {
        let dir = pack(
            r#"{
            "schema": "e3d-io-schema-pack/v1",
            "version": { "id": "e3d99" },
            "binaries": { "attlib": "attlib.dat" }
        }"#,
        );
        let error = SchemaPack::open(dir.path()).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("e3d99"), "{message}");
        assert!(message.contains("e3d210"), "{message}");
        assert!(message.contains("e3d31"), "{message}");
    }

    fn versioned_database(
        root: &Path,
        relative: &str,
        main_identity: u32,
        index_identity: u32,
        e3d31_metadata_session: bool,
    ) -> PathBuf {
        const WORDS: u32 = 64;
        const PAGE_BYTES: usize = WORDS as usize * 4;
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut pages = vec![vec![0u8; PAGE_BYTES]; 5];
        let mut put = |page: usize, at: usize, value: u32| {
            pages[page][at..at + 4].copy_from_slice(&value.to_be_bytes());
        };
        put(0, 0x34, WORDS);
        put(0, 0x28, 2);
        put(0, 0x2C, 1);
        put(0, 0x30, 2);
        put(0, 0x38, 16_192);
        put(0, 0x20, e3d_attlib::db1_hash("DESI"));
        put(2, 0x00, 3);
        put(2, 0x04, u32::MAX);
        put(2, 0x08, 1);
        put(2, 0x0C, 1);
        put(2, 0x1C, 4);
        put(2, 0x20, 1);
        put(2, 0x24, 3);
        put(2, 0x28, 1);
        put(3, 0x00, 5);
        put(3, 0x04, main_identity);
        put(4, 0x00, 5);
        put(4, 0x04, index_identity);
        if e3d31_metadata_session {
            let marker = E3D31_BINARY_SESSION_MARKER.as_bytes();
            put(2, 0xA0, marker.len().div_ceil(4) as u32);
            pages[2][0xA4..0xA4 + marker.len()].copy_from_slice(marker);
        }

        let mut file = fs::File::create(&path).unwrap();
        for page in pages {
            file.write_all(&page).unwrap();
        }
        file.flush().unwrap();
        path
    }

    #[test]
    fn session_comments_and_path_names_are_not_version_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let without_marker = versioned_database(
            dir.path(),
            "directory-called-E3D3.1/ams8000_0001",
            E3D210.data_page_main,
            E3D210.data_page_index,
            false,
        );
        let with_marker = versioned_database(
            dir.path(),
            "directory-called-E3D2.1/ams1112_0001",
            E3D31.data_page_main,
            E3D31.data_page_index,
            true,
        );
        for database in [without_marker, with_marker] {
            let evidence = database_version_evidence(&database).unwrap();
            assert_eq!(evidence.version, None);
            assert_eq!(evidence.source, VersionEvidenceSource::Unknown);
            assert!(matches!(
                detect_database_version(database),
                Err(SchemaError::DatabaseVersionNotDetected { .. })
            ));
        }
    }

    #[test]
    fn a_truncated_session_is_a_read_error_not_an_unknown_version() {
        let dir = tempfile::tempdir().unwrap();
        let database = versioned_database(
            dir.path(),
            "project/ams8000_0001",
            E3D210.data_page_main,
            E3D210.data_page_index,
            false,
        );
        fs::OpenOptions::new()
            .write(true)
            .open(&database)
            .unwrap()
            .set_len(2 * 256 + 64)
            .unwrap();
        assert!(matches!(
            database_version_evidence(database),
            Err(SchemaError::DatabaseRead { .. })
        ));
    }

    #[test]
    fn an_incompatible_database_tree_pair_is_not_assigned_a_release() {
        let dir = tempfile::tempdir().unwrap();
        let database = versioned_database(
            dir.path(),
            "ams000/ams8000_0001",
            E3D210.data_page_main,
            99,
            false,
        );
        assert!(matches!(
            detect_database_version(database),
            Err(SchemaError::UnknownDatabaseVersion {
                data_page_main: 7_618_377,
                data_page_index: 99,
                ..
            })
        ));
    }

    #[test]
    fn a_project_without_a_communication_database_is_unknown() {
        let dir = tempfile::tempdir().unwrap();
        versioned_database(
            dir.path(),
            "project/ams8000_0001",
            E3D210.data_page_main,
            E3D210.data_page_index,
            false,
        );
        assert!(matches!(
            detect_project_version(dir.path()),
            Err(SchemaError::ProjectVersionNotDetected { candidates: 0, .. })
        ));
    }

    #[test]
    fn measured_comm_versions_map_without_guessing_unknown_values() {
        let versions = BTreeSet::from(["3.1.9.0".to_string(), "1.3.13.0".to_string()]);
        assert_eq!(
            classify_communication_versions(&versions),
            Some((&E3D31, "3.1.9.0".to_string()))
        );

        let versions = BTreeSet::from(["12.1.SP4.51".to_string()]);
        assert_eq!(
            classify_communication_versions(&versions),
            Some((&E3D210, "12.1.SP4.51".to_string()))
        );

        for unknown in ["", "1.3.13.0", "3.2.0.0", "4.0.0.0"] {
            let versions = BTreeSet::from([unknown.to_string()]);
            assert_eq!(classify_communication_versions(&versions), None);
        }
    }

    #[test]
    fn a_raw_comm_version_tolerates_prefix_words_but_keeps_the_exact_token() {
        let attribute = crate::record::explicit::ExplicitAttribute {
            hash: e3d_attlib::db1_hash("VERS"),
            type_code: 0,
            data_words: 4,
            is_uda: false,
            value: ExplicitValue::RawWords(vec![7, 0x332E_312E, 0x392E_3000, 0]),
        };
        assert_eq!(
            explicit_version_token(&attribute).as_deref(),
            Some("3.1.9.0")
        );
        assert_eq!(
            version_token("AVEVA Everything3D 3.1.9.0").as_deref(),
            Some("3.1.9.0")
        );
    }

    #[test]
    #[ignore = "requires the installed E3D 2.1 and 3.1 AvevaMarineSample projects"]
    fn live_project_evidence_comes_from_comm_vers_and_legacy_ams_stays_unknown() {
        let old = Path::new(r"D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000");
        let new = Path::new(r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000");
        let (old_sys_upgrade, _) = scan_version_fields(&old.join("amssys")).unwrap();
        let (new_sys_upgrade, _) = scan_version_fields(&new.join("amssys")).unwrap();
        assert_eq!(old_sys_upgrade, Some(12010401));
        assert_eq!(new_sys_upgrade, old_sys_upgrade);

        let (old_desi_upgrade, _) = scan_version_fields(&old.join("ams8000_0001")).unwrap();
        let (new_desi_upgrade, _) = scan_version_fields(&new.join("ams8000_0001")).unwrap();
        assert_eq!(old_desi_upgrade, Some(12010104));
        assert_eq!(new_desi_upgrade, old_desi_upgrade);

        let e3d31 =
            project_version_evidence(r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000").unwrap();
        assert_eq!(e3d31.version, Some(&E3D31));
        assert_eq!(e3d31.source, VersionEvidenceSource::CommunicationVersion);
        assert_eq!(e3d31.raw_text.as_deref(), Some("3.1.9.0"));
        assert!(e3d31.database.ends_with("amscom"));

        let e3d210 = project_version_evidence(r"D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000");
        assert!(matches!(
            e3d210,
            Err(SchemaError::ProjectVersionNotDetected { .. })
        ));
    }

    #[test]
    #[ignore = "requires the installed E3D 3.1 AvevaMarineSample project"]
    fn live_comm_evidence_does_not_depend_on_the_directory_name() {
        let source = Path::new(r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\amscom");
        let dir = tempfile::tempdir().unwrap();
        let copied = dir.path().join("directory-called-E3D2.1").join("database");
        fs::create_dir_all(copied.parent().unwrap()).unwrap();
        fs::copy(source, &copied).unwrap();
        let evidence = project_version_evidence(copied.parent().unwrap()).unwrap();
        assert_eq!(evidence.version, Some(&E3D31));
        assert_eq!(evidence.raw_text.as_deref(), Some("3.1.9.0"));
    }

    /// The shipped 3.1 pack must not send anyone to a path only one machine
    /// has. It carries its own binaries, so the last place it looks is inside
    /// itself; a re-added `binaries.root` would make a working tree depend on
    /// a drive again without any test noticing.
    #[test]
    fn the_shipped_3_1_pack_names_no_path_outside_itself() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/e3d31");
        let pack = SchemaPack::open(&root).unwrap();
        assert_eq!(pack.version().id, "e3d31");
        assert!(
            pack.binaries_root.is_none(),
            "manifest records an absolute binaries.root: {:?}",
            pack.binaries_root
        );
        assert_eq!(
            pack.binary_candidates("desvir.dat").last().unwrap(),
            &root.join("binaries").join("desvir.dat")
        );
    }

    #[test]
    fn shipped_pack_manifests_repeat_the_measured_tree_identities() {
        let schema_root = bundled_schema_root();
        for version in KNOWN_VERSIONS {
            let manifest_path = schema_root.join(version.id).join(MANIFEST_FILE);
            let manifest: Value =
                serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
            assert_eq!(
                manifest["constants"]["data_page_main"].as_u64(),
                Some(u64::from(version.data_page_main)),
                "{}",
                manifest_path.display()
            );
            assert_eq!(
                manifest["constants"]["data_page_index"].as_u64(),
                Some(u64::from(version.data_page_index)),
                "{}",
                manifest_path.display()
            );
        }
    }

    #[test]
    fn shipped_packs_carry_the_schema_files_used_by_gen_model() {
        let schema_root = bundled_schema_root();
        for version in KNOWN_VERSIONS {
            let pack = SchemaPack::open_version(&schema_root, version.id).unwrap();
            assert!(pack.attlib().unwrap().is_file(), "{}", version.id);
            for db_type in ["DESI", "CATA", "DICT"] {
                assert!(
                    pack.template(db_type).unwrap().is_file(),
                    "{} {db_type}",
                    version.id
                );
            }
        }
    }

    #[test]
    fn a_manifest_of_another_format_is_refused() {
        let dir = pack(r#"{ "schema": "something-else/v9" }"#);
        assert!(matches!(
            SchemaPack::open(dir.path()),
            Err(SchemaError::UnknownManifestSchema { .. })
        ));
    }
}
