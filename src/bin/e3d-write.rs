//! `e3d-write` — the batch-instruction CLI over the write path's
//! [`Transaction`] facade (`docs/plans/2026-09-22-write-path-phase-2-plan.md`
//! §2 T1d): one run reads one JSON file of instructions and applies it to one
//! database as **one session**, through exactly the calls a Rust caller makes
//! — `insert` / `set` / `set_hunk` / `rename` / `delete` / `move_to` /
//! `reorder` / `add_reference_member` / `remove_reference_member`, then
//! `commit_reporting`. It is the scriptable entry point until a consumer links
//! the crate, and the fixture the E3D gate (`tools/e3d/write_gate.ps1`) drives.
//!
//! ```text
//! e3d-write apply --db <file> --ops <ops.json>
//!                 [--schema <dir>]                the schema pack (default: the crate's schema/e3d31)
//!                 [--base <sesno>]                open at this session; a merge if the file has moved on
//!                 [--on-clash refuse|writer-wins] what a merge does where both sides changed one thing (default refuse)
//!                 [--dry-run]                     stage and roll back: every check, nothing committed
//!                 [--no-dictionary]               do not hold values to E3D's dictionary limits
//!                 [--user <name>] [--comment <text>]
//!                 [--report <report.json>]
//! ```
//!
//! # The instruction file
//!
//! A JSON array of instructions, or an object `{"user": …, "comment": …, "ops":
//! [...]}` that also carries the session's `USER` and comment (`--user` /
//! `--comment` override; the defaults are `e3dio` and the file's name).
//!
//! | `op` | fields | facade call |
//! |---|---|---|
//! | `insert` | `owner`, `noun`; optional `name`, `id`, `attrs`, `at` | `insert`, then `set` per attribute, `reorder` for `at` |
//! | `set` (or `update`) | `refno`, `attrs` | `set` / `set_hunk` per attribute |
//! | `rename` | `refno`, `name` | `rename` |
//! | `delete` | `refno` | `delete` |
//! | `move` | `refno`, `owner`; optional `at` | `move_to` |
//! | `reorder` | `refno`, `at` | `reorder` |
//! | `reference` | `group`, `member`; optional `at` | `add_reference_member` |
//! | `unreference` | `group`, `member` | `remove_reference_member` |
//!
//! An element — `owner`, `refno`, `group`, `member`, the anchor of an `at` — is
//! written as `=24384/26484` or `24384/26484` (a refno), `/NAME` (looked up in
//! the base session's `NAME` table; `/*` is the `WORL`), or `@id`: an element
//! inserted **earlier in the same file** with that `id`. `at` is `"last"` (the
//! default), `"first"`, `{"after": <element>}` or `{"before": <element>}`.
//! `attrs` maps attribute names to their text — `"POS": "1000 2000 3000"`,
//! `"DESC": "反应堆厂房"`, `"PURP": "PIPE"`, `"CREF": "/PUMP-1"`, `"ROKEYS":
//! "K1|K2"`, `"XLEN": "unset"` ([`e3d_io::write::value`] has the table; the
//! descriptor decides the shape) — or to `{"value": "…", "hunk": true}` to
//! store the value out of the record as a hunk ([`Transaction::set_hunk`]).
//! Attributes of one instruction are set in the order the JSON reader hands
//! them over (by name).
//!
//! Instructions are applied in order and the first one refused stops the run
//! before anything is written — the message names the instruction (`op #n
//! (...)`) and carries the facade's reason — exit code 1. `--dry-run` runs
//! the same batch through [`Transaction::stage`]: every check the commit
//! makes (the shapes, the dictionary, the records laid out, and on a database
//! that moved on since the base the merge's conflict and clash scan) — then
//! rolls the staged pages back, so the file is byte-identical afterwards
//! (page 0 is never touched). The write-mode sentinel lock is taken for that
//! moment, so a database E3D holds open for writing is reported `Locked` by a
//! dry run just as the commit would be.
//!
//! # The report
//!
//! `--report` writes one JSON object: the database, the session opened at and
//! the newest before the run, every instruction with the elements it resolved
//! to (an insert with the refno it was `given` and the `refno` it was written
//! as — they differ when the commit re-minted it), the `commit` (`written` /
//! `previous` / `appended_pages` from the `CommitReport`, plus `merged_onto`,
//! `reminted`, `retried_after_move`), or on a dry run the `staged` session
//! number and page count that were rolled back, or on a refusal the `error`
//! with the instruction's index and the reason. The report is written whether
//! the run succeeded or not, so a script can read it either way.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{json, Map, Value};

