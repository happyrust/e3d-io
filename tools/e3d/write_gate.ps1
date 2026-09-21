<#
.SYNOPSIS
  The E3D gate for the write path, run unattended: write sessions with e3d-io into a
  copy of the AMS sample project, then have a real E3D 3.1 Design TTY session read them
  back and check what it prints (docs/plans/2026-09-20-dabacon-write-plan.md §7, Q8).

.DESCRIPTION
  1. Copies the sample project (all but the picture and drawing folders) to a scratch
     directory once, and resets its ams8000_0001 from the original before every run —
     the live project is never touched.
  2. Runs `e3d-write-box <copy db> --group --hunk`: four sessions — a BOX, a GPWL+GROU
     with reference members, two EQUIs whose DESC/DESP/UUIDIA are hunks (one chain across
     a page boundary), a FIXING with a type-19 string list inline and as a hunk.
  3. Renders write_gate.mac.template with the refnos the bin printed and runs it through
     gen-model's l3_suite `--check-driver` (des.exe TTY, login SYSTEM/XXXXXX, MDB /ALL,
     project AMS pointed at the copy via --project-dir). The macro is read-only.
  4. Greps the per-question logs for the values written and prints PASS/FAIL per question;
     exits 1 on any FAIL.

  Other E3D sessions may be running: the driver is told to tolerate them
  (L3_ALLOW_EXISTING_E3D_SESSION=1) and waits only for the session it starts.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File tools/e3d/write_gate.ps1
#>
[CmdletBinding()]
param(
    [string]$Project = 'D:\AVEVA\Projects\E3D3.1\AvevaMarineSample',
    [string]$Scratch = 'D:\e3dio-gate',
    [string]$GenModel = 'D:\work\plant-code\old\gen-model',
    [string]$L3Exe = 'D:\Rust\target\debug\l3_suite.exe',
    [string]$DbRelative = 'ams000\ams8000_0001',
    [switch]$RefreshCopy
)

$ErrorActionPreference = 'Stop'
$crate = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$copy = Join-Path $Scratch (Split-Path $Project -Leaf)
$copyDb = Join-Path $copy $DbRelative
$logDir = Join-Path $Scratch 'logs'
$macro = Join-Path $Scratch 'write_gate.mac'
$driverOut = Join-Path $Scratch 'driver-out'

foreach ($required in @($Project, (Join-Path $Project $DbRelative), $GenModel, $L3Exe)) {
    if (-not (Test-Path $required)) { throw "missing: $required" }
}

# 1. The project copy (evars + ams000 + the small folders; no pictures, no drawings).
if ($RefreshCopy -or -not (Test-Path (Join-Path $copy 'ams000\amssys'))) {
    New-Item -ItemType Directory -Force $copy | Out-Null
    robocopy $Project $copy /E /XD amspic amsdrg /R:1 /W:1 /NFL /NDL /NJH /NJS /NP | Out-Null
    if ($LASTEXITCODE -ge 8) { throw "robocopy failed with $LASTEXITCODE" }
    Write-Output "project copied to $copy"
}
Copy-Item (Join-Path $Project $DbRelative) $copyDb -Force
Write-Output "reset $copyDb from the original"

# 2. The sessions.
Push-Location $crate
try {
    $binOut = & cargo run --quiet --bin e3d-write-box -- $copyDb --group --hunk 2>&1 | Out-String
} finally { Pop-Location }
if ($LASTEXITCODE) { Write-Output $binOut; throw 'e3d-write-box failed' }
Set-Content -Path (Join-Path $Scratch 'e3d-write-box.out.txt') -Value $binOut
$refno = {
    param([string]$label)
    $m = [regex]::Match($binOut, "$label\s*:\s*(\d+/\d+)")
    if (-not $m.Success) { throw "e3d-write-box output has no '$label'" }
    $m.Groups[1].Value
}
$ids = @{
    BOX    = & $refno 'new BOX'
    GPWL   = & $refno 'new GPWL'
    GROU   = & $refno 'new GROU'
    EQUI1  = & $refno 'new EQUI 1'
    EQUI2  = & $refno 'new EQUI 2'
    FIXING = & $refno 'new FIXING'
    ZONE   = & $refno 'owner ZONE'
}
$spanning = [regex]::Match($binOut, 'hunk chains that cross a page boundary: (\d+)').Groups[1].Value
Write-Output ("wrote: BOX {0} GPWL {1} GROU {2} EQUI {3} {4} FIXING {5}; hunk chains across a page: {6}" -f $ids.BOX, $ids.GPWL, $ids.GROU, $ids.EQUI1, $ids.EQUI2, $ids.FIXING, $spanning)

# 3. The macro, rendered; then E3D.
New-Item -ItemType Directory -Force $logDir, $driverOut | Out-Null
Remove-Item (Join-Path $logDir '*.log') -Force -ErrorAction SilentlyContinue
$forward = { param([string]$p) $p -replace '\\', '/' }
$text = Get-Content (Join-Path $PSScriptRoot 'write_gate.mac.template') -Raw
$text = $text.Replace('{{LOGDIR}}', (& $forward $logDir)).Replace('{{MACROLOG}}', (& $forward ([IO.Path]::ChangeExtension($macro, '.log'))))
foreach ($key in $ids.Keys) { $text = $text.Replace("{{$key}}", $ids[$key]) }
Set-Content -Path $macro -Value $text -Encoding ASCII

