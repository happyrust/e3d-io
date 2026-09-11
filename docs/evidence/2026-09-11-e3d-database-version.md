# E3D database version evidence

## Result

Automatic schema selection must use the current `VERS` attributes on `COMM`
elements in the project's Communication database. It must not use a session
comment, a directory name, file metadata, or `WORL.UPGDNO`.

If no current `COMM.VERS` value names a measured release, the result is
`Unknown`. An explicit `e3d_version` remains the only override.

## IDA evidence (`Everything3D3.1/core.dll`)

- `DB_DB::upgdno()` at `0x5914DF0` gets `ATT_UPGDNO` from the database World.
- `DB_DB::upgdno(int)` at `0x5914EB0` writes that attribute.
- `DB_UpgradeCtrl::applicableVersion()` at `0x5A308F0` compares `UPGDNO` with
  registered database-migration numbers.
- `ADM_DBModification::setDbUpgradeNumber()` at `0x5A85B10` writes `UPGDNO`
  and saves work with the comment `Software upgrade number established`.
- `DB_Communication::updateVersion()` at `0x5A0CD50` writes
  `VersionInfo::version()` to `ATT_VERS` on a `COMM` element.
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

The E3D 3.1 `amscom` current index contains `COMM.VERS=3.1.9.0`. The E3D 2.1
`amscom` has no measured product-version value, so strict automatic detection
returns `Unknown` instead of inferring 2.10 from absence.

Copying the 3.1 `amscom` under a directory named `E3D2.1` leaves the result at
`e3d31`, confirming that the path spelling does not participate.

## Additional E3D 3.1 project sample

The production `e3d-descriptor header` path was run against seven more current
Communication databases:

- `AvevaPlantSample/aps000`: `COMM.VERS=3.1.9.0`, selected `e3d31`.
- `TEST/TES000`: `COMM.VERS=3.1.9.0`, selected `e3d31`.
- `YCYK-E3D/acw000`: `COMM.VERS=3.1.9.0`, selected `e3d31`.
- `E3D3.1-slyk-project/YCYK-E3D/acw000`: `COMM.VERS=3.1.8.1`, selected
  `e3d31`.
- `AvevaCatalogue/acp000`: no measured `COMM.VERS`, returned `Unknown`
  (`UPGDNO=12010101` is diagnostic only).
- `SCB/scb000`: no measured `COMM.VERS`, returned `Unknown`.
- `ZDJ/ZDJ000`: no measured `COMM.VERS`, returned `Unknown`.

This is why gen-model limits discovery to `included_projects`: an excluded
project with recognizable evidence must not lend its version to an otherwise
unknown configured project. When several included projects share one schema,
the highest measured version among those projects is selected.
