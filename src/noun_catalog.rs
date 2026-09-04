//! Optional live-E3D noun/attribute catalog.
//!
//! The catalog complements, but never replaces, the authoritative physical
//! descriptors loaded from `desvir.dat`: `noun_layout.json` supplies the
//! ordered logical attribute surface and `noun_attr_fields.json` preserves all
//! exported `DbAttributeField` values. Physical offsets, sizes and storage
//! layouts continue to come exclusively from [`NounTemplate`].

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::Path;

use serde_json::{Map, Value};

use crate::record::descriptor::{
    AttributeExtractionStatus, DescriptorValue, ElementExtraction, ExtractedAttribute, StorageShape,
};
use crate::record::template::{NounTemplate, TemplateAttr};
use crate::refno::RefNo;

#[derive(Debug, Clone, PartialEq)]
pub struct CatalogAttribute {
    pub name: String,
    pub hash: u32,
    pub type_code: u32,
    pub type_name: String,
    pub is_array: bool,
    pub max_size: u32,
    pub true_size: u32,
    pub is_uda: bool,
    pub is_pseudo: bool,
    pub hidden: bool,
    pub no_claim: bool,
    pub default_probe: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CatalogNoun {
    pub noun: String,
    pub short_name: String,
    pub noun_hash: u32,
    pub base: String,
    pub hard: String,
    pub is_udet: bool,
    pub is_pseudo: bool,
    pub is_world: bool,
    pub is_primary: bool,
    pub visible: bool,
    pub db_types: Vec<u32>,
    /// Preserved in exactly the order emitted by live E3D.
    pub attrs: Vec<CatalogAttribute>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttributeFieldRecord {
    pub name: String,
    /// Every exported `DbAttributeField` value, retained without narrowing the
    /// live schema. Callers can interpret individual fields such as `DCHC`.
    pub fields: Map<String, Value>,
}

#[derive(Debug, Clone, Default)]
pub struct NounCatalog {
    nouns: Vec<CatalogNoun>,
    noun_indices: HashMap<u32, usize>,
    fields: HashMap<u32, AttributeFieldRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconciledSource {
    CatalogAndDescriptor,
    CatalogOnly,
    DescriptorOnly,
}

#[derive(Debug)]
pub struct ReconciledAttribute<'a> {
    pub source: ReconciledSource,
    pub catalog_ordinal: Option<usize>,
    pub descriptor_ordinal: Option<usize>,
    pub catalog: Option<&'a CatalogAttribute>,
    pub descriptor: Option<&'a TemplateAttr>,
    pub fields: Option<&'a AttributeFieldRecord>,
}

#[derive(Debug)]
pub struct ReconciledNoun<'a> {
    pub noun: Option<&'a CatalogNoun>,
    pub attributes: Vec<ReconciledAttribute<'a>>,
}

impl NounCatalog {
    pub fn parse(layout_json: &str, fields_json: &str) -> Result<Self, CatalogError> {
        let layout: Value = serde_json::from_str(layout_json)?;
        let fields: Value = serde_json::from_str(fields_json)?;
        let layout = layout
            .as_array()
            .ok_or(CatalogError::ExpectedArray("noun layout"))?;
        let fields = fields
            .as_object()
            .ok_or(CatalogError::ExpectedObject("attribute fields"))?;

        let mut parsed_fields = HashMap::with_capacity(fields.len());
        for (hash_text, value) in fields {
            let hash = hash_text
                .parse::<u32>()
                .map_err(|_| CatalogError::InvalidFieldHash(hash_text.clone()))?;
            let object = object(value, "attribute field record")?;
            let name = string(object, "name", "attribute field record")?.to_owned();
            let values = object
                .get("f")
                .and_then(Value::as_object)
                .ok_or(CatalogError::MissingField("attribute field record.f"))?
                .clone();
            if parsed_fields
                .insert(
                    hash,
                    AttributeFieldRecord {
                        name,
                        fields: values,
                    },
                )
                .is_some()
            {
                return Err(CatalogError::DuplicateFieldHash(hash));
            }
        }

        let mut nouns = Vec::with_capacity(layout.len());
        let mut noun_indices = HashMap::with_capacity(layout.len());
        for value in layout {
            let noun = parse_noun(value)?;
            let noun_hash = noun.noun_hash;
            if noun_indices.insert(noun_hash, nouns.len()).is_some() {
                return Err(CatalogError::DuplicateNounHash(noun_hash));
            }
            nouns.push(noun);
        }

        for noun in &nouns {
            for attr in &noun.attrs {
                if !parsed_fields.contains_key(&attr.hash) {
                    return Err(CatalogError::MissingAttributeFields {
                        noun: noun.noun.clone(),
                        attribute: attr.name.clone(),
                        hash: attr.hash,
                    });
                }
            }
        }

        Ok(Self {
            nouns,
            noun_indices,
            fields: parsed_fields,
        })
    }