$previous = $env:L3_ALLOW_EXISTING_E3D_SESSION
$env:L3_ALLOW_EXISTING_E3D_SESSION = '1'
Push-Location $GenModel
try {
    & $L3Exe --check-driver $macro --project-dir $copy --output $driverOut 2>&1 | Tee-Object -FilePath (Join-Path $driverOut 'l3_suite.stdout.txt') | Select-Object -First 3
    $driverExit = $LASTEXITCODE
} finally {
    Pop-Location
    if ($null -eq $previous) { Remove-Item Env:\L3_ALLOW_EXISTING_E3D_SESSION -ErrorAction SilentlyContinue } else { $env:L3_ALLOW_EXISTING_E3D_SESSION = $previous }
}
if ($driverExit) { throw "E3D TTY driver failed ($driverExit); see $driverOut" }

# 4. The verdicts.
$failures = 0
function Check([string]$question, [string]$log, [string[]]$expected) {
    $path = Join-Path $logDir $log
    if (-not (Test-Path $path)) { Write-Output "FAIL $question — $log was never written (the block did not reach ALPHA LOG END)"; $script:failures++; return }
    $content = Get-Content $path -Raw
    $missing = @($expected | Where-Object { $content -notmatch [regex]::Escape($_) })
    $errors = @([regex]::Matches($content, '(?m)^.*(ERROR|\(\d+,\d+\)).*$') | ForEach-Object { $_.Value })
    if ($missing.Count -eq 0 -and $errors.Count -eq 0) {
        Write-Output "PASS $question"
    } else {
        Write-Output "FAIL $question"
        foreach ($m in $missing) { Write-Output "     missing: $m" }
        foreach ($e in $errors) { Write-Output "     E3D said: $e" }
        $script:failures++
    }
}
$desc = { param([int]$n) "Description e3d-io hunk gate EQUI ${n}: DESC stored out of the record (type 22) @065 @070 @075 @080 @085 @090 @095 @100 @105 @110 @115" }
Check 'Q1  BOX geometry and owner (session 1)' '10_box.log' @('Type BOX', 'Position E 1000mm N 2000mm U 3000mm', 'Xlength 300mm', 'Ylength 400mm', 'Zlength 500mm')
Check 'Q2  GROU lists ZONE and BOX as reference members (session 2)' '20_grou.log' @('Type GROU', 'ZONE 1', "BOX 1 =$($ids.BOX)", 'Type GPWL', "GROU 1 =$($ids.GROU)")
Check 'Q3a EQUI 1: DESC as a type-22 hunk' '31_equi1_desc.log' @((& $desc 1))
Check 'Q3b EQUI 1: DESP as a type-21 hunk (100 reals)' '32_equi1_desp.log' @('Desparam 1001.3 1002.3', '1100.3')
Check 'Q3c EQUI 1: UUIDIA as a type-20 hunk' '33_equi1_uuidia.log' @('Uuidia 11 12 13 14')
Check 'Q3d EQUI 2: DESC as a type-22 hunk' '36_equi2_desc.log' @((& $desc 2))
Check 'Q3e EQUI 2: DESP as a type-21 hunk whose chain crosses a page' '37_equi2_desp.log' @('Desparam 2001.3 2002.3', '2100.3')
Check 'Q3f EQUI 2: UUIDIA as a type-20 hunk' '38_equi2_uuidia.log' @('Uuidia 21 22 23 24')
Check 'Q4a FIXING: ROKEYS, a type-19 list stored inline' '41_fixing_rokeys.log' @('Rokeys', ' 1 K1', ' 2 K2')
Check 'Q4b FIXING: ROVALS, the same list as a type-23 hunk' '42_fixing_rovals.log' @('Rovals', ' 1 V1', ' 2 V2')
Check 'Q5  ZONE lists the four new members last' '50_zone.log' @("BOX 1 =$($ids.BOX)", "EQUI 1 =$($ids.EQUI1)", "EQUI 2 =$($ids.EQUI2)", "FIXING 1 =$($ids.FIXING)")
Check 'Q6  Q ATT walks every attribute of the EQUI and the FIXING' '60_equi2_att.log' @((& $desc 2), 'Desparam 2001.3')
Check 'Q6b Q ATT of the FIXING' '61_fixing_att.log' @('Type FIXING')
Check 'Q7  E3D reads the session page (user e3dio, our comment)' '70_sessions.log' @('User: e3dio', 'ROVALS as a hunk (type 23)')
if ([int]$spanning -lt 1) { Write-Output 'FAIL     no hunk chain crossed a page boundary in this run, so the continuation link went unexercised'; $failures++ }

Write-Output ''
if ($failures) { Write-Output "$failures FAIL — logs in $logDir"; exit 1 }
Write-Output "all PASS — E3D 3.1 read every session e3d-io wrote (logs in $logDir)"
