//! E3D's own limits on an attribute — the ones its dictionary applies before a
//! value reaches the record, which the schema templates do not carry
//! (`docs/plans/2026-09-22-write-path-phase-2-plan.md` §1.2 / T1b; the TTY gate
//! `docs/evidence/2026-09-21-e3d-tty-gate-write-path.md` is the oracle).
//!
//! The data is the live export `schema/e3d31/catalog/noun_attr_fields.json`:
//! one record per attribute E3D's dictionary answers for, with its
//! `DbAttributeField` values, of which two matter here —
//!
//! - **whether the attribute is there at all**: `RATM` and `UNIPAR` are not,
//!   and E3D says "does not have attribute" to a write of them although the
//!   template stores them;
//! - **`SIZE`**: the dictionary's length limit — `DESC` 120 **characters**
//!   where the template allows 480 **bytes**, `NAME` 500, `DESP` 100 reals,
//!   `POS` 3, `UUIDIA` 4, `ROKEYS` 20 items where the template declares
//!   500 000.
//!
//! Two behaviours the export does not state are table-driven exceptions the
//! gate observed: `UUIDIA` is **set once** (E3D refuses to change it although
//! `CHANGE=True`), and `USRWEI` is a **pseudo attribute** (stored, but E3D
//! reads it unset). Both lists can be extended.
//!
//! A [`Dictionary`] is optional: a `Transaction` without one behaves as
//! before, checking only the template's shape and capacity.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::record::descriptor::DescriptorValue;

/// Attributes E3D lets a caller set only while they are unset.
pub const SET_ONCE: [&str; 1] = ["UUIDIA"];

/// Attributes E3D stores but never reads back (a write is pointless, so it
/// is refused rather than silently lost).
pub const PSEUDO: [&str; 1] = ["USRWEI"];

/// One attribute's dictionary entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictionaryEntry {
    /// The attribute's DB name as the dictionary spells it.
    pub name: String,
    /// `SIZE`: the character count of a text, the element count of an array,
    /// 1 for a scalar. `None` when the export carries no `SIZE`.
    pub size: Option<u32>,
    /// `TYPE`: E3D's logical type code (1 integer, 2 real, 4 text, 5
    /// reference, 8 position, 11 string list, …), kept for a caller that
    /// wants to show it; the write path shapes values by the template.
    pub e3d_type: Option<u32>,
}

/// How a value fails the dictionary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DictionaryError {
    #[error("{name} is not an attribute in E3D's dictionary (E3D: \"does not have attribute\")")]
    NotInDictionary { name: String },
    #[error("{name}: {chars} characters, the dictionary allows {max}")]
    TextTooLong { name: String, chars: usize, max: u32 },
    #[error("{name}: {count} elements, the dictionary allows {max}")]
    TooManyElements { name: String, count: usize, max: u32 },
    #[error("{name} is set once: it already holds a value, and E3D refuses to change it")]
    SetOnce { name: String },
    #[error("{name} is a pseudo attribute: E3D stores it but never reads it back")]
    Pseudo { name: String },
}

/// The dictionary's limits, keyed by attribute hash.
#[derive(Debug, Clone, Default)]
pub struct Dictionary {
    entries: HashMap<u32, DictionaryEntry>,
    set_once: HashSet<u32>,
    pseudo: HashSet<u32>,
}

impl Dictionary {
    /// Load `noun_attr_fields.json` (the pack's `catalog/noun_attr_fields.json`).
    pub fn read(path: impl AsRef<Path>) -> Result<Self, DictionaryLoadError> {
        let path = path.as_ref();
        let json = std::fs::read_to_string(path).map_err(|source| DictionaryLoadError::Io { path: path.to_path_buf(), source })?;
        Self::parse(&json).map_err(|source| DictionaryLoadError::Json { path: path.to_path_buf(), source })
    }

    /// The dictionary a schema pack carries, if the pack lists the export.
    pub fn from_pack(pack: &crate::schema::SchemaPack) -> Result<Option<Self>, DictionaryLoadError> {
        pack.noun_attr_fields().map(Self::read).transpose()
    }

