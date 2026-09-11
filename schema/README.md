# Schema packs

One directory per E3D release. Everything that is bound to a release lives
inside it, so a reader is given one pack and cannot silently combine two.

```text
schema/
  e3d210/
    manifest.json
    binaries/
  e3d31/
    manifest.json     release identity, magic constants, and the file map
    binaries/         desvir.dat, attlib.dat, dicvir.dat, … (committed, see below)
    catalog/          noun_layout.json, noun_attr_fields.json, catalog_manifest.json
    metadata/         compiled parser_metadata.v1.json (git-ignored)
```

## Why per-release directories

None of the three release-bound inputs states its own release. `*vir.dat`
declares a `template_type` — `DESI`, `CATA`, `DICT` — which says what kind of
database it describes, not which E3D wrote it; `attlib.dat` and the catalog
export say nothing at all.

So a `desvir.dat` from one release with an `attlib.dat` from another parses
without complaint. The descriptors decide the word offsets, so the result is
plausible numbers read out of the wrong words. Nothing in the read path can
notice. Keeping each release in its own directory is the cheap structural
answer: paths that come from one pack cannot come from two.

## Using a pack

```rust
use e3d_io::SchemaPack;

let pack = SchemaPack::open_version("schema", "e3d31")?;
let desvir = pack.template("DESI")?;
let attlib = pack.attlib()?;
```

From the CLI, `--schema schema/e3d31` (or `--schema-root schema --e3d-version
e3d31`) fills in `--desvir`, `--attlib`, `--dicvir`, `--catalog-layout` and
`--catalog-fields`. An explicit flag always wins over the pack, so a one-off
file needs no pack edit.

Commands that read element descriptors (`extract`, `extract-complete` and
`list-uda`) must name a schema pack, version, or explicit descriptor files:

```powershell
e3d-descriptor extract --schema schema\e3d31 `
  --db D:\...\ams8000_0001 --refno 24384/46
```

Rust callers use the same route:

```rust
let pack = SchemaPack::open_version("schema", configured_e3d_version)?;
```

`database_version_evidence` / `project_version_evidence` expose diagnostics
without assigning a release.

Both releases use tree pair `(7618377, 13387743)`, so that pair cannot select a
pack. `COMM.VERS` cannot select it either:
`DB_Communication::updateVersion()` writes the current client's version, and
`DB_Communication::clearComm()` clears the field on logout or expunge. One
database can contain several module versions while an idle one contains none.
`SchemaPack::detect_for_database` and `detect_for_project` therefore fail
loudly until a durable release field is proven.

`WORL.UPGDNO` cannot make this choice. IDA's `DB_DB::upgdno()` reads a database
migration number, and the measured 2.10/3.1 pairs are identical
(`amssys=12010401`, `ams8000_0001=12010104`). Likewise,
`Saving Meta Data.` is only the save comment used by
`DB_DB::setMetaData()` after writing `DBMDAT/DBMVAL/DBMEXT`. These fields,
paths, timestamps and file sizes can be reported diagnostically but are not
consulted for selection.

## Where the AVEVA binaries live

In the repository. Each supported pack's `binaries/` holds that release's
`*vir.dat` files and `attlib.dat` — a clone can use either supported release
with nothing installed and no manifest names an absolute path.

They are AVEVA installation files. Committing them trades redistribution for
a checkout that is honest about what it tested: while `binaries/` was
git-ignored every machine but one had the whole set absent at once, and a run
that read no schema file still reported a clean `cargo test`. Do not publish
this repository outside the parties already licensed for that installation.

`SchemaPack` still searches rather than assumes, because a second machine may
need to point at its own installation. It tries three places in order and the
error names all three when none has the file:

1. the directory named by the manifest's `binaries.root_env` environment
   variable — `E3D210_SCHEMA_DIR` for 2.10 and `E3D31_SCHEMA_DIR` for 3.1;
2. `<pack>/binaries/`;
3. the `binaries.root` the manifest records, if it records one.

The environment variable comes first because it is the only one of the three
that a second machine can set without editing a checked-in file. That also
means a stale `E3D31_SCHEMA_DIR` shadows a pack that carries its own copies:
unset it, or override the individual file with an explicit flag.

Filling a new pack's `binaries/` is a copy and nothing else — no renaming, no
subdirectories:

```powershell
Copy-Item "<install>\*vir.dat", "<install>\attlib.dat" schema\e3d31\binaries\
```

## Adding a release

1. `mkdir schema/<id>` and copy `e3d31/manifest.json`, then correct
   `version.id`, `version.display` and the file map.
2. Add an `E3dVersion` constant for it in `src/schema.rs` and list it in
   `KNOWN_VERSIONS`. A pack whose `version.id` has no constants is refused by
   name; it does not fall back to 3.1's.
3. Measure the tree identities — do not carry them over, and do not assume
   they uniquely identify a release. Automatic selection additionally needs a
   database field that is durable across login/logout, has one project meaning,
   and is validated against real projects. A session comment, executable
   literal, or live-client field is not enough.

E3D 2.10 was added after a full primary-extent census, not from the
`7618321` constant found in its `core.dll`: all 828 files under
`D:/AVEVA/Projects/E3D2.1` use `data_page_main = 7 618 377` and
`data_page_index = 13 387 743`, exactly the pair used by all 1002 files under
the installed E3D 3.1 project. No durable discriminator has yet been proven;
the pair remains a fail-loud compatibility check and release choice is
explicit.