use e3d_io::record::template::TemplateProvider;
use e3d_io::session::SessionSelector;
use e3d_io::write::{Dictionary, MemberPosition, OnClash, SessionRequest, Transaction, TransactionError};
use e3d_io::{ReadOnlyEngine, RefNo};

const DEFAULT_SCHEMA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schema/e3d31");
const DEFAULT_USER: &str = "e3dio";
const USAGE: &str = "usage: e3d-write apply --db <file> --ops <ops.json> [--schema <dir>] [--base <sesno>] \
[--on-clash refuse|writer-wins] [--dry-run] [--no-dictionary] [--user <name>] [--comment <text>] [--report <report.json>]";

/// How a run ends short of a commit.
enum Failure {
    /// The command line: exit 2.
    Usage(String),
    /// Setting up — the database, the schema, the instruction file: exit 1.
    Setup(String),
    /// An instruction the facade refused, or the staging / commit: exit 1.
    Refused { op: Option<usize>, message: String },
}

impl Failure {
    fn exit_code(&self) -> ExitCode {
        match self {
            Failure::Usage(_) => ExitCode::from(2),
            Failure::Setup(_) | Failure::Refused { .. } => ExitCode::FAILURE,
        }
    }

    fn message(&self) -> &str {
        match self {
            Failure::Usage(message) | Failure::Setup(message) => message,
            Failure::Refused { message, .. } => message,
        }
    }
}

struct Args {
    db: PathBuf,
    ops: PathBuf,
    schema: PathBuf,
    base: Option<u32>,
    on_clash: OnClash,
    dry_run: bool,
    dictionary: bool,
    user: Option<String>,
    comment: Option<String>,
    report: Option<PathBuf>,
}

fn parse_args() -> Result<Args, Failure> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("apply") => {}
        Some("--help" | "-h") | None => return Err(Failure::Usage(USAGE.to_string())),
        Some(other) => return Err(Failure::Usage(format!("unknown command {other:?}\n{USAGE}"))),
    }
    let mut db = None;
    let mut ops = None;
    let mut schema = None;
    let mut base = None;
    let mut on_clash = OnClash::Refuse;
    let mut dry_run = false;
    let mut dictionary = true;
    let mut user = None;
    let mut comment = None;
    let mut report = None;
    while let Some(flag) = args.next() {
        let mut value = |what: &str| args.next().ok_or_else(|| Failure::Usage(format!("{flag} wants {what}\n{USAGE}")));
        match flag.as_str() {
            "--db" => db = Some(PathBuf::from(value("a database file")?)),
            "--ops" => ops = Some(PathBuf::from(value("an instruction file")?)),
            "--schema" => schema = Some(PathBuf::from(value("a schema directory")?)),
            "--base" => {
                let text = value("a session number")?;
                base = Some(text.parse::<u32>().map_err(|_| Failure::Usage(format!("--base {text}: not a session number\n{USAGE}")))?);
            }
            "--on-clash" => {
                on_clash = match value("refuse or writer-wins")?.as_str() {
                    "refuse" => OnClash::Refuse,
                    "writer-wins" => OnClash::WriterWins,
                    other => return Err(Failure::Usage(format!("--on-clash {other}: refuse or writer-wins\n{USAGE}"))),
                }
            }
            "--dry-run" => dry_run = true,
            "--no-dictionary" => dictionary = false,
            "--user" => user = Some(value("a user name")?),
            "--comment" => comment = Some(value("a session comment")?),
            "--report" => report = Some(PathBuf::from(value("a file to write")?)),
            other => return Err(Failure::Usage(format!("unknown flag {other}\n{USAGE}"))),
        }
    }
    Ok(Args {
        db: db.ok_or_else(|| Failure::Usage(format!("--db is required\n{USAGE}")))?,
        ops: ops.ok_or_else(|| Failure::Usage(format!("--ops is required\n{USAGE}")))?,
        schema: schema.unwrap_or_else(|| PathBuf::from(DEFAULT_SCHEMA)),
        base,
        on_clash,
        dry_run,
        dictionary,
        user,
        comment,
        report,
    })
}

// --- the instruction file ---

/// How an instruction names an element.
#[derive(Debug, Clone)]
enum Element {
    RefNo(RefNo),
    Name(String),
    Id(String),
}

