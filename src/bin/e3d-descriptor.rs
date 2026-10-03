use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::path::{Path, PathBuf};

use e3d_attlib::{db1_dehash, db1_hash, lookup_system_name, AttlibData};
use e3d_io::attlib_metadata::AttributeMetadataCatalog;
use e3d_io::engine::ScanTier;
use e3d_io::index::{read_spamap, SpamapEntry, SpamapRead};
use e3d_io::noun_catalog::{NounCatalog, ReconciledSource};
use e3d_io::record::descriptor::{AttributeExtractionStatus, DescriptorValue, ElementExtraction};
use e3d_io::record::element::ParsedElement;
use e3d_io::record::template::{NounTemplate, StorageLayout, TemplateProvider};
use e3d_io::record::template_file::TemplateFile;
use e3d_io::refno::RefNo;
use e3d_io::schema::SchemaPack;
use e3d_io::session::{
    ChangeKind, ChangedElement, DbView, ElementDelta, SessionDiff, SessionSelector,
};
use e3d_io::tty::{
    apply_reference_names, parse_attlist_capture, reconcile_attlist, CompleteAttributeRow,
    CompleteAttributeSource, ValueComparison,
};
use e3d_io::uda_catalog::{UdaCatalog, UdaDefaultResolution};
use e3d_io::ReadOnlyEngine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;

const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let command = args.next().ok_or_else(usage)?;
    if command == "spamap-census" {
        return spamap_census_command(args);
    }
    if command == "spamap-check" {
        return spamap_check_command(args);
    }
    let mut options = parse_options(args)?;
    apply_schema_pack(&command, &mut options)?;
    if command == "compile-metadata" {
        let summary = compile_metadata(&options)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&summary).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    if command == "emit-attr-info" {
        let summary = emit_attr_info(&options)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&summary).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    if command == "emit-uda-table" {
        let summary = emit_uda_table(&options)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&summary).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    if command == "sessions" {
        let summary = sessions_command(&options)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&summary).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    if command == "spamap" {
        let summary = spamap_command(&options)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&summary).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    if command == "diff" {
        let summary = diff_command(&options)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&summary).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    if command == "project-dab-tags" {
        let project_root = required(&options, "project-root")?;
        let report = e3d_io::project_dabacon_tags(project_root)
            .map_err(|error| format!("aggregate project DAB tags: {error}"))?;
        println!(
            "{}",
            serde_json::to_string_pretty(&project_dabacon_tags_json(report))
                .map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    if command == "header" {
        let db = required(&options, "db")?;
        let engine = ReadOnlyEngine::open(db).map_err(|e| format!("open database: {e}"))?;
        let database = Path::new(db);
        let context = database.parent().unwrap_or(database);
        let evidence = e3d_io::project_version_evidence(context)
            .or_else(|_| e3d_io::database_version_evidence(database))
            .map_err(|e| format!("read database project version diagnostics: {e}"))?;
        let descriptor = engine.descriptor();
        let header = engine.header_diagnostics();
        let summary = json!({
            "db": db,
            "e3d_version": Value::Null,
            "automatic_schema_selection": "disabled",
            "version_diagnostic_source": "database-file",
            "version_diagnostic_field": evidence.source.field_name(),
            "version_diagnostic_raw_value": evidence.raw_value,
            "version_diagnostic_raw_text": evidence.raw_text,
            "version_diagnostic_session_page": evidence.session_page,
            "version_diagnostic_database": evidence.database,
            "dbnum": descriptor.db_mark,
            "db_type": descriptor.db_type_name(),
            "db_type_hash": descriptor.db_type_hash,
            "latest_ses_pgno": descriptor.latest_ses_pgno,
            "extent": descriptor.ext_no,
            "page_size": descriptor.page_size,
            "header_creation_info": header.creation_info,
            "header_template_version": header.template_version,
            "header_format": header.header_format,
            "selected_session_number": header.session_number,
            "selected_session_dabacon_version_tag": header.dabacon_version_tag,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&summary).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    let desvir = required(&options, "desvir")?;
    let attlib_path = required(&options, "attlib")?;
    let attlib = AttlibData::parse_file(attlib_path)?;
    let metadata = AttributeMetadataCatalog::parse_file(attlib_path, &attlib)?;
    let catalog = load_catalog(&options)?;
    let template_type = options
        .get("template-type")
        .map(|value| parse_noun(value))
        .transpose()?
        .unwrap_or(DESIGN_TEMPLATE_TYPE);
    let mut provider = TemplateProvider::open(desvir, Some(template_type))
        .map_err(|e| format!("open descriptor template: {e}"))?;

    let output = match command.as_str() {
        "list-uda" => {
            let db = required(&options, "db")?;
            let noun = options
                .get("noun")
                .map(|value| parse_noun(value))
                .transpose()?;
            let uda = UdaCatalog::read(db, desvir, &attlib)?;
            let definitions = uda
                .definitions()
                .iter()
                .filter(|definition| noun.is_none_or(|noun| definition.applies_to(noun)))
                .map(|definition| {
                    let (resolved_default, default_resolution) = match definition.resolve_default()
                    {
                        UdaDefaultResolution::Unset => {
                            (value_json(&DescriptorValue::Unset), "unset")
                        }
                        UdaDefaultResolution::Value(ref value) => (value_json(value), "value"),
                        UdaDefaultResolution::Invalid { .. } => {
                            (value_json(&DescriptorValue::Unset), "invalid_for_type")
                        }
                    };
                    json!({
                        "refno": definition.refno.to_string(),
                        "owner": definition.owner.to_string(),
                        "key": definition.key,
                        "element_name": definition.element_name,
                        "name": definition.name,
                        "dynamic_name": definition.dynamic_name,
                        "is_pseudo": definition.is_pseudo,
                        "value_type_hash": definition.value_type_hash,
                        "value_type": definition.value_type,
                        "length": definition.length,
                        "applicable_nouns": definition.applicable_nouns,
                        "default_is_defined": definition.default_is_defined,
                        "default_type": definition.default_type,
                        "default_mode": definition.default_mode,
                        "default_text": definition.default_text,
                        "default_real": definition.default_real,
                        "default_location": definition.default_location,
                        "default_attribute": definition.default_attribute,
                        "default_expression": definition.default_expression,
                        "resolved_default": resolved_default,
                        "default_resolution": default_resolution,
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "definition_count": uda.definitions().len(),
                "selected_count": definitions.len(),
                "issue_count": uda.issues().len(),
                "issues": uda.issues().iter().map(|issue| json!({
                    "refno": issue.refno.to_string(),
                    "detail": issue.detail,
                })).collect::<Vec<_>>(),
                "definitions": definitions,
            })
        }
        "describe" => {
            let noun = parse_noun(required(&options, "noun")?)?;
            let template = provider
                .template_for(noun)
                .map_err(|e| format!("load noun descriptor: {e}"))?
                .ok_or_else(|| format!("noun descriptor 0x{noun:08X} is missing"))?;
            describe_json(template, &attlib, &metadata, catalog.as_ref())
        }
        "extract" | "extract-complete" => {
            let db = required(&options, "db")?;
            let refno = parse_refno(required(&options, "refno")?)?;
            let uda = uda_enabled(&options)?;
            let mut engine = ReadOnlyEngine::open(db).map_err(|e| format!("open database: {e}"))?;
            let mut extraction = engine
                .extract_element_with_descriptors(refno, &attlib, &mut provider)
                .map_err(|e| format!("extract element: {e}"))?;
            if command == "extract-complete" {
                let template = provider
                    .template_for(extraction.noun_hash)
                    .map_err(|e| format!("load noun descriptor: {e}"))?;
                if let Some(template) = template.as_ref() {
                    read_stream_defaults_as_type_zero(&mut extraction, template);
                }
            }
            if let Some(catalog) = catalog.as_ref() {
                catalog.apply_static_defaults(&mut extraction);
                catalog.apply_runtime_derived(&mut extraction);
                // Last, so the two above keep first claim on anything they know.
                catalog.apply_pseudo_defaults(&mut extraction);
            }
            let dictionary_dbs = dictionary_databases(&options)?;
            if !uda && !dictionary_dbs.is_empty() {
                return Err(
                    "--dictionary-db/--dictionary-db-list needs --uda on: which dictionaries \
                     hold an element's UDA values is decided by the MDB, so reading them \
                     has to be asked for"
                        .into(),
                );
            }
            if uda && !dictionary_dbs.is_empty() {
                let dicvir = required(&options, "dicvir")?;
                let mut uda = UdaCatalog::default();
                for dictionary_db in dictionary_dbs {
                    uda.merge_in_order(UdaCatalog::read(dictionary_db, dicvir, &attlib)?);
                }
                let view = engine
                    .find_element(refno)
                    .map_err(|e| format!("reopen element for UDA values: {e}"))?
                    .ok_or_else(|| format!("element {refno} disappeared during UDA extraction"))?;
                let parsed = ParsedElement::parse_as(&view.raw_bytes, engine.text_format())
                    .map_err(|e| format!("parse element for UDA values: {e}"))?;
                uda.materialize_for_element(&mut extraction, &parsed.explicit_attributes);
            }
            if command == "extract-complete" {
                let tty_path = required(&options, "tty-log")?;
                let capture = fs::read_to_string(tty_path)
                    .map_err(|e| format!("read TTY capture {tty_path}: {e}"))?;
                let captured_rows = parse_attlist_capture(&capture)?;
                // A UDA's value is not in the element; it is in whichever
                // dictionary databases the MDB puts in front of this one.
                // Scoring those rows with no dictionary loaded measures the
                // wiring rather than the decoder, so they are set aside and
                // counted instead of compared.
                let (tty_rows, deferred_uda): (Vec<_>, Vec<_>) = if uda {
                    (captured_rows, Vec::new())
                } else {
                    captured_rows
                        .into_iter()
                        .partition(|row| !row.name.starts_with(':'))
                };
                let mut complete =
                    reconcile_attlist(&tty_rows, &extraction, &metadata, catalog.as_ref());
                let reference_dbs = reference_databases(&options, db);
                let reference_names =
                    resolve_reference_names(&complete, &reference_dbs, catalog.as_ref())?;
                apply_reference_names(&mut complete, &reference_names);
                if let Some(catalog) = catalog.as_ref() {
                    apply_display_name(&mut complete, &mut engine, catalog, refno);
                }
                complete_extraction_json(
                    &extraction,
                    &complete,
                    catalog.as_ref(),
                    &metadata,
                    uda,
                    &deferred_uda,
                )
            } else {
                extraction_json(&extraction, &metadata)
            }
        }
        _ => return Err(usage()),
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&output).map_err(|e| e.to_string())?
    );
    Ok(())
}

/// Compile the complete installation schema into one deterministic JSON file.
///
/// Physical locations come only from every `*vir.dat` child payload.  The live
/// noun export and attribute-field export are retained verbatim at the top
/// level so consumers can join logical metadata without reconstructing it.
fn compile_metadata(options: &HashMap<String, String>) -> Result<Value, String> {
    let template_dir = PathBuf::from(required(options, "template-dir")?);
    let attlib_path = PathBuf::from(required(options, "attlib")?);
    let layout_path = PathBuf::from(required(options, "catalog-layout")?);
    let fields_path = PathBuf::from(required(options, "catalog-fields")?);
    let output_path = PathBuf::from(required(options, "output")?);

    let layout_text = fs::read_to_string(&layout_path)
        .map_err(|e| format!("read {}: {e}", layout_path.display()))?;
    let fields_text = fs::read_to_string(&fields_path)
        .map_err(|e| format!("read {}: {e}", fields_path.display()))?;
    // Validate the two live exports before copying them into the compiled file.
    let catalog = NounCatalog::parse(&layout_text, &fields_text)
        .map_err(|e| format!("validate noun catalog: {e}"))?;
    let noun_layout: Value = serde_json::from_str(&layout_text).map_err(|e| e.to_string())?;
    let attribute_fields: Value = serde_json::from_str(&fields_text).map_err(|e| e.to_string())?;

    let attlib = AttlibData::parse_file(&attlib_path)?;
    let report_metadata = AttributeMetadataCatalog::parse_file(&attlib_path, &attlib)?;

    let mut template_paths = fs::read_dir(&template_dir)
        .map_err(|e| format!("read template directory {}: {e}", template_dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.to_ascii_lowercase().ends_with("vir.dat"))
        })
        .collect::<Vec<_>>();
    template_paths.sort_by_key(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default()
    });
    if template_paths.is_empty() {
        return Err(format!(
            "no *vir.dat files found in {}",
            template_dir.display()
        ));
    }

    let mut databases = Vec::with_capacity(template_paths.len());
    let mut source_templates = Vec::with_capacity(template_paths.len());
    let mut noun_payloads: HashMap<u32, Vec<String>> = HashMap::new();
    let mut variant_count = 0usize;
    let mut descriptor_count = 0usize;
    let mut empty_template_count = 0usize;

    for path in &template_paths {
        let file_name = file_name(path)?;
        let file_sha256 = sha256_file(path)?;
        let file_bytes = fs::metadata(path)
            .map_err(|e| format!("stat {}: {e}", path.display()))?
            .len();
        let mut file = TemplateFile::open(path).map_err(|e| e.to_string())?;
        let header = file.read_header().map_err(|e| e.to_string())?;
        let children = file.read_children(&header).map_err(|e| e.to_string())?;
        let mut nouns = Vec::with_capacity(children.len());

        for child in &children {
            let payload = file.read_payload(child).map_err(|e| {
                format!("read template {} noun 0x{:08X}: {e}", file_name, child.hash)
            })?;
            let payload_sha256 = sha256_words(&payload);
            noun_payloads
                .entry(child.hash)
                .or_default()
                .push(payload_sha256.clone());
            variant_count += 1;

            let (physical_status, attributes) = if payload.len() < 14 {
                empty_template_count += 1;
                ("empty", Vec::new())
            } else {
                let template =
                    NounTemplate::parse_template_payload(&payload, child.hash).map_err(|e| {
                        format!(
                            "parse template {} noun 0x{:08X}: {e}",
                            file_name, child.hash
                        )
                    })?;
                descriptor_count += template.attrs.len();
                let attributes = template
                    .attrs
                    .iter()
                    .enumerate()
                    .map(|(ordinal, descriptor)| {
                        let def = attlib.attr_def(descriptor.hash);
                        json!({
                            "ordinal": ordinal,
                            "hash": descriptor.hash,
                            "name": attribute_name(descriptor.hash),
                            "report_name": report_metadata.report_name(descriptor.hash),
                            "stride": descriptor.stride,
                            "storage_type_code": descriptor.storage_type_code,
                            "logical_count": descriptor.logical_count,
                            "semantic_type_code": def.map(|value| value.data_type),
                            "semantic_kind": def.map(|value| value.kind),
                            "semantic_size": def.map(|value| value.size),
                            "compact": storage_json(descriptor.compact),
                            "wide": storage_json(descriptor.wide),
                            "tail": descriptor.tail,
                        })
                    })
                    .collect::<Vec<_>>();
                ("descriptor", attributes)
            };

            nouns.push(json!({
                "noun_hash": child.hash,
                "noun_name": db1_dehash(child.hash),
                "physical_status": physical_status,
                "payload_words": payload.len(),
                "payload_sha256": payload_sha256,
                "attributes": attributes,
            }));
        }

        source_templates.push(json!({
            "file": file_name,
            "sha256": file_sha256,
            "bytes": file_bytes,
            "db_type_hash": header.template_type,
            "db_type": db1_dehash(header.template_type),
            "noun_count": children.len(),
        }));
        databases.push(json!({
            "db_type_hash": header.template_type,
            "db_type": db1_dehash(header.template_type),
            "template_file": file_name,
            "nouns": nouns,
        }));
    }

    let reused_nouns = noun_payloads
        .values()
        .filter(|payloads| payloads.len() > 1)
        .count();
    let differing_layout_nouns = noun_payloads
        .values()
        .filter(|payloads| payloads.len() > 1)
        .filter(|payloads| payloads.iter().collect::<HashSet<_>>().len() > 1)
        .count();

    let compiled = json!({
        "schema_version": 1,
        "format": "e3d-parser-metadata",
        "generator": format!("e3d-io/{}", env!("CARGO_PKG_VERSION")),
        "sources": {
            "templates": source_templates,
            "attlib": source_file_json(&attlib_path)?,
            "noun_layout": source_file_json(&layout_path)?,
            "noun_attr_fields": source_file_json(&fields_path)?,
        },
        "counts": {
            "database_types": databases.len(),
            "noun_variants": variant_count,
            "unique_nouns": noun_payloads.len(),
            "reused_nouns": reused_nouns,
            "differing_layout_nouns": differing_layout_nouns,
            "physical_descriptors": descriptor_count,
            "empty_templates": empty_template_count,
            "logical_nouns": catalog.noun_count(),
            "logical_declarations": catalog.declaration_count(),
            "attribute_fields": catalog.field_count(),
            "attribute_field_values": catalog.attribute_field_value_count(),
        },
        "databases": databases,
        "noun_layout": noun_layout,
        "attribute_fields": attribute_fields,
    });

    // Keep the checked-in artifact compact: the source catalogs already carry
    // 203k field values and pretty-printing more than doubles the byte size.
    // serde_json's default map order is deterministic, so compact output still
    // supports byte-for-byte regeneration checks.
    let mut bytes = serde_json::to_vec(&compiled).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("create output directory {}: {e}", parent.display()))?;
    }
    fs::write(&output_path, &bytes).map_err(|e| format!("write {}: {e}", output_path.display()))?;

    Ok(json!({
        "output": output_path.display().to_string(),
        "sha256": sha256_bytes(&bytes),
        "bytes": bytes.len(),
        "counts": compiled["counts"].clone(),
    }))
}