    pub fn read(
        layout_path: impl AsRef<Path>,
        fields_path: impl AsRef<Path>,
    ) -> Result<Self, CatalogError> {
        let layout_path = layout_path.as_ref();
        let fields_path = fields_path.as_ref();
        let layout = fs::read_to_string(layout_path).map_err(|source| CatalogError::Read {
            path: layout_path.display().to_string(),
            source,
        })?;
        let fields = fs::read_to_string(fields_path).map_err(|source| CatalogError::Read {
            path: fields_path.display().to_string(),
            source,
        })?;
        Self::parse(&layout, &fields)
    }

    pub fn noun_count(&self) -> usize {
        self.nouns.len()
    }

    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    pub fn declaration_count(&self) -> usize {
        self.nouns.iter().map(|noun| noun.attrs.len()).sum()
    }

    pub fn attribute_field_value_count(&self) -> usize {
        self.fields.values().map(|record| record.fields.len()).sum()
    }

    pub fn noun(&self, noun_hash: u32) -> Option<&CatalogNoun> {
        self.noun_indices
            .get(&noun_hash)
            .and_then(|index| self.nouns.get(*index))
    }

    pub fn attribute_fields(&self, hash: u32) -> Option<&AttributeFieldRecord> {
        self.fields.get(&hash)
    }

    /// The heading `q att` prints for one attribute.
    ///
    /// [`AttributeMetadataCatalog`] recovers a name by walking the `attlib.dat`
    /// metadata record and taking the first token without whitespace, which is
    /// the heading for most attributes but not all: `BANG` yields
    /// `BangleDoubleConverter` and `POS` yields `PositonEditor`, both editor
    /// labels rather than headings. `QTXT` is the exported query text itself,
    /// so it needs no recovery — it says `Bangle`, which is what E3D printed.
    ///
    /// No two of the 4271 exported attributes share a `QTXT`, so a heading
    /// found here identifies one attribute.
    ///
    /// [`AttributeMetadataCatalog`]: crate::attlib_metadata::AttributeMetadataCatalog
    pub fn query_heading(&self, hash: u32) -> Option<&str> {
        self.fields
            .get(&hash)?
            .fields
            .get("QTXT")?
            .get("s")?
            .as_str()
    }

