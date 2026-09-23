<#
.SYNOPSIS
  The E3D gate for the write path, run unattended: write sessions with e3d-io into a
  copy of the AMS sample project, then have a real E3D 3.1 Design TTY session read them
  back and check what it prints (docs/plans/2026-09-20-dabacon-write-plan.md §7, Q8).

.DESCRIPTION
  1. Copies the sample project (all but the picture and drawing folders) to a scratch
     directory once, and resets its ams8000_0001 from the original before every run —
     the live project is never touched.
  2. Runs `e3d-write-box <copy db> --group --hunk --move --big --strings --text`: nine
     sessions — a BOX, a GPWL+GROU with reference members, two EQUIs whose
     DESC/DESP/UUIDIA are hunks (one chain across a page boundary), a FIXING with a
     type-19 string list inline and as a hunk, three BOXes under the second ZONE (two
     referenced by the GROU) then one moved to the third ZONE / one reordered / one
     unreferenced (T1a: move_to, reorder, reference members), a ZONE with 8 200 BOXes
     (the format word's member count saturated at 0x3FFF, plan §1.3 #11), an EQUI and a
     FIXING whose every value was typed as text and set by attribute name under E3D's
     dictionary (T1b: Transaction::set), and last a Chinese NAME on the BOX / DESC on the
     ZONE / session comment stored the way the header's FORMAT says (S5).
  3. Renders write_gate.mac.template with the refnos the bin printed and runs it through
     gen-model's l3_suite `--check-driver` (des.exe TTY, login SYSTEM/XXXXXX, MDB /ALL,
     project AMS pointed at the copy via --project-dir). The macro is read-only.
  4. Greps the per-question logs for the values written and prints PASS/FAIL per question.
  5. T1c, the merge: a second E3D session creates an EQUI (with a BOX in it) under the
     first ZONE and SAVEWORKs (write_gate_e3d_saves.mac.template — the one macro that
     writes); then `e3d-write-box --merge-from <the bin's last session>` opens a
     transaction at that older base and commits a merge onto E3D's session; then a third
     E3D session reads both back (write_gate_merge.mac.template). Exits 1 on any FAIL.

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
# The driver is a gen-model bin; when the default build is not there (gen-model's main
# worktree does not always compile against a neighbouring e3d-model WIP), take one built
# from a scratch worktree into its own target dir (2026-09-23: `.scratch\wt-f47\gen-model`,
# `CARGO_TARGET_DIR=D:\Rust\target-l3`, 3m45s).
if (-not (Test-Path $L3Exe)) {
    $fallback = @('D:\Rust\target-l3\debug\l3_suite.exe', 'D:\Rust\target\release\l3_suite.exe') | Where-Object { Test-Path $_ } | Select-Object -First 1
    if ($fallback) { Write-Output "l3_suite not at $L3Exe; using $fallback"; $L3Exe = $fallback }
}
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
    $binOut = & cargo run --quiet --bin e3d-write-box -- $copyDb --group --hunk --move --big --strings --text 2>&1 | Out-String
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
    BOX     = & $refno 'new BOX'
    GPWL    = & $refno 'new GPWL'
    GROU    = & $refno 'new GROU'
    EQUI1   = & $refno 'new EQUI 1'
    EQUI2   = & $refno 'new EQUI 2'
    FIXING  = & $refno 'new FIXING'
    ZONE    = & $refno 'owner ZONE'
    BOXM1   = & $refno 'new BOX A'
    BOXM2   = & $refno 'new BOX B'
    BOXM3   = & $refno 'new BOX C'
    ZONE2   = & $refno 'owner ZONE 2'
    ZONE3   = & $refno 'owner ZONE 3'
    BIGZONE = & $refno 'new ZONE big'
    STREQUI   = & $refno 'new EQUI str'
    STRFIXING = & $refno 'new FIXING str'
}
$spanning = [regex]::Match($binOut, 'hunk chains that cross a page boundary: (\d+)').Groups[1].Value
$bigMembers = [int][regex]::Match($binOut, 'big members\s*:\s*(\d+)').Groups[1].Value
# The text the FIXING's CREF was set from (the first ZONE's /NAME), which E3D prints back.
$crefText = [regex]::Match($binOut, 'CREF text\s*:\s*(\S+)').Groups[1].Value
# The bin's last session: the base the T1c merge transaction is opened at.
$lastSesno = [int](([regex]::Matches($binOut, 'wrote session (\d+)') | Select-Object -Last 1).Groups[1].Value)
Write-Output ("wrote: BOX {0} GPWL {1} GROU {2} EQUI {3} {4} FIXING {5}; hunk chains across a page: {6}" -f $ids.BOX, $ids.GPWL, $ids.GROU, $ids.EQUI1, $ids.EQUI2, $ids.FIXING, $spanning)
Write-Output ("       moves: BOX A {0} B {1} C {2} under ZONE 2 {3}, A moved to ZONE 3 {4}; big ZONE {5} with {6} BOXes" -f $ids.BOXM1, $ids.BOXM2, $ids.BOXM3, $ids.ZONE2, $ids.ZONE3, $ids.BIGZONE, $bigMembers)
Write-Output ("       by name: EQUI {0}, FIXING {1} (CREF = {2})" -f $ids.STREQUI, $ids.STRFIXING, $crefText)

