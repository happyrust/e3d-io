//! Every expression this crate renders, counted, and written out so it can be
//! put beside what the write-through pipeline stored for the same element.
//!
//! Reconnaissance for `docs/specs/direct-mode-expression-dialect.md` in
//! gen-model. Two outputs:
//!
//! * a frequency table of the **syntax elements** that appear in rendered
//!   strings, per renderer and per noun — this is a full scan, not a sample;
//!   and
//! * a TSV of `refno / noun / attribute / renderer / rendered`, one line per
//!   rendered value, which joins against `SELECT * FROM <NOUN>` in SurrealDB
//!   on `id = <NOUN>:<word0>_<word1>`.
//!
//! ```text
//! cargo run --release --example genmodel_expr_dialect_probe -- \
//!     <db> <vir.dat> [out.tsv] [limit]
//! ```
//!
//! With no arguments it scans the two AMS catalogue databases and one design
//! database, writing `expr_dialect.tsv` next to the working directory.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::record::template::TemplateProvider;
use e3d_io::ReadOnlyEngine;

// Same schema pack the tests read, so a probe and the test it is chasing
// cannot end up on two different installations.
#[path = "../tests/common/mod.rs"]
mod common;

const ATTLIB: &str = common::ATTLIB;
const CATVIR: &str = common::CATVIR;
const DESVIR: &str = common::DESVIR;
const AMS: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Defaults into the temp directory rather than the working one: a 46 MB
    // untracked file in the repository root shows up in everybody's
    // `git status`.
    let out_path = args
        .get(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("expr_dialect.tsv"));
    let limit: usize = args
        .get(3)
        .and_then(|value| value.parse().ok())
        .unwrap_or(usize::MAX);

    let targets: Vec<(PathBuf, PathBuf)> = if args.len() >= 2 {
        vec![(PathBuf::from(&args[0]), PathBuf::from(&args[1]))]
    } else {
        vec![
            (Path::new(AMS).join("ams5052_0001"), PathBuf::from(CATVIR)),
            (Path::new(AMS).join("ams5053_0001"), PathBuf::from(CATVIR)),
            (Path::new(AMS).join("ams8000_0001"), PathBuf::from(DESVIR)),
        ]
    };

    let Ok(attlib) = e3d_attlib::AttlibData::parse_file(ATTLIB) else {
        println!("cannot parse {ATTLIB}");
        return;
    };

    let mut sink = BufWriter::new(File::create(&out_path).expect("create the TSV"));
    writeln!(sink, "db\trefno\tnoun\tattr\trenderer\trendered").expect("write the header");

    let mut total = Tally::default();
    for (db, vir) in &targets {
        scan(db, vir, &attlib, limit, &mut sink, &mut total);
    }
    sink.flush().expect("flush the TSV");

    total.report();
    println!("\nTSV written to {}", out_path.display());
}

#[derive(Default)]
struct Tally {
    databases: usize,
    elements: usize,
    extraction_failed: usize,
    values: usize,
    /// Rendered values per renderer.
    by_renderer: BTreeMap<&'static str, usize>,
    /// Values that are raw words no renderer claims.
    unrendered: usize,
    unrendered_examples: BTreeMap<String, usize>,
    /// Explicit-stream text values, which the descriptor path already renders
    /// through `catalogue_pml`.
    explicit_text: usize,
    /// Syntax element -> (occurrences, one real example, nouns it appears on).
    elements_seen: BTreeMap<&'static str, ElementStat>,
    /// Distinct rendered strings, capped, so the shapes can be eyeballed.
    distinct: BTreeMap<String, usize>,
}

#[derive(Default)]
struct ElementStat {
    count: usize,
    example: String,
    nouns: BTreeMap<String, usize>,
    attrs: BTreeMap<String, usize>,
}

impl Tally {
    fn note(&mut self, kind: &'static str, text: &str, noun: &str, attr: &str) {
        let stat = self.elements_seen.entry(kind).or_default();
        stat.count += 1;
        if stat.example.is_empty() {
            stat.example = text.to_string();
        }
        *stat.nouns.entry(noun.to_string()).or_default() += 1;
        *stat.attrs.entry(attr.to_string()).or_default() += 1;
    }