/// Emit the `all_attr_info` replacement table (schema `e3d-attr-info/v2`).
///
/// Physical layout comes from every `*vir.dat` child payload and is keyed by
/// `(db_type, noun, attribute)` so the 41 nouns whose layout differs between
/// databases stay distinguishable. The flat `noun_attr_info_map` that the
/// legacy consumer reads is *not* written to disk; it is folded at load time
/// from `by_db`, and every slot that folding has to discard is listed in
/// `flat_conflicts` rather than silently dropped.
///
/// `att_type` and `default_val` cannot be recovered from the binary alone.
/// When a legacy `all_attr_info.json` is supplied its values are copied
/// verbatim, which makes the overlap regression-free by construction; the
/// derivation rule below only ever fills ground the legacy file never covered.
fn emit_attr_info(options: &HashMap<String, String>) -> Result<Value, String> {
    let template_dir = PathBuf::from(required(options, "template-dir")?);
    let attlib_path = PathBuf::from(required(options, "attlib")?);
    let output_path = PathBuf::from(required(options, "output")?);
    let flat_winner = options
        .get("flat-winner")
        .map(String::as_str)
        .unwrap_or("DESI")
        .to_ascii_uppercase();

    let attlib = AttlibData::parse_file(&attlib_path)?;
    let report_metadata = AttributeMetadataCatalog::parse_file(&attlib_path, &attlib)?;
    let catalog = load_catalog(options)?
        .ok_or("emit-attr-info requires --catalog-layout and --catalog-fields")?;

    let legacy = match options.get("legacy-attr-info") {
        Some(path) => LegacyAttrInfo::read(path)?,
        None => LegacyAttrInfo::default(),
    };
    let derived_defaults = legacy.learn_default_rule(&catalog);

    let mut template_paths = fs::read_dir(&template_dir)
        .map_err(|e| format!("read template directory {}: {e}", template_dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.to_ascii_lowercase().ends_with("vir.dat"))
        })
        .collect::<Vec<_>>();
    template_paths.sort();
    if template_paths.is_empty() {
        return Err(format!(
            "no *vir.dat files found in {}",
            template_dir.display()
        ));
    }

    let mut by_db = serde_json::Map::new();
    let mut sources = Vec::with_capacity(template_paths.len());
    let mut flat: HashMap<(u32, u32), (String, u32, Value)> = HashMap::new();
    let mut flat_conflicts = Vec::new();
    let mut triples = 0usize;
    let mut nouns_seen: HashSet<u32> = HashSet::new();
    let mut verbatim = 0usize;
    let mut derived = 0usize;
    let mut with_descriptor_default = 0usize;

    for path in &template_paths {
        let mut file = TemplateFile::open(path).map_err(|e| e.to_string())?;
        let header = file.read_header().map_err(|e| e.to_string())?;
        let children = file.read_children(&header).map_err(|e| e.to_string())?;
        let db_type = header.template_type;
        let db_name = db1_dehash(db_type).unwrap_or_else(|| format!("0x{db_type:08X}"));
        sources.push(json!({
            "file": file_name(path)?,
            "sha256": sha256_file(path)?,
            "db_type_hash": db_type,
            "db_type": db_name,
            "noun_count": children.len(),
        }));

        let mut nouns = serde_json::Map::new();
        for child in &children {
            if file.read_payload(child).map_err(|e| e.to_string())?.len() < 14 {
                continue;
            }
            let template = NounTemplate::load_child(&mut file, child, child.hash)
                .map_err(|e| format!("parse {} noun 0x{:08X}: {e}", db_name, child.hash))?;
            if template.attrs.is_empty() {
                continue;
            }
            nouns_seen.insert(child.hash);

            let mut attrs = serde_json::Map::new();
            for (ordinal, descriptor) in template.attrs.iter().enumerate() {
                triples += 1;
                let hash = descriptor.hash;
                let name = attribute_name(hash);
                let catalog_attr = catalog
                    .noun(child.hash)
                    .and_then(|noun| noun.attrs.iter().find(|attribute| attribute.hash == hash));
                let fields = catalog.attribute_fields(hash);

                let (att_type, from_legacy_type) = match legacy.att_type(child.hash, hash) {
                    Some(value) => (value.to_string(), true),
                    None => (catalog_att_type(catalog_attr), false),
                };
                let is_array = catalog_attr.map(|a| a.is_array).unwrap_or(false);

                let (default_val, source) = match legacy.default_val(child.hash, hash) {
                    Some(value) => {
                        verbatim += 1;
                        (value.clone(), "verbatim")
                    }
                    None => {
                        derived += 1;
                        (
                            derived_defaults
                                .lookup(&att_type, fields)
                                .cloned()
                                .unwrap_or_else(|| fallback_default(&att_type, is_array)),
                            "derived",
                        )
                    }
                };

                // What the noun itself states this attribute defaults to, out
                // of `*vir.dat` chunks 1 and 2. Emitted beside `default_val`
                // rather than in place of it: the derived value is what the
                // current consumer already reads, and the two want comparing
                // before either replaces the other.
                let descriptor_default = e3d_io::record::descriptor::extract_default(
                    descriptor,
                    attlib.attr_def(hash),
                    &template.defaults,
                    name.clone(),
                )
                .and_then(|attribute| attribute.value)
                .map(|value| value_json(&value));
                if descriptor_default.is_some() {
                    with_descriptor_default += 1;
                }

                let wide_raw = descriptor.wide.location.raw;
                let entry = json!({
                        "name": name,
                        "hash": hash,
                        "offset": wide_raw,
                        "default_val": default_val,
                        "att_type": att_type,
                        "ityp": Value::Null,
                        "ordinal": ordinal,
                        "storage_type_code": descriptor.storage_type_code,
                        "logical_count": descriptor.logical_count,
                        "stride": descriptor.stride,
                        "is_array": is_array,
                        "report_name": report_metadata.report_name(hash),
                        "compact": slot_json(descriptor.compact),
                        "wide": slot_json(descriptor.wide),
                        "implicit": descriptor.wide.location.word_offset() != 0,
                        "default_source": source,
                        "descriptor_default": descriptor_default,
                    "att_type_source": (if from_legacy_type { "legacy" } else { "catalog" }),
                    "db_type": db_type,
                });
                attrs.insert(hash.to_string(), entry.clone());

                let key = (child.hash, hash);
                let this_wins = db_name == flat_winner;
                match flat.get(&key) {
                    None => {
                        flat.insert(key, (db_name.clone(), wide_raw, entry));
                    }
                    Some((held_db, held_raw, _)) => {
                        let held_wins = held_db == &flat_winner;
                        if *held_raw != wide_raw {
                            let keep_held = held_wins || !this_wins;
                            flat_conflicts.push(json!({
                                "noun": db1_dehash(child.hash),
                                "noun_hash": child.hash,
                                "attr": attribute_name(hash),
                                "attr_hash": hash,
                                "kept_db": if keep_held { held_db.clone() } else { db_name.clone() },
                                "dropped_db": if keep_held { db_name.clone() } else { held_db.clone() },
                                "kept_offset": if keep_held { *held_raw } else { wide_raw },
                                "dropped_offset": if keep_held { wide_raw } else { *held_raw },
                            }));
                        }
                        if this_wins && !held_wins {
                            flat.insert(key, (db_name.clone(), wide_raw, entry));
                        }
                    }
                }
            }
            nouns.insert(child.hash.to_string(), Value::Object(attrs));
        }
        by_db.insert(db_type.to_string(), Value::Object(nouns));
    }

    let mut flat_entries: Vec<(u32, u32, Value)> = flat
        .into_iter()
        .map(|((noun, attr), (_, _, entry))| (noun, attr, entry))
        .collect();
    flat_entries.sort_by_key(|(noun, attr, _)| (*noun, *attr));

    let document = json!({
        "schema": "e3d-attr-info/v2",
        "generator": format!("e3d-io/{}", env!("CARGO_PKG_VERSION")),
        "flat_winner": flat_winner,
        "sources": {
            "templates": sources,
            "attlib": source_file_json(&attlib_path)?,
        },
        "counts": {
            "database_types": by_db.len(),
            "nouns": nouns_seen.len(),
            "triples": triples,
            "default_verbatim": verbatim,
            "default_derived": derived,
            "with_descriptor_default": with_descriptor_default,
            "flat_conflicts": flat_conflicts.len(),
        },
        "flat_conflicts": flat_conflicts,
        "by_db": by_db,
    });

    let mut bytes = serde_json::to_vec(&document).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    write_output(&output_path, &bytes)?;

    // Optional v1-shaped sibling. `PdmsDatabaseInfo` has no `serde(default)` on
    // `noun_attr_info_map`, so until the loader learns to fold `by_db` itself a
    // drop-in file is the only way to A/B the new data against the old parser.
    let compat = match options.get("compat-output") {
        Some(path) => {
            let mut noun_map: serde_json::Map<String, Value> = serde_json::Map::new();
            for (&(noun, attr), entry) in &legacy.full_entries {
                let slot = noun_map
                    .entry(noun.to_string())
                    .or_insert_with(|| Value::Object(serde_json::Map::new()));
                if let Some(object) = slot.as_object_mut() {
                    object.insert(attr.to_string(), entry.clone());
                }
            }
            let legacy_only_pairs = legacy
                .full_entries
                .keys()
                .filter(|key| {
                    !flat_entries
                        .iter()
                        .any(|(noun, attr, _)| key == &&(*noun, *attr))
                })
                .count();
            for (noun, attr, entry) in &flat_entries {
                let slot = noun_map
                    .entry(noun.to_string())
                    .or_insert_with(|| Value::Object(serde_json::Map::new()));
                if let Some(object) = slot.as_object_mut() {
                    object.insert(
                        attr.to_string(),
                        json!({
                            "name": entry["name"].clone(),
                            "hash": entry["hash"].clone(),
                            "offset": entry["offset"].clone(),
                            "default_val": entry["default_val"].clone(),
                            "att_type": entry["att_type"].clone(),
                            "ityp": Value::Null,
                        }),
                    );
                }
            }
            let pairs = noun_map
                .values()
                .filter_map(Value::as_object)
                .map(|attrs| attrs.len())
                .sum::<usize>();
            let compat_document = json!({
                "noun_attr_info_map": noun_map,
                "named_attr_info_map": serde_json::Map::new(),
            });
            let mut compat_bytes =
                serde_json::to_vec(&compat_document).map_err(|e| e.to_string())?;
            compat_bytes.push(b'\n');
            let compat_path = PathBuf::from(path);
            write_output(&compat_path, &compat_bytes)?;
            json!({
                "output": compat_path.display().to_string(),
                "sha256": sha256_bytes(&compat_bytes),
                "bytes": compat_bytes.len(),
                "pairs": pairs,
                "legacy_only_pairs": legacy_only_pairs,
            })
        }
        None => Value::Null,
    };

    Ok(json!({
        "output": output_path.display().to_string(),
        "sha256": sha256_bytes(&bytes),
        "bytes": bytes.len(),
        "counts": document["counts"].clone(),
        "compat": compat,
    }))
}

