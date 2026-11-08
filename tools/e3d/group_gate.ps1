<#
.SYNOPSIS
  The E3D gate for the *group* write path (GPWL -> GPSET -> GPITEM -> GPREF, plus a SCOSEL
  rules set): write the sessions with e3d-io into a copy of the APS sample project, then
  have a real E3D 3.1 Design TTY session read them back and check what it prints.

.DESCRIPTION
  `write_gate.ps1` already covers GPWL, but its member container is a GROU -- a
  reference-list noun reached through `add_reference_member`. A GPSET is not that: its
  members are GPITEMs whose GPREF names the real element
  (docs/plans/2026-11-08-gpwl-gpset-m5-write-side.md §2: "搞混的后果是成员链上出现真身
  refno，而 E3D 展开时找不到 GPITEM，分组变空"). No E3D session has yet opened a GPSET this
  code wrote, so this gate is the one that says the GPSET shape is real.

  What it writes, into `AvevaPlantSample` (the project that actually has GPWL/GPSET --
  `AvevaMarineSample` has no GPWL under a WORLD at all):

    1. GPWL /E3DIO-GPWL under the WORL.
    2. GPSET /E3DIO-GPSET-ITEMS under it, holding two GPITEMs: one whose GPREF names an
       EQUI in the same database, one whose GPREF names an element in *another* database
       (the cross-db case the read side already models).
    3. GPSET /E3DIO-GPSET-RULES under it, holding one GPITEM and a SCOSEL rules set
       written through `set_int_array_hunk`.

  Then a read-only TTY macro asks E3D about all of it: the GPWL's and both GPSETs'
  REF/TYPE/OWNER/MEM. The point of the gate is the GPSET's `Q MEM`: if E3D expands the
  group it prints the real elements, and if the GPITEM/GPREF shape is wrong it prints
  nothing or the GPITEMs.

  Two preconditions this script *checks and reports*, because a PASS without them would be
  meaningless:

    * `SCOSEL` is only written when `encode(decode(sample)) == sample` holds over the 27
      real SCOSELs in `tests/fixtures/group-contract/scosel-real-samples.json`
      (plan §3.3: a round-trip unit test proves self-consistency, not that E3D accepts the
      words). While that check fails, the rules set is written as the *sample's own bytes*
      copied verbatim -- not re-encoded -- and the gate says so.
    * The cross-db target must really be in another database, or that half of the gate is
      not being exercised.

.PARAMETER SkipTty
  Write the sessions and read them back with e3d-io only, skipping the E3D TTY half. Use
  it where no E3D install is present; the gate then cannot say PASS overall.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File tools/e3d/group_gate.ps1

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File tools/e3d/group_gate.ps1 -SkipTty
#>
[CmdletBinding()]
param(
    [string]$Project = 'D:\AVEVA\Projects\E3D3.1\AvevaPlantSample',
    [string]$Scratch = 'D:\e3dio-group-gate',
    [string]$GenModel = 'D:\work\plant-code\old\gen-model',
    [string]$L3Exe = 'D:\Rust\target\debug\l3_suite.exe',
    [string]$InstallDir = 'E:\reverse\e3d\shadow_e3d31_gen_model_test',
    # The database the gate writes into. It must be a design database that the project's
    # MDB actually opens: `aps250204_0001` is the one holding the 27 real GPSETs (and the
    # GPWLs above them). `aps250218_0001` looks similar but is a bare WORL + MNUM library --
    # writes there succeed and read back through e3d-io, while E3D reports every refno in it
    # as undefined because the MDB never opens it.
    [string]$DbRelative = 'aps000\aps250204_0001',
    # E3D project code, passed to `des.exe` and used by the launcher to name `<CODE>000`.
    # The sample project is `AvevaPlantSample`, whose databases live in `aps000`, so the
    # code is APS; leaving it at the AMS default makes the launch script check `ams000`.
    [string]$ProjectCode = 'APS',
    # The design element the same-db GPITEM's GPREF points at, as `word0/word1`. The written
    # database (`aps250204`) holds no design element of its own, so this comes from
    # `aps250110_0001`: `2013286654/2405` is `/KIOSK_PUMP_STANDBY` (EQUI), verified by
    # navigating to it in a TTY session (Q4 in the gate).
    [string]$Target = '2013286654/2405',
    # Cross-db target, written as `word0/word1` (the form E3D's `=A/B` navigation takes).
    # A `/NAME` lookup would only work inside the written database's own NAME table, and
    # the cross-db case is precisely the one where the target is elsewhere.
    [string]$CrossDbTarget = '',
    [switch]$RefreshCopy,
    [switch]$SkipTty
)

$ErrorActionPreference = 'Stop'
$gateClock = [Diagnostics.Stopwatch]::StartNew()
$sourceHash = (Get-FileHash (Join-Path $Project $DbRelative) -Algorithm SHA256).Hash
if (-not $env:L3_E3D_INSTALL_DIR) { $env:L3_E3D_INSTALL_DIR = $InstallDir }
$crate = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$copy = Join-Path $Scratch (Split-Path $Project -Leaf)
$copyDb = Join-Path $copy $DbRelative
$logDir = Join-Path $Scratch 'logs'
$macro = Join-Path $Scratch 'group_gate.mac'
$driverOut = Join-Path $Scratch 'driver-out'
$opsDir = Join-Path $PSScriptRoot 'group_gate_ops'
$opsOut = Join-Path $Scratch 'ops'
$samplesFixture = Join-Path $GenModel 'tests\fixtures\group-contract\scosel-real-samples.json'

foreach ($required in @($Project, (Join-Path $Project $DbRelative), $GenModel, $opsDir, $samplesFixture)) {
    if (-not (Test-Path $required)) { throw "missing: $required" }
}
if (-not $SkipTty) {
    foreach ($required in @($L3Exe, (Join-Path $env:L3_E3D_INSTALL_DIR 'des.exe'))) {
        if (-not (Test-Path $required)) { throw "missing: $required (pass -SkipTty to run the write half only)" }
    }
}
$failures = 0

function Invoke-Native([scriptblock]$command) {
    $eap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $lines = & $command 2>&1 | ForEach-Object { if ($_ -is [Management.Automation.ErrorRecord]) { $_.Exception.Message } else { [string]$_ } }
        $code = $LASTEXITCODE
    } finally { $ErrorActionPreference = $eap }
    [pscustomobject]@{ Output = (($lines | Where-Object { $_ -ne $null }) -join "`n"); ExitCode = $code }
}

