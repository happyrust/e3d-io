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
//! corpora use `(7618377, 13387743)`. The binary writer does leave one usable
//! discriminator. E3D 3.1's `core.dll` contains and writes sessions commented
//! `Saving Meta Data.`; the 2.10 binary contains no such literal. Five
//! databases in the installed 3.1 project carry those sessions, while none of
//! 4,232 database extents in the 2.1 project does. [`detect_database_version`]
//! reads that retained session chain, and [`detect_project_version`] takes the
//! newest format evidenced by any primary database under the project root.
//! Paths and directory names are never evidence.
//!
//! This deliberately detects the binary's minimum required schema, not the
//! name of the product installation around it. Of 824 database paths shared
//! by the two installed project trees, 816 have the same length and the
//! sampled unchanged files are byte-for-byte identical. No algorithm can
//! label identical bytes differently because one copy lives under a directory
//! called `E3D3.1`. An untouched copy therefore remains 2.1-compatible; once
//! 3.1 writes its metadata session, the binary promotes the project to the
//! 3.1 pack.
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

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::E3dReaderError;
use crate::page::{PageHeader, PageId, PageType};
use crate::session::DbView;

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
/// The literal occurs in `Everything3D3.1/core.dll` and not in
/// `Everything3D2.10/core.dll`. It was also censused across the installed
/// project binaries: 0 hits in 2.1, 5 database hits in 3.1.
pub const E3D31_BINARY_SESSION_MARKER: &str = "Saving Meta Data.";

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

    #[error("read project directory {path}: {source}")]
    ProjectRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "{root}: no readable primary database binary identified the E3D release \
         ({candidates} candidates checked{detail})"
    )]
    ProjectVersionNotDetected {
        root: PathBuf,
        candidates: usize,
        detail: String,
    },
}

/// Detect the minimum E3D schema required by one Dabacon database binary.
///
/// The selected session's primary-element / RefNo-index pair first validates
/// the database family. The retained session chain then supplies the version
/// evidence: a `Saving Meta Data.` session can only have been written by 3.1.
/// Without that evidence the bytes remain 2.1-compatible, regardless of the
/// directory in which somebody placed the file.
pub fn detect_database_version(
    database: impl AsRef<Path>,
) -> Result<&'static E3dVersion, SchemaError> {
    let path = database.as_ref();
    let mut view = DbView::open(path).map_err(|source| SchemaError::DatabaseRead {
        path: path.to_path_buf(),
        source,
    })?;
    detect_database_version_in_view(path, &mut view)
}

fn detect_database_version_in_view(
    path: &Path,
    view: &mut DbView,
) -> Result<&'static E3dVersion, SchemaError> {
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

    let sessions = view
        .walk_sessions()
        .map_err(|source| SchemaError::DatabaseRead {
            path: path.to_path_buf(),
            source,
        })?;
    if sessions
        .iter()
        .any(|session| session.comment == E3D31_BINARY_SESSION_MARKER)
    {
        Ok(&E3D31)
    } else {
        Ok(&E3D210)
    }
}

/// Detect the highest binary schema evidenced by a project tree.
///
/// Every primary Dabacon file is inspected, with SYST databases first. A
/// single 3.1 metadata session promotes the whole project to the 3.1 pack so a
/// mixed upgraded project reads new records with the newer schema. If every
/// retained chain is 2.1-compatible, the 2.1 pack is selected. Directory names
/// and filesystem timestamps do not participate.
pub fn detect_project_version(
    project_root: impl AsRef<Path>,
) -> Result<&'static E3dVersion, SchemaError> {
    let root = project_root.as_ref();
    if root.is_file() {
        return detect_database_version(root);
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
            if let Some(db_type) = primary_database_type(&path) {
                candidates.push((db_type, path));
            }
        }
        // The stack pops from the end; reverse to preserve lexical traversal.
        directories.extend(children.into_iter().rev());
    }

    let syst = e3d_attlib::db1_hash("SYST");
    candidates.sort_by(|(left_type, left_path), (right_type, right_path)| {
        (*left_type != syst)
            .cmp(&(*right_type != syst))
            .then_with(|| left_path.cmp(right_path))
    });

    let candidate_count = candidates.len();
    let mut checked = 0usize;
    let mut first_failure = None;
    for (_, path) in candidates {
        match detect_database_version(&path) {
            Ok(version) if version == &E3D31 => return Ok(&E3D31),
            Ok(_) => checked += 1,
            Err(error @ SchemaError::UnknownDatabaseVersion { .. }) => return Err(error),
            Err(error) => {
                first_failure.get_or_insert_with(|| error.to_string());
            }
        }
    }
    if checked > 0 && first_failure.is_none() {
        return Ok(&E3D210);
    }

    Err(SchemaError::ProjectVersionNotDetected {
        root: root.to_path_buf(),
        candidates: candidate_count,
        detail: first_failure
            .map(|failure| format!("; first failure: {failure}"))
            .unwrap_or_default(),
    })
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

    /// Detect one database's release and open the matching pack below
    /// `schema_root`.
    pub fn detect_for_database(
        schema_root: impl AsRef<Path>,
        database: impl AsRef<Path>,
    ) -> Result<Self, SchemaError> {
        let version = detect_database_version(database)?;
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
    fn database_version_comes_from_binary_session_evidence_not_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let e3d210 = versioned_database(
            dir.path(),
            "directory-called-E3D3.1/ams8000_0001",
            E3D210.data_page_main,
            E3D210.data_page_index,
            false,
        );
        let e3d31 = versioned_database(
            dir.path(),
            "directory-called-E3D2.1/ams1112_0001",
            E3D31.data_page_main,
            E3D31.data_page_index,
            true,
        );
        assert_eq!(detect_database_version(e3d210).unwrap(), &E3D210);
        assert_eq!(detect_database_version(e3d31).unwrap(), &E3D31);
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
    fn project_detection_uses_the_highest_binary_evidence_any_file_carries() {
        let dir = tempfile::tempdir().unwrap();
        versioned_database(
            dir.path(),
            "old/ams8000_0001",
            E3D210.data_page_main,
            E3D210.data_page_index,
            false,
        );
        versioned_database(
            dir.path(),
            "new/ams1112_0001",
            E3D31.data_page_main,
            E3D31.data_page_index,
            true,
        );
        assert_eq!(detect_project_version(dir.path()).unwrap(), &E3D31);
    }

    #[test]
    fn an_all_legacy_binary_project_selects_the_2_10_pack() {
        let dir = tempfile::tempdir().unwrap();
        versioned_database(
            dir.path(),
            "project/ams8000_0001",
            E3D210.data_page_main,
            E3D210.data_page_index,
            false,
        );
        assert_eq!(detect_project_version(dir.path()).unwrap(), &E3D210);
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