/// Emit the project's UDA definitions as a standalone table.
///
/// `all_attr_info.json` cannot carry these: UDAs live in a Dictionary database
/// rather than in `*vir.dat`, so the descriptor pipeline never sees them. The
/// consumer needs three things this table supplies and nothing else does —
/// which nouns a UDA applies to (`ELEL`), what it is called (`STUDNA`, or
/// `DYUDNA` for dynamically installed ones), and what it defaults to (`DFLT`
/// type-checked against `UTYP`). Most UDAs on a real element are the default:
/// nothing is stored on the element at all, and E3D prints the Dictionary
/// value.
///
/// Defaults are written in the same `AttrVal` tag shape `all_attr_info.json`
/// uses, so a consumer can deserialise both with one type.
fn emit_uda_table(options: &HashMap<String, String>) -> Result<Value, String> {
    let attlib_path = PathBuf::from(required(options, "attlib")?);
    let dicvir_path = PathBuf::from(required(options, "dicvir")?);
    let output_path = PathBuf::from(required(options, "output")?);
    let dictionaries = dictionary_databases(options)?;
    if dictionaries.is_empty() {
        return Err("emit-uda-table requires --dictionary-db or --dictionary-db-list".into());
    }

    let attlib = AttlibData::parse_file(&attlib_path)?;

    // MDB search order decides which definition wins a duplicated UKEY, so the
    // list is merged in the order it was given rather than sorted.
    let mut catalog = UdaCatalog::default();
    let mut sources = Vec::with_capacity(dictionaries.len());
    for path in &dictionaries {
        let path = Path::new(path);
        let one = UdaCatalog::read(path, &dicvir_path, &attlib)?;
        sources.push(json!({
            "file": file_name(path)?,
            "sha256": sha256_file(path)?,
            "definitions": one.definitions().len(),
            "issues": one.issues().len(),
        }));
        catalog.merge_in_order(one);
    }

    let mut definitions = Vec::with_capacity(catalog.definitions().len());
    let mut by_noun: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    let mut named = 0usize;
    let mut with_default = 0usize;
    let mut invalid_defaults = 0usize;
    for definition in catalog.definitions() {
        if definition.name.is_empty() {
            continue;
        }
        named += 1;
        let value_type = definition.value_type.as_deref().unwrap_or("");
        let att_type = uda_att_type(value_type);
        // A UDA the Dictionary gives no usable default reads as `unset` in
        // `q att`, not as its type's zero — `:TPress` is a REAL and E3D prints
        // `unset`, never `0`. Ordinary attributes go the other way (`JNTC 0`),
        // so the absence has to survive into the table rather than being
        // flattened here. `type_zero` stays available for typed consumers.
        let resolution = definition.resolve_default();
        let (default_val, default_source) = match &resolution {
            UdaDefaultResolution::Value(value) => {
                with_default += 1;
                (attr_val_json(value, att_type), "dictionary")
            }
            UdaDefaultResolution::Invalid { .. } => {
                invalid_defaults += 1;
                (Value::Null, "invalid")
            }
            UdaDefaultResolution::Unset => (Value::Null, "unset"),
        };
        for noun in &definition.applicable_nouns {
            by_noun.entry(*noun).or_default().push(definition.key);
        }
        definitions.push(json!({
            "ukey": definition.key,
            "old_key": definition.old_key,
            "name": definition.name,
            "attr_name": format!(":{}", definition.name),
            "element_name": definition.element_name,
            "dynamic_name": definition.dynamic_name,
            "is_pseudo": definition.is_pseudo,
            "value_type": definition.value_type,
            "att_type": att_type,
            "is_array": value_type.ends_with("AR"),
            "length": definition.length,
            "nouns": definition.applicable_nouns,
            "default_val": default_val,
            "default_source": default_source,
            "type_zero": att_type_zero(att_type),
            "default_text": definition.default_text,
        }));
    }
    definitions.sort_by_key(|entry| entry["ukey"].as_u64().unwrap_or_default());
    for keys in by_noun.values_mut() {
        keys.sort_unstable();
        keys.dedup();
    }

    let issues = catalog
        .issues()
        .iter()
        .map(|issue| json!({"refno": issue.refno.to_string(), "detail": issue.detail}))
        .collect::<Vec<_>>();

    let document = json!({
        "schema": "e3d-uda-table/v1",
        "generator": format!("e3d-io/{}", env!("CARGO_PKG_VERSION")),
        "sources": {
            "dictionaries": sources,
            "dicvir": source_file_json(&dicvir_path)?,
            "attlib": source_file_json(&attlib_path)?,
        },
        "counts": {
            "definitions": catalog.definitions().len(),
            "named": named,
            "with_dictionary_default": with_default,
            "invalid_defaults": invalid_defaults,
            "nouns": by_noun.len(),
            "issues": issues.len(),
        },
        "issues": issues,
        "by_noun": by_noun
            .iter()
            .map(|(noun, keys)| (noun.to_string(), json!(keys)))
            .collect::<serde_json::Map<_, _>>(),
        "definitions": definitions,
    });

    let mut bytes = serde_json::to_vec(&document).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    write_output(&output_path, &bytes)?;

    Ok(json!({
        "output": output_path.display().to_string(),
        "sha256": sha256_bytes(&bytes),
        "bytes": bytes.len(),
        "counts": document["counts"].clone(),
    }))
}

/// `UTYP` in the spelling the consumer's `DbAttributeType` uses. Array UDAs
/// keep their element type; `is_array` carries the rest.
fn uda_att_type(value_type: &str) -> &'static str {
    match value_type.trim_end_matches("AR") {
        "TEXT" => "STRING",
        "WORD" => "WORD",
        "LOG" => "BOOL",
        "INT" => "INTEGER",
        "REAL" => "DOUBLE",
        "REF" => "ELEMENT",
        _ => "Unknown",
    }
}

/// What the attribute reads as when the Dictionary declares no usable default.
fn att_type_zero(att_type: &str) -> Value {
    match att_type {
        "WORD" => json!({"WordType": "unset"}),
        "BOOL" => json!({"BoolType": false}),
        "INTEGER" => json!({"IntegerType": 0}),
        "DOUBLE" => json!({"DoubleType": 0.0}),
        "ELEMENT" => json!({"ElementType": ""}),
        _ => json!({"StringType": ""}),
    }
}

fn attr_val_json(value: &DescriptorValue, att_type: &str) -> Value {
    match value {
        DescriptorValue::Text(text) => json!({"StringType": text}),
        DescriptorValue::Word { raw, text } => json!({
            "WordType": text.clone().unwrap_or_else(|| db1_dehash(*raw).unwrap_or_default())
        }),
        DescriptorValue::Bool(flag) => json!({"BoolType": flag}),
        DescriptorValue::Int(number) => json!({"IntegerType": number}),
        DescriptorValue::Real(number) => json!({"DoubleType": number}),
        DescriptorValue::RefNo(refno) => json!({
            "ElementType": if refno.word0 == 0 && refno.word1 == 0 {
                String::new()
            } else {
                refno.to_string()
            }
        }),
        _ => att_type_zero(att_type),
    }
}

fn write_output(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("create output directory {}: {e}", parent.display()))?;
        }
    }
    fs::write(path, bytes).map_err(|e| format!("write {}: {e}", path.display()))
}

fn slot_json(slot: e3d_io::record::template::StorageDescriptor) -> Value {
    json!({
        "storage_words": slot.storage_words,
        "encoded_location": slot.location.raw,
        "word_offset": slot.location.word_offset(),
        "bool_bit": slot.location.bool_bit_index(),
        "flags": slot.flags,
    })
}

/// `DbAttributeType` variant name implied by the logical catalog.
///
/// The legacy file folds array-ness into the type name for integers and
/// element references (`INTVEC`, `RefU64Vec`) but not for doubles, so this
/// mirrors that quirk rather than normalising it — the goal is byte-level
/// agreement with the consumer's existing expectations, not a tidier taxonomy.
fn catalog_att_type(attribute: Option<&e3d_io::noun_catalog::CatalogAttribute>) -> String {
    let Some(attribute) = attribute else {
        return "Unknown".to_string();
    };
    match (attribute.type_name.as_str(), attribute.is_array) {
        ("INTEGER", true) => "INTVEC".to_string(),
        ("ELEMENT", true) => "RefU64Vec".to_string(),
        ("BOOLEAN", _) => "BOOL".to_string(),
        ("STRINGARRAY", _) => "STRING".to_string(),
        ("BLOB", _) => "Unknown".to_string(),
        (name, _) => name.to_string(),
    }
}

fn fallback_default(att_type: &str, is_array: bool) -> Value {
    if is_array {
        return match att_type {
            "INTEGER" | "INTVEC" => json!({ "IntArrayType": [] }),
            "DOUBLE" => json!({ "DoubleArrayType": [] }),
            "ELEMENT" | "RefU64Vec" => json!({ "RefU64Array": [] }),
            "STRING" => json!({ "StringArrayType": [] }),
            _ => json!({ "StringArrayType": [] }),
        };
    }
    match att_type {
        "BOOL" => json!({ "BoolType": false }),
        "INTEGER" => json!({ "IntegerType": 0 }),
        "DOUBLE" => json!({ "DoubleType": 0.0 }),
        "ELEMENT" => json!({ "ElementType": "" }),
        "WORD" => json!({ "WordType": "unset" }),
        "STRING" => json!({ "StringType": "" }),
        "POSITION" | "DIRECTION" | "ORIENTATION" => json!({ "Vec3Type": [0.0, 0.0, 0.0] }),
        "INTVEC" => json!({ "IntArrayType": [] }),
        "RefU64Vec" => json!({ "RefU64Array": [] }),
        _ => json!({ "StringType": "" }),
    }
}

/// The previous `all_attr_info.json`, read only so its hand-curated
/// `att_type` and `default_val` survive the migration untouched.
#[derive(Default)]
struct LegacyAttrInfo {
    entries: HashMap<(u32, u32), (String, Value)>,
    full_entries: HashMap<(u32, u32), Value>,
}

impl LegacyAttrInfo {
    fn read(path: &str) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        let root: Value = serde_json::from_str(&text).map_err(|e| format!("parse {path}: {e}"))?;
        let map = root
            .get("noun_attr_info_map")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("{path} has no noun_attr_info_map object"))?;
        let mut entries = HashMap::new();
        let mut full_entries = HashMap::new();
        for (noun_key, attrs) in map {
            let Ok(noun) = noun_key.parse::<i64>() else {
                continue;
            };
            let Some(attrs) = attrs.as_object() else {
                continue;
            };
            for (attr_key, info) in attrs {
                let Ok(attr) = attr_key.parse::<i64>() else {
                    continue;
                };
                let att_type = info
                    .get("att_type")
                    .and_then(Value::as_str)
                    .unwrap_or("Unknown")
                    .to_string();
                let default_val = info.get("default_val").cloned().unwrap_or(Value::Null);
                entries.insert((noun as u32, attr as u32), (att_type, default_val));
                full_entries.insert((noun as u32, attr as u32), info.clone());
            }
        }
        Ok(Self {
            entries,
            full_entries,
        })
    }

    fn att_type(&self, noun: u32, attr: u32) -> Option<&str> {
        self.entries
            .get(&(noun, attr))
            .map(|(att_type, _)| att_type.as_str())
    }

    fn default_val(&self, noun: u32, attr: u32) -> Option<&Value> {
        self.entries
            .get(&(noun, attr))
            .map(|(_, default_val)| default_val)
    }

    /// Learn `(att_type, DEFI, DFDP, SIZE) -> default_val` from the legacy
    /// file instead of hard-coding it. Combinations the legacy file disagrees
    /// with itself about are dropped, so a learned rule is only ever applied
    /// where the evidence was unanimous.
    fn learn_default_rule(&self, catalog: &NounCatalog) -> DefaultRule {
        let mut votes: HashMap<String, HashSet<String>> = HashMap::new();
        let mut sample: HashMap<String, Value> = HashMap::new();
        for (&(_, attr), (att_type, value)) in &self.entries {
            let key = DefaultRule::key(att_type, catalog.attribute_fields(attr));
            votes
                .entry(key.clone())
                .or_default()
                .insert(value.to_string());
            sample.entry(key).or_insert_with(|| value.clone());
        }
        let table = votes
            .into_iter()
            .filter(|(_, values)| values.len() == 1)
            .filter_map(|(key, _)| sample.remove(&key).map(|value| (key, value)))
            .collect();
        DefaultRule { table }
    }
}

struct DefaultRule {
    table: HashMap<String, Value>,
}

impl DefaultRule {
    fn key(att_type: &str, fields: Option<&e3d_io::noun_catalog::AttributeFieldRecord>) -> String {
        let field = |name: &str| -> String {
            fields
                .and_then(|record| record.fields.get(name))
                .and_then(|value| value.get("i"))
                .and_then(Value::as_i64)
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string())
        };
        format!(
            "{att_type}|{}|{}|{}",
            field("DEFI"),
            field("DFDP"),
            field("SIZE")
        )
    }

    fn lookup(
        &self,
        att_type: &str,
        fields: Option<&e3d_io::noun_catalog::AttributeFieldRecord>,
    ) -> Option<&Value> {
        self.table.get(&Self::key(att_type, fields))
    }
}

