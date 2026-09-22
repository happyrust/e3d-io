//! What did reading a record from its own addresses change about the values?
//!
//! `522a252` replaced a reader that searched — for the end of a record, and
//! for its blocks behind the implicit region — with one that reads the four
//! address words the record carries. This probe measures what that did to the
//! attributes and members the two readers hand back, over the same corpus and
//! the same record positions.
//!
//! **The same source file compiles at `522a252^` and at `HEAD`.** Everything
//! it touches — `DbView`, `IndexCursor`, `read_element_record`,
//! `ParsedElement`, `ExplicitAttribute` — has the same shape at both commits,
//! and `src/index` is byte-identical between them, so the walk yields the same
//! keys in the same order on both sides. The only variable left is L3.
//!
//! ```powershell
//! # before, in a worktree at 522a252^
//! cargo run --release --example record_l3_value_delta_probe -- dump before.tsv
//! # after, on main
//! cargo run --release --example record_l3_value_delta_probe -- dump after.tsv
//! cargo run --release --example record_l3_value_delta_probe -- diff before.tsv after.tsv out
//! ```
//!
//! `dump` writes one line per indexed key. `diff` reads two of those in
//! lockstep, refuses to go on if the keys do not line up, and reports what
//! changed. The structural facts each line carries — where the header says the
//! blocks are, whether they tile the space behind the implicit region, whether
//! any of them state a continuation — are read from the pages, not from the
//! reader, so they are the same on both sides and are what a change is
//! attributed to.

use std::collections::BTreeMap;
use std::fmt::Write as FmtWrite;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use e3d_io::index::{IndexCursor, IndexEntry};
use e3d_io::noun_catalog::NounCatalog;
use e3d_io::page::PageId;
use e3d_io::record::element::ParsedElement;
use e3d_io::record::explicit::ExplicitValue;
use e3d_io::record::read_element_record;
use e3d_io::refno::RefNo;
use e3d_io::session::DbView;

const AMS_DIR: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";
const CATALOG: &str = "schema/e3d31/catalog";

/// Elements of an array rendered in full; the rest is covered by the digest.
///
/// A whole-corpus dump abbreviates, because ELDATA runs to 4096 integers and
/// there are 789 831 keys. `redump` re-reads a named handful with this raised,
/// which is what makes the value-level check in `splice` possible.
static ARRAY_SHOWN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(24);

fn array_shown() -> usize {
    ARRAY_SHOWN.load(std::sync::atomic::Ordering::Relaxed)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("dump") => dump(&args[1..]),
        Some("redump") => redump(&args[1..]),
        Some("diff") => diff(&args[1..]),
        Some("splice") => splice(&args[1..]),
        Some("case") => case(&args[1..]),
        _ => {
            eprintln!(
                "usage:\n  \
                 record_l3_value_delta_probe dump <out.tsv> [corpus-dir] [max-files]\n  \
                 record_l3_value_delta_probe redump <keys.tsv> <out.tsv> [corpus-dir]\n  \
                 record_l3_value_delta_probe diff <before.tsv> <after.tsv> <out-dir> \
                 [catalog-dir]\n  \
                 record_l3_value_delta_probe splice <before-full.tsv> <after-full.tsv>\n  \
                 record_l3_value_delta_probe case <db-path> <page> <offset-words> [catalog-dir]"
            );
            std::process::exit(2);
        }
    }
}

// ---------------------------------------------------------------- dump side

fn dump(args: &[String]) {
    let out_path = args.first().expect("an output path");
    let dir = args.get(1).map(String::as_str).unwrap_or(AMS_DIR);
    let max_files: usize = args
        .get(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(usize::MAX);

    let files = corpus(dir, max_files);
    eprintln!("{} databases under {dir}", files.len());

    let mut out = BufWriter::with_capacity(1 << 22, File::create(out_path).expect("output opens"));
    let mut keys = 0usize;
    let mut tally = Tally::default();
    for (index, path) in files.iter().enumerate() {
        keys += dump_db(path, &mut out, &mut tally);
        if index % 25 == 24 {
            eprintln!("  {}/{} databases, {keys} keys", index + 1, files.len());
        }
    }
    out.flush().expect("output flushes");
    eprintln!("{} databases, {keys} keys -> {out_path}", files.len());

    // Which reader produced this file, checked rather than assumed.
    //
    // Two builds of one package in one target directory hand back whichever
    // binary was uplifted last, and a dump from the wrong one compares equal to
    // itself and looks like "the rewrite changed nothing". The reader under
    // test is the one visible in the numbers: reading from the stated addresses
    // hands over exactly the record, so `record_end == raw_bytes.len()` on
    // every key; searching for `0x00000007` overshoots on 96% of the corpus.
    eprintln!(
        "exact records (record_end == bytes handed over): {} of {} ({:.1}%)",
        tally.exact,
        tally.parsed,
        100.0 * tally.exact as f64 / tally.parsed.max(1) as f64
    );
    eprintln!(
        "read failures {}, parse failures {}",
        tally.read_failed, tally.parse_failed
    );
    eprintln!(
        "reader signature: {}",
        if tally.exact == tally.parsed {
            "reads from the stated addresses (522a252 or later)"
        } else {
            "searches for the record's end (before 522a252)"
        }
    );
}

#[derive(Default)]
struct Tally {
    parsed: usize,
    exact: usize,
    read_failed: usize,
    parse_failed: usize,
}

/// The 429 files the page layout and the index were adjudicated on.
fn corpus(dir: &str, max_files: usize) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("sample directory reads")
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let metadata = entry.metadata().ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let sampled = name.starts_with("ams")
                && name.len() > 5
                && name.as_bytes()[name.len() - 5] == b'_'
                && name[name.len() - 4..].bytes().all(|b| b.is_ascii_digit())
                && name[3..name.len() - 5].bytes().all(|b| b.is_ascii_digit());
            let sized = metadata.len() < 25 * 1024 * 1024 && metadata.len() > 20 * 1024;
            (metadata.is_file() && sampled && sized).then(|| entry.path())
        })
        .collect();
    files.sort();
    files.truncate(max_files);
    files
}

