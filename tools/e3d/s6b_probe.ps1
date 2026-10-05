<#
.SYNOPSIS
  Task 13.1 of .kiro/specs/write-path-phase-3 (requirement 13, clauses 2, 11 and 12): when
  the file's newest session is e3d-io's and page 1's Bucket_Counter sits below a word1
  e3d-io has issued, does an E3D 3.1 Design TTY session's NEW hand out that refno?

.DESCRIPTION
  pwsh -NoProfile -File tools/e3d/s6b_probe.ps1 [-Case Live|Deleted|Gap|All] [-Assets <dir>]

  Each case starts from a scratch project copy restored to the installed project (robocopy
  /MIR: only files an earlier session touched are copied again) and a fresh copy of -Source
  (the installed ams8000_0001, only ever read):
    Live     e3d-io inserts three ZONEs under -IoOwner in one session; they stay.
    Deleted  e3d-io inserts three ZONEs in one session and deletes them in the next.
    Gap      as Deleted, but the next session deletes only the two lowest: the highest stays,
             so "skip live numbers one by one" and "start above the live maximum" differ.
  Both e3d-write runs pass --spatial off --user e3dio; the refnos come only from their success
  reports, and the file's newest session must be the batch just written. A case whose
  Bucket_Counter is not below e3d-io's highest issued word1 is indeterminate.
  Then one E3D session (gen-model's l3_suite --check-driver, des.exe TTY, login SYSTEM) runs,
  in one TTY: copy after login; select -E3dOwner; copy right before NEW; NEW EQUI; Q REF;
  copy after NEW; SAVEWORK; Q SESSION; copy after SAVEWORK. Every copy is taken twice (SYSCOM
  inside the TTY) and only counts when both copies agree. The source, the file after e3d-io
  and the file after E3D has gone are read directly, twice. Each snapshot records page 0 and
  page 1 read at the header's page size, the bucket table, the session chain and the extent's
  live maximum.
  The log is scored only inside this run's [S6B-<id>-...] markers (s6b_checks.ps1). A valid
  case reports its observation (fresh / historical_reuse / live_duplicate); an invalid one is
  indeterminate with its reasons. Reusing a number is an observation, not a probe failure.
  Writes <RunRoot>\<case>\: result.json, pages.json (page 0 and 1 of every snapshot, hex),
  logs, reports and manifest.json (path, length, SHA-256 of every file, the database copies
  included). With -Assets, everything but the database copies is copied to <Assets>\<case>.
  Exit: 0 valid; 2 indeterminate; 1 the probe failed or the source changed.
#>
[CmdletBinding()]
param(
    [ValidateSet('Live', 'Deleted', 'Gap', 'All')][string]$Case = 'All',
    [string]$Source = '',
    [string]$Project = 'D:\AVEVA\Projects\E3D3.1\AvevaMarineSample',
    [string]$Scratch = 'D:\e3dio-s6b-probe',
    [string]$RunRoot = '',
    [string]$Assets = '',
    [string]$GenModel = 'D:\work\plant-code\old\gen-model',
    [string]$L3Exe = 'D:\Rust\target\debug\l3_suite.exe',
    [string]$DbRelative = 'ams000\ams8000_0001',
    [string]$IoOwner = '/1RX03-PIPEBJ',
    [string]$E3dOwner = '/1RX03-MSUP',
    [int]$Extent = 24384,
    [int]$TtyTimeoutSeconds = 600,
    [int]$QuitGraceSeconds = 20
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 's6b_checks.ps1')
$scratchRoot = [IO.Path]::GetFullPath($Scratch).TrimEnd('\')
if (-not $RunRoot) { $RunRoot = Join-Path $scratchRoot ('runs\' + (Get-Date -Format 'yyyyMMdd-HHmmss')) }
$RunRoot = [IO.Path]::GetFullPath($RunRoot)
if (-not (Test-S6bUnderRoot $RunRoot $scratchRoot)) { throw "RunRoot $RunRoot is not inside the scratch $scratchRoot" }

if ($Case -eq 'All') {
    # One case per process, one after the other, through this same PowerShell.
    $self = (Get-Process -Id $PID).Path
    $codes = [ordered]@{}
    foreach ($one in 'Live', 'Deleted', 'Gap') {
        $forwarded = @('-RunRoot', $RunRoot)
        foreach ($key in $PSBoundParameters.Keys) { if ($key -notin @('Case', 'RunRoot')) { $forwarded += @("-$key", [string]$PSBoundParameters[$key]) } }
        & $self -NoProfile -ExecutionPolicy Bypass -File $PSCommandPath -Case $one @forwarded
        $codes[$one] = $LASTEXITCODE
    }
    $verdicts = @()
    foreach ($one in $codes.Keys) {
        $path = Join-Path $RunRoot "$($one.ToLower())\result.json"
        if (Test-Path $path) { $verdicts += (Get-Content $path -Raw | ConvertFrom-Json).verdict }
    }
    $summary = [ordered]@{
        run_root = $RunRoot
        exit_codes = $codes
        observations = [ordered]@{}
        branch_without_persistence = (Get-S6bBranch $verdicts $null)
    }
    foreach ($one in $codes.Keys) {
        $path = Join-Path $RunRoot "$($one.ToLower())\result.json"
        $summary.observations[$one] = if (Test-Path $path) { $v = (Get-Content $path -Raw | ConvertFrom-Json).verdict; "$($v.verdict) $($v.observation)" } else { 'no result' }
    }
    [IO.File]::WriteAllText((Join-Path $RunRoot 'summary.json'), ($summary | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding($false)))
    Write-Output ("s6b All: {0}" -f (($summary.observations.GetEnumerator() | ForEach-Object { "$($_.Key) $($_.Value)" }) -join '; '))
    Write-Output "  page-1 persistence is settled by the evidence document, not by this probe; summary $RunRoot\summary.json"
    if (@($codes.Values | Where-Object { $_ -eq 1 }).Count) { exit 1 }
    if (@($codes.Values | Where-Object { $_ -ne 0 }).Count) { exit 2 }
    exit 0
}

if (-not (Test-Path $L3Exe)) {
    $fallback = @('D:\Rust\target-l3\debug\l3_suite.exe', 'D:\Rust\target\release\l3_suite.exe') | Where-Object { Test-Path $_ } | Select-Object -First 1
    if ($fallback) { $L3Exe = $fallback }
}
$crate = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
if (-not $Source) { $Source = Join-Path $Project $DbRelative }
foreach ($required in @($Source, $Project, $GenModel, $L3Exe)) {
    if (-not (Test-Path $required)) { throw "missing: $required" }
}
$projectFull = [IO.Path]::GetFullPath($Project).TrimEnd('\')
$sourceFull = [IO.Path]::GetFullPath($Source)
foreach ($pair in @(@($scratchRoot, $projectFull), @($projectFull, $scratchRoot))) {
    if ($pair[0] -ieq $pair[1] -or (Test-S6bUnderRoot $pair[0] $pair[1])) { throw "the scratch $scratchRoot and the project $projectFull overlap" }
}
if (Test-S6bUnderRoot $sourceFull $scratchRoot) { throw "the source $sourceFull lies inside the scratch" }
$copy = Join-Path $scratchRoot (Split-Path $projectFull -Leaf)
$copyDb = Join-Path $copy $DbRelative
$runDir = Join-Path $RunRoot $Case.ToLower()
foreach ($target in @($copy, $copyDb, $runDir)) { if (-not (Test-S6bUnderRoot $target $scratchRoot)) { throw "$target is not inside the scratch $scratchRoot" } }
$runId = [guid]::NewGuid().ToString('N').Substring(0, 8)
$tag = "S6B-$runId"
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
    if (-not (Test-Path -LiteralPath $path)) { return '' }
    $bytes = [IO.File]::ReadAllBytes($path)
    try { return (New-Object Text.UTF8Encoding($false, $true)).GetString($bytes) }
    catch { return [Text.Encoding]::GetEncoding([Globalization.CultureInfo]::CurrentCulture.TextInfo.ANSICodePage).GetString($bytes) }
}
function Get-Sha([string]$path) { (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash }

$sourceHash = Get-Sha $Source
$sourceLength = (Get-Item -LiteralPath $Source).Length
$failure = $null
$result = [ordered]@{ case = $Case; run_id = $runId; run_dir = $runDir; source = [ordered]@{ path = $Source; sha256 = $sourceHash; length = $sourceLength } }
$stoppedE3d = @()
$cleanup = New-Object System.Collections.Generic.List[string]
try {
    # 1. The scratch copy, restored to the installed project, and the database reset.
    New-Item -ItemType Directory -Force $copy | Out-Null
    robocopy $projectFull $copy /MIR /XD amspic amsdrg /R:1 /W:1 /NFL /NDL /NJH /NJS /NP | Out-Null
    if ($LASTEXITCODE -ge 8) { throw "robocopy failed with $LASTEXITCODE" }
    Copy-Item -LiteralPath $Source -Destination $copyDb -Force
    if ((Get-Sha $copyDb) -ne $sourceHash) { throw "the scratch database $copyDb does not match the source" }
    if (Test-Path $runDir) { throw "run directory $runDir already exists; every run keeps its own" }
    New-Item -ItemType Directory -Force $runDir | Out-Null

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
    # The first two pages, read whole at the header's page size; a short read throws.
    function Read-Head([string]$file, [int]$pageSize) {
        $buffer = New-Object byte[] (2 * $pageSize)
        $stream = [IO.File]::Open($file, 'Open', 'Read', 'ReadWrite')
        try {
            $got = 0
            while ($got -lt $buffer.Length) { $n = $stream.Read($buffer, $got, $buffer.Length - $got); if ($n -le 0) { break }; $got += $n }
        } finally { $stream.Close() }
        if ($got -ne $buffer.Length) { throw "short read: $got of $($buffer.Length) bytes from $file" }
        $p0 = New-Object byte[] $pageSize; $p1 = New-Object byte[] $pageSize
        [Array]::Copy($buffer, 0, $p0, 0, $pageSize); [Array]::Copy($buffer, $pageSize, $p1, 0, $pageSize)
        , @($p0, $p1)
    }
    $script:pages = [ordered]@{}
    # One snapshot. $second is the in-TTY twin copy; without one the file is read twice.
    function Get-Snapshot([string]$file, [string]$label, [string]$second = '') {
        $snap = [ordered]@{ label = $label; file = $file }
        $problems = New-Object System.Collections.Generic.List[string]
        if (-not (Test-Path -LiteralPath $file)) {
            $snap.missing = $file; $snap.window_valid = $false; $snap.window_problems = @("missing $file"); return $snap
        }
        $snap.file_length = (Get-Item -LiteralPath $file).Length
        $snap.file_sha256 = Get-Sha $file
        if ($second) {
            if (-not (Test-Path -LiteralPath $second)) { $problems.Add('second copy missing') }
            else {
                $snap.second = [ordered]@{ file = $second; length = (Get-Item -LiteralPath $second).Length; sha256 = (Get-Sha $second) }
                if ($snap.second.length -ne $snap.file_length -or $snap.second.sha256 -ne $snap.file_sha256) { $problems.Add('the two copies differ') }
            }
        } else {
            Start-Sleep -Milliseconds 500
            $again = [ordered]@{ length = (Get-Item -LiteralPath $file).Length; sha256 = (Get-Sha $file) }
            $snap.second = $again
            if ($again.length -ne $snap.file_length -or $again.sha256 -ne $snap.file_sha256) { $problems.Add('the file changed between two reads') }
        }
        try {
            $header = Invoke-Descriptor @('header', '--db', $file)
            $pageSize = [int]$header.page_size
            $snap.page_size = $pageSize
            $head = Read-Head $file $pageSize
            $w0 = Get-S6bPageWords $head[0] $pageSize
            $w1 = Get-S6bPageWords $head[1] $pageSize
            $table = ConvertTo-S6bBucketTable $w1
            $sha = [Security.Cryptography.SHA256]::Create()
            $snap.page0_sha256 = -join ($sha.ComputeHash($head[0]) | ForEach-Object { $_.ToString('X2') })
            $snap.page1_sha256 = -join ($sha.ComputeHash($head[1]) | ForEach-Object { $_.ToString('X2') })
            $snap.page0_word6 = [int64]$w0[6]; $snap.page0_word12 = [int64]$w0[12]; $snap.page0_word96 = [int64]$w0[96]
            $snap.page1_head = @([int64]$w1[0], [int64]$w1[1], [int64]$w1[2])
            $snap.page1_valid = $table.valid
            $snap.page1_problems = @($table.problems)
            $snap.page1_entries = @($table.entries)
            $snap.table = $table
            $snap.words0 = $w0; $snap.words1 = $w1
            $script:pages[$label] = [ordered]@{ page_size = $pageSize; page0_hex = [BitConverter]::ToString($head[0]).Replace('-', ''); page1_hex = [BitConverter]::ToString($head[1]).Replace('-', '') }
            foreach ($p in $table.problems) { $problems.Add("page 1: $p") }
            $sessions = Invoke-Descriptor @('sessions', '--db', $file)
            $snap.newest = [int]$sessions.newest
            $snap.top_sessions = @($sessions.sessions | Select-Object -First 4 | ForEach-Object { [ordered]@{ sesno = $_.sesno; user = $_.user; comment = $_.comment; reason = $_.tail.reason; base_sesno = $_.tail.base_sesno; page = $_.page } })
            $next = Invoke-Descriptor @('next-word1', '--db', $file, '--extent', "$Extent")
            $snap.bucket_counter = $next.bucket_counter
            $snap.scan_max_word1 = $next.scan_max_word1
            $snap.index_keys = $next.index_keys
        } catch { $problems.Add($_.Exception.Message) }
        $snap.window_valid = ($problems.Count -eq 0)
        $snap.window_problems = @($problems)
        $snap
    }

    # 2. e3d-io's sessions. Refnos come only from a success report; the newest session must be
    # the one the report names, written as e3dio with the batch's comment.
    $snapshots = [ordered]@{}
    $snapshots['source'] = Get-Snapshot $Source 'source'
    function Invoke-Batch([string]$name, $ops) {
        $opsPath = Join-Path $runDir "io-$name.json"
        [IO.File]::WriteAllText($opsPath, ($ops | ConvertTo-Json -Depth 5), $utf8)
        $report = Join-Path $runDir "io-$name.report.json"
        $run = Invoke-Native { & $writer apply --db $copyDb --ops $opsPath --report $report --user e3dio --spatial off }
        [IO.File]::WriteAllText((Join-Path $runDir "io-$name.out.txt"), $run.Output, $utf8)
        if ($run.ExitCode) { Write-Output $run.Output; throw "e3d-write apply ($name) failed ($($run.ExitCode))" }
        $parsed = [IO.File]::ReadAllText($report, $utf8) | ConvertFrom-Json
        $sesno = [int]$parsed.commit.written.sesno
        $chain = Invoke-Descriptor @('sessions', '--db', $copyDb)
        $top = $chain.sessions | Select-Object -First 1
        $own = ([int]$chain.newest -eq $sesno -and $top.user -eq 'e3dio' -and $top.comment -eq $ops.comment)
        [pscustomobject]@{ report = $parsed; sesno = $sesno; newest_is_batch = $own }
    }
    $insert = Invoke-Batch 'insert' (New-S6bInsertOps $Case $IoOwner)
    $issued = @($insert.report.ops | ForEach-Object { [string]$_.refno } | Where-Object { $_ -match '^\d+/\d+$' })
    if ($issued.Count -ne 3) { throw "the insert report names $($issued.Count) refnos, not 3" }
    $ioSessions = @($insert.sesno)
    $batches = @($insert)
    $doomed = @(Get-S6bDoomed $Case $issued)
    if ($doomed.Count) {
        $delete = Invoke-Batch 'delete' (New-S6bDeleteOps $Case $doomed)
        $ioSessions += $delete.sesno
        $batches += $delete
    }
    $live = @($issued | Where-Object { $doomed -notcontains $_ })
    $snapshots['before_e3d'] = Get-Snapshot $copyDb 'before_e3d'
    $newestIsE3dio = ($batches[-1].newest_is_batch -and $snapshots['before_e3d'].newest -eq $ioSessions[-1])
    $counterBefore = ($snapshots['before_e3d'].page1_entries | Where-Object { $_[0] -eq $Extent } | Select-Object -First 1)
    $counterBefore = if ($counterBefore) { [int64]$counterBefore[1] } else { $null }

    # 3. One TTY, twin copies at every observation boundary.
    $copies = [ordered]@{}
    foreach ($label in @('after_login', 'before_new', 'after_new', 'after_save')) {
        $copies[$label] = @((Join-Path $runDir "$label.a.db"), (Join-Path $runDir "$label.b.db"))
    }
    $bodyLog = Join-Path $runDir 'body.log'
    $macroPath = Join-Path $runDir 's6b.mac'
    $macroLog = [IO.Path]::ChangeExtension($macroPath, '.log')
    $macro = New-Object Text.StringBuilder
    function Add-Copies([string]$label) {
        foreach ($destination in $copies[$label]) {
            [void]$macro.AppendLine("SYSCOM 'cmd.exe /c copy /y `"$copyDb`" `"$destination`"'")
        }
    }
    [void]$macro.AppendLine("ALPHA LOG `"$(& $forward $bodyLog)`" OVER")
    [void]$macro.AppendLine("`$P [$tag-BEGIN]")
    [void]$macro.AppendLine('Q USER')
    Add-Copies 'after_login'
    [void]$macro.AppendLine($E3dOwner)
    [void]$macro.AppendLine('handle any')
    [void]$macro.AppendLine("  `$P [$tag-OWNER-ERROR] `$!!error.text")
    [void]$macro.AppendLine('endhandle')
    Add-Copies 'before_new'
    [void]$macro.AppendLine("NEW EQUI /E3DIO-S6B-E3D-$runId")
    [void]$macro.AppendLine('handle any')
    [void]$macro.AppendLine("  `$P [$tag-NEW-ERROR] `$!!error.text")
    [void]$macro.AppendLine('elsehandle none')
    [void]$macro.AppendLine("  `$P [$tag-NEW-BEGIN]")
    [void]$macro.AppendLine('  Q REF')
    [void]$macro.AppendLine("  `$P [$tag-NEW-END]")
    [void]$macro.AppendLine('endhandle')
    Add-Copies 'after_new'
    [void]$macro.AppendLine("SAVEWORK 'e3d-io S6b probe $Case'")
    [void]$macro.AppendLine('handle any')
    [void]$macro.AppendLine("  `$P [$tag-SAVE-ERROR] `$!!error.text")
    [void]$macro.AppendLine('elsehandle none')
    [void]$macro.AppendLine("  `$P [$tag-SAVE-BEGIN]")
    [void]$macro.AppendLine('  Q SESSION')
    [void]$macro.AppendLine("  `$P [$tag-SAVE-END]")
    [void]$macro.AppendLine('endhandle')
    Add-Copies 'after_save'
    [void]$macro.AppendLine("`$P [$tag-END]")
    [void]$macro.AppendLine('ALPHA LOG END')
    [void]$macro.AppendLine("ALPHA LOG `"$(& $forward $macroLog)`" OVER")
    [void]$macro.AppendLine("`$P [$tag-DONE]")
    [void]$macro.AppendLine('ALPHA LOG END')
    [IO.File]::WriteAllText($macroPath, $macro.ToString(), $utf8)
    $driverOut = Join-Path $runDir 'driver-out'
    New-Item -ItemType Directory -Force $driverOut | Out-Null
    $driverStdout = Join-Path $runDir 'driver.stdout.txt'
    $driverStderr = Join-Path $runDir 'driver.stderr.txt'
    $driverArgs = @('--check-driver', $macroPath, '--project-dir', $copy, '--output', $driverOut, '--target-db-file', $copyDb, '--aios-project', 'e3dio-s6b-probe')
    $previous = $env:L3_ALLOW_EXISTING_E3D_SESSION
    try {
        $env:L3_ALLOW_EXISTING_E3D_SESSION = '1'
        $driver = Start-Process -FilePath $L3Exe -ArgumentList ($driverArgs | ForEach-Object { "`"$_`"" }) -WorkingDirectory $GenModel -WindowStyle Hidden -PassThru -RedirectStandardOutput $driverStdout -RedirectStandardError $driverStderr
    } finally {
        if ($null -eq $previous) { Remove-Item Env:\L3_ALLOW_EXISTING_E3D_SESSION -ErrorAction SilentlyContinue } else { $env:L3_ALLOW_EXISTING_E3D_SESSION = $previous }
    }
    $null = $driver.Handle
    $wrappers = Join-Path $GenModel 'output\l3-suite'
    function Get-Wrappers { @(Get-ChildItem $wrappers -Directory -Filter "driver-$($driver.Id)-*" -ErrorAction SilentlyContinue) }
    function Read-Marker($dir, [string]$file) { [string](Get-Content (Join-Path $dir.FullName $file) -Raw -ErrorAction SilentlyContinue) }
    function Stop-OwnedSession($dir) {
        $desPid = 0
        if (-not [int]::TryParse((Read-Marker $dir 'driver.pid').Trim(), [ref]$desPid)) { return $false }
        $process = Get-CimInstance Win32_Process -Filter "ProcessId=$desPid" -ErrorAction SilentlyContinue
        if (-not $process -or $process.Name -ine 'des.exe' -or $process.CreationDate -lt $started) { return $false }
        # The wrapper is uniquely associated with this driver PID and fresh macro.
        Get-CimInstance Win32_Process -Filter "ParentProcessId=$desPid" | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
        Stop-Process -Id $desPid -Force -ErrorAction SilentlyContinue
        $true
    }
    $started = $driver.StartTime
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $doneSince = @{}
    $timedOut = $false
    try {
        while (-not $driver.HasExited) {
            if ($timer.Elapsed.TotalSeconds -ge $TtyTimeoutSeconds) { $timedOut = $true; break }
            foreach ($dir in Get-Wrappers) {
                if ($stoppedE3d -contains $dir.Name -or (Read-Marker $dir 'driver-done.log') -notmatch 'L3-DONE') { continue }
                if (-not $doneSince.ContainsKey($dir.Name)) { $doneSince[$dir.Name] = Get-Date }
                if (((Get-Date) - $doneSince[$dir.Name]).TotalSeconds -ge $QuitGraceSeconds -and (Stop-OwnedSession $dir)) { $stoppedE3d += $dir.Name }
            }
            Start-Sleep -Milliseconds 500
            $driver.Refresh()
        }
    } finally {
        if (-not $driver.HasExited) {
            foreach ($dir in Get-Wrappers) {
                if (-not (Stop-OwnedSession $dir)) { $cleanup.Add("could not confirm owned E3D process: $($dir.Name)") }
            }
            Stop-Process -Id $driver.Id -Force -ErrorAction SilentlyContinue
        }
    }
    $driver.WaitForExit()
    $log = Read-S6bLog (Read-LogText $bodyLog) $runId
    foreach ($label in $copies.Keys) { $snapshots[$label] = Get-Snapshot $copies[$label][0] $label $copies[$label][1] }
    $snapshots['after_e3d'] = Get-Snapshot $copyDb 'after_e3d'
    $tables = [ordered]@{}; $newest = [ordered]@{}; $windows = [ordered]@{}
    foreach ($label in $snapshots.Keys) {
        $tables[$label] = if ($snapshots[$label].table) { $snapshots[$label].table } else { [pscustomobject]@{ valid = $false; problems = @('missing table'); count = 0; entries = @() } }
        $newest[$label] = $snapshots[$label].newest
        $windows[$label] = $snapshots[$label].window_valid
    }
    $dirs = @(Get-Wrappers)
    $driverOk = ($driver.ExitCode -eq 0 -and -not $timedOut -and $dirs.Count -eq 1 -and (Read-Marker $dirs[0] 'driver-done.log') -match 'L3-DONE' -and $cleanup.Count -eq 0)
    $verdict = Get-S6bVerdict ([pscustomobject]@{ issued = $issued; live = $live; deleted = $doomed; extent = $Extent; newest_is_e3dio = $newestIsE3dio; counter_before = $counterBefore; tables = $tables; newest = $newest; windows = $windows; log = $log; driver_ok = $driverOk })
    if ($log.saved_sesno -ne $snapshots['after_save'].newest) { $verdict.valid = $false; $verdict.verdict = 'indeterminate'; $verdict.reasons += 'Q SESSION does not match the after_save snapshot' }
    $result.e3dio_sessions = $ioSessions; $result.e3dio_issued = $issued; $result.e3dio_live = $live; $result.e3dio_deleted = $doomed
    $result.e3d_refno = $log.refno; $result.e3d_saved_sesno = $log.saved_sesno
    $result.driver_exit = $driver.ExitCode; $result.timed_out = $timedOut; $result.elapsed_seconds = $timer.Elapsed.TotalSeconds
    $result.verdict = $verdict; $result.e3d_errors = @($log.problems); $result.cleanup_problems = @($cleanup)
    $result.e3d_stopped_after_done = $stoppedE3d.Count -gt 0
    $result.snapshots = @()
    $result.page_differences = [ordered]@{}
    $previousLabel = $null
    foreach ($label in $snapshots.Keys) {
        $s = $snapshots[$label]
        if ($previousLabel -and $s.words1 -and $snapshots[$previousLabel].words1) {
            $result.page_differences["$previousLabel->$label"] = Compare-S6bWords $snapshots[$previousLabel].words1 $s.words1
        }
        $s.Remove('words0'); $s.Remove('table')
        # Keep word1 until differences have been calculated; strip below.
        $result.snapshots += $s
        $previousLabel = $label
    }
    foreach ($s in $result.snapshots) { $s.Remove('words1') }
    [IO.File]::WriteAllText((Join-Path $runDir 'pages.json'), ($script:pages | ConvertTo-Json -Depth 5), $utf8)
} catch {
    $failure = $_.Exception.Message
    $result.failure = $failure
    Write-Output "FAIL S6B $Case : $failure"
} finally {
    $result.source_after = [ordered]@{ sha256 = (Get-Sha $Source); length = (Get-Item -LiteralPath $Source).Length }
    $result.source_unchanged = ($sourceHash -eq $result.source_after.sha256 -and $sourceLength -eq $result.source_after.length)
    if (-not $result.source_unchanged) { $failure = 'SOURCE DATABASE CHANGED'; $result.failure = $failure }
    if (Test-Path -LiteralPath $runDir) {
        [IO.File]::WriteAllText((Join-Path $runDir 'result.json'), ($result | ConvertTo-Json -Depth 12), $utf8)
        $manifest = @(Get-ChildItem $runDir -File -Recurse | ForEach-Object { [ordered]@{ path = [IO.Path]::GetRelativePath($runDir, $_.FullName); length = $_.Length; sha256 = (Get-Sha $_.FullName) } })
        [IO.File]::WriteAllText((Join-Path $runDir 'manifest.json'), ($manifest | ConvertTo-Json -Depth 4), $utf8)
        if ($Assets) {
            $assetDir = Join-Path ([IO.Path]::GetFullPath($Assets)) $Case.ToLower()
            if (Test-Path -LiteralPath $assetDir) { throw "asset directory exists; will not overwrite $assetDir" }
            foreach ($f in Get-ChildItem $runDir -File -Recurse | Where-Object Extension -NE '.db') {
                $destination = Join-Path $assetDir ([IO.Path]::GetRelativePath($runDir, $f.FullName))
                New-Item -ItemType Directory -Force (Split-Path $destination -Parent) | Out-Null
                Copy-Item -LiteralPath $f.FullName -Destination $destination
            }
        }
    }
}
if ($failure) { exit 1 }
Write-Output "S6B $Case : $($result.verdict.verdict) $($result.verdict.observation); source unchanged=$($result.source_unchanged); session=$($result.e3d_saved_sesno); result=$runDir\result.json"
foreach ($reason in $result.verdict.reasons) { Write-Output "  $reason" }
if (-not $result.verdict.valid) { exit 2 }
exit 0
