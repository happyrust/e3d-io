//! Golden hashes of the files the write tests leave behind
//! (`.kiro/specs/write-path-phase-3/design.md`, Scale_Probe 与 Golden_Hashes).
//!
//! Every [`DbCopy`](super::write_fixture::DbCopy) hands its file here when it
//! is dropped. A session only ever appends, so the final file holds every
//! batch the test wrote on that copy, and one hash covers them all.
//!
//! A copy is named by the test crate, the test (libtest names each test's
//! thread after it) and how many copies that test had made before it. Not by
//! the line that made it: the tests these hashes guard get edited while the
//! hashes must keep holding — an explicit `with_spatial(Off)`, a changed
//! expected refno — and a line number would orphan every recording below the
//! edit without a word.
//!
//! - With `E3DIO_GOLDEN_RECORD=<file>`, each copy appends one JSON line to
//!   `<file>`. `tools/golden/record.ps1` runs the tests twice this way and
//!   keeps what both runs agree on.
//! - Otherwise every `tests/golden/*.json` is loaded once, and a copy one of
//!   them names is checked: first that it was taken from the bytes the golden
//!   was recorded on, then that the file it ends as is the recorded one. The
//!   two failures read differently on purpose — the first is the sample
//!   having moved, the second is the code. A copy no golden names is not
//!   hashed at all.

#![allow(dead_code)]

use std::cell::Cell;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::panic::Location;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use sha2::{Digest, Sha256};

/// Where the golden files live, one JSON file per purpose.
pub const GOLDEN_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden");

/// Names the file to record into; unset, the copies are checked instead.
pub const RECORD_ENV: &str = "E3DIO_GOLDEN_RECORD";

thread_local! {
    static COPIES_MADE: Cell<u32> = const { Cell::new(0) };
}

/// The name the next copy made on this thread goes by.
pub fn next_key() -> String {
    let seq = COPIES_MADE.with(|made| {
        let seq = made.get();
        made.set(seq + 1);
        seq
    });
    let thread = std::thread::current();
    format!("{}::{}#{seq}", env!("CARGO_CRATE_NAME"), thread.name().unwrap_or("<unnamed>"))
}

/// What one golden file says about one copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    /// File name of the sample the copy was taken from.
    pub source: String,
    pub source_sha256: String,
    pub sha256: String,
    pub len: u64,
}

/// A copy at the end of its test, as `DbCopy` hands it over.
pub struct Finished<'a> {
    pub key: &'a str,
    pub source: &'a Path,
    /// The copy's bytes before any write — the sample as it was copied.
    pub original: &'a [u8],
    pub path: &'a Path,
    pub made_at: &'static Location<'static>,
}

/// Record or check one finished copy, per [`RECORD_ENV`].
pub fn finish(copy: Finished<'_>) {
    if let Some(out) = std::env::var_os(RECORD_ENV) {
        record(out, &copy);
        return;
    }
    let Some(recorded) = goldens().get(copy.key) else {
        return;
    };
    if let Err(why) = verify(recorded, &copy) {
        panic!("{why}");
    }
}

/// Check `copy` against every golden entry recorded under its name.
pub fn verify(recorded: &[(String, Recorded)], copy: &Finished<'_>) -> Result<(), String> {
    let source_sha256 = sha256_hex(copy.original);
    let bytes = fs::read(copy.path).map_err(|error| {
        format!("golden [{}]: cannot read the copy {}: {error}", copy.key, copy.path.display())
    })?;
    let sha256 = sha256_hex(&bytes);
    for (file, want) in recorded {
        if source_sha256 != want.source_sha256 {
            return Err(format!(
                "golden {file} [{}]: sample drift, not a code change — the copy made at {} was taken from {} \
                 with SHA-256 {source_sha256}, and the golden was recorded on {} with SHA-256 {}. \
                 Pin the sample (tools/golden/snapshot.ps1) or re-record (tools/golden/record.ps1).",
                copy.key,
                copy.made_at,
                copy.source.display(),
                want.source,
                want.source_sha256,
            ));
        }
        if sha256 != want.sha256 || bytes.len() as u64 != want.len {
            return Err(format!(
                "golden {file} [{}]: the copy made at {} ends as {} bytes with SHA-256 {sha256}; \
                 recorded {} bytes with SHA-256 {}",
                copy.key,
                copy.made_at,
                bytes.len(),
                want.len,
                want.sha256,
            ));
        }
    }
    Ok(())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02X}")).collect()
}