fn dump_db(path: &Path, out: &mut impl Write, tally: &mut Tally) -> usize {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();

    let mut view = match DbView::open(path) {
        Ok(view) => view,
        Err(error) => {
            writeln!(out, "#openfail\t{name}\t{}", one_line(&error.to_string())).unwrap();
            return 0;
        }
    };
    let sesno = view.selected_session();
    let ext = view.index_root().ext;
    let entries: Vec<IndexEntry> = match IndexCursor::new(&mut view)
        .walk()
        .map(|record| record.map(|record| record.entry()))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(entries) => entries,
        Err(error) => {
            writeln!(out, "#walkfail\t{name}\t{}", one_line(&error.to_string())).unwrap();
            return 0;
        }
    };

    let page_size = view.pages_mut().page_size() as usize;
    writeln!(
        out,
        "#db\t{name}\tsesno={sesno}\text={ext}\tkeys={}\tpage_size={page_size}",
        entries.len()
    )
    .unwrap();

    for entry in &entries {
        write_row(&name, entry, &mut view, ext, page_size, out, tally);
    }
    entries.len()
}

/// One key, one line — the only place a record becomes text, so the two builds
/// cannot differ in how they say it, only in what they read.
fn write_row(
    name: &str,
    entry: &IndexEntry,
    view: &mut DbView,
    ext: u16,
    page_size: usize,
    out: &mut impl Write,
    tally: &mut Tally,
) {
    let facts = facts(view.pages_mut(), ext, entry, page_size);
    let (status, detail, members, attrs) = match read_element_record(view.pages_mut(), ext, entry) {
        Ok(record) => match ParsedElement::parse(&record.raw_bytes) {
            Ok(parsed) => {
                tally.parsed += 1;
                tally.exact += usize::from(parsed.record_end == record.raw_bytes.len());
                (
                    "ok",
                    format!(
                        "{}|{:08X}|{}|{}|{}|{}",
                        parsed.refno,
                        parsed.noun_hash,
                        parsed.impl_len_words,
                        parsed.prefix_offset,
                        parsed.record_end,
                        record.raw_bytes.len()
                    ),
                    render_members(&parsed.members),
                    render_attributes(&parsed),
                )
            }
            Err(error) => {
                tally.parse_failed += 1;
                ("parsefail", one_line(&error), String::new(), String::new())
            }
        },
        Err(error) => {
            tally.read_failed += 1;
            (
                "readfail",
                one_line(&error.to_string()),
                String::new(),
                String::new(),
            )
        }
    };
    writeln!(
        out,
        "{name}\t{}\t{}\t{}\t{status}\t{detail}\t{members}\t{attrs}\t{facts}",
        entry.page_no, entry.offset_words, entry.refno
    )
    .unwrap();
}

/// Re-read a named set of keys with arrays rendered in full.
///
/// Same line format as [`dump`], so [`Row`] parses both. Takes
/// `db<TAB>page<TAB>offset_words` lines, which is columns 1..3 of
/// `changed.tsv`.
fn redump(args: &[String]) {
    let keys_path = args.first().expect("a key list");
    let out_path = args.get(1).expect("an output path");
    let dir = args.get(2).map(String::as_str).unwrap_or(AMS_DIR);
    ARRAY_SHOWN.store(usize::MAX, std::sync::atomic::Ordering::Relaxed);

    let mut wanted: BTreeMap<String, Vec<(u32, u32)>> = BTreeMap::new();
    for line in BufReader::new(File::open(keys_path).expect("the key list opens")).lines() {
        let line = line.expect("the key list reads");
        let mut columns = line.split('\t');
        let (Some(db), Some(page), Some(offset)) = (columns.next(), columns.next(), columns.next())
        else {
            continue;
        };
        let (Ok(page), Ok(offset)) = (page.parse(), offset.parse()) else {
            continue;
        };
        wanted
            .entry(db.to_string())
            .or_default()
            .push((page, offset));
    }

    let mut out = BufWriter::with_capacity(1 << 20, File::create(out_path).expect("output opens"));
    let mut written = 0usize;
    let mut tally = Tally::default();
    for (db, mut keys) in wanted {
        keys.sort_unstable();
        keys.dedup();
        let path = Path::new(dir).join(&db);
        let mut view = DbView::open(&path).expect("the database opens");
        let ext = view.index_root().ext;
        let page_size = view.pages_mut().page_size() as usize;
        for (page_no, offset_words) in keys {
            let entry = IndexEntry {
                refno: RefNo::new(0, 0),
                page_no,
                offset_words,
                flag: 1,
            };
            write_row(&db, &entry, &mut view, ext, page_size, &mut out, &mut tally);
            written += 1;
        }
    }
    out.flush().expect("output flushes");
    eprintln!("{written} keys -> {out_path}");
    eprintln!(
        "reader signature: {}",
        if tally.exact == tally.parsed {
            "reads from the stated addresses (522a252 or later)"
        } else {
            "searches for the record's end (before 522a252)"
        }
    );
}

// ------------------------------------------------------- structural facts

/// One tail block, decoded here rather than through `record::block` so that
/// this file also compiles at `522a252^`, where that module does not exist.
#[derive(Clone, Copy)]
struct Blk {
    kind: u16,
    total_bytes: usize,
    continues_at: Option<(u32, usize)>,
}

fn blk_at(data: &[u8], at: usize, owner: RefNo) -> Option<Blk> {
    let header = data.get(at..at + 20)?;
    let kind = u16::from_be_bytes([header[0], header[1]]);
    if kind != 1 && kind != 2 {
        return None;
    }
    let words = u16::from_be_bytes([header[2], header[3]]) as usize;
    if words < 5 {
        return None;
    }
    if RefNo::new(be32(header, 4)?, be32(header, 8)?) != owner {
        return None;
    }
    let page_no = be32(header, 12)?;
    let packed = be32(header, 16)?;
    Some(Blk {
        kind,
        total_bytes: words * 4,
        continues_at: (page_no != 0 || packed != 0)
            .then_some((page_no, (packed >> 12) as usize * 2)),
    })
}

fn slot(data: &[u8], at: usize) -> Option<(u32, usize)> {
    let page_no = be32(data, at)?;
    let packed = be32(data, at + 4)?;
    (page_no != 0 || packed != 0).then_some((page_no, (packed >> 12) as usize * 2))
}

