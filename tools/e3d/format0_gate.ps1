<#
.SYNOPSIS
  The E3D gate for the write path's T4 FORMAT 0 check
  (docs/plans/2026-09-22-write-path-phase-2-plan.md §2 T4): e3d-io writes a Chinese NAME,
  DESC and session comment into a FORMAT 0 database of the TES000 project, and a real
  E3D 3.1 reads every one of them back.

.DESCRIPTION
  FORMAT 0 (header word +0x154 != 1) stores text in the project's code page, every byte above
  127 inside an escaped run (`&~ … &`, high bit cleared); e3d-io's writer escapes by the
  header's word (src/record/text.rs). The database is TES000's *EQUIB/DESI_COMP — dbno 1035,
  FINO 23, file TES023 — which holds SITE /锅炉岛设备 and its ZONE /10HDE, and is a member of
  MDB /COMP. (MDB /工艺 serves dbno 1035 from *EQUIB/DESI — FINO 0, file TES1035_0001 — whose
  WORLD has had no members since its session 1627, like every equipment database of /工艺.
  A DB element with FINO n > 0 lives in TESnnn, one with FINO 0 in TES<dbno>_<extno, 4 digits>.)

  1. The project copy: the project is copied under -Scratch once (or with -RefreshCopy); the
     database and the system / comms databases are reset from the original each run. The
     launcher binds projects_dir to -Scratch, so E3D opens the copy only because the
     project's evars roots every folder at %projects_dir%: the gate refuses to run if it
     does not, and fails if any file in the original's database folder changed.
  2. `e3d-write apply format0_gate_ops/10_text.json`, one session: ZONE /10HDE renamed
     /反应堆厂房-E3DIO, its DESC set to 反应堆厂房 -3.320m~±0.00m (an escaped run inside
     plain text), an EQUI /E3DIO-锅炉设备 with a Chinese DESC inserted under it, the session
     commented in Chinese.
  3. e3d-io reads the four strings back (e3d-descriptor extract), and the appended pages
     hold the ZONE's name and DESC escaped — the bytes tests/write_stage5_text.rs pins —
     and none of the text as UTF-8.
  4. A real E3D 3.1 TTY session logs into the copy with MDB /COMP and queries the elements
     by refno (format0_gate.mac.template). A macro cannot carry Chinese — E3D reads it in a
     single-byte code page — so every Chinese string the verdicts match came out of E3D.
     The control: E3D also names the SITE, which E3D itself wrote.

  The session goes through gen-model's scripts/e3d/run_ams_c_entrymacro.bat — the launcher
  behind l3_suite --check-driver, parameterised by its L3_E3D_* variables — inside
  l3_suite's wrapper (L3-ALIVE, the macro, L3-DONE, QUIT). l3_suite itself is not used: it
  insists on the AMS project layout, which TES000 does not have.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File tools/e3d/format0_gate.ps1
#>
[CmdletBinding()]
param(
    [string]$Project = 'D:\AVEVA\Projects\E3D3.1\TEST',
    [string]$ProjectCode = 'TES',
    [string]$Login = 'SYSTEM/XXXXXX',
    [string]$Mdb = '/COMP',
    [string]$DbRelative = 'TES000\TES023',
    [string]$DbName = 'EQUIB/DESI_COMP',
    [string]$Scratch = 'D:\e3dio-tesgate',
    [string]$Launcher = 'D:\work\plant-code\old\gen-model\scripts\e3d\run_ams_c_entrymacro.bat',
    [int]$TimeoutSeconds = 300,
    [switch]$RefreshCopy
)

$ErrorActionPreference = 'Stop'
$crate = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$leaf = Split-Path $Project -Leaf
$copy = Join-Path $Scratch $leaf
$evars = "evars$leaf.bat"
$originalDb = Join-Path $Project $DbRelative
$copyDb = Join-Path $copy $DbRelative
$dbFolder = Split-Path $originalDb -Parent
$logDir = Join-Path $Scratch 'logs-format0'
$opsFile = Join-Path $PSScriptRoot 'format0_gate_ops\10_text.json'
$utf8 = New-Object Text.UTF8Encoding($false)

foreach ($required in @($originalDb, (Join-Path $Project $evars), $Launcher, $opsFile)) {
    if (-not (Test-Path $required)) { throw "missing: $required" }
}
# e3d-descriptor prints UTF-8; Windows PowerShell would decode it in the console's code page.
$consoleEncoding = [Console]::OutputEncoding
[Console]::OutputEncoding = $utf8
$failures = 0

