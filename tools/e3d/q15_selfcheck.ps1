$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'q15_checks.ps1')

# AMS8000, extent 24384: index maximum 26483, counter 26506, so K = 23.
function Case([int64]$First, [int64]$Floor, [string]$E3dRef, [hashtable]$Change = @{}) {
    $run = [ordered]@{
        extent = 24384; index_max = 26483; counter_before = 26506
        issued = @(0..22 | ForEach-Object { "24384/$($First + $_)" })
        floor_after_insert = $Floor; e3d_saved = $true; e3d_ref = $E3dRef
    }
    foreach ($key in $Change.Keys) { $run[$key] = $Change[$key] }
    Get-Q15Verdict ([pscustomobject]$run)
}
function Expect([string]$Want, $Verdict, [string]$Label) {
    if ($Verdict.verdict -ne $Want) { throw "$Label`: wanted $Want, got $($Verdict.verdict) ($($Verdict.reasons -join '; '))" }
    $script:count++
}
$count = 0

# Page 1 maintained: e3d-io issues 26506..26528 and raises the counter to 26529, which E3D takes.
Expect 'pass' (Case 26506 26529 '24384/26529') 'maintained'
# c61b097: e3d-io issues 26484..26506 from the index maximum, page 1 stays at 26506, and E3D
# is handed 26506, issued and deleted.
$old = Case 26484 26506 '24384/26506'
Expect 'fail' $old 'c61b097'
if (($old.reasons -join ' ') -notmatch 'issued and deleted') { throw "c61b097: the reuse is not named: $($old.reasons -join '; ')" }
# E3D above the raised counter, or on a deleted number while the floor matches: still FAIL.
Expect 'fail' (Case 26506 26529 '24384/26530') 'past the floor'
Expect 'fail' (Case 26506 26529 '24384/26510') 'a deleted number'
# The case does not hold: indeterminate, never a PASS.
Expect 'indeterminate' (Case 26506 26529 '24384/26529' @{ index_max = 26506 }) 'M = C'
Expect 'indeterminate' (Case 26506 26529 '24384/26529' @{ issued = @('24384/26506', '24384/26507') }) 'not K issued'
Expect 'indeterminate' (Case 26506 26529 '24384/26529' @{ issued = @(0..22 | ForEach-Object { "32576/$(100 + $_)" }) }) 'issued elsewhere'
Expect 'indeterminate' (Case 26506 26529 '32576/98') 'E3D in another extent'
Expect 'indeterminate' (Case 26506 26529 '24384/26529' @{ e3d_saved = $false }) 'not saved'
Expect 'indeterminate' (Case 26506 26529 '') 'no Q REF'
# A report's refno written as an element (=w0/w1) reads the same.
Expect 'pass' (Case 26506 26529 '24384/26529' @{ issued = @(0..22 | ForEach-Object { "=24384/$(26506 + $_)" }) }) '=refno'

foreach ($file in @('alternate_write_gate.ps1', 'q15_checks.ps1', 'q15_selfcheck.ps1')) {
    $tokens = $null; $errors = $null
    $null = [Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot $file), [ref]$tokens, [ref]$errors)
    if ($errors.Count) { throw ($errors | Out-String) }
}
$gate = Get-Content (Join-Path $PSScriptRoot 'alternate_write_gate.ps1') -Raw
foreach ($needle in @('[switch]$OnlyQ15', '[string]$WriteExe', "q15_checks.ps1", 'Get-Q15Verdict', 'PASS Q15', 'FAIL Q15')) {
    if (-not $gate.Contains($needle)) { throw "alternate_write_gate.ps1 lacks $needle" }
}
$macro = Get-Content (Join-Path $PSScriptRoot 'alternate_write_e3d.mac.template') -Raw
foreach ($pair in @{ LOGDIR = 'D:/scratch/logs-alt'; MACROLOG = 'D:/scratch/alt_e3d_q15.log'; ZONE = '24384/47'; E3DNAME = 'E3DIO-Q15-E3D'; ROUND = 'q15'; POS = '0' }.GetEnumerator()) {
    $macro = $macro.Replace("{{$($pair.Key)}}", $pair.Value)
}
if ($macro -match '\{\{[^}]+\}\}') { throw "unresolved Q15 macro placeholder: $($Matches[0])" }
if ($macro -notmatch 'alt_e3d_q15\.log' -or $macro -notmatch 'NEW EQUI /E3DIO-Q15-E3D' -or $macro -notmatch 'Q REF') { throw 'Q15 macro does not NEW, Q REF and log to alt_e3d_q15.log' }
Write-Output "PASS Q15 offline: $count verdict cases, syntax, gate wiring and macro rendering"
