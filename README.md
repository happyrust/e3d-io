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
- **String lists and hunks**: a storage-type-19 value is a `TextList` (inline or in its slot), and a value stored out of the record in a hunk (types 20–23) is read through its directory entry — `ReadOnlyEngine::extract_element_with_descriptors` and the `DbElement` facade resolve it before decoding (`record::hunk::resolve_hunks`), so `DBMDAT.DBMVAL`'s 5 709 ints arrive as the array, not as five address words. The pure `extract_parsed_element_with_descriptors` has no pages and leaves the entry as `RawWords`.
- **Header and session words**: every word `core.dll` serves from page 0 and from a session page is tabulated against what this crate reads it as, in `meta::DbDescriptor::parse` and `session::at`. The extract model (`EXTID` / `OWNEXT` / `NOWNEX` / `INCON` / `ACCMOD` / `SUBTYP`) is tabulated but not read: no local database is an extract.
- **Write path** (E3D 3.1): `e3d_io::write` appends a session — new, changed and deleted elements, names, the `TYPEX` / reverse-reference / primary-element (`CLAIM`) tables, three-way merge against a database that moved on — and commits it through page 0, never touching a page already on disk. A real E3D 3.1 Design reads what it writes (`docs/evidence/2026-09-21-e3d-tty-gate-write-path.md`). Details below.
- **Not yet**: ATGTDF dedup, the extract / claim model, session diffs of the NAME / TYPEX / reverse-reference tables; on the write side, the UDA tables, multi-extent files and page-1 bucket-table maintenance (the refno counter, S6b) (`docs/plans/2026-09-21-write-path-next-steps.md`).

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
§7): new records, tree pages and the session page are appended after the
newest session's last page. Transaction also raises the physical page-1
bucket counters in place, preserving entries and claim words, and syncs them
before writing page 0's `LSPGNO` last. Page 0 commits the session; page 1 has
no historical version. A failure between those writes may skip refnos while
leaving the previous session readable. A crash before that
write leaves a file every reader still sees as the previous session; a page 0
that moved while the write was staged is refused (`WriteError::Moved`).

Three steps: open a `Transaction`, change values by descriptor, commit.

```rust
use e3d_attlib::db1_hash;
use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::record::template::TemplateProvider;
use e3d_io::write::{SessionRequest, Transaction};

let mut templates = TemplateProvider::open(r"schema\e3d31\binaries\desvir.dat", Some(0x000B_0692))?;
let mut tx = Transaction::open(path, &mut templates)?;
let format = tx.text_format();               // the header's FORMAT, for text values

// A new BOX under an existing ZONE: the refno is minted in the ZONE's extent,
// the ZONE's member list gains it, and its NAME goes into the record and the
// NAME table together.
let the_box = tx.insert(zone, box_noun, Some("/E3DIO-BOX"), |draft, template| {
    draft.set_attribute(template, db1_hash("POS"), &DescriptorValue::RealArray(vec![1000.0, 2000.0, 3000.0]), format)?;
    draft.set_attribute(template, db1_hash("XLEN"), &DescriptorValue::Real(300.0), format)?;
    Ok(())
})?;

// One attribute of an existing element, by its descriptor.
tx.update(zone, |draft, template| {
    draft.set_attribute(template, db1_hash("DESC"), &DescriptorValue::Text("pump bay".into()), format)?;
    Ok(())
})?;

let report = tx.commit(&SessionRequest { user: "e3dio", comment: "one box, one DESC", clock: None })?;
println!("{the_box}: session {} on {} new pages", report.written.sesno, report.appended_pages.len());
```

The primitive form underneath — `write_element_session_with(path, &request, &[ElementWrite], &[NamedEdit], WriteOptions { templates })` over `RecordDraft`s — is still public for a caller that mints refnos and threads `NAME` edits itself; the copy-on-write mechanics of the trees (`index_cow`, `keyed_cow`, `claim_cow`, `named_cow`, the claim plan) are crate-private since 0.3.0, reachable only through their error types on `WriteError`.