fn source_file_json(path: &Path) -> Result<Value, String> {
    Ok(json!({
        "file": file_name(path)?,
        "sha256": sha256_file(path)?,
        "bytes": fs::metadata(path)
            .map_err(|e| format!("stat {}: {e}", path.display()))?
            .len(),
    }))
}

fn file_name(path: &Path) -> Result<String, String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(String::from)
        .ok_or_else(|| format!("path has no UTF-8 file name: {}", path.display()))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("read {} for hash: {e}", path.display()))?;
    Ok(sha256_bytes(&bytes))
}

fn sha256_words(words: &[u32]) -> String {
    let mut hasher = Sha256::new();
    for word in words {
        hasher.update(word.to_be_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn complete_extraction_json(
    extraction: &ElementExtraction,
    rows: &[CompleteAttributeRow],
    catalog: Option<&NounCatalog>,
    metadata: &AttributeMetadataCatalog,
    uda_enabled: bool,
    deferred_uda: &[e3d_io::tty::TtyAttributeRow],
) -> Value {
    let mut source_counts: HashMap<&'static str, usize> = HashMap::new();
    let mut comparison_counts: HashMap<&'static str, usize> = HashMap::new();
    for row in rows {
        *source_counts.entry(source_name(row.source)).or_default() += 1;
        *comparison_counts
            .entry(comparison_name(row.comparison))
            .or_default() += 1;
    }
    let unique_count = rows
        .iter()
        .map(|row| row.name.to_ascii_lowercase())
        .collect::<std::collections::HashSet<_>>()
        .len();
    let tty_names = rows
        .iter()
        .map(|row| row.name.to_ascii_lowercase())
        .collect::<std::collections::HashSet<_>>();
    let catalog_noun = catalog.and_then(|catalog| catalog.noun(extraction.noun_hash));
    let catalog_names = catalog_noun
        .map(|noun| {
            noun.attrs
                .iter()
                .map(|attr| {
                    metadata
                        .report_name(attr.hash)
                        .unwrap_or(&attr.name)
                        .to_ascii_lowercase()
                })
                .collect::<std::collections::HashSet<_>>()
        })
        .unwrap_or_default();
    let catalog_missing_from_tty = catalog_noun
        .map(|noun| {
            noun.attrs
                .iter()
                .filter_map(|attr| {
                    let report_name = metadata.report_name(attr.hash).unwrap_or(&attr.name);
                    (!tty_names.contains(&report_name.to_ascii_lowercase())).then(|| {
                        json!({"hash": attr.hash, "name": attr.name, "report_name": report_name})
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let tty_not_in_catalog = rows
        .iter()
        .filter(|row| !catalog_names.contains(&row.name.to_ascii_lowercase()))
        .map(|row| row.name.clone())
        .collect::<Vec<_>>();
    json!({
        "refno": {
            "word0": extraction.refno.word0,
            "word1": extraction.refno.word1,
            "display": extraction.refno.to_string(),
        },
        "owner": extraction.owner.to_string(),
        "noun_hash": extraction.noun_hash,
        "noun_name": extraction.noun_name,
        "tty_row_count": rows.len(),
        "tty_unique_count": unique_count,
        "omitted_row_count": 0,
        "uda_enabled": uda_enabled,
        "deferred_uda_row_count": deferred_uda.len(),
        "deferred_uda_rows": deferred_uda.iter().map(|row| json!({
            "name": row.name,
            "rendered_value": row.rendered_value,
        })).collect::<Vec<_>>(),
        "source_counts": source_counts,
        "comparison_counts": comparison_counts,
        "catalog_comparison": {
            "enabled": catalog.is_some(),
            "noun_present": catalog_noun.is_some(),
            "logical_attribute_count": catalog_noun.map(|noun| noun.attrs.len()),
            "catalog_missing_from_tty": catalog_missing_from_tty,
            "tty_not_in_catalog": tty_not_in_catalog,
        },
        "attributes": rows.iter().map(|row| json!({
            "ordinal": row.ordinal,
            "name": row.name,
            "rendered_value": row.rendered_value,
            "source": source_name(row.source),
            "binary_name": row.binary_name,
            "binary_status": row.binary_status.as_ref().map(status_json),
            "binary_value": row.binary_value.as_ref().map(value_json),
            "resolved_reference_name": row.resolved_reference_name,
            "comparison": comparison_name(row.comparison),
        })).collect::<Vec<_>>(),
    })
}

fn comparison_name(comparison: ValueComparison) -> &'static str {
    match comparison {
        ValueComparison::Equivalent => "equivalent",
        ValueComparison::NeedsReferenceResolution => "needs_reference_resolution",
        ValueComparison::Mismatch => "mismatch",
        ValueComparison::MissingBinaryValue => "missing_binary_value",
    }
}

fn source_name(source: CompleteAttributeSource) -> &'static str {
    match source {
        CompleteAttributeSource::BinaryImplicit => "binary_implicit",
        CompleteAttributeSource::BinaryExplicit => "binary_explicit",
        CompleteAttributeSource::CatalogDefault => "catalog_default",
        CompleteAttributeSource::RuntimeDerived => "runtime_derived",
        CompleteAttributeSource::BinaryUnresolved => "binary_unresolved",
        CompleteAttributeSource::Structural => "structural",
        CompleteAttributeSource::TtyAuthority => "tty_authority",
    }
}

fn project_dabacon_tags_json(report: e3d_io::ProjectDabaconTagReport) -> Value {
    let e3d_io::ProjectDabaconTagReport {
        root,
        discovered_databases,
        databases,
        issues,
        latest_tag_database_counts,
        historical_tag_session_counts,
        historical_tag_database_counts,
        latest_untagged_databases,
        historical_untagged_databases,
        historical_untagged_sessions,
    } = report;
    let readable_databases = databases.len();
    let issue_count = issues.len();
    let databases = databases
        .into_iter()
        .map(|database| {
            let e3d_io::DatabaseDabaconTagReport {
                database,
                dbnum,
                db_type_hash,
                db_type_name,
                latest_session_number,
                latest_dabacon_version_tag,
                retained_sessions,
                historical_tag_session_counts,
                untagged_retained_sessions,
            } = database;
            let retained_session_count = retained_sessions.len();
            let sessions = retained_sessions
                .into_iter()
                .map(|session| {
                    json!({
                        "session_number": session.session_number,
                        "session_page": session.session_page,
                        "dabacon_version_tag": session.dabacon_version_tag,
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "database": database,
                "dbnum": dbnum,
                "db_type_hash": db_type_hash,
                "db_type": db_type_name,
                "latest_session_number": latest_session_number,
                "latest_dabacon_version_tag": latest_dabacon_version_tag,
                "retained_session_count": retained_session_count,
                "historical_tag_session_counts": historical_tag_session_counts,
                "untagged_retained_sessions": untagged_retained_sessions,
                "sessions": sessions,
            })
        })
        .collect::<Vec<_>>();
    let issues = issues
        .into_iter()
        .map(|issue| {
            json!({
                "database": issue.database,
                "detail": issue.detail,
            })
        })
        .collect::<Vec<_>>();
    let tagged_historical_sessions: usize = historical_tag_session_counts.values().sum();

    json!({
        "project_root": root,
        "release_selection": "diagnostic_only",
        "discovered_databases": discovered_databases,
        "readable_databases": readable_databases,
        "issue_count": issue_count,
        "issues": issues,
        "latest": {
            "tag_database_counts": latest_tag_database_counts,
            "untagged_databases": latest_untagged_databases,
        },
        "history": {
            "tag_session_counts": historical_tag_session_counts,
            "tag_database_counts": historical_tag_database_counts,
            "tagged_sessions": tagged_historical_sessions,
            "untagged_databases": historical_untagged_databases,
            "untagged_sessions": historical_untagged_sessions,
        },
        "databases": databases,
    })
}

/// `sessions --db PATH`: the retained session chain, newest first.
///
/// The chain is what a range can be cut at. `MERGE CHANGES` leaves holes —
/// `ams7000_0001` keeps 1, 237, 238, 262 and 263 of its 263 sessions — and a
/// caller choosing `--from` needs to see them before `diff` refuses a number
/// that is not there.
fn sessions_command(options: &HashMap<String, String>) -> Result<Value, String> {
    let db = required(options, "db")?;
    let mut view = DbView::open(db).map_err(|e| format!("open database {db}: {e}"))?;
    let chain = view
        .walk_sessions()
        .map_err(|e| format!("walk the session chain of {db}: {e}"))?;
    let db_type = db_type_name(&mut view);
    let descriptor = view.descriptor();
    Ok(json!({
        "db": db,
        "db_num": descriptor.db_mark,
        "db_type": db_type,
        "newest": chain.first().map(|s| s.sesno),
        "oldest": chain.last().map(|s| s.sesno),
        "retained": chain.len(),
        "sessions": chain.iter().map(session_json).collect::<Vec<_>>(),
    }))
}

/// The database type (`DESI`, `CATA`, …): the db1 hash at +0x20 of page 0,
/// which is what says which `*vir.dat` decodes this file's records.
fn db_type_name(view: &mut DbView) -> Option<String> {
    let page0 = view.pages_mut().get_page(0).ok()?;
    let hash = u32::from_be_bytes(page0.get(0x20..0x24)?.try_into().ok()?);
    db1_dehash(hash)
}

fn session_json(session: &e3d_io::session::SessionPage) -> Value {
    json!({
        "sesno": session.sesno,
        "page": session.page.page,
        "previous_page": session.previous.map(|p| p.page),
        "index_root": session.index_root.page,
        "end_page": session.end.page,
        "user": session.user,
        "comment": session.comment,
        "date": session.date.map(|date| date.to_string()),
        "utc_date": session.utc_date.map(|date| date.to_string()),
        "zone_offset_seconds": session.zone_offset_seconds(),
        "stamp_words": session.stamp_words,
        "tail": session.tail.map(|tail| {
            json!({
                "runum": tail.runum,
                "memext": tail.memext,
                "mempps": tail.mempps,
                "base_sesno": tail.base_sesno,
                "reason": tail.reason,
                "linkps": tail.linkps,
            })
        }),
    })
}

/// `spamap --db PATH [--session N] [--refno WORD0/WORD1]`: one session's SPAMAP
/// table — its root, how many pages hang off it, and every entry or one
/// refno's, each as the eleven words stored.
fn spamap_command(options: &HashMap<String, String>) -> Result<Value, String> {
    let db = required(options, "db")?;
    let at = match options.get("session") {
        Some(sesno) => SessionSelector::Exact(sesno.parse().map_err(|e| format!("invalid --session {sesno}: {e}"))?),
        None => SessionSelector::Latest,
    };
    let refno = options.get("refno").map(|refno| parse_refno(refno)).transpose()?;
    let mut view = DbView::open_at(db, at).map_err(|e| format!("open database {db}: {e}"))?;
    match read_spamap(&mut view).map_err(|e| format!("read the SPAMAP of {db}: {e}"))? {
        SpamapRead::NoTable { sesno } => Ok(json!({ "db": db, "sesno": sesno, "spamap": "no_table" })),
        SpamapRead::Table { sesno, table } => {
            let entries: Vec<Value> = match refno {
                Some(refno) => table.for_refno(refno).map(entry_words).collect(),
                None => table.entries().iter().map(entry_words).collect(),
            };
            Ok(json!({
                "db": db,
                "sesno": sesno,
                "root_page": table.root.page,
                "pages": table.pages.len(),
                "entries_in_table": table.entries().len(),
                "refno": refno.map(|refno| refno.to_string()),
                "entries": entries,
            }))
        }
    }
}

/// An entry as stored: four key words, then seven value words, unsigned.
fn entry_words(entry: &SpamapEntry) -> Value {
    json!(entry.key.iter().chain(&entry.value).copied().collect::<Vec<u32>>())
}

/// What the census says about how it reads Key0; printed with every census so
/// the histograms carry their own definition.
const CENSUS_KEY0_READING: &str = "key0 = N * 10^8 + min, min the signed 0.1 mm minimum the value stores for one axis, \
     N = 6 * axis (x 0, y 1, z 2) + bucket (1..5); an entry no axis reads that way counts as key0_mismatch \
     (docs/evidence/2026-09-22-e3d-spamap-consequences.md, section 0 item 7)";

/// How many entries of the first kind a census lists in full.
const CENSUS_LISTED: usize = 100;

/// `spamap-census --input FILE_OR_DIR [--input …] [--session FILE=N …]`
/// (requirement 3.7–3.12): per database, the SPAMAP table's size and, per noun
/// of the session's refno index, how many entries its elements have and what
/// those entries say.
///
/// Read only. The same inputs at the same sessions print the same bytes:
/// inputs sorted, every tally a `BTreeMap`, no clock. A file in an input
/// directory whose first page is no database descriptor is listed as not a
/// database; a database that cannot be read is listed with its error, the
/// census goes on, and the exit code is then 1.
fn spamap_census_command(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut inputs: Vec<PathBuf> = Vec::new();
    let mut sessions: BTreeMap<PathBuf, u32> = BTreeMap::new();
    while let Some(flag) = args.next() {
        let value = args.next().ok_or_else(|| format!("missing value for {flag}"))?;
        match flag.as_str() {
            "--input" => inputs.push(PathBuf::from(value)),
            "--session" => {
                let (file, sesno) =
                    value.rsplit_once('=').ok_or_else(|| format!("--session takes FILE=N, got {value}"))?;
                let sesno = sesno.parse().map_err(|e| format!("--session {value}: {e}"))?;
                sessions.insert(PathBuf::from(file), sesno);
            }
            other => return Err(format!("spamap-census takes --input and --session, got {other}")),
        }
    }
    if inputs.is_empty() {
        return Err("spamap-census needs at least one --input".into());
    }

    let mut databases: Vec<PathBuf> = Vec::new();
    let mut not_databases: Vec<String> = Vec::new();
    let mut errors: Vec<(String, String)> = Vec::new();
    for input in &inputs {
        if !input.is_dir() {
            databases.push(input.clone());
            continue;
        }
        let listing = match fs::read_dir(input) {
            Ok(listing) => listing,
            Err(error) => {
                errors.push((display_path(input), format!("list the directory: {error}")));
                continue;
            }
        };
        for entry in listing {
            let path = match entry {
                Ok(entry) => entry.path(),
                Err(error) => {
                    errors.push((display_path(input), format!("list the directory: {error}")));
                    continue;
                }
            };
            if !path.is_file() {
                continue;
            }
            match starts_with_a_database_descriptor(&path) {
                Ok(true) => databases.push(path),
                Ok(false) => not_databases.push(display_path(&path)),
                Err(error) => errors.push((display_path(&path), error)),
            }
        }
    }
    databases.sort();
    databases.dedup();
    not_databases.sort();
    if let Some(stray) = sessions.keys().find(|file| !databases.contains(file)) {
        return Err(format!("--session names {}, which is not a database among the inputs", stray.display()));
    }

    let mut reports = Vec::new();
    for path in &databases {
        match census_database(path, sessions.get(path).copied()) {
            Ok(report) => reports.push(report),
            Err(error) => errors.push((display_path(path), error)),
        }
    }
    errors.sort();
    let summary = json!({
        "key0_reading": CENSUS_KEY0_READING,
        "databases": reports,
        "not_databases": not_databases,
        "errors": errors.iter().map(|(path, error)| json!({ "path": path, "error": error })).collect::<Vec<_>>(),
    });
    println!("{}", serde_json::to_string_pretty(&summary).map_err(|e| e.to_string())?);
    if !errors.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

/// Whether `path` opens with a page 0 that parses as a database descriptor.
fn starts_with_a_database_descriptor(path: &Path) -> Result<bool, String> {
    use std::io::Read;
    let mut head = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(4096).read_to_end(&mut head))
        .map_err(|e| format!("read the first page: {e}"))?;
    Ok(e3d_io::DbDescriptor::parse(&head).is_ok())
}

/// One database's census, at `session` or the newest.
fn census_database(path: &Path, session: Option<u32>) -> Result<Value, String> {
    let at = session.map_or(SessionSelector::Latest, SessionSelector::Exact);
    let mut view = DbView::open_at(path, at).map_err(|e| format!("open: {e}"))?;
    let sesno = view.selected_session();
    let spamap = read_spamap(&mut view).map_err(|e| format!("read the SPAMAP: {e}"))?;
    let mut engine = ReadOnlyEngine::open_at(path, sesno).map_err(|e| format!("open the engine: {e}"))?;
    let mut nouns: HashMap<RefNo, u32> = HashMap::new();
    for element in engine.scan_elements(ScanTier::Header).map_err(|e| format!("walk the refno index: {e}"))? {
        let element = element.map_err(|e| format!("read an element record: {e}"))?;
        nouns.insert(element.refno, element.noun_hash);
    }

    let (table, entries): (Value, &[SpamapEntry]) = match &spamap {
        SpamapRead::NoTable { .. } => (json!("no_table"), &[]),
        SpamapRead::Table { table, .. } => (
            json!({ "root_page": table.root.page, "pages": table.pages.len(), "entries": table.entries().len() }),
            table.entries(),
        ),
    };
    let mut owned: HashMap<RefNo, Vec<&SpamapEntry>> = HashMap::new();
    let mut dangling: Vec<&SpamapEntry> = Vec::new();
    let mut key0_mismatch: Vec<&SpamapEntry> = Vec::new();
    for entry in entries {
        if !nouns.contains_key(&entry.refno()) {
            dangling.push(entry);
            continue;
        }
        if census_axis_bucket(entry).is_none() {
            key0_mismatch.push(entry);
        }
        owned.entry(entry.refno()).or_default().push(entry);
    }

    #[derive(Default)]
    struct NounTally {
        elements: u64,
        by_entry_count: [u64; 4],
        axis_bucket: BTreeMap<String, u64>,
        flags: BTreeMap<u32, u64>,
        k: BTreeMap<u32, u64>,
        k_combinations: BTreeMap<String, u64>,
    }
    let mut tallies: BTreeMap<String, NounTally> = BTreeMap::new();
    for (refno, &noun) in &nouns {
        let tally = tallies.entry(db1_dehash(noun).unwrap_or_else(|| format!("#{noun}"))).or_default();
        tally.elements += 1;
        let mine = owned.get(refno).map_or(&[][..], Vec::as_slice);
        tally.by_entry_count[mine.len().min(3)] += 1;
        if mine.is_empty() {
            continue;
        }
        let mut ks: Vec<u32> = mine.iter().map(|entry| entry.k()).collect();
        ks.sort_unstable();
        let combination = ks.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        *tally.k_combinations.entry(combination).or_default() += 1;
        for entry in mine {
            let cell = match census_axis_bucket(entry) {
                Some((axis, bucket)) => format!("{}{bucket}", ["x", "y", "z"][axis]),
                None => "key0_mismatch".to_owned(),
            };
            *tally.axis_bucket.entry(cell).or_default() += 1;
            *tally.flags.entry(entry.flags()).or_default() += 1;
            *tally.k.entry(entry.k()).or_default() += 1;
        }
    }

    let counted = |tally: &BTreeMap<u32, u64>| -> BTreeMap<String, u64> {
        tally.iter().map(|(value, count)| (value.to_string(), *count)).collect()
    };
    // The same three histograms over every entry of the table, dangling ones
    // included: what the table holds, whatever the index says.
    let mut totals_axis_bucket: BTreeMap<String, u64> = BTreeMap::new();
    let mut totals_flags: BTreeMap<u32, u64> = BTreeMap::new();
    let mut totals_k: BTreeMap<u32, u64> = BTreeMap::new();
    for entry in entries {
        let cell = match census_axis_bucket(entry) {
            Some((axis, bucket)) => format!("{}{bucket}", ["x", "y", "z"][axis]),
            None => "key0_mismatch".to_owned(),
        };
        *totals_axis_bucket.entry(cell).or_default() += 1;
        *totals_flags.entry(entry.flags()).or_default() += 1;
        *totals_k.entry(entry.k()).or_default() += 1;
    }
    let nouns_json: BTreeMap<String, Value> = tallies
        .iter()
        .map(|(noun, tally)| {
            let [none, one, two, more] = tally.by_entry_count;
            let report = json!({
                "elements": tally.elements,
                "elements_by_entry_count": { "0": none, "1": one, "2": two, "3+": more },
                "axis_bucket": tally.axis_bucket,
                "flags": counted(&tally.flags),
                "k": counted(&tally.k),
                "k_combinations": tally.k_combinations,
            });
            (noun.clone(), report)
        })
        .collect();
    Ok(json!({
        "path": display_path(path),
        "sesno": sesno,
        "spamap": table,
        "elements": nouns.len(),
        "dangling": {
            "entries": dangling.len(),
            "first": dangling.iter().take(CENSUS_LISTED).map(|entry| entry_words(entry)).collect::<Vec<_>>(),
        },
        "key0_mismatch": {
            "entries": key0_mismatch.len(),
            "first": key0_mismatch.iter().take(CENSUS_LISTED).map(|entry| entry_words(entry)).collect::<Vec<_>>(),
        },
        "totals": {
            "axis_bucket": totals_axis_bucket,
            "flags": counted(&totals_flags),
            "k": counted(&totals_k),
        },
        "nouns": nouns_json,
    }))
}

/// `spamap-check --input FILE_OR_DIR … [--session FILE=N …] (--schema PACK |
/// --desvir PATH --attlib PATH) [--rules confirmed|candidates] [--exclude
/// FILE] [--refno W0/W1]` (requirements 4.10, 4.11, 4.15–4.17): per database,
/// the codec under the chosen rules held against its SPAMAP table in three
/// items — Key0 from each entry's own words, entry count and k, flags — each
/// with agree / total, how many misses wait on a pending rule, by noun, and
/// its first misses with the rules they wait on. `--exclude` names elements taken out of the count and flags items,
/// one `DATABASE<TAB>W0/W1<TAB>evidence` per line. Inputs as `spamap-census`
/// takes them; a database that cannot be read is listed, and the exit code is
/// then 1.
fn spamap_check_command(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut inputs: Vec<PathBuf> = Vec::new();
    let mut sessions: BTreeMap<PathBuf, u32> = BTreeMap::new();
    let mut options: HashMap<String, String> = HashMap::new();
    while let Some(flag) = args.next() {
        let value = args.next().ok_or_else(|| format!("missing value for {flag}"))?;
        match flag.as_str() {
            "--input" => inputs.push(PathBuf::from(value)),
            "--session" => {
                let (file, sesno) =
                    value.rsplit_once('=').ok_or_else(|| format!("--session takes FILE=N, got {value}"))?;
                sessions.insert(PathBuf::from(file), sesno.parse().map_err(|e| format!("--session {value}: {e}"))?);
            }
            "--schema" | "--desvir" | "--attlib" | "--rules" | "--exclude" | "--refno" => {
                options.insert(flag.trim_start_matches("--").to_owned(), value);
            }
            other => return Err(format!("spamap-check does not take {other}")),
        }
    }
    if inputs.is_empty() {
        return Err("spamap-check needs at least one --input".into());
    }
    let (desvir, attlib) = match (options.get("schema"), options.get("desvir"), options.get("attlib")) {
        (Some(pack), None, None) => {
            let pack = SchemaPack::open(pack).map_err(|e| format!("open schema pack: {e}"))?;
            let desvir = pack.template("DESI").map_err(|e| format!("schema pack template: {e}"))?;
            let attlib = pack.attlib().map_err(|e| format!("schema pack attlib: {e}"))?;
            (desvir, attlib)
        }
        (None, Some(desvir), Some(attlib)) => (PathBuf::from(desvir), PathBuf::from(attlib)),
        _ => return Err("spamap-check needs --schema PACK_DIR, or --desvir and --attlib".into()),
    };
    let candidates = e3d_io::spatial::candidates::pick();
    let (rules_name, rules) = match options.get("rules").map(String::as_str).unwrap_or("confirmed") {
        "confirmed" => ("confirmed", *e3d_io::core_rules::RuleSet::confirmed()),
        "candidates" => ("candidates", e3d_io::core_rules::RuleSet::candidates(&candidates)),
        other => return Err(format!("--rules takes confirmed or candidates, got {other}")),
    };
    let only = options.get("refno").map(|refno| parse_refno(refno)).transpose()?;
    let exclusions = match options.get("exclude") {
        Some(file) => read_exclusions(Path::new(file))?,
        None => Vec::new(),
    };

    let mut databases: Vec<PathBuf> = Vec::new();
    let mut errors: Vec<(String, String)> = Vec::new();
    for input in &inputs {
        if !input.is_dir() {
            databases.push(input.clone());
            continue;
        }
        match fs::read_dir(input) {
            Ok(listing) => {
                for path in listing.filter_map(Result::ok).map(|entry| entry.path()).filter(|path| path.is_file()) {
                    match starts_with_a_database_descriptor(&path) {
                        Ok(true) => databases.push(path),
                        Ok(false) => {}
                        Err(error) => errors.push((display_path(&path), error)),
                    }
                }
            }
            Err(error) => errors.push((display_path(input), format!("list the directory: {error}"))),
        }
    }
    databases.sort();
    databases.dedup();

    let mut reports = Vec::new();
    for path in &databases {
        let check = SpamapCheck { rules: &rules, desvir: &desvir, attlib: &attlib, exclusions: &exclusions, only };
        match check.database(path, sessions.get(path).copied()) {
            Ok(report) => reports.push(report),
            Err(error) => errors.push((display_path(path), error)),
        }
    }
    errors.sort();
    let summary = json!({
        "rules": rules_name,
        "databases": reports,
        "errors": errors.iter().map(|(path, error)| json!({ "path": path, "error": error })).collect::<Vec<_>>(),
    });
    println!("{}", serde_json::to_string_pretty(&summary).map_err(|e| e.to_string())?);
    if !errors.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

/// One `--exclude` line: the database's file name, the element, the
/// session-chain evidence that the entry predates a change that does not
/// recompute it (requirement 4.17).
fn read_exclusions(file: &Path) -> Result<Vec<(String, RefNo, String)>, String> {
    let text = fs::read_to_string(file).map_err(|e| format!("read {}: {e}", file.display()))?;
    let mut exclusions = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.splitn(3, '\t');
        let (Some(database), Some(refno), Some(evidence)) = (fields.next(), fields.next(), fields.next()) else {
            return Err(format!("{}:{}: expected DATABASE<TAB>W0/W1<TAB>evidence", file.display(), number + 1));
        };
        exclusions.push((database.to_owned(), parse_refno(refno)?, evidence.to_owned()));
    }
    Ok(exclusions)
}

/// What one `spamap-check` run holds every database to.
struct SpamapCheck<'a> {
    rules: &'a e3d_io::core_rules::RuleSet,
    desvir: &'a Path,
    attlib: &'a Path,
    exclusions: &'a [(String, RefNo, String)],
    only: Option<RefNo>,
}

impl SpamapCheck<'_> {
    fn database(&self, path: &Path, session: Option<u32>) -> Result<Value, String> {
        use e3d_io::spatial::check::{check, Authors};
        let at = session.map_or(SessionSelector::Latest, SessionSelector::Exact);
        let mut view = DbView::open_at(path, at).map_err(|e| format!("open: {e}"))?;
        let sesno = view.selected_session();
        let authors = Authors::from_sessions(&view.walk_sessions().map_err(|e| format!("walk the sessions: {e}"))?);
        let table = match read_spamap(&mut view).map_err(|e| format!("read the SPAMAP: {e}"))? {
            SpamapRead::NoTable { .. } => return Ok(json!({ "path": display_path(path), "sesno": sesno, "spamap": "no_table" })),
            SpamapRead::Table { table, .. } => table,
        };
        let attlib = AttlibData::parse_file(self.attlib).map_err(|e| format!("attlib: {e}"))?;
        let templates =
            TemplateProvider::open(self.desvir, Some(DESIGN_TEMPLATE_TYPE)).map_err(|e| format!("templates: {e}"))?;
        let state = e3d_io::spatial::state::ReadState::open(path, sesno, attlib, templates)
            .map_err(|e| format!("open the element state: {e}"))?;
        let entries: Vec<SpamapEntry> = match self.only {
            Some(refno) => table.for_refno(refno).copied().collect(),
            None => table.entries().to_vec(),
        };
        let database = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let authored = |refno: RefNo| authors.e3d_wrote(&state, refno);
        let excluded = |refno: RefNo| self.exclusions.iter().any(|(db, excluded, _)| *db == database && *excluded == refno);
        let codec = e3d_io::spatial::codec::SpamapCodec::new(self.rules);
        let checked = check(&entries, &state, &codec, &authored, &excluded);
        let item = |item: &e3d_io::spatial::check::Item| {
            json!({
                "agree": item.agree,
                "total": item.total,
                "waiting": item.waiting,
                "by_noun": item.by_noun.iter().map(|(noun, (agree, total))| (noun.clone(), json!([agree, total]))).collect::<BTreeMap<_, _>>(),
                "first": item.misses.iter().map(|miss| json!({
                    "stored": miss.stored.iter().map(entry_words).collect::<Vec<_>>(),
                    "codec": miss.codec,
                    "reasons": miss.reasons,
                })).collect::<Vec<_>>(),
            })
        };
        Ok(json!({
            "path": display_path(path),
            "sesno": sesno,
            "refno": self.only.map(|refno| refno.to_string()),
            "entries": entries.len(),
            "authored_elements": checked.authored_elements,
            "excluded_elements": checked.excluded_elements,
            "unreadable_records": state.failures().iter().map(|(refno, why)| json!([refno.to_string(), why])).collect::<Vec<_>>(),
            "key0": item(&checked.key0),
            "entries_k": item(&checked.entries_k),
            "flags": item(&checked.flags),
        }))
    }
}

/// The census's own reading of Key0 ([`CENSUS_KEY0_READING`]), as
/// `(axis, bucket)`; `None` is a Key0 mismatch. Independent of the codec's
/// rules, confirmed or not.
fn census_axis_bucket(entry: &SpamapEntry) -> Option<(usize, i64)> {
    const STEP: i64 = 100_000_000;
    let key0 = i64::from(entry.key0());
    (0..3).find_map(|axis| {
        let rest = key0 - i64::from(entry.value[axis] as i32);
        if rest < 0 || rest % STEP != 0 {
            return None;
        }
        let n = rest / STEP;
        (n / 6 == axis as i64 && (1..=5).contains(&(n % 6))).then_some((axis, n % 6))
    })
}

/// `diff --db PATH --from N [--to N] [--at-or-before on] [--steps on]
/// [--content on] [--limit N] [--desvir PATH --attlib PATH [--template-type T]]`.
///
/// Without `--content` the output is the key set with record addresses,
/// which is what the index says and costs only index pages. `--content on`
/// reads every changed record on the side(s) it exists on and adds the
/// template-free delta; with `--desvir` and `--attlib` the delta also carries
/// the descriptor-decoded attribute rows. `--limit` caps how many changes get
/// content, because one range of `ams7000_0001` is 69 076 of them.
fn diff_command(options: &HashMap<String, String>) -> Result<Value, String> {
    let db = required(options, "db")?;
    let snap = on_off(options, "at-or-before")?;
    let selector = |name: &str| -> Result<Option<SessionSelector>, String> {
        options
            .get(name)
            .map(|value| {
                let sesno = value
                    .parse::<u32>()
                    .map_err(|e| format!("--{name} takes a session number: {e}"))?;
                Ok(if snap {
                    SessionSelector::AtOrBefore(sesno)
                } else {
                    SessionSelector::Exact(sesno)
                })
            })
            .transpose()
    };
    let from = selector("from")?.ok_or("missing --from")?;
    let to = selector("to")?.unwrap_or(SessionSelector::Latest);
    let steps = on_off(options, "steps")?;
    let content = on_off(options, "content")?;
    let limit = options
        .get("limit")
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|e| format!("--limit takes a count: {e}"))
        })
        .transpose()?
        .unwrap_or(usize::MAX);

    let mut view = DbView::open_at_with_cache(db, SessionSelector::Latest, 16_384)
        .map_err(|e| format!("open database {db}: {e}"))?;

    let mut provider = match (options.get("desvir"), options.get("attlib")) {
        (Some(desvir), Some(attlib_path)) => {
            let template_type = options
                .get("template-type")
                .map(|value| parse_noun(value))
                .transpose()?
                .unwrap_or(DESIGN_TEMPLATE_TYPE);
            let attlib = AttlibData::parse_file(attlib_path)?;
            let provider = TemplateProvider::open(desvir, Some(template_type))
                .map_err(|e| format!("open descriptor template: {e}"))?;
            Some((attlib, provider))
        }
        (None, None) => None,
        _ => return Err("--desvir and --attlib go together".into()),
    };

    if steps {
        let steps = view
            .diff_steps(from, to)
            .map_err(|e| format!("diff {db}: {e}"))?;
        let mut out = Vec::with_capacity(steps.len());
        for step in &steps {
            out.push(diff_json(
                &mut view,
                step,
                content,
                limit,
                provider.as_mut(),
            )?);
        }
        return Ok(json!({
            "db": db,
            "steps": out,
        }));
    }

    let diff = view
        .diff_sessions(from, to)
        .map_err(|e| format!("diff {db}: {e}"))?;
    let mut out = diff_json(&mut view, &diff, content, limit, provider.as_mut())?;
    out["db"] = json!(db);
    Ok(out)
}

fn diff_json(
    view: &mut DbView,
    diff: &SessionDiff,
    content: bool,
    limit: usize,
    mut templates: Option<&mut (AttlibData, TemplateProvider)>,
) -> Result<Value, String> {
    let tally = diff.tally();
    let mut changes = Vec::with_capacity(diff.changes().len().min(limit));
    if content {
        let text_format = view.text_format();
        for element in view.changed_elements(diff).take(limit) {
            let element = element.map_err(|e| format!("read a changed record: {e}"))?;
            let mut delta =
                ElementDelta::of_as(&element, text_format).map_err(|e| e.to_string())?;
            if let Some((attlib, provider)) = templates.as_deref_mut() {
                delta.with_attribute_rows(attlib, provider);
            }
            changes.push(changed_element_json(&element, &delta));
        }
    } else {
        for change in diff.changes().iter().take(limit) {
            changes.push(candidate_json(*change));
        }
    }
    Ok(json!({
        "from": session_json(&diff.from),
        "to": session_json(&diff.to),
        "tally": {
            "deleted": tally.deleted,
            "inserted": tally.inserted,
            "modified": tally.modified,
            "unchanged_compared": tally.unchanged_compared,
            "shared_subtrees_pruned": tally.shared_subtrees_pruned,
        },
        "flag_only_drifts": diff.index.diagnostics.flag_only_drifts,
        "changes_total": diff.changes().len(),
        "changes_listed": changes.len(),
        "changes": changes,
    }))
}

fn position_json(position: e3d_io::index::RecordPosition) -> Value {
    json!({
        "ext": position.page.ext,
        "page": position.page.page,
        "offset_words": position.offset_words,
    })
}

fn candidate_json(change: e3d_io::index::IndexCandidate) -> Value {
    use e3d_io::index::IndexCandidate;
    match change {
        IndexCandidate::Inserted { refno, target } => json!({
            "refno": refno.to_string(),
            "kind": "inserted",
            "after": position_json(target),
        }),
        IndexCandidate::Deleted { refno, base } => json!({
            "refno": refno.to_string(),
            "kind": "deleted",
            "before": position_json(base),
        }),
        IndexCandidate::Modified {
            refno,
            base,
            target,
        } => json!({
            "refno": refno.to_string(),
            "kind": "modified",
            "before": position_json(base),
            "after": position_json(target),
        }),
    }
}

fn record_side_json(
    view: &e3d_io::record::ElementRecordView,
    parsed: Option<&ParsedElement>,
) -> Value {
    let mut out = json!({
        "page": view.page_no,
        "byte_offset": view.byte_offset,
        "bytes": view.raw_bytes.len(),
    });
    if let Some(parsed) = parsed {
        out["noun"] = json!(db1_dehash(parsed.noun_hash));
        out["noun_hash"] = json!(parsed.noun_hash);
        out["owner"] = json!(parsed.owner.to_string());
        out["members"] = json!(parsed.members.len());
        out["explicit_attributes"] = json!(parsed.explicit_attributes.len());
    }
    out
}

fn changed_element_json(element: &ChangedElement, delta: &ElementDelta) -> Value {
    let relocated_only = matches!(
        element.kind,
        ChangeKind::Modified {
            relocated_only: true
        }
    );
    let refnos = |list: &[RefNo]| list.iter().map(ToString::to_string).collect::<Vec<_>>();
    let mut delta_json = json!({
        "owner": delta.owner.map(|o| json!({"before": o.before.to_string(), "after": o.after.to_string()})),
        "members_added": refnos(&delta.members_added),
        "members_removed": refnos(&delta.members_removed),
        "members_reordered": delta.members_reordered,
        "explicit_changed": delta.explicit_changed.iter().map(|c| json!({
            "hash": c.hash,
            "name": c.name,
            "before": c.before.as_ref().map(explicit_value_json),
            "after": c.after.as_ref().map(explicit_value_json),
        })).collect::<Vec<_>>(),
        "implicit_words_changed": delta.implicit_words_changed.iter().map(|c| json!({
            "word": c.word,
            "before": c.before,
            "after": c.after,
        })).collect::<Vec<_>>(),
    });
    if let Some(rows) = &delta.attribute_rows {
        delta_json["attribute_rows"] = rows
            .iter()
            .map(|row| {
                json!({
                    "name": row.name,
                    "before": row.before.as_ref().map(value_json),
                    "after": row.after.as_ref().map(value_json),
                    "before_status": row.before_status.as_ref().map(status_json),
                    "after_status": row.after_status.as_ref().map(status_json),
                })
            })
            .collect();
    }
    if let Some(reason) = &delta.attribute_rows_unavailable {
        delta_json["attribute_rows_unavailable"] = json!(reason);
    }
    json!({
        "refno": element.refno.to_string(),
        "kind": element.kind.name(),
        "relocated_only": relocated_only,
        "noun": delta.noun_name,
        "before": element.before.as_ref().map(|v| record_side_json(v, delta.before.as_ref())),
        "after": element.after.as_ref().map(|v| record_side_json(v, delta.after.as_ref())),
        "delta": delta_json,
    })
}

fn explicit_value_json(value: &e3d_io::record::explicit::ExplicitValue) -> Value {
    use e3d_io::record::explicit::ExplicitValue;
    match value {
        ExplicitValue::Integer(v) => json!(v),
        ExplicitValue::Real(v) => json!(v),
        ExplicitValue::Bool(v) => json!(v),
        ExplicitValue::Text(v) => json!(v),
        ExplicitValue::RefNo(v) => json!(v.to_string()),
        ExplicitValue::IntArray(v) => json!(v),
        ExplicitValue::RealArray(v) => json!(v),
        ExplicitValue::RefNoArray(v) => {
            json!(v.iter().map(ToString::to_string).collect::<Vec<_>>())
        }
        ExplicitValue::TextList(v) => json!(v),
        ExplicitValue::RawWords(v) => json!({"words": v}),
    }
}

/// An `on`/`off` option, absent meaning off.
fn on_off(options: &HashMap<String, String>, name: &str) -> Result<bool, String> {
    match options.get(name).map(String::as_str) {
        None | Some("off") => Ok(false),
        Some("on") => Ok(true),
        Some(other) => Err(format!("--{name} takes on or off, got {other}")),
    }
}

fn parse_options(args: impl Iterator<Item = String>) -> Result<HashMap<String, String>, String> {
    let mut out = HashMap::new();
    let mut args = args.peekable();
    while let Some(flag) = args.next() {
        let key = flag
            .strip_prefix("--")
            .ok_or_else(|| format!("expected --option, got {flag}"))?;
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {flag}"))?;
        out.insert(key.to_owned(), value);
    }
    Ok(out)
}

fn required<'a>(options: &'a HashMap<String, String>, name: &str) -> Result<&'a str, String> {
    options
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| format!("missing --{name}"))
}

/// Fill release-bound paths from one schema pack.
///
/// `--schema PATH` opens that pack directly; `--schema-root PATH
/// --e3d-version ID` opens `PATH/ID`. A database-reading descriptor command
/// must name a pack or version: automatic selection is disabled until a
/// durable database release field is proven. `header` can still report
/// `COMM.VERS` and `UPGDNO` as diagnostics.
///
/// An explicit flag always wins, so a one-off file needs no pack edit. Only
/// the files the pack actually has are filled in: a pack that carries no
/// `dicvir.dat` leaves `--dicvir` alone rather than failing a command that was
/// never going to read UDAs.
///
/// Which `*vir.dat` `--desvir` gets follows `--template-type`, because that
/// flag already says which database type is being read — `--template-type
/// DICT` is how `list-uda` asks for `dicvir.dat`.
fn apply_schema_pack(command: &str, options: &mut HashMap<String, String>) -> Result<(), String> {
    let pack = match (options.get("schema"), options.get("schema-root")) {
        (Some(_), Some(_)) => return Err("use either --schema or --schema-root, not both".into()),
        (Some(path), None) => SchemaPack::open(path),
        (None, Some(root)) => match options.get("e3d-version") {
            Some(version) => SchemaPack::open_version(root, version),
            None if auto_schema_command(command) => {
                let db = required(options, "db")
                    .map_err(|_| "--schema-root needs --e3d-version or --db".to_string())?;
                SchemaPack::detect_for_database(root, db)
            }
            None => {
                return Err("--schema-root needs --e3d-version for this command".to_string());
            }
        },
        (None, None) => {
            if options.contains_key("e3d-version") {
                return Err("--e3d-version needs --schema-root".into());
            }
            if !auto_schema_command(command)
                || (options.contains_key("desvir") && options.contains_key("attlib"))
            {
                return Ok(());
            }
            let db = required(options, "db")?;
            SchemaPack::detect_for_database(e3d_io::bundled_schema_root(), db)
        }
    }
    .map_err(|e| format!("open schema pack: {e}"))?;

    let db_type = options
        .get("template-type")
        .cloned()
        .unwrap_or_else(|| "DESI".into());
    if !options.contains_key("desvir") {
        let path = pack
            .template(&db_type)
            .map_err(|e| format!("resolve --desvir from schema pack: {e}"))?;
        options.insert("desvir".into(), display_path(&path));
    }
    if !options.contains_key("attlib") {
        let path = pack
            .attlib()
            .map_err(|e| format!("resolve --attlib from schema pack: {e}"))?;
        options.insert("attlib".into(), display_path(&path));
    }
    if !options.contains_key("dicvir") {
        if let Ok(path) = pack.template("DICT") {
            options.insert("dicvir".into(), display_path(&path));
        }
    }
    for (flag, path) in [
        ("catalog-layout", pack.noun_layout()),
        ("catalog-fields", pack.noun_attr_fields()),
    ] {
        if options.contains_key(flag) {
            continue;
        }
        if let Some(path) = path.filter(|path| path.is_file()) {
            options.insert(flag.into(), display_path(&path));
        }
    }
    Ok(())
}

fn auto_schema_command(command: &str) -> bool {
    matches!(command, "extract" | "extract-complete" | "list-uda")
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Whether to read UDA values at all.
///
/// Off by default: a UDA's value is stored in a dictionary database, and which
/// dictionaries apply to an element is a property of the MDB the element was
/// opened under. Until that is wired up, reading them produces answers that
/// look like decoder output but are really a guess about project setup.
fn uda_enabled(options: &HashMap<String, String>) -> Result<bool, String> {
    match options.get("uda").map(String::as_str) {
        None | Some("off") => Ok(false),
        Some("on") => Ok(true),
        Some(other) => Err(format!("--uda takes on or off, got {other}")),
    }
}

fn dictionary_databases(options: &HashMap<String, String>) -> Result<Vec<&str>, String> {
    if options.contains_key("dictionary-db") && options.contains_key("dictionary-db-list") {
        return Err("use either --dictionary-db or --dictionary-db-list, not both".into());
    }
    if let Some(path) = options.get("dictionary-db") {
        return Ok(vec![path]);
    }
    Ok(options
        .get("dictionary-db-list")
        .map(|paths| {
            paths
                .split(';')
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .collect()
        })
        .unwrap_or_default())
}

fn reference_databases<'a>(
    options: &'a HashMap<String, String>,
    design_db: &'a str,
) -> Vec<&'a str> {
    std::iter::once(design_db)
        .chain(
            options
                .get("reference-db-list")
                .into_iter()
                .flat_map(|paths| paths.split(';'))
                .map(str::trim)
                .filter(|path| !path.is_empty()),
        )
        .collect()
}