    /// Materialise defaults for descriptor rows that have no persisted
    /// implicit or explicit value.
    ///
    /// IDA's `DB_Element::atdefault` first tries the persisted attribute and
    /// then resolves a type-specific default. Array absence remains `unset`;
    /// scalar BOOL/DOUBLE/ELEMENT values materialise as false/zero/Nulref;
    /// strings, words, directions and datetimes remain the distinct E3D
    /// `unset` value. Pseudo attributes are deliberately left unresolved for
    /// the derived-attribute evaluator.
    ///
    /// The default is the type's and not the attribute's: what the catalog
    /// records as a given attribute's own default — `QTYPE ANY`, `CLUTCH
    /// AUTO`, `WELDTY MANU` — is not what `q att` prints for an element that
    /// stores nothing. All six such rows in the 128-element TTY sample print
    /// `unset`, and no row in it is answered by a non-zero declared default.
    pub fn apply_static_defaults(&self, extraction: &mut ElementExtraction) -> usize {
        let Some(noun) = self.noun(extraction.noun_hash) else {
            return 0;
        };
        let mut catalog_by_hash: HashMap<u32, VecDeque<&CatalogAttribute>> = HashMap::new();
        for attribute in &noun.attrs {
            catalog_by_hash
                .entry(attribute.hash)
                .or_default()
                .push_back(attribute);
        }

        let mut resolved = 0;
        for extracted in &mut extraction.attributes {
            let Some(catalog) = catalog_by_hash
                .get_mut(&extracted.hash)
                .and_then(VecDeque::pop_front)
            else {
                continue;
            };
            if extracted.status != AttributeExtractionStatus::NonImplicit || catalog.is_pseudo {
                continue;
            }
            let Some(value) = type_zero(catalog, extracted.storage_type_code) else {
                continue;
            };
            extracted.value = Some(value);
            extracted.status = AttributeExtractionStatus::CatalogDefault;
            resolved += 1;
        }
        resolved
    }

    /// The value a pseudo attribute reports when nothing else has claimed one.
    ///
    /// A pseudo attribute has no storage — E3D computes it on demand — which
    /// is why [`Self::apply_static_defaults`] leaves it alone. But where the
    /// computation has nothing to work from it does not decline: it hands back
    /// the type's zero, and `q att` prints `unset`, `Nulref`, `0` or `false`.
    /// That is what E3D printed for 117 of the 121 unresolved pseudo
    /// attributes in the 128-element TTY sample.
    ///
    /// This deliberately runs after [`Self::apply_runtime_derived`], so a
    /// pseudo attribute whose real rule is known has already answered and is
    /// left untouched. The handful the sample disagrees with — `Bthkness`,
    /// whose expression is a PML formula held elsewhere, and `SkeyScale`,
    /// which E3D reports as 100 — are not hidden by this: the row is still
    /// compared against TTY and now reads as a disagreement instead of as
    /// nothing at all.
    pub fn apply_pseudo_defaults(&self, extraction: &mut ElementExtraction) -> usize {
        let Some(noun) = self.noun(extraction.noun_hash) else {
            return 0;
        };
        let mut catalog_by_hash: HashMap<u32, VecDeque<&CatalogAttribute>> = HashMap::new();
        for attribute in &noun.attrs {
            catalog_by_hash
                .entry(attribute.hash)
                .or_default()
                .push_back(attribute);
        }

        let mut resolved = 0;
        for extracted in &mut extraction.attributes {
            let Some(catalog) = catalog_by_hash
                .get_mut(&extracted.hash)
                .and_then(VecDeque::pop_front)
            else {
                continue;
            };
            if extracted.status != AttributeExtractionStatus::NonImplicit || !catalog.is_pseudo {
                continue;
            }
            let Some(value) = type_zero(catalog, extracted.storage_type_code) else {
                continue;
            };
            extracted.value = Some(value);
            extracted.status = AttributeExtractionStatus::CatalogDefault;
            resolved += 1;
        }
        resolved
    }