# E3D must open the copy, not the original: every folder the project's evars sets has to hang
# off %projects_dir%, which the launcher binds to -Scratch.
$outside = @(foreach ($line in Get-Content (Join-Path $Project $evars)) {
    if ($line -match '^\s*set\s+"?\w+=([^"]*)') {
        $value = $Matches[1]
        if ($value -match '[\\/:]' -and $value -notmatch '^%projects_dir%') { $line.Trim() }
    }
})
if ($outside.Count) { throw "$evars sets a folder outside %projects_dir%, so E3D would open the original project: $($outside -join ' | ')" }
function Fingerprint([string]$folder) {
    (Get-ChildItem $folder -File | Sort-Object Name | ForEach-Object { "$($_.Name)|$($_.Length)|$($_.LastWriteTimeUtc.Ticks)" }) -join "`n"
}
$originalBefore = Fingerprint $dbFolder

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
$writeExe = Join-Path $targetDir 'debug\e3d-write.exe'
$descExe = Join-Path $targetDir 'debug\e3d-descriptor.exe'
$schema = Join-Path $crate 'schema\e3d31'
foreach ($required in @($writeExe, $descExe)) { if (-not (Test-Path $required)) { throw "built bin missing: $required" } }

# 1. The project copy (everything but the picture folder; reset the databases this run touches).
if ($RefreshCopy -or -not (Test-Path (Join-Path $copy $evars))) {
    New-Item -ItemType Directory -Force $copy | Out-Null
    robocopy $Project $copy /E /XD "${ProjectCode}PIC" /R:1 /W:1 /NFL /NDL /NJH /NJS /NP | Out-Null
    if ($LASTEXITCODE -ge 8) { throw "robocopy failed with $LASTEXITCODE" }
    Write-Output "project copied to $copy"
}
$dbFolderRelative = Split-Path $DbRelative -Parent
foreach ($relative in @($DbRelative, (Join-Path $dbFolderRelative "${ProjectCode}sys"), (Join-Path $dbFolderRelative "${ProjectCode}com"))) {
    Copy-Item (Join-Path $Project $relative) (Join-Path $copy $relative) -Force
}
Write-Output "reset $copyDb and the system / comms databases from the original"
New-Item -ItemType Directory -Force $logDir | Out-Null
Remove-Item (Join-Path $logDir '*.log') -Force -ErrorAction SilentlyContinue

# 2. e3d-io writes the Chinese session.
$report = Join-Path $Scratch 'format0_text.report.json'
Remove-Item $report -Force -ErrorAction SilentlyContinue
$originalLength = (Get-Item $copyDb).Length
$apply = Invoke-Native { & $writeExe apply --db $copyDb --ops $opsFile --schema $schema --report $report }
if ($apply.ExitCode) { Write-Output $apply.Output; throw 'e3d-write apply failed' }
$rep = [IO.File]::ReadAllText($report, $utf8) | ConvertFrom-Json
$zoneRef = ($rep.ops | Where-Object { $_.op -eq 'rename' }).refno
$equiRef = ($rep.ops | Where-Object { $_.op -eq 'insert' }).refno
$written = [int]$rep.commit.written.sesno
Write-Output ("e3d-write: session {0} on base {1}: ZONE {2} renamed and described, EQUI {3} inserted" -f $written, $rep.commit.base_sesno, $zoneRef, $equiRef)

# The expected text is the instruction file's own; the SITE's name is E3D's, spelled as code
# points so this file reads the same under any PowerShell encoding: /锅炉岛设备.
$ops = [IO.File]::ReadAllText($opsFile, $utf8) | ConvertFrom-Json
$zoneName = ($ops.ops | Where-Object { $_.op -eq 'rename' }).name
$zoneDesc = ($ops.ops | Where-Object { $_.op -eq 'set' }).attrs.DESC
$equiOp = $ops.ops | Where-Object { $_.op -eq 'insert' }
$equiName = $equiOp.name
$equiDesc = $equiOp.attrs.DESC
$comment = $ops.comment
$siteName = '/' + (-join (@(0x9505, 0x7089, 0x5C9B, 0x8BBE, 0x5907) | ForEach-Object { [char]$_ }))

