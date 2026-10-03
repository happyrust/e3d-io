//! The Corpus of the write path's third phase (`.kiro/specs/write-path-phase-3`,
//! requirements, Glossary): the installed databases every Corpus check runs
//! over, named here once.
//!
//! A list, not `E3D_CORPUS_ROOTS`: a check the spec makes a gate (requirement
//! 2.2 — a Corpus test, 0 failed and not skipped) must not turn into a skip
//! because a variable is unset. That variable names whole installations for
//! the release-mode gates that walk them (`refno_dbno_corpus.rs`), which is a
//! different question.

#![allow(dead_code)]

pub const AMS8000: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";
pub const AMS7999: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7999_0001";
pub const AMS7351: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7351_0001";
pub const AMS251142: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams251142_0001";
pub const TES023: &str = r"D:\AVEVA\Projects\E3D3.1\TEST\TES000\TES023";

/// The databases the requirements name.
pub const CORPUS: &[&str] = &[AMS8000, AMS7999, AMS7351, AMS251142, TES023];

/// SPAMAP_Baseline: `ams8000_0001` as of session 640, the table the
/// 2026-09-22 evidence counted. Selected by number, so the sessions E3D has
/// saved onto the file since do not move it.
pub const SPAMAP_BASELINE: &str = AMS8000;
pub const SPAMAP_BASELINE_SESSION: u32 = 640;