    /// Evaluate pseudo attributes whose E3D runtime rule is known locally.
    ///
    /// IDA shows that `DB_Element::getAtt(double&)` initializes the result to
    /// zero before dispatching a pseudo getter. The BRAN head/tail duct
    /// dimensions are pseudo attributes. For a circular end, identified by a
    /// positive persisted bore, the duct width and height therefore resolve to
    /// zero. A zero or absent bore is deliberately left unresolved because it
    /// may denote a rectangular duct whose dimensions require the Design
    /// plugger.
    ///
    /// Core3D's Design text plugger (`DESPTX`) dispatches `INCO` to `QROFCN`
    /// and `RLSTOR` to `QROSRU`. When their backing integer arrays are absent,
    /// `QROFCN` reports no text while `QROSRU` emits the literal `unset`. An
    /// explicit backing value has already changed the extraction status before
    /// this evaluator runs, so these defaults only claim genuinely absent data.
    pub fn apply_runtime_derived(&self, extraction: &mut ElementExtraction) -> usize {
        let Some(noun) = self.noun(extraction.noun_hash) else {
            return 0;
        };
        if noun.short_name == "UDA" {
            let default_unit = text_value(extraction, "UUNI");
            let uda_name = text_value(extraction, "NAME").and_then(|name| {
                name.rsplit('/')
                    .find(|part| !part.is_empty())
                    .map(str::to_owned)
            });
            let mut resolved = 0;
            if let Some(default_unit) = default_unit {
                for extracted in &mut extraction.attributes {
                    if extracted.name == "TUNI"
                        && matches!(
                            extracted.status,
                            AttributeExtractionStatus::NonImplicit
                                | AttributeExtractionStatus::CatalogDefault
                        )
                    {
                        extracted.value = Some(DescriptorValue::Text(default_unit.clone()));
                        extracted.status = AttributeExtractionStatus::RuntimeDerived {
                            evaluator: "uda_default_unit_falls_back_to_dimension".into(),
                        };
                        resolved += 1;
                    }
                }
            }
            if !extraction
                .attributes
                .iter()
                .any(|attribute| attribute.name == "UDNA")
            {
                if let (Some(value), Some(catalog)) = (
                    uda_name,
                    noun.attrs.iter().find(|attribute| attribute.name == "UDNA"),
                ) {
                    let descriptor = TemplateAttr::single_word(catalog.hash);
                    extraction.attributes.push(ExtractedAttribute {
                        hash: catalog.hash,
                        name: catalog.name.clone(),
                        storage_type_code: descriptor.storage_type_code,
                        semantic_type_code: Some(catalog.type_code),
                        semantic_size: Some(catalog.max_size),
                        layout: extraction.layout,
                        storage: descriptor.storage(extraction.layout),
                        status: AttributeExtractionStatus::RuntimeDerived {
                            evaluator: "uda_name_from_element_name".into(),
                        },
                        value: Some(DescriptorValue::Text(value)),
                    });
                    resolved += 1;
                }
            }
            return resolved;
        }
        if noun.short_name != "BRAN" {
            return 0;
        }

        let circular_head = positive_real(extraction, "HBOR");
        let circular_tail = positive_real(extraction, "TBOR");
        let mut resolved = 0;
        for extracted in &mut extraction.attributes {
            if extracted.status != AttributeExtractionStatus::NonImplicit {
                continue;
            }
            let circular_end = match extracted.name.as_str() {
                "HEADW" | "HEADH" => circular_head,
                "TAILW" | "TAILH" => circular_tail,
                _ => false,
            };
            if circular_end {
                extracted.value = Some(DescriptorValue::Real(0.0));
                extracted.status = AttributeExtractionStatus::RuntimeDerived {
                    evaluator: "bran_circular_end_dimensions".into(),
                };
                resolved += 1;
                continue;
            }

            let (value, evaluator) = match extracted.name.as_str() {
                "INCO" => (
                    DescriptorValue::Text(String::new()),
                    "design_qrofcn_absent_internal_points",
                ),
                "RLST" | "RLSTOR" => (DescriptorValue::Unset, "design_qrosru_absent_saved_rules"),
                _ => continue,
            };
            extracted.value = Some(value);
            extracted.status = AttributeExtractionStatus::RuntimeDerived {
                evaluator: evaluator.into(),
            };
            resolved += 1;
        }
        resolved
    }

