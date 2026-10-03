//! The rule registry (`src/core_rules.rs`) keeps its own promises: one row per
//! rule under a unique name, every confirmed rule backed by an Evidence_Doc
//! whose 结论 names it, and the write side never reaching for candidates.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use e3d_io::core_rules::{RuleStatus, RULES};

const REPO: &str = env!("CARGO_MANIFEST_DIR");

#[test]
fn rule_names_are_unique() {
    let mut seen = HashSet::new();
    for rule in RULES {
        assert!(seen.insert(rule.name), "{} appears twice", rule.name);
    }
}

#[test]
fn every_confirmed_rule_is_stated_in_the_conclusion_of_its_evidence() {
    for rule in RULES {
        let RuleStatus::Confirmed { evidence } = rule.status else {
            continue;
        };
        let file = evidence.split('#').next().unwrap();
        let text = fs::read_to_string(Path::new(REPO).join(file))
            .unwrap_or_else(|error| panic!("{}: evidence {file}: {error}", rule.name));
        assert!(
            conclusions(&text).iter().any(|section| section.contains(rule.name)),
            "{}: no 结论 section of {file} names it",
            rule.name
        );
    }
}

/// The text of every section whose heading says 结论, up to the next heading
/// at its level or above.
fn conclusions(markdown: &str) -> Vec<String> {
    let lines: Vec<&str> = markdown.lines().collect();
    let level = |line: &str| line.chars().take_while(|&c| c == '#').count();
    let mut sections = Vec::new();
    for (start, line) in lines.iter().enumerate() {
        let depth = level(line);
        if depth == 0 || !line.contains("结论") {
            continue;
        }
        let body: Vec<&str> =
            lines[start + 1..].iter().take_while(|next| !(1..=depth).contains(&level(next))).copied().collect();
        sections.push(body.join("\n"));
    }
    sections
}

#[test]
fn the_write_side_never_names_candidates() {
    let mut offenders = Vec::new();
    visit(&Path::new(REPO).join("src/write"), &mut offenders);
    assert!(offenders.is_empty(), "src/write names candidates in {offenders:?}");
}

fn visit(dir: &Path, offenders: &mut Vec<String>) {
    for entry in fs::read_dir(dir).expect("list src/write") {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            visit(&path, offenders);
        } else if path.extension().is_some_and(|ext| ext == "rs")
            && fs::read_to_string(&path).expect("read a source file").contains("candidates")
        {
            offenders.push(path.display().to_string());
        }
    }
}

#[test]
fn the_conclusion_finder_reads_a_section_to_the_next_heading() {
    let doc = "# Evidence\n## 环境\nsha\n## 结论\n- spamap.key0 confirmed\n### detail\nstill in\n## 复现命令\nspamap.flags\n";
    let found = conclusions(doc);
    assert_eq!(found.len(), 1);
    assert!(found[0].contains("spamap.key0") && found[0].contains("still in"));
    assert!(!found[0].contains("spamap.flags"));
}
