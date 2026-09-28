//! `e3d-picture`: look inside DRAFT picture files (GW / Wigwam GTIGS).
//!
//! ```text
//! e3d-picture info <file>                      header, name, tables, counts, segment tree
//! e3d-picture list <file> [--page] [--csv] [-o <out>]
//!                                              one line per primitive (gwdump.py --list / --csv)
//! e3d-picture roundtrip <file | dir> [-v]      decode → re-encode → byte compare (the corpus gate)
//! e3d-picture locate --db <padd file | dir> --pic-dir <dir> [--refno w0/w1 | --name <name>] [--schema <pack>]
//!                                              the picture file of each sheet, and whether it is there
//! e3d-picture thumbnail --db <padd file | dir> (--refno w0/w1 | --name <name>) -o <png> [--schema <pack>]
//!                                              the sheet's stored 128 × 128 thumbnail as PNG
//! ```
//!
//! `list` and `roundtrip` print exactly what `D:\ida_scratch\draft31\gwdump.py`
//! prints for the same input, so the two can be diffed. `--page` places
//! coordinates on the sheet (mm) through the composed segment transforms;
//! without it they are the segment's own. `roundtrip` exits 2 when any item
//! did not re-encode to its bytes. `locate` without `--refno` / `--name`
//! lists every picture owner (sheets, overlays, backgrounds, libraries) and
//! ends with a census; it exits 2 when a sheet's file is missing.

use std::collections::BTreeMap;
use std::env;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use e3d_io::picture::listing::{list_header, list_lines, summary, CSV_HEADER};
use e3d_io::picture::locate::{locate, picture_version, thumbnail, PaddProject};
use e3d_io::picture::roundtrip::{roundtrip, roundtrip_dir};
use e3d_io::picture::PictureFile;
use e3d_io::{DbElement, RefNo, SchemaPack};

const DEFAULT_SCHEMA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schema/e3d31");

const USAGE: &str = "usage:
  e3d-picture info <file>
  e3d-picture list <file> [--page] [--csv] [-o <out>]
  e3d-picture roundtrip <file | dir> [-v]
  e3d-picture locate --db <padd file | dir> --pic-dir <dir> [--refno w0/w1 | --name <name>] [--schema <pack>]
  e3d-picture thumbnail --db <padd file | dir> (--refno w0/w1 | --name <name>) -o <png> [--schema <pack>]";

enum Failure {
    Usage(String),
    Run(String),
    /// A round trip that found differences: already reported, exit 2.
    Inexact,
}

fn main() {
    match run() {
        Ok(()) => {}
        Err(Failure::Usage(message)) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
        Err(Failure::Run(message)) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
        Err(Failure::Inexact) => std::process::exit(2),
    }
}

fn run() -> Result<(), Failure> {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut positional = Vec::new();
    let mut page = false;
    let mut csv = false;
    let mut verbose = false;
    let mut out: Option<PathBuf> = None;
    let mut db: Option<PathBuf> = None;
    let mut pic_dir: Option<PathBuf> = None;
    let mut refno: Option<String> = None;
    let mut name: Option<String> = None;
    let mut schema = PathBuf::from(DEFAULT_SCHEMA);
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        let mut value = |flag: &str| {
            iter.next()
                .ok_or_else(|| Failure::Usage(format!("{flag} wants a value\n{USAGE}")))
        };
        match arg.as_str() {
            "--page" => page = true,
            "--csv" => csv = true,
            "-v" | "--verbose" => verbose = true,
            "-o" | "--out" => out = Some(PathBuf::from(value("-o")?)),
            "--db" => db = Some(PathBuf::from(value("--db")?)),
            "--pic-dir" => pic_dir = Some(PathBuf::from(value("--pic-dir")?)),
            "--refno" => refno = Some(value("--refno")?),
            "--name" => name = Some(value("--name")?),
            "--schema" => schema = PathBuf::from(value("--schema")?),
            "-h" | "--help" => return Err(Failure::Usage(USAGE.to_string())),
            other if other.starts_with('-') => {
                return Err(Failure::Usage(format!("unknown option {other:?}\n{USAGE}")))
            }
            _ => positional.push(arg),
        }
    }
    let mut positional = positional.into_iter();
    let command = positional
        .next()
        .ok_or_else(|| Failure::Usage(USAGE.to_string()))?;
    let wants_path = matches!(command.as_str(), "info" | "list" | "roundtrip");
    let target = positional.next().map(PathBuf::from);
    if positional.next().is_some() {
        return Err(Failure::Usage(format!("too many arguments\n{USAGE}")));
    }
    let target = match (wants_path, target) {
        (true, Some(target)) => Some(target),
        (true, None) => return Err(Failure::Usage(format!("{command} wants a path\n{USAGE}"))),
        (false, Some(extra)) => {
            return Err(Failure::Usage(format!(
                "{command} takes options, not {}\n{USAGE}",
                extra.display()
            )))
        }
        (false, None) => None,
    };
    let selection = Selection { refno, name };

    match command.as_str() {
        "info" => info(&target.unwrap()),
        "list" => list(&target.unwrap(), page, csv, out.as_deref()),
        "roundtrip" => roundtrip_command(&target.unwrap(), verbose),
        "locate" => {
            let db = db.ok_or_else(|| Failure::Usage(format!("locate wants --db\n{USAGE}")))?;
            let pic_dir = pic_dir
                .ok_or_else(|| Failure::Usage(format!("locate wants --pic-dir\n{USAGE}")))?;
            locate_command(&schema, &db, &pic_dir, &selection)
        }
        "thumbnail" => {
            let db = db.ok_or_else(|| Failure::Usage(format!("thumbnail wants --db\n{USAGE}")))?;
            let out =
                out.ok_or_else(|| Failure::Usage(format!("thumbnail wants -o <png>\n{USAGE}")))?;
            if selection.is_empty() {
                return Err(Failure::Usage(format!(
                    "thumbnail wants --refno or --name\n{USAGE}"
                )));
            }
            thumbnail_command(&schema, &db, &selection, &out)
        }
        other => Err(Failure::Usage(format!(
            "unknown command {other:?}\n{USAGE}"
        ))),
    }
}

