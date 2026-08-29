//! Elements to capture next, chosen so the six undecoded encodings get more
//! than one reading each.
//!
//! `Ptcdirection`, `Paxis` kind 12, `Pxbshear` and `Qusexpression` each appear
//! once or twice in the 2026-08-26 TTY sample, which is not enough to recover
//! a variable-length encoding from. The databases hold every element of those
//! nouns; this reports one element per distinct stored value, as manifest rows
//! `Capture-AttlistLoop.ps1` can take straight in.
//!
//! Words are annotated with the name they dehash to. `Qusexpression` turned out
//! to carry DB1 name hashes inline — 641779 is `db1_hash("PIPE")` — so a stream
//! printed as text can be lined up against the words that produced it without
//! capturing anything first.
//!
//! Run with:
//! `cargo test --test scratch_pick_encoding_samples -- --ignored --nocapture`

use std::collections::BTreeMap;

use e3d_attlib::{db1_dehash, db1_hash, system_names, AttlibData};
use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::record::element::ParsedElement;
use e3d_io::record::template::StorageLayout;
use e3d_io::record::template::TemplateProvider;

use e3d_io::ReadOnlyEngine;

const AMS: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";
const SHADOW: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all";
const MANIFEST_OUT: &str = r"D:\work\plant-code\e3d-io-noun-descriptor\evidence\20260827-encoding-samples\sample-angles.tsv";

/// How a capture set is thinned. One TTY reading per group is enough; the
/// question each attribute asks decides what counts as the same reading.
#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code, reason = "kept for the next attribute that needs thinning")]
enum GroupBy {
    /// Every distinct stored value earns a capture.
    WholeValue,
    /// Only the word before the terminator varies in a way we cannot read;
    /// the rest of the stream repeats. Capture one element per code.
    TrailingCode,
}

/// `(noun, attribute, how to thin the capture set)` for every unread encoding.
const WANTED: &[(&str, &str, GroupBy)] = &[
    // The 2026-08-27 capture settled the six direction codes (11..16 are
    // X, -X, Y, -Y, Z, -Z). What is still unread is the angle between them,
    // and that varies inside a direction code — so every stored value earns
    // its own reading now.
    ("PTCA", "PTCD", GroupBy::WholeValue),
    ("PTMI", "PAXI", GroupBy::WholeValue),
    ("SSLC", "PXBS", GroupBy::WholeValue),
    ("SSLC", "PXTS", GroupBy::WholeValue),
    ("SSLC", "PYBS", GroupBy::WholeValue),
    ("SSLC", "PYTS", GroupBy::WholeValue),
    ("TABHQU", "QUSEXP", GroupBy::WholeValue),
    ("TABQUE", "QUSEXP", GroupBy::WholeValue),
];

fn words_of(value: &DescriptorValue) -> Option<Vec<u32>> {
    match value {
        DescriptorValue::RawWords(words) | DescriptorValue::WordArray(words) => Some(words.clone()),
        DescriptorValue::IntArray(words) => Some(words.iter().map(|w| *w as u32).collect()),
        DescriptorValue::Word { raw, .. } => Some(vec![*raw]),
        _ => None,
    }
}