impl Element {
    fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if let Some(id) = text.strip_prefix('@') {
            if id.is_empty() {
                return Err("an @id needs the id".to_string());
            }
            return Ok(Element::Id(id.to_string()));
        }
        if text.starts_with('/') {
            return Ok(Element::Name(text.to_string()));
        }
        let body = text.strip_prefix('=').unwrap_or(text);
        if let Some((word0, word1)) = body.split_once('/') {
            if let (Ok(word0), Ok(word1)) = (word0.trim().parse::<u32>(), word1.trim().parse::<u32>()) {
                return Ok(Element::RefNo(RefNo::new(word0, word1)));
            }
        }
        Err(format!("{text:?} is not an element: write =word0/word1, /NAME (/* for the WORL) or @id"))
    }
}

impl std::fmt::Display for Element {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Element::RefNo(refno) => write!(f, "={refno}"),
            Element::Name(name) => f.write_str(name),
            Element::Id(id) => write!(f, "@{id}"),
        }
    }
}

#[derive(Debug, Clone)]
enum Position {
    Last,
    First,
    After(Element),
    Before(Element),
}

/// One attribute's text, stored inline or as a hunk.
#[derive(Debug, Clone)]
struct Attr {
    name: String,
    text: String,
    hunk: bool,
}

#[derive(Debug, Clone)]
enum Op {
    Insert { id: Option<String>, owner: Element, noun: String, name: Option<String>, attrs: Vec<Attr>, at: Option<Position> },
    Set { refno: Element, attrs: Vec<Attr> },
    Rename { refno: Element, name: String },
    Delete { refno: Element },
    Move { refno: Element, owner: Element, at: Position },
    Reorder { refno: Element, at: Position },
    Reference { group: Element, member: Element, at: Position },
    Unreference { group: Element, member: Element },
}

impl Op {
    fn kind(&self) -> &'static str {
        match self {
            Op::Insert { .. } => "insert",
            Op::Set { .. } => "set",
            Op::Rename { .. } => "rename",
            Op::Delete { .. } => "delete",
            Op::Move { .. } => "move",
            Op::Reorder { .. } => "reorder",
            Op::Reference { .. } => "reference",
            Op::Unreference { .. } => "unreference",
        }
    }
}

/// The file as read: the session's words and the instructions.
struct OpsFile {
    user: Option<String>,
    comment: Option<String>,
    ops: Vec<Op>,
}

fn read_ops(path: &Path) -> Result<OpsFile, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    // A byte-order mark, which Windows editors like to put in front of UTF-8.
    let text = text.trim_start_matches('\u{feff}');
    let value: Value = serde_json::from_str(text).map_err(|e| format!("{}: not JSON: {e}", path.display()))?;
    let (user, comment, list) = match value {
        Value::Array(list) => (None, None, list),
        Value::Object(mut object) => {
            let user = take_string(&mut object, "user")?;
            let comment = take_string(&mut object, "comment")?;
            let ops = match object.remove("ops") {
                Some(Value::Array(list)) => list,
                Some(_) => return Err("\"ops\" must be an array of instructions".to_string()),
                None => return Err("an instruction file is an array, or an object with \"ops\"".to_string()),
            };
            if let Some(stray) = object.keys().next() {
                return Err(format!("unknown field {stray:?} beside \"user\" / \"comment\" / \"ops\""));
            }
            (user, comment, ops)
        }
        _ => return Err("an instruction file is an array, or an object with \"ops\"".to_string()),
    };
    let mut ops = Vec::with_capacity(list.len());
    let mut ids: HashMap<String, usize> = HashMap::new();
    for (index, item) in list.into_iter().enumerate() {
        let op = parse_op(item).map_err(|reason| format!("op #{index}: {reason}"))?;
        if let Op::Insert { id: Some(id), .. } = &op {
            if let Some(earlier) = ids.insert(id.clone(), index) {
                return Err(format!("op #{index}: id {id:?} is already op #{earlier}'s"));
            }
        }
        ops.push(op);
    }
    Ok(OpsFile { user, comment, ops })
}

fn take_string(object: &mut Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match object.remove(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(other) => Err(format!("\"{key}\" must be a string, not {other}")),
    }
}

fn take_element(object: &mut Map<String, Value>, key: &str) -> Result<Element, String> {
    let text = take_string(object, key)?.ok_or_else(|| format!("\"{key}\" is required"))?;
    Element::parse(&text).map_err(|reason| format!("\"{key}\": {reason}"))
}