- **Elements**: `ElementWrite::{Insert, Update, Delete}` over a `RecordDraft` — the inverse of `read_record` + `ParsedElement`. `RecordDraft::set_attribute(&template, hash, &DescriptorValue, text_format)` places a value where the noun's descriptor puts the attribute, in the shape the read side decodes (`write::attr`); every decoded attribute of the 7 825 elements of three sample databases writes back word for word (`tests/write_stage4_set_attribute.rs`). Members: `push_member` / `remove_member` / `move_member_after`. Records longer than a page split their tail blocks across pages, and the header's two block counts saturate at `0x3FFF` (`write::FORMAT_COUNT_MAX`) as `core.dll`'s do — E3D loads exactly the count stated unless it reads `0x3FFF`, then measures the chain, dropping trailing zero words, so a saturated block that ends in a zero word is refused (`WriteError::SaturatedBlockEndsInZero`); large values go out of the record as hunks (`set_int_array_hunk`, `set_text_hunk`, …). Text is stored the way the header's `FORMAT` says (`record::text::encode_as`): UTF-8 as it is under `FORMAT 1`, the project code page (GBK) in `&~ … &` runs under any other — the form `core.dll`'s own converter writes, refusing what it refuses — for attributes, string lists, hunks, the session's `USER` / `DESC` and the `NAME` table key alike; E3D 3.1 reads back a Chinese `NAME`, `DESC` and session comment (`tools/e3d/write_gate.ps1` Q7 / Q8).
- **Names**: `NamedEdit` keeps the `NAME` table (variable-key B+tree) in step; a name already taken is refused as conflict 561 before anything is written.
- **Tables, with the schema** (`WriteOptions.templates`): `TYPEX`, the type-16 and type-17 reverse-reference tables (`SPRE`, `CATR`, `HCREF`, `MDSYSF`, …, created when missing) and the primary-element session index under `CLAIM` follow every write the way `sub_5AB91B0` / `sub_5AB3620` / `sub_5B0C760` keep them. Without templates the tables are carried over untouched.
- **Merge** (`write_merge_session[_with]`): when the database moved on since the writer's base session, the change is rebased onto the latest session the way `DOPENW` does — 562 / 590 / 561 conflicts refuse the whole write, attribute-level and member-level (anchored add / remove / move) three-way reconciliation otherwise, whole-list replacement for reference-list nouns (`GROU`), an extract-flush mode where the committed side wins, `MergeOptions.on_clash = OnClash::Refuse` to have a tie both sides changed refused (`WriteError::MergeClashes`, each `MergeClash` naming the element and the attribute / owner / list) rather than broken, and INDEX de-duplication when the merge converges; `stage_merge_session_with` is its two-phase form.
- **Transaction facade** (`write::Transaction`): `open(path, &mut templates)` → `insert(owner, noun, name, build)` / `set(refno, "DESC", "反应堆厂房")` / `update(refno, edit)` / `rename(refno, name)` / `delete(refno)` / `move_to(refno, new_owner, MemberPosition)` / `reorder(refno, MemberPosition)` / `add_reference_member(group, member, MemberPosition)` / `remove_reference_member(group, member)` → `commit(request)`. `set` takes an attribute **by name from its text** — `"POS"` `"1000 2000 3000"`, `"PURP"` `"PIPE"`, `"BUIL"` `"true"`, `"ROKEYS"` `"K1|K2"`, `"CREF"` `"/PUMP-1"` or `"=24384/26484"`, anything `"unset"` (`write::value` has the table; the descriptor decides the shape) — and, with `with_dictionary(&Dictionary)` (`write::dictionary`, from the pack's `catalog/noun_attr_fields.json`), holds it to E3D's own limits before it reaches the record: an attribute the dictionary lacks is refused as E3D refuses it (`RATM`), a text to `SIZE` **characters** as well as the template's bytes (`DESC` 120 / 480), an array to `SIZE` elements, `UUIDIA` to one setting, `USRWEI` to none. It mints the refno (in the owner's extent, the greater of base index max + 1 and the confirmed page-1 NextAvailable counter — `core.dll`'s page-1 counter, `docs/evidence/2026-09-22-core-dll-refno-allocation.md`), starts a new record the way `4CEIL` does (chunk-1 defaults, the chunk-2 explicit prefix seeded, `TYPEX` appended at staging — `RecordDraft::from_template`), keeps the owner's member list and the `NAME` table in step — a move is E3D's `INCLUDE` (owner word, both lists, both owners' claims), a reorder its `REORDER` (one list, positions `Last` / `First` / `After(m)` / `Before(m)`), and a `GROU`'s reference list is edited only by name — and hands the batch to `stage_element_session_with`. Its record and appended pages match the primitive path; Transaction additionally maintains SPAMAP and page-1 counters (`tests/write_stage6_transaction.rs`). `open_at(path, SessionSelector, &mut templates)` opens at any retained session as the **base**; when the file has moved on by commit time (E3D, or another writer, saved meanwhile) the batch is merged onto the newest session as `SAVEWORK` merges it — 561 / 562 / 590 refused as `TransactionError::Conflicts`, an inserted refno the other side took re-minted with every list, key and reference following (`commit_reporting` → `TransactionCommit::reminted`), and the same attribute changed on both sides **refused** by default (`OnClash::Refuse`, `TransactionError::Clashes` naming each) or taken this batch's way with `with_on_clash(OnClash::WriterWins)`; a writer that gets in between the staging and the page-0 write is merged over once (`tests/write_stage6_merge.rs`).
- **Batch CLI** (`e3d-write`, `src/bin/e3d-write.rs`): `e3d-write apply --db <file> --ops ops.json [--schema schema\e3d31] [--base N] [--on-clash refuse|writer-wins] [--spatial require|skip|off] [--dry-run] [--no-dictionary] [--user U --comment C] [--report report.json]` applies one JSON file of instructions as one session through exactly the facade calls above — `{"op": "insert", "owner": "/1RX03-MSUP", "noun": "BOX", "name": "/E3DIO-BOX", "id": "box", "attrs": {"POS": "1000 2000 3000", "XLEN": "300"}}`, then `set` (`update`) / `rename` / `delete` / `move` / `reorder` / `reference` / `unreference` naming elements as `=w0/w1`, `/NAME` (`/*` the WORL) or `@id` (an insert earlier in the file), positions as `"last"` / `"first"` / `{"after": e}` / `{"before": e}`, a value as its text or `{"value": "…", "hunk": true}` to store it out of the record (`Transaction::set_hunk`). The dictionary is on unless `--no-dictionary`; `--base` opens at an older session and the commit is the merge above; `--dry-run` runs the batch through the staging — every check, the merge's scans included — and rolls it back, byte-identical; the first refused instruction stops the run before anything is written (`op #n (...)`: the reason), exit 1; `--report` writes the instructions with the refnos they resolved to and were given / written as, and the commit (`merged_onto`, `reminted`, `appended_pages`) or the error. `tests/write_stage6_cli.rs` runs the binary.
- **Two phases**: `stage_element_session[_with]` writes everything but page 0 and returns a `StagedSession`; `commit` re-reads page 0, refuses if it moved, then writes it; `rollback` truncates the appended pages.
- **Write-mode lock**: staging takes `core.dll`'s sentinel lock (an exclusive `LockFileEx` on the byte at `0x7FFFFFF6FFFFFFFF`, held until commit/rollback), so a file a real E3D is writing is refused at once as `WriteError::Locked`; on non-Windows the lock is a no-op and the page-0 re-read (`WriteError::Moved`) is the guard.
- **What E3D said**: `tools/e3d/write_gate.ps1` writes — with `e3d-write apply`, one instruction file per session from `tools/e3d/write_gate_ops/` — a box, a group of references, hunks of every kind, a string list, three boxes that are then moved / reordered / referenced, a ZONE of 8 200 boxes, and an EQUI and a FIXING whose every value was typed as text and set by attribute name into a scratch copy of the AMS sample project and reads them back through a real E3D 3.1 Design TTY session — first Q0 (six instruction files that must be refused with a reason — a 121-character `DESC`, a four-element `POS`, `RATM`, a misspelt attribute, a refno nobody has, an owner deleted earlier in the file — and a `--dry-run`, with the database byte-identical after all of them), then twenty-three questions (a Chinese `NAME`, `DESC` and session comment among them; `Q OWNER` and `Q MEM` after `move_to` / `reorder`; all 8 200 members listed, which takes the format word's member count written saturated at `0x3FFF` the way `core.dll` writes it; `Q DESC` / `Q POS` / `Q ROKEYS` / `Q CREF` printing back exactly the strings `Transaction::set` was given) — then has E3D itself `NEW` an EQUI and a BOX under the first ZONE and `SAVEWORK`, commits a transaction opened at the session before that as a merge onto E3D's (`e3d-write apply --base`), and asks five more: E3D's BOX as E3D left it, the ZONE listing E3D's EQUI (first, where `NEW` at the owner puts it) and the merged BOX (last, after its anchor), the merged BOX's name / owner / `XLEN` / `POS`, the other ZONE's `FUNC`, `Q SESSION` = the merge. The complete gate now has 39 questions: Q0, the existing 28, Q13/Q13a–h (spatial search, move, resize, delete, SAVEWORK and world coordinates), and Q14 (allocation floor). The latest archived complete TTY run passed all 39 in 211.602 s. A second gate, `tools/e3d/alternate_write_gate.ps1`, has E3D and `e3d-write` write to one ZONE **in turn** for five rounds — each E3D `SAVEWORK` opening on e3d-io's latest session and each `e3d-write` appending on E3D's — then a read-only E3D session lists all ten EQUIs (both sides) by name and the session tail `BASESN` chain is checked contiguous and unbroken (`e3d-descriptor sessions`); all rounds pass; Q15 then checks that E3D NEW does not reuse numbers issued and deleted by e3d-io. The archived five-round gate plus Q15 passed in 82.034 s. Three rules a caller needs from that run: a template's count for a text attribute is its **byte** capacity (`DESC` is 480 bytes for E3D's 120-character limit), `UUIDIA` is set-once, and `RATM` / `UNIPAR` / `USRWEI` are not attributes E3D's dictionary answers for.
- **SPAMAP**: Transaction maintains spatial entries under `SpatialPolicy::Require` (default), `Skip`, or `Off`. Caller AABBs take precedence, then built-in BOX/CYLI world bounds and confirmed member aggregation. Require refuses missing bounds; Skip removes the affected old entries and reports them; Off retains the primitive write behavior. Primitive writes do not maintain SPAMAP. CLI `apply --spatial require|skip|off` uses the same default; `insert`, `set`, and `move` may supply `aabb: [xmin,ymin,zmin,xmax,ymax,zmax]`, six finite JSON numbers in world millimetres with each min ≤ max. Dry-run reports the planned spatial differences before rollback.
- **Allocation**: `allocation_basis()` reports the pinned session/root, page-1 counter use, and first number per extent. `e3d-descriptor next-word1 --db FILE --extent WORD0 [--session N]` independently diagnoses the expected number. `cargo run --release --example write_scale_probe -- --open-only DB --runs 3` measures open and first insertion on a temporary, warm copy and checks source/copy hashes. Group 12 mixed staging measurements passed: 100k/10k ratios 7.23 (Off) and 8.72 (Require); Require 10k stage plus commit took 231.74 ms. See the staging acceptance evidence below.
- **Page 1 bucket table**: Transaction raises each listed extent counter above the pinned index and final inserted refnos, including reminted numbers. It preserves entry order and third-word claims; absent or malformed tables are left untouched. Commit writes and syncs page 1 before page 0. Primitive APIs maintain neither page 1 nor SPAMAP; callers using them own that responsibility. See [ADR 0003](docs/adr/0003-page-1-in-place-bucket-write.md) for the crash window.
- **Not written**: the UDA tables (`UDATAB` / `UDAFTB` / `UUIDAR`), the extract model (attribute 17), multi-extent files (refused), records in the compact layout (refused), text with no stored form under the file's `FORMAT` (refused, as `core.dll` refuses it: a GBK code with a byte below `0xA0` such as `–`, anything outside the BMP).