/// Read an attribute the element does not store as its type's zero rather than
/// as the value the noun declares it defaults to.
///
/// `sub_5B03B90` answers a `GCEA` for an unstored attribute out of the noun's
/// own defaults in `*vir.dat`, and the extractor follows it. Listing an
/// element's attributes does not come that way. Of the 30 rows in the
/// 128-element TTY sample where such a default says anything the attribute's
/// type zero does not, E3D printed the zero for 29 and the declared default
/// for one. The other 31 in the sample declare the zero anyway, so nothing
/// turns on them either way.
///
/// The 29 are all `WORD` or `STRING` — `NAPP 2`, `QTYPE ANY`, `CLUTCH AUTO`,
/// `WELDTY MANU`, `FISSUE -1`, `FINPUT Unknown` — and the one that goes the
/// other way, `TABGRO.Fixedb`, is the only `BOOL` among them. So the rule is
/// held to the types it was learned from: a boolean keeps what the noun
/// declared. That leaves the rest of the sample where it was. Twenty-seven of
/// the sampled nouns' booleans declare `true`, and only two of them ever reach
/// this path: `TABGRO.Fixedb`, which E3D prints as `true`, and `CCTA.Activ`,
/// which E3D answers with a sentence about how many users are active.
///
/// The declared default is still what tells us the attribute's shape, so it is
/// emptied rather than discarded: `NAPP` reads back as the integer 0 and
/// `PDSHAP` as four zero words, which is what `q att` shows for both. Only the
/// explicit stream is treated this way. Chunk 1 holds defaults for attributes
/// living in the implicit region, reached by offset rather than by hash, and
/// no noun the sample covers has one.
fn read_stream_defaults_as_type_zero(
    extraction: &mut ElementExtraction,
    template: &NounTemplate,
) -> usize {
    let mut zeroed = 0;
    for extracted in &mut extraction.attributes {
        if extracted.status != AttributeExtractionStatus::TemplateDefault {
            continue;
        }
        if matches!(extracted.value, Some(DescriptorValue::Bool(_))) {
            continue;
        }
        let from_explicit_stream = template
            .attrs
            .iter()
            .find(|attr| attr.hash == extracted.hash)
            .is_some_and(|attr| attr.wide.location.word_offset() == 0)
            && template.defaults.explicit_for(extracted.hash).is_some();
        if !from_explicit_stream {
            continue;
        }
        extracted.value = extracted.value.as_ref().map(type_zero);
        // The same answer the catalog gives everywhere else it knows a type
        // but the element stored nothing: a default, not a reading.
        extracted.status = AttributeExtractionStatus::CatalogDefault;
        zeroed += 1;
    }
    zeroed
}

