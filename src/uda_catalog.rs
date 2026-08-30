//! Project UDA definitions decoded from a Dictionary database.
//!
//! System attributes come from `attlib.dat` and the per-database `*vir.dat`
//! descriptor. Project attributes do not: E3D persists them as `UDA`
//! elements below the Dictionary world. `UKEY` is the attribute hash stored
//! on Design elements, `STUDNA` is the static display name, `DYUDNA` supplies
//! the name for dynamically installed UDAs, and `ELEL` lists applicable nouns.
//!
//! A key can be re-allocated. When it is, the definition keeps the previous
//! hash in `OLDKEY`, and elements written before the re-allocation still carry
//! their value under it — so a lookup that only knows `UKEY` reads them as
//! unset.

use std::collections::HashSet;
use std::path::Path;

use e3d_attlib::AttlibData;

use crate::record::descriptor::{
    AttributeExtractionStatus, DescriptorValue, ElementExtraction, ExtractedAttribute,
};
use crate::record::element::ParsedElement;
use crate::record::explicit::{ExplicitAttribute, ExplicitValue};
use crate::record::template::{EncodedLocation, StorageDescriptor, TemplateProvider};
use crate::{ReadOnlyEngine, RefNo};

pub const DICTIONARY_TEMPLATE_TYPE: u32 = 0x000E_272F;
pub const UDA_NOUN_HASH: u32 = 0x0008_1F4B;

#[derive(Debug, Clone, PartialEq)]
pub struct UdaDefinition {
    pub refno: RefNo,
    pub owner: RefNo,
    /// Runtime key used in the owning element's UDA value tables.
    pub key: u32,
    /// `OLDKEY`: the hash this UDA used before its key was re-allocated.
    /// Elements last written before the re-allocation still store the value
    /// under it. `None` when the key has never moved.
    pub old_key: Option<u32>,
    /// Slash-prefixed Dictionary element name.
    pub element_name: String,
    /// User-facing attribute name, without the `:` used by `Q ATT`.
    pub name: String,
    /// True when the runtime name came from DYUDNA rather than STUDNA.
    pub dynamic_name: bool,
    /// Dictionary UPSEUD flag for pseudo UDA definitions.
    pub is_pseudo: bool,
    pub value_type_hash: u32,
    pub value_type: Option<String>,
    pub length: u32,
    pub applicable_nouns: Vec<u32>,
    pub default_is_defined: bool,
    pub default_mode: u32,
    pub default_text: Option<String>,
    pub default_real: Option<f64>,
    pub default_location: Option<u32>,
    pub default_attribute: Option<u32>,
    pub default_expression: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UdaDefaultResolution {
    Unset,
    Value(DescriptorValue),
    /// A Dictionary default exists, but its text is not valid for UTYP. E3D
    /// consequently reports the UDA as `unset` (for example REAL `TRUE`).
    Invalid {
        raw: String,
        expected_type: String,
    },
}

impl UdaDefinition {
    pub fn applies_to(&self, noun_hash: u32) -> bool {
        self.applicable_nouns.contains(&noun_hash)
    }