## DRAFT pictures

A DRAFT sheet's view graphics are not in the PADD database: DRAW writes them
with core.dll's Wigwam kernel to `<proj>pic\NN\M<w0>-<w1>-<EXFI>-<PVNO>`
(`w0/w1` the sheet's refno, `EXFI` its extract database, `PVNO` its picture
version, `NN = w1 mod 32`). `e3d_io::picture` reads those files the way
`sub_54DC674` op 9 loads them and encodes every item back to the bytes it
came from (format: `docs/evidence/2026-09-28-gw-picture-format.md`; plan:
`docs/plans/2026-09-24-gw-picture-read-plan.md`).

```rust
use e3d_io::picture::{locate, thumbnail, PaddProject, Payload, PictureFile, Transform};
use e3d_io::SchemaPack;

// From the database: where the sheet's picture is, and its stored thumbnail.
let pack = SchemaPack::open(r"schema\e3d31")?;
let project = PaddProject::open(&pack, r"D:\AVEVA\Projects\E3D3.1\AvevaPlantSample\aps000")?;
let sheet = project.find_named("/APS-0001/1")?.remove(0);
let where_ = locate(std::path::Path::new(r"...\apspic"), &sheet)?.expect("PVNO > 0");
let png = thumbnail(&sheet)?.expect("a displayed sheet has one").png;   // 128 × 128 PNG bytes

// The file: header, name, one picture, its segment tree, groups, primitives.
let file = PictureFile::open(&where_.path)?;
let picture = &file.pictures()[0];
for (index, depth) in picture.walk() {                    // pre-order, transforms compose down
    let segment = picture.segment(index).unwrap();
    let to_sheet: Transform = picture.composed_transform(index);   // local → sheet mm
    for (group, prim) in segment.prims() {
        match &prim.payload {
            Payload::Points(points) => { /* MARKER / LINE / FILL / BOUNDARY */ }
            Payload::Conic { start, spans } => { /* arcs: spans[i].points(from, 24) */ }
            Payload::Text(text) => { /* text.text(), text.format() */ }
            _ => {}
        }
    }
}
```