# 3. e3d-io's side: the strings read back, and how they lie on disk.
function Attr([string]$refno, [string]$name) {
    $eap = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    try { $json = & $descExe extract --db $copyDb --schema $schema --refno $refno 2>$null | Out-String } finally { $ErrorActionPreference = $eap }
    (($json | ConvertFrom-Json).attributes | Where-Object { $_.name -eq $name }).value
}
$wrong = @(foreach ($want in @(
        [pscustomobject]@{ Ref = $zoneRef; Attr = 'NAME'; Text = $zoneName },
        [pscustomobject]@{ Ref = $zoneRef; Attr = 'DESC'; Text = $zoneDesc },
        [pscustomobject]@{ Ref = $equiRef; Attr = 'NAME'; Text = $equiName },
        [pscustomobject]@{ Ref = $equiRef; Attr = 'DESC'; Text = $equiDesc })) {
    $got = Attr $want.Ref $want.Attr
    if ($got -cne $want.Text) { "$($want.Ref) $($want.Attr) = '$got', wanted '$($want.Text)'" }
})
if ($wrong.Count -eq 0) {
    Write-Output 'PASS E1  e3d-io reads back both NAMEs and both DESCs'
} else {
    Write-Output 'FAIL E1  e3d-io reads back what it wrote'
    foreach ($w in $wrong) { Write-Output "     $w" }
    $failures++
}
# The appended pages carry the ZONE's name key and DESC escaped, byte for byte as
# tests/write_stage5_text.rs pins them (the key E3D itself writes for this name), and no
# Chinese run of any of the five strings as UTF-8.
$bytes = [IO.File]::ReadAllBytes($copyDb)
$latin1 = [Text.Encoding]::GetEncoding(28591)
$appended = $latin1.GetString($bytes, $originalLength, $bytes.Length - $originalLength)
$escaped = @('/&~74S&6Q3''7? &-E3DIO', '&~74S&6Q3''7? & -3.320m~&~!@ &0.00m')
$missing = @($escaped | Where-Object { $appended.IndexOf($_, [StringComparison]::Ordinal) -lt 0 })
$raw = @(foreach ($text in @($zoneName, $zoneDesc, $equiName, $equiDesc, $comment)) {
    foreach ($run in [regex]::Matches($text, '[^\x00-\x7F]+')) {
        if ($appended.IndexOf($latin1.GetString($utf8.GetBytes($run.Value)), [StringComparison]::Ordinal) -ge 0) { $run.Value }
    }
})
if ($missing.Count -eq 0 -and $raw.Count -eq 0) {
    Write-Output ("PASS E2  the {0} appended bytes hold the ZONE's name key and DESC escaped, and no Chinese as UTF-8" -f ($bytes.Length - $originalLength))
} else {
    Write-Output 'FAIL E2  the appended pages store the text as FORMAT 0 says'
    foreach ($m in $missing) { Write-Output "     escaped form not found: $m" }
    foreach ($r in $raw) { Write-Output "     stored as UTF-8: $r" }
    $failures++
}