    pub fn resolve_default(&self) -> UdaDefaultResolution {
        let Some(raw) = self
            .default_text
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return UdaDefaultResolution::Unset;
        };
        let value_type = self.value_type.as_deref().unwrap_or("");
        let value = match value_type {
            "TEXT" | "TEXTAR" => Some(DescriptorValue::Text(raw.to_owned())),
            // A WORD default is a name, and E3D reports it as one: ATTA's
            // :COMPTYPE defaults to ANY and that is what `Q ATT` prints.
            "WORD" | "WORDAR" => Some(DescriptorValue::Word {
                raw: e3d_attlib::db1_hash(raw),
                text: Some(raw.to_owned()),
            }),
            "LOG" => match raw.to_ascii_uppercase().as_str() {
                "TRUE" => Some(DescriptorValue::Bool(true)),
                "FALSE" => Some(DescriptorValue::Bool(false)),
                _ => None,
            },
            "INT" => raw.parse::<i32>().ok().map(DescriptorValue::Int),
            "REAL" => raw.parse::<f64>().ok().map(DescriptorValue::Real),
            "REF" if raw.eq_ignore_ascii_case("NULREF") => {
                Some(DescriptorValue::RefNo(RefNo::new(0, 0)))
            }
            _ => None,
        };
        value
            .map(UdaDefaultResolution::Value)
            .unwrap_or_else(|| UdaDefaultResolution::Invalid {
                raw: raw.to_owned(),
                expected_type: value_type.to_owned(),
            })
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct UdaCatalog {
    definitions: Vec<UdaDefinition>,
    issues: Vec<UdaDefinitionIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdaDefinitionIssue {
    pub refno: RefNo,
    pub detail: String,
}

impl UdaCatalog {
    /// Read every `UDA` element the Dictionary database's current session
    /// indexes.
    ///
    /// The candidate list used to come from every index page in the file, live
    /// or abandoned, so it carried older copies and RefNos out of trees no
    /// session reaches; each one was reopened and kept only if the parser
    /// confirmed its identity and noun. The session's own tree names each
    /// element once, at the copy that session wrote, so those two filters are
    /// now checks rather than the mechanism.
    pub fn read(
        dictionary_db: impl AsRef<Path>,
        dicvir: impl AsRef<Path>,
        attlib: &AttlibData,
    ) -> Result<Self, String> {
        let mut engine = ReadOnlyEngine::open(dictionary_db)
            .map_err(|error| format!("open Dictionary database: {error}"))?;
        let mut provider = TemplateProvider::open(dicvir, Some(DICTIONARY_TEMPLATE_TYPE))
            .map_err(|error| format!("open Dictionary descriptors: {error}"))?;
        let mut candidates = engine
            .indexed_refnos()
            .map_err(|error| format!("read the Dictionary index: {error}"))?
            .into_iter()
            .map(|(refno, _, _)| refno)
            .collect::<Vec<_>>();
        candidates.sort_by_key(|refno| (refno.word0, refno.word1));
        candidates.dedup();

        let mut seen = HashSet::new();
        let mut definitions = Vec::new();
        let mut issues = Vec::new();
        for refno in candidates {
            let view = match engine.find_element(refno) {
                Ok(Some(view)) => view,
                Ok(None) => continue,
                Err(error) => {
                    issues.push(UdaDefinitionIssue {
                        refno,
                        detail: format!("read Dictionary element: {error}"),
                    });
                    continue;
                }
            };
            let parsed = match ParsedElement::parse(&view.raw_bytes) {
                Ok(parsed) => parsed,
                Err(error) => {
                    issues.push(UdaDefinitionIssue {
                        refno,
                        detail: format!("parse indexed Dictionary element: {error}"),
                    });
                    continue;
                }
            };
            if parsed.refno != refno || parsed.noun_hash != UDA_NOUN_HASH || !seen.insert(refno) {
                continue;
            }
            let extraction =
                match engine.extract_element_with_descriptors(refno, attlib, &mut provider) {
                    Ok(extraction) => extraction,
                    Err(error) => {
                        issues.push(UdaDefinitionIssue {
                            refno,
                            detail: format!("decode UDA definition: {error}"),
                        });
                        continue;
                    }
                };
            match parse_definition(&extraction) {
                Ok(definition) => {
                    if definition.name.is_empty() {
                        issues.push(UdaDefinitionIssue {
                            refno,
                            detail: format!(
                                "UDA definition has empty STUDNA; UKEY {} and element name {:?} require runtime alias resolution",
                                definition.key, definition.element_name
                            ),
                        });
                    }
                    definitions.push(definition);
                }
                Err(detail) => issues.push(UdaDefinitionIssue { refno, detail }),
            }
        }
        Ok(Self {
            definitions,
            issues,
        })
    }

    pub fn definitions(&self) -> &[UdaDefinition] {
        &self.definitions
    }

    /// Merge another Dictionary catalog in MDB search order. The first
    /// definition for a UKEY remains authoritative, matching the runtime's
    /// ordered Dictionary lookup while retaining every scan issue.
    pub fn merge_in_order(&mut self, other: Self) {
        let mut known_keys = self
            .definitions
            .iter()
            .map(|definition| definition.key)
            .collect::<HashSet<_>>();
        self.definitions.extend(
            other
                .definitions
                .into_iter()
                .filter(|definition| known_keys.insert(definition.key)),
        );
        self.issues.extend(other.issues);
    }

    /// Indexed UDA-shaped records that could not form a complete definition.
    /// This commonly includes deleted/tombstone versions; it is exposed so a
    /// damaged live definition can never disappear silently.
    pub fn issues(&self) -> &[UdaDefinitionIssue] {
        &self.issues
    }

    pub fn applicable_to(&self, noun_hash: u32) -> Vec<&UdaDefinition> {
        self.definitions
            .iter()
            .filter(|definition| definition.applies_to(noun_hash))
            .collect()
    }

    /// The definition a stored attribute hash belongs to. A current `UKEY`
    /// always wins over someone else's `OLDKEY`.
    pub fn by_key(&self, key: u32) -> Option<&UdaDefinition> {
        self.definitions
            .iter()
            .find(|definition| definition.key == key)
            .or_else(|| {
                self.definitions
                    .iter()
                    .find(|definition| definition.old_key == Some(key))
            })
    }

    pub fn by_name(&self, name: &str) -> Vec<&UdaDefinition> {
        self.definitions
            .iter()
            .filter(|definition| definition.name.eq_ignore_ascii_case(name))
            .collect()
    }

    /// Append Dictionary UDAs that are valid for this element's noun. Persisted
    /// explicit values win; otherwise the type-checked Dictionary default is
    /// materialised exactly as E3D reports it (including logical `unset`).
    pub fn materialize_for_element(
        &self,
        extraction: &mut ElementExtraction,
        explicit_attributes: &[ExplicitAttribute],
    ) -> usize {
        let mut seen_keys = HashSet::new();
        let mut added = 0;
        for definition in self.definitions.iter().filter(|definition| {
            !definition.name.is_empty() && definition.applies_to(extraction.noun_hash)
        }) {
            if !seen_keys.insert(definition.key) {
                continue;
            }
            let explicit = explicit_attributes
                .iter()
                .find(|attribute| attribute.hash == definition.key)
                .or_else(|| {
                    definition.old_key.and_then(|old| {
                        explicit_attributes
                            .iter()
                            .find(|attribute| attribute.hash == old)
                    })
                });
            let (status, value) = match explicit {
                Some(attribute) => (
                    AttributeExtractionStatus::DecodedExplicit,
                    explicit_value(&attribute.value),
                ),
                None => {
                    let value = match definition.resolve_default() {
                        UdaDefaultResolution::Value(value) => value,
                        UdaDefaultResolution::Unset | UdaDefaultResolution::Invalid { .. } => {
                            DescriptorValue::Unset
                        }
                    };
                    (AttributeExtractionStatus::CatalogDefault, value)
                }
            };
            extraction.attributes.push(ExtractedAttribute {
                hash: definition.key,
                name: format!(":{}", definition.name),
                storage_type_code: 0,
                semantic_type_code: None,
                semantic_size: Some(definition.length),
                layout: extraction.layout,
                storage: StorageDescriptor {
                    storage_words: 0,
                    location: EncodedLocation { raw: 0 },
                    flags: 0,
                },
                status,
                value: Some(value),
            });
            added += 1;
        }
        added
    }
}

fn explicit_value(value: &ExplicitValue) -> DescriptorValue {
    match value {
        ExplicitValue::Integer(value) => DescriptorValue::Int(*value),
        ExplicitValue::Real(value) => DescriptorValue::Real(*value),
        ExplicitValue::Bool(value) => DescriptorValue::Bool(*value),
        ExplicitValue::Text(value) => DescriptorValue::Text(value.clone()),
        ExplicitValue::RefNo(value) => DescriptorValue::RefNo(*value),
        ExplicitValue::IntArray(value) => DescriptorValue::IntArray(value.clone()),
        ExplicitValue::RealArray(value) => DescriptorValue::RealArray(value.clone()),
        ExplicitValue::RefNoArray(value) => DescriptorValue::RefNoArray(value.clone()),
        ExplicitValue::RawWords(value) => DescriptorValue::RawWords(value.clone()),
    }
}

fn parse_definition(extraction: &ElementExtraction) -> Result<UdaDefinition, String> {
    let required_word = |name: &str| {
        word(extraction, name)
            .ok_or_else(|| format!("UDA {} has no decoded {name}", extraction.refno))
    };
    let static_name = packed_text(extraction, "STUDNA").unwrap_or_default();
    let dynamic_name = static_name.is_empty();
    let name = if dynamic_name {
        text(extraction, "DYUDNA").unwrap_or_default()
    } else {
        static_name
    };
    Ok(UdaDefinition {
        refno: extraction.refno,
        owner: extraction.owner,
        key: required_word("UKEY")?,
        old_key: word(extraction, "OLDKEY").filter(|hash| *hash != 0),
        element_name: text(extraction, "NAME").unwrap_or_default(),
        name,
        dynamic_name,
        is_pseudo: bool_value(extraction, "UPSEUD").unwrap_or(false),
        value_type_hash: required_word("UTYP")?,
        value_type: word_text(extraction, "UTYP"),
        length: required_word("ULEN")?,
        applicable_nouns: word_array(extraction, "ELEL").unwrap_or_default(),
        default_is_defined: bool_value(extraction, "DTYP").unwrap_or(false),
        default_mode: word(extraction, "DMOD").unwrap_or(0),
        default_text: text(extraction, "DFLT"),
        default_real: real(extraction, "DREA"),
        default_location: word(extraction, "DFTLOC"),
        default_attribute: word(extraction, "DFTATT"),
        default_expression: text(extraction, "DFTEXP"),
    })
}

fn attribute<'a>(extraction: &'a ElementExtraction, name: &str) -> Option<&'a DescriptorValue> {
    extraction
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .and_then(|attribute| attribute.value.as_ref())
}

fn word(extraction: &ElementExtraction, name: &str) -> Option<u32> {
    match attribute(extraction, name)? {
        DescriptorValue::Word { raw, .. } => Some(*raw),
        DescriptorValue::Int(value) => Some(*value as u32),
        _ => None,
    }
}

fn word_text(extraction: &ElementExtraction, name: &str) -> Option<String> {
    match attribute(extraction, name)? {
        DescriptorValue::Word { text, .. } => text.clone(),
        _ => None,
    }
}

fn bool_value(extraction: &ElementExtraction, name: &str) -> Option<bool> {
    match attribute(extraction, name)? {
        DescriptorValue::Bool(value) => Some(*value),
        _ => None,
    }
}

fn real(extraction: &ElementExtraction, name: &str) -> Option<f64> {
    match attribute(extraction, name)? {
        DescriptorValue::Real(value) => Some(*value),
        _ => None,
    }
}

fn text(extraction: &ElementExtraction, name: &str) -> Option<String> {
    match attribute(extraction, name)? {
        DescriptorValue::Text(value) => Some(value.clone()),
        _ => None,
    }
}

fn word_array(extraction: &ElementExtraction, name: &str) -> Option<Vec<u32>> {
    match attribute(extraction, name)? {
        DescriptorValue::WordArray(values) | DescriptorValue::RawWords(values) => {
            Some(values.clone())
        }
        DescriptorValue::IntArray(values) => {
            Some(values.iter().map(|value| *value as u32).collect())
        }
        _ => None,
    }
}

/// `STUDNA` is character storage, so the descriptor path already hands back a
/// string. The word-array reading stays for values that reach here through the
/// explicit stream, where the count and the characters arrive as bare words.
fn packed_text(extraction: &ElementExtraction, name: &str) -> Option<String> {
    if let Some(value) = text(extraction, name) {
        return Some(value);
    }
    let words = word_array(extraction, name)?;
    let length = *words.first()? as usize;
    let mut bytes = Vec::with_capacity(words.len().saturating_sub(1) * 4);
    for word in words.iter().skip(1) {
        bytes.extend_from_slice(&word.to_be_bytes());
    }
    bytes.truncate(length.min(bytes.len()));
    Some(crate::record::text::decode(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::descriptor::{AttributeExtractionStatus, ExtractedAttribute};
    use crate::record::template::{EncodedLocation, StorageDescriptor, StorageLayout};

    fn extracted(name: &str, value: DescriptorValue) -> ExtractedAttribute {
        ExtractedAttribute {
            hash: 0,
            name: name.into(),
            storage_type_code: 0,
            semantic_type_code: None,
            semantic_size: None,
            layout: StorageLayout::Compact,
            storage: StorageDescriptor {
                storage_words: 0,
                location: EncodedLocation { raw: 0 },
                flags: 0,
            },
            status: AttributeExtractionStatus::Decoded,
            value: Some(value),
        }
    }

    #[test]
    fn decodes_packed_studna_and_definition_fields() {
        let extraction = ElementExtraction {
            refno: RefNo::new(13292, 23),
            owner: RefNo::new(13292, 2),
            noun_hash: UDA_NOUN_HASH,
            noun_name: Some("UDA".into()),
            implicit_word_count: 0,
            format_word_10: 0,
            layout: StorageLayout::Compact,
            attributes: vec![
                extracted(
                    "UKEY",
                    DescriptorValue::Word {
                        raw: 398_835_873,
                        text: None,
                    },
                ),
                extracted("NAME", DescriptorValue::Text("/H-AIRQUANTITY".into())),
                extracted(
                    "STUDNA",
                    DescriptorValue::RawWords(vec![
                        13, 0x482D4149, 0x52515541, 0x4E544954, 0x59000000,
                    ]),
                ),
                extracted(
                    "UTYP",
                    DescriptorValue::Word {
                        raw: 768_519,
                        text: Some("REAL".into()),
                    },
                ),
                extracted("ULEN", DescriptorValue::Word { raw: 1, text: None }),
                extracted("ELEL", DescriptorValue::WordArray(vec![808_220])),
                extracted("DTYP", DescriptorValue::Bool(true)),
                extracted("DMOD", DescriptorValue::Word { raw: 0, text: None }),
                extracted("DREA", DescriptorValue::Real(0.0)),
                extracted("DFLT", DescriptorValue::Text("0".into())),
            ],
        };
        let definition = parse_definition(&extraction).unwrap();
        assert_eq!(definition.name, "H-AIRQUANTITY");
        assert_eq!(definition.key, 398_835_873);
        assert_eq!(definition.value_type.as_deref(), Some("REAL"));
        assert!(definition.applies_to(808_220));
        assert!(definition.default_is_defined);
        assert_eq!(definition.default_real, Some(0.0));
        assert_eq!(
            definition.resolve_default(),
            UdaDefaultResolution::Value(DescriptorValue::Real(0.0))
        );
    }

    #[test]
    fn dynamic_uda_name_falls_back_to_dyudna() {
        let extraction = ElementExtraction {
            refno: RefNo::new(15_198, 527),
            owner: RefNo::new(15_198, 525),
            noun_hash: UDA_NOUN_HASH,
            noun_name: Some("UDA".into()),
            implicit_word_count: 0,
            format_word_10: 0,
            layout: StorageLayout::Wide,
            attributes: vec![
                extracted(
                    "UKEY",
                    DescriptorValue::Word {
                        raw: 42,
                        text: None,
                    },
                ),
                extracted("NAME", DescriptorValue::Text("/descriptive-name".into())),
                extracted("STUDNA", DescriptorValue::RawWords(vec![0; 6])),
                extracted("DYUDNA", DescriptorValue::Text("PFILoose".into())),
                extracted("UPSEUD", DescriptorValue::Bool(true)),
                extracted(
                    "UTYP",
                    DescriptorValue::Word {
                        raw: 942_752,
                        text: Some("TEXT".into()),
                    },
                ),
                extracted("ULEN", DescriptorValue::Word { raw: 5, text: None }),
                extracted("ELEL", DescriptorValue::WordArray(vec![808_220])),
            ],
        };

        let definition = parse_definition(&extraction).unwrap();
        assert_eq!(definition.name, "PFILoose");
        assert!(definition.dynamic_name);
        assert!(definition.is_pseudo);
    }

    /// `/FQ56/UDASUP/COMPTYPE` is a WORD whose Dictionary default is the name
    /// ANY, and `Q ATT` on an ATTA prints ANY. Refusing WORD as a type turned
    /// that into `unset`.
    #[test]
    fn a_word_default_resolves_to_the_name_it_holds() {
        let definition = UdaDefinition {
            refno: RefNo::new(13292, 75),
            owner: RefNo::new(13292, 59),
            key: 451_053_852,
            old_key: None,
            element_name: "/FQ56/UDASUP/COMPTYPE".into(),
            name: "COMPTYPE".into(),
            dynamic_name: false,
            is_pseudo: false,
            value_type_hash: 623_723,
            value_type: Some("WORD".into()),
            length: 4,
            applicable_nouns: vec![566_245],
            default_is_defined: true,
            default_mode: 0,
            default_text: Some("ANY".into()),
            default_real: None,
            default_location: None,
            default_attribute: None,
            default_expression: None,
        };
        assert_eq!(
            definition.resolve_default(),
            UdaDefaultResolution::Value(DescriptorValue::Word {
                raw: e3d_attlib::db1_hash("ANY"),
                text: Some("ANY".into()),
            })
        );
    }

    #[test]
    fn merged_catalog_keeps_first_dictionary_definition_for_duplicate_ukey() {
        let definition = |refno: RefNo, name: &str| UdaDefinition {
            refno,
            owner: RefNo::new(0, 0),
            key: 42,
            old_key: None,
            element_name: String::new(),
            name: name.into(),
            dynamic_name: false,
            is_pseudo: false,
            value_type_hash: 0,
            value_type: Some("TEXT".into()),
            length: 120,
            applicable_nouns: vec![808_220],
            default_is_defined: false,
            default_mode: 0,
            default_text: None,
            default_real: None,
            default_location: None,
            default_attribute: None,
            default_expression: None,
        };
        let mut merged = UdaCatalog {
            definitions: vec![definition(RefNo::new(1, 1), "FIRST")],
            issues: Vec::new(),
        };
        merged.merge_in_order(UdaCatalog {
            definitions: vec![definition(RefNo::new(2, 2), "SECOND")],
            issues: vec![UdaDefinitionIssue {
                refno: RefNo::new(2, 3),
                detail: "damaged candidate".into(),
            }],
        });

        assert_eq!(merged.definitions.len(), 1);
        assert_eq!(merged.definitions[0].name, "FIRST");
        assert_eq!(merged.issues.len(), 1);
    }

    #[test]
    fn materialisation_does_not_emit_an_unnamed_colon_attribute() {
        let catalog = UdaCatalog {
            definitions: vec![UdaDefinition {
                refno: RefNo::new(1, 1),
                owner: RefNo::new(1, 0),
                key: 42,
                old_key: None,
                element_name: "/descriptive-name-only".into(),
                name: String::new(),
                dynamic_name: false,
                is_pseudo: false,
                value_type_hash: 0,
                value_type: Some("TEXT".into()),
                length: 120,
                applicable_nouns: vec![808_220],
                default_is_defined: false,
                default_mode: 0,
                default_text: None,
                default_real: None,
                default_location: None,
                default_attribute: None,
                default_expression: None,
            }],
            issues: Vec::new(),
        };
        let mut extraction = ElementExtraction {
            refno: RefNo::new(2, 2),
            owner: RefNo::new(2, 1),
            noun_hash: 808_220,
            noun_name: Some("BRAN".into()),
            implicit_word_count: 0,
            format_word_10: 0,
            layout: StorageLayout::Compact,
            attributes: Vec::new(),
        };

        assert_eq!(catalog.materialize_for_element(&mut extraction, &[]), 0);
        assert!(extraction.attributes.is_empty());
    }
}