    /// Merge logical order/metadata with physical descriptors by hash.
    ///
    /// Catalog rows remain in live-E3D order. Descriptors absent from the
    /// catalog are appended in physical order. Duplicate hashes are paired in
    /// occurrence order rather than collapsed.
    pub fn reconcile<'a>(&'a self, template: &'a NounTemplate) -> ReconciledNoun<'a> {
        let noun = self.noun(template.noun_hash);
        let mut by_hash: HashMap<u32, VecDeque<(usize, &TemplateAttr)>> = HashMap::new();
        for (ordinal, descriptor) in template.attrs.iter().enumerate() {
            by_hash
                .entry(descriptor.hash)
                .or_default()
                .push_back((ordinal, descriptor));
        }

        let mut attributes = Vec::new();
        if let Some(noun) = noun {
            for (ordinal, catalog) in noun.attrs.iter().enumerate() {
                let physical = by_hash.get_mut(&catalog.hash).and_then(VecDeque::pop_front);
                attributes.push(ReconciledAttribute {
                    source: if physical.is_some() {
                        ReconciledSource::CatalogAndDescriptor
                    } else {
                        ReconciledSource::CatalogOnly
                    },
                    catalog_ordinal: Some(ordinal),
                    descriptor_ordinal: physical.map(|(ordinal, _)| ordinal),
                    catalog: Some(catalog),
                    descriptor: physical.map(|(_, descriptor)| descriptor),
                    fields: self.attribute_fields(catalog.hash),
                });
            }
        }

        let mut remaining = by_hash
            .into_values()
            .flatten()
            .collect::<Vec<(usize, &TemplateAttr)>>();
        remaining.sort_by_key(|(ordinal, _)| *ordinal);
        for (ordinal, descriptor) in remaining {
            attributes.push(ReconciledAttribute {
                source: ReconciledSource::DescriptorOnly,
                catalog_ordinal: None,
                descriptor_ordinal: Some(ordinal),
                catalog: None,
                descriptor: Some(descriptor),
                fields: self.attribute_fields(descriptor.hash),
            });
        }

        ReconciledNoun { noun, attributes }
    }
}

fn positive_real(extraction: &ElementExtraction, name: &str) -> bool {
    extraction
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .is_some_and(|attribute| is_positive_real(attribute.value.as_ref()))
}

fn text_value(extraction: &ElementExtraction, name: &str) -> Option<String> {
    extraction
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .and_then(|attribute| match attribute.value.as_ref() {
            Some(DescriptorValue::Text(value)) => Some(value.clone()),
            _ => None,
        })
}

fn is_positive_real(value: Option<&DescriptorValue>) -> bool {
    matches!(value, Some(DescriptorValue::Real(value)) if *value > 0.0)
}

/// The type's zero, with the descriptor settling any disagreement over what
/// the type is.
///
/// `DETA` is declared `BOOL` by the live catalog export, but every template
/// that carries it gives it a two-word reference slot, and `q att` prints
/// `Nulref` for a `GPART` that stores nothing — not `false`. The descriptor is
/// the one that describes the bytes, so where it states a shape the catalog's
/// type name contradicts, the shape wins.
fn type_zero(attribute: &CatalogAttribute, storage_type_code: u32) -> Option<DescriptorValue> {
    if !attribute.is_array
        && StorageShape::of(storage_type_code) == Some(StorageShape::Reference)
        && attribute.type_name != "ELEMENT"
    {
        return Some(DescriptorValue::RefNo(RefNo::new(0, 0)));
    }
    static_default_value(attribute)
}

fn static_default_value(attribute: &CatalogAttribute) -> Option<DescriptorValue> {
    if attribute.is_array {
        return Some(DescriptorValue::Unset);
    }

    match attribute.type_name.as_str() {
        "BOOL" => Some(DescriptorValue::Bool(false)),
        "INTEGER" => Some(DescriptorValue::Int(0)),
        "DOUBLE" => Some(DescriptorValue::Real(0.0)),
        "ELEMENT" => Some(DescriptorValue::RefNo(RefNo::new(0, 0))),
        // An empty word slot reads as zero, which E3D writes as `0` for some
        // attributes and `unset` for others; `unset` proper belongs to the
        // types that have no slot to be empty.
        "WORD" => Some(DescriptorValue::Word { raw: 0, text: None }),
        "STRING" | "DIRECTION" | "POSITION" | "DATETIME" => Some(DescriptorValue::Unset),
        _ => None,
    }
}

fn parse_noun(value: &Value) -> Result<CatalogNoun, CatalogError> {
    let object = object(value, "noun")?;
    let noun = string(object, "noun", "noun")?.to_owned();
    let attrs = object
        .get("attrs")
        .and_then(Value::as_array)
        .ok_or(CatalogError::MissingField("noun.attrs"))?
        .iter()
        .map(parse_attribute)
        .collect::<Result<Vec<_>, _>>()?;
    let db_types = object
        .get("dbTypes")
        .and_then(Value::as_array)
        .ok_or(CatalogError::MissingField("noun.dbTypes"))?
        .iter()
        .map(|value| number_value(value, "noun.dbTypes"))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CatalogNoun {
        noun_hash: number(object, "hash", "noun")?,
        noun,
        short_name: string(object, "short", "noun")?.to_owned(),
        base: string(object, "base", "noun")?.to_owned(),
        hard: string(object, "hard", "noun")?.to_owned(),
        is_udet: boolean(object, "isUdet", "noun")?,
        is_pseudo: boolean(object, "isPseudo", "noun")?,
        is_world: boolean(object, "isWorld", "noun")?,
        is_primary: boolean(object, "isPrimary", "noun")?,
        visible: boolean(object, "visible", "noun")?,
        db_types,
        attrs,
    })
}

