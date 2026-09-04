# e3d-io

Standalone read-only IO library for AVEVA E3D 3.1 (`Everything3D 3.1`) database files. Provides page-level access, B+tree index search, element-record decoding, and a high-level attribute resolver.

Reverse-engineered from `core.dll` via IDA Pro; all field offsets and constants in this crate cite IDA evidence in the parent IDA exports / docs (see [`pdms-io-fork-engine-v2/ida_exports/3.1/`](https://github.com/) and `docs/ida-3.1-{structures,attributes}.md` in the goals repo).

## Status

- **Read path**: page I/O + LRU cache + descriptor + session + B+tree index search + element record decoder.
- **Attribute decoding**: implicit and explicit scalar/array values, noun descriptors loaded directly from `desvir.dat`, report names loaded directly from `attlib.dat`, and a TTY `ATTLIST` reconciliation path for runtime-composed attributes.
- **Defaults**: an attribute with nothing stored falls back to the noun's own default record, read from chunks 1 and 2 of the same `*vir.dat` child as the descriptor — the values `core.dll` returns from `sub_5B03B90`.
- **High-level API**: `engine.extract_element_with_descriptors(refno, &attlib, &mut provider)` returns an `ElementExtraction` carrying one row per descriptor, each with an explicit status.
- **Session ranges**: `engine.diff_sessions(from, to)` (or `DbView::diff_sessions`) names every RefNo inserted, deleted or rewritten between two retained sessions, `changed_elements` streams the record on each side, and `session::ElementDelta` says which owner / members / explicit attributes / implicit words / descriptor rows differ. `diff_steps` gives the same range one retained session at a time. `SessionSelector::AtOrBefore(n)` snaps a watermark to the newest retained session `<= n`; a session `MERGE CHANGES` removed is otherwise an error naming the retained sessions either side of it. CLI: `e3d-descriptor sessions --db …` and `e3d-descriptor diff --db … --from N [--to N] [--content on] [--steps on]`.
- **Not yet**: ATGTDF dedup, multi-extent, writeback, session diffs of the NAME / TYPEX / reverse-reference tables.

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
makes the mix hard to express.

Every command accepts a pack in place of the individual path flags:

```powershell
e3d-descriptor describe --schema schema\e3d31 --noun BRAN
```

That fills `--desvir`, `--attlib`, `--dicvir`, `--catalog-layout` and
`--catalog-fields`; an explicit flag still wins. The 3.1 pack carries its own
copies of the 20 `*vir.dat` and `attlib.dat` in `schema\e3d31\binaries\`, and
they are committed, so a fresh clone runs the line above with nothing
installed. They are AVEVA installation files: do not publish this repository
outside the parties already licensed for that installation. A machine that
wants its own installation instead points `$E3D31_SCHEMA_DIR` at it — the pack
looks in that variable first, then `schema\e3d31\binaries\`, then a
`binaries.root` its manifest records, and names every place it looked when it
finds none.
`schema/README.md` covers adding a release; `e3d_io::SchemaPack` is the same
resolution from Rust.

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

## Byte-order convention

E3D 3.1 database files use **big-endian** byte order (Fortran legacy). `f64` values use **Fortran word-swap** storage: two big-endian 32-bit words in `(lo, hi)` order rather than standard IEEE 754 BE. See `record::attrs::read_f64` for the implementation.

## License

(Add as needed.)
