# E3D database version evidence

## Result

No investigated database field is a durable E3D release discriminator.
Automatic SchemaPack selection is disabled; callers must choose `e3d210` or
`e3d31` explicitly.

The read-only diagnostics now expose three distinct header meanings without
turning any of them into a release:

- page-0 `CRINFO` is the database **creator** banner;
- page-0 `TMPLVN` is the database-template compatibility version;
- the selected session's DAB tag is the database engine that wrote that
  **session**.

`COMM.VERS`, `WORL.UPGDNO`, `DBBLBX`, session comments, directory names and
filesystem metadata likewise remain diagnostics only.

## IDA evidence (`Everything3D3.1/core.dll`)

- `DB_DB::upgdno()` at `0x5914DF0` gets `ATT_UPGDNO` from the database World.
- `DB_DB::upgdno(int)` at `0x5914EB0` writes that attribute.
- `DB_UpgradeCtrl::applicableVersion()` at `0x5A308F0` compares `UPGDNO` with
  registered database-migration numbers.
- `ADM_DBModification::setDbUpgradeNumber()` at `0x5A85B10` writes `UPGDNO`
  and saves work with the comment `Software upgrade number established`.
- `DB_Communication::updateVersion()` at `0x5A0CD50` writes
  `VersionInfo::version()` to `ATT_VERS` on a `COMM` element.
- `DB_Communication::clearComm(DB_Element&, int)` at `0x5A0A480` writes an
  empty string to `ATT_VERS` while clearing the communication entry. Its callers
  include `clearAllComm`, the no-argument `clearComm`, and `expunge`.
- `DB_DB::setMetaData()` at `0x5912B60` writes
  `DBMDAT/DBMVAL/DBMEXT`, then saves with the comment
  `Saving Meta Data.`. The comment carries no release value.

The field hashes are `UPGDNO=0x0D4F20A4` and `VERS=0x000E04A9`.

## Header evidence from IDA

### Page 0: creator and template compatibility

The low-level Dabacon layer in the E3D 2.1 binary retains its internal routine
names, making the field dispatch explicit:

- `db2_create_master` at `0x1062B510` initializes the 128-word descriptor
  prefix. It writes `TMPLNO` at word 8, a signed `TMPLVN` at word 9,
  `CRINFO`'s word count at word 16 and its packed ASCII at word 17 onward.
- `db2_get_p0_int_att` at `0x10626A40` maps `TMPLVN` to word 9.
- `db2_get_p0_arr_att` at `0x10627170` maps `CRINFO` to the
  count-plus-payload region at words 16/17.
- `db2_open_db` at `0x1062F8F0` takes `abs(TMPLVN)` and compares it with
  word 3 of the registered `*vir.dat` template header. A mismatch reports
  `db verno ... template verno ...`.
- The E3D 3.1 counterpart at `0x5AF9EF0` performs the same comparison against
  the same page-0 words.
- `FORMAT` at word 85 (`+0x154`) controls conversion of packed text fields.
  It is observed as both 0 and 1 inside the same release and is not a release
  number.

`CRINFO` can contain a product and build string, but the write occurs when the
database is created. Compaction preserves it, and ordinary later sessions do
not replace it. It therefore answers “which tool created this database?”, not
“which E3D release should open it now?”.

### Session trailer: DAB engine last writer

The database engine itself has a separate build banner:

- E3D 2.1 `DBVRSN` at `0x1065A110` copies
  `DAB2_28_19_ntopt`.
- E3D 3.1 `DBVRSN` at `0x5B24010` copies
  `DAB2_35_23opt`.
- E3D 2.1 `db1_init` at `0x106339F0` derives page-0 word 1 from the first
  digit after `DAB`. Both binaries therefore write the same page-0 major
  value, 2.
- `db5_create_session_page` at `0x105E6A20` copies eight bytes beginning
  after the `DAB` prefix into session words 503/504 (`+0x7DC..+0x7E3`).

The persisted tags for these two binaries are consequently `2_28_19_` and
`2_35_23o`. An older retained E3D 3.1 session also contains `2_35_16o`,
confirming that this is a concrete DAB engine build tag rather than an abstract
E3D release field.