/// A word is shown with the name it dehashes to when one exists.
///
/// The base-27 range starts at 0x81BF2, so opcodes and small counts stay bare;
/// only words that could be a name at all get a name printed next to them.
fn annotate(words: &[u32]) -> String {
    words
        .iter()
        .map(|&word| match system_names::lookup_by_hash(word) {
            Some(name) => format!("{word}={name}"),
            None => match db1_dehash(word) {
                Some(name) if name.len() >= 3 => format!("{word}~{name}"),
                _ => format!("{word}"),
            },
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn describe(value: &DescriptorValue) -> String {
    match words_of(value) {
        Some(words) if words.len() == 1 => format!("w{}", words[0]),
        Some(words) => format!("{words:?}"),
        None => format!("{value:?}"),
    }
}

/// The reading a capture would settle, i.e. what makes two elements the same.
fn capture_key(value: &DescriptorValue, group: GroupBy) -> String {
    match (group, words_of(value)) {
        (GroupBy::TrailingCode, Some(words)) if words.len() >= 2 => {
            format!("len{} code{}", words[0], words[words.len() - 2])
        }
        _ => describe(value),
    }
}

#[test]
#[ignore = "requires the installed E3D 3.1 catalogue project"]
fn report_one_element_per_distinct_stored_value() {
    let attlib_path = format!("{SHADOW}\\attlib.dat");
    if !std::path::Path::new(&attlib_path).exists() {
        eprintln!("SKIP: {attlib_path} not present");
        return;
    }
    let attlib = AttlibData::parse_file(&attlib_path).expect("parse attlib");
    let db = format!("{AMS}\\ams7000_0001");
    let mut provider =
        TemplateProvider::open(format!("{SHADOW}\\catvir.dat"), Some(db1_hash("CATA")))
            .expect("open catvir");
    let mut engine = ReadOnlyEngine::open(&db).expect("open catalogue database");

    let mut templates = BTreeMap::new();
    for (noun, _, _) in WANTED {
        if templates.contains_key(noun) {
            continue;
        }
        if let Ok(Some(template)) = provider.template_for(db1_hash(noun)) {
            templates.insert(*noun, template.clone());
        }
    }

    // Every index entry, not one per RefNo. A Dabacon file keeps every version
    // of an element it has written and the index addresses all of them, so
    // taking the first copy of each RefNo hides whole nouns behind a stale
    // entry that decodes as something else. Which copy is current does not
    // matter here — the capture re-reads the element and compares against what
    // E3D prints, so a stale copy can only cost a wasted capture.
    let mut indexed = engine.scan_all_indexed_refnos().expect("scan index");
    indexed.sort_by_key(|(refno, page, offset)| (refno.word0, refno.word1, *page, *offset));

    // (noun, attribute) -> capture key -> stored values -> the elements.
    type ByValue = BTreeMap<String, Vec<String>>;
    let mut seen: BTreeMap<(&str, &str), BTreeMap<String, ByValue>> = BTreeMap::new();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for (refno, page, offset) in &indexed {
        let Ok(view) = engine.read_element_direct(*refno, *page, *offset) else {
            continue;
        };
        let Ok(parsed) = ParsedElement::parse(&view.raw_bytes) else {
            continue;
        };
        let Some((noun, template)) = templates
            .iter()
            .find(|(noun, _)| db1_hash(noun) == parsed.noun_hash)
        else {
            continue;
        };
        *counts.entry(noun).or_default() += 1;
        let Ok(extraction) =
            engine.extract_parsed_element_with_descriptors(*refno, &parsed, &attlib, template)
        else {
            continue;
        };
        let layout = match extraction.layout {
            StorageLayout::Wide => "wide",
            StorageLayout::Compact => "compact",
        };
        let row = format!(
            "ams7000_0001\t{noun}\t{layout}\t{}\t{refno}",
            extraction.implicit_word_count
        );
        for (wanted_noun, attribute, group) in WANTED {
            if wanted_noun != noun {
                continue;
            }
            let hash = db1_hash(attribute);
            let Some(value) = extraction
                .attributes
                .iter()
                .find(|item| item.hash == hash)
                .and_then(|item| item.value.as_ref())
            else {
                continue;
            };
            seen.entry((wanted_noun, attribute))
                .or_default()
                .entry(capture_key(value, *group))
                .or_default()
                .entry(describe(value))
                .or_default()
                .push(row.clone());
        }
    }

    println!("elements read by noun:");
    for (noun, count) in &counts {
        println!("  {noun:<8} {count}");
    }

    let mut manifest = Vec::new();
    // The stored words next to the element that holds them, so a capture log
    // can be lined up against them without scanning the database again.
    let mut values_tsv = Vec::new();
    for ((noun, attribute), keys) in &seen {
        let values: usize = keys.values().map(BTreeMap::len).sum();
        println!("\n=== {noun}.{attribute}: {values} distinct values in {} capture groups ===", keys.len());
        for (key, values) in keys {
            println!("  [{key}]");
            let mut first = true;
            for (value, rows) in values {
                let words = value
                    .trim_start_matches('w')
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .split(", ")
                    .filter_map(|word| word.parse::<u32>().ok())
                    .collect::<Vec<_>>();
                println!("    x{:<4} {}", rows.len(), annotate(&words));
                if first {
                    println!("          {}", rows[0]);
                    let refno = rows[0].rsplit('\t').next().unwrap_or_default();
                    let joined = words
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(" ");
                    values_tsv.push(format!("{refno}\t{noun}\t{attribute}\t{joined}"));
                    manifest.push(rows[0].clone());
                    first = false;
                }
            }
        }
    }

    write_manifest(MANIFEST_OUT, &mut manifest, &values_tsv);
}

fn write_manifest(target: &str, manifest: &mut Vec<String>, values_tsv: &[String]) {
    manifest.sort();
    manifest.dedup();
    let path = std::path::Path::new(target);
    std::fs::create_dir_all(path.parent().expect("manifest parent")).expect("create evidence dir");
    std::fs::write(path, format!("{}\n", manifest.join("\n"))).expect("write manifest");
    let values_path = path.with_file_name(format!(
        "{}-values.tsv",
        path.file_stem().expect("stem").to_string_lossy()
    ));
    std::fs::write(&values_path, format!("{}\n", values_tsv.join("\n"))).expect("write values");
    println!("stored words -> {}", values_path.display());
    println!("\n--- manifest ({} rows) -> {target} ---", manifest.len());
    for row in manifest.iter() {
        println!("{row}");
    }
}

/// Every noun the exported catalog gives a `PAXI`.
const AXIS_NOUNS: &[&str] = &[
    "SSLC", "TUBE", "SSPH", "SDSH", "PTMI", "BOXI", "NSSL", "LCYL", "SCYL", "SCON", "NSCO", "NSSP",
    "NSDS", "SDIS", "PTAX", "NLCY", "NSCY",
];

/// The catalogue databases the AMS `/ALL` MDB reads, so a hit here is an
/// element the TTY capture can actually navigate to.
/// `acp7008_0001` and `ams7997_0001` were scanned too and hold no `PAXI` at
/// all, so they are not worth the minute they cost.
const AXIS_SCAN_DBS: &[&str] = &[
    r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5052_0001",
    r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5053_0001",
    r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7000_0001",
];

const AXIS_MANIFEST_OUT: &str =
    r"D:\work\plant-code\e3d-io-noun-descriptor\evidence\20260827-encoding-samples\sample-axes.tsv";

/// Where does a `Paxis` pair that the decoder declines take its angle from?
///
/// `12` with `-4` prints `X DDANGLE Y` and `61` with `20` prints
/// `-Z PARAM 20X`, one reading each in `ams7000_0001`. This looks for more of
/// them across every catalogue database the MDB can reach, and names one
/// element per distinct pair.
#[test]
#[ignore = "requires the installed E3D 3.1 catalogue project"]
fn report_axis_pairs_the_decoder_declines() {
    let attlib_path = format!("{SHADOW}\\attlib.dat");
    if !std::path::Path::new(&attlib_path).exists() {
        eprintln!("SKIP: {attlib_path} not present");
        return;
    }
    let attlib = AttlibData::parse_file(&attlib_path).expect("parse attlib");
    let mut provider =
        TemplateProvider::open(format!("{SHADOW}\\catvir.dat"), Some(db1_hash("CATA")))
            .expect("open catvir");
    let templates: BTreeMap<&str, _> = AXIS_NOUNS
        .iter()
        .filter_map(|noun| {
            let template = provider.template_for(db1_hash(noun)).ok()??;
            Some((*noun, template.clone()))
        })
        .collect();
    let wanted = db1_hash("PAXI");

    // value -> the elements holding it, and whether the decoder reads it.
    let mut seen: BTreeMap<String, (bool, Vec<String>)> = BTreeMap::new();
    for path in AXIS_SCAN_DBS {
        let Some(db) = std::path::Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        let started = std::time::Instant::now();
        let Ok(mut engine) = ReadOnlyEngine::open(path) else {
            println!("{db:<16} could not be opened");
            continue;
        };
        let Ok(indexed) = engine.scan_all_indexed_refnos() else {
            println!("{db:<16} index unreadable");
            continue;
        };
        let mut hits = 0;
        for (refno, page, offset) in &indexed {
            let Ok(view) = engine.read_element_direct(*refno, *page, *offset) else {
                continue;
            };
            let Ok(parsed) = ParsedElement::parse(&view.raw_bytes) else {
                continue;
            };
            let Some((noun, template)) = templates
                .iter()
                .find(|(noun, _)| db1_hash(noun) == parsed.noun_hash)
            else {
                continue;
            };
            let Ok(extraction) =
                engine.extract_parsed_element_with_descriptors(*refno, &parsed, &attlib, template)
            else {
                continue;
            };
            let Some(words) = extraction
                .attributes
                .iter()
                .find(|item| item.hash == wanted)
                .and_then(|item| item.value.as_ref())
                .and_then(words_of)
            else {
                continue;
            };
            hits += 1;
            let layout = match extraction.layout {
                StorageLayout::Wide => "wide",
                StorageLayout::Compact => "compact",
            };
            let readable = e3d_io::record::axis_spec::render(&words).is_some();
            seen.entry(format!("{words:?}"))
                .or_insert((readable, Vec::new()))
                .1
                .push(format!(
                    "{db}\t{noun}\t{layout}\t{}\t{refno}",
                    extraction.implicit_word_count
                ));
        }
        println!(
            "{db:<16} {:>8} index entries, {hits:>6} with PAXI  ({}s)",
            indexed.len(),
            started.elapsed().as_secs()
        );
    }

    let mut manifest = Vec::new();
    let mut values_tsv = Vec::new();
    // Both groups earn a capture. The declined ones are the point of the
    // scan, but the pair rule was fitted to four readings and now answers
    // dozens of combinations that no one has ever seen E3D print, so those
    // need checking rather than trusting.
    for (readable, label) in [(false, "DECLINED"), (true, "read by the pair rule")] {
        println!("\n=== {label} ===");
        for (value, (_, rows)) in seen.iter().filter(|(_, (read, _))| *read == readable) {
            // The three-part kinds are the count-prefixed axis and point forms
            // the sample already settled; only the pairs are new.
            let is_pair_rule = value
                .trim_matches(['[', ']'])
                .split(", ")
                .next()
                .and_then(|kind| kind.parse::<u32>().ok())
                .is_some_and(|kind| (kind as i32) < 0);
            println!("  x{:<5} {value}", rows.len());
            if readable && !is_pair_rule {
                continue;
            }
            // One element per distinct pair, preferring a database the earlier
            // capture already proved reachable.
            let row = rows
                .iter()
                .find(|row| row.starts_with("ams7000_0001"))
                .unwrap_or(&rows[0]);
            println!("        {row}");
            let refno = row.rsplit('\t').next().unwrap_or_default();
            values_tsv.push(format!("{refno}\tPAXI\t{value}"));
            manifest.push(row.clone());
        }
    }
    write_manifest(AXIS_MANIFEST_OUT, &mut manifest, &values_tsv);
}
