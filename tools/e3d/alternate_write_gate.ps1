<#
.SYNOPSIS
  The E3D gate for the write path's T4 alternate-write check
  (docs/plans/2026-09-22-write-path-phase-2-plan.md §2 T4): E3D and e3d-io write in turn
  on the same ZONE for N rounds, then a real E3D 3.1 reads every element both sides wrote
  and the session tail BASESN chain is verified unbroken.

.DESCRIPTION
  Each round r:
    1. A real E3D 3.1 TTY session opens the AMS work copy (its newest session is e3d-io's
       from the round before), creates an EQUI /E3DIO-ALT-E3D-r under the first ZONE, and
       SAVEWORKs (alternate_write_e3d.mac.template, stateful driver).
    2. e3d-io reads back the EQUI E3D just wrote (e3d-descriptor extract), proving e3d-io
       sees E3D's write.
    3. `e3d-write apply` (opening the newest session — E3D's — as its base) inserts an EQUI
       /E3DIO-ALT-IO-r under the same ZONE; the report must show base = the E3D session
       just written and no merge (a straight append, BASESN = base).
  After N rounds, one read-only E3D session lists the ZONE's members: all 2N EQUIs, by
  name (E3D reads every e3d-io write). Then `e3d-descriptor sessions` verifies the session
  tail BASESN chain is unbroken from the pre-run session to the newest, and that the
  e3d-io sessions carry user e3dio on top of an E3D session each.

  Q15 (requirement 13, S6b; q15_checks.ps1) follows the rounds, on the DB reset from the
  original: e3d-io inserts K = C - M EQUIs under the ZONE in one session (M the ZONE
  extent's index maximum, C its page-1 Bucket_Counter) and deletes them all in the next;
  then E3D NEWs an EQUI there and SAVEWORKs. E3D must be handed page 1's counter after the
  insert, none of the numbers e3d-io issued and deleted. -OnlyQ15 runs Q15 alone;
  -WriteExe runs the e3d-io side with another build of e3d-write (an older one without
  --spatial is called without it), e.g. c61b097's, on which Q15 must FAIL.

  Shares the driver contract with write_gate.ps1 (l3_suite --check-driver, detached des.exe
  via run_ams_c_entrymacro.bat) but is self-contained. Other E3D sessions are tolerated
  (L3_ALLOW_EXISTING_E3D_SESSION=1).

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File tools/e3d/alternate_write_gate.ps1 -Rounds 5

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File tools/e3d/alternate_write_gate.ps1 -OnlyQ15 -WriteExe D:\Rust\target-c61b097\debug\e3d-write.exe
#>
[CmdletBinding()]
param(
    [int]$Rounds = 5,
    [string]$Project = 'D:\AVEVA\Projects\E3D3.1\AvevaMarineSample',
    [string]$Scratch = 'D:\e3dio-gate',
    [string]$GenModel = 'D:\work\plant-code\old\gen-model',
    [string]$L3Exe = 'D:\Rust\target\debug\l3_suite.exe',
    [string]$DbRelative = 'ams000\ams8000_0001',
    [string]$ZoneName = '/1RX03-MSUP',
    [switch]$RefreshCopy,
    [switch]$OnlyQ15,
    [string]$WriteExe = ''
)

$ErrorActionPreference = 'Stop'
if (-not (Test-Path $L3Exe)) {
    $fallback = @('D:\Rust\target-l3\debug\l3_suite.exe', 'D:\Rust\target\release\l3_suite.exe') | Where-Object { Test-Path $_ } | Select-Object -First 1
    if ($fallback) { Write-Output "l3_suite not at $L3Exe; using $fallback"; $L3Exe = $fallback }
}
$crate = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$copy = Join-Path $Scratch (Split-Path $Project -Leaf)
$copyDb = Join-Path $copy $DbRelative
$logDir = Join-Path $Scratch 'logs-alt'
$utf8 = New-Object Text.UTF8Encoding($false)

foreach ($required in @($Project, (Join-Path $Project $DbRelative), $GenModel, $L3Exe, $PSScriptRoot)) {
    if (-not (Test-Path $required)) { throw "missing: $required" }
}
$failures = 0

# --- build the two bins this gate drives ---
function Invoke-Native([scriptblock]$command) {
    $eap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $lines = & $command 2>&1 | ForEach-Object { if ($_ -is [Management.Automation.ErrorRecord]) { $_.Exception.Message } else { [string]$_ } }
        $code = $LASTEXITCODE
    } finally { $ErrorActionPreference = $eap }
    [pscustomobject]@{ Output = (($lines | Where-Object { $_ -ne $null }) -join "`n"); ExitCode = $code }
}
Push-Location $crate
try {
    $build = Invoke-Native { cargo build --quiet --bin e3d-write --bin e3d-descriptor }
    if ($build.ExitCode) { Write-Output $build.Output; throw 'cargo build failed' }
    $eap = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    try { $metaJson = & cargo metadata --format-version 1 --no-deps 2>$null | Out-String } finally { $ErrorActionPreference = $eap }
    $targetDir = ($metaJson | ConvertFrom-Json).target_directory
} finally { Pop-Location }
$builtWriteExe = Join-Path $targetDir 'debug\e3d-write.exe'
$writeExe = if ($WriteExe) { $WriteExe } else { $builtWriteExe }
$descExe = Join-Path $targetDir 'debug\e3d-descriptor.exe'
$schema = Join-Path $crate 'schema\e3d31'
foreach ($required in @($builtWriteExe, $writeExe, $descExe)) { if (-not (Test-Path $required)) { throw "bin missing: $required" } }
# An e3d-write from before the spatial policy (c61b097) has no --spatial and refuses it.
$spatialOff = if ((Invoke-Native { & $writeExe --help }).Output -match '--spatial') { @('--spatial', 'off') } else { @() }

# 1. The project copy (evars + ams000 + small folders; reset the one DB each run).
if ($RefreshCopy -or -not (Test-Path (Join-Path $copy 'ams000\amssys'))) {
    New-Item -ItemType Directory -Force $copy | Out-Null
    robocopy $Project $copy /E /XD amspic amsdrg /R:1 /W:1 /NFL /NDL /NJH /NJS /NP | Out-Null
    if ($LASTEXITCODE -ge 8) { throw "robocopy failed with $LASTEXITCODE" }
    Write-Output "project copied to $copy"
}
Copy-Item (Join-Path $Project $DbRelative) $copyDb -Force
Write-Output "reset $copyDb from the original"
New-Item -ItemType Directory -Force $logDir | Out-Null
Remove-Item (Join-Path $logDir '*.log') -Force -ErrorAction SilentlyContinue

# --- E3D driver, same contract as write_gate.ps1 ---
$forward = { param([string]$p) $p -replace '\\', '/' }
function Invoke-E3D([string]$template, [string]$macroName, [hashtable]$vars, [switch]$stateful) {
    $macroPath = Join-Path $Scratch $macroName
    $out = Join-Path $Scratch ('driver-out-' + [IO.Path]::GetFileNameWithoutExtension($macroName))
    New-Item -ItemType Directory -Force $out | Out-Null
    $text = Get-Content (Join-Path $PSScriptRoot $template) -Raw
    $text = $text.Replace('{{LOGDIR}}', (& $forward $logDir)).Replace('{{MACROLOG}}', (& $forward ([IO.Path]::ChangeExtension($macroPath, '.log'))))
    foreach ($key in $vars.Keys) { $text = $text.Replace("{{$key}}", [string]$vars[$key]) }
    if ($text -match '\{\{[A-Z0-9]+\}\}') { throw "$macroName still has a placeholder: $($Matches[0])" }
    Set-Content -Path $macroPath -Value $text -Encoding ASCII
    $driverArgs = @('--check-driver', $macroPath, '--project-dir', $copy, '--output', $out)
    if ($stateful) { $driverArgs += @('--target-db-file', $copyDb, '--aios-project', 'e3dio-alt-gate') }
    $previous = $env:L3_ALLOW_EXISTING_E3D_SESSION
    $env:L3_ALLOW_EXISTING_E3D_SESSION = '1'
    Push-Location $GenModel
    try {
        & $L3Exe @driverArgs 2>&1 | Tee-Object -FilePath (Join-Path $out 'l3_suite.stdout.txt') | Select-Object -First 3
        $driverExit = $LASTEXITCODE
    } finally {
        Pop-Location
        if ($null -eq $previous) { Remove-Item Env:\L3_ALLOW_EXISTING_E3D_SESSION -ErrorAction SilentlyContinue } else { $env:L3_ALLOW_EXISTING_E3D_SESSION = $previous }
    }
    if ($driverExit) { throw "E3D TTY driver failed ($driverExit) on $macroName; see $out" }
}
function Read-LogText([string]$path) {
    if (-not (Test-Path $path)) { return '' }
    $bytes = [IO.File]::ReadAllBytes($path)
    try { return (New-Object Text.UTF8Encoding($false, $true)).GetString($bytes) }
    catch { return [Text.Encoding]::GetEncoding([Globalization.CultureInfo]::CurrentCulture.TextInfo.ANSICodePage).GetString($bytes) }
}
function Sessions() {
    $eap = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    try { $json = & $descExe sessions --db $copyDb 2>$null | Out-String } finally { $ErrorActionPreference = $eap }
    ($json | ConvertFrom-Json).sessions
}
function NameOf([string]$refno) {
    $eap = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    try { $json = & $descExe extract --db $copyDb --schema $schema --refno $refno 2>$null | Out-String } finally { $ErrorActionPreference = $eap }
    $x = $json | ConvertFrom-Json
    ($x.attributes | Where-Object { $_.name -eq 'NAME' }).value
}
function NextWord1([int]$extent) {
    $eap = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    try { $json = & $descExe next-word1 --db $copyDb --extent $extent 2>$null | Out-String } finally { $ErrorActionPreference = $eap }
    $json | ConvertFrom-Json
}
# One e3d-io session from an instruction file: its report, or why there is none.
function Invoke-Apply([string]$name, [string]$opsJson) {
    $opsPath = Join-Path $Scratch "$name.json"
    $reportPath = Join-Path $Scratch "$name.report.json"
    [IO.File]::WriteAllText($opsPath, $opsJson, $utf8)
    $run = Invoke-Native { & $writeExe apply --db $copyDb --ops $opsPath --schema $schema @spatialOff --report $reportPath }
    if ($run.ExitCode) { return [pscustomobject]@{ report = $null; error = "e3d-write $name failed ($($run.ExitCode)): $($run.Output)" } }
    [pscustomobject]@{ report = ([IO.File]::ReadAllText($reportPath, $utf8) | ConvertFrom-Json); error = $null }
}

. (Join-Path $PSScriptRoot 'q15_checks.ps1')
function Invoke-Q15 {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    Copy-Item (Join-Path $Project $DbRelative) $copyDb -Force
    $extent = [int](($zoneRef -split '/')[0])
    $before = NextWord1 $extent
    $run = [ordered]@{
        write_exe = $writeExe; extent = $extent; index_max = [int64]$before.scan_max_word1; counter_before = [int64]$before.bucket_counter
        issued = @(); floor_after_insert = $null; counter_after_delete = $null; e3d_saved = $false; e3d_ref = ''
    }
    $k = $run.counter_before - $run.index_max
    if ($k -gt 0) {
        $inserts = @(1..$k | ForEach-Object { "{`"op`":`"insert`",`"owner`":`"$ZoneName`",`"noun`":`"EQUI`",`"name`":`"/E3DIO-Q15-$_`"}" })
        $insert = Invoke-Apply 'q15_insert' "{`"comment`":`"e3d-io Q15 insert K = $k`",`"ops`":[$($inserts -join ',')]}"
        if ($insert.error) { Write-Output "     $($insert.error)" }
        else {
            $run.issued = @($insert.report.ops | ForEach-Object { [string]$_.refno })
            $run.floor_after_insert = [int64](NextWord1 $extent).bucket_counter
            $deletes = @($run.issued | ForEach-Object { "{`"op`":`"delete`",`"refno`":`"$_`"}" })
            $delete = Invoke-Apply 'q15_delete' "{`"comment`":`"e3d-io Q15 delete all $k`",`"ops`":[$($deletes -join ',')]}"
            if ($delete.error) { Write-Output "     $($delete.error)" }
            else {
                $run.counter_after_delete = [int64](NextWord1 $extent).bucket_counter
                try {
                    Invoke-E3D 'alternate_write_e3d.mac.template' 'alt_e3d_q15.mac' @{ ZONE = $zoneRef; E3DNAME = 'E3DIO-Q15-E3D'; ROUND = 'q15'; POS = 0 } -stateful
                } catch {
                    Write-Output "     (E3D Q15: $($_.Exception.Message))"
                }
                $log = Read-LogText (Join-Path $logDir 'alt_e3d_q15.log')
                $run.e3d_saved = $log -match 'ALT-E3D-SAVED'
                $run.e3d_ref = [regex]::Match($log, 'Ref =(\d+/\d+)').Groups[1].Value
            }
        }
    }
    $verdict = Get-Q15Verdict ([pscustomobject]$run)
    $run.verdict = $verdict.verdict
    $run.reasons = @($verdict.reasons)
    $run.elapsed_seconds = $timer.Elapsed.TotalSeconds
    [IO.File]::WriteAllText((Join-Path $logDir 'q15.json'), ($run | ConvertTo-Json -Depth 4), $utf8)
    if ($verdict.verdict -eq 'pass') {
        Write-Output ("PASS Q15 extent {0}: e3d-io issued and deleted K = {1} ({2}..{3}), page 1 {4} -> {5}, and E3D's NEW took {6}, none of them ({7:N1} s)" -f $extent, $k, $run.issued[0], $run.issued[-1], $run.counter_before, $run.floor_after_insert, $run.e3d_ref, $run.elapsed_seconds)
    } else {
        Write-Output ("FAIL Q15 ({0}) extent {1}: M {2}, C {3}, issued {4}, page 1 after the insert {5}, E3D took '{6}' ({7:N1} s)" -f $verdict.verdict, $extent, $run.index_max, $run.counter_before, $(if ($run.issued.Count) { "$($run.issued[0])..$($run.issued[-1])" } else { 'none' }), $run.floor_after_insert, $run.e3d_ref, $run.elapsed_seconds)
        foreach ($reason in $verdict.reasons) { Write-Output "     $reason" }
        $script:failures++
    }
}

# Resolve the first ZONE's refno once, through a dry-run that references it by name.
$probeOps = Join-Path $Scratch 'alt_probe.json'
[IO.File]::WriteAllText($probeOps, "[{`"op`":`"insert`",`"owner`":`"$ZoneName`",`"noun`":`"EQUI`"}]", $utf8)
$probe = Invoke-Native { & $builtWriteExe apply --db $copyDb --ops $probeOps --schema $schema --spatial off --dry-run --report (Join-Path $Scratch 'alt_probe.report.json') }
if ($probe.ExitCode) { Write-Output $probe.Output; throw "cannot resolve $ZoneName" }
$zoneRef = ([IO.File]::ReadAllText((Join-Path $Scratch 'alt_probe.report.json'), $utf8) | ConvertFrom-Json).ops[0].owner
$baseBefore = [int]((Sessions)[0].sesno)
if ($OnlyQ15) {
    Write-Output "alternate-write gate: Q15 alone on ZONE $ZoneName ($zoneRef), e3d-write $writeExe"
    Invoke-Q15
    Write-Output ''
    if ($failures) { Write-Output "$failures FAIL — logs in $logDir"; exit 1 }
    Write-Output "all PASS — Q15 on ZONE $ZoneName (logs in $logDir)"
    exit 0
}
Write-Output "alternate-write gate: $Rounds round(s) on ZONE $ZoneName ($zoneRef), base session $baseBefore"

# The rounds.
$names = New-Object Collections.Generic.List[string]
$e3dSessions = New-Object Collections.Generic.List[int]
$ioSessions = New-Object Collections.Generic.List[int]
# The session that was newest before each round's E3D write, so we can check both
# directions of the hand-off: E3D opened on it, and (after E3D's own quit-save) e3d-io
# opens on whatever is newest then.
$prevLatest = $baseBefore
$e3dSaveSessions = New-Object Collections.Generic.List[int]
for ($r = 1; $r -le $Rounds; $r++) {
    $e3dName = "E3DIO-ALT-E3D-$r"
    $ioName = "/E3DIO-ALT-IO-$r"
    $latestBeforeE3d = [int]((Sessions | Sort-Object sesno -Descending)[0].sesno)
    # 1. E3D writes on top of whatever is newest (e3d-io's from the round before).
    try {
        Invoke-E3D 'alternate_write_e3d.mac.template' "alt_e3d_$r.mac" @{ ZONE = $zoneRef; E3DNAME = $e3dName; ROUND = $r; POS = ($r * 100) } -stateful
    } catch {
        Write-Output "     (E3D round ${r}: $($_.Exception.Message))"
    }
    $log = Read-LogText (Join-Path $logDir "alt_e3d_$r.log")
    if ($log -notmatch 'ALT-E3D-SAVED') {
        Write-Output "FAIL R$r  E3D did not save round $r (see alt_e3d_$r.log)"
        foreach ($e in @([regex]::Matches($log, '(?m)^.*(ERROR|\(\d+,\d+\)).*$') | ForEach-Object { $_.Value })) { Write-Output "     E3D said: $e" }
        $failures++
        break
    }
    $e3dRef = [regex]::Match($log, 'Ref =(\d+/\d+)').Groups[1].Value
    $e3dSes = [int][regex]::Match($log, 'Last saved session number (\d+)').Groups[1].Value
    $e3dSaveSessions.Add($e3dSes)
    # E3D's SAVEWORK session must be based on what was newest before it — e3d-io's
    # from the round before (the base session for round 1). This is the "E3D opens on
    # e3d-io's latest" direction.
    $e3dTail = Sessions | Where-Object { [int]$_.sesno -eq $e3dSes } | Select-Object -First 1
    if ($null -eq $e3dTail -or [int]$e3dTail.tail.base_sesno -ne $latestBeforeE3d) {
        Write-Output "FAIL R$r  E3D's SAVEWORK session $e3dSes has BASESN $($e3dTail.tail.base_sesno), wanted $latestBeforeE3d (the newest before this round)"; $failures++; break
    }
    # 2. e3d-io reads E3D's EQUI back.
    $readName = NameOf $e3dRef
    if ($readName -ne "/$e3dName") {
        Write-Output "FAIL R$r  e3d-io read E3D's EQUI $e3dRef as '$readName', wanted /$e3dName"; $failures++; break
    }
    # 3. e3d-io writes, opening the newest session (E3D's — including its quit-save) as
    # its base: a straight append, BASESN = that session.
    $latestAfterE3d = [int]((Sessions | Sort-Object sesno -Descending)[0].sesno)
    $ioOps = Join-Path $Scratch "alt_io_$r.json"
    [IO.File]::WriteAllText($ioOps, "{`"comment`":`"e3d-io T4 alternate round $r`",`"ops`":[{`"op`":`"insert`",`"owner`":`"$ZoneName`",`"noun`":`"EQUI`",`"name`":`"$ioName`",`"attrs`":{`"POS`":`"$($r * 100) 0 0`",`"FUNC`":`"ALT-IO-$r`"}}]}", $utf8)
    $ioReport = Join-Path $Scratch "alt_io_$r.report.json"
    $io = Invoke-Native { & $writeExe apply --db $copyDb --ops $ioOps --schema $schema @spatialOff --report $ioReport }
    if ($io.ExitCode) { Write-Output "FAIL R$r  e3d-write round $r failed"; Write-Output $io.Output; $failures++; break }
    $rep = [IO.File]::ReadAllText($ioReport, $utf8) | ConvertFrom-Json
    $ioSes = [int]$rep.commit.written.sesno
    $ioSessions.Add($ioSes)
    $ok = ($null -eq $rep.commit.merged_onto) -and ([int]$rep.commit.base_sesno -eq $latestAfterE3d) -and ([int]$rep.commit.previous.sesno -eq $latestAfterE3d)
    if (-not $ok) {
        Write-Output "FAIL R$r  e3d-io session ${ioSes}: base=$($rep.commit.base_sesno) previous=$($rep.commit.previous.sesno) merged_onto=$($rep.commit.merged_onto), wanted a straight append on the newest session $latestAfterE3d"; $failures++; break
    }
    $names.Add("/$e3dName"); $names.Add($ioName)
    $extra = if ($latestAfterE3d -gt $e3dSes) { " (+$($latestAfterE3d - $e3dSes) quit-save)" } else { '' }
    Write-Output ("  R{0}: E3D SAVEWORK {1} on {2}{3} (EQUI {4} {5}) -> e3d-io {6} on {7} ({8})" -f $r, $e3dSes, $latestBeforeE3d, $extra, $e3dRef, "/$e3dName", $ioSes, $latestAfterE3d, $ioName)
    $prevLatest = $ioSes
}

if ($failures -eq 0) {
    Write-Output "PASS T4a $Rounds rounds: E3D and e3d-io wrote in turn, each e3d-io session a straight append on the E3D session before it, and e3d-io read every E3D EQUI"
    # Read-back: E3D lists every EQUI both sides made.
    Invoke-E3D 'alternate_write_readback.mac.template' 'alt_readback.mac' @{ ZONE = $zoneRef }
    $readback = Read-LogText (Join-Path $logDir 'alt_readback.log')
    $missing = @($names | Where-Object { $readback -notmatch [regex]::Escape($_) })
    $navErr = @([regex]::Matches($readback, '(?m)^.*(ERROR|\(\d+,\d+\)).*$') | ForEach-Object { $_.Value })
    if ($missing.Count -eq 0 -and $navErr.Count -eq 0) {
        Write-Output "PASS T4b E3D read back all $($names.Count) EQUIs (both sides) from the ZONE by name"
    } else {
        Write-Output 'FAIL T4b E3D did not list every EQUI'
        foreach ($m in $missing) { Write-Output "     missing: $m" }
        foreach ($e in $navErr) { Write-Output "     E3D said: $e" }
        $failures++
    }
    # The session chain, from the pre-run session to the newest: session numbers are
    # contiguous (nothing lost across the hand-offs), every non-zero BASESN points back
    # to a session that exists (E3D's own login / quit "Saving Meta Data" sessions carry
    # BASESN 0, which is "none" and allowed), and each e3d-io session is user e3dio with
    # BASESN = the session right before it (the E3D one it opened on).
    $chain = @(Sessions | Sort-Object sesno | Where-Object { [int]$_.sesno -ge $baseBefore })
    $sesnos = @($chain | ForEach-Object { [int]$_.sesno })
    $broken = @()
    for ($i = 1; $i -lt $chain.Count; $i++) {
        if ($sesnos[$i] -ne $sesnos[$i - 1] + 1) { $broken += "gap: $($sesnos[$i-1]) then $($sesnos[$i])" }
        $base = [int]$chain[$i].tail.base_sesno
        if ($base -ne 0 -and $sesnos -notcontains $base) { $broken += "session $($sesnos[$i]) BASESN $base has no such session" }
    }
    foreach ($s in $ioSessions) {
        $idx = [array]::IndexOf($sesnos, [int]$s)
        $rec = $chain[$idx]
        if ($rec.user -ne 'e3dio') { $broken += "e3d-io session $s has user '$($rec.user)', wanted e3dio" }
        if ($idx -lt 1 -or [int]$rec.tail.base_sesno -ne $sesnos[$idx - 1]) { $broken += "e3d-io session $s BASESN $($rec.tail.base_sesno), wanted $($sesnos[$idx-1])" }
    }
    if ($broken.Count -eq 0) {
        Write-Output ("PASS T4c the {0} sessions {1}..{2} number contiguously, every BASESN back-references an existing session, and each e3d-io session is user e3dio based on the E3D session before it" -f $chain.Count, $chain[0].sesno, $chain[-1].sesno)
    } else {
        Write-Output 'FAIL T4c the session chain is broken'
        foreach ($b in $broken) { Write-Output "     $b" }
        $failures++
    }
}

# Q15 resets the DB from the original: the rounds' copy has been checked and is not reused.
Invoke-Q15

Write-Output ''
if ($failures) { Write-Output "$failures FAIL — logs in $logDir"; exit 1 }
Write-Output "all PASS — E3D 3.1 and e3d-io alternated $Rounds rounds on one ZONE with an unbroken session chain, and Q15 held (logs in $logDir)"