fn parse_attribute(value: &Value) -> Result<CatalogAttribute, CatalogError> {
    let object = object(value, "attribute")?;
    Ok(CatalogAttribute {
        name: string(object, "name", "attribute")?.to_owned(),
        hash: number(object, "hash", "attribute")?,
        type_code: number(object, "type", "attribute")?,
        type_name: string(object, "typeName", "attribute")?.to_owned(),
        is_array: boolean(object, "isArray", "attribute")?,
        max_size: number(object, "maxSize", "attribute")?,
        true_size: number(object, "trueSize", "attribute")?,
        is_uda: boolean(object, "isUda", "attribute")?,
        is_pseudo: boolean(object, "isPseudo", "attribute")?,
        hidden: boolean(object, "hidden", "attribute")?,
        no_claim: boolean(object, "noClaim", "attribute")?,
        default_probe: string(object, "def", "attribute")?.to_owned(),
    })
}

fn object<'a>(
    value: &'a Value,
    context: &'static str,
) -> Result<&'a Map<String, Value>, CatalogError> {
    value
        .as_object()
        .ok_or(CatalogError::ExpectedObject(context))
}

fn string<'a>(
    object: &'a Map<String, Value>,
    key: &'static str,
    context: &'static str,
) -> Result<&'a str, CatalogError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or(CatalogError::WrongFieldType {
            context,
            key,
            expected: "string",
        })
}

fn boolean(
    object: &Map<String, Value>,
    key: &'static str,
    context: &'static str,
) -> Result<bool, CatalogError> {
    object
        .get(key)
        .and_then(Value::as_bool)
        .ok_or(CatalogError::WrongFieldType {
            context,
            key,
            expected: "boolean",
        })
}

fn number(
    object: &Map<String, Value>,
    key: &'static str,
    context: &'static str,
) -> Result<u32, CatalogError> {
    object
        .get(key)
        .ok_or(CatalogError::WrongFieldType {
            context,
            key,
            expected: "u32",
        })
        .and_then(|value| number_value(value, context))
}

