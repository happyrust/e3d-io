#Requires -Version 7.0
<#
.SYNOPSIS
  Prepare the 10.5 E3D Design graphics gate on a scratch copy of the AMS sample.

.DESCRIPTION
  Copies the source project (no pictures, no drawings) into -Scratch, resets
  ams8000_0001 from the source, then writes through `e3d-write apply` the two
  Q13 batches the gate looks at: q13_control.json under `--spatial off` (a BOX
  with no SPAMAP entry, the negative case) and q13_boxes.json under `require`
  (BOXes with entries, the positive case). Refnos come from each `--report`.
  The read side then checks the precondition the gate depends on: the positive
  BOX has SPAMAP entries and the control BOX has none.

  The source project is never written: its ams000 directory is inventoried
  before and after (regression_checks.ps1) into <Scratch>\evidence, together
  with gui-gate.json (elements, sessions, the designviewLimits.pmlcmd hashes
  and AIOS markers, and the commands for the session). Design is then started
  on the copy with tools/e3d/gui_gate_launch.bat.

.EXAMPLE
  pwsh -NoProfile -File tools/e3d/gui_gate_prepare.ps1
#>
[CmdletBinding()]
param(
    [string]$Project = 'D:\AVEVA\Projects\E3D3.1\AvevaMarineSample',
    [string]$Scratch = 'D:\e3dio-gui-gate',
    [string]$DbRelative = 'ams000\ams8000_0001',
    [string[]]$LimitsPmlLibs = @('E:\reverse\e3d\PMLLIB', 'E:\reverse\e3d\shadow_e3d31_aps_all\PMLLIB'),
    [switch]$RefreshCopy
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'regression_checks.ps1')
if (Test-RegressionOverlap $Scratch $Project) { throw 'Scratch must not overlap the source project' }
$crate = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$opsDir = Join-Path $PSScriptRoot 'write_gate_ops'
$copy = Join-Path $Scratch (Split-Path $Project -Leaf)
$copyDb = Join-Path $copy $DbRelative
$evidence = Join-Path $Scratch 'evidence'
$opsOut = Join-Path $Scratch 'ops'
$schema = Join-Path $crate 'schema\e3d31'
$utf8 = [Text.UTF8Encoding]::new($false)
New-Item -ItemType Directory -Force $evidence, $opsOut | Out-Null

function Write-Json($Value, [string]$Path) { [IO.File]::WriteAllText($Path, (ConvertTo-Json -InputObject $Value -Depth 12), $utf8) }
# A refusal e3d-write reports arrives on stderr; fold it into the returned text.
function Invoke-Native([scriptblock]$Command) {
    $eap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $lines = & $Command 2>&1 | ForEach-Object { [string]$_ }
        $code = $LASTEXITCODE
    } finally { $ErrorActionPreference = $eap }
    [pscustomobject]@{ Output = ($lines -join "`n"); ExitCode = $code }
}

$sourceRoot = Join-Path $Project 'ams000'
$before = Get-RegressionSnapshot $sourceRoot
Write-Json $before (Join-Path $evidence 'ams-before.json')

if ($RefreshCopy -or -not (Test-Path (Join-Path $copy 'ams000'))) {
    New-Item -ItemType Directory -Force $copy | Out-Null
    robocopy $Project $copy /E /XD amspic amsdrg /R:1 /W:1 /NFL /NDL /NJH /NJS /NP | Out-Null
    if ($LASTEXITCODE -ge 8) { throw "robocopy failed with $LASTEXITCODE" }
    Write-Output "project copied to $copy"
}
Copy-Item -LiteralPath (Join-Path $Project $DbRelative) -Destination $copyDb -Force
Write-Output "reset $copyDb from the source"

Push-Location $crate
try {
    $build = Invoke-Native { cargo build --quiet --bin e3d-write --bin e3d-descriptor }
    if ($build.ExitCode) { throw "cargo build failed:`n$($build.Output)" }
    $eap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { $metaJson = & cargo metadata --format-version 1 --no-deps 2>$null | Out-String } finally { $ErrorActionPreference = $eap }
    $head = (& git rev-parse HEAD | Out-String).Trim()
} finally { Pop-Location }
$targetDir = ($metaJson | ConvertFrom-Json).target_directory
$write = Join-Path $targetDir 'debug\e3d-write.exe'
$descriptor = Join-Path $targetDir 'debug\e3d-descriptor.exe'

# The Q13 SITE sits at BOX 24384/24777's world origin, as in q13_gate.ps1.
$frame = Invoke-Native { & $descriptor world-frame --db $copyDb --refno 24384/24777 --schema $schema --rules confirmed }
if ($frame.ExitCode) { throw $frame.Output }
$origin = (($frame.Output | ConvertFrom-Json).frame.origin_mm | ForEach-Object { ([double]$_).ToString('R', [Globalization.CultureInfo]::InvariantCulture) }) -join ' '