fn take_position(object: &mut Map<String, Value>, key: &str) -> Result<Option<Position>, String> {
    let position = match object.remove(key) {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(word)) => match word.to_ascii_lowercase().as_str() {
            "last" => Position::Last,
            "first" => Position::First,
            other => return Err(format!("\"{key}\": {other:?} is not a position: \"last\", \"first\", {{\"after\": e}} or {{\"before\": e}}")),
        },
        Some(Value::Object(mut anchor)) => {
            let after = take_string(&mut anchor, "after")?;
            let before = take_string(&mut anchor, "before")?;
            if let Some(stray) = anchor.keys().next() {
                return Err(format!("\"{key}\": unknown field {stray:?}; a position is {{\"after\": e}} or {{\"before\": e}}"));
            }
            match (after, before) {
                (Some(after), None) => Position::After(Element::parse(&after).map_err(|r| format!("\"{key}\".after: {r}"))?),
                (None, Some(before)) => Position::Before(Element::parse(&before).map_err(|r| format!("\"{key}\".before: {r}"))?),
                _ => return Err(format!("\"{key}\": a position is either {{\"after\": e}} or {{\"before\": e}}")),
            }
        }
        Some(other) => return Err(format!("\"{key}\": {other} is not a position")),
    };
    Ok(Some(position))
}

fn take_attrs(object: &mut Map<String, Value>) -> Result<Vec<Attr>, String> {
    let Some(attrs) = object.remove("attrs") else {
        return Ok(Vec::new());
    };
    let Value::Object(attrs) = attrs else {
        return Err("\"attrs\" must be an object of attribute name to text".to_string());
    };
    let mut out = Vec::with_capacity(attrs.len());
    for (name, value) in attrs {
        let attr = match value {
            Value::String(text) => Attr { name: name.clone(), text, hunk: false },
            Value::Object(mut spec) => {
                let text = take_string(&mut spec, "value")?.ok_or_else(|| format!("attrs.{name}: {{\"value\": …}} is required"))?;
                let hunk = match spec.remove("hunk") {
                    None | Some(Value::Null) => false,
                    Some(Value::Bool(hunk)) => hunk,
                    Some(other) => return Err(format!("attrs.{name}.hunk must be true or false, not {other}")),
                };
                if let Some(stray) = spec.keys().next() {
                    return Err(format!("attrs.{name}: unknown field {stray:?} beside \"value\" / \"hunk\""));
                }
                Attr { name: name.clone(), text, hunk }
            }
            other => return Err(format!("attrs.{name}: a text, or {{\"value\": …, \"hunk\": true}}, not {other}")),
        };
        out.push(attr);
    }
    Ok(out)
}

fn parse_op(item: Value) -> Result<Op, String> {
    let Value::Object(mut object) = item else {
        return Err(format!("an instruction is an object, not {item}"));
    };
    let kind = take_string(&mut object, "op")?.ok_or_else(|| "\"op\" is required".to_string())?;
    let op = match kind.to_ascii_lowercase().as_str() {
        "insert" => Op::Insert {
            id: take_string(&mut object, "id")?,
            owner: take_element(&mut object, "owner")?,
            noun: take_string(&mut object, "noun")?.ok_or("\"noun\" is required")?.trim().to_ascii_uppercase(),
            name: take_string(&mut object, "name")?,
            attrs: take_attrs(&mut object)?,
            at: take_position(&mut object, "at")?,
        },
        "set" | "update" => Op::Set { refno: take_element(&mut object, "refno")?, attrs: take_attrs(&mut object)? },
        "rename" => Op::Rename {
            refno: take_element(&mut object, "refno")?,
            name: take_string(&mut object, "name")?.ok_or("\"name\" is required")?,
        },
        "delete" => Op::Delete { refno: take_element(&mut object, "refno")? },
        "move" => Op::Move {
            refno: take_element(&mut object, "refno")?,
            owner: take_element(&mut object, "owner")?,
            at: take_position(&mut object, "at")?.unwrap_or(Position::Last),
        },
        "reorder" => Op::Reorder {
            refno: take_element(&mut object, "refno")?,
            at: take_position(&mut object, "at")?.ok_or("\"at\" is required for a reorder")?,
        },
        "reference" => Op::Reference {
            group: take_element(&mut object, "group")?,
            member: take_element(&mut object, "member")?,
            at: take_position(&mut object, "at")?.unwrap_or(Position::Last),
        },
        "unreference" => Op::Unreference { group: take_element(&mut object, "group")?, member: take_element(&mut object, "member")? },
        other => {
            return Err(format!(
                "\"op\": {other:?} is not an instruction: insert, set, rename, delete, move, reorder, reference, unreference"
            ))
        }
    };
    if let Some(stray) = object.keys().next() {
        return Err(format!("unknown field {stray:?} on a {kind}"));
    }
    Ok(op)
}