# 4. E3D reads it back, in l3_suite's wrapper, through the parameterised launcher.
$forward = { param([string]$p) $p -replace '\\', '/' }
$macro = Join-Path $Scratch 'format0_gate.mac'
$text = Get-Content (Join-Path $PSScriptRoot 'format0_gate.mac.template') -Raw
foreach ($pair in @(@('LOGDIR', (& $forward $logDir)), @('ZONE', $zoneRef), @('EQUI', $equiRef))) { $text = $text.Replace("{{$($pair[0])}}", [string]$pair[1]) }
if ($text -match '\{\{[A-Z0-9]+\}\}') { throw "format0_gate.mac still has a placeholder: $($Matches[0])" }
Set-Content -Path $macro -Value $text -Encoding ASCII
$alive = Join-Path $logDir 'driver-alive.log'
$done = Join-Path $logDir 'driver-done.log'
$wrapper = Join-Path $Scratch 'format0_driver.mac'
Set-Content -Path $wrapper -Encoding ASCII -Value @(
    "ALPHA LOG `"$(& $forward $alive)`" OVER", '$P L3-ALIVE', 'ALPHA LOG END',
    "`$M `"$(& $forward $macro)`"",
    "ALPHA LOG `"$(& $forward $done)`" OVER", '$P L3-DONE', 'ALPHA LOG END',
    'QUIT')
$driverEnv = [ordered]@{
    L3_E3D_PROJECTS_DIR    = $Scratch
    L3_E3D_PROJECT_EVAR    = (Join-Path $copy $evars)
    L3_E3D_PROJECT         = $ProjectCode
    L3_E3D_LOGIN           = $Login
    L3_E3D_MDB             = $Mdb
    L3_E3D_TIMEOUT_SECONDS = [string]$TimeoutSeconds
    L3_E3D_PID_FILE        = (Join-Path $Scratch 'format0_driver.pid')
}
$saved = @{}
foreach ($key in $driverEnv.Keys) {
    $saved[$key] = [Environment]::GetEnvironmentVariable($key)
    [Environment]::SetEnvironmentVariable($key, $driverEnv[$key])
}
$clock = [Diagnostics.Stopwatch]::StartNew()
try {
    $run = Invoke-Native { cmd /d /c $Launcher (& $forward $wrapper) }
} finally {
    foreach ($key in $saved.Keys) { [Environment]::SetEnvironmentVariable($key, $saved[$key]) }
}
Set-Content -Path (Join-Path $logDir 'driver.txt') -Value $run.Output
$aliveSeen = (Test-Path $alive) -and ((Get-Content $alive -Raw) -match 'L3-ALIVE')
$doneSeen = (Test-Path $done) -and ((Get-Content $done -Raw) -match 'L3-DONE')
# The shadow install often dies in DLL_PROCESS_DETACH long after L3-DONE is on disk, so the
# markers are the verdict, not the exit code (as in l3_suite).
Write-Output ("E3D: TTY session {0} s, launcher exit {1}, reached the command loop: {2}, finished the macro: {3}" -f [int]$clock.Elapsed.TotalSeconds, $run.ExitCode, $aliveSeen, $doneSeen)
if (-not ($aliveSeen -and $doneSeen)) { Write-Output $run.Output; throw "the E3D session did not run the macro through; see $logDir" }

# 5. The verdicts.
# E3D writes ALPHA LOG in the process's ANSI code page (GBK on a Chinese Windows), not
# UTF-8. Read a log as UTF-8 when it is valid UTF-8, else as the ANSI code page, and say which.
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
# A `Q MEM` line: ` i TYPE n /NAME` or ` i TYPE n =word0/word1`.
$memberLine = '(?m)^\s*\d+\s+[A-Z][A-Z0-9]*\s+\d+\s*(=\d+/\d+|/\S+)\s*$'
function Check([string]$question, [string]$log, [string[]]$expected, [string]$lastMember = '') {
    $path = Join-Path $logDir $log
    if (-not (Test-Path $path)) { Write-Output "FAIL $question — $log was never written (the block did not reach ALPHA LOG END)"; $script:failures++; return }
    $content = Read-LogText $path
    $missing = @($expected | Where-Object { $content.IndexOf($_, [StringComparison]::Ordinal) -lt 0 })
    $errors = @([regex]::Matches($content, '(?m)^.*(ERROR|\(\d+,\d+\)).*$') | ForEach-Object { $_.Value })
    $problems = @()
    $lines = @([regex]::Matches($content, $memberLine) | ForEach-Object { $_.Value.Trim() })
    if ($lastMember -and ($lines.Count -eq 0 -or -not $lines[-1].EndsWith($lastMember, [StringComparison]::Ordinal))) {
        $problems += "the last member line is '$(if ($lines.Count) { $lines[-1] } else { '<none>' })', wanted one ending $lastMember"
    }
    if ($missing.Count -eq 0 -and $errors.Count -eq 0 -and $problems.Count -eq 0) {
        Write-Output "PASS $question"
    } else {
        Write-Output "FAIL $question"
        foreach ($m in $missing) { Write-Output "     missing: $m" }
        foreach ($p in $problems) { Write-Output "     $p" }
        foreach ($e in $errors) { Write-Output "     E3D said: $e" }
        $script:failures++
    }
}
Check "F0  E3D is in the project copy with MDB $Mdb" '00_project.log' @("MDB: $Mdb")
Check "F1  control: E3D names the SITE it wrote itself, and serves $DbName for the MDB" '10_control.log' @('Type ZONE', "Owner $siteName", "Dbname $DbName")
Check 'F2a ZONE renamed: a Chinese NAME, record and table' '20_zone_name.log' @("Name $zoneName")
Check 'F2b EQUI inserted: a Chinese NAME, under the renamed ZONE' '40_equi.log' @('Type EQUI', "Name $equiName", "Owner $zoneName")
Check 'F2c the ZONE lists the new EQUI last, by its Chinese name' '50_zone_members.log' @() -lastMember $equiName
Check 'F3a ZONE DESC: an escaped run inside plain text' '30_zone_desc.log' @("Description $zoneDesc")
Check 'F3b EQUI DESC: Chinese and ASCII runs' '40_equi.log' @("Description $equiDesc")
Check 'F4  Q SESSION: the e3d-io session, user e3dio, its Chinese comment' '60_session.log' @("Last saved session number $written", 'User: e3dio', "Description: $comment")
Write-Output ("     (logs decoded as: {0})" -f (($script:logEncodings.GetEnumerator() | Sort-Object Key | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join ', '))
if ((Fingerprint $dbFolder) -ceq $originalBefore) {
    Write-Output "PASS F5  the original $dbFolder is untouched: E3D and e3d-io wrote only the copy"
} else {
    Write-Output "FAIL F5  a file in the original $dbFolder changed during the gate"
    $failures++
}

Write-Output ''
[Console]::OutputEncoding = $consoleEncoding
if ($failures) { Write-Output "$failures FAIL — logs in $logDir"; exit 1 }
Write-Output "all PASS — E3D 3.1 read back every Chinese NAME, DESC and comment e3d-io wrote into the FORMAT 0 database $DbRelative (logs in $logDir)"