$ids = [ordered]@{}
$sessions = [ordered]@{}
function Invoke-Batch([string]$Name, [hashtable]$Vars, [string]$Policy) {
    $text = [IO.File]::ReadAllText((Join-Path $opsDir "$Name.json"), $utf8)
    foreach ($key in $Vars.Keys) { $text = $text.Replace("{{$key}}", [string]$Vars[$key]) }
    if ($text -match '\{\{[A-Z0-9]+\}\}') { throw "$Name still has a placeholder: $($Matches[0])" }
    $rendered = Join-Path $opsOut "$Name.json"
    $report = Join-Path $opsOut "$Name.report.json"
    [IO.File]::WriteAllText($rendered, $text, $utf8)
    Remove-Item -LiteralPath $report -Force -ErrorAction SilentlyContinue
    $run = Invoke-Native { & $write apply --db $copyDb --ops $rendered --report $report --schema $schema --spatial $Policy --user e3dio --comment "10.5 GUI gate $Name" }
    [IO.File]::WriteAllText((Join-Path $opsOut "$Name.out.txt"), $run.Output, $utf8)
    if ($run.ExitCode) { throw "e3d-write apply $Name failed ($($run.ExitCode)):`n$($run.Output)" }
    $parsed = [IO.File]::ReadAllText($report, $utf8) | ConvertFrom-Json
    $source = $text | ConvertFrom-Json
    for ($i = 0; $i -lt $source.ops.Count; $i++) {
        if ($source.ops[$i].id) { $ids[$source.ops[$i].id] = [ordered]@{ name = $source.ops[$i].name; refno = $parsed.ops[$i].refno } }
    }
    $sessions[$Name] = [ordered]@{ spatial = $Policy; sesno = $parsed.commit.written.sesno }
    Write-Output ("{0,-12} --spatial {1,-7} session {2}" -f $Name, $Policy, $parsed.commit.written.sesno)
}
Invoke-Batch 'q13_control' @{ ORIGIN = $origin } 'off'
Invoke-Batch 'q13_boxes' @{ QZONE = $ids['zone'].refno } 'require'

$entries = [ordered]@{}
foreach ($id in @('main', 'control')) {
    $refno = $ids[$id].refno
    $run = Invoke-Native { & $descriptor spamap --db $copyDb --refno $refno }
    if ($run.ExitCode) { throw $run.Output }
    $entries[$id] = @(($run.Output | ConvertFrom-Json).entries).Count
}
if ($entries['main'] -lt 1) { throw "positive BOX $($ids['main'].refno) has no SPAMAP entry" }
if ($entries['control'] -ne 0) { throw "control BOX $($ids['control'].refno) has $($entries['control']) SPAMAP entries" }

$limits = @(foreach ($root in $LimitsPmlLibs) {
    $file = Join-Path $root 'common\commands\designviewLimits.pmlcmd'
    if (-not (Test-Path -LiteralPath $file)) { [ordered]@{ path = $file; missing = $true }; continue }
    $text = [IO.File]::ReadAllText($file)
    [ordered]@{
        path = $file
        sha256 = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash
        markers = @([regex]::Matches($text, '-- AIOS AMS: [^\r\n]*') | ForEach-Object Value | Select-Object -Unique)
    }
})

$after = Get-RegressionSnapshot $sourceRoot
Write-Json $after (Join-Path $evidence 'ams-after-prepare.json')
if (-not (Test-RegressionSnapshot $before $after)) { throw "the source $sourceRoot changed while preparing" }

$positive = $ids['main'].name
$negative = $ids['control'].name
$gate = [ordered]@{
    prepared_at = [DateTimeOffset]::Now.ToString('o')
    head = $head
    source = $Project
    copy = $copy
    db = $copyDb
    source_db_sha256 = (Get-FileHash -LiteralPath (Join-Path $Project $DbRelative) -Algorithm SHA256).Hash
    copy_db_sha256 = (Get-FileHash -LiteralPath $copyDb -Algorithm SHA256).Hash
    origin_mm = $origin
    sessions = $sessions
    elements = $ids
    spamap_entries = $entries
    designview_limits_before_launch = $limits
    positive = [ordered]@{ element = $positive; refno = $ids['main'].refno; commands = @("ADD $positive", $positive, 'LIMITS CE'); expect = 'visible in the 3D view; no (2,536)' }
    negative = [ordered]@{ element = $negative; refno = $ids['control'].refno; commands = @("ADD $negative", $negative, 'LIMITS CE'); expect = '(2,536)' }
    finish = 'QUIT without SAVEWORK; no e3d process or write lock left; source ams000 inventoried again'
}
Write-Json $gate (Join-Path $evidence 'gui-gate.json')

Write-Output "positive $positive = $($ids['main'].refno), $($entries['main']) SPAMAP entr$(if ($entries['main'] -eq 1) { 'y' } else { 'ies' })"
Write-Output "negative $negative = $($ids['control'].refno), no SPAMAP entry"
Write-Output "source ams000 unchanged; evidence in $evidence"
Write-Output "next: tools\e3d\gui_gate_launch.bat (PROJECTS_ROOT = $Scratch)"