# 3. The macro, rendered; then E3D.
New-Item -ItemType Directory -Force $logDir, $driverOut | Out-Null
Remove-Item (Join-Path $logDir '*.log') -Force -ErrorAction SilentlyContinue
$forward = { param([string]$p) $p -replace '\\', '/' }
# Render `$template` with `$vars` (plus {{LOGDIR}} / {{MACROLOG}}) to `$macroName` in the
# scratch dir and run it through one E3D TTY session; the driver's own output goes to a
# folder named after the macro. A macro that SAVEWORKs is "stateful" to the driver, which
# then wants the database file it is expected to advance (and a project label it does not
# read) and refuses to call the run done unless the file's session number moved.
function Invoke-E3D([string]$template, [string]$macroName, [hashtable]$vars, [switch]$stateful) {
    $macroPath = Join-Path $Scratch $macroName
    $out = Join-Path $Scratch ('driver-out-' + [IO.Path]::GetFileNameWithoutExtension($macroName))
    New-Item -ItemType Directory -Force $out | Out-Null
    $text = Get-Content (Join-Path $PSScriptRoot $template) -Raw
    $text = $text.Replace('{{LOGDIR}}', (& $forward $logDir)).Replace('{{MACROLOG}}', (& $forward ([IO.Path]::ChangeExtension($macroPath, '.log'))))
    foreach ($key in $vars.Keys) { $text = $text.Replace("{{$key}}", $vars[$key]) }
    Set-Content -Path $macroPath -Value $text -Encoding ASCII

    $driverArgs = @('--check-driver', $macroPath, '--project-dir', $copy, '--output', $out)
    if ($stateful) { $driverArgs += @('--target-db-file', $copyDb, '--aios-project', 'e3dio-write-gate') }
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
Invoke-E3D 'write_gate.mac.template' (Split-Path $macro -Leaf) $ids

# 4. The verdicts.
$failures = 0
# E3D writes ALPHA LOG in the process's ANSI code page (GBK on a Chinese Windows), not
# UTF-8; the ASCII questions read the same either way, the text question does not. Read a
# log as UTF-8 when it is valid UTF-8, else as the ANSI code page, and say which.
$script:logEncodings = @{}
function Read-LogText([string]$path) {
    $bytes = [IO.File]::ReadAllBytes($path)
    try {
        $text = (New-Object Text.UTF8Encoding($false, $true)).GetString($bytes)
        $script:logEncodings[(Split-Path $path -Leaf)] = 'utf-8'
        return $text
    } catch {
        $ansi = [Text.Encoding]::GetEncoding([Globalization.CultureInfo]::CurrentCulture.TextInfo.ANSICodePage)
        $script:logEncodings[(Split-Path $path -Leaf)] = $ansi.WebName
        return $ansi.GetString($bytes)
    }
}
# A `Q MEM` line: ` i TYPE n /NAME` or ` i TYPE n =word0/word1` — the list index, the type,
# the ordinal among that type, then the name or refno, glued to the ordinal once it has
# two digits (` 4 PIPE 4 /1RX-333`, `15 PIPE 15/1RX-311`, `8200 BOX 8200=24384/34691`).
$memberLine = '(?m)^\s*\d+\s+[A-Z][A-Z0-9]*\s+\d+\s*(=\d+/\d+|/\S+)\s*$'
function Check(
    [string]$question, [string]$log, [string[]]$expected,
    [string[]]$absent = @(),   # strings that must not appear
    [switch]$ordered,          # $expected must appear in this order
    [string]$lastMember = '',  # the last `Q MEM` line must contain this
    [int]$memberLines = -1     # exactly this many `Q MEM` lines
) {
    $path = Join-Path $logDir $log
    if (-not (Test-Path $path)) { Write-Output "FAIL $question — $log was never written (the block did not reach ALPHA LOG END)"; $script:failures++; return }
    $content = Read-LogText $path
    $missing = @($expected | Where-Object { $content -notmatch [regex]::Escape($_) })
    $present = @($absent | Where-Object { $content -match [regex]::Escape($_) })
    $errors = @([regex]::Matches($content, '(?m)^.*(ERROR|\(\d+,\d+\)).*$') | ForEach-Object { $_.Value })
    $problems = @()
    if ($ordered -and $missing.Count -eq 0) {
        $previous = -1
        foreach ($e in $expected) {
            $at = $content.IndexOf($e)
            if ($at -lt $previous) { $problems += "'$e' should come after the one before it" }
            $previous = $at
        }
    }
    $lines = @([regex]::Matches($content, $memberLine) | ForEach-Object { $_.Value.Trim() })
    if ($lastMember -and ($lines.Count -eq 0 -or $lines[-1] -notmatch [regex]::Escape($lastMember))) {
        $problems += "the last member line is '$(if ($lines.Count) { $lines[-1] } else { '<none>' })', wanted $lastMember"
    }
    if ($memberLines -ge 0 -and $lines.Count -ne $memberLines) {
        $problems += "$($lines.Count) member lines, wanted $memberLines"
    }
    if ($missing.Count -eq 0 -and $present.Count -eq 0 -and $errors.Count -eq 0 -and $problems.Count -eq 0) {
        Write-Output "PASS $question"
    } else {
        Write-Output "FAIL $question"
        foreach ($m in $missing) { Write-Output "     missing: $m" }
        foreach ($p in $present) { Write-Output "     must not appear: $p" }
        foreach ($p in $problems) { Write-Output "     $p" }
        foreach ($e in $errors) { Write-Output "     E3D said: $e" }
        $script:failures++
    }
}
$desc = { param([int]$n) "Description e3d-io hunk gate EQUI ${n}: DESC stored out of the record (type 22) @065 @070 @075 @080 @085 @090 @095 @100 @105 @110 @115" }
# The text session (plan S5) names the BOX, so E3D lists it by name from then on. The
# expected strings are spelled as code points so this file reads the same under any
# PowerShell encoding: /E3DIO-盒子, 反应堆厂房 -3.320m~±0.00m, 中文 NAME / DESC / 会话注释
# (e3d-write-box's TEXT_BOX_NAME / TEXT_ZONE_DESC / TEXT_COMMENT).
$cjk = { param([int[]]$points) -join ($points | ForEach-Object { [char]$_ }) }
$boxName = '/E3DIO-' + (& $cjk 0x76D2, 0x5B50)
$zoneDesc = (& $cjk 0x53CD, 0x5E94, 0x5806, 0x5382, 0x623F) + ' -3.320m~' + [char]0x00B1 + '0.00m'
$commentTail = (& $cjk 0x4E2D, 0x6587) + ' NAME / DESC / ' + (& $cjk 0x4F1A, 0x8BDD, 0x6CE8, 0x91CA)
Check 'Q1  BOX geometry and owner (session 1)' '10_box.log' @('Type BOX', 'Position E 1000mm N 2000mm U 3000mm', 'Xlength 300mm', 'Ylength 400mm', 'Zlength 500mm')
Check 'Q2  GROU lists ZONE and BOX as reference members (session 2)' '20_grou.log' @('Type GROU', 'ZONE 1', "BOX 1 $boxName", 'Type GPWL', "GROU 1 =$($ids.GROU)")
Check 'Q3a EQUI 1: DESC as a type-22 hunk' '31_equi1_desc.log' @((& $desc 1))
Check 'Q3b EQUI 1: DESP as a type-21 hunk (100 reals)' '32_equi1_desp.log' @('Desparam 1001.3 1002.3', '1100.3')
Check 'Q3c EQUI 1: UUIDIA as a type-20 hunk' '33_equi1_uuidia.log' @('Uuidia 11 12 13 14')
Check 'Q3d EQUI 2: DESC as a type-22 hunk' '36_equi2_desc.log' @((& $desc 2))
Check 'Q3e EQUI 2: DESP as a type-21 hunk whose chain crosses a page' '37_equi2_desp.log' @('Desparam 2001.3 2002.3', '2100.3')
Check 'Q3f EQUI 2: UUIDIA as a type-20 hunk' '38_equi2_uuidia.log' @('Uuidia 21 22 23 24')
Check 'Q4a FIXING: ROKEYS, a type-19 list stored inline' '41_fixing_rokeys.log' @('Rokeys', ' 1 K1', ' 2 K2')
Check 'Q4b FIXING: ROVALS, the same list as a type-23 hunk' '42_fixing_rovals.log' @('Rovals', ' 1 V1', ' 2 V2')
Check 'Q5  ZONE lists the four new members last' '50_zone.log' @("BOX 1 $boxName", "EQUI 1 =$($ids.EQUI1)", "EQUI 2 =$($ids.EQUI2)", "FIXING 1 =$($ids.FIXING)")
Check 'Q6  Q ATT walks every attribute of the EQUI and the FIXING' '60_equi2_att.log' @((& $desc 2), 'Desparam 2001.3')
Check 'Q6b Q ATT of the FIXING' '61_fixing_att.log' @('Type FIXING')
# `Q SESSION` prints the last saved session only — the text session, whose comment is Chinese.
Check 'Q7  E3D reads the session page (user e3dio, our comment, in Chinese)' '70_sessions.log' @('User: e3dio', $commentTail)
Check 'Q8a BOX: a Chinese NAME, record and table (last session)' '80_box_name.log' @("Name $boxName")
Check 'Q8b ZONE: a Chinese DESC with an escaped run inside plain text' '81_zone_desc.log' @("Description $zoneDesc")
Write-Output ("     (text logs decoded as: {0})" -f (($script:logEncodings.GetEnumerator() | Where-Object { $_.Key -match '^(70|8)' } | Sort-Object Key | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join ', '))
if ([int]$spanning -lt 1) { Write-Output 'FAIL     no hunk chain crossed a page boundary in this run, so the continuation link went unexercised'; $failures++ }
# T1a (plan §2 T1a: the gate's Q OWNER / Q MEMBERS questions). The moved BOX A says its
# owner is ZONE 3 and navigating OWNER lands there; ZONE 2 lists C before B and no A;
# ZONE 3 lists A last; the GROU gained A and lost B.
Check 'Q9a BOX A moved (move_to): Q OWNER is ZONE 3, OWNER navigates there' '90_move_owner.log' @('Name /E3DIO-MOVE-A', "=$($ids.ZONE3)") -absent @("=$($ids.ZONE2)")
Check 'Q9b ZONE 2 (reorder): lists C before B, A no longer' '91_move_zone2.log' @('/E3DIO-MOVE-C', '/E3DIO-MOVE-B') -ordered -absent @('/E3DIO-MOVE-A')
Check 'Q9c ZONE 3: lists A as its last member' '92_move_zone3.log' @('/E3DIO-MOVE-A') -lastMember '/E3DIO-MOVE-A'
Check 'Q9d GROU (reference members): A added, B removed, old members kept' '93_move_grou.log' @('ZONE 1', "BOX 1 $boxName", '/E3DIO-MOVE-A') -absent @('/E3DIO-MOVE-B')
# §1.3 #11: the member count of a 16 400-word member block is written 0x3FFF, so E3D
# measures the chain — every one of the 8 200 BOXes is listed, not `16 400 & 0x3FFF`.
Check "Q10 big ZONE: Q MEM lists all $bigMembers BOXes (format word count saturated)" '95_big_zone.log' @('Name /E3DIO-BIG-ZONE') -memberLines $bigMembers
# T1b (plan §2 T1b: the gate's Q19). Every value was typed as text and set by attribute
# name (Transaction::set) under E3D's dictionary; E3D must print back exactly what was
# typed — the strings here are e3d-write-box's STR_* constants.
Check 'Q11a EQUI set by name: DESC / POS / FUNC / PURP print back as typed' '96_str_equi.log' @('Name /E3DIO-STR-EQUI', 'Description e3d-io T1b: DESC typed as text and set by name through Transaction::set', 'Position E 1234mm N 2345mm U 3456mm', 'Function STRING-PATH', 'Purpose PIPE')
Check 'Q11b FIXING set by name: ROKEYS as a | list, BUIL true, NUMB 42, CREF from /NAME' '97_str_fixing.log' @('Name /E3DIO-STR-FIXING', 'Rokeys', ' 1 S1', ' 2 S2', ' 3 S3', 'Built true', 'Number 42', "Cref $crefText")

# 5. T1c (plan §2 T1c: the gate's Q20). E3D saves a session of its own — an EQUI it
# creates under the first ZONE, with a BOX in it — after the bin's last session; a
# transaction opened at that last session then commits: a merge onto E3D's session (E3D
# saves once more on quit, so the newest session is past the SAVEWORK's). E3D must read
# both, and the ZONE's list must be the merge of both edits: E3D's EQUI where E3D put it
# (first — `NEW` at the owner), the merged BOX anchored after the base's last member (its
# preceding sibling in the transaction's list, `sub_5B20CA0`), i.e. last.
try {
    Invoke-E3D 'write_gate_e3d_saves.mac.template' 'write_gate_e3d_saves.mac' @{ ZONE = $ids.ZONE } -stateful
} catch {
    # The driver refuses to call a stateful run done unless the file's session advanced;
    # the log below says what E3D did, so report rather than throw.
    Write-Output "     (E3D's saving session: $($_.Exception.Message))"
}
$savesLog = Join-Path $logDir 'e3d_saves.log'
$saves = if (Test-Path $savesLog) { Read-LogText $savesLog } else { '' }
if ($saves -notmatch 'E3DSAVE-SAVED') {
    Write-Output 'FAIL Q12  E3D did not save its own session (see e3d_saves.log); the merge was not attempted'
    foreach ($e in @([regex]::Matches($saves, '(?m)^.*(ERROR|E3DSAVE-\S+-ERROR|\(\d+,\d+\)).*$') | ForEach-Object { $_.Value })) { Write-Output "     E3D said: $e" }
    $failures++
} else {
    # The refnos E3D minted: the `Ref =` after each labelled line.
    $e3dEqui = [regex]::Match($saves, 'E3DSAVE-EQUI\s+Ref =(\d+/\d+)').Groups[1].Value
    $e3dBox = [regex]::Match($saves, 'E3DSAVE-BOX\s+Ref =(\d+/\d+)').Groups[1].Value
    $e3dSesno = [regex]::Match($saves, 'Last saved session number (\d+)').Groups[1].Value
    Push-Location $crate
    try {
        $mergeOut = & cargo run --quiet --bin e3d-write-box -- $copyDb --merge-from $lastSesno 2>&1 | Out-String
    } finally { Pop-Location }
    Set-Content -Path (Join-Path $Scratch 'e3d-write-box.merge.out.txt') -Value $mergeOut
    if ($LASTEXITCODE) {
        Write-Output "FAIL Q12  e3d-write-box --merge-from $lastSesno failed"
        Write-Output $mergeOut
        $failures++
    } else {
        $mergeBox = [regex]::Match($mergeOut, 'new BOX merge\s*:\s*(\d+/\d+)').Groups[1].Value
        $mergeSesno = [regex]::Match($mergeOut, 'wrote session (\d+)').Groups[1].Value
        $mergedOnto = [regex]::Match($mergeOut, 'merged onto\s*:\s*(\d+)').Groups[1].Value
        $reminted = [regex]::Match($mergeOut, 're-minted\s*:\s*(.*)').Groups[1].Value.Trim()
        Write-Output ("       merge: E3D saved session {0} (EQUI {1}, BOX {2}); the transaction at base {3} merged onto {4} as session {5}, its BOX {6}, re-minted {7}" -f $e3dSesno, $e3dEqui, $e3dBox, $lastSesno, $mergedOnto, $mergeSesno, $mergeBox, $reminted)
        # E3D saves again when it quits (T2 saw its "Saving Meta Data" sessions), so the
        # newest session the merge lands on may be past the SAVEWORK's number; it must not be
        # before it.
        if ([int]$mergedOnto -lt [int]$e3dSesno) {
            Write-Output "FAIL Q12  the merge went onto session $mergedOnto, before E3D's save $e3dSesno"; $failures++
        } elseif ([int]$mergedOnto -gt [int]$e3dSesno) {
            Write-Output ("       (E3D saved {0} more session(s) after its SAVEWORK, on quit; the merge went onto the newest, {1})" -f ([int]$mergedOnto - [int]$e3dSesno), $mergedOnto)
        }
        Invoke-E3D 'write_gate_merge.mac.template' 'write_gate_merge.mac' @{ ZONE = $ids.ZONE; ZONE2 = $ids.ZONE2; E3DBOX = $e3dBox; MERGEBOX = $mergeBox }
        Check 'Q12a the BOX E3D saved is still there after the merge, as E3D left it' '98_merge_e3d_box.log' @('Name /E3DIO-E3D-BOX', 'Owner /E3DIO-E3D-EQUI', 'Xlength 777mm')
        # E3D's `NEW EQUI` with the ZONE as the current element puts the EQUI **first** in the
        # ZONE's list (` 1 EQUI 1 /E3DIO-E3D-EQUI`), not last (2026-09-23, this run); the merged
        # BOX, anchored after the base's last member (`/E3DIO-STR-FIXING`, `sub_5B20CA0`), is
        # last — so the merged list is E3D's EQUI, the base's members, the merged BOX: one more
        # than E3D's list, two more than the base's.
        $zoneBefore = @([regex]::Matches((Read-LogText (Join-Path $logDir '50_zone.log')), $memberLine)).Count
        Check 'Q12b the first ZONE lists E3D''s EQUI (first, where NEW put it), the base''s members, then the merged BOX after its anchor' '99_merge_zone.log' @(' 1 EQUI 1 /E3DIO-E3D-EQUI', '/E3DIO-STR-FIXING', '/E3DIO-MERGE-BOX') -ordered -lastMember '/E3DIO-MERGE-BOX' -memberLines ($zoneBefore + 2)
        Check 'Q12c the merged BOX: name, owner, the values set by name' '99b_merge_box.log' @('Name /E3DIO-MERGE-BOX', "Owner $crefText", 'Xlength 555mm', 'Position E 5000mm N 5000mm U 5000mm')
        Check 'Q12d the second ZONE''s FUNC from the merged session' '99c_merge_zone2.log' @('Function MERGED-BY-E3DIO')
        Check 'Q12e the newest session is the merge, by e3dio, one past E3D''s' '99d_merge_session.log' @("Last saved session number $mergeSesno", 'User: e3dio')
    }
}

Write-Output ''
if ($failures) { Write-Output "$failures FAIL — logs in $logDir"; exit 1 }
Write-Output "all PASS — E3D 3.1 read every session e3d-io wrote (logs in $logDir)"