fn be32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

/// Where the record says its parts are, read straight off the pages.
///
/// Identical on both sides by construction — no reader is involved — which is
/// what makes it usable as the reason a value changed.
///
/// * `exp` / `mem`: the two header slots, `page:byte` or `-`.
/// * `ie`: the byte just past the implicit region, on the record's own page.
/// * `tile`: 1 when the blocks the addresses reach exactly fill the space
///   behind the implicit region on that page, which is the only shape reading
///   by adjacency can reproduce.
/// * `cont`: how many of them state a continuation.
/// * `edge`: how many end exactly at the end of their page.
/// * `S`: every block reached, `page:byte:kind:words:contpage:contbyte`.
fn facts(
    pages: &mut e3d_io::page::cache::PageCache,
    ext: u16,
    entry: &IndexEntry,
    page_size: usize,
) -> String {
    let at = entry.offset_words as usize * 2;
    let home = PageId::new(ext, entry.page_no);
    let Ok(page) = pages.get(home) else {
        return "unreadable".to_string();
    };
    let Some(header) = page.get(at..).filter(|rest| rest.len() >= 44) else {
        return "short".to_string();
    };
    let implicit_words = u32::from_be_bytes(header[0..4].try_into().unwrap());
    if implicit_words < 11 {
        return "notarecord".to_string();
    }
    let refno = RefNo::new(be32(header, 4).unwrap(), be32(header, 8).unwrap());
    let explicit = slot(header, 24);
    let members = slot(header, 32);
    let implicit_end = at + implicit_words as usize * 4;

    let mut blocks: Vec<(u32, usize, u16, usize, Option<(u32, usize)>)> = Vec::new();
    let mut seen: Vec<(u32, usize)> = Vec::new();
    for start in [members, explicit].into_iter().flatten() {
        let mut cursor = start;
        loop {
            if seen.contains(&cursor) || seen.len() > 64 {
                break;
            }
            seen.push(cursor);
            let Ok(data) = pages.get(PageId::new(ext, cursor.0)) else {
                break;
            };
            let Some(block) = blk_at(data, cursor.1, refno) else {
                break;
            };
            blocks.push((
                cursor.0,
                cursor.1,
                block.kind,
                block.total_bytes,
                block.continues_at,
            ));
            match block.continues_at {
                Some(next) => cursor = next,
                None => break,
            }
        }
    }

    let total: usize = blocks.iter().map(|block| block.3).sum();
    let mut tiles = !blocks.is_empty();
    let mut want = implicit_end;
    let mut ordered = blocks.clone();
    ordered.sort_by_key(|block| (block.0, block.1));
    for block in &ordered {
        if block.0 != entry.page_no || block.1 != want {
            tiles = false;
            break;
        }
        want += block.3;
    }
    if blocks.is_empty() {
        tiles = true;
    }

    // Does a list begin exactly where reading by adjacency looks for it? That
    // is the one thing the old reader needed to find anything at all; when it
    // is false, the blocks are somewhere the old reader had no pointer to.
    let head_is_adjacent = blocks.is_empty()
        || [members, explicit]
            .into_iter()
            .flatten()
            .any(|(page_no, byte)| page_no == entry.page_no && byte == implicit_end);

    let continues = blocks.iter().filter(|block| block.4.is_some()).count();
    let at_edge = blocks
        .iter()
        .filter(|block| block.1 + block.3 == page_size)
        .count();

    let mut rendered = String::new();
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 {
            rendered.push(',');
        }
        let (cont_page, cont_byte) = block.4.unwrap_or((0, 0));
        let _ = write!(
            rendered,
            "{}:{}:{}:{}:{}:{}",
            block.0,
            block.1,
            block.2,
            block.3 / 4,
            cont_page,
            cont_byte
        );
    }

    format!(
        "exp={};mem={};ie={}:{};tail={};tile={};head={};cont={};edge={};S={}",
        addr(explicit),
        addr(members),
        entry.page_no,
        implicit_end,
        total,
        u8::from(tiles),
        u8::from(head_is_adjacent),
        continues,
        at_edge,
        rendered
    )
}

fn addr(value: Option<(u32, usize)>) -> String {
    match value {
        Some((page_no, byte)) => format!("{page_no}:{byte}"),
        None => "-".to_string(),
    }
}

// ------------------------------------------------------------- rendering

fn render_members(members: &[RefNo]) -> String {
    let mut out = String::new();
    for (index, member) in members.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(out, "{member}");
    }
    out
}

fn render_attributes(parsed: &ParsedElement) -> String {
    let mut out = String::new();
    for (index, attribute) in parsed.explicit_attributes.iter().enumerate() {
        if index > 0 {
            out.push(';');
        }
        let _ = write!(
            out,
            "{:08X}:{}:{}={}",
            attribute.hash,
            attribute.type_code,
            u8::from(attribute.is_uda),
            render_value(&attribute.value)
        );
    }
    out
}

fn render_value(value: &ExplicitValue) -> String {
    match value {
        ExplicitValue::Integer(v) => format!("i{v}"),
        ExplicitValue::Real(v) => format!("r{v:?}"),
        ExplicitValue::Bool(v) => format!("b{}", u8::from(*v)),
        ExplicitValue::Text(v) => format!("t{}", escape(v)),
        ExplicitValue::RefNo(v) => format!("n{v}"),
        ExplicitValue::IntArray(v) => array("ia", v.len(), v.iter().map(|x| x.to_string())),
        ExplicitValue::RealArray(v) => array("ra", v.len(), v.iter().map(|x| format!("{x:?}"))),
        ExplicitValue::RefNoArray(v) => array("na", v.len(), v.iter().map(|x| x.to_string())),
        ExplicitValue::TextList(v) => array("tl", v.len(), v.iter().map(|x| escape(x))),
        ExplicitValue::RawWords(v) => array("rw", v.len(), v.iter().map(|x| format!("{x:08X}"))),
    }
}