/// The zero of a value's own type — what E3D reports for an attribute of that
/// type with nothing behind it.
fn type_zero(value: &DescriptorValue) -> DescriptorValue {
    match value {
        DescriptorValue::Unset
        | DescriptorValue::Text(_)
        | DescriptorValue::TextList(_)
        | DescriptorValue::RefNoArray(_) => DescriptorValue::Unset,
        // An empty word slot is not the same as `unset`, and which of the two
        // E3D writes depends on the attribute rather than the value: `NAPP`
        // shows `0` and `QTYPE` shows `unset`, both from a word holding zero.
        DescriptorValue::Word { .. } => DescriptorValue::Word { raw: 0, text: None },
        DescriptorValue::Bool(_) => DescriptorValue::Bool(false),
        DescriptorValue::Int(_) => DescriptorValue::Int(0),
        DescriptorValue::Real(_) => DescriptorValue::Real(0.0),
        DescriptorValue::RefNo(_) => DescriptorValue::RefNo(RefNo::new(0, 0)),
        DescriptorValue::IntArray(values) => DescriptorValue::IntArray(vec![0; values.len()]),
        DescriptorValue::RealArray(values) => DescriptorValue::RealArray(vec![0.0; values.len()]),
        DescriptorValue::WordArray(values) => DescriptorValue::WordArray(vec![0; values.len()]),
        DescriptorValue::BoolArray(values) => DescriptorValue::BoolArray(vec![false; values.len()]),
        DescriptorValue::RawWords(values) => DescriptorValue::RawWords(vec![0; values.len()]),
    }
}

