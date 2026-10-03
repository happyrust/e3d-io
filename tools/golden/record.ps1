<#
.SYNOPSIS
  Record golden hashes of the files the write tests leave behind
  (tests/common/golden.rs) into one tests/golden/*.json file.

.DESCRIPTION
  Pins the samples first (snapshot.ps1), then runs the given test targets
  -Runs times (default 2) with E3DIO_GOLDEN_RECORD set, so every DbCopy
  appends its key, source and hashes when it is dropped. A key is kept only
  when every run made it exactly once, from the same sample bytes, ending in
  the same bytes. The rest are dropped and listed with the reason, for the
  Spec_Ledger table of dropped golden keys.

  Refuses to record from a failing run. Writes one entry per line, keys in
  ordinal order, so a re-recording diffs entry by entry.

.EXAMPLE
  powershell -NoProfile -File tools/golden/record.ps1 -Out tests/golden/off_compat.json -Test write_stage6_transaction,write_stage6_merge,write_stage6_set_by_name,write_stage6_spread,write_stage6_cli
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Out,
    [Parameter(Mandatory = $true)][string[]]$Test,
    [int]$Runs = 2
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location -LiteralPath $repo

# With -File the list arrives as one comma-joined string.
$tests = @($Test | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
$targets = @()
foreach ($t in $tests) { $targets += '--test'; $targets += $t }

$manifest = @(& (Join-Path $PSScriptRoot 'snapshot.ps1'))

$work = Join-Path ([IO.Path]::GetTempPath()) ('e3dio-golden-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $work | Out-Null

$records = @()
for ($i = 1; $i -le $Runs; $i++) {
    $file = Join-Path $work "run$i.jsonl"
    $env:E3DIO_GOLDEN_RECORD = $file
    Write-Host "[golden] run $i of ${Runs}: cargo test --no-fail-fast $($targets -join ' ')"
    $ErrorActionPreference = 'Continue'
    & cargo test --no-fail-fast @targets
    $code = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'
    Remove-Item Env:\E3DIO_GOLDEN_RECORD
    if ($code -ne 0) {
        throw "[golden] run $i failed (cargo exit $code); golden hashes are recorded only from passing runs. Records so far: $work"
    }
    $made = @{}
    $twice = @{}
    if (Test-Path -LiteralPath $file) {
        foreach ($line in Get-Content -LiteralPath $file -Encoding UTF8) {
            if (-not $line.Trim()) { continue }
            $entry = $line | ConvertFrom-Json
            if ($made.ContainsKey($entry.key)) { $twice[$entry.key] = $true } else { $made[$entry.key] = $entry }
        }
    }
    $records += , @{ made = $made; twice = $twice }
}

$keys = New-Object 'System.Collections.Generic.SortedSet[string]' ([StringComparer]::Ordinal)
foreach ($run in $records) { foreach ($key in $run.made.Keys) { [void]$keys.Add($key) } }

$kept = New-Object 'System.Collections.Generic.List[object]'
$dropped = New-Object 'System.Collections.Generic.List[string]'
foreach ($key in $keys) {
    $why = $null
    $first = $null
    foreach ($run in $records) {
        if ($run.twice.ContainsKey($key)) { $why = 'made twice in one run'; break }
        if (-not $run.made.ContainsKey($key)) { $why = 'not made in every run'; break }
        $entry = $run.made[$key]
        if ($null -ne $entry.unreadable) { $why = "copy unreadable when dropped: $($entry.unreadable)"; break }
        if ($null -eq $first) {
            $first = $entry
        } elseif ($entry.sha256 -ne $first.sha256 -or $entry.len -ne $first.len -or $entry.source_sha256 -ne $first.source_sha256) {
            $why = 'ends differently between runs'
            break
        }
    }
    if ($why) { $dropped.Add("$key`t$why") } else { $kept.Add($first) }
}

function Quote([string]$text) { '"' + ($text -replace '\\', '\\' -replace '"', '\"') + '"' }

$head = (& git rev-parse HEAD).Trim()
$uncommitted = @(& git status --porcelain --untracked-files=no -- src tests Cargo.toml Cargo.lock).Count -gt 0
$used = @{}
foreach ($entry in $kept) { $used[$entry.source] = $true }

$lines = New-Object 'System.Collections.Generic.List[string]'
$lines.Add('{')
$lines.Add('  "recorded_on": ' + (Quote $head) + ',')
$lines.Add('  "uncommitted_changes_under_src_or_tests": ' + $(if ($uncommitted) { 'true' } else { 'false' }) + ',')
$lines.Add('  "tests": [' + (($tests | ForEach-Object { Quote $_ }) -join ', ') + '],')
$samples = New-Object 'System.Collections.Generic.List[object]'
foreach ($row in ($manifest | Select-Object -Skip 1)) {
    $cols = $row -split "`t"
    if ($used.ContainsKey($cols[0])) { $samples.Add($cols) }
}
$lines.Add('  "samples": {')
for ($i = 0; $i -lt $samples.Count; $i++) {
    $s = $samples[$i]
    $comma = if ($i -lt $samples.Count - 1) { ',' } else { '' }
    $lines.Add('    ' + (Quote $s[0]) + ': { "len": ' + $s[1] + ', "sha256": ' + (Quote $s[2]) + ', "installed": ' + (Quote $s[3]) + ', "installed_last_write": ' + (Quote $s[4]) + ' }' + $comma)
}
$lines.Add('  },')
$lines.Add('  "entries": {')
for ($i = 0; $i -lt $kept.Count; $i++) {
    $e = $kept[$i]
    $comma = if ($i -lt $kept.Count - 1) { ',' } else { '' }
    $lines.Add('    ' + (Quote $e.key) + ': { "source": ' + (Quote $e.source) + ', "source_sha256": ' + (Quote $e.source_sha256) + ', "sha256": ' + (Quote $e.sha256) + ', "len": ' + $e.len + ' }' + $comma)
}
$lines.Add('  }')
$lines.Add('}')

$outPath = if ([IO.Path]::IsPathRooted($Out)) { $Out } else { Join-Path $repo $Out }
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $outPath) | Out-Null
[IO.File]::WriteAllText($outPath, (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
Remove-Item -Recurse -Force -LiteralPath $work

Write-Output "[golden] wrote ${Out}: $($kept.Count) entries, recorded on $head$(if ($uncommitted) { ' WITH UNCOMMITTED CHANGES under src/tests' })"
Write-Output "[golden] dropped $($dropped.Count):"
$dropped | ForEach-Object { Write-Output "  $_" }
Write-Output '[golden] snapshot manifest:'
$manifest | ForEach-Object { Write-Output "  $_" }