fn record(out: OsString, copy: &Finished<'_>) {
    static APPEND: Mutex<()> = Mutex::new(());
    let source = copy.source.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let line = match fs::read(copy.path) {
        Ok(bytes) => serde_json::json!({
            "key": copy.key,
            "source": source,
            "source_sha256": sha256_hex(copy.original),
            "sha256": sha256_hex(&bytes),
            "len": bytes.len(),
            "made_at": copy.made_at.to_string(),
        }),
        Err(error) => serde_json::json!({ "key": copy.key, "unreadable": error.to_string() }),
    };
    let _one_writer = APPEND.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut file = OpenOptions::new().create(true).append(true).open(&out).expect("open the golden record file");
    writeln!(file, "{line}").expect("append to the golden record file");
}

/// Every golden file's entries, by copy name.
fn goldens() -> &'static BTreeMap<String, Vec<(String, Recorded)>> {
    static GOLDENS: OnceLock<BTreeMap<String, Vec<(String, Recorded)>>> = OnceLock::new();
    GOLDENS.get_or_init(|| load(Path::new(GOLDEN_DIR)))
}

/// Read the `*.json` files of `dir`; an absent directory has none.
pub fn load(dir: &Path) -> BTreeMap<String, Vec<(String, Recorded)>> {
    let mut by_key: BTreeMap<String, Vec<(String, Recorded)>> = BTreeMap::new();
    let mut retirements = Vec::new();
    let Ok(listing) = fs::read_dir(dir) else {
        return by_key;
    };
    let mut files: Vec<_> = listing
        .map(|entry| entry.expect("list the golden directory").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    for path in files {
        let file = path.file_name().expect("a listed file has a name").to_string_lossy().into_owned();
        let text = fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let json: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|error| panic!("{} is not JSON: {error}", path.display()));
        if let Some(rows) = json.get("retired") {
            for row in rows.as_array().expect("retired must be an array") {
                let field = |name: &str| row[name].as_str().expect("retirement needs string fields").to_owned();
                assert!(!field("reason").is_empty(), "retirement needs a reason");
                retirements.push((field("file"), field("key"), field("sha256")));
            }
        }
        let entries = json
            .get("entries")
            .and_then(|entries| entries.as_object())
            .unwrap_or_else(|| panic!("{} has no \"entries\" object", path.display()));
        for (key, entry) in entries {
            let text = |name: &str| {
                entry
                    .get(name)
                    .and_then(|value| value.as_str())
                    .unwrap_or_else(|| panic!("{file} [{key}] has no string \"{name}\""))
                    .to_owned()
            };
            let len = entry
                .get("len")
                .and_then(|value| value.as_u64())
                .unwrap_or_else(|| panic!("{file} [{key}] has no \"len\""));
            let recorded =
                Recorded { source: text("source"), source_sha256: text("source_sha256"), sha256: text("sha256"), len };
            by_key.entry(key.clone()).or_default().push((file.clone(), recorded));
        }
    }
    // Allocation behavior changes retire exact historical recordings, never an
    // entire file or a wildcard. Pin the old hash so later edits fail closed.
    for (file, key, hash) in retirements {
        let entries = by_key.get_mut(&key).expect("retirement names an unknown key");
        let at = entries.iter().position(|(f, _)| f == &file).expect("retirement names an unknown file/key");
        assert_eq!(entries[at].1.sha256, hash, "retirement target hash changed");
        entries.remove(at);
        if entries.is_empty() { by_key.remove(&key); }
    }
    by_key
}