# 1. The project copy. The whole project, not just the one database: the cross-db GPREF is
#    resolved through the same copy, and E3D's own expansion walks it too.
if ($RefreshCopy -or -not (Test-Path (Join-Path $copy 'aps000\apssys'))) {
    New-Item -ItemType Directory -Force $copy | Out-Null
    robocopy $Project $copy /E /XD apsspic apsdrg /R:1 /W:1 /NFL /NDL /NJH /NJS /NP | Out-Null
    if ($LASTEXITCODE -ge 8) { throw "robocopy failed with $LASTEXITCODE" }
    Write-Output "project copied to $copy"
}
Copy-Item (Join-Path $Project $DbRelative) $copyDb -Force
Write-Output "reset $copyDb from the original"

Push-Location $crate
try {
    $build = Invoke-Native { cargo build --quiet --bin e3d-group-gate }
    if ($build.ExitCode) { Write-Output $build.Output; throw 'cargo build --bin e3d-group-gate failed' }
} finally { Pop-Location }
$utf8 = New-Object Text.UTF8Encoding($false)

# 2. Preconditions, reported before anything is written.

# 2a. The SCOSEL precondition (plan §3.3). The gate does not re-encode: until
#     `encode(decode(x)) == x` holds over all 27 real samples, the only words known to be
#     E3D's own are the sample's bytes, so those are what `e3d-group-gate` writes.
$samples = [IO.File]::ReadAllText($samplesFixture, $utf8) | ConvertFrom-Json
if (-not $samples.samples -or $samples.samples.Count -ne 27) {
    throw "$samplesFixture does not hold the 27 samples (found $($samples.samples.Count))"
}
Write-Output ("SCOSEL: the rules set is written from the sample's own bytes ({0} words, {1} {2})" -f $samples.samples[0].words.Count, $samples.samples[0].refno, $samples.samples[0].name)
Write-Output '        the re-encode precondition (encode(decode(sample)) == sample over all 27) is NOT yet met, so this gate proves the write path, not the encoder'

# 2b. The cross-db half is only exercised when a target in another library is given.
if (-not $CrossDbTarget) {
    Write-Output 'cross-db target: none given (-CrossDbTarget); the cross-db half is skipped'
}

# 3. The session: the write half, through `e3d-group-gate` (the facade, not the CLI --
#    `GPREF` is not in E3D's dictionary, so the CLI's `attrs` path refuses it and only
#    `Transaction::set_explicit_refno` can write it).
Write-Output 'sessions:'
$gateJson = Join-Path $Scratch 'gate.json'
Remove-Item $gateJson -Force -ErrorAction SilentlyContinue
$gateArgs = @('run', '--quiet', '--bin', 'e3d-group-gate', '--', '--db', $copyDb, '--out', $gateJson,
              '--scosel', (($samples.samples[0].words) -join ' '))
