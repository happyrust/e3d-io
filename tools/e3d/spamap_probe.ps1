<#
.SYNOPSIS
  One SPAMAP probe against a real E3D 3.1 Design TTY session (task 7.3 of
  .kiro/specs/write-path-phase-3): E3D makes one change in the write gate's project copy
  and SAVEWORKs, and the watched refnos' SPAMAP entries are read before and after.

.DESCRIPTION
  powershell -NoProfile -ExecutionPolicy Bypass -File tools/e3d/spamap_probe.ps1 <probe.pml>
  powershell -NoProfile -ExecutionPolicy Bypass -File tools/e3d/spamap_probe.ps1 -SelfCheck

  A probe file (tools/e3d/spamap_probes/*.pml) is the PML E3D runs before the SAVEWORK,
  headed by directive lines:
    -- watch: 24384/24777 24384/24778     refnos whose entries are compared (required)
    -- setup: tools/e3d/spamap_probes/x.json
                                          an `e3d-write apply` instruction file written
                                          first, as e3d-io's own session (USER e3dio);
                                          the refnos it mints fill {{SETUP0}}, {{SETUP1}}, ...
                                          in the body and in the watch list
    -- expect: 24384/24777=rewritten      verdicts the run must give (exit 1 otherwise)
    -- comment: text                      the SAVEWORK comment
  1. Resets ams8000_0001 of the gate's project copy (tools/e3d/write_gate.ps1; copied
     there once if missing) from -Source, by default the installed original, which is
     only read. The probe never touches the original or D:\e3dio-fixtures.
  2. Runs the setup, if any.
  3. Reads the newest session and each watched refno's entries (`e3d-descriptor spamap`).
  4. Renders the body into one ALPHA LOG block that ends with SAVEWORK and Q SESSION and
     runs it through gen-model's l3_suite --check-driver (des.exe TTY, login SYSTEM). The
     driver calls a run that SAVEWORKs done only when the file's session moved; when it
     did not (a SAVEWORK E3D refused), the log is read all the same and its errors kept.
     E3D on this host no longer leaves the TTY on the wrapper's QUIT (since 2026-10-03,
     read-only macros too), which would hold the driver for its 20-minute timeout: once
     the wrapper has written L3-DONE, E3D gets -QuitGraceSeconds to exit, then the
     des.exe the driver started is stopped. The driver judges by its markers, not the
     exit code, and reads that as "completed, then exited dirty".
  5. Reads the entries again in the session the SAVEWORK wrote and gives each refno a
     verdict: rewritten (entries differ), same (identical), appeared, vanished, none.
     E3D's SAVEWORK session is E3D's own (its USER is the Windows account, not e3dio),
     so its elements count as E3D_Authored_Element.
  The whole table is compared as well: every refno whose entries the SAVEWORK session
  rewrote, added or dropped is listed, watched or not.
  Writes <Scratch>\probe\<name>\result.json and prints a summary. -SelfCheck runs every
  spamap_probes/selfcheck_*.pml, one probe each, the two known cases of 2026-09-22
  (session 648): selfcheck_box_xlen, E3D sets the XLEN of the native BOX 24384/24777,
  whose entry must be rewritten (requirement 5.2); selfcheck_e3dio_box_xlen, E3D sets the
  XLEN of a BOX e3d-io wrote in the setup session, which must gain its entry.
#>
[CmdletBinding()]
param(
    [Parameter(Position = 0)][string]$ProbeFile = '',
    [switch]$SelfCheck,
    [string]$Source = '',
    [string]$Project = 'D:\AVEVA\Projects\E3D3.1\AvevaMarineSample',
    [string]$Scratch = 'D:\e3dio-gate',
    [string]$GenModel = 'D:\work\plant-code\old\gen-model',
    [string]$L3Exe = 'D:\Rust\target\debug\l3_suite.exe',
    [string]$DbRelative = 'ams000\ams8000_0001',
    [int]$QuitGraceSeconds = 20
)

$ErrorActionPreference = 'Stop'
if ($SelfCheck) {
    $forwarded = @()
    foreach ($key in $PSBoundParameters.Keys) { if ($key -notin 'SelfCheck', 'ProbeFile') { $forwarded += @("-$key", [string]$PSBoundParameters[$key]) } }
    $failed = @()
    foreach ($probe in @(Get-ChildItem (Join-Path $PSScriptRoot 'spamap_probes') -Filter 'selfcheck_*.pml' | Sort-Object Name)) {
        & powershell -NoProfile -ExecutionPolicy Bypass -File $PSCommandPath $probe.FullName @forwarded
        if ($LASTEXITCODE) { $failed += $probe.BaseName }
    }
    if ($failed.Count) { Write-Output "SELF-CHECK FAIL: $($failed -join ', ')"; exit 1 }
    Write-Output 'SELF-CHECK PASS'
    exit 0
}
if (-not $ProbeFile) { throw 'usage: tools/e3d/spamap_probe.ps1 <probe.pml> | -SelfCheck' }
if (-not (Test-Path $L3Exe)) {
    $fallback = @('D:\Rust\target-l3\debug\l3_suite.exe', 'D:\Rust\target\release\l3_suite.exe') | Where-Object { Test-Path $_ } | Select-Object -First 1
    if ($fallback) { $L3Exe = $fallback }
}
$crate = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$ProbeFile = (Resolve-Path $ProbeFile).Path
if (-not $Source) { $Source = Join-Path $Project $DbRelative }
foreach ($required in @($ProbeFile, $Source, $Project, $GenModel, $L3Exe)) {
    if (-not (Test-Path $required)) { throw "missing: $required" }
}
$name = [IO.Path]::GetFileNameWithoutExtension($ProbeFile)
$copy = Join-Path $Scratch (Split-Path $Project -Leaf)
$copyDb = Join-Path $copy $DbRelative
$runDir = Join-Path $Scratch "probe\$name"
$utf8 = New-Object Text.UTF8Encoding($false)
$forward = { param([string]$p) $p -replace '\\', '/' }

# Run a native command with stderr folded into its text (see write_gate.ps1).
function Invoke-Native([scriptblock]$command) {
    $eap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $lines = & $command 2>&1 | ForEach-Object { if ($_ -is [Management.Automation.ErrorRecord]) { $_.Exception.Message } else { [string]$_ } }
        $code = $LASTEXITCODE
    } finally { $ErrorActionPreference = $eap }
    [pscustomobject]@{ Output = (($lines | Where-Object { $_ -ne $null }) -join "`n"); ExitCode = $code }
}
# E3D writes ALPHA LOG in the ANSI code page unless the text is plain ASCII (see write_gate.ps1).
function Read-LogText([string]$path) {
    if (-not (Test-Path $path)) { return '' }
    $bytes = [IO.File]::ReadAllBytes($path)
    try { return (New-Object Text.UTF8Encoding($false, $true)).GetString($bytes) }
    catch { return [Text.Encoding]::GetEncoding([Globalization.CultureInfo]::CurrentCulture.TextInfo.ANSICodePage).GetString($bytes) }
}

# The probe file: directives, then the body.
$watch = @(); $expect = [ordered]@{}; $setup = ''; $comment = "e3d-io SPAMAP probe $name"
$body = New-Object Collections.Generic.List[string]
foreach ($line in [IO.File]::ReadAllLines($ProbeFile, $utf8)) {
    if ($line -match '^--\s*watch:\s*(.+)$') { $watch += @($Matches[1].Trim() -split '[\s,]+' | Where-Object { $_ }) }
    elseif ($line -match '^--\s*setup:\s*(.+)$') { $setup = $Matches[1].Trim() }
    elseif ($line -match '^--\s*comment:\s*(.+)$') { $comment = $Matches[1].Trim() }
    elseif ($line -match '^--\s*expect:\s*(.+)$') {
        foreach ($pair in ($Matches[1].Trim() -split '[\s,]+' | Where-Object { $_ })) {
            $refno, $verdict = $pair -split '=', 2
            $expect[$refno] = $verdict
        }
    }
    else { $body.Add($line) }
}
if ($watch.Count -eq 0) { throw "$ProbeFile has no '-- watch:' line" }
if ($comment -match "'") { throw "the SAVEWORK comment may not hold a quote: $comment" }

# 1. The copy, and the database reset.
if (-not (Test-Path (Join-Path $copy 'ams000\amssys'))) {
    New-Item -ItemType Directory -Force $copy | Out-Null
    robocopy $Project $copy /E /XD amspic amsdrg /R:1 /W:1 /NFL /NDL /NJH /NJS /NP | Out-Null
    if ($LASTEXITCODE -ge 8) { throw "robocopy failed with $LASTEXITCODE" }
}
Copy-Item $Source $copyDb -Force
$sourceHash = (Get-FileHash $Source -Algorithm SHA256).Hash
New-Item -ItemType Directory -Force $runDir | Out-Null
Remove-Item (Join-Path $runDir '*') -Recurse -Force -ErrorAction SilentlyContinue

# The tools, built where cargo puts them (CARGO_TARGET_DIR is honoured).
Push-Location $crate
try {
    $build = Invoke-Native { cargo build --quiet --release --bin e3d-descriptor --bin e3d-write }
    if ($build.ExitCode) { Write-Output $build.Output; throw 'cargo build failed' }
    $eap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { $metaJson = & cargo metadata --format-version 1 --no-deps 2>$null | Out-String } finally { $ErrorActionPreference = $eap }
    $targetDir = ($metaJson | ConvertFrom-Json).target_directory
} finally { Pop-Location }
$descriptor = Join-Path $targetDir 'release\e3d-descriptor.exe'
$writer = Join-Path $targetDir 'release\e3d-write.exe'
function Invoke-Descriptor([string[]]$arguments) {
    $eap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { $out = & $descriptor @arguments 2>$null; $code = $LASTEXITCODE } finally { $ErrorActionPreference = $eap }
    if ($code) { throw "e3d-descriptor $($arguments -join ' ') failed ($code)" }
    ($out -join "`n") | ConvertFrom-Json
}
function Get-Entries([string]$refno, [int]$sesno) {
    $read = Invoke-Descriptor @('spamap', '--db', $copyDb, '--session', "$sesno", '--refno', $refno)
    if ($read.spamap -eq 'no_table') { return @() }
    @($read.entries | ForEach-Object { , @($_) })
}
function Get-Newest { [int](Invoke-Descriptor @('sessions', '--db', $copyDb)).newest }
# The whole table as refno -> its entries' text (sorted), for the table-wide comparison.
function Get-Table([int]$sesno) {
    $read = Invoke-Descriptor @('spamap', '--db', $copyDb, '--session', "$sesno")
    $lists = @{}
    if ($read.spamap -ne 'no_table') {
        foreach ($entry in $read.entries) {
            $refno = "$($entry[1])/$($entry[2])"
            if (-not $lists.ContainsKey($refno)) { $lists[$refno] = New-Object Collections.Generic.List[string] }
            $lists[$refno].Add('[' + ($entry -join ',') + ']')
        }
    }
    $byRefno = @{}
    foreach ($refno in $lists.Keys) { $byRefno[$refno] = (@($lists[$refno]) | Sort-Object) -join ' ' }
    $byRefno
}

# 2. The setup session, written by e3d-io.
$setupInfo = $null
$fill = @{}
if ($setup) {
    $setupPath = Join-Path $crate $setup
    $report = Join-Path $runDir 'setup.report.json'
    $run = Invoke-Native { & $writer apply --db $copyDb --ops $setupPath --report $report }
    [IO.File]::WriteAllText((Join-Path $runDir 'setup.out.txt'), $run.Output, $utf8)
    if ($run.ExitCode) { Write-Output $run.Output; throw "e3d-write apply $setup failed ($($run.ExitCode))" }
    $parsed = [IO.File]::ReadAllText($report, $utf8) | ConvertFrom-Json
    $minted = @($parsed.ops | ForEach-Object { $_.refno })
    for ($i = 0; $i -lt $minted.Count; $i++) { $fill["SETUP$i"] = [string]$minted[$i] }
    $setupInfo = [ordered]@{ ops = $setup; session = [int]$parsed.commit.written.sesno; refnos = $minted }
}
function Expand([string]$text) {
    foreach ($key in $fill.Keys) { $text = $text.Replace("{{$key}}", $fill[$key]) }
    if ($text -match '\{\{[A-Z0-9]+\}\}') { throw "$name still has a placeholder: $($Matches[0])" }
    $text
}
$watch = @($watch | ForEach-Object { Expand $_ })
$expanded = [ordered]@{}
foreach ($key in $expect.Keys) { $expanded[(Expand $key)] = $expect[$key] }
$expect = $expanded

# 3. Before.
$beforeSesno = Get-Newest
$before = @{}
foreach ($refno in $watch) { $before[$refno] = @(Get-Entries $refno $beforeSesno) }
$tableBefore = Get-Table $beforeSesno

# 4. E3D.
$bodyLog = Join-Path $runDir 'body.log'
$macroPath = Join-Path $runDir "$name.mac"
$macroLog = [IO.Path]::ChangeExtension($macroPath, '.log')
$macro = New-Object Text.StringBuilder
[void]$macro.AppendLine("ALPHA LOG `"$(& $forward $bodyLog)`" OVER")
[void]$macro.AppendLine("`$P PROBE-BEGIN $name")
foreach ($line in $body) { [void]$macro.AppendLine((Expand $line)) }
[void]$macro.AppendLine('$P PROBE-SAVE')
[void]$macro.AppendLine("SAVEWORK '$comment'")
[void]$macro.AppendLine('handle any')
[void]$macro.AppendLine('  $P PROBE-SAVE-ERROR $!!error.text')
[void]$macro.AppendLine('elsehandle none')
[void]$macro.AppendLine('  $P PROBE-SAVED')
[void]$macro.AppendLine('  Q SESSION')
[void]$macro.AppendLine('endhandle')
[void]$macro.AppendLine('$P PROBE-END')
[void]$macro.AppendLine('ALPHA LOG END')
[void]$macro.AppendLine("ALPHA LOG `"$(& $forward $macroLog)`" OVER")
[void]$macro.AppendLine("`$P PROBE-DONE $name")
[void]$macro.AppendLine('ALPHA LOG END')
Set-Content -Path $macroPath -Value $macro.ToString() -Encoding ASCII

$driverOut = Join-Path $runDir 'driver-out'
New-Item -ItemType Directory -Force $driverOut | Out-Null
$driverArgs = @('--check-driver', $macroPath, '--project-dir', $copy, '--output', $driverOut, '--target-db-file', $copyDb, '--aios-project', 'e3dio-spamap-probe')
$driverStdout = Join-Path $runDir 'driver.stdout.txt'
$driverStderr = Join-Path $runDir 'driver.stderr.txt'
$previous = $env:L3_ALLOW_EXISTING_E3D_SESSION
$env:L3_ALLOW_EXISTING_E3D_SESSION = '1'
try {
    $driver = Start-Process -FilePath $L3Exe -ArgumentList ($driverArgs | ForEach-Object { "`"$_`"" }) -WorkingDirectory $GenModel -NoNewWindow -PassThru -RedirectStandardOutput $driverStdout -RedirectStandardError $driverStderr
} finally {
    if ($null -eq $previous) { Remove-Item Env:\L3_ALLOW_EXISTING_E3D_SESSION -ErrorAction SilentlyContinue } else { $env:L3_ALLOW_EXISTING_E3D_SESSION = $previous }
}
$null = $driver.Handle
# The driver's wrapper folders are output\l3-suite\driver-<its pid>-<time> under gen-model,
# each with the des.exe pid it launched; a guarded retry adds a second one.
$wrappers = Join-Path $GenModel 'output\l3-suite'
function Get-Wrappers { @(Get-ChildItem $wrappers -Directory -Filter "driver-$($driver.Id)-*" -ErrorAction SilentlyContinue) }
# [string]: a file not there yet reads as AutomationNull, which -notmatch turns into an
# empty (false) array instead of $true.
function Read-Marker([IO.DirectoryInfo]$dir, [string]$file) { [string](Get-Content (Join-Path $dir.FullName $file) -Raw -ErrorAction SilentlyContinue) }
function Stop-E3dSession([IO.DirectoryInfo]$dir) {
    $desPid = 0
    if (-not [int]::TryParse((Read-Marker $dir 'driver.pid').Trim(), [ref]$desPid)) { return $false }
    if ((Get-Process -Id $desPid -ErrorAction SilentlyContinue).ProcessName -ne 'des') { return $false }
    Get-CimInstance Win32_Process -Filter "ParentProcessId=$desPid" | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    Stop-Process -Id $desPid -Force -ErrorAction SilentlyContinue
    $true
}
$doneSince = @{}
$stoppedE3d = @()
try {
    while (-not $driver.HasExited) {
        foreach ($dir in Get-Wrappers) {
            if ($stoppedE3d -contains $dir.Name -or (Read-Marker $dir 'driver-done.log') -notmatch 'L3-DONE') { continue }
            if (-not $doneSince.ContainsKey($dir.Name)) { $doneSince[$dir.Name] = Get-Date }
            if (((Get-Date) - $doneSince[$dir.Name]).TotalSeconds -ge $QuitGraceSeconds -and (Stop-E3dSession $dir)) { $stoppedE3d += $dir.Name }
        }
        Start-Sleep -Milliseconds 500
    }
} finally {
    # Leaving early (an error, Ctrl+C) must not leave E3D holding the copy.
    if (-not $driver.HasExited) {
        foreach ($dir in Get-Wrappers) { [void](Stop-E3dSession $dir) }
        Stop-Process -Id $driver.Id -Force -ErrorAction SilentlyContinue
    }
}
$driver.WaitForExit()
$driverText = (@($driverStdout, $driverStderr) | ForEach-Object { Get-Content $_ -Raw -ErrorAction SilentlyContinue }) -join "`n"
$driver = [pscustomobject]@{ Output = $driverText; ExitCode = $driver.ExitCode }
[IO.File]::WriteAllText((Join-Path $runDir 'driver.out.txt'), $driverText, $utf8)
$log = Read-LogText $bodyLog
$errors = @([regex]::Matches($log, '(?m)^.*(ERROR|\(\d+,\s*\d+\)).*$') | ForEach-Object { $_.Value.Trim() })
$saved = [regex]::Match($log, 'Last saved session number (\d+)')
$savedSesno = if ($saved.Success) { [int]$saved.Groups[1].Value } else { $null }
$newestAfter = Get-Newest

# 5. After, and the verdicts.
$afterSesno = if ($savedSesno) { $savedSesno } else { $newestAfter }
$rows = @()
$unmet = @()
foreach ($refno in $watch) {
    $was = @($before[$refno])
    $now = @(Get-Entries $refno $afterSesno)
    $wasText = ($was | ForEach-Object { '[' + ($_ -join ',') + ']' }) -join ' '
    $nowText = ($now | ForEach-Object { '[' + ($_ -join ',') + ']' }) -join ' '
    $verdict = if ($was.Count -eq 0 -and $now.Count -eq 0) { 'none' }
        elseif ($was.Count -eq 0) { 'appeared' }
        elseif ($now.Count -eq 0) { 'vanished' }
        elseif ($wasText -eq $nowText) { 'same' }
        else { 'rewritten' }
    if ($expect.Contains($refno) -and $expect[$refno] -ne $verdict) { $unmet += "$refno is $verdict, expected $($expect[$refno])" }
    $rows += [ordered]@{ refno = $refno; verdict = $verdict; before = $was; after = $now }
}
foreach ($refno in $expect.Keys) { if ($watch -notcontains $refno) { $unmet += "$refno is expected but not watched" } }
# Table-wide: every refno whose entries the session changed, watched or not.
$tableAfter = Get-Table $afterSesno
$table = [ordered]@{
    entries_before = @($tableBefore.Keys).Count
    entries_after = @($tableAfter.Keys).Count
    rewritten = @($tableBefore.Keys | Where-Object { $tableAfter.ContainsKey($_) -and $tableAfter[$_] -ne $tableBefore[$_] } | Sort-Object)
    appeared = @($tableAfter.Keys | Where-Object { -not $tableBefore.ContainsKey($_) } | Sort-Object)
    vanished = @($tableBefore.Keys | Where-Object { -not $tableAfter.ContainsKey($_) } | Sort-Object)
}

$probeShown = if ($ProbeFile.StartsWith($crate + '\', [StringComparison]::OrdinalIgnoreCase)) { $ProbeFile.Substring($crate.Length + 1) } else { $ProbeFile }
$result = [ordered]@{
    probe = (& $forward $probeShown)
    source = [ordered]@{ path = $Source; sha256 = $sourceHash }
    setup = $setupInfo
    before_sesno = $beforeSesno
    saved_sesno = $savedSesno
    newest_after = $newestAfter
    driver_exit = $driver.ExitCode
    e3d_stopped_after_done = ($stoppedE3d.Count -gt 0)
    e3d_errors = $errors
    watch = $rows
    table = $table
    unmet = $unmet
}
[IO.File]::WriteAllText((Join-Path $runDir 'result.json'), ($result | ConvertTo-Json -Depth 8), $utf8)

Write-Output ("probe {0}: source {1} ({2}), before session {3}, SAVEWORK session {4}, newest after {5}" -f $name, $Source, $sourceHash.Substring(0, 16), $beforeSesno, $(if ($savedSesno) { $savedSesno } else { 'none' }), $newestAfter)
if ($setupInfo) { Write-Output ("  setup {0}: e3d-io session {1}, refnos {2}" -f $setupInfo.ops, $setupInfo.session, ($setupInfo.refnos -join ' ')) }
foreach ($row in $rows) {
    Write-Output ("  {0,-14} {1}" -f $row.refno, $row.verdict)
    Write-Output ("    before {0}" -f (($row.before | ForEach-Object { '[' + ($_ -join ',') + ']' }) -join ' '))
    Write-Output ("    after  {0}" -f (($row.after | ForEach-Object { '[' + ($_ -join ',') + ']' }) -join ' '))
}
Write-Output ("  table: {0} -> {1} refnos with entries; rewritten {2}, appeared {3}, vanished {4}" -f $table.entries_before, $table.entries_after, $table.rewritten.Count, $table.appeared.Count, $table.vanished.Count)
foreach ($kind in 'rewritten', 'appeared', 'vanished') {
    $list = $table[$kind]
    if ($list.Count) { Write-Output ("    {0}: {1}{2}" -f $kind, (($list | Select-Object -First 20) -join ' '), $(if ($list.Count -gt 20) { " ... (+$($list.Count - 20), see result.json)" } else { '' })) }
}
foreach ($e in $errors) { Write-Output "  E3D said: $e" }
if ($stoppedE3d.Count) { Write-Output "  E3D had not left the TTY $QuitGraceSeconds s after L3-DONE; stopped it" }
if (-not $savedSesno) { Write-Output "  no SAVEWORK session (driver exit $($driver.ExitCode)); see $runDir" }
Write-Output "  log $bodyLog"
if ($unmet.Count) {
    foreach ($u in $unmet) { Write-Output "FAIL $u" }
    exit 1
}
if ($expect.Count) { Write-Output "PASS $name" }
exit 0
