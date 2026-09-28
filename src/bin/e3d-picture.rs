//! `e3d-picture`: look inside DRAFT picture files (GW / Wigwam GTIGS).
//!
//! ```text
//! e3d-picture info <file>                      header, name, tables, counts, segment tree
//! e3d-picture list <file> [--page] [--csv] [-o <out>]
//!                                              one line per primitive (gwdump.py --list / --csv)
//! e3d-picture roundtrip <file | dir> [-v]      decode → re-encode → byte compare (the corpus gate)
//! ```
//!
//! `list` and `roundtrip` print exactly what `D:\ida_scratch\draft31\gwdump.py`
//! prints for the same input, so the two can be diffed. `--page` places
//! coordinates on the sheet (mm) through the composed segment transforms;
//! without it they are the segment's own. `roundtrip` exits 2 when any item
//! did not re-encode to its bytes.

use std::env;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use e3d_io::picture::listing::{list_header, list_lines, summary, CSV_HEADER};
use e3d_io::picture::roundtrip::{roundtrip, roundtrip_dir};
use e3d_io::picture::PictureFile;

const USAGE: &str = "usage:
  e3d-picture info <file>
  e3d-picture list <file> [--page] [--csv] [-o <out>]
  e3d-picture roundtrip <file | dir> [-v]";

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
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--page" => page = true,
            "--csv" => csv = true,
            "-v" | "--verbose" => verbose = true,
            "-o" => {
                out = Some(PathBuf::from(iter.next().ok_or_else(|| {
                    Failure::Usage(format!("-o wants a path\n{USAGE}"))
                })?))
            }
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
    let target = positional
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| Failure::Usage(format!("{command} wants a path\n{USAGE}")))?;
    if positional.next().is_some() {
        return Err(Failure::Usage(format!("too many arguments\n{USAGE}")));
    }

    match command.as_str() {
        "info" => info(&target),
        "list" => list(&target, page, csv, out.as_deref()),
        "roundtrip" => roundtrip_command(&target, verbose),
        other => Err(Failure::Usage(format!(
            "unknown command {other:?}\n{USAGE}"
        ))),
    }
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
