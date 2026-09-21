# e3d-io

Standalone IO library for AVEVA E3D 2.1 (`Everything3D 2.10`) and E3D 3.1
database files. Provides page-level access, B+tree index search,
element-record decoding, a high-level attribute resolver, and — for E3D 3.1 —
a write path that appends sessions to a database the way `core.dll` does
(see [Write path](#write-path)).

Reverse-engineered from `core.dll` via IDA Pro; all field offsets and constants in this crate cite IDA evidence in the parent IDA exports / docs (see [`pdms-io-fork-engine-v2/ida_exports/3.1/`](https://github.com/) and `docs/ida-3.1-{structures,attributes}.md` in the goals repo).

## Status

- **Read path**: page I/O + LRU cache + descriptor + session + B+tree index search + element record decoder.
- **Attribute decoding**: implicit and explicit scalar/array values, noun descriptors loaded directly from `desvir.dat`, report names loaded directly from `attlib.dat`, and a TTY `ATTLIST` reconciliation path for runtime-composed attributes.
- **Defaults**: an attribute with nothing stored falls back to the noun's own default record, read from chunks 1 and 2 of the same `*vir.dat` child as the descriptor — the values `core.dll` returns from `sub_5B03B90`.
- **High-level API**: `engine.extract_element_with_descriptors(refno, &attlib, &mut provider)` returns an `ElementExtraction` carrying one row per descriptor, each with an explicit status.
- **Session ranges**: `engine.diff_sessions(from, to)` (or `DbView::diff_sessions`) names every RefNo inserted, deleted or rewritten between two retained sessions, `changed_elements` streams the record on each side, and `session::ElementDelta` says which owner / members / explicit attributes / implicit words / descriptor rows differ. `diff_steps` gives the same range one retained session at a time. `SessionSelector::AtOrBefore(n)` snaps a watermark to the newest retained session `<= n`; a session `MERGE CHANGES` removed is otherwise an error naming the retained sessions either side of it. CLI: `e3d-descriptor sessions --db …` and `e3d-descriptor diff --db … --from N [--to N] [--content on] [--steps on]`.
- **Extents**: opening `<stem>_0001` attaches the sibling files `_0002`, `_0003`, … next to it (`page::io::ExtentSet`); a page address whose extent has no file is `PageIoError::MissingExtent`, named rather than redirected into the primary file.
- **Text**: strings are decoded by the header's `FORMAT` word, the same branch `core.dll` takes before reading any string — 1 is UTF-8 as stored, anything else is the project's escaped charset (`&~ … &`, GBK here). See `record::text`.
- **Header and session words**: every word `core.dll` serves from page 0 and from a session page is tabulated against what this crate reads it as, in `meta::DbDescriptor::parse` and `session::at`. The extract model (`EXTID` / `OWNEXT` / `NOWNEX` / `INCON` / `ACCMOD` / `SUBTYP`) is tabulated but not read: no local database is an extract.
- **Write path** (E3D 3.1): `e3d_io::write` appends a session — new, changed and deleted elements, names, the `TYPEX` / reverse-reference / primary-element (`CLAIM`) tables, three-way merge against a database that moved on — and commits it through page 0, never touching a page already on disk. A real E3D 3.1 Design reads what it writes (`docs/evidence/2026-09-21-e3d-tty-gate-write-path.md`). Details below.
- **Not yet**: ATGTDF dedup, the extract / claim model, session diffs of the NAME / TYPEX / reverse-reference tables; on the write side, `SPAMAP`, the UDA tables, non-ASCII text under a non-UTF-8 `FORMAT`, multi-extent files and the write-mode sentinel lock (`docs/plans/2026-09-21-write-path-next-steps.md`).

`summarize_element` and `resolve_attribute` are deprecated. They locate an
attribute at word `6 + atgtdf_position - 1`, as though attributes sat in
ATGTDF order right behind a six-word header; `core.dll` uses the descriptor's
own `encoded_location & 0xFFFFF`, which for `BRAN` runs 11, 12, 12, 12, 12,
13, 20, 27, 34, 41, 43 — neither sequential nor based at six.

The descriptor/extraction path does not require an `att noun map` JSON file.
Project defaults and applicability come directly from the Dictionary databases
in MDB search order. Static names use `STUDNA`; dynamically installed names use
`DYUDNA`/`UPSEUD`. Supply one database with `--dictionary-db`, or an ordered
Windows semicolon list with `--dictionary-db-list`:

```powershell
e3d-descriptor extract-complete `
  --db path\to\design_db `
  --desvir path\to\desvir.dat `
  --attlib path\to\attlib.dat `
  --refno 24384/23257 `
  --dictionary-db-list 'path\to\project_dictionary;path\to\product_dictionary' `
  --reference-db-list 'path\to\catalogue_database' `
  --dicvir path\to\dicvir.dat `
  --tty-log path\to\noun_attlist.log
```

`extract-complete` uses the TTY capture as an authority check and row-order
fixture. The result preserves duplicates, `unset`, and empty rendered values,
and records whether each row came from implicit/explicit binary data,
Dictionary defaults, runtime derivation, structure, or a remaining TTY-only
gap. Plain `extract` performs the same binary and Dictionary decoding without a
TTY capture.

When `--reference-db-list` is supplied, non-null `RefNo` values are resolved to
their persisted `NAME` values. The complete report compares typed binary values
to the TTY rendering, including units, positions, directions, WORD values, and
named references, and reports per-row `equivalent`/mismatch status.

The checked-in `schema/e3d31/catalog` live export can optionally add the complete
public noun order and every exported `DbAttributeField` value to descriptor
reports without becoming a parsing dependency:

```powershell
e3d-descriptor describe `
  --desvir path\to\desvir.dat `
  --attlib path\to\attlib.dat `
  --noun BRAN `
  --catalog-layout schema\e3d31\catalog\noun_layout.json `
  --catalog-fields schema\e3d31\catalog\noun_attr_fields.json
```

## Schema packs

Everything bound to an E3D release — the installation's `*vir.dat` and
`attlib.dat`, and the catalog export taken beside them — lives under
`schema/<release>/`. None of those files states its own release, so mixing two
is silent: the descriptors decide the word offsets, and the wrong ones read
plausible numbers out of the wrong words. One directory per release is what
makes the mix hard to express. E3D 2.10 (`e3d210`) and E3D 3.1 (`e3d31`) are
currently measured and bundled.

Every command accepts a pack in place of the individual path flags:

```powershell
e3d-descriptor describe --schema schema\e3d31 --noun BRAN
```

Database-reading descriptor commands must name a pack or version:

```powershell
e3d-descriptor extract --schema schema\e3d31 `
  --db D:\...\ams8000_0001 --refno 24384/46
```

IDA confirms that `DB_Communication::updateVersion()` writes
`VersionInfo::version()` into `COMM.VERS`, but
`DB_Communication::clearComm()` clears it when that client exits. A current
Communication database can also contain several module versions. The field is
reported by `e3d-descriptor header` for diagnosis but never selects a schema.
`WORL.UPGDNO`, `Saving Meta Data.`, directory names, timestamps and file sizes
are likewise non-evidence. The primary/index tree pair
`(7618377, 13387743)` is shared by both releases and serves only as a
database-family validation. Until a durable release field is proven,
automatic selection fails loudly.

That fills `--desvir`, `--attlib`, `--dicvir`, `--catalog-layout` and
`--catalog-fields`; an explicit flag still wins. Both packs carry their own
licensed binaries. Do not publish this repository outside the parties already
licensed for those installations. A machine that wants its own installation
instead points `$E3D210_SCHEMA_DIR` or `$E3D31_SCHEMA_DIR` at it — the pack
looks in that variable first, then its own `binaries\`, then a
`binaries.root` its manifest records, and names every place it looked when it
finds none.
`schema/README.md` covers adding a release; `e3d_io::SchemaPack` is the same
resolution from Rust.

## Building

`Cargo.toml` names `e3d-attlib` by path, `../e3d-attlib`, so the two
repositories are checked out side by side:

```powershell
git clone <host>/e3d-attlib
git clone <host>/e3d-io
cd e3d-io; cargo test
```

The path dependency rather than a git or registry one is deliberate: seven
downstream crates — `e3d-model`, its five verify variants, and `gen-model` —
depend on `e3d-io` and `e3d-attlib` both, both by path. If this crate reached
`e3d-attlib` any other way each of them would build two `e3d-attlib` crates in
one graph, and the `AttlibData` they hand to an `e3d-io` call would be a
different type from the one it takes.

Rows are labelled `catalog_and_descriptor`, `catalog_only`, or
`descriptor_only`. Only the descriptor side supplies physical storage
locations, so internal nouns such as `DBMDAT` remain binary-authoritative even
when they are absent from the public E3D type enumeration.

## Dependencies

- `thiserror = "2"` — error enums
- `nom = "8"` — binary parsing
- `e3d-attlib` (sibling path crate) — attribute schema & system-name lookup

## Quick start

```rust
use e3d_io::ReadOnlyEngine;
use e3d_io::record::template::TemplateProvider;
use e3d_io::refno::RefNo;
use e3d_attlib::AttlibData;

const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;

let mut engine = ReadOnlyEngine::open(r"path\to\ams8000_0001")?;
let attlib = AttlibData::parse_file(r"path\to\attlib.dat")?;
let mut provider = TemplateProvider::open(r"path\to\desvir.dat", Some(DESIGN_TEMPLATE_TYPE))?;

let extraction = engine.extract_element_with_descriptors(
    RefNo::new(24384, 23257),  // RefNo of the element to decode
    &attlib,
    &mut provider,
)?;

println!("noun = {:?}", extraction.noun_name);  // e.g. Some("BRAN")
for attribute in &extraction.attributes {
    println!("  {:<8} {:?} {:?}", attribute.name, attribute.status, attribute.value);
}
```

Every descriptor produces a row, so nothing is dropped silently. On
`BRAN 24384/23257` in the AVEVA Marine sample the 98 rows come out as 41
`Decoded`, 8 `DecodedExplicit`, 4 `TemplateDefault` and 45 `NonImplicit`.

## Write path

`e3d_io::write` writes an E3D 3.1 database the way `core.dll`'s `SAVEWORK`
does (`docs/evidence/2026-09-20-core-dll-dabacon-write-path-algorithms.md`;
plan and record of every step in `docs/plans/2026-09-20-dabacon-write-plan.md`
§7): pages already on disk are never modified, the new records, tree pages
and session page are appended after the newest session's last page, and page
0's `LSPGNO` — written last — is the only commit point. A crash before that
write leaves a file every reader still sees as the previous session; a page 0
that moved while the write was staged is refused (`WriteError::Moved`).

```rust
use e3d_io::record::template::TemplateProvider;
use e3d_io::write::{write_element_session_with, ElementWrite, RecordDraft, SessionRequest, WriteOptions};

let mut templates = TemplateProvider::open(r"schema\e3d31\binaries\desvir.dat", Some(0x000B_0692))?;
let zone_template = templates.template_for(zone_noun)?.expect("desvir lists ZONE").clone();

// Re-emit an existing record with one attribute changed, by its descriptor.
let record = engine.find_element(zone)?.expect("indexed");
let mut draft = RecordDraft::from_record_bytes(&record.raw_bytes)?;
draft.set_attribute(&zone_template, db1_hash("DESC"), &DescriptorValue::Text("pump bay".into()), engine.text_format())?;

let report = write_element_session_with(
    path,
    &SessionRequest { user: "e3dio", comment: "one DESC", clock: None },
    &[ElementWrite::Update(draft)],
    &[],                                        // NAME table edits
    WriteOptions { templates: Some(&mut templates) },
)?;
println!("session {} on {} new pages", report.written.sesno, report.appended_pages.len());
```

- **Elements**: `ElementWrite::{Insert, Update, Delete}` over a `RecordDraft` — the inverse of `read_record` + `ParsedElement`. `RecordDraft::set_attribute(&template, hash, &DescriptorValue, text_format)` places a value where the noun's descriptor puts the attribute, in the shape the read side decodes (`write::attr`); every decoded attribute of the 7 825 elements of three sample databases writes back word for word (`tests/write_stage4_set_attribute.rs`). Members: `push_member` / `remove_member` / `move_member_after`. Records longer than a page split their tail blocks across pages; large values go out of the record as hunks (`set_int_array_hunk`, `set_text_hunk_ascii`, …).
- **Names**: `NamedEdit` keeps the `NAME` table (variable-key B+tree) in step; a name already taken is refused as conflict 561 before anything is written.
- **Tables, with the schema** (`WriteOptions.templates`): `TYPEX`, the type-16 and type-17 reverse-reference tables (`SPRE`, `CATR`, `HCREF`, `MDSYSF`, …, created when missing) and the primary-element session index under `CLAIM` follow every write the way `sub_5AB91B0` / `sub_5AB3620` / `sub_5B0C760` keep them. Without templates the tables are carried over untouched.
- **Merge** (`write_merge_session[_with]`): when the database moved on since the writer's base session, the change is rebased onto the latest session the way `DOPENW` does — 562 / 590 / 561 conflicts refuse the whole write, attribute-level and member-level (anchored add / remove / move) three-way reconciliation otherwise, whole-list replacement for reference-list nouns (`GROU`), an extract-flush mode where the committed side wins, and INDEX de-duplication when the merge converges.
- **Two phases**: `stage_element_session[_with]` writes everything but page 0 and returns a `StagedSession`; `commit` re-reads page 0, refuses if it moved, then writes it; `rollback` truncates the appended pages.
- **What E3D said**: `tools/e3d/write_gate.ps1` writes a box, a group of references, hunks of every kind and a string list into a scratch copy of the AMS sample project and reads them back through a real E3D 3.1 Design TTY session — fourteen questions, about ten seconds. Three rules a caller needs from that run: a template's count for a text attribute is its **byte** capacity (`DESC` is 480 bytes for E3D's 120-character limit), `UUIDIA` is set-once, and `RATM` / `UNIPAR` / `USRWEI` are not attributes E3D's dictionary answers for.
- **Not written**: `SPAMAP` (needs the geometry the spatial grid is computed from), the UDA tables (`UDATAB` / `UDAFTB` / `UUIDAR`), the extract model (attribute 17), page 1's bucket table, multi-extent files (refused), records in the compact layout (refused), non-ASCII text under a `FORMAT` other than 1 (refused until the escaped charset has an encoder), and `core.dll`'s write-mode sentinel lock — take the file out of E3D's write mode before writing.

## API surface (key types)

| Type / fn | Purpose |
|---|---|
| `ReadOnlyEngine::open(path)` | Open a DB file; auto-detects 4-byte-words `page_size` field |
| `ReadOnlyEngine::find_element(refno)` | B+tree search across all index roots |
| `ReadOnlyEngine::extract_element_with_descriptors(refno, attlib, provider)` | One row per descriptor, with status and value |
| `ReadOnlyEngine::diff_sessions(from, to)` / `diff_steps(from, to)` | Net change between two sessions, or one retained step at a time; `changed_elements(&diff)` streams the records behind it |
| `session::ElementDelta::of(&changed)` | Owner / members / explicit / implicit-word differences of one changed element; `with_attribute_rows` adds the descriptor-decoded rows |
| `session::SessionSelector::{Latest, Exact, AtOrBefore}` | How `open_at` and `diff_sessions` name a session |
| `record::template::TemplateProvider::open(path, expected_type)` | Cached `*vir.dat` reader; loads descriptors and defaults per noun |
| `record::descriptor::StorageShape::of(code)` | What one element of a storage type looks like |
| `record::element::ParsedElement::parse(raw_bytes)` | Element record header (impl_len/refno/noun_hash/owner) |
| `record::atnlog::decode_attribute_by_hash(parsed, attlib, hash, follow_syno, header_words)` | Record-level attribute decoder |
| `record::atnlog::AttrValue` | Decoded value enum (15 variants: 9 scalars + 5 arrays + Raw) |
| `index::IndexPageView::parse(page_bytes)` | B+tree index page parser |
| `meta::DbDescriptor`, `meta::FileInfo` | Database metadata |
| `write::write_element_session[_with]` / `stage_element_session[_with]` | Append a session of element inserts / updates / deletes (+ `NamedEdit`s); the staged form stops before page 0 |
| `write::write_merge_session[_with]` | The same, rebased onto a database that moved on since the writer's base session |
| `write::RecordDraft` | An element record to write: `from_record_bytes`, `set_attribute`, members, hunks |
| `write::set_reference` / `set_reference_array` / `table_keys` | Type-16 / type-17 references and the keyed-table keys a record files |
| `write::WriteOptions { templates }` / `MergeOptions` | Hand the schema over and the tables follow the write |

## Byte-order convention

Supported E3D 2.1/3.1 database files use **big-endian** byte order (Fortran
legacy). `f64` values use **Fortran word-swap** storage: two big-endian 32-bit
words in `(lo, hi)` order rather than standard IEEE 754 BE. See
`record::attrs::read_f64` for the implementation.

## License

(Add as needed.)