- **Layout** (`word`, `file`, `directory`): 128-byte records of 8-byte words, each half big-endian; a 21-word header (15 version characters, then the name block, attribute table and picture table positions); the picture table's directory replays into a segment tree with the `rel | levels << 4` rule core.dll uses. Positions are trusted, the file is never scanned to its end (re-saved pictures keep their old tail).
- **Content** (`segment`, `prim`, `text`): segment flags / transform / user block (`Role` = the DRAFT noun the segment stands for — `SHEE`, `VIEW`, `DESI`, `GEOM` with its member table, `PPTS`, Marine `TEXP` notes), groups, and the ten primitive kinds field by field — `TextFormat` unpacks the mixed-radix attribute word to `FONT    11,RIGHT,CENTRE,HBODY`, TEXT2 runs rebuild their escaped source (`SHOP&/TEST`). Writer residue (a TEXT's last-word tail, an odd run's pad) is kept so `encode()` is the original bytes.
- **Semantics** (`geom`, `style`): `Transform` composition; arc spans `[mx, my, w, ex, ey]` with `M` the arc's midpoint and `w = bulge²`, expanded clockwise by θ = 4·atan(√w) (`Span::points`); the 365-entry DRAFT colour table replayed from core.dll `FWSCOL` + DruidNet `ColourTableManager` (`Colour::of(n)`, `> 365` is a layer id); pen names (`8004` = `THIN CHAIN`).
- **Gate** (`roundtrip`): `roundtrip(&file)` / `roundtrip_dir(pic_dir)` re-encode header, name, table, directory, every segment and group header and every primitive and compare bytes. Plant 881 + Marine 1 885 = 2 766 files, 24 873 647 primitives, 0 differing bytes; the ten kind counts and every listing line match the Python reference (`draft31\gwdump.py`, frozen as the oracle) character for character (`tests/picture_*.rs`).
- **CLI** `e3d-picture`: `info <file>` · `list <file> [--page] [--csv] [-o out]` · `roundtrip <file | dir>` · `locate --db <padd | dir> --pic-dir <dir> [--refno w0/w1 | --name N]` · `thumbnail --db … (--refno | --name) -o x.png`.
- **Not done here**: annotation / background redraw, SVG / PNG rendering, INSTANCE resolution across pictures, writing pictures E3D would load (the DRAFT semantics layer is planned as a separate crate, `docs/plans/2026-09-28-draft-module-next-steps-plan.md` P3 / P4).