// --- resolving elements ---

/// Names to refnos: the base session's `NAME` table and `WORL`, and the ids of
/// the inserts made so far.
struct Resolver {
    engine: ReadOnlyEngine,
    ids: HashMap<String, RefNo>,
}

impl Resolver {
    fn resolve(&mut self, element: &Element) -> Result<RefNo, String> {
        match element {
            Element::RefNo(refno) => {
                // Said by number: it has to be in the base (an element this file
                // inserts is named by its @id, its number being the commit's to
                // re-mint), so a slip is caught at the instruction, not at staging.
                let indexed = self.engine.find_element(*refno).map_err(|e| format!("={refno}: {e}"))?.is_some();
                if !indexed && !self.ids.values().any(|id| id == refno) {
                    return Err(format!(
                        "={refno} is not indexed in the base session (an element inserted in this file is named by its @id)"
                    ));
                }
                Ok(*refno)
            }
            Element::Id(id) => self
                .ids
                .get(id)
                .copied()
                .ok_or_else(|| format!("@{id}: no earlier insert in this file has id {id:?}")),
            Element::Name(name) if name == "/*" => Ok(self
                .engine
                .world()
                .map_err(|e| format!("/*: {e}"))?
                .ok_or("/*: the database indexes no WORL")?
                .header()
                .refno),
            Element::Name(name) => self
                .engine
                .find_by_name(name)
                .map_err(|e| format!("{name}: {e}"))?
                .ok_or_else(|| format!("{name}: no element of the base session is so named (an element inserted in this file is named by its @id)")),
        }
    }

    fn position(&mut self, at: &Position) -> Result<MemberPosition, String> {
        Ok(match at {
            Position::Last => MemberPosition::Last,
            Position::First => MemberPosition::First,
            Position::After(anchor) => MemberPosition::After(self.resolve(anchor)?),
            Position::Before(anchor) => MemberPosition::Before(self.resolve(anchor)?),
        })
    }
}

fn refno_json(refno: RefNo) -> Value {
    Value::String(refno.to_string())
}

fn position_json(at: &MemberPosition) -> Value {
    match at {
        MemberPosition::Last => json!("last"),
        MemberPosition::First => json!("first"),
        MemberPosition::After(anchor) => json!({ "after": anchor.to_string() }),
        MemberPosition::Before(anchor) => json!({ "before": anchor.to_string() }),
    }
}

// --- the run ---

/// What the run says about itself, as the report and the stdout summary
/// accumulate it.
struct Run {
    report: Map<String, Value>,
    ops: Vec<Value>,
    lines: Vec<String>,
}

impl Run {
    fn say(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
    }
}