/// An array shows its first [`ARRAY_SHOWN`] elements and a digest of all of
/// them, so a long one stays one line and still compares exactly.
fn array(tag: &str, len: usize, items: impl Iterator<Item = String>) -> String {
    let all: Vec<String> = items.collect();
    let mut digest = 0xcbf2_9ce4_8422_2325u64;
    for item in &all {
        for byte in item.as_bytes() {
            digest ^= u64::from(*byte);
            digest = digest.wrapping_mul(0x0000_0100_0000_01b3);
        }
        digest ^= 0xFF;
        digest = digest.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let limit = array_shown();
    let shown = all
        .iter()
        .take(limit)
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    let more = if len > limit { "+" } else { "" };
    format!("{tag}{len}#{digest:016x}[{shown}{more}]")
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            ';' | '|' | ',' | '[' | ']' | '=' | '\t' | '\n' | '\r' => {
                let _ = write!(out, "\\x{:02X}", ch as u32 & 0xFF);
            }
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\x{:02X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

fn one_line(text: &str) -> String {
    text.replace(['\t', '\n', '\r'], " ")
}

// ---------------------------------------------------------------- diff side

#[derive(Clone)]
struct Row {
    db: String,
    page_no: u32,
    offset_words: u32,
    key: String,
    status: String,
    detail: String,
    members: String,
    attrs: String,
    facts: String,
}

impl Row {
    fn parse(line: &str) -> Option<Self> {
        let mut columns = line.split('\t');
        let db = columns.next()?.to_string();
        let page_no = columns.next()?.parse().ok()?;
        let offset_words = columns.next()?.parse().ok()?;
        Some(Self {
            db,
            page_no,
            offset_words,
            key: columns.next()?.to_string(),
            status: columns.next()?.to_string(),
            detail: columns.next()?.to_string(),
            members: columns.next()?.to_string(),
            attrs: columns.next()?.to_string(),
            facts: columns.next().unwrap_or_default().to_string(),
        })
    }

    fn at(&self) -> String {
        format!("{} {}:{}", self.db, self.page_no, self.offset_words)
    }

    fn noun_hash(&self) -> &str {
        self.detail.split('|').nth(1).unwrap_or("?")
    }

    fn fact(&self, key: &str) -> &str {
        self.facts
            .split(';')
            .find_map(|part| part.strip_prefix(key)?.strip_prefix('='))
            .unwrap_or("")
    }

    /// hash -> the values stored under it, in stream order.
    fn attribute_map(&self) -> BTreeMap<String, Vec<String>> {
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        if self.attrs.is_empty() {
            return out;
        }
        for item in self.attrs.split(';') {
            let (hash, rest) = item.split_once(':').unwrap_or((item, ""));
            out.entry(hash.to_string())
                .or_default()
                .push(rest.to_string());
        }
        out
    }
}

/// Why one record's values differ, in the vocabulary the evidence file uses.
///
/// These are not exclusive and are not guessed: each is read off the pages, so
/// a record can carry more than one and every changed record must carry at
/// least one. [`Cause::Unexplained`] exists so that a change the structure does
/// not account for is counted and named rather than folded into a total.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Cause {
    /// §4 — the old reader never delivered the record: no aligned
    /// `0x00000007` inside its 64 KiB limit.
    EndMarker,
    /// §3.1 — no list begins where adjacency looks, so the old reader had
    /// nothing to walk from.
    BlockAddress,
    /// §3.2 — a block states a continuation. The old reader bridged the page
    /// boundary by stepping over filler instead of following the address.
    Continuation,
    /// §2 — the two words after a block's owner are its continuation address,
    /// and the old explicit decoder took the payload from +12 unless they read
    /// zero. On a block that continues they do not, so the address itself was
    /// decoded as attribute data.
    PayloadStart,
    /// Nothing in the record's own structure accounts for the change.
    Unexplained,
}

impl Cause {
    fn label(self) -> &'static str {
        match self {
            Self::EndMarker => "end-marker search (§4)",
            Self::BlockAddress => "block address (§3.1)",
            Self::Continuation => "continuation (§3.2)",
            Self::PayloadStart => "payload +20 (§2)",
            Self::Unexplained => "UNEXPLAINED",
        }
    }
}

fn causes(before: &Row, after: &Row) -> Vec<Cause> {
    if before.status != "ok" {
        return vec![Cause::EndMarker];
    }
    let mut out = Vec::new();
    if after.fact("head") == "0" {
        out.push(Cause::BlockAddress);
    }
    if after.fact("cont") != "0" {
        out.push(Cause::Continuation);
        out.push(Cause::PayloadStart);
    }
    if out.is_empty() {
        out.push(Cause::Unexplained);
    }
    out
}

fn cause_label(causes: &[Cause]) -> String {
    causes
        .iter()
        .map(|cause| cause.label())
        .collect::<Vec<_>>()
        .join(" + ")
}

#[derive(Default)]
struct Counters {
    keys: usize,
    identical: usize,
    changed: usize,
    before_failed: usize,
    after_failed: usize,
    both_failed: usize,
    members_changed: usize,
    attrs_changed: usize,
    noun_hash_changed: usize,
    surplus_bytes_before: usize,
    added: usize,
    added_declared: usize,
    removed: usize,
    removed_declared: usize,
    modified: usize,
    modified_declared: usize,
    members_added: usize,
    members_removed: usize,
}

fn diff(args: &[String]) {
    let before_path = args.first().expect("the before dump");
    let after_path = args.get(1).expect("the after dump");
    let out_dir = args.get(2).map(String::as_str).unwrap_or("l3-delta");
    let catalog_dir = args.get(3).map(String::as_str).unwrap_or(CATALOG);

    std::fs::create_dir_all(out_dir).expect("output directory");
    let catalog = NounCatalog::read(
        Path::new(catalog_dir).join("noun_layout.json"),
        Path::new(catalog_dir).join("noun_attr_fields.json"),
    )
    .ok();
    if catalog.is_none() {
        eprintln!("no catalog under {catalog_dir}: hashes will be reported unnamed");
    }
    let noun_name = |hex: &str| -> String {
        let hash = u32::from_str_radix(hex, 16).unwrap_or(0);
        catalog
            .as_ref()
            .and_then(|catalog| catalog.noun(hash))
            .map(|noun| noun.noun.clone())
            .unwrap_or_else(|| format!("hash:{hex}"))
    };
    // An attribute E3D declares, as opposed to a hash the old decoder produced
    // by starting the stream eight bytes early. 4271 attributes are exported;
    // a "removed" attribute that is not one of them was never an attribute.
    let known = |hex: &str| -> bool {
        let hash = u32::from_str_radix(hex, 16).unwrap_or(0);
        catalog
            .as_ref()
            .is_some_and(|catalog| catalog.attribute_fields(hash).is_some())
    };
    let attr_name = |hex: &str| -> String {
        let hash = u32::from_str_radix(hex, 16).unwrap_or(0);
        catalog
            .as_ref()
            .and_then(|catalog| catalog.attribute_fields(hash))
            .map(|field| field.name.clone())
            .unwrap_or_else(|| format!("hash:{hex}"))
    };

    let mut before = BufReader::with_capacity(1 << 22, File::open(before_path).expect("before"))
        .lines()
        .map(|line| line.expect("before reads"));
    let mut after = BufReader::with_capacity(1 << 22, File::open(after_path).expect("after"))
        .lines()
        .map(|line| line.expect("after reads"));

    let mut counters = Counters::default();
    let mut by_cause: BTreeMap<Cause, usize> = BTreeMap::new();
    let mut by_cause_set: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_noun: BTreeMap<String, [usize; 4]> = BTreeMap::new();
    let mut by_attribute: BTreeMap<String, [usize; 3]> = BTreeMap::new();
    let mut per_db: BTreeMap<String, usize> = BTreeMap::new();
    let mut regressions: Vec<String> = Vec::new();
    let mut invented: BTreeMap<String, usize> = BTreeMap::new();
    let mut cases: BTreeMap<String, Vec<String>> = BTreeMap::new();

    let mut changed_out = BufWriter::with_capacity(
        1 << 22,
        File::create(Path::new(out_dir).join("changed.tsv")).expect("changed.tsv"),
    );
    writeln!(
        changed_out,
        "db\tpage\toff\tkey\tcause\tbefore_status\tafter_status\tadded\tremoved\tmodified\t\
         members_before\tmembers_after\tnoun\tdetail"
    )
    .unwrap();

    loop {
        let (line_before, line_after) = match (before.next(), after.next()) {
            (Some(a), Some(b)) => (a, b),
            (None, None) => break,
            _ => panic!("the two dumps have different lengths"),
        };
        if line_before.starts_with('#') || line_after.starts_with('#') {
            assert_eq!(
                line_before, line_after,
                "the two dumps disagree about which databases they cover"
            );
            continue;
        }
        let before = Row::parse(&line_before).expect("a before row");
        let after = Row::parse(&line_after).expect("an after row");
        assert_eq!(
            (&before.db, before.page_no, before.offset_words, &before.key),
            (&after.db, after.page_no, after.offset_words, &after.key),
            "the two dumps are not at the same key"
        );
        counters.keys += 1;

        let same = before.status == after.status
            && before.members == after.members
            && before.attrs == after.attrs
            && before.noun_hash() == after.noun_hash();
        if same {
            counters.identical += 1;
            continue;
        }
        counters.changed += 1;
        *per_db.entry(before.db.clone()).or_default() += 1;

        match (before.status.as_str(), after.status.as_str()) {
            ("ok", "ok") => {}
            ("ok", _) => counters.after_failed += 1,
            (_, "ok") => counters.before_failed += 1,
            _ => counters.both_failed += 1,
        }
        if before.status == "ok" && after.status == "ok" && before.noun_hash() != after.noun_hash()
        {
            counters.noun_hash_changed += 1;
        }

        let causes = causes(&before, &after);
        let cause = cause_label(&causes);
        for one in &causes {
            *by_cause.entry(*one).or_default() += 1;
        }
        *by_cause_set.entry(cause.clone()).or_default() += 1;

        let before_attrs = before.attribute_map();
        let after_attrs = after.attribute_map();
        let mut added = Vec::new();
        let mut removed = Vec::new();
        let mut modified = Vec::new();
        for hash in before_attrs.keys().chain(after_attrs.keys()) {
            let (old, new) = (before_attrs.get(hash), after_attrs.get(hash));
            match (old, new) {
                (None, Some(values)) => added.push((hash.clone(), values.clone())),
                (Some(values), None) => removed.push((hash.clone(), values.clone())),
                (Some(old), Some(new)) if old != new => {
                    modified.push((hash.clone(), old.clone(), new.clone()))
                }
                _ => {}
            }
        }
        added.dedup_by(|a, b| a.0 == b.0);
        removed.dedup_by(|a, b| a.0 == b.0);
        modified.dedup_by(|a, b| a.0 == b.0);

        counters.added += added.len();
        counters.added_declared += added.iter().filter(|(hash, _)| known(hash)).count();
        counters.removed += removed.len();
        counters.removed_declared += removed.iter().filter(|(hash, _)| known(hash)).count();
        counters.modified += modified.len();
        counters.modified_declared += modified.iter().filter(|(hash, _, _)| known(hash)).count();
        if !added.is_empty() || !removed.is_empty() || !modified.is_empty() {
            counters.attrs_changed += 1;
        }

        let members_before = count_members(&before.members);
        let members_after = count_members(&after.members);
        if before.members != after.members {
            counters.members_changed += 1;
            let (gained, lost) = member_delta(&before.members, &after.members);
            counters.members_added += gained;
            counters.members_removed += lost;
        }

        let noun = if after.status == "ok" {
            noun_name(after.noun_hash())
        } else {
            noun_name(before.noun_hash())
        };
        let slot = by_noun.entry(noun.clone()).or_default();
        slot[0] += 1;
        slot[1] += added.len();
        slot[2] += removed.len();
        slot[3] += modified.len();
        for (hash, _) in &added {
            by_attribute.entry(attr_name(hash)).or_default()[0] += 1;
        }
        for (hash, _) in &removed {
            by_attribute.entry(attr_name(hash)).or_default()[1] += 1;
        }
        for (hash, _, _) in &modified {
            by_attribute.entry(attr_name(hash)).or_default()[2] += 1;
        }

        // Every hash the old reader reported and the new one does not, split
        // by whether E3D declares such an attribute at all. A hash that is not
        // in the 4271 exported ones was produced by decoding something that is
        // not an attribute header, so losing it is not losing a value.
        let (lost_real, lost_invented): (Vec<_>, Vec<_>) =
            removed.iter().partition(|(hash, _)| known(hash));
        for (hash, _) in &lost_invented {
            *invented.entry(hash.clone()).or_default() += 1;
        }

        // A record the new reader delivers with less than the old one is the
        // only shape that could be a step backwards, so every one is listed
        // rather than folded into a total.
        let lost_ground = (!lost_real.is_empty() && after.status == "ok")
            || (before.status == "ok" && after.status != "ok")
            || (members_after < members_before && after.status == "ok");
        if lost_ground {
            regressions.push(format!(
                "{}\t{}\t{}\t{cause}\tlost={}\tmembers {members_before}->{members_after}\t{}->{}",
                before.at(),
                before.key,
                noun,
                lost_real
                    .iter()
                    .map(|(hash, values)| format!("{}={}", attr_name(hash), join_short(values)))
                    .collect::<Vec<_>>()
                    .join(" "),
                before.status,
                after.status
            ));
        }

        let detail = format!(
            "added[{}] removed[{}] modified[{}]",
            added
                .iter()
                .take(6)
                .map(|(hash, values)| format!("{}={}", attr_name(hash), join_short(values)))
                .collect::<Vec<_>>()
                .join(" "),
            removed
                .iter()
                .take(6)
                .map(|(hash, values)| format!("{}={}", attr_name(hash), join_short(values)))
                .collect::<Vec<_>>()
                .join(" "),
            modified
                .iter()
                .take(6)
                .map(|(hash, old, new)| format!(
                    "{}: {} -> {}",
                    attr_name(hash),
                    join_short(old),
                    join_short(new)
                ))
                .collect::<Vec<_>>()
                .join(" ")
        );
        writeln!(
            changed_out,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{members_before}\t{members_after}\t{noun}\t{detail}",
            before.db,
            before.page_no,
            before.offset_words,
            before.key,
            cause,
            before.status,
            after.status,
            added.len(),
            removed.len(),
            modified.len()
        )
        .unwrap();

        let bucket = cases.entry(cause).or_default();
        if bucket.len() < 12 {
            bucket.push(format!(
                "{} key {} noun {}\n    facts   {}\n    before  [{}] members[{}] {}\n    \
                 after   [{}] members[{}] {}\n    delta   {}",
                before.at(),
                before.key,
                noun,
                after.facts,
                before.status,
                truncate(&before.members, 160),
                truncate(&before.attrs, 420),
                after.status,
                truncate(&after.members, 160),
                truncate(&after.attrs, 420),
                detail
            ));
        }

        if before.status == "ok" && after.status == "ok" {
            let before_total: usize = before
                .detail
                .split('|')
                .nth(5)
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            let before_end: usize = before
                .detail
                .split('|')
                .nth(4)
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            counters.surplus_bytes_before += before_total.saturating_sub(before_end);
        }
    }
    changed_out.flush().unwrap();

    let mut report =
        BufWriter::new(File::create(Path::new(out_dir).join("report.txt")).expect("report.txt"));
    let mut say = |line: String| {
        println!("{line}");
        writeln!(report, "{line}").unwrap();
    };

    say(format!("keys compared            {}", counters.keys));
    say(format!("identical                {}", counters.identical));
    say(format!("changed                  {}", counters.changed));
    say(format!(
        "  before failed, after ok  {}",
        counters.before_failed
    ));
    say(format!(
        "  before ok, after failed  {}",
        counters.after_failed
    ));
    say(format!(
        "  both failed              {}",
        counters.both_failed
    ));
    say(format!(
        "  noun hash changed        {}",
        counters.noun_hash_changed
    ));
    say(format!(
        "records with attribute changes {}",
        counters.attrs_changed
    ));
    say(format!(
        "records with member changes    {}",
        counters.members_changed
    ));
    say(format!(
        "attributes  added {}  removed {}  modified {}",
        counters.added, counters.removed, counters.modified
    ));
    say(format!(
        "  of which E3D declares as attributes: added {}  removed {}  modified {}",
        counters.added_declared, counters.removed_declared, counters.modified_declared
    ));
    say(format!(
        "members     added {}  removed {}",
        counters.members_added, counters.members_removed
    ));
    say(format!(
        "bytes the old reader handed over past the record's own end, on changed keys: {}",
        counters.surplus_bytes_before
    ));

    say(String::new());
    say("by cause (a record can carry more than one)".to_string());
    for (cause, count) in &by_cause {
        say(format!("  {:<24} {count}", cause.label()));
    }
    say("by cause combination".to_string());
    let mut sets: Vec<_> = by_cause_set.iter().collect();
    sets.sort_by_key(|(name, count)| (std::cmp::Reverse(**count), (*name).clone()));
    for (name, count) in sets {
        say(format!("  {name:<62} {count}"));
    }

    say(String::new());
    say("by noun (records, +added, -removed, ~modified)".to_string());
    let mut nouns: Vec<_> = by_noun.iter().collect();
    nouns.sort_by_key(|(name, slot)| (std::cmp::Reverse(slot[0]), (*name).clone()));
    for (name, slot) in nouns.iter().take(60) {
        say(format!(
            "  {:<20} {:>7}  +{:<7} -{:<7} ~{}",
            name, slot[0], slot[1], slot[2], slot[3]
        ));
    }
    say(format!("  ... {} nouns in total", by_noun.len()));

    say(String::new());
    say("by attribute (+added, -removed, ~modified)".to_string());
    let mut attributes: Vec<_> = by_attribute.iter().collect();
    attributes.sort_by_key(|(name, slot)| {
        (
            std::cmp::Reverse(slot[0] + slot[1] + slot[2]),
            (*name).clone(),
        )
    });
    for (name, slot) in attributes.iter().take(60) {
        say(format!(
            "  {:<28} +{:<8} -{:<8} ~{}",
            name, slot[0], slot[1], slot[2]
        ));
    }
    say(format!("  ... {} attributes in total", by_attribute.len()));

    say(String::new());
    say(format!(
        "databases with at least one changed record: {}",
        per_db.len()
    ));
    let mut dbs: Vec<_> = per_db.iter().collect();
    dbs.sort_by_key(|(name, count)| (std::cmp::Reverse(**count), (*name).clone()));
    for (name, count) in dbs.iter().take(20) {
        say(format!("  {name:<20} {count}"));
    }

    say(String::new());
    say(format!(
        "hashes the old reader reported that E3D does not declare as attributes: {} distinct, \
         {} occurrences",
        invented.len(),
        invented.values().sum::<usize>()
    ));
    let mut made_up: Vec<_> = invented.iter().collect();
    made_up.sort_by_key(|(name, count)| (std::cmp::Reverse(**count), (*name).clone()));
    for (name, count) in made_up.iter().take(30) {
        say(format!("  {name:<12} {count}"));
    }

    say(String::new());
    say(format!(
        "records where the new reader delivers less — a declared attribute, a member, or the \
         whole record: {}",
        regressions.len()
    ));
    for line in regressions.iter().take(200) {
        say(format!("  {line}"));
    }
    if regressions.len() > 200 {
        say(format!("  ... and {} more", regressions.len() - 200));
    }

    say(String::new());
    say("worked cases".to_string());
    for (cause, bucket) in &cases {
        say(format!("== {cause} =="));
        for case in bucket {
            say(format!("  {case}"));
        }
    }

    report.flush().unwrap();
    eprintln!("wrote {out_dir}/report.txt and {out_dir}/changed.tsv");
}

// ------------------------------------------------------------ splice check

/// Is every changed array value the old one with the record's own continuation
/// addresses taken out of it?
///
/// The claim §2 makes about values, checked rather than illustrated. A block's
/// two words after the owner are `(page_no, packed)`; the old explicit decoder
/// took the payload from byte 12 unless those two words read zero, so on a
/// block that continues it read the address as attribute data. The address
/// therefore sits inside the value, and deleting it should give the value the
/// new reader returns. If some other pair has to be deleted to make the two
/// line up, the deletion is not explained and the record is named.
///
/// Needs `redump` output on both sides: the ordinary dump abbreviates arrays.
fn splice(args: &[String]) {
    let before_path = args.first().expect("the before full dump");
    let after_path = args.get(1).expect("the after full dump");

    let mut before = BufReader::with_capacity(1 << 22, File::open(before_path).expect("before"))
        .lines()
        .map(|line| line.expect("before reads"));
    let mut after = BufReader::with_capacity(1 << 22, File::open(after_path).expect("after"))
        .lines()
        .map(|line| line.expect("after reads"));

    let (mut records, mut checked, mut explained, mut pairs) = (0usize, 0usize, 0usize, 0usize);
    let mut unexplained: Vec<String> = Vec::new();
    let mut not_array: Vec<String> = Vec::new();
    let mut on_the_count: Vec<String> = Vec::new();

    while let (Some(line_before), Some(line_after)) = (before.next(), after.next()) {
        let before = Row::parse(&line_before).expect("a before row");
        let after = Row::parse(&line_after).expect("an after row");
        assert_eq!(
            before.at(),
            after.at(),
            "the two dumps are not at the same key"
        );
        records += 1;
        if before.status != "ok" || after.status != "ok" {
            continue;
        }
        let declared = continuations(&after);
        let before_attrs = before.attribute_map();
        let after_attrs = after.attribute_map();
        for (hash, new_values) in &after_attrs {
            let Some(old_values) = before_attrs.get(hash) else {
                continue;
            };
            if old_values == new_values {
                continue;
            }
            checked += 1;
            let (Some(old), Some(new)) = (old_values.first(), new_values.first()) else {
                continue;
            };
            let (Some(old), Some(new)) = (elements(old), elements(new)) else {
                not_array.push(format!("{} {hash}: {old} -> {new}", before.at()));
                continue;
            };
            match align(&old, &new, &declared) {
                Some((skipped, taken)) => {
                    explained += 1;
                    pairs += taken.len();
                    // An integer array states its own length in its first data
                    // word. When an insertion lands there, the old reader took
                    // a page number for the length: the value is not the same
                    // one with two words in it, it is a shorter array starting
                    // a word early.
                    if !skipped.is_empty() {
                        on_the_count.push(format!(
                            "{} {hash}: {} elements instead of {}, and what is left of the \
                             insertion is in front of them as {skipped:?}",
                            before.at(),
                            old.len(),
                            new.len()
                        ));
                    }
                }
                None => unexplained.push(format!(
                    "{} {hash}: no run of deletions of the record's own continuation addresses \
                     {declared:?} lines the two up ({} -> {} elements)",
                    before.at(),
                    old.len(),
                    new.len()
                )),
            }
        }
    }

    println!("records read              {records}");
    println!("values that differ        {checked}");
    println!("explained by deleting the record's own continuation addresses: {explained}");
    println!("addresses deleted         {pairs}");
    println!(
        "explained by the insertion landing on the array's own length word: {}",
        on_the_count.len()
    );
    for line in on_the_count.iter().take(20) {
        println!("  {line}");
    }
    println!("values that are not arrays: {}", not_array.len());
    for line in not_array.iter().take(20) {
        println!("  {line}");
    }
    println!("values not explained      {}", unexplained.len());
    for line in unexplained.iter().take(40) {
        println!("  {line}");
    }
}

/// `(page_no, packed)` for every continuation the record's blocks declare.
///
/// `packed` is rebuilt the way a leaf value encodes it — `offset_words << 12`
/// with the low 12 bits the flag, which reads 1 on all 158 526 continuations
/// in the corpus — because that is the word the old decoder saw.
fn continuations(row: &Row) -> Vec<(String, String)> {
    row.fact("S")
        .split(',')
        .filter_map(|block| {
            let parts: Vec<&str> = block.split(':').collect();
            let page: u32 = parts.get(4)?.parse().ok()?;
            let byte: u32 = parts.get(5)?.parse().ok()?;
            (page != 0).then(|| (page.to_string(), (((byte / 2) << 12) | 1).to_string()))
        })
        .collect()
}

/// The elements of a rendered array value, or `None` when it is not one.
fn elements(value: &str) -> Option<Vec<String>> {
    let (head, rest) = value.split_once('[')?;
    if !head.contains('#') {
        return None;
    }
    let body = rest.strip_suffix(']')?;
    if body.ends_with('+') {
        return None;
    }
    Some(if body.is_empty() {
        Vec::new()
    } else {
        body.split(' ').map(str::to_string).collect()
    })
}

/// Turn the old value into the new one by deleting addresses the record itself
/// declares, and say what had to be deleted.
///
/// Greedy from the left: the two agree until an insertion, and only a pair the
/// record declares as a continuation may be deleted, so an alignment that
/// succeeds is an alignment made of this record's own addresses rather than of
/// whatever two words happened to fit.
///
/// The leading `skipped` elements cover the case where the insertion landed on
/// the array's length word — the old array then begins mid-insertion and is
/// shorter, so a prefix of it has to be dropped before the walk lines up.
#[allow(clippy::type_complexity)]
fn align(
    old: &[String],
    new: &[String],
    declared: &[(String, String)],
) -> Option<(Vec<String>, Vec<(String, String)>)> {
    for skip in 0..old.len().min(3) {
        let rest = &old[skip..];
        let mut taken = Vec::new();
        let (mut i, mut j) = (0usize, 0usize);
        let mut lines_up = true;
        while i < rest.len() {
            if j < new.len() && rest[i] == new[j] {
                i += 1;
                j += 1;
            } else if i + 1 < rest.len()
                && declared.contains(&(rest[i].clone(), rest[i + 1].clone()))
            {
                taken.push((rest[i].clone(), rest[i + 1].clone()));
                i += 2;
            } else {
                lines_up = false;
                break;
            }
        }
        if lines_up && (skip > 0 || !taken.is_empty()) {
            return Some((old[..skip].to_vec(), taken));
        }
    }
    None
}

// ---------------------------------------------------------------- one record

/// One record, in full, from whichever reader this binary was built with.
///
/// The dump abbreviates long arrays; a case study cannot. Run the same
/// arguments against both builds and the two outputs are the before and after
/// of one element, word for word.
fn case(args: &[String]) {
    let path = args.first().expect("a database path");
    let page_no: u32 = args.get(1).expect("a page").parse().expect("a page number");
    let offset_words: u32 = args
        .get(2)
        .expect("an offset in 2-byte words")
        .parse()
        .expect("an offset");
    let catalog_dir = args.get(3).map(String::as_str).unwrap_or(CATALOG);
    let catalog = NounCatalog::read(
        Path::new(catalog_dir).join("noun_layout.json"),
        Path::new(catalog_dir).join("noun_attr_fields.json"),
    )
    .ok();

    let mut view = DbView::open(path).expect("the database opens");
    let sesno = view.selected_session();
    let ext = view.index_root().ext;
    let page_size = view.pages_mut().page_size() as usize;
    let entry = IndexEntry {
        refno: RefNo::new(0, 0),
        page_no,
        offset_words,
        flag: 1,
    };

    println!(
        "{path} session {sesno} ext {ext} page {page_no} byte {}",
        offset_words * 2
    );
    println!(
        "facts   {}",
        facts(view.pages_mut(), ext, &entry, page_size)
    );

    match read_element_record(view.pages_mut(), ext, &entry) {
        Ok(record) => {
            let parsed = match ParsedElement::parse(&record.raw_bytes) {
                Ok(parsed) => parsed,
                Err(error) => {
                    println!("parse failed: {error}");
                    return;
                }
            };
            let noun = catalog
                .as_ref()
                .and_then(|catalog| catalog.noun(parsed.noun_hash))
                .map(|noun| noun.noun.clone())
                .unwrap_or_else(|| format!("hash:{:08X}", parsed.noun_hash));
            println!(
                "refno {} noun {noun} owner {} implicit {} words, prefix {}, parser stopped at \
                 {} of {} bytes handed over",
                parsed.refno,
                parsed.owner,
                parsed.impl_len_words,
                parsed.prefix_offset,
                parsed.record_end,
                record.raw_bytes.len()
            );
            println!("members ({})", parsed.members.len());
            for member in &parsed.members {
                println!("    {member}");
            }
            println!("attributes ({})", parsed.explicit_attributes.len());
            for attribute in &parsed.explicit_attributes {
                let name = catalog
                    .as_ref()
                    .and_then(|catalog| catalog.attribute_fields(attribute.hash))
                    .map(|field| field.name.clone())
                    .unwrap_or_else(|| "<not a declared attribute>".to_string());
                println!(
                    "    {:08X} {name:<12} type {:<6} words {:<4} uda {} = {}",
                    attribute.hash,
                    attribute.type_code,
                    attribute.data_words,
                    u8::from(attribute.is_uda),
                    full_value(&attribute.value)
                );
            }
        }
        Err(error) => println!("read failed: {error}"),
    }
}

fn full_value(value: &ExplicitValue) -> String {
    match value {
        ExplicitValue::IntArray(v) => format!("int[{}] {v:?}", v.len()),
        ExplicitValue::RealArray(v) => format!("real[{}] {v:?}", v.len()),
        ExplicitValue::RefNoArray(v) => format!(
            "refno[{}] {}",
            v.len(),
            v.iter().map(RefNo::to_string).collect::<Vec<_>>().join(" ")
        ),
        ExplicitValue::RawWords(v) => format!(
            "raw[{}] {}",
            v.len(),
            v.iter()
                .map(|word| format!("{word:08X}"))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        other => render_value(other),
    }
}

fn count_members(members: &str) -> usize {
    if members.is_empty() {
        0
    } else {
        members.split(',').count()
    }
}

fn member_delta(before: &str, after: &str) -> (usize, usize) {
    let old: Vec<&str> = if before.is_empty() {
        Vec::new()
    } else {
        before.split(',').collect()
    };
    let new: Vec<&str> = if after.is_empty() {
        Vec::new()
    } else {
        after.split(',').collect()
    };
    let gained = new.iter().filter(|item| !old.contains(item)).count();
    let lost = old.iter().filter(|item| !new.contains(item)).count();
    (gained, lost)
}

fn join_short(values: &[String]) -> String {
    truncate(&values.join("/"), 120)
}

fn truncate(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…(+{})", &text[..end], text.len() - end)
}
