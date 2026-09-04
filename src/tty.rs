//! Parser and reconciler for an E3D TTY `ATTLIST` authority capture.

use std::collections::HashMap;

use crate::attlib_metadata::AttributeMetadataCatalog;
use crate::noun_catalog::NounCatalog;
use crate::record::descriptor::{
    AttributeExtractionStatus, DescriptorValue, ElementExtraction, ExtractedAttribute,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TtyAttributeRow {
    pub ordinal: usize,
    pub name: String,
    pub rendered_value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompleteAttributeSource {
    BinaryImplicit,
    BinaryExplicit,
    CatalogDefault,
    RuntimeDerived,
    BinaryUnresolved,
    Structural,
    TtyAuthority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueComparison {
    Equivalent,
    NeedsReferenceResolution,
    Mismatch,
    MissingBinaryValue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompleteAttributeRow {
    pub ordinal: usize,
    pub name: String,
    pub rendered_value: String,
    pub source: CompleteAttributeSource,
    pub binary_name: Option<String>,
    pub binary_status: Option<AttributeExtractionStatus>,
    pub binary_value: Option<DescriptorValue>,
    pub resolved_reference_name: Option<String>,
    pub comparison: ValueComparison,
}

pub fn parse_attlist_capture(input: &str) -> Result<Vec<TtyAttributeRow>, String> {
    let lines: Vec<&str> = input.lines().map(str::trim_end).collect();
    let begin = lines
        .iter()
        .position(|line| line.trim() == "NOUN-ATTLIST-BEGIN")
        .ok_or_else(|| "NOUN-ATTLIST-BEGIN marker is missing".to_string())?;
    let end = lines
        .iter()
        .position(|line| line.trim() == "NOUN-ATTLIST-END")
        .ok_or_else(|| "NOUN-ATTLIST-END marker is missing".to_string())?;
    if end <= begin {
        return Err("NOUN-ATTLIST-END precedes begin marker".into());
    }

    let mut rows = Vec::new();
    let mut cursor = begin + 1;
    while cursor < end {
        let line = lines[cursor].trim();
        let Some(name) = line.strip_prefix("ZZA ") else {
            cursor += 1;
            continue;
        };
        let name = name.trim().to_string();
        let value_line = lines
            .get(cursor + 1)
            .ok_or_else(|| format!("TTY value line missing after {name}"))?
            .trim();
        if value_line.starts_with("ZZA ") || value_line == "NOUN-ATTLIST-END" {
            return Err(format!("TTY value line missing after {name}"));
        }
        let mut rendered_value = strip_exact_heading(value_line, &name)
            .unwrap_or(value_line)
            .to_string();
        // A value too long for one line simply continues on the next, with
        // nothing to mark it: `Blength (600mm … 700mm` is finished by
        // `710mm 720mm )`. Skipping those lines silently truncated the value
        // and made the attribute look like a decoding disagreement.
        let mut cursor_after = cursor + 2;
        while cursor_after < end {
            let continuation = lines[cursor_after].trim();
            if continuation.is_empty() || continuation.starts_with("ZZA ") {
                break;
            }
            rendered_value.push(' ');
            rendered_value.push_str(continuation);
            cursor_after += 1;
        }
        rows.push(TtyAttributeRow {
            ordinal: rows.len(),
            name,
            rendered_value,
        });
        cursor = cursor_after;
    }
    if rows.is_empty() {
        return Err("TTY ATTLIST capture contains no ZZA rows".into());
    }
    Ok(rows)
}

fn strip_exact_heading<'a>(line: &'a str, heading: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(heading)?;
    match rest.chars().next() {
        None => Some(rest),
        Some(ch) if ch.is_whitespace() => Some(rest.trim_start()),
        Some(_) => None,
    }
}

/// TTY headings that neither `attlib` nor the catalog's query text answers.
///
/// `q att` heads the position vector `Position` and the position flag
/// `Posflag`. The only names attlib offers for those two attributes are
/// `PositonEditor` and `Positional` — editor labels it hands to 349 and 102
/// attributes respectively, never as a report heading — and it holds no name
/// `Position` or `Posflag` anywhere. So the heading is matched to the system
/// name instead, and the values line up: `POS` decodes to
/// `[7036.2, -21611.8, 3200.0]` where E3D prints
/// `E 7036mm S 21612mm U 3200mm`.
///
/// `Detref` is the same shape of problem one step further out: attlib and the
/// catalog both call `DETA` `Detail`, and no attribute anywhere in the 4271
/// exported field records is headed `Detref`. Two things say the heading is
/// `DETA`'s all the same. The rows of `q att` follow the catalog's attribute
/// order exactly for `GPART` and `PRTELE`, and `Detref` lands on `DETA`'s slot
/// in both. And `SPCO.DETA` is a two-word reference in the template, which is
/// what `/AAPA100-SR` needs to be read from — where the catalog calls the same
/// attribute `BOOL`.
const TTY_HEADING_ALIASES: &[(&str, &[&str])] = &[
    ("position", &["pos"]),
    ("posflag", &["posf", "posi"]),
    ("detref", &["deta"]),
];

pub fn reconcile_attlist(
    tty_rows: &[TtyAttributeRow],
    extraction: &ElementExtraction,
    metadata: &AttributeMetadataCatalog,
    catalog: Option<&NounCatalog>,
) -> Vec<CompleteAttributeRow> {
    let mut system_by_report_name: HashMap<String, &ExtractedAttribute> = HashMap::new();
    let mut uda_by_report_name: HashMap<String, &ExtractedAttribute> = HashMap::new();
    for attribute in &extraction.attributes {
        let names = if attribute.name.starts_with(':') {
            &mut uda_by_report_name
        } else {
            &mut system_by_report_name
        };
        names.insert(attribute.name.to_ascii_lowercase(), attribute);
        if attribute.name.starts_with(':') {
            names.insert(
                attribute.name.trim_start_matches(':').to_ascii_lowercase(),
                attribute,
            );
        }
        if let Some(report_name) = metadata.report_name(attribute.hash) {
            names.insert(report_name.to_ascii_lowercase(), attribute);
        }
        // Last, so the exported query text outranks the name recovered by
        // walking the attlib record: where the two disagree it is the recovery
        // that has picked up an editor label.
        if let Some(heading) = catalog.and_then(|catalog| catalog.query_heading(attribute.hash)) {
            names.insert(heading.to_ascii_lowercase(), attribute);
        }
    }

    tty_rows
        .iter()
        .map(|tty| {
            let key = tty.name.to_ascii_lowercase();
            if matches!(key.as_str(), "type" | "owner" | "lock") {
                let (binary_name, binary_value) = match key.as_str() {
                    "type" => (
                        Some("noun".to_string()),
                        extraction.noun_name.clone().map(DescriptorValue::Text),
                    ),
                    "owner" => (
                        Some("owner".to_string()),
                        Some(DescriptorValue::RefNo(extraction.owner)),
                    ),
                    _ => (None, None),
                };
                return CompleteAttributeRow {
                    ordinal: tty.ordinal,
                    name: tty.name.clone(),
                    rendered_value: tty.rendered_value.clone(),
                    source: CompleteAttributeSource::Structural,
                    binary_name,
                    binary_status: None,
                    resolved_reference_name: None,
                    comparison: match key.as_str() {
                        "type" => ValueComparison::Equivalent,
                        // A `WORLD` is owned by nothing, and E3D prints that as
                        // `Nulref`. Asking for a name for the null reference
                        // left three rows waiting on a lookup that could never
                        // answer, so the owner is compared like any other
                        // reference and only a real one goes on to resolution.
                        "owner" => binary_value
                            .as_ref()
                            .map(|value| compare_tty_value(value, &tty.rendered_value))
                            .unwrap_or(ValueComparison::MissingBinaryValue),
                        "lock" if tty.rendered_value.eq_ignore_ascii_case("false") => {
                            ValueComparison::Equivalent
                        }
                        _ => ValueComparison::Mismatch,
                    },
                    binary_value,
                };
            }
            let matched = if key.starts_with(':') {
                uda_by_report_name
                    .get(&key)
                    .or_else(|| uda_by_report_name.get(key.trim_start_matches(':')))
            } else {
                system_by_report_name
                    .get(&key)
                    .or_else(|| {
                        TTY_HEADING_ALIASES
                            .iter()
                            .filter(|(heading, _)| *heading == key)
                            .flat_map(|(_, system_names)| system_names.iter())
                            .find_map(|system_name| system_by_report_name.get(*system_name))
                    })
                    .or_else(|| uda_by_report_name.get(&key))
            };
            let Some(binary) = matched else {
                return CompleteAttributeRow {
                    ordinal: tty.ordinal,
                    name: tty.name.clone(),
                    rendered_value: tty.rendered_value.clone(),
                    source: CompleteAttributeSource::TtyAuthority,
                    binary_name: None,
                    binary_status: None,
                    binary_value: None,
                    resolved_reference_name: None,
                    comparison: ValueComparison::MissingBinaryValue,
                };
            };
            let source = match binary.status {
                AttributeExtractionStatus::Decoded => CompleteAttributeSource::BinaryImplicit,
                AttributeExtractionStatus::DecodedExplicit => {
                    CompleteAttributeSource::BinaryExplicit
                }
                AttributeExtractionStatus::CatalogDefault => {
                    CompleteAttributeSource::CatalogDefault
                }
                AttributeExtractionStatus::RuntimeDerived { .. } => {
                    CompleteAttributeSource::RuntimeDerived
                }
                _ => CompleteAttributeSource::BinaryUnresolved,
            };
            let unit = stored_unit(catalog, binary.hash);
            let comparison = binary
                .value
                .as_ref()
                .map(|value| {
                    compare_extracted_tty_value(&binary.name, value, &tty.rendered_value, unit)
                })
                .unwrap_or(ValueComparison::MissingBinaryValue);
            CompleteAttributeRow {
                ordinal: tty.ordinal,
                name: tty.name.clone(),
                rendered_value: tty.rendered_value.clone(),
                source,
                binary_name: Some(binary.name.clone()),
                binary_status: Some(binary.status.clone()),
                binary_value: binary.value.clone(),
                resolved_reference_name: None,
                comparison,
            }
        })
        .collect()
}

fn compare_extracted_tty_value(
    name: &str,
    value: &DescriptorValue,
    rendered: &str,
    unit: StoredUnit,
) -> ValueComparison {
    if name.eq_ignore_ascii_case("IMPSTC")
        && matches!(value, DescriptorValue::Int(0))
        && rendered.eq_ignore_ascii_case("undefined")
    {
        return ValueComparison::Equivalent;
    }
    compare_tty_value_as(value, rendered, unit)
}

pub fn apply_reference_names(
    rows: &mut [CompleteAttributeRow],
    names: &HashMap<crate::RefNo, String>,
) {
    for row in rows {
        if row.comparison != ValueComparison::NeedsReferenceResolution {
            continue;
        }
        let Some(named) = row
            .binary_value
            .as_ref()
            .and_then(|value| name_reference_value(value, names))
        else {
            continue;
        };
        row.comparison = if named.eq_ignore_ascii_case(row.rendered_value.trim()) {
            ValueComparison::Equivalent
        } else {
            ValueComparison::Mismatch
        };
        row.resolved_reference_name = Some(named);
    }
}

/// What E3D shows for a reference value once every target it names is known.
///
/// A reference array is named entry by entry. `MBOL.Sbrarray` holds ten slots
/// of which two are filled, and E3D prints ` 1 /MSAAD 2 /MSAAG` — the empty
/// slots do not appear, and each one that does carries its position.
/// `SCOM.Blrfarray` holds one and E3D prints `/MSIDBHP-4-24` bare, so the
/// position is written only where there is more than one entry to tell apart.
///
/// The position is the slot's rather than a running count of the filled ones.
/// The sample cannot separate the two readings, its empty slots all being at
/// the tail; the slot is what an E3D array subscript means.
///
/// Nothing is offered unless every filled slot resolved. A partly named list
/// would read as a disagreement about the value where it is really a database
/// the lookup was not given.
fn name_reference_value(
    value: &DescriptorValue,
    names: &HashMap<crate::RefNo, String>,
) -> Option<String> {
    match value {
        DescriptorValue::RefNo(refno) => names.get(refno).cloned(),
        DescriptorValue::RefNoArray(refnos) => {
            let filled: Vec<(usize, &String)> = refnos
                .iter()
                .enumerate()
                .filter(|(_, refno)| refno.word0 != 0 || refno.word1 != 0)
                .map(|(index, refno)| names.get(refno).map(|name| (index + 1, name)))
                .collect::<Option<Vec<_>>>()?;
            match filled.as_slice() {
                [] => None,
                [(_, only)] => Some((*only).clone()),
                many => Some(
                    many.iter()
                        .map(|(position, name)| format!("{position} {name}"))
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
            }
        }
        _ => None,
    }
}

/// What the catalog says the number behind a word-shaped value means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StoredUnit {
    Unknown,
    /// Hundredths of a degree. The record holds `4500` where `q att` prints
    /// `45`.
    Angle,
}

/// `db1_hash("ANGL")`, the `UNIT` every angle attribute in the exported
/// catalog carries — 125 of the 4271, `SSLC.Pxbshear` and `SCTN.Bangle`
/// among them.
const UNIT_ANGLE: u64 = 773_119;

fn stored_unit(catalog: Option<&NounCatalog>, hash: u32) -> StoredUnit {
    let unit = catalog
        .and_then(|catalog| catalog.attribute_fields(hash))
        .and_then(|record| record.fields.get("UNIT"))
        .and_then(|unit| unit.get("i"))
        .and_then(serde_json::Value::as_u64);
    match unit == Some(UNIT_ANGLE) {
        true => StoredUnit::Angle,
        false => StoredUnit::Unknown,
    }
}

/// Catalogue geometry stores expressions, not numbers: a five-word tuple E3D
/// prints as `0.7 TIMES PARAM 2`, a pair it prints as `-Y`, `P7` or `-X45Y`, a
/// chain of turns it prints as `X ( 45 ) Z`, a terminated run of point numbers
/// it prints as `P1 T0 P2`, or a whole formula.
fn rendered_by_shape(values: &[u32]) -> Option<String> {
    crate::record::catalogue_expr::render(values)
        .or_else(|| crate::record::axis_spec::render(values))
        .or_else(|| crate::record::direction_spec::render(values))
        .or_else(|| crate::record::point_list::render(values))
        .or_else(|| {
            let signed: Vec<i32> = values.iter().map(|value| *value as i32).collect();
            crate::record::catalogue_pml::render(&signed)
        })
        .or_else(|| crate::record::text::packed(values))
}

fn compare_tty_value(value: &DescriptorValue, rendered: &str) -> ValueComparison {
    compare_tty_value_as(value, rendered, StoredUnit::Unknown)
}

fn compare_tty_value_as(
    value: &DescriptorValue,
    rendered: &str,
    unit: StoredUnit,
) -> ValueComparison {
    let rendered = rendered.trim();
    let equivalent = match value {
        // An attribute with nothing stored reads as the null reference when
        // its type is a reference, which is how E3D prints it.
        DescriptorValue::Unset => {
            rendered.eq_ignore_ascii_case("unset") || rendered.eq_ignore_ascii_case("nulref")
        }
        // An array of nothing but zeros is the absence of a value rather than
        // a list of them, and E3D writes that absence more than one way:
        // `Skey` stores [0, 0] and prints `unset`, while `PDShape` falls back
        // to a zeroed catalogue tuple and prints `0`. Where E3D printed a PML
        // formula instead, the expression is held elsewhere in the catalogue
        // and the disagreement stands.
        value
            if is_all_zero(value)
                && matches!(rendered.to_ascii_lowercase().as_str(), "unset" | "0" | "") =>
        {
            true
        }
        DescriptorValue::Bool(value) => rendered
            .parse::<bool>()
            .is_ok_and(|rendered| rendered == *value),
        DescriptorValue::Int(value) => number_agrees(*value as f64, rendered),
        DescriptorValue::Real(value) => number_agrees(*value, rendered),
        // A fixed-width text field is padded with blanks, and one holding
        // nothing but blanks is the absence of a value rather than a value
        // made of spaces: `SDTE.Skey` holds four of them where E3D prints
        // nothing at all, and `CATE.Skey` holds none where it prints `unset`.
        DescriptorValue::Text(value) if value.trim().is_empty() => {
            rendered.is_empty() || rendered.eq_ignore_ascii_case("unset")
        }
        DescriptorValue::Text(value) => value.trim().eq_ignore_ascii_case(rendered),
        DescriptorValue::RefNo(value) if value.word0 == 0 && value.word1 == 0 => {
            rendered.eq_ignore_ascii_case("nulref")
        }
        // E3D could not name this target either, and says so by printing the
        // RefNo it failed on: `Unknown Ref (=15192/28820)`. Reading the same
        // RefNo out of the record is the whole of the agreement available, so
        // sending the row on to resolution would only stall it forever.
        DescriptorValue::RefNo(value) if unknown_ref(rendered) == Some(value.to_string()) => true,
        DescriptorValue::RefNo(value) => {
            if rendered == value.to_string() {
                true
            } else {
                return ValueComparison::NeedsReferenceResolution;
            }
        }
        DescriptorValue::Word { raw, text } => {
            text.as_deref()
                .is_some_and(|text| text.eq_ignore_ascii_case(rendered))
                // An empty word slot is zero under every reading of the word,
                // and E3D writes that zero with whatever unit the attribute
                // carries: `SCTN.Bangle` prints `0degree`. Which reading it is
                // stays undetermined — attlib holds no definition for `BANG`
                // and the descriptor gives it one word either way — but every
                // reading of an empty slot agrees on the value.
                || (*raw == 0
                    && !rendered.contains(char::is_whitespace)
                    && (rendered.is_empty()
                        || rendered.eq_ignore_ascii_case("unset")
                        || number_agrees(0.0, rendered)))
                || rendered
                    .parse::<u32>()
                    .is_ok_and(|rendered| rendered == *raw)
                // A word no dictionary resolves, whose bits read as a negative
                // integer, is a sentinel and not a hash: `Rlock` stores
                // 0xFFFFFFFF and E3D prints -1.
                || (text.is_none()
                    && (*raw as i32) < 0
                    && rendered
                        .parse::<i32>()
                        .is_ok_and(|rendered| rendered == *raw as i32))
                // An angle attribute holding one word holds hundredths of a
                // degree. `SSLC.Pxbshear` stores 1150 where E3D prints 11.5,
                // and the six values that attribute takes across
                // `ams7000_0001` read back on 2026-08-27 as 11.5, 15, 45,
                // -11.5, -15 and -45.
                || (unit == StoredUnit::Angle
                    && text.is_none()
                    && number_agrees(f64::from(*raw as i32) / 100.0, rendered))
        }
        // An orientation is three angles, and E3D states it as the two axis
        // directions they produce. Comparing the geometry sidesteps having to
        // reproduce E3D's choice of wording for a given direction.
        DescriptorValue::RealArray(values)
            if values.len() == 3 && parse_orientation(rendered).is_some() =>
        {
            let (y, z) = parse_orientation(rendered).expect("guarded above");
            let (actual_y, actual_z) =
                crate::record::orientation::axes([values[0], values[1], values[2]]);
            // TTY prints angles to three decimals, so a thousandth of a degree
            // is the finest agreement the text can express.
            let tolerance = 1e-4;
            actual_y
                .iter()
                .zip(y)
                .chain(actual_z.iter().zip(z))
                .all(|(actual, expected)| nearly_equal(*actual, expected, tolerance))
        }
        // Three numbers are usually a point or a direction, but not always:
        // `SCOM.Param` holds `[50, 61.2, TUB]` and E3D prints it as the list it
        // is. So the vector reading applies only where the text is one, and
        // anything else falls through to the list below.
        DescriptorValue::RealArray(values)
            if values.len() == 3
                && (parse_position(rendered).is_some() || parse_direction(rendered).is_some()) =>
        {
            let expected = parse_position(rendered)
                .or_else(|| parse_direction(rendered))
                .expect("guarded above");
            {
                // As with a scalar, the text cannot say more than it shows:
                // `E 7036mm S 21612mm U 3200mm` pins each component only to
                // the nearest millimetre, so a stored 7036.2 agrees with it.
                let printed = printed_tolerance(rendered);
                values.iter().zip(expected).all(|(actual, expected)| {
                    nearly_equal(*actual, expected, 2e-5)
                        || (actual - expected).abs() <= printed * (1.0 + 1e-9)
                })
            }
        }
        DescriptorValue::IntArray(values) => {
            let words: Vec<u32> = values.iter().map(|value| *value as u32).collect();
            rendered_by_shape(&words).is_some_and(|text| text.trim().eq_ignore_ascii_case(rendered))
                || list_matches_rendering(
                    &values.iter().map(|value| *value as f64).collect::<Vec<_>>(),
                    rendered,
                )
        }
        DescriptorValue::RealArray(values) => list_matches_rendering(values, rendered),
        DescriptorValue::BoolArray(values) => {
            let tokens: Vec<&str> = rendered.split_whitespace().collect();
            tokens.len() == values.len()
                && values
                    .iter()
                    .zip(&tokens)
                    .all(|(value, token)| token.parse::<bool>().is_ok_and(|token| token == *value))
        }
        DescriptorValue::RefNoArray(_) => return ValueComparison::NeedsReferenceResolution,
        DescriptorValue::WordArray(values) | DescriptorValue::RawWords(values) => {
            // `SDTE.Skey` holds four spaces behind a count of four, which E3D
            // shows as nothing at all.
            if rendered_by_shape(values)
                .is_some_and(|text| text.trim().eq_ignore_ascii_case(rendered))
            {
                return ValueComparison::Equivalent;
            }
            // Arrays whose storage is not described carry their own length in
            // the first word: `Level` reaches here as [2, 0, 10] where E3D
            // prints `0 10`. Both readings are offered because nothing in the
            // header says which one this attribute is.
            let readings = [
                values.as_slice(),
                match values.split_first() {
                    Some((count, rest)) if *count as usize == rest.len() => rest,
                    _ => &[],
                },
            ];
            readings
                .iter()
                .filter(|reading| !reading.is_empty())
                .any(|reading| words_match_rendering(reading, rendered))
        }
    };
    if equivalent {
        ValueComparison::Equivalent
    } else {
        ValueComparison::Mismatch
    }
}

/// The RefNo inside `Unknown Ref (=15192/28820)`, if that is what was printed.
fn unknown_ref(rendered: &str) -> Option<String> {
    rendered
        .strip_prefix("Unknown Ref (=")?
        .strip_suffix(')')
        .map(str::to_string)
}

fn nearly_equal(left: f64, right: f64, tolerance: f64) -> bool {
    (left - right).abs() <= tolerance * left.abs().max(right.abs()).max(1.0)
}

fn is_all_zero(value: &DescriptorValue) -> bool {
    match value {
        DescriptorValue::IntArray(values) => values.iter().all(|value| *value == 0),
        DescriptorValue::RealArray(values) => values.iter().all(|value| *value == 0.0),
        DescriptorValue::WordArray(values) | DescriptorValue::RawWords(values) => {
            values.iter().all(|value| *value == 0)
        }
        DescriptorValue::BoolArray(values) => values.iter().all(|value| !*value),
        _ => false,
    }
}

/// Does a stored number agree with the text E3D printed for it?
///
/// E3D prints a fixed number of decimals, so the text cannot say more than it
/// shows: `ELBO.Angle` holds 31.0915 and prints `31.091degree`, and a `Param`
/// entry held as a 32-bit float widens to 50.7999992370605 and prints `50.8`.
/// Agreement to the printed precision is therefore all that can be asked —
/// but only where the text printed a fractional part at all, so that a bare
/// `0mm` still has to mean zero.
fn number_agrees(actual: f64, rendered: &str) -> bool {
    let Some(printed) = parse_leading_number(rendered) else {
        return false;
    };
    if nearly_equal(printed, actual, 1e-9) {
        return true;
    }
    // A number printed without any fractional part was still rounded if the
    // value behind it has one: `SBOL.Xtralength` holds 12.5 and prints `13mm`.
    // A stored whole number gets no such licence — 12 printing as `13` is a
    // disagreement, not a rounding, so `Level 12` cannot pass as `13`.
    let decimals = match printed_decimals(rendered) {
        Some(decimals) => decimals,
        None if actual.fract() != 0.0 => 0,
        None => return false,
    };
    // Half of the last printed digit, plus room for the binary representation
    // of that halfway point.
    let tolerance = 0.5 * 10f64.powi(-(decimals as i32));
    (actual - printed).abs() <= tolerance * (1.0 + 1e-9)
}

/// How many decimals the leading number in `rendered` was printed to, or
/// `None` if it was printed without a fractional part.
fn printed_decimals(rendered: &str) -> Option<u32> {
    let fraction = rendered
        .split_once('.')?
        .1
        .chars()
        .take_while(char::is_ascii_digit)
        .count();
    (fraction > 0).then_some(fraction as u32)
}

/// Half of the last digit the coarsest number in `rendered` was printed to.
///
/// A vector is printed component by component and they need not share a
/// precision, so the loosest one sets what the whole rendering can be held to.
fn printed_tolerance(rendered: &str) -> f64 {
    rendered
        .split_whitespace()
        .filter(|token| parse_leading_number(token).is_some())
        .map(|token| 0.5 * 10f64.powi(-(printed_decimals(token).unwrap_or(0) as i32)))
        .fold(0.0, f64::max)
}

fn parse_leading_number(value: &str) -> Option<f64> {
    let end = value
        .char_indices()
        .skip(1)
        .find(|(_, ch)| !matches!(ch, '0'..='9' | '.' | 'e' | 'E' | '+' | '-'))
        .map(|(index, _)| index)
        .unwrap_or(value.len());
    value.get(..end)?.parse().ok()
}

/// Does a word array read as what E3D printed — as words, or as numbers?
///
/// E3D prints word arrays as the words themselves (`NUT WASH WASH NUT`, not
/// the four hashes behind them), so the names are tried before the numbers.
fn words_match_rendering(values: &[u32], rendered: &str) -> bool {
    let names: Vec<&str> = rendered.split_whitespace().collect();
    let names_match = names.len() == values.len()
        && values.iter().zip(&names).all(|(raw, name)| {
            crate::record::descriptor::word_name(*raw)
                .is_some_and(|text| text.eq_ignore_ascii_case(name))
        });
    names_match
        || list_matches_rendering(
            &values.iter().map(|value| *value as f64).collect::<Vec<_>>(),
            rendered,
        )
}

/// Does a stored list read as what E3D printed, item by item?
///
/// The two kinds can share one list: `SCOM.Param` holds
/// `[25, 50.8, 690548, 277, 120, 100, 257]` and E3D prints
/// `25 50.8 WFBH 277 120 100 257`, where only the middle item is a word. A
/// list compared as all numbers or all names disagrees with that on every
/// item, which says nothing about whether it was read correctly.
fn list_matches_rendering(values: &[f64], rendered: &str) -> bool {
    let tokens: Vec<&str> = rendered
        .trim_start_matches('(')
        .trim_end_matches(')')
        .split_whitespace()
        .filter(|token| !matches!(*token, "(" | ")"))
        .collect();
    tokens.len() == values.len()
        && values.iter().zip(&tokens).all(|(value, token)| {
            number_agrees(*value, token)
                || word_behind(*value).is_some_and(|name| name.eq_ignore_ascii_case(token))
        })
}

/// The word a list item stands for, when the item is one.
fn word_behind(value: f64) -> Option<String> {
    let raw = value as u32;
    if value < 0.0 || f64::from(raw) != value {
        return None;
    }
    crate::record::descriptor::word_name(raw)
}

fn parse_position(value: &str) -> Option<[f64; 3]> {
    let tokens = value.split_whitespace().collect::<Vec<_>>();
    if tokens.len() != 6 {
        return None;
    }
    let mut out = [0.0; 3];
    for pair in tokens.chunks_exact(2) {
        let magnitude = parse_leading_number(pair[1])?;
        let (axis, sign) = match pair[0].to_ascii_uppercase().as_str() {
            "E" => (0, 1.0),
            "W" => (0, -1.0),
            "N" => (1, 1.0),
            "S" => (1, -1.0),
            "U" => (2, 1.0),
            "D" => (2, -1.0),
            _ => return None,
        };
        out[axis] = sign * magnitude;
    }
    Some(out)
}

/// Unit vector of a bare compass letter, in (east, north, up).
fn compass_axis(letter: &str) -> Option<[f64; 3]> {
    match letter.to_ascii_uppercase().as_str() {
        "E" => Some([1.0, 0.0, 0.0]),
        "W" => Some([-1.0, 0.0, 0.0]),
        "N" => Some([0.0, 1.0, 0.0]),
        "S" => Some([0.0, -1.0, 0.0]),
        "U" => Some([0.0, 0.0, 1.0]),
        "D" => Some([0.0, 0.0, -1.0]),
        _ => None,
    }
}

fn parse_direction(value: &str) -> Option<[f64; 3]> {
    let tokens = value.split_whitespace().collect::<Vec<_>>();
    // An axis-aligned direction is printed as the bare compass letter: `U`,
    // not `N 0 U`.
    if let [single] = tokens[..] {
        return compass_axis(single);
    }
    let Some(start) = tokens.first() else {
        return None;
    };
    if tokens.len() < 3 || tokens.len() % 2 == 0 {
        return None;
    }
    // `W 5 S 0.0083 D` first turns five degrees from west towards south,
    // then tilts that horizontal direction 0.0083 degrees down.
    let mut direction = compass_axis(start)?;
    for pair in tokens[1..].chunks_exact(2) {
        let angle = parse_leading_number(pair[0])?.to_radians();
        let end = compass_axis(pair[1])?;
        let dot = direction.iter().zip(end).map(|(a, b)| a * b).sum::<f64>();
        if dot.abs() >= 1e-9 {
            return None;
        }
        direction = [
            direction[0] * angle.cos() + end[0] * angle.sin(),
            direction[1] * angle.cos() + end[1] * angle.sin(),
            direction[2] * angle.cos() + end[2] * angle.sin(),
        ];
    }
    Some(direction)
}

/// The two axis directions in `Y is W 10 S and Z is N 10 W`.
fn parse_orientation(value: &str) -> Option<([f64; 3], [f64; 3])> {
    let rest = value.trim().strip_prefix("Y is ")?;
    let (y, z) = rest.split_once(" and Z is ")?;
    Some((parse_direction(y)?, parse_direction(z)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::template::{EncodedLocation, StorageDescriptor, StorageLayout};
    use crate::RefNo;

    #[test]
    fn parser_preserves_duplicates_and_unset_values() {
        let capture = "NOUN-ATTLIST-BEGIN\nZZA Name\nName /A\nZZA :X\n:X unset\nZZA :X\n:X 1\nNOUN-ATTLIST-END\n";
        let rows = parse_attlist_capture(capture).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].rendered_value, "/A");
        assert_eq!(rows[1].name, ":X");
        assert_eq!(rows[2].rendered_value, "1");
    }

    #[test]
    fn parser_does_not_strip_a_longer_returned_heading() {
        let capture = "NOUN-ATTLIST-BEGIN\nZZA :Room\n:Roomfloor unset\nNOUN-ATTLIST-END\n";
        let rows = parse_attlist_capture(capture).unwrap();
        assert_eq!(rows[0].rendered_value, ":Roomfloor unset");
    }

    #[test]
    fn colon_uda_does_not_alias_same_named_system_attribute() {
        let extraction = ElementExtraction {
            refno: RefNo::new(1, 1),
            owner: RefNo::new(0, 0),
            noun_hash: 1,
            noun_name: Some("TEST".into()),
            implicit_word_count: 6,
            format_word_10: 0,
            layout: StorageLayout::Compact,
            attributes: vec![ExtractedAttribute {
                hash: 1,
                name: "TPRESS".into(),
                storage_type_code: 2,
                semantic_type_code: None,
                semantic_size: None,
                layout: StorageLayout::Compact,
                storage: StorageDescriptor {
                    storage_words: 1,
                    location: EncodedLocation { raw: 1 },
                    flags: 0,
                },
                status: AttributeExtractionStatus::Decoded,
                value: Some(DescriptorValue::Real(0.0)),
            }],
        };
        let tty = vec![TtyAttributeRow {
            ordinal: 0,
            name: ":TPress".into(),
            rendered_value: "unset".into(),
        }];
        let rows = reconcile_attlist(
            &tty,
            &extraction,
            &AttributeMetadataCatalog::default(),
            None,
        );
        assert_eq!(rows[0].source, CompleteAttributeSource::TtyAuthority);
    }

    #[test]
    fn system_heading_outranks_a_same_named_colon_uda() {
        let attribute = |hash: u32, name: &str, value: DescriptorValue| ExtractedAttribute {
            hash,
            name: name.into(),
            storage_type_code: 16,
            semantic_type_code: None,
            semantic_size: None,
            layout: StorageLayout::Wide,
            storage: StorageDescriptor {
                storage_words: 2,
                location: EncodedLocation { raw: 1 },
                flags: 0,
            },
            status: AttributeExtractionStatus::Decoded,
            value: Some(value),
        };
        let extraction = ElementExtraction {
            refno: RefNo::new(24383, 71313),
            owner: RefNo::new(0, 0),
            noun_hash: 821683,
            noun_name: Some("SCTN".into()),
            implicit_word_count: 45,
            format_word_10: 0,
            layout: StorageLayout::Wide,
            attributes: vec![
                attribute(
                    643429,
                    "Spref",
                    DescriptorValue::RefNo(RefNo::new(23984, 110872)),
                ),
                attribute(1, ":SPREF", DescriptorValue::RefNo(RefNo::new(0, 0))),
            ],
        };
        let tty = vec![TtyAttributeRow {
            ordinal: 0,
            name: "Spref".into(),
            rendered_value: "23984/110872".into(),
        }];

        let rows = reconcile_attlist(
            &tty,
            &extraction,
            &AttributeMetadataCatalog::default(),
            None,
        );
        assert_eq!(rows[0].binary_name.as_deref(), Some("Spref"));
        assert_eq!(rows[0].comparison, ValueComparison::Equivalent);
    }

    #[test]
    fn compares_e3d_position_and_direction_renderings_to_binary_vectors() {
        assert_eq!(
            compare_tty_value(
                &DescriptorValue::RealArray(vec![
                    -0.9961946877019986,
                    -0.08715574183867308,
                    -0.00014442599510043695,
                ]),
                "W 5 S 0.0083 D",
            ),
            ValueComparison::Equivalent
        );
        assert_eq!(
            compare_tty_value(
                &DescriptorValue::RealArray(vec![-3118.12, -8969.2, 333.6]),
                "W 3118.12mm S 8969.2mm U 333.6mm",
            ),
            ValueComparison::Equivalent
        );
        assert_eq!(
            compare_tty_value(
                &DescriptorValue::RealArray(vec![0.0, 0.9999878265012184, -0.004934252665720292,]),
                "N 0.2827 D",
            ),
            ValueComparison::Equivalent
        );
        assert_eq!(
            compare_tty_value(
                &DescriptorValue::RealArray(vec![-0.46422381113782396, 0.8857179309309899, 0.0,]),
                "N 27.66 W",
            ),
            ValueComparison::Equivalent
        );
        // ATTA.POS, printed to whole millimetres: the text cannot distinguish
        // 7036.2 from 7036, so it does not get to call it a disagreement.
        assert_eq!(
            compare_tty_value(
                &DescriptorValue::RealArray(vec![7036.2, -21611.8, 3200.0]),
                "E 7036mm S 21612mm U 3200mm",
            ),
            ValueComparison::Equivalent
        );
        // A difference the printed text could have shown is still a
        // disagreement.
        assert_eq!(
            compare_tty_value(
                &DescriptorValue::RealArray(vec![7036.2, -21611.8, 3200.0]),
                "E 7040mm S 21612mm U 3200mm",
            ),
            ValueComparison::Mismatch
        );
    }

    #[test]
    fn impstc_integer_default_is_rendered_as_undefined() {
        assert_eq!(
            compare_extracted_tty_value(
                "IMPSTC",
                &DescriptorValue::Int(0),
                "Undefined",
                StoredUnit::Unknown,
            ),
            ValueComparison::Equivalent
        );
        assert_eq!(
            compare_extracted_tty_value(
                "OTHER",
                &DescriptorValue::Int(0),
                "Undefined",
                StoredUnit::Unknown,
            ),
            ValueComparison::Mismatch
        );
    }

    /// Three numbers are not always a point. `SCOM.Param` holds a word among
    /// them, and the seven-item sibling of this row already reads correctly.
    #[test]
    fn three_numbers_that_are_not_a_point_are_read_as_the_list_they_are() {
        assert_eq!(
            compare_tty_value(
                &DescriptorValue::RealArray(vec![50.0, 61.2, 533486.0]),
                "50 61.2 TUB",
            ),
            ValueComparison::Equivalent
        );
        assert_eq!(
            compare_tty_value(
                &DescriptorValue::RealArray(vec![50.0, 61.2, 533486.0]),
                "50 61.3 TUB",
            ),
            ValueComparison::Mismatch
        );
    }

    /// `attlib` has no report name for the position vector or its flag, so
    /// the TTY heading is matched to the system name.
    #[test]
    fn tty_heading_reaches_an_attribute_attlib_never_names() {
        let attribute = |hash: u32, name: &str, value: DescriptorValue| ExtractedAttribute {
            hash,
            name: name.into(),
            storage_type_code: 2,
            semantic_type_code: None,
            semantic_size: None,
            layout: StorageLayout::Wide,
            storage: StorageDescriptor {
                storage_words: 1,
                location: EncodedLocation { raw: 1 },
                flags: 0,
            },
            status: AttributeExtractionStatus::Decoded,
            value: Some(value),
        };
        let extraction = ElementExtraction {
            refno: RefNo::new(1, 1),
            owner: RefNo::new(0, 0),
            noun_hash: 1,
            noun_name: Some("ATTA".into()),
            implicit_word_count: 6,
            format_word_10: 0,
            layout: StorageLayout::Wide,
            attributes: vec![
                attribute(
                    545713,
                    "POS",
                    DescriptorValue::RealArray(vec![7036.2, -21611.8, 3200.0]),
                ),
                attribute(722860, "POSF", DescriptorValue::Bool(true)),
            ],
        };
        let tty = ["Position E 7036mm S 21612mm U 3200mm", "Posflag true"]
            .iter()
            .enumerate()
            .map(|(ordinal, line)| {
                let (name, rendered_value) = line.split_once(' ').expect("name and value");
                TtyAttributeRow {
                    ordinal,
                    name: name.into(),
                    rendered_value: rendered_value.into(),
                }
            })
            .collect::<Vec<_>>();
        let rows = reconcile_attlist(
            &tty,
            &extraction,
            &AttributeMetadataCatalog::default(),
            None,
        );
        assert_eq!(rows[0].binary_name.as_deref(), Some("POS"));
        assert_eq!(rows[0].comparison, ValueComparison::Equivalent);
        assert_eq!(rows[1].binary_name.as_deref(), Some("POSF"));
        assert_eq!(rows[1].comparison, ValueComparison::Equivalent);
    }

    /// The catalog's exported query text is the heading itself, so it reaches
    /// an attribute whose recovered attlib name is an editor label.
    #[test]
    fn tty_heading_comes_from_the_catalog_query_text() {
        let catalog = NounCatalog::parse(
            r#"[{"noun":"SECTION","short":"SCTN","hash":1,"base":"SCTN","hard":"HARD",
                 "isUdet":false,"isPseudo":false,"isWorld":false,"isPrimary":true,
                 "visible":true,"dbTypes":[1],"attrs":[{"name":"BANG","hash":679457,"type":2,
                 "typeName":"DOUBLE","isArray":false,"maxSize":1,"trueSize":0,"isUda":false,
                 "isPseudo":true,"hidden":false,"noClaim":false,"def":"e2/153"}]}]"#,
            r#"{"679457":{"name":"BANG","f":{"QTXT":{"s":"Bangle"}}}}"#,
        )
        .expect("parse catalog");
        let extraction = ElementExtraction {
            refno: RefNo::new(1, 1),
            owner: RefNo::new(0, 0),
            noun_hash: 1,
            noun_name: Some("SCTN".into()),
            implicit_word_count: 16,
            format_word_10: 0,
            layout: StorageLayout::Wide,
            attributes: vec![ExtractedAttribute {
                hash: 679457,
                name: "BANG".into(),
                storage_type_code: 3,
                semantic_type_code: None,
                semantic_size: None,
                layout: StorageLayout::Wide,
                storage: StorageDescriptor {
                    storage_words: 1,
                    location: EncodedLocation { raw: 14 },
                    flags: 0,
                },
                status: AttributeExtractionStatus::Decoded,
                value: Some(DescriptorValue::Word { raw: 0, text: None }),
            }],
        };
        let tty = vec![TtyAttributeRow {
            ordinal: 0,
            name: "Bangle".into(),
            rendered_value: "0degree".into(),
        }];

        let without = reconcile_attlist(
            &tty,
            &extraction,
            &AttributeMetadataCatalog::default(),
            None,
        );
        assert_eq!(without[0].binary_name, None);

        let with = reconcile_attlist(
            &tty,
            &extraction,
            &AttributeMetadataCatalog::default(),
            Some(&catalog),
        );
        assert_eq!(with[0].binary_name.as_deref(), Some("BANG"));
        assert_eq!(with[0].comparison, ValueComparison::Equivalent);
    }

    /// `MBOL.Sbrarray` and `SCOM.Blrfarray`, the two reference arrays of the
    /// 2026-08-26 TTY sample.
    #[test]
    fn a_reference_array_is_named_entry_by_entry() {
        let row = |value: DescriptorValue, rendered: &str| CompleteAttributeRow {
            ordinal: 0,
            name: "Sbrarray".into(),
            rendered_value: rendered.into(),
            source: CompleteAttributeSource::BinaryExplicit,
            binary_name: Some("SBRA".into()),
            binary_status: Some(AttributeExtractionStatus::Decoded),
            binary_value: Some(value),
            resolved_reference_name: None,
            comparison: ValueComparison::NeedsReferenceResolution,
        };
        let mut names = HashMap::new();
        names.insert(RefNo::new(15192, 58748), "/MSAAD".to_string());
        names.insert(RefNo::new(15192, 58751), "/MSAAG".to_string());
        names.insert(RefNo::new(15192, 85074), "/MSIDBHP-4-24".to_string());

        let mut ten_slots_two_filled =
            std::iter::repeat_n(RefNo::new(0, 0), 10).collect::<Vec<_>>();
        ten_slots_two_filled[0] = RefNo::new(15192, 58748);
        ten_slots_two_filled[1] = RefNo::new(15192, 58751);
        let mut rows = vec![
            row(
                DescriptorValue::RefNoArray(ten_slots_two_filled),
                " 1 /MSAAD 2 /MSAAG",
            ),
            row(
                DescriptorValue::RefNoArray(vec![RefNo::new(15192, 85074)]),
                "/MSIDBHP-4-24",
            ),
            // A target no supplied database could name stays waiting rather
            // than being called a disagreement.
            row(
                DescriptorValue::RefNoArray(vec![RefNo::new(15192, 58748), RefNo::new(999, 1)]),
                " 1 /MSAAD 2 /SOMEWHERE",
            ),
        ];
        apply_reference_names(&mut rows, &names);
        assert_eq!(rows[0].comparison, ValueComparison::Equivalent);
        assert_eq!(
            rows[0].resolved_reference_name.as_deref(),
            Some("1 /MSAAD 2 /MSAAG")
        );
        assert_eq!(rows[1].comparison, ValueComparison::Equivalent);
        assert_eq!(
            rows[2].comparison,
            ValueComparison::NeedsReferenceResolution
        );
    }

    #[test]
    fn empty_word_slot_agrees_with_a_zero_printed_in_any_unit() {
        let empty = DescriptorValue::Word { raw: 0, text: None };
        for rendered in ["0", "0degree", "0mm", "unset", ""] {
            assert_eq!(
                compare_tty_value(&empty, rendered),
                ValueComparison::Equivalent,
                "empty word slot against {rendered:?}"
            );
        }
        // A printed value that is not zero stays a disagreement, and so does a
        // rendering of several numbers — a scalar slot cannot be all of them.
        for rendered in ["0.5degree", "1degree", "0 10"] {
            assert_eq!(
                compare_tty_value(&empty, rendered),
                ValueComparison::Mismatch,
                "empty word slot against {rendered:?}"
            );
        }
    }

    /// A value E3D wrapped is one value, not one value and some litter.
    #[test]
    fn parser_rejoins_a_wrapped_value() {
        let capture = "NOUN-ATTLIST-BEGIN\nZZA Blength\nBlength (600mm 620mm\n710mm 720mm )\nZZA Bweight\nBweight unset\nNOUN-ATTLIST-END\n";
        let rows = parse_attlist_capture(capture).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].rendered_value, "(600mm 620mm 710mm 720mm )");
        assert_eq!(rows[1].rendered_value, "unset");
    }

    /// The four ways E3D says "there is nothing here", against what the
    /// database actually holds in each case.
    #[test]
    fn absence_reads_the_same_in_both_directions() {
        for rendered in ["unset", "Nulref"] {
            assert_eq!(
                compare_tty_value(&DescriptorValue::Unset, rendered),
                ValueComparison::Equivalent
            );
        }
        // SDTE.Skey
        assert_eq!(
            compare_tty_value(&DescriptorValue::WordArray(vec![0, 0]), "unset"),
            ValueComparison::Equivalent
        );
        // PTCA.PDShape falls back to a zeroed catalogue tuple, which E3D
        // prints as `0` rather than `unset`.
        assert_eq!(
            compare_tty_value(&DescriptorValue::RawWords(vec![0, 0, 0, 0]), "0"),
            ValueComparison::Equivalent
        );
        // The same zeroed tuple stored on an element whose expression lives
        // elsewhere is not agreement — we have nothing to say about it.
        assert_eq!(
            compare_tty_value(
                &DescriptorValue::RawWords(vec![0, 0, 0, 0]),
                "( 0.5 * ATTRIB PARA[20 ] )"
            ),
            ValueComparison::Mismatch
        );
        // SCTN.Srelease
        assert_eq!(
            compare_tty_value(&DescriptorValue::Word { raw: 0, text: None }, ""),
            ValueComparison::Equivalent
        );
        // SDTE.Skey holds four blanks and E3D prints nothing; CATE.Skey holds
        // none and E3D prints `unset`. A text that is only padding says the
        // same thing either way.
        for (stored, rendered) in [("    ", ""), ("", "unset"), ("    ", "unset")] {
            assert_eq!(
                compare_tty_value(&DescriptorValue::Text(stored.into()), rendered),
                ValueComparison::Equivalent,
                "{stored:?} against {rendered:?}"
            );
        }
        // Padding around a real value is still padding, but the value itself
        // has to be there.
        assert_eq!(
            compare_tty_value(&DescriptorValue::Text("VCFL".into()), "VCFL"),
            ValueComparison::Equivalent
        );
        assert_eq!(
            compare_tty_value(&DescriptorValue::Text("VCFL".into()), "unset"),
            ValueComparison::Mismatch
        );
        // A value that is present must still be read: ATTA's :COMPTYPE is
        // unset in the record while E3D reports ANY.
        assert_eq!(
            compare_tty_value(&DescriptorValue::Unset, "ANY"),
            ValueComparison::Mismatch
        );
    }

    /// ATTA / BEND / ELBO / FTUB / REDU / WELD all hold 0xFFFFFFFF in `Rlock`
    /// and E3D prints -1 for every one of them.
    #[test]
    fn an_unresolvable_word_that_reads_as_negative_is_a_sentinel() {
        assert_eq!(
            compare_tty_value(
                &DescriptorValue::Word {
                    raw: 0xFFFF_FFFF,
                    text: None
                },
                "-1"
            ),
            ValueComparison::Equivalent
        );
        // A word that does resolve is its name, not its bits.
        assert_eq!(
            compare_tty_value(
                &DescriptorValue::Word {
                    raw: 0xFFFF_FFFF,
                    text: Some("SOMETHING".into())
                },
                "-1"
            ),
            ValueComparison::Mismatch
        );
    }

    /// PNOD / SNOD `Fixity` reaches the comparison as the characters of FREE
    /// packed behind their own byte count.
    #[test]
    fn count_prefixed_characters_read_as_the_text_they_spell() {
        assert_eq!(
            compare_tty_value(
                &DescriptorValue::RawWords(vec![4, 0x4652_4545, 0, 0]),
                "FREE"
            ),
            ValueComparison::Equivalent
        );
    }

    /// E3D prints to a fixed number of decimals, so agreement can only be
    /// asked to the precision it printed — and no further.
    #[test]
    fn a_number_only_has_to_agree_as_far_as_e3d_printed_it() {
        // ELBO.Angle
        assert_eq!(
            compare_tty_value(&DescriptorValue::Real(31.0915), "31.091degree"),
            ValueComparison::Equivalent
        );
        // A `Param` entry held as a 32-bit float.
        assert_eq!(
            compare_tty_value(&DescriptorValue::Real(50.7999992370605), "50.8"),
            ValueComparison::Equivalent
        );
        assert_eq!(
            compare_tty_value(&DescriptorValue::Real(31.5), "31.091degree"),
            ValueComparison::Mismatch
        );
        // Printing no decimals at all is still a precision, not a claim to be
        // exact: `COATIN.Cthickness` holds 0.01 and prints `0mm`. What it does
        // not excuse is more than half a unit.
        assert_eq!(
            compare_tty_value(&DescriptorValue::Real(0.01), "0mm"),
            ValueComparison::Equivalent
        );
        assert_eq!(
            compare_tty_value(&DescriptorValue::Real(0.6), "0mm"),
            ValueComparison::Mismatch
        );
    }

    #[test]
    fn non_null_named_reference_requires_name_resolution() {
        assert_eq!(
            compare_tty_value(&DescriptorValue::RefNo(RefNo::new(13245, 846025)), "/VMB1"),
            ValueComparison::NeedsReferenceResolution
        );
    }

    /// `SPCO.Catref` points at 15192/28820 and E3D prints
    /// `Unknown Ref (=15192/28820)`: it read the same RefNo and had nothing to
    /// name it with either.
    #[test]
    fn a_reference_e3d_could_not_name_agrees_on_the_refno_it_printed() {
        assert_eq!(
            compare_tty_value(
                &DescriptorValue::RefNo(RefNo::new(15192, 28820)),
                "Unknown Ref (=15192/28820)"
            ),
            ValueComparison::Equivalent
        );
        // Agreement is on the number, so a different one is still a
        // disagreement rather than a shared shrug.
        assert_eq!(
            compare_tty_value(
                &DescriptorValue::RefNo(RefNo::new(15192, 28821)),
                "Unknown Ref (=15192/28820)"
            ),
            ValueComparison::NeedsReferenceResolution
        );
    }

    /// `SBOL.Xtralength` holds 12.5 and E3D prints `13mm`; `COATIN.Cthickness`
    /// holds 0.01 and prints `0mm`.
    #[test]
    fn a_fractional_value_may_be_printed_rounded_to_a_whole_number() {
        for (stored, rendered) in [(12.5, "13mm"), (0.01, "0mm"), (9.5, "10mm")] {
            assert_eq!(
                compare_tty_value(&DescriptorValue::Real(stored), rendered),
                ValueComparison::Equivalent,
                "{stored} against {rendered:?}"
            );
        }
        // Rounding explains half a unit, not more.
        assert_eq!(
            compare_tty_value(&DescriptorValue::Real(12.4), "13mm"),
            ValueComparison::Mismatch
        );
        // A whole number was not rounded on its way to the screen, so it has
        // to be the number that was printed.
        assert_eq!(
            compare_tty_value(&DescriptorValue::Int(12), "13"),
            ValueComparison::Mismatch
        );
    }

    #[test]
    fn resolved_reference_name_closes_the_comparison() {
        let mut rows = vec![CompleteAttributeRow {
            ordinal: 0,
            name: "Pspec".into(),
            rendered_value: "/VMB1".into(),
            source: CompleteAttributeSource::BinaryImplicit,
            binary_name: Some("PSPE".into()),
            binary_status: Some(AttributeExtractionStatus::Decoded),
            binary_value: Some(DescriptorValue::RefNo(RefNo::new(13245, 846025))),
            resolved_reference_name: None,
            comparison: ValueComparison::NeedsReferenceResolution,
        }];
        let names = HashMap::from([(RefNo::new(13245, 846025), "/VMB1".into())]);
        apply_reference_names(&mut rows, &names);
        assert_eq!(rows[0].comparison, ValueComparison::Equivalent);
        assert_eq!(rows[0].resolved_reference_name.as_deref(), Some("/VMB1"));
    }
}