    /// Parse the export: `{ "<hash>": { "name": "…", "f": { "SIZE": { "i": n }, "TYPE": { "i": n }, … } }, … }`.
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        let root: serde_json::Map<String, serde_json::Value> = serde_json::from_str(json)?;
        let mut entries = HashMap::with_capacity(root.len());
        for (key, record) in &root {
            let Ok(hash) = key.parse::<u32>() else { continue };
            let Some(name) = record.get("name").and_then(|v| v.as_str()) else { continue };
            let field = |field: &str| {
                record
                    .get("f")
                    .and_then(|f| f.get(field))
                    .and_then(|f| f.get("i"))
                    .and_then(|i| i.as_i64())
                    .and_then(|i| u32::try_from(i).ok())
            };
            entries.insert(hash, DictionaryEntry { name: name.to_string(), size: field("SIZE"), e3d_type: field("TYPE") });
        }
        let mut dictionary = Self { entries, set_once: HashSet::new(), pseudo: HashSet::new() };
        for name in SET_ONCE {
            dictionary.set_once.insert(e3d_attlib::db1_hash(name));
        }
        for name in PSEUDO {
            dictionary.pseudo.insert(e3d_attlib::db1_hash(name));
        }
        Ok(dictionary)
    }

    /// A dictionary built by hand — for a test, or a project whose export is
    /// not at hand. The exception lists start as [`SET_ONCE`] / [`PSEUDO`].
    pub fn from_entries(entries: impl IntoIterator<Item = (u32, DictionaryEntry)>) -> Self {
        let mut dictionary = Self { entries: entries.into_iter().collect(), set_once: HashSet::new(), pseudo: HashSet::new() };
        for name in SET_ONCE {
            dictionary.set_once.insert(e3d_attlib::db1_hash(name));
        }
        for name in PSEUDO {
            dictionary.pseudo.insert(e3d_attlib::db1_hash(name));
        }
        dictionary
    }

    /// Mark an attribute as set-once (added to [`SET_ONCE`]).
    pub fn add_set_once(&mut self, name: &str) -> &mut Self {
        self.set_once.insert(e3d_attlib::db1_hash(name));
        self
    }

    /// Mark an attribute as pseudo (added to [`PSEUDO`]).
    pub fn add_pseudo(&mut self, name: &str) -> &mut Self {
        self.pseudo.insert(e3d_attlib::db1_hash(name));
        self
    }

    pub fn entry(&self, hash: u32) -> Option<&DictionaryEntry> {
        self.entries.get(&hash)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn is_set_once(&self, hash: u32) -> bool {
        self.set_once.contains(&hash)
    }

    pub fn is_pseudo(&self, hash: u32) -> bool {
        self.pseudo.contains(&hash)
    }

    /// Check a write of `value` to attribute `hash` (spelt `name` for the
    /// message) against the dictionary: the attribute must be in it and not
    /// pseudo; a set-once attribute must be unset (`already_set` says whether
    /// the record holds a value); a text may not exceed `SIZE` characters and
    /// an array may not exceed `SIZE` elements. `Unset` is always allowed for
    /// an attribute the dictionary knows.
    pub fn check(&self, hash: u32, name: &str, value: &DescriptorValue, already_set: bool) -> Result<(), DictionaryError> {
        let name = name.to_string();
        let Some(entry) = self.entries.get(&hash) else {
            return Err(DictionaryError::NotInDictionary { name });
        };
        if self.pseudo.contains(&hash) {
            return Err(DictionaryError::Pseudo { name });
        }
        if matches!(value, DescriptorValue::Unset) {
            return Ok(());
        }
        if self.set_once.contains(&hash) && already_set {
            return Err(DictionaryError::SetOnce { name });
        }
        let Some(max) = entry.size else {
            return Ok(());
        };
        match value {
            DescriptorValue::Text(text) => {
                let chars = text.chars().count();
                if chars > max as usize {
                    return Err(DictionaryError::TextTooLong { name, chars, max });
                }
            }
            DescriptorValue::RealArray(items) => check_count(name, items.len(), max)?,
            DescriptorValue::IntArray(items) => check_count(name, items.len(), max)?,
            DescriptorValue::WordArray(items) => check_count(name, items.len(), max)?,
            DescriptorValue::RefNoArray(items) => check_count(name, items.len(), max)?,
            DescriptorValue::BoolArray(items) => check_count(name, items.len(), max)?,
            DescriptorValue::TextList(items) => check_count(name, items.len(), max)?,
            // A scalar has no count to check; raw words are the caller's
            // business (a type-14 record's stride is not the dictionary's SIZE).
            DescriptorValue::Unset
            | DescriptorValue::Bool(_)
            | DescriptorValue::Int(_)
            | DescriptorValue::Real(_)
            | DescriptorValue::RefNo(_)
            | DescriptorValue::Word { .. }
            | DescriptorValue::RawWords(_) => {}
        }
        Ok(())
    }
}

fn check_count(name: String, count: usize, max: u32) -> Result<(), DictionaryError> {
    if count > max as usize {
        return Err(DictionaryError::TooManyElements { name, count, max });
    }
    Ok(())
}

/// Why the export could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum DictionaryLoadError {
    #[error("{path}: {source}")]
    Io { path: std::path::PathBuf, source: std::io::Error },
    #[error("{path}: {source}")]
    Json { path: std::path::PathBuf, source: serde_json::Error },
}

#[cfg(test)]
mod tests {
    use super::*;
    use e3d_attlib::db1_hash;