## API surface (key types)

Phase-3 evidence: [database version](docs/evidence/2026-09-11-e3d-database-version.md), [write algorithms](docs/evidence/2026-09-20-core-dll-dabacon-write-path-algorithms.md), [TTY gate](docs/evidence/2026-09-21-e3d-tty-gate-write-path.md), [draft data flow](docs/evidence/2026-09-22-core-dll-record-draft-data-flow.md), [allocation](docs/evidence/2026-09-22-core-dll-refno-allocation.md), [text charset](docs/evidence/2026-09-22-core-dll-text-charset-write.md), [SPAMAP consequences](docs/evidence/2026-09-22-e3d-spamap-consequences.md), [baseline](docs/evidence/2026-10-01-write-path-phase-3-baseline.md), [SPAMAP encoding](docs/evidence/2026-10-03-spamap-encoding.md), [coverage](docs/evidence/2026-10-03-spamap-coverage.md), [census](docs/evidence/2026-10-03-spamap-census.md), [branch entries](docs/evidence/2026-10-03-spamap-branch-entries.md), [flags](docs/evidence/2026-10-03-spamap-flags.md), [recompute](docs/evidence/2026-10-03-spamap-recompute.md), [record item](docs/evidence/2026-10-03-spamap-record-item.md), [AABB](docs/evidence/2026-10-03-spamap-aabb.md), [member bounds](docs/evidence/2026-10-03-spamap-aabb-members.md), [Q13](docs/evidence/2026-10-04-q13-spatial-gate.md), [allocation counter/order](docs/evidence/2026-10-05-allocation-counter-index.md), [Golden migration](docs/evidence/2026-10-05-allocation-floor-goldens.md), [Q14/open acceptance](docs/evidence/2026-10-05-allocation-q14-gate.md), [staging baseline](docs/evidence/2026-10-05-staging-scale-baseline.md), [staging acceptance](docs/evidence/2026-10-05-staging-scale-acceptance.md), [S6b bucket evidence](docs/evidence/2026-10-05-s6b-bucket-evidence.md), [Q15 acceptance](docs/evidence/2026-10-06-s6b-q15-gate.md), and [GUI gate attempt](docs/evidence/2026-10-06-e3d-spatial-gui-attempt.md).

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
| `write::Transaction` | `open[_at]` / `insert` / `set` / `set_hunk` / `update` / `rename` / `delete` / `move_to` / `reorder` / `add_reference_member` / `commit[_reporting]`: mints refnos, keeps owner member lists and the `NAME` table in step, merges when the file moved on |
| `e3d-write apply` (bin) | One JSON file of instructions → one session through `Transaction`; `--dry-run`, `--base`, `--on-clash`, `--report` |
| `write::write_element_session[_with]` / `stage_element_session[_with]` | Append a session of element inserts / updates / deletes (+ `NamedEdit`s); the staged form stops before page 0 |
| `write::write_merge_session[_with]` | The same, rebased onto a database that moved on since the writer's base session |
| `write::RecordDraft` | An element record to write: `from_record_bytes`, `set_attribute`, members, hunks |
| `write::set_reference` / `set_reference_array` / `table_keys` | Type-16 / type-17 references and the keyed-table keys a record files |
| `write::WriteOptions { templates }` / `MergeOptions` | Hand the schema over and the tables follow the write |
| `picture::PictureFile::open(path)` | A DRAFT picture file: header, name, pictures → segment tree → groups → primitives |
| `picture::{Picture::walk, composed_transform}`, `Span::points`, `Colour::of`, `TextFormat::decode` | Tree order, local → sheet mm, arc expansion, the 365-colour table, text attribute words |
| `picture::{locate, thumbnail, PaddProject}` | From a PADD sheet to its picture file and its stored 128 × 128 PNG |
| `picture::{roundtrip, roundtrip_dir}` | Re-encode every decoded item and compare bytes — the corpus gate |
| `e3d-picture` (bin) | `info` / `list` / `roundtrip` / `locate` / `thumbnail` |

