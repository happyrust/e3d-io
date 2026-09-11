# E3D database version evidence

## Result

No investigated database field is a durable E3D release discriminator.
Automatic SchemaPack selection is disabled; callers must choose `e3d210` or
`e3d31` explicitly.

`COMM.VERS`, `WORL.UPGDNO`, session comments, directory names and file metadata
remain available only as diagnostics.

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