    const EXPORT: &str = r#"{
        "604480": {"name": "DESC", "f": {"TYPE": {"i": 4}, "SIZE": {"i": 120}, "CHANGE": {"i": 1, "b": true}}},
        "545713": {"name": "POS", "f": {"TYPE": {"i": 8}, "SIZE": {"i": 3}}},
        "19749198": {"name": "UUIDIA", "f": {"TYPE": {"i": 1}, "SIZE": {"i": 4}}},
        "132795174": {"name": "USRWEI", "f": {"TYPE": {"i": 1}, "SIZE": {"i": 500}}},
        "286553556": {"name": "ROKEYS", "f": {"TYPE": {"i": 11}, "SIZE": {"i": 20}}},
        "810996": {"name": "XLEN", "f": {"TYPE": {"i": 2}}},
        "not-a-hash": {"name": "JUNK", "f": {}}
    }"#;

    #[test]
    fn the_export_is_read_by_hash_with_size_and_type() {
        let dictionary = Dictionary::parse(EXPORT).unwrap();
        assert_eq!(dictionary.len(), 6, "the junk key is skipped");
        assert_eq!(db1_hash("DESC"), 604_480, "the keys are db1 hashes");
        assert_eq!(
            dictionary.entry(db1_hash("DESC")),
            Some(&DictionaryEntry { name: "DESC".into(), size: Some(120), e3d_type: Some(4) })
        );
        assert_eq!(dictionary.entry(db1_hash("XLEN")).and_then(|e| e.size), None, "no SIZE, no limit");
        assert!(dictionary.entry(db1_hash("RATM")).is_none());
        assert!(dictionary.is_set_once(db1_hash("UUIDIA")));
        assert!(dictionary.is_pseudo(db1_hash("USRWEI")));
    }

    #[test]
    fn the_checks_are_the_gates_rules() {
        let dictionary = Dictionary::parse(EXPORT).unwrap();
        let text = |n: usize| DescriptorValue::Text("x".repeat(n));
        let desc = db1_hash("DESC");
        assert_eq!(dictionary.check(desc, "DESC", &text(120), false), Ok(()));
        assert_eq!(
            dictionary.check(desc, "DESC", &text(121), false),
            Err(DictionaryError::TextTooLong { name: "DESC".into(), chars: 121, max: 120 })
        );
        // Characters, not bytes: 120 CJK characters are 360 UTF-8 bytes and pass.
        assert_eq!(dictionary.check(desc, "DESC", &DescriptorValue::Text("堆".repeat(120)), false), Ok(()));
        assert_eq!(
            dictionary.check(db1_hash("RATM"), "RATM", &DescriptorValue::Real(1.0), false),
            Err(DictionaryError::NotInDictionary { name: "RATM".into() })
        );
        assert_eq!(
            dictionary.check(db1_hash("POS"), "POS", &DescriptorValue::RealArray(vec![1.0; 4]), false),
            Err(DictionaryError::TooManyElements { name: "POS".into(), count: 4, max: 3 })
        );
        assert_eq!(dictionary.check(db1_hash("POS"), "POS", &DescriptorValue::RealArray(vec![1.0; 3]), true), Ok(()));
        let uuidia = db1_hash("UUIDIA");
        assert_eq!(dictionary.check(uuidia, "UUIDIA", &DescriptorValue::IntArray(vec![1, 2, 3, 4]), false), Ok(()));
        assert_eq!(
            dictionary.check(uuidia, "UUIDIA", &DescriptorValue::IntArray(vec![1, 2, 3, 4]), true),
            Err(DictionaryError::SetOnce { name: "UUIDIA".into() })
        );
        assert_eq!(dictionary.check(uuidia, "UUIDIA", &DescriptorValue::Unset, true), Ok(()), "unsetting is not setting");
        assert_eq!(
            dictionary.check(db1_hash("USRWEI"), "USRWEI", &DescriptorValue::Int(1), false),
            Err(DictionaryError::Pseudo { name: "USRWEI".into() })
        );
        assert_eq!(
            dictionary.check(db1_hash("ROKEYS"), "ROKEYS", &DescriptorValue::TextList(vec!["k".into(); 21]), false),
            Err(DictionaryError::TooManyElements { name: "ROKEYS".into(), count: 21, max: 20 })
        );
        assert_eq!(dictionary.check(db1_hash("XLEN"), "XLEN", &DescriptorValue::Real(1e9), false), Ok(()), "a scalar without SIZE");

        let mut extended = dictionary.clone();
        extended.add_set_once("DESC").add_pseudo("POS");
        assert_eq!(extended.check(desc, "DESC", &text(3), true), Err(DictionaryError::SetOnce { name: "DESC".into() }));
        assert_eq!(
            extended.check(db1_hash("POS"), "POS", &DescriptorValue::RealArray(vec![]), false),
            Err(DictionaryError::Pseudo { name: "POS".into() })
        );
    }
}