# `aps250204` holds only WORL / MNUM / GPWL / GPSET, so the same-db GPREF needs a target
# from a library that has design elements. Without it E3D answers "reference to an
# illegal element" and Q4 cannot pass for any reason of ours.
if ($Target) { $gateArgs += @('--target', $Target) }
if ($CrossDbTarget) { $gateArgs += @('--cross-db', $CrossDbTarget.TrimStart('=')) }
Push-Location $crate
try { $written = Invoke-Native { & cargo @gateArgs } } finally { Pop-Location }
if ($written.ExitCode -or -not (Test-Path $gateJson)) {
    Write-Output 'FAIL the group session was refused:'; Write-Output $written.Output; $failures++
    $ids = $null
} else {
    $ids = [IO.File]::ReadAllText($gateJson, $utf8) | ConvertFrom-Json
    Write-Output ("  GPWL {0}  GPSET-items {1}  GPSET-rules {2}" -f $ids.ref.gpwl, $ids.ref.gpset_items, $ids.ref.gpset_rules)
    Write-Output ("  GPITEM {0} -> {1}   GPITEM {2} -> {3}   session {4}" -f $ids.ref.gpitem_here, $ids.ref.target_here, $ids.gpitem_cross, $ids.target_cross, $ids.sesno)
}

# 4. The TTY half.
if ($SkipTty) {
    Write-Output 'TTY half skipped (-SkipTty): this run does NOT establish that E3D accepts the GPSET shape'
} elseif ($ids) {
    New-Item -ItemType Directory -Force $logDir, $driverOut | Out-Null
    Remove-Item (Join-Path $logDir '*.log') -Force -ErrorAction SilentlyContinue
    $forward = { param([string]$p) $p -replace '\\', '/' }
    $text = Get-Content (Join-Path $PSScriptRoot 'group_gate.mac.template') -Raw
    $text = $text.Replace('{{LOGDIR}}', (& $forward $logDir))
    # The driver reads `<macro name>.log` beside the macro and fails the whole run without
    # it; the per-question logs live in `logs/` and are written first.
    $text = $text.Replace('{{QUERYLOG}}', (& $forward ($macro -replace '\.mac$', '.log')))
    $vars = @{
        GPWL         = $ids.ref.gpwl
        GPSET_ITEMS  = $ids.ref.gpset_items
        GPSET_RULES  = $ids.ref.gpset_rules
        GPITEM_HERE  = $ids.ref.gpitem_here
        TARGET_HERE  = $ids.ref.target_here
    }
    foreach ($key in $vars.Keys) { $text = $text.Replace("{{$key}}", $vars[$key]) }
    if ($text -match '\{\{[^}]+\}\}') { throw "unresolved macro placeholder: $($Matches[0])" }
    Set-Content -Path $macro -Value $text -Encoding ASCII

    $driverArgs = @('--check-driver', $macro, '--project-dir', $copy, '--output', $driverOut,
                    '--e3d-project', $ProjectCode)
    $previous = $env:L3_ALLOW_EXISTING_E3D_SESSION
    $env:L3_ALLOW_EXISTING_E3D_SESSION = '1'
    Push-Location $GenModel
    try {
        $driver = Invoke-Native { & $L3Exe @driverArgs }
        [IO.File]::WriteAllText((Join-Path $driverOut 'l3_suite.stdout.txt'), $driver.Output, $utf8)
    } finally {
        Pop-Location
        if ($null -eq $previous) { Remove-Item Env:\L3_ALLOW_EXISTING_E3D_SESSION -ErrorAction SilentlyContinue } else { $env:L3_ALLOW_EXISTING_E3D_SESSION = $previous }
    }
    if ($driver.ExitCode) { Write-Output "FAIL the E3D TTY driver failed ($($driver.ExitCode)); see $driverOut"; Write-Output ($driver.Output -split "`n" | Select-Object -Last 20); $failures++ }

    # E3D writes ALPHA LOG in the process's ANSI code page, not UTF-8; these questions are
    # ASCII, so either reading works, but read it the same way the write gate does.
    function Read-LogText([string]$path) {
        $bytes = [IO.File]::ReadAllBytes($path)
        try { return (New-Object Text.UTF8Encoding($false, $true)).GetString($bytes) }
        catch {
            $ansi = [Text.Encoding]::GetEncoding([Globalization.CultureInfo]::CurrentCulture.TextInfo.ANSICodePage)
            return $ansi.GetString($bytes)
        }
    }
    $memberLine = '(?m)^\s*\d+\s+[A-Z][A-Z0-9]*\s+\d+\s*(=\d+/\d+|/\S+)\s*$'
    function Check([string]$question, [string]$log, [string[]]$expected, [string[]]$absent = @(), [string[]]$memberTypes = @()) {
        $path = Join-Path $logDir $log
        if (-not (Test-Path $path)) {
            Write-Output "FAIL $question -- $log was never written (the block did not reach ALPHA LOG END)"
            $script:failures++
            return
        }
        $content = Read-LogText $path
        $missing = @($expected | Where-Object { $content -notlike "*$_*" })
        $present = @($absent | Where-Object { $content -like "*$_*" })
        $navError = [regex]::Match($content, '(?m)^\$\$ GATE-\d+-.*NAV-ERROR (.+)$')
        if ($missing.Count -or $present.Count -or $navError.Success) {
            Write-Output "FAIL $question"
            foreach ($m in $missing) { Write-Output "     missing: $m" }
            foreach ($p in $present) { Write-Output "     must not appear: $p" }
            if ($navError.Success) { Write-Output "     E3D said: $($navError.Groups[1].Value)" }
            $script:failures++
            return
        }
        if ($memberTypes.Count) {
            $lines = [regex]::Matches($content, $memberLine)
            foreach ($type in $memberTypes) {
                if (-not ($lines | Where-Object { $_.Value -match "\s$type\s" })) {
                    Write-Output "FAIL $question -- no `Q MEM` line of type $type"
                    $script:failures++
                    return
                }
            }
        }
        Write-Output "PASS $question"
    }

    Check 'Q0  the session reads the copied project' '00_project.log' @('GATE-00-END')
    # E3D prints a GPWL member by name (`1 GPSET 1 /E3DIO-GPWL/ITEMS`), and a GPSET member
    # by refno (`1 GPITEM 1 =<ref>`). Both were measured on APS 250204 (2026-11-08); the
    # two levels genuinely differ, so the two assertions differ too.
    Check 'Q1  the GPWL is under the WORL and lists the two GPSETs' '10_gpwl.log' @('Type GPWL', 'GPSET 1 /E3DIO-GPWL/ITEMS', 'GPSET 2 /E3DIO-GPWL/RULES')
    # What E3D actually does, measured on APS 250204 (2026-11-08): a GPSET's `Q MEM`
    # prints `1 GPITEM 1 =<ref>` -- the GPITEM itself, NOT the element its GPREF names.
    # So the assertion is "the member line is a GPITEM pointing at the refno we wrote",
    # not "the real element appears". `Q ATT` on the GPITEM prints Name/Type/Lock/Owner/
    # Description/SItem and no GPREF at all: GPREF is not in E3D's dictionary, which is
    # why `Q GPREF` crashes the session (0xC0000005) and must never be asked.
    Check 'Q2  the GPSET lists its GPITEM member at the refno the write produced' '20_gpset_items.log' @('Type GPSET', "GPITEM 1 =$($ids.ref.gpitem_here)")
    Check 'Q3  the GPITEM is a GPITEM, owned by the GPSET' '30_gpitem.log' @('Type GPITEM', 'Owner /E3DIO-GPWL/ITEMS')
    # The target question is only answerable when the GPREF points at a real design
    # element. A library that holds none (`aps250204`) needs `--target`; without it the
    # gate points the GPREF at a system noun and E3D answers "reference to an illegal
    # element", which is a write-side problem, not an E3D one.
    Check 'Q4  the target is a design element E3D accepts' '40_target.log' @('GATE-40-END') -absent @('NAV-ERROR')
    Check 'Q5  the rules set lists its explicit GPITEM member' '50_gpset_rules.log' @('Type GPSET')
} else {
    Write-Output 'TTY half skipped: the write session was refused, so there is nothing to read back'
}

$gateClock.Stop()
if ($failures) { Write-Output ("{0} check(s) failed; logs in {1}" -f $failures, $logDir); exit 1 }
if ($SkipTty) {
    # Nothing here says E3D accepts the shape; say what was and was not established.
    Write-Output ("write half PASS in {0:N0}s — the sessions were written; the TTY half did not run, so the GPSET shape is NOT yet verified against E3D" -f $gateClock.Elapsed.TotalSeconds)
    exit 0
}
Write-Output ("all PASS in {0:N0}s — E3D 3.1 read the group sessions e3d-group-gate wrote (logs in {1})" -f $gateClock.Elapsed.TotalSeconds, $logDir)