/// Which element(s) `locate` / `thumbnail` act on.
struct Selection {
    refno: Option<String>,
    name: Option<String>,
}

impl Selection {
    fn is_empty(&self) -> bool {
        self.refno.is_none() && self.name.is_none()
    }

    /// The selected elements, or every picture owner when nothing is selected.
    fn elements(&self, project: &PaddProject) -> Result<Vec<DbElement>, Failure> {
        if let Some(refno) = &self.refno {
            let (w0, w1) = refno
                .split_once('/')
                .and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<i64>().ok()?)))
                .ok_or_else(|| Failure::Usage(format!("--refno wants w0/w1, not {refno:?}")))?;
            return Ok(vec![project.element(RefNo::new(w0, w1 as u32))]);
        }
        if let Some(name) = &self.name {
            let found = project
                .find_named(name)
                .map_err(|e| Failure::Run(e.to_string()))?;
            if found.is_empty() {
                return Err(Failure::Run(format!("no element is named {name}")));
            }
            return Ok(found);
        }
        project
            .picture_owners()
            .map_err(|e| Failure::Run(e.to_string()))
    }
}

fn open_project(schema: &Path, db: &Path) -> Result<PaddProject, Failure> {
    let pack = SchemaPack::open(schema).map_err(|e| Failure::Run(e.to_string()))?;
    PaddProject::open(&pack, db).map_err(|e| Failure::Run(e.to_string()))
}

fn run_error(error: impl std::fmt::Display) -> Failure {
    Failure::Run(error.to_string())
}

fn locate_command(
    schema: &Path,
    db: &Path,
    pic_dir: &Path,
    selection: &Selection,
) -> Result<(), Failure> {
    let project = open_project(schema, db)?;
    let elements = selection.elements(&project)?;
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    // noun → (owners, PVNO ≤ 0, found, missing)
    let mut census: BTreeMap<String, (usize, usize, usize, usize)> = BTreeMap::new();
    let mut sheets_missing = 0usize;
    writeln!(out, "noun\trefno\tname\texfi\tpvno\tfile\tcopies").map_err(run_error)?;
    for element in &elements {
        let noun = element.element_type().map_err(run_error)?;
        let name = element
            .stored_name()
            .map_err(run_error)?
            .unwrap_or_default();
        let entry = census.entry(noun.clone()).or_default();
        entry.0 += 1;
        let Some((exfi, pvno)) = picture_version(element).map_err(run_error)? else {
            entry.1 += 1;
            writeln!(out, "{noun}\t{}\t{name}\t\t\t(no PVNO)\t", element.refno())
                .map_err(run_error)?;
            continue;
        };
        let Some(location) = locate(pic_dir, element).map_err(run_error)? else {
            entry.1 += 1;
            writeln!(
                out,
                "{noun}\t{}\t{name}\t{exfi}\t{pvno}\t(no picture)\t",
                element.refno()
            )
            .map_err(run_error)?;
            continue;
        };
        let status = if location.exists() {
            entry.2 += 1;
            location.path.display().to_string()
        } else {
            entry.3 += 1;
            if noun == "SHEE" || noun == "OVER" {
                sheets_missing += 1;
            }
            format!("MISSING {}", location.path.display())
        };
        let copies: Vec<String> = location
            .copies()
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        writeln!(
            out,
            "{noun}\t{}\t{name}\t{exfi}\t{pvno}\t{status}\t{}",
            element.refno(),
            copies.join(" ")
        )
        .map_err(run_error)?;
    }
    writeln!(out, "\nnoun: owners / PVNO<=0 / found / missing").map_err(run_error)?;
    for (noun, (n, zero, found, missing)) in &census {
        writeln!(out, "  {noun:<8} {n:>5} {zero:>5} {found:>5} {missing:>5}").map_err(run_error)?;
    }
    out.flush().map_err(run_error)?;
    if sheets_missing > 0 {
        Err(Failure::Inexact)
    } else {
        Ok(())
    }
}

