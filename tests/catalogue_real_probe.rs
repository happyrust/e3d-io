//! Read-only probe of a real ams1112 catalogue element and the catalogue
//! geometry its spec reference reaches, to anchor the evaluator's semantics
//! before the PML walker is written. Reads the dabacon files directly — no live
//! E3D — so it is safe to run while a `des` process holds a session elsewhere.
//!
//! The catalogue chain crosses projects: a FITT in `AvevaMarineSample`
//! (`ams<n>_0001`) references a spec in `AvevaCatalogue` (`acp<n>_0001`). The
//! resolver here maps a database number to whichever of the two holds it, so the
//! probe can follow `SPRE` across the project boundary. Run with `--nocapture`
//! to read the layout; it asserts only that a catalogue element carrying design
//! parameters (`DESP`) was found, which is the fact the evaluator's `param(n)`
//! is built on.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use e3d_io::db_element::{DbElement, DbFilePin, DbFileResolver, DbSet};
use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::record::{catalogue_expr, catalogue_pml};
use e3d_io::{ReadOnlyEngine, ScanTier};

mod common;

const AMS1112: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams1112_0001";
const ATTLIB: &str = common::ATTLIB;
const DESVIR: &str = common::DESVIR;
const CATVIR: &str = common::CATVIR;

const AMS_DIR: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";
const ACP_DIR: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaCatalogue\acp000";

const CATALOG_NOUNS: [&str; 4] = ["FITT", "SBFI", "FIXING", "PFIT"];

fn fixtures_present() -> bool {
    [AMS1112, ATTLIB, DESVIR, CATVIR]
        .iter()
        .all(|path| Path::new(path).exists())
}

/// Map a database number to whichever project file holds it: the design sample
/// (`ams<n>_0001`) or the catalogue (`acp<n>_0001`). Everything but the design
/// database opens against the catalogue template — the chain from a FITT runs
/// into spec and catalogue elements, all of `CATA` shape.
struct MultiProjectResolver;
impl DbFileResolver for MultiProjectResolver {
    fn resolve(&self, dbno: u32) -> Option<DbFilePin> {
        let ams = PathBuf::from(format!(r"{AMS_DIR}\ams{dbno}_0001"));
        let acp = PathBuf::from(format!(r"{ACP_DIR}\acp{dbno}_0001"));
        let file = if ams.exists() {
            ams
        } else if acp.exists() {
            acp
        } else {
            return None;
        };
        Some(DbFilePin {
            file,
            template: PathBuf::from(CATVIR),
            db_type: None,
            sesno: None,
        })
    }
}

fn render_words(words: &[u32]) -> Option<String> {
    if let Some(text) = catalogue_expr::render(words) {
        return Some(format!("expr-tuple -> {text}"));
    }
    let signed: Vec<i32> = words.iter().map(|word| *word as i32).collect();
    catalogue_pml::render(&signed).map(|text| format!("pml -> {text}"))
}

fn describe(value: &DescriptorValue) -> String {
    match value {
        DescriptorValue::RawWords(words) => match render_words(words) {
            Some(text) => format!("RawWords{words:?} {text}"),
            None => format!("RawWords{words:?}"),
        },
        other => format!("{other:?}"),
    }
}

fn dump_attrs(prefix: &str, element: &DbElement) {
    let Ok(names) = element.attribute_names() else {
        return;
    };
    for name in names {
        if let Ok(Some(value)) = element.attribute(&name) {
            eprintln!("{prefix}{name} = {}", describe(&value));
        }
    }
}

/// Members `depth` levels deep, capped per level so a wide geometry set does not
/// bury the shapes worth reading. Errors are printed, never panicked on: a
/// dangling cross-project reference is a fact to see, not a test failure.
fn dump_members(prefix: &str, element: &DbElement, depth: usize) {
    if depth == 0 {
        return;
    }
    let members = match element.members() {
        Ok(members) => members,
        Err(error) => {
            eprintln!("{prefix}(members unavailable: {error})");
            return;
        }
    };
    for (seen, member) in members.enumerate() {
        if seen >= 8 {
            eprintln!("{prefix}… more members omitted");
            break;
        }
        let noun = member
            .element_type()
            .unwrap_or_else(|error| format!("<{error}>"));
        eprintln!(
            "{prefix}- {} {} name={:?}",
            member.refno(),
            noun,
            member.stored_name().ok().flatten()
        );
        dump_attrs(&format!("{prefix}    "), &member);
        dump_members(&format!("{prefix}  "), &member, depth - 1);
    }
}

#[test]
fn probe_a_real_catalogue_element() {
    if !fixtures_present() {
        eprintln!("catalogue probe: fixtures absent, skipping");
        return;
    }

    let attlib = e3d_attlib::AttlibData::parse_file(ATTLIB).expect("attlib parses");
    let set = Arc::new(DbSet::with_resolver(
        Arc::new(attlib),
        Box::new(MultiProjectResolver),
    ));
    set.add_db(DbFilePin {
        file: PathBuf::from(AMS1112),
        template: PathBuf::from(DESVIR),
        db_type: Some("DESI".to_string()),
        sesno: None,
    })
    .expect("ams1112 opens");

    // The first catalogue element the header scan reaches.
    let mut engine = ReadOnlyEngine::open(AMS1112).expect("open ams1112");
    let mut found: Option<(DbElement, String)> = None;
    for item in engine.scan_elements(ScanTier::Header).expect("scan") {
        let refno = item.expect("scan element").refno;
        let element = set.element(refno);
        if let Ok(noun) = element.element_type() {
            if CATALOG_NOUNS.contains(&noun.as_str()) {
                found = Some((element, noun));
                break;
            }
        }
    }
    let (element, noun) = found.expect("ams1112 holds a FITT/SBFI/FIXING/PFIT");
    eprintln!(
        "=== catalogue element {} {} name={:?}",
        element.refno(),
        noun,
        element.stored_name().ok().flatten()
    );
    dump_attrs("  ", &element);

    // The design parameters `PARAM n` binds to: a real array, 1-based (GATPAR).
    let design_params = element.get_double_array("DESP").expect("read DESP");
    eprintln!("  >>> DESP (design parameters, PARAM n) = {design_params:?}");

    // Follow the spec reference across the project boundary and every other
    // reference that leaves this database, dumping the geometry it reaches.
    for name in element.attribute_names().unwrap_or_default() {
        let Ok(Some(DescriptorValue::RefNo(target))) = element.attribute(&name) else {
            continue;
        };
        if !target.is_valid() || target.dbno() == element.db_no() {
            continue;
        }
        eprintln!("  ref {name} -> {} (db {})", target, target.dbno());
        let catalog = set.element(target);
        match catalog.element_type() {
            Ok(target_noun) => {
                eprintln!(
                    "      {} {} name={:?}",
                    catalog.refno(),
                    target_noun,
                    catalog.stored_name().ok().flatten()
                );
                dump_attrs("        ", &catalog);
                dump_members("        ", &catalog, 4);
            }
            Err(error) => eprintln!("      (unresolved: {error})"),
        }
    }

    assert!(
        design_params.is_some(),
        "a catalogue element carries its design parameters in DESP"
    );
}