fn number_value(value: &Value, context: &'static str) -> Result<u32, CatalogError> {
    value
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(CatalogError::InvalidNumber(context))
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("read catalog {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("expected JSON array for {0}")]
    ExpectedArray(&'static str),
    #[error("expected JSON object for {0}")]
    ExpectedObject(&'static str),
    #[error("missing required catalog field {0}")]
    MissingField(&'static str),
    #[error("{context}.{key} must be a {expected}")]
    WrongFieldType {
        context: &'static str,
        key: &'static str,
        expected: &'static str,
    },
    #[error("{0} must be an unsigned 32-bit integer")]
    InvalidNumber(&'static str),
    #[error("invalid decimal attribute-field hash key {0}")]
    InvalidFieldHash(String),
    #[error("duplicate noun hash 0x{0:08X}")]
    DuplicateNounHash(u32),
    #[error("duplicate attribute-field hash 0x{0:08X}")]
    DuplicateFieldHash(u32),
    #[error("{noun}.{attribute} (0x{hash:08X}) is absent from noun_attr_fields.json")]
    MissingAttributeFields {
        noun: String,
        attribute: String,
        hash: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::template::TemplateAttr;
    use e3d_attlib::db1_hash;

    const LAYOUT: &str = r#"[{"noun":"TEST TYPE","short":"TEST","hash":939107,"base":"BASE","hard":"HARD","isUdet":false,"isPseudo":false,"isWorld":false,"isPrimary":true,"visible":true,"dbTypes":[1,2],"attrs":[{"name":"B","hash":2,"type":1,"typeName":"INTEGER","isArray":false,"maxSize":1,"trueSize":0,"isUda":false,"isPseudo":false,"hidden":false,"noClaim":false,"def":"i:0"},{"name":"A","hash":1,"type":5,"typeName":"BOOLEAN","isArray":false,"maxSize":1,"trueSize":0,"isUda":false,"isPseudo":false,"hidden":true,"noClaim":true,"def":"b:false"}]}]"#;
    const FIELDS: &str = r#"{"1":{"name":"A","f":{"DCHC":{"i":7}}},"2":{"name":"B","f":{"DCHC":{"i":0},"EXTRA":{"s":"kept"}}},"3":{"name":"C","f":{}}}"#;

    #[test]
    fn parses_order_and_preserves_all_field_values() {
        let catalog = NounCatalog::parse(LAYOUT, FIELDS).unwrap();
        let noun = catalog.noun(db1_hash("TEST")).unwrap();
        assert_eq!(
            noun.attrs
                .iter()
                .map(|attr| attr.name.as_str())
                .collect::<Vec<_>>(),
            ["B", "A"]
        );
        assert_eq!(
            catalog.attribute_fields(2).unwrap().fields["EXTRA"]["s"],
            "kept"
        );
    }

    #[test]
    fn reconciliation_keeps_catalog_order_and_appends_physical_only() {
        let catalog = NounCatalog::parse(LAYOUT, FIELDS).unwrap();
        let mut template = NounTemplate::new(db1_hash("TEST"));
        template.attrs = vec![TemplateAttr::single_word(1), TemplateAttr::single_word(3)];
        let merged = catalog.reconcile(&template);
        assert_eq!(merged.attributes.len(), 3);
        assert_eq!(merged.attributes[0].catalog.unwrap().name, "B");
        assert_eq!(merged.attributes[0].source, ReconciledSource::CatalogOnly);
        assert_eq!(merged.attributes[1].catalog.unwrap().name, "A");
        assert_eq!(
            merged.attributes[1].source,
            ReconciledSource::CatalogAndDescriptor
        );
        assert_eq!(merged.attributes[2].descriptor.unwrap().hash, 3);
        assert_eq!(
            merged.attributes[2].source,
            ReconciledSource::DescriptorOnly
        );
    }

    #[test]
    fn descriptor_only_noun_stays_extractable_without_catalog_entry() {
        let catalog = NounCatalog::parse(LAYOUT, FIELDS).unwrap();
        let mut template = NounTemplate::new(db1_hash("DBMDAT"));
        template.attrs = vec![TemplateAttr::single_word(3)];
        let merged = catalog.reconcile(&template);
        assert!(merged.noun.is_none());
        assert_eq!(
            merged.attributes[0].source,
            ReconciledSource::DescriptorOnly
        );
    }

    fn catalog_attribute(
        type_name: &str,
        is_array: bool,
        is_pseudo: bool,
        probe: &str,
    ) -> CatalogAttribute {
        CatalogAttribute {
            name: "X".into(),
            hash: 1,
            type_code: 0,
            type_name: type_name.into(),
            is_array,
            max_size: 1,
            true_size: 0,
            is_uda: false,
            is_pseudo,
            hidden: false,
            no_claim: false,
            default_probe: probe.into(),
        }
    }

    #[test]
    fn static_defaults_preserve_e3d_unset_zero_false_and_nulref_distinctions() {
        assert_eq!(
            static_default_value(&catalog_attribute("STRING", false, false, "e2/153")),
            Some(DescriptorValue::Unset)
        );
        assert_eq!(
            static_default_value(&catalog_attribute("INTEGER", false, false, "i:0")),
            Some(DescriptorValue::Int(0))
        );
        assert_eq!(
            static_default_value(&catalog_attribute("BOOL", false, false, "e2/153")),
            Some(DescriptorValue::Bool(false))
        );
        assert_eq!(
            static_default_value(&catalog_attribute("DOUBLE", false, false, "e2/153")),
            Some(DescriptorValue::Real(0.0))
        );
        assert_eq!(
            static_default_value(&catalog_attribute("ELEMENT", false, false, "e2/153")),
            Some(DescriptorValue::RefNo(RefNo::new(0, 0)))
        );
        assert_eq!(
            static_default_value(&catalog_attribute("INTEGER", true, false, "i:0")),
            Some(DescriptorValue::Unset)
        );
        // A word slot with nothing in it holds zero rather than `unset`, and
        // E3D spells that zero `0` on `NAPP` and `unset` on `QTYPE`.
        assert_eq!(
            static_default_value(&catalog_attribute("WORD", false, false, "i:0")),
            Some(DescriptorValue::Word { raw: 0, text: None })
        );
        // The default belongs to the type: an attribute the catalog says
        // defaults to `ANY` still reads as an empty slot, because `q att`
        // never printed a declared default for an element storing nothing.
        assert_eq!(
            static_default_value(&catalog_attribute("WORD", false, false, "i:550045")),
            Some(DescriptorValue::Word { raw: 0, text: None })
        );
        assert_eq!(
            static_default_value(&catalog_attribute("INTEGER", false, false, "i:2")),
            Some(DescriptorValue::Int(0))
        );
    }

    #[test]
    fn a_reference_slot_defaults_to_nulref_whatever_the_catalog_calls_the_type() {
        // Storage type 4 is a two-word RefNo. `DETA` reaches it while the
        // catalog calls the attribute `BOOL`, and `q att` prints `Nulref`.
        assert_eq!(
            type_zero(&catalog_attribute("BOOL", false, false, "e2/153"), 4),
            Some(DescriptorValue::RefNo(RefNo::new(0, 0)))
        );
        // Nothing changes for a slot the descriptor agrees about, nor for an
        // array, whose absence stays `unset`.
        assert_eq!(
            type_zero(&catalog_attribute("BOOL", false, false, "e2/153"), 5),
            Some(DescriptorValue::Bool(false))
        );
        assert_eq!(
            type_zero(&catalog_attribute("BOOL", true, false, "e2/153"), 4),
            Some(DescriptorValue::Unset)
        );
    }

    #[test]
    fn circular_end_rule_does_not_claim_absent_or_zero_bore() {
        assert!(!is_positive_real(None));
        assert!(!is_positive_real(Some(&DescriptorValue::Real(0.0))));
        assert!(!is_positive_real(Some(&DescriptorValue::Unset)));
        assert!(is_positive_real(Some(&DescriptorValue::Real(50.0))));
    }
}