fn thumbnail_command(
    schema: &Path,
    db: &Path,
    selection: &Selection,
    out: &Path,
) -> Result<(), Failure> {
    let project = open_project(schema, db)?;
    let elements = selection.elements(&project)?;
    let sheet = elements
        .iter()
        .find(|e| e.element_type().ok().as_deref() == Some("SHEE"))
        .or(elements.first())
        .ok_or_else(|| Failure::Run("nothing selected".to_string()))?;
    let thumb = thumbnail(sheet)
        .map_err(run_error)?
        .ok_or_else(|| Failure::Run(format!("{} has no thumbnail", sheet.refno())))?;
    std::fs::write(out, &thumb.png).map_err(|e| Failure::Run(format!("{}: {e}", out.display())))?;
    println!(
        "{} {}: ELDATA payload {} bytes -> {} bytes PNG -> {}",
        sheet.element_type().unwrap_or_default(),
        sheet.refno(),
        thumb.payload.len(),
        thumb.png.len(),
        out.display()
    );
    Ok(())
}

fn open(path: &Path) -> Result<PictureFile, Failure> {
    PictureFile::open(path).map_err(|e| Failure::Run(e.to_string()))
}

fn info(path: &Path) -> Result<(), Failure> {
    let file = open(path)?;
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    for line in summary(&file) {
        writeln!(out, "{line}").map_err(|e| Failure::Run(e.to_string()))?;
    }
    let last = file.last_used_record();
    if (last as usize) < file.records() {
        writeln!(
            out,
            "trailing  : records {}..{} are not referenced (residue of an earlier save)",
            last + 1,
            file.records()
        )
        .map_err(|e| Failure::Run(e.to_string()))?;
    }
    Ok(())
}

fn list(path: &Path, page: bool, csv: bool, out: Option<&Path>) -> Result<(), Failure> {
    let file = open(path)?;
    let lines = list_lines(&file, page, csv);
    let write = |sink: &mut dyn Write| -> io::Result<()> {
        if csv {
            writeln!(sink, "{CSV_HEADER}")?;
        } else {
            writeln!(sink, "{}", list_header(&file, page))?;
        }
        for line in &lines {
            writeln!(sink, "{line}")?;
        }
        Ok(())
    };
    match out {
        Some(out_path) => {
            let mut sink = BufWriter::new(
                File::create(out_path)
                    .map_err(|e| Failure::Run(format!("{}: {e}", out_path.display())))?,
            );
            write(&mut sink).map_err(|e| Failure::Run(e.to_string()))?;
            sink.flush().map_err(|e| Failure::Run(e.to_string()))?;
            println!("wrote {} primitives -> {}", lines.len(), out_path.display());
        }
        None => {
            let stdout = io::stdout();
            let mut sink = BufWriter::new(stdout.lock());
            write(&mut sink).map_err(|e| Failure::Run(e.to_string()))?;
        }
    }
    Ok(())
}

fn roundtrip_command(path: &Path, verbose: bool) -> Result<(), Failure> {
    if path.is_dir() {
        let report = roundtrip_dir(path, |one| {
            if verbose {
                eprintln!(
                    "{} {}{}",
                    one.path.display(),
                    one.stats_line(),
                    if one.is_exact() { "" } else { "  MISMATCH" }
                );
            }
        })
        .map_err(|e| Failure::Run(e.to_string()))?;
        for line in report.lines() {
            println!("{line}");
        }
        return if report.is_exact() {
            Ok(())
        } else {
            Err(Failure::Inexact)
        };
    }
    let file = open(path)?;
    let report = roundtrip(&file).map_err(|e| Failure::Run(e.to_string()))?;
    println!("roundtrip {}: {}", path.display(), report.stats_line());
    let kinds: Vec<String> = report
        .kinds
        .iter()
        .map(|(&k, n)| format!("{}({k})={n}", e3d_io::picture::prim::kind_name(k)))
        .collect();
    println!("  prim kinds: {}", kinds.join(", "));
    for mismatch in &report.mismatches {
        println!("  mismatch {mismatch:?}");
    }
    if report.is_exact() {
        Ok(())
    } else {
        Err(Failure::Inexact)
    }
}