fn run(args: &Args, out: &mut Run) -> Result<(), Failure> {
    let setup = |message: String| Failure::Setup(message);
    let db = args.db.to_string_lossy().into_owned();
    out.report.insert("database".into(), json!(db));
    out.report.insert("ops_file".into(), json!(args.ops.to_string_lossy()));
    out.report.insert("dry_run".into(), json!(args.dry_run));

    let file = read_ops(&args.ops).map_err(setup)?;
    let user = args.user.clone().or(file.user).unwrap_or_else(|| DEFAULT_USER.to_string());
    let comment = args.comment.clone().or(file.comment).unwrap_or_else(|| {
        args.ops.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "e3d-write apply".to_string())
    });
    out.report.insert("user".into(), json!(user));
    out.report.insert("comment".into(), json!(comment));

    let desvir = args.schema.join("binaries").join("desvir.dat");
    let dictionary_path = args.schema.join("catalog").join("noun_attr_fields.json");
    let mut provider = TemplateProvider::open(&desvir, None).map_err(|e| setup(format!("{}: {e}", desvir.display())))?;
    let dictionary = if args.dictionary && dictionary_path.is_file() {
        Some(Dictionary::read(&dictionary_path).map_err(|e| setup(format!("{}: {e}", dictionary_path.display())))?)
    } else {
        None
    };

    let latest_before = ReadOnlyEngine::open(&args.db).map_err(|e| setup(format!("{db}: {e}")))?.session().sesno;
    let selector = match args.base {
        Some(sesno) => SessionSelector::Exact(sesno),
        None => SessionSelector::Latest,
    };
    let opened_at = args.base.map_or("the newest session".to_string(), |sesno| format!("session {sesno}"));
    let mut tx = Transaction::open_at(&args.db, selector, &mut provider)
        .map_err(|e| setup(format!("{db}: opening a transaction at {opened_at}: {e}")))?
        .with_on_clash(args.on_clash);
    if let Some(dictionary) = &dictionary {
        tx = tx.with_dictionary(dictionary);
    }
    let base = tx.base_sesno();
    let mut resolver = Resolver {
        engine: ReadOnlyEngine::open_at(&args.db, SessionSelector::Exact(base)).map_err(|e| setup(format!("{db}: {e}")))?,
        ids: HashMap::new(),
    };
    out.report.insert("base_sesno".into(), json!(base));
    out.report.insert("latest_before".into(), json!(latest_before));
    out.report.insert("dictionary".into(), dictionary.as_ref().map_or(Value::Null, |d| json!(d.len())));
    out.report.insert("on_clash".into(), json!(match args.on_clash { OnClash::Refuse => "refuse", OnClash::WriterWins => "writer-wins" }));

    out.say(format!("e3d-write apply: {} instruction(s) from {} onto {db}", file.ops.len(), args.ops.display()));
    out.say(format!(
        "  base session     : {base}{}",
        if latest_before == base { String::new() } else { format!(" (the newest is {latest_before}: the commit is a merge)") }
    ));
    out.say(match &dictionary {
        Some(dictionary) => format!("  dictionary       : {} attributes from {}", dictionary.len(), dictionary_path.display()),
        None if args.dictionary => format!("  dictionary       : none ({} is not there)", dictionary_path.display()),
        None => "  dictionary       : none (--no-dictionary)".to_string(),
    });

    // Every instruction, in order; the first refusal ends the run.
    let mut inserts: Vec<(usize, RefNo)> = Vec::new();
    for (index, op) in file.ops.iter().enumerate() {
        let mut entry = Map::new();
        entry.insert("index".into(), json!(index));
        entry.insert("op".into(), json!(op.kind()));
        let refused = |what: String, reason: String| Failure::Refused { op: Some(index), message: format!("op #{index} ({what}): {reason}") };
        match op {
            Op::Insert { id, owner, noun, name, attrs, at } => {
                let what = format!("insert {noun} under {owner}");
                let owner_refno = resolver.resolve(owner).map_err(|r| refused(what.clone(), r))?;
                let noun_hash = e3d_attlib::db1_hash(noun);
                let refno = tx
                    .insert(owner_refno, noun_hash, name.as_deref(), |_, _| Ok(()))
                    .map_err(|e| refused(what.clone(), e.to_string()))?;
                for attr in attrs {
                    set_attr(&mut tx, refno, attr).map_err(|r| refused(format!("{what}: {}", attr.name), r))?;
                }
                if let Some(at) = at {
                    let position = resolver.position(at).map_err(|r| refused(what.clone(), r))?;
                    tx.reorder(refno, position).map_err(|e| refused(format!("{what} at {}", position_json(&position)), e.to_string()))?;
                    entry.insert("at".into(), position_json(&position));
                }
                if let Some(id) = id {
                    resolver.ids.insert(id.clone(), refno);
                    entry.insert("id".into(), json!(id));
                }
                entry.insert("noun".into(), json!(noun));
                entry.insert("owner".into(), refno_json(owner_refno));
                if let Some(name) = name {
                    entry.insert("name".into(), json!(name));
                }
                entry.insert("attrs".into(), json!(attrs.iter().map(|a| a.name.as_str()).collect::<Vec<_>>()));
                entry.insert("given".into(), refno_json(refno));
                entry.insert("refno".into(), refno_json(refno));
                inserts.push((index, refno));
                out.say(format!(
                    "  #{index} insert {noun} under {owner_refno} -> {refno}{}{}",
                    name.as_deref().map_or(String::new(), |n| format!(" {n}")),
                    id.as_deref().map_or(String::new(), |i| format!(" [@{i}]"))
                ));
            }
            Op::Set { refno, attrs } => {
                let what = format!("set {refno}");
                let target = resolver.resolve(refno).map_err(|r| refused(what.clone(), r))?;
                for attr in attrs {
                    set_attr(&mut tx, target, attr).map_err(|r| refused(format!("set {target} {}", attr.name), r))?;
                }
                entry.insert("refno".into(), refno_json(target));
                entry.insert("attrs".into(), json!(attrs.iter().map(|a| a.name.as_str()).collect::<Vec<_>>()));
                out.say(format!("  #{index} set {target} {}", attrs.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")));
            }
            Op::Rename { refno, name } => {
                let what = format!("rename {refno} {name}");
                let target = resolver.resolve(refno).map_err(|r| refused(what.clone(), r))?;
                tx.rename(target, name).map_err(|e| refused(what, e.to_string()))?;
                entry.insert("refno".into(), refno_json(target));
                entry.insert("name".into(), json!(name));
                out.say(format!("  #{index} rename {target} {name}"));
            }
            Op::Delete { refno } => {
                let what = format!("delete {refno}");
                let target = resolver.resolve(refno).map_err(|r| refused(what.clone(), r))?;
                tx.delete(target).map_err(|e| refused(what, e.to_string()))?;
                entry.insert("refno".into(), refno_json(target));
                out.say(format!("  #{index} delete {target}"));
            }
            Op::Move { refno, owner, at } => {
                let what = format!("move {refno} under {owner}");
                let target = resolver.resolve(refno).map_err(|r| refused(what.clone(), r))?;
                let owner_refno = resolver.resolve(owner).map_err(|r| refused(what.clone(), r))?;
                let position = resolver.position(at).map_err(|r| refused(what.clone(), r))?;
                tx.move_to(target, owner_refno, position).map_err(|e| refused(what, e.to_string()))?;
                entry.insert("refno".into(), refno_json(target));
                entry.insert("owner".into(), refno_json(owner_refno));
                entry.insert("at".into(), position_json(&position));
                out.say(format!("  #{index} move {target} under {owner_refno} at {}", position_json(&position)));
            }
            Op::Reorder { refno, at } => {
                let what = format!("reorder {refno}");
                let target = resolver.resolve(refno).map_err(|r| refused(what.clone(), r))?;
                let position = resolver.position(at).map_err(|r| refused(what.clone(), r))?;
                tx.reorder(target, position).map_err(|e| refused(what, e.to_string()))?;
                entry.insert("refno".into(), refno_json(target));
                entry.insert("at".into(), position_json(&position));
                out.say(format!("  #{index} reorder {target} at {}", position_json(&position)));
            }
            Op::Reference { group, member, at } => {
                let what = format!("reference {member} in {group}");
                let group_refno = resolver.resolve(group).map_err(|r| refused(what.clone(), r))?;
                let member_refno = resolver.resolve(member).map_err(|r| refused(what.clone(), r))?;
                let position = resolver.position(at).map_err(|r| refused(what.clone(), r))?;
                tx.add_reference_member(group_refno, member_refno, position).map_err(|e| refused(what, e.to_string()))?;
                entry.insert("group".into(), refno_json(group_refno));
                entry.insert("member".into(), refno_json(member_refno));
                entry.insert("at".into(), position_json(&position));
                out.say(format!("  #{index} reference {member_refno} in {group_refno} at {}", position_json(&position)));
            }
            Op::Unreference { group, member } => {
                let what = format!("unreference {member} from {group}");
                let group_refno = resolver.resolve(group).map_err(|r| refused(what.clone(), r))?;
                let member_refno = resolver.resolve(member).map_err(|r| refused(what.clone(), r))?;
                tx.remove_reference_member(group_refno, member_refno).map_err(|e| refused(what, e.to_string()))?;
                entry.insert("group".into(), refno_json(group_refno));
                entry.insert("member".into(), refno_json(member_refno));
                out.say(format!("  #{index} unreference {member_refno} from {group_refno}"));
            }
        }
        out.ops.push(Value::Object(entry));
    }

    // On a database that moved on, say which given refnos the newest session
    // already holds — the commit re-mints them (a dry run cannot report the
    // new numbers; the commit does).
    if latest_before != base && !inserts.is_empty() {
        let mut latest = ReadOnlyEngine::open(&args.db).map_err(|e| setup(format!("{db}: {e}")))?;
        for (index, given) in &inserts {
            let taken = latest.find_element(*given).map_err(|e| setup(format!("{db}: {e}")))?.is_some();
            if let Some(Value::Object(entry)) = out.ops.get_mut(*index) {
                entry.insert("taken_in_latest".into(), json!(taken));
            }
            if taken {
                out.say(format!("  #{index}: {given} is taken in session {latest_before}; the commit re-mints it"));
            }
        }
    }

    let request = SessionRequest { user: &user, comment: &comment, clock: None };
    if args.dry_run {
        let staged = tx.stage(&request).map_err(|e| Failure::Refused { op: None, message: format!("staging: {e}") })?;
        let (previous, summary) = (staged.previous(), staged.staged());
        let pages = summary.end.page.saturating_sub(previous.end.page);
        staged.rollback().map_err(|e| Failure::Refused { op: None, message: format!("rolling the staged pages back: {e}") })?;
        out.report.insert(
            "staged".into(),
            json!({ "sesno": summary.sesno, "previous_sesno": previous.sesno, "pages": pages, "rolled_back": true }),
        );
        out.report.insert("commit".into(), Value::Null);
        out.say(format!(
            "  dry run          : session {} would follow {} on {pages} page(s); staged and rolled back, nothing committed",
            summary.sesno, previous.sesno
        ));
        return Ok(());
    }

    let commit = tx.commit_reporting(&request).map_err(|e| Failure::Refused { op: None, message: format!("commit: {e}") })?;
    for (given, written) in &commit.reminted {
        if let Some((index, _)) = inserts.iter().find(|(_, refno)| refno == given) {
            if let Some(Value::Object(entry)) = out.ops.get_mut(*index) {
                entry.insert("refno".into(), refno_json(*written));
            }
        }
    }
    out.report.insert("staged".into(), Value::Null);
    out.report.insert(
        "commit".into(),
        json!({
            "written": { "sesno": commit.report.written.sesno, "page": commit.report.written.page.page, "end": commit.report.written.end.page },
            "previous": { "sesno": commit.report.previous.sesno, "page": commit.report.previous.page.page, "end": commit.report.previous.end.page },
            "appended_pages": commit.report.appended_pages,
            "base_sesno": commit.base_sesno,
            "merged_onto": commit.merged_onto,
            "reminted": commit.reminted.iter().map(|(given, written)| json!({ "given": given.to_string(), "written": written.to_string() })).collect::<Vec<_>>(),
            "retried_after_move": commit.retried_after_move,
        }),
    );
    out.say(format!("wrote session {} to {db}", commit.report.written.sesno));
    out.say(format!("  previous session : {}", commit.report.previous.sesno));
    if let Some(onto) = commit.merged_onto {
        out.say(format!("  merged onto      : {onto} (base {})", commit.base_sesno));
    }
    for (given, written) in &commit.reminted {
        out.say(format!("  re-minted        : {given} -> {written}"));
    }
    if commit.retried_after_move {
        out.say("  retried on Moved : yes");
    }
    out.say(format!("  appended pages   : {}", commit.report.appended_pages.len()));
    Ok(())
}

/// One attribute of an instruction onto the transaction: `set`, or `set_hunk`.
fn set_attr(tx: &mut Transaction<'_>, refno: RefNo, attr: &Attr) -> Result<(), String> {
    let result = if attr.hunk { tx.set_hunk(refno, &attr.name, &attr.text) } else { tx.set(refno, &attr.name, &attr.text) };
    result.map_err(|e| match e {
        // These name the attribute and quote the text themselves.
        TransactionError::Value { .. } | TransactionError::Dictionary(_) | TransactionError::NotAHunk { .. } => e.to_string(),
        other => {
            let mut shown = attr.text.chars().take(40).collect::<String>();
            if shown.len() < attr.text.len() {
                shown.push('…');
            }
            format!("{} = {shown:?}{}: {other}", attr.name, if attr.hunk { " (as a hunk)" } else { "" })
        }
    })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(failure) => {
            eprintln!("e3d-write: {}", failure.message());
            return failure.exit_code();
        }
    };
    let mut out = Run { report: Map::new(), ops: Vec::new(), lines: Vec::new() };
    let outcome = run(&args, &mut out);
    let mut report = out.report;
    report.insert("ops".into(), Value::Array(out.ops));
    let code = match &outcome {
        Ok(()) => {
            report.insert("error".into(), Value::Null);
            ExitCode::SUCCESS
        }
        Err(failure) => {
            let op = match failure {
                Failure::Refused { op, .. } => *op,
                _ => None,
            };
            report.insert("error".into(), json!({ "op": op, "message": failure.message() }));
            failure.exit_code()
        }
    };
    let mut text = String::new();
    for line in &out.lines {
        let _ = writeln!(text, "{line}");
    }
    print!("{text}");
    if let Some(path) = &args.report {
        match serde_json::to_string_pretty(&Value::Object(report)).map_err(|e| e.to_string()).and_then(|json| fs::write(path, json + "\n").map_err(|e| e.to_string())) {
            Ok(()) => println!("  report           : {}", path.display()),
            Err(error) => {
                eprintln!("e3d-write: writing the report {}: {error}", path.display());
                if outcome.is_ok() {
                    return ExitCode::FAILURE;
                }
            }
        }
    }
    if let Err(failure) = &outcome {
        eprintln!("e3d-write: {}", failure.message());
    }
    code
}