/// The name E3D shows for each reference the TTY capture named rather than
/// numbered.
///
/// Two passes, because a stored name outranks a built one: every database is
/// asked for a `NAME` record first, and only what nobody could name that way
/// falls through to the positional description E3D composes for nameless
/// elements — `SCYLINDER 1 of GMSET /GRZGG00-G3-8`. Running the fallback per
/// database would let a nameless copy in the first file beat a named copy in
/// the second.
fn resolve_reference_names(
    rows: &[CompleteAttributeRow],
    databases: &[&str],
    catalog: Option<&NounCatalog>,
) -> Result<HashMap<RefNo, String>, String> {
    let mut unresolved = rows
        .iter()
        .filter(|row| row.comparison == ValueComparison::NeedsReferenceResolution)
        .flat_map(|row| match row.binary_value.as_ref() {
            Some(DescriptorValue::RefNo(refno)) => vec![*refno],
            Some(DescriptorValue::RefNoArray(refnos)) => refnos.clone(),
            _ => Vec::new(),
        })
        .filter(|refno| refno.word0 != 0 || refno.word1 != 0)
        .collect::<HashSet<_>>();
    let mut names = HashMap::new();
    let name_hash = db1_hash("NAME");
    for database in databases {
        if unresolved.is_empty() {
            break;
        }
        let mut engine = ReadOnlyEngine::open(database)
            .map_err(|error| format!("open reference database {database}: {error}"))?;
        for refno in unresolved.clone() {
            let Some(view) = engine
                .find_element(refno)
                .map_err(|error| format!("resolve {refno} in {database}: {error}"))?
            else {
                continue;
            };
            let parsed = ParsedElement::parse_as(&view.raw_bytes, engine.text_format())
                .map_err(|error| format!("parse reference {refno} in {database}: {error}"))?;
            let Some(name) = parsed
                .explicit_attributes
                .iter()
                .rev()
                .find_map(|attribute| {
                    (attribute.hash == name_hash)
                        .then_some(&attribute.value)
                        .and_then(|value| match value {
                            e3d_io::record::explicit::ExplicitValue::Text(value) => {
                                Some(value.clone())
                            }
                            _ => None,
                        })
                })
            else {
                continue;
            };
            names.insert(refno, name);
            unresolved.remove(&refno);
        }
    }
    let Some(catalog) = catalog else {
        return Ok(names);
    };
    for database in databases {
        if unresolved.is_empty() {
            break;
        }
        let mut engine = ReadOnlyEngine::open(database)
            .map_err(|error| format!("open reference database {database}: {error}"))?;
        for refno in unresolved.clone() {
            let Some(display) = e3d_io::element_name::display_name(&mut engine, catalog, refno)
            else {
                continue;
            };
            names.insert(refno, display);
            unresolved.remove(&refno);
        }
    }
    Ok(names)
}

/// Answer the `Name` row for an element that stores no name.
///
/// Nothing in the record holds it — E3D builds it from the owner chain — so it
/// is filled in here, beside reference resolution, rather than pretended to be
/// a decoded value.
///
/// `Name` is a pseudo attribute, so the catalog fallback offers `unset` for
/// it. That answer is displaced here rather than left in place: E3D always
/// prints something for `Name`, and the owner chain is where it comes from.
fn apply_display_name(
    rows: &mut [CompleteAttributeRow],
    engine: &mut ReadOnlyEngine,
    catalog: &NounCatalog,
    refno: RefNo,
) {
    let Some(row) = rows.iter_mut().find(|row| {
        row.name.eq_ignore_ascii_case("name")
            && matches!(row.binary_value, None | Some(DescriptorValue::Unset))
    }) else {
        return;
    };
    let Some(display) = e3d_io::element_name::display_name(engine, catalog, refno) else {
        return;
    };
    row.comparison = if display.eq_ignore_ascii_case(row.rendered_value.trim()) {
        ValueComparison::Equivalent
    } else {
        ValueComparison::Mismatch
    };
    row.binary_value = Some(DescriptorValue::Text(display));
    row.source = CompleteAttributeSource::RuntimeDerived;
}

fn load_catalog(options: &HashMap<String, String>) -> Result<Option<NounCatalog>, String> {
    match (options.get("catalog-layout"), options.get("catalog-fields")) {
        (None, None) => Ok(None),
        (Some(layout), Some(fields)) => NounCatalog::read(layout, fields)
            .map(Some)
            .map_err(|error| format!("load noun catalog: {error}")),
        _ => Err("--catalog-layout and --catalog-fields must be supplied together".into()),
    }
}

fn parse_noun(input: &str) -> Result<u32, String> {
    if let Some(hex) = input
        .strip_prefix("0x")
        .or_else(|| input.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16).map_err(|e| format!("invalid noun hash {input}: {e}"))
    } else if input.bytes().all(|byte| byte.is_ascii_digit()) {
        input
            .parse::<u32>()
            .map_err(|e| format!("invalid noun hash {input}: {e}"))
    } else {
        Ok(db1_hash(input))
    }
}

fn parse_refno(input: &str) -> Result<RefNo, String> {
    let (word0, word1) = input
        .split_once('/')
        .ok_or_else(|| format!("invalid RefNo {input}; expected WORD0/WORD1"))?;
    let word0 = word0
        .parse::<u32>()
        .map_err(|e| format!("invalid RefNo word0: {e}"))?;
    let signed = word1
        .parse::<i32>()
        .map_err(|e| format!("invalid RefNo word1: {e}"))?;
    Ok(RefNo::new(word0, signed as u32))
}