    fn report(&self) {
        println!("\n==== totals ====");
        println!("databases scanned: {}", self.databases);
        println!("elements extracted: {}", self.elements);
        println!(
            "descriptor extraction failed on: {}",
            self.extraction_failed
        );
        println!("attribute values inspected: {}", self.values);
        println!(
            "explicit-stream text values (already PML-rendered): {}",
            self.explicit_text
        );
        println!("\n==== raw word tuples, by renderer ====");
        for (renderer, count) in &self.by_renderer {
            println!("{renderer:<16} {count:>8}");
        }
        println!("{:<16} {:>8}", "(nothing)", self.unrendered);
        println!("\n-- shapes nothing renders, by first two words --");
        let mut items: Vec<(&String, &usize)> = self.unrendered_examples.iter().collect();
        items.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
        for (shape, count) in items.into_iter().take(20) {
            println!("  {count:>8}  {shape}");
        }

        println!("\n==== syntax elements in rendered strings ====");
        let mut items: Vec<(&&str, &ElementStat)> = self.elements_seen.iter().collect();
        items.sort_by_key(|(_, stat)| std::cmp::Reverse(stat.count));
        for (kind, stat) in items {
            println!(
                "{kind:<22} {:>8}   e.g. {:<44} nouns: {}",
                stat.count,
                truncate(&stat.example, 44),
                top(&stat.nouns, 6)
            );
            println!("{:<22} {:>8}   attrs: {}", "", "", top(&stat.attrs, 8));
        }

        println!("\n==== most common distinct rendered strings ====");
        let mut items: Vec<(&String, &usize)> = self.distinct.iter().collect();
        items.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
        for (text, count) in items.into_iter().take(40) {
            println!("  {count:>8}  {}", truncate(text, 100));
        }
    }
}

fn top(counts: &BTreeMap<String, usize>, n: usize) -> String {
    let mut items: Vec<(&String, &usize)> = counts.iter().collect();
    items.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
    let mut out = String::new();
    for (name, count) in items.into_iter().take(n) {
        let _ = write!(out, "{name}:{count} ");
    }
    out
}

fn truncate(text: &str, n: usize) -> String {
    match text.chars().count() > n {
        true => text.chars().take(n).collect::<String>() + "…",
        false => text.to_string(),
    }
}

fn scan(
    db: &Path,
    vir: &Path,
    attlib: &e3d_attlib::AttlibData,
    limit: usize,
    sink: &mut BufWriter<File>,
    tally: &mut Tally,
) {
    let Ok(mut provider) = TemplateProvider::open(vir, None) else {
        println!("cannot open descriptors {}", vir.display());
        return;
    };
    let Ok(mut engine) = ReadOnlyEngine::open(db) else {
        println!("cannot open {}", db.display());
        return;
    };
    let Ok(keys) = engine.indexed_refnos() else {
        println!("cannot walk {}", db.display());
        return;
    };
    let name = db
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let started = Instant::now();
    tally.databases += 1;
    let scanned = keys.len().min(limit);

    for (refno, _, _) in keys.iter().take(limit) {
        let extraction =
            match engine.extract_element_with_descriptors(*refno, attlib, &mut provider) {
                Ok(extraction) => extraction,
                Err(_) => {
                    tally.extraction_failed += 1;
                    continue;
                }
            };
        tally.elements += 1;
        let noun = extraction
            .noun_name
            .clone()
            .unwrap_or_else(|| format!("0x{:08X}", extraction.noun_hash));
        for attribute in &extraction.attributes {
            let (renderer, text) = match &attribute.value {
                Some(DescriptorValue::RawWords(words)) => {
                    tally.values += 1;
                    match render_by_shape(words) {
                        Some(pair) => pair,
                        None => {
                            tally.unrendered += 1;
                            let shape = format!("{:?}", &words[..words.len().min(2)]);
                            *tally.unrendered_examples.entry(shape).or_default() += 1;
                            continue;
                        }
                    }
                }
                // The descriptor path already runs `catalogue_pml` over an
                // explicit IntArray, so these arrive as text.
                Some(DescriptorValue::Text(value)) if is_expression(value) => {
                    tally.values += 1;
                    tally.explicit_text += 1;
                    ("explicit_text", value.clone())
                }
                _ => continue,
            };
            *tally.by_renderer.entry(renderer).or_default() += 1;
            classify(&text, &noun, &attribute.name, tally);
            if tally.distinct.len() < 200_000 {
                *tally.distinct.entry(text.clone()).or_default() += 1;
            }
            let _ = writeln!(
                sink,
                "{name}\t{}_{}\t{noun}\t{}\t{renderer}\t{}",
                refno.word0,
                refno.word1,
                attribute.name,
                text.replace('\t', " ").replace('\n', " ")
            );
        }
    }

    println!(
        "{name}: {scanned} keys scanned in {:.1} s",
        started.elapsed().as_secs_f64()
    );
}

/// A stored text value that is a formula rather than a name.
fn is_expression(value: &str) -> bool {
    value.contains("ATTRIB")
        || value.contains("PARAM")
        || value.contains(" OF ")
        || value.starts_with("( ")
}