The tag is better last-writer evidence than a filesystem path, but it still
cannot select a SchemaPack:

- a 3.1 project may contain databases that have not gained a 3.1-written
  session and still carry `2_27_*` or `2_28_*`;
- opening an older database with a newer client changes the next session's tag
  without changing who originally created the database;
- other AVEVA products may ship the same DAB engine build.

### `DBBLBX`: persistent but outside the header

E3D 3.1 `DB_InfoProvider::PutMetaData()` at `0x5CB6780` writes a
`/*METABLOB HASH` `DBMETA` root with a `DBMDAT` child. Its
`DBMEXT` is `db1_hash("DBBLBX") = 0x14A2AE6B`; `DBMVAL` is an opaque
service-provided signature. `DB_MDB::closeMDB()` at `0x59F2FD0` reaches the
monitor path that can refresh this value.

E3D 2.1 contains neither `DBBLBX` nor the metadata provider methods. Presence
is therefore useful one-way evidence that a metadata-capable 3.1-era client
wrote the database, but absence is common in valid 3.1 projects. The payload
is not a plaintext product version, and the record is not part of page 0 or
page 1.

## Real-file cross-check

Files inspected:

- `D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\amssys`
- `D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\amssys`
- the corresponding `ams8000_0001` and `amscom` files

Both `amssys` World records store `UPGDNO=12010401`. Both
`ams8000_0001` World records store `UPGDNO=12010104`. Those values therefore
cannot distinguish the releases.

The E3D 3.1 `amscom` current index contains both `COMM.VERS=3.1.9.0` and
`COMM.VERS=1.3.13.0` on different communication entries. The E3D 2.1 `amscom`
has no current value. These are client/module versions with a session
lifecycle, not one database-format version.

Copying the 3.1 `amscom` under a directory named `E3D2.1` preserves the same
diagnostic strings and still assigns no release, confirming that the path
spelling does not participate.

### Release-paired header cross-check

Eleven matching databases under the installed E3D 2.1 and 3.1
`AvevaMarineSample/ams000` trees were compared word for word.

- Six page-0 descriptors are completely identical.
- Every pair has the same `TMPLNO`, `TMPLVN` and `FORMAT`. Observed
  `TMPLVN` is -1 for SYST/DESI/CATA/MISC/NSEQ and -3 for COMM.
- The remaining page-0 differences are dynamic state: latest-session
  pointers, RefNo allocation state and compact/expunge/header-change counts.
- Paired `ams8000_0001` files have the same `CRINFO`, including
  `AVEVA Administration Admin Mk1.4.0.3[A1403-2]`, even though one copy now
  lives under the E3D 3.1 project tree.
- Paired system databases likewise retain old creator banners such as
  `AVEVA Marine Project Creator Mk12.1.SP4.0[4074]`.

Latest-session tags show last-writer rather than path semantics:

- `amssys` and `ams8000_0001` currently read `2_28_19_` in the 2.1 copy and
  `2_35_23o` in the 3.1 copy because the latter files gained sessions from the
  newer engine.
- `ams7000_0001` reads `2_28_19_` in both trees.
- `ams7351_0001` reads `2_28_18_` in both trees.
- paired COMM and MISC databases retain still older `2_19_d0_` and
  `2_27_2_n` tags.

Across eight installed E3D 3.1 system databases, only three latest sessions
carry `2_35_23o`; the other five retain older DAB tags. Thus a `2_35_*` tag is
strong positive last-writer evidence, while its absence says nothing about the
configured E3D release.

For `DBBLBX`, the eleven-file 3.1 sample contains the marker in only
`amssys` and `ams8000_0001` (2/11); the paired 2.1 files contain none.
Across the eight 3.1 system databases only one contains it. This makes it a
weaker-coverage one-way diagnostic than the per-session DAB tag.

## Additional E3D 3.1 project sample

The production `e3d-descriptor header` diagnostic path was run against seven
more current Communication databases:

- `AvevaPlantSample/aps000`: current values include `3.1.9.0`.
- `TEST/TES000`: current values include `3.1.9.0`.
- `YCYK-E3D/acw000`: current values include `3.1.9.0`.
- `E3D3.1-slyk-project/YCYK-E3D/acw000`: current values include `3.1.8.1`.
- `AvevaCatalogue/acp000`: no current `VERS`; its E3D 2.1 and E3D 3.1
  `acpcom` files are byte-for-byte identical (same SHA-256 and 2011 mtime).
- `SCB/scb000`: current `VERS` values are empty. Unreferenced old record bytes
  contain `1.8.3.0`, which is not current evidence.
- `ZDJ/ZDJ000`: current `VERS` is empty; unreferenced old record bytes likewise
  contain `1.8.3.0`.

All three idle Communication databases still report only their original
session 1 (2011, 2017 and 2013 respectively). SCB and ZDJ have newer filesystem
mtimes but unchanged session metadata, consistent with communication entries
being updated and cleared rather than a durable schema upgrade being recorded.

## Read-only API and startup logging

`e3d-io` exposes these facts without producing a release conclusion:

- `database_header_diagnostics(path)` opens the newest session and returns
  `DatabaseHeaderDiagnostics`;
- `ReadOnlyEngine::header_diagnostics()` returns diagnostics for the session
  selected by that engine, including historical `open_at(...)` sessions;
- `project_dabacon_tags(project_root)` discovers every primary extensionless
  database and returns latest tags, every retained `(session, tag)`, per-tag
  session/database counts, untagged counts and per-file read issues;
- `DbDescriptor` exposes `template_version`, `creation_info` and
  `header_format`;
- `SessionPage` exposes `dabacon_version_tag`;
- `e3d-descriptor header` emits `header_creation_info`,
  `header_template_version`, `header_format`,
  `selected_session_number` and
  `selected_session_dabacon_version_tag`.
- `e3d-descriptor project-dab-tags --project-root PATH` emits the complete
  deterministic project report as JSON, including every database and retained
  session rather than the bounded health summary.

`gen-model-refactor` includes the same facts in its default schema-load and
main-project SYST startup diagnostics under these stable keys:

- `header_crinfo`
- `header_tmplno`
- `header_tmplvn`
- `header_format`
- `header_session_page`
- `header_session_number`
- `session_dab_tag`

A read failure is emitted as `header_diagnostics_error=...`. Header diagnostics
are deliberately non-blocking: they do not add a Meta blocker, skip a SYST
file or alter the configured/default SchemaPack.

The project aggregation follows the same boundary. A database whose descriptor
prefix is valid but whose session chain cannot be read is retained in
`ProjectDabaconTagReport::issues`; it does not hide the other databases.
Directory traversal failure remains an error because the candidate set would
be incomplete. Secondary extents and extension-bearing backup copies are not
counted as separate logical databases. The report is not a cross-file
transaction: each database is frozen at the latest-session pointer observed
when that file is opened.

Running the CLI against the current E3D 3.1 Marine sample found 449 readable
primary databases with zero issues and 2,888 tagged retained sessions. Latest
tags span nine DAB builds; the largest groups are `2_28_18_` (423 databases),
`2_28_19_` (11) and `2_35_23o` (6). This spread is further evidence that a
project directory name cannot be converted into one database-engine tag.

`gen-model-refactor` records a pending snapshot before its health server starts,
then performs the file work on a detached blocking worker. The health server
does not wait for it: `/health.dabacon_tags` is `pending` while it runs, then
exposes bounded latest/history counts and at most 20 per-file issues. This
diagnostic status does not alter the top-level health status.

Regression coverage:

- `e3d-io/tests/header_diagnostics_real.rs` pairs the installed 2.1 and 3.1
  databases and pins creator/template/last-writer semantics, including a 3.1
  tree database that retains an old `2_28_19_` tag.
- `e3d-io/tests/project_dabacon_tags.rs` pins deterministic discovery,
  latest/historical aggregation, untagged sessions, per-file issues and a
  full installed-project census.
- `gen-model-refactor` tests
  `schema_loading_diagnostics_name_success_and_full_failure_context`,
  `startup_sys_scan_reports_version_evidence_without_blocking` and
  `startup_header_summary_names_creator_template_and_selected_session_writer`
  pin the startup keys and their non-blocking behavior.