fn describe_json(
    template: &NounTemplate,
    attlib: &AttlibData,
    metadata: &AttributeMetadataCatalog,
    catalog: Option<&NounCatalog>,
) -> Value {
    if let Some(catalog) = catalog {
        let reconciled = catalog.reconcile(template);
        let catalog_only_count = reconciled
            .attributes
            .iter()
            .filter(|row| row.source == ReconciledSource::CatalogOnly)
            .count();
        let descriptor_only_count = reconciled
            .attributes
            .iter()
            .filter(|row| row.source == ReconciledSource::DescriptorOnly)
            .count();
        return json!({
            "noun_hash": template.noun_hash,
            "noun_name": reconciled.noun.map(|noun| noun.noun.clone()).or_else(|| db1_dehash(template.noun_hash)),
            "authoritative": template.is_authoritative(),
            "physical_attribute_count": template.attrs.len(),
            "logical_attribute_count": reconciled.noun.map(|noun| noun.attrs.len()),
            "catalog_only_count": catalog_only_count,
            "descriptor_only_count": descriptor_only_count,
            "catalog": {
                "noun_count": catalog.noun_count(),
                "attribute_field_count": catalog.field_count(),
                "noun_present": reconciled.noun.is_some(),
                "short_name": reconciled.noun.map(|noun| noun.short_name.as_str()),
                "base": reconciled.noun.map(|noun| noun.base.as_str()),
                "hard": reconciled.noun.map(|noun| noun.hard.as_str()),
                "db_types": reconciled.noun.map(|noun| noun.db_types.as_slice()),
            },
            "attributes": reconciled.attributes.iter().map(|row| {
                let hash = row.catalog.map(|attr| attr.hash)
                    .or_else(|| row.descriptor.map(|attr| attr.hash))
                    .expect("reconciled row has catalog or descriptor");
                let descriptor = row.descriptor.map(|descriptor| {
                    let def = attlib.attr_def(descriptor.hash);
                    json!({
                        "ordinal": row.descriptor_ordinal,
                        "stride": descriptor.stride,
                        "storage_type_code": descriptor.storage_type_code,
                        "logical_count": descriptor.logical_count,
                        "semantic_type_code": def.map(|def| def.data_type),
                        "semantic_size": def.map(|def| def.size),
                        "compact": storage_json(descriptor.compact),
                        "wide": storage_json(descriptor.wide),
                        "tail": descriptor.tail,
                    })
                });
                let logical = row.catalog.map(|attr| json!({
                    "ordinal": row.catalog_ordinal,
                    "type_code": attr.type_code,
                    "type_name": attr.type_name,
                    "is_array": attr.is_array,
                    "max_size": attr.max_size,
                    "true_size": attr.true_size,
                    "is_uda": attr.is_uda,
                    "is_pseudo": attr.is_pseudo,
                    "hidden": attr.hidden,
                    "no_claim": attr.no_claim,
                    "default_probe": attr.default_probe,
                }));
                json!({
                    "hash": hash,
                    "name": row.catalog.map(|attr| attr.name.clone()).unwrap_or_else(|| attribute_name(hash)),
                    "report_name": metadata.report_name(hash),
                    "source": reconciled_source_name(row.source),
                    "logical": logical,
                    "physical": descriptor,
                    "attribute_fields": row.fields.map(|fields| &fields.fields),
                })
            }).collect::<Vec<_>>()
        });
    }

    json!({
        "noun_hash": template.noun_hash,
        "noun_name": db1_dehash(template.noun_hash),
        "authoritative": template.is_authoritative(),
        "attribute_count": template.attrs.len(),
        "attributes": template.attrs.iter().map(|descriptor| {
            let def = attlib.attr_def(descriptor.hash);
            json!({
                "hash": descriptor.hash,
                "name": attribute_name(descriptor.hash),
                "report_name": metadata.report_name(descriptor.hash),
                "stride": descriptor.stride,
                "storage_type_code": descriptor.storage_type_code,
                "logical_count": descriptor.logical_count,
                "semantic_type_code": def.map(|d| d.data_type),
                "semantic_size": def.map(|d| d.size),
                "compact": storage_json(descriptor.compact),
                "wide": storage_json(descriptor.wide),
                "tail": descriptor.tail,
            })
        }).collect::<Vec<_>>()
    })
}

fn reconciled_source_name(source: ReconciledSource) -> &'static str {
    match source {
        ReconciledSource::CatalogAndDescriptor => "catalog_and_descriptor",
        ReconciledSource::CatalogOnly => "catalog_only",
        ReconciledSource::DescriptorOnly => "descriptor_only",
    }
}

fn storage_json(storage: e3d_io::record::template::StorageDescriptor) -> Value {
    json!({
        "storage_words": storage.storage_words,
        "encoded_location": storage.location.raw,
        "word_offset": storage.location.word_offset(),
        "upper_bits": storage.location.upper_bits(),
        "bool_bit_index": storage.location.bool_bit_index(),
        "flags": storage.flags,
    })
}

fn extraction_json(extraction: &ElementExtraction, metadata: &AttributeMetadataCatalog) -> Value {
    json!({
        "refno": {
            "word0": extraction.refno.word0,
            "word1": extraction.refno.word1,
            "display": extraction.refno.to_string(),
        },
        "noun_hash": extraction.noun_hash,
        "noun_name": extraction.noun_name,
        "implicit_word_count": extraction.implicit_word_count,
        "format_word_10": extraction.format_word_10,
        "layout": layout_name(extraction.layout),
        "attributes": extraction.attributes.iter().map(|attribute| json!({
            "hash": attribute.hash,
            "name": attribute.name,
            "report_name": metadata.report_name(attribute.hash),
            "storage_type_code": attribute.storage_type_code,
            "semantic_type_code": attribute.semantic_type_code,
            "semantic_size": attribute.semantic_size,
            "layout": layout_name(attribute.layout),
            "storage": storage_json(attribute.storage),
            "status": status_json(&attribute.status),
            "value": attribute.value.as_ref().map(value_json),
        })).collect::<Vec<_>>()
    })
}

fn status_json(status: &AttributeExtractionStatus) -> Value {
    match status {
        AttributeExtractionStatus::Decoded => json!({"kind": "decoded"}),
        AttributeExtractionStatus::DecodedExplicit => json!({"kind": "decoded_explicit"}),
        AttributeExtractionStatus::TemplateDefault => json!({"kind": "template_default"}),
        AttributeExtractionStatus::CatalogDefault => json!({"kind": "catalog_default"}),
        AttributeExtractionStatus::RuntimeDerived { evaluator } => {
            json!({"kind": "runtime_derived", "evaluator": evaluator})
        }
        AttributeExtractionStatus::NonImplicit => json!({"kind": "non_implicit"}),
        AttributeExtractionStatus::DefaultRequired => json!({"kind": "default_required"}),
        #[allow(deprecated)]
        AttributeExtractionStatus::MetadataMissing => json!({"kind": "metadata_missing"}),
        AttributeExtractionStatus::Unsupported { detail } => {
            json!({"kind": "unsupported", "detail": detail})
        }
        AttributeExtractionStatus::OutOfBounds { detail } => {
            json!({"kind": "out_of_bounds", "detail": detail})
        }
    }
}

fn value_json(value: &DescriptorValue) -> Value {
    match value {
        DescriptorValue::Unset => json!({"kind": "unset"}),
        DescriptorValue::Bool(value) => json!(value),
        DescriptorValue::Int(value) => json!(value),
        DescriptorValue::Real(value) => json!(value),
        DescriptorValue::Text(value) => json!(value),
        DescriptorValue::RefNo(value) => json!({
            "word0": value.word0,
            "word1": value.word1,
            "display": value.to_string(),
        }),
        DescriptorValue::Word { raw, text } => json!({"raw": raw, "text": text}),
        DescriptorValue::IntArray(value) => json!(value),
        DescriptorValue::RealArray(value) => json!(value),
        DescriptorValue::BoolArray(value) => json!(value),
        DescriptorValue::RefNoArray(value) => Value::Array(
            value
                .iter()
                .map(|item| {
                    json!({
                        "word0": item.word0,
                        "word1": item.word1,
                        "display": item.to_string(),
                    })
                })
                .collect(),
        ),
        DescriptorValue::WordArray(value) => Value::Array(
            value
                .iter()
                .map(|raw| json!({"raw": raw, "text": e3d_io::record::descriptor::word_name(*raw)}))
                .collect(),
        ),
        DescriptorValue::TextList(value) => json!(value),
        DescriptorValue::RawWords(value) => match e3d_io::record::catalogue_expr::render(value) {
            Some(expression) => json!({"words": value, "expression": expression}),
            None => json!(value),
        },
    }
}

fn attribute_name(hash: u32) -> String {
    lookup_system_name(hash)
        .map(String::from)
        .or_else(|| db1_dehash(hash))
        .unwrap_or_else(|| format!("0x{hash:08X}"))
}

fn layout_name(layout: StorageLayout) -> &'static str {
    match layout {
        StorageLayout::Compact => "compact",
        StorageLayout::Wide => "wide",
    }
}

fn usage() -> String {
    "usage:\n  any command below takes [--schema PACK_DIR | --schema-root DIR --e3d-version ID]\n  in place of --desvir/--attlib/--dicvir/--catalog-layout/--catalog-fields; automatic\n  schema selection is disabled until a durable database release field is proven.\n  An explicit file flag still wins. See schema/README.md.\n\n  e3d-descriptor sessions --db PATH\n  e3d-descriptor spamap --db PATH [--session N] [--refno WORD0/WORD1]\n  e3d-descriptor spamap-census --input FILE_OR_DIR [--input ...] [--session FILE=N ...]\n  e3d-descriptor spamap-check --input FILE_OR_DIR [--input ...] [--session FILE=N ...] (--schema PACK_DIR | --desvir PATH --attlib PATH) [--rules confirmed|candidates] [--exclude FILE] [--refno WORD0/WORD1]\n  e3d-descriptor project-dab-tags --project-root PATH\n  e3d-descriptor diff --db PATH --from N [--to N] [--at-or-before on] [--steps on] [--content on] [--limit N] [--desvir PATH --attlib PATH [--template-type TYPE]]\n  e3d-descriptor describe --desvir PATH --attlib PATH --noun NOUN [--template-type TYPE] [--catalog-layout PATH --catalog-fields PATH]\n  e3d-descriptor extract --db PATH [--desvir PATH --attlib PATH] --refno WORD0/WORD1 [--uda on] [--dictionary-db PATH | --dictionary-db-list PATH1;PATH2] [--dicvir PATH] [--template-type TYPE]\n  e3d-descriptor extract-complete --db PATH [--desvir PATH --attlib PATH] --refno WORD0/WORD1 --tty-log PATH [--uda on] [--dictionary-db PATH | --dictionary-db-list PATH1;PATH2] [--reference-db-list PATH1;PATH2] [--dicvir PATH] [--template-type TYPE]\n  e3d-descriptor list-uda --db PATH [--desvir DICVIR --attlib PATH] --template-type DICT [--noun NOUN]\n  e3d-descriptor compile-metadata --template-dir PATH --attlib PATH --catalog-layout PATH --catalog-fields PATH --output PATH\n  e3d-descriptor emit-attr-info --template-dir PATH --attlib PATH --catalog-layout PATH --catalog-fields PATH --output PATH [--legacy-attr-info PATH] [--flat-winner DESI] [--compat-output PATH]\n  e3d-descriptor emit-uda-table --attlib PATH --dicvir PATH --output PATH (--dictionary-db PATH | --dictionary-db-list PATH1;PATH2)"
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_identifiers_parse_without_fixture_files() {
        assert_eq!(parse_noun("BRAN").unwrap(), db1_hash("BRAN"));
        assert_eq!(parse_noun("0x000C551C").unwrap(), 0x000C551C);
        let refno = parse_refno("24384/23257").unwrap();
        assert_eq!((refno.word0, refno.word1), (24384, 23257));
    }

    #[test]
    fn options_require_flag_value_pairs() {
        let options = parse_options(
            ["--noun", "BRAN", "--desvir", "schema.dat"]
                .into_iter()
                .map(String::from),
        )
        .unwrap();
        assert_eq!(required(&options, "noun").unwrap(), "BRAN");
        assert!(parse_options(["--noun"].into_iter().map(String::from)).is_err());
    }

    #[test]
    fn dictionary_database_list_preserves_mdb_order() {
        let options = parse_options(
            ["--dictionary-db-list", "first; second ;third"]
                .into_iter()
                .map(String::from),
        )
        .unwrap();
        assert_eq!(
            dictionary_databases(&options).unwrap(),
            vec!["first", "second", "third"]
        );
    }

    #[test]
    fn project_dab_tags_json_keeps_latest_and_history_explicit() {
        let database = PathBuf::from("project").join("ams8000_0001");
        let report = e3d_io::ProjectDabaconTagReport {
            root: PathBuf::from("project"),
            discovered_databases: 1,
            databases: vec![e3d_io::DatabaseDabaconTagReport {
                database: database.clone(),
                dbnum: 8000,
                db_type_hash: db1_hash("DESI"),
                db_type_name: Some("DESI".to_owned()),
                latest_session_number: 2,
                latest_dabacon_version_tag: Some("2_35_23o".to_owned()),
                retained_sessions: vec![
                    e3d_io::SessionDabaconTag {
                        session_number: 2,
                        session_page: 20,
                        dabacon_version_tag: Some("2_35_23o".to_owned()),
                    },
                    e3d_io::SessionDabaconTag {
                        session_number: 1,
                        session_page: 4,
                        dabacon_version_tag: Some("2_28_19_".to_owned()),
                    },
                ],
                historical_tag_session_counts: BTreeMap::from([
                    ("2_28_19_".to_owned(), 1),
                    ("2_35_23o".to_owned(), 1),
                ]),
                untagged_retained_sessions: 0,
            }],
            issues: Vec::new(),
            latest_tag_database_counts: BTreeMap::from([("2_35_23o".to_owned(), 1)]),
            historical_tag_session_counts: BTreeMap::from([
                ("2_28_19_".to_owned(), 1),
                ("2_35_23o".to_owned(), 1),
            ]),
            historical_tag_database_counts: BTreeMap::from([
                ("2_28_19_".to_owned(), 1),
                ("2_35_23o".to_owned(), 1),
            ]),
            latest_untagged_databases: 0,
            historical_untagged_databases: 0,
            historical_untagged_sessions: 0,
        };

        let output = project_dabacon_tags_json(report);
        assert_eq!(output["release_selection"], "diagnostic_only");
        assert_eq!(output["latest"]["tag_database_counts"]["2_35_23o"], 1);
        assert_eq!(output["history"]["tag_session_counts"]["2_28_19_"], 1);
        assert_eq!(output["databases"][0]["database"], json!(database));
        assert_eq!(output["databases"][0]["sessions"][1]["session_number"], 1);
        assert!(
            usage().contains("e3d-descriptor project-dab-tags --project-root PATH"),
            "usage must advertise the project aggregation command"
        );
    }
}
