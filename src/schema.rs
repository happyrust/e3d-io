//! Which E3D release a set of schema files came from, and where those files are.
//!
//! Three kinds of input are release-bound and **none of them states its own
//! release**: an installation's `*vir.dat` descriptor files, its `attlib.dat`,
//! and the live catalog export taken beside them. Mixing two releases is
//! therefore silent. `desvir.dat` from one and `attlib.dat` from another both
//! parse; the descriptors decide the word offsets, so the result is plausible
//! numbers read out of the wrong words rather than an error. The reader has no
//! way to notice — `TemplateFile` checks `template_type`, which says `DESI`,
//! not `3.1`.
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
//! A pack is meant to carry its own binaries — the 3.1 pack holds all 20
//! `*vir.dat` and `attlib.dat` — so that naming the pack names everything.
//! They are AVEVA installation files and cannot be redistributed, so
//! `binaries/` is git-ignored and a fresh clone finds it empty; that is why
//! [`SchemaPack`] searches for each file rather than assuming it is there.
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
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Magic numbers that change between E3D releases.
///
/// These are the values `core.dll` compares against, so they belong to the
/// binary that wrote a database rather than to the database. Everything else
/// this crate reads — page size, session chain, descriptor geometry — is
/// stated by the file itself and needs no entry here.
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

/// AVEVA Everything3D 3.1, the release every constant in this crate was
/// recovered from.
pub const E3D31: E3dVersion = E3dVersion {
    id: "e3d31",
    display: "AVEVA Everything3D 3.1",
    data_page_main: crate::meta::constants::DATA_PAGE_MAIN,
    data_page_index: crate::meta::constants::DATA_PAGE_INDEX,
};

/// Every release this crate can name.
///
/// E3D 2.10 is deliberately absent. Its `data_page_main` is known to be
/// 7 618 321 rather than 7 618 377, but its index-tree identity has never been
/// measured, and a release entry carrying one real number and one guessed one
/// is worse than no entry: the guess would be indistinguishable from evidence
/// at every call site. Add the release here once both words are read off a
/// 2.10 file.
pub const KNOWN_VERSIONS: &[&E3dVersion] = &[&E3D31];

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

    /// An unmeasured release is refused by name rather than falling back to
    /// 3.1's constants, which is the failure this module exists to prevent.
    #[test]
    fn an_unknown_release_is_refused_and_lists_the_known_ones() {
        let dir = pack(
            r#"{
            "schema": "e3d-io-schema-pack/v1",
            "version": { "id": "e3d210" },
            "binaries": { "attlib": "attlib.dat" }
        }"#,
        );
        let error = SchemaPack::open(dir.path()).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("e3d210"), "{message}");
        assert!(message.contains("e3d31"), "{message}");
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
    fn a_manifest_of_another_format_is_refused() {
        let dir = pack(r#"{ "schema": "something-else/v9" }"#);
        assert!(matches!(
            SchemaPack::open(dir.path()),
            Err(SchemaError::UnknownManifestSchema { .. })
        ));
    }
}