## Unified write regression

On Windows with PowerShell 7, run `pwsh -NoProfile -File tools/e3d/regression.ps1` to run Cargo tests, Clippy, documentation, offline scorer checks, and the three real E3D TTY gates in sequence. The alternate gate runs five rounds and Q15. Existing E3D installations, projects and drivers are required; override `-AMSProject`, `-TESProject`, `-GenModel`, `-L3Exe` and `-Format0Launcher` as needed.

`-OfflineOnly` runs only Cargo and offline checks; it does not certify E3D TTY acceptance. `-OutputRoot` defaults to `D:\e3dio-regression`; each run creates a unique directory with `results.json`, `summary.md`, `manifest.json`, stage output and redacted original gate evidence. Project database copies remain in scratch subdirectories and are excluded from the evidence manifest. Source database directories and Golden hashes are checked before and after the run. A failed stage stops later stages and returns a nonzero exit code while retaining evidence. GUI acceptance remains a separate manual gate.

To verify saved evidence without launching E3D, run `pwsh -NoProfile -File tools/e3d/regression_verify.ps1 -RunDirectory docs/evidence/assets/2026-10-07-unified-regression/full -RequireTTY`. Verification checks manifest paths, file lengths and hashes, required result/summary files, successful stage exit codes and logs, status consistency and source/Golden snapshots. Omit `-RequireTTY` to verify offline or failed records; a valid failed record is not a successful regression. This checks the saved record and does not rerun the gates or authenticate its provenance.

## IDA core layout evidence

The read-only core.dll investigation uses the pinned `ida-bridge` commit `1e192b2c9c598b4ffaf7f0ec69b14cfbb7d44d69`, IDA 9.2, and the wrapper `tools/ida/ib.ps1`. It analyzes copies when the source IDB is locked and records the source/copy hashes. Queries and the decision to keep the current hunk placement are documented in `docs/evidence/2026-10-07-core-layout-ida-bridge.md`; no IDB, public API, Golden, or gate script is changed.

## Byte-order convention

Supported E3D 2.1/3.1 database files use **big-endian** byte order (Fortran
legacy). `f64` values use **Fortran word-swap** storage: two big-endian 32-bit
words in `(lo, hi)` order rather than standard IEEE 754 BE. See
`record::attrs::read_f64` for the implementation.

## License

(Add as needed.)
