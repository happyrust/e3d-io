<#
.SYNOPSIS
  Pin the sample databases the write tests copy (DbCopy::of in
  tests/common/write_fixture.rs), so golden hashes stay comparable while E3D
  keeps saving onto the installed samples.

.DESCRIPTION
  Copies each sample into the snapshot directory once. A sample already there
  is left alone unless -Refresh is given: the point of a snapshot is that it
  does not move. Rewrites MANIFEST.tsv (name, length, SHA-256, installed path,
  and the installed file's last write time as copied) and prints it, for the
  notes that go with a golden recording in the Spec_Ledger.

  The directory is the one DbCopy::of looks in: E3DIO_WRITE_FIXTURES when set,
  else D:\e3dio-fixtures\phase3.

.EXAMPLE
  powershell -NoProfile -File tools/golden/snapshot.ps1
#>
[CmdletBinding()]
param(
    [string]$Dir = $(if ($env:E3DIO_WRITE_FIXTURES) { $env:E3DIO_WRITE_FIXTURES } else { 'D:\e3dio-fixtures\phase3' }),
    [switch]$Refresh
)

$ErrorActionPreference = 'Stop'

# Every sample the write tests call DbCopy::of with.
$samples = @(
    'D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001',
    'D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5100_0001',
    'D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams251142_0001',
    'D:\AVEVA\Projects\E3D3.1\TEST\TES000\TES023'
)

New-Item -ItemType Directory -Force -Path $Dir | Out-Null
$rows = @("name`tlength`tsha256`tinstalled`tinstalled_last_write")
foreach ($sample in $samples) {
    $name = Split-Path -Leaf $sample
    $pinned = Join-Path $Dir $name
    if ($Refresh -or -not (Test-Path -LiteralPath $pinned)) {
        if (-not (Test-Path -LiteralPath $sample)) {
            Write-Warning "[snapshot] not installed, not pinned: $sample"
            continue
        }
        # Copy-Item keeps the installed file's last write time, which the
        # manifest reports as the moment the snapshot was taken of.
        Copy-Item -LiteralPath $sample -Destination $pinned -Force
        Write-Host "[snapshot] pinned $name"
    }
    $file = Get-Item -LiteralPath $pinned
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $pinned).Hash
    $rows += "$name`t$($file.Length)`t$hash`t$sample`t$($file.LastWriteTime.ToString('yyyy-MM-dd HH:mm:ss'))"
}
[IO.File]::WriteAllText((Join-Path $Dir 'MANIFEST.tsv'), (($rows -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
$rows | ForEach-Object { Write-Output $_ }