/// The dispatcher `src/tty.rs` keeps private, in the same order.
fn render_by_shape(words: &[u32]) -> Option<(&'static str, String)> {
    if let Some(text) = e3d_io::record::catalogue_expr::render(words) {
        return Some(("catalogue_expr", text));
    }
    if let Some(text) = e3d_io::record::axis_spec::render(words) {
        return Some(("axis_spec", text));
    }
    if let Some(text) = e3d_io::record::direction_spec::render(words) {
        return Some(("direction_spec", text));
    }
    if let Some(text) = e3d_io::record::point_list::render(words) {
        return Some(("point_list", text));
    }
    let signed: Vec<i32> = words.iter().map(|word| *word as i32).collect();
    if let Some(text) = e3d_io::record::catalogue_pml::render(&signed) {
        return Some(("catalogue_pml", text));
    }
    e3d_io::record::text::packed(words).map(|text| ("text", text))
}

/// Count the syntax elements a rendered string contains.
///
/// Deliberately shallow: it looks for the exact spellings the renderers emit
/// (`src/record/{catalogue_expr,axis_spec,direction_spec,point_list,catalogue_pml}.rs`),
/// so a new spelling shows up as "unclassified" rather than being folded into
/// a bucket it does not belong to.
fn classify(text: &str, noun: &str, attr: &str, tally: &mut Tally) {
    let mut matched = false;
    let note = |kind: &'static str, tally: &mut Tally| {
        tally.note(kind, text, noun, attr);
    };

    if text.contains("ATTRIB ") {
        matched = true;
        if let Some(rest) = after(text, "ATTRIB ") {
            if rest.contains('[') {
                note("ATTRIB NAME[n ]", tally);
            } else if rest.split_whitespace().count() >= 2
                && !rest.split_whitespace().nth(1).is_some_and(is_operator)
            {
                note("ATTRIB NAME QUAL", tally);
            } else {
                note("ATTRIB NAME", tally);
            }
        }
    }
    if text.contains(" OF ") {
        matched = true;
        note("... OF ...", tally);
    }
    if text.contains("- PARAM ") {
        matched = true;
        note("- PARAM n", tally);
    } else if text.contains("IPARAM ") {
        matched = true;
        note("IPARAM n", tally);
    } else if text.contains("PARAM ") {
        matched = true;
        note("PARAM n", tally);
    }
    if text.contains(" TIMES ") {
        matched = true;
        note("c TIMES ...", tally);
    }
    for word in ["DDANGLE", "DDRADIUS", "DDHEIGHT"] {
        if text.contains(word) {
            matched = true;
            note("DDANGLE/RADIUS/HEIGHT", tally);
            break;
        }
    }
    if text.contains("DD#") {
        matched = true;
        note("DD#n (unnamed)", tally);
    }
    if text.starts_with("AXIS ") {
        matched = true;
        note("AXIS <dir>", tally);
    }
    if text
        .split_whitespace()
        .any(|token| matches!(token, "X" | "Y" | "Z" | "-X" | "-Y" | "-Z"))
    {
        matched = true;
        note("axis letter", tally);
    }
    if text.split_whitespace().any(|token| {
        token.starts_with('P') && token[1..].chars().all(|c| c.is_ascii_digit()) && token.len() > 1
    }) {
        matched = true;
        note("P<n> point", tally);
    }
    if text.split_whitespace().any(|token| {
        token.starts_with('T') && token[1..].chars().all(|c| c.is_ascii_digit()) && token.len() > 1
    }) {
        matched = true;
        note("T<n> tangent", tally);
    }
    for (needle, kind) in [
        ("MAX", "MAX(...)"),
        ("MIN", "MIN(...)"),
        ("IFTRUE", "IFTRUE(...)"),
        ("PI", "PI"),
        ("SQRT", "SQRT(...)"),
        ("ABS", "ABS(...)"),
        ("COS", "COS(...)"),
        ("SIN", "SIN(...)"),
        ("TAN", "TAN(...)"),
    ] {
        if text
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|w| w == needle)
        {
            matched = true;
            note(kind, tally);
        }
    }
    for (needle, kind) in [
        (" + ", "operator +"),
        (" - ", "operator -"),
        (" * ", "operator *"),
        (" / ", "operator /"),
    ] {
        if text.contains(needle) {
            matched = true;
            note(kind, tally);
        }
    }
    if text.starts_with("( ") {
        matched = true;
        note("parenthesised", tally);
    }
    if text.parse::<f64>().is_ok() {
        matched = true;
        note("bare number", tally);
    }
    if !matched {
        note("UNCLASSIFIED", tally);
    }
}

fn after<'a>(text: &'a str, needle: &str) -> Option<&'a str> {
    text.find(needle).map(|at| &text[at + needle.len()..])
}

fn is_operator(token: &str) -> bool {
    matches!(token, "+" | "-" | "*" | "/" | ")" | "(")
}
