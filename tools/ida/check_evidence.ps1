<#
.SYNOPSIS
  Machine checks on a Phase_3_Evidence_Doc (requirements 1.7-1.9 and
  Property 25 of .kiro/specs/write-path-phase-3; tools/ida/EVIDENCE_TEMPLATE.md).

.DESCRIPTION
  powershell -NoProfile -File tools/ida/check_evidence.ps1 <doc>

  - The four sections (environment, conclusions, reproduction commands,
    counterexamples) are there, under the template's Chinese headings.
  - Environment: a 40-hex HEAD; two or more 64-hex SHA-256 values on
    SHA-256 lines, the first equal to the last.
  - Reproduction commands, every line in their code blocks: starts with
    tools/ida/ib.ps1, a script in the repository by relative path, or cargo
    (after an optional "powershell ... -File"); names no temporary directory,
    no idalib-<n> / idaui-<n>, no ida-bridge installation path; every
    tools/ida/ file it names is tracked by git.
  - Conclusions: every 0x token is hexadecimal; an address after "@" is
    written in 0x hexadecimal.
  One line per failure; exit 0 when there is none, 1 otherwise.
#>
$ErrorActionPreference = 'Stop'

if ($args.Count -ne 1) {
    [Console]::Error.WriteLine('usage: tools/ida/check_evidence.ps1 <doc>')
    exit 2
}
$doc = [string]$args[0]
$repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)

# The headings, from code points: Windows PowerShell reads this file in the
# ANSI code page.
function Text([int[]]$Points) { -join ($Points | ForEach-Object { [char]$_ }) }
$Sections = [ordered]@{
    environment = Text 0x73AF, 0x5883
    conclusions = Text 0x7ED3, 0x8BBA
    commands = Text 0x590D, 0x73B0, 0x547D, 0x4EE4
    counterexamples = Text 0x53CD, 0x4F8B
}
# The installation places tools/ida/ib.ps1 looks in.
$InstallPlaces = @('C:\Users\dpc\.agents-old\tools\ida-bridge', 'C:\Users\dpc\.agents\tools\ida-bridge')
if ($env:IDA_BRIDGE_HOME) { $InstallPlaces += $env:IDA_BRIDGE_HOME }

$lines = @(Get-Content -LiteralPath $doc -Encoding UTF8)
$failures = New-Object System.Collections.Generic.List[string]

# The lines under the level-2 heading that contains $Name, up to the next one.
function Get-Section([string]$Name) {
    $inside = $false
    $found = $false
    $body = @()
    foreach ($line in $lines) {
        if ($line -match '^## ') {
            $inside = $line.Contains($Name)
            if ($inside) { $found = $true }
            continue
        }
        if ($inside) { $body += $line }
    }
    if (-not $found) { return $null }
    return , $body
}

$bodies = @{}
foreach ($key in $Sections.Keys) {
    $body = Get-Section $Sections[$key]
    if ($null -eq $body) { $failures.Add("missing section: $($Sections[$key]) ($key)") } else { $bodies[$key] = $body }
}

if ($bodies.ContainsKey('environment')) {
    $environment = $bodies['environment']
    if (-not ($environment | Where-Object { $_ -match 'HEAD' -and $_ -match '\b[0-9a-fA-F]{40}\b' })) {
        $failures.Add('environment: no HEAD line with a 40-hex commit')
    }
    $hashes = @($environment | Where-Object { $_ -match 'SHA-256' } | ForEach-Object {
            [regex]::Matches($_, '\b[0-9a-fA-F]{64}\b') | ForEach-Object { $_.Value.ToUpperInvariant() }
        })
    if ($hashes.Count -lt 2) {
        $failures.Add("environment: $($hashes.Count) SHA-256 value(s); the first and the last call of the round each record one")
    } elseif ($hashes[0] -ne $hashes[-1]) {
        $failures.Add("environment: the IDB changed during the round ($($hashes[0]) -> $($hashes[-1]))")
    }
}

if ($bodies.ContainsKey('commands')) {
    $commands = @()
    $fenced = $false
    foreach ($line in $bodies['commands']) {
        if ($line -match '^\s*```') { $fenced = -not $fenced; continue }
        if ($fenced -and $line.Trim() -and $line.Trim() -notmatch '^#') { $commands += $line.Trim() }
    }
    if ($commands.Count -eq 0) { $failures.Add('commands: no command in a code block') }
    $temp = @('%TEMP%', '%TMP%', '$env:TEMP', '$env:TMP', [IO.Path]::GetTempPath().TrimEnd('\'))
    foreach ($command in $commands) {
        $entry = $command -replace '^(powershell(\.exe)?|pwsh)\s+((-\w+(\s+\S+)?)\s+)*?-File\s+', ''
        $first = ($entry -split '\s+')[0].Trim('"')
        $relative = $first -notmatch '^([A-Za-z]:|[\\/])'
        if ($first -ne 'cargo' -and -not ($relative -and (Test-Path -LiteralPath (Join-Path $repo $first) -PathType Leaf))) {
            $failures.Add("commands: '$command' starts with neither tools/ida/ib.ps1, a repository script by relative path, nor cargo")
        }
        $flat = $command.Replace('/', '\').ToLowerInvariant()
        foreach ($place in $temp + $InstallPlaces) {
            if ($flat.Contains($place.Replace('/', '\').ToLowerInvariant())) { $failures.Add("commands: '$command' names $place") }
        }
        if ($command -match '\bida(lib|ui)-\d+\b') { $failures.Add("commands: '$command' names an IDA instance") }
        foreach ($match in [regex]::Matches($command, 'tools[\\/]ida[\\/][\w.\\/-]+')) {
            $file = $match.Value.Replace('\', '/')
            # Windows PowerShell turns a native command's redirected stderr into
            # a terminating error under 'Stop'.
            $ErrorActionPreference = 'Continue'
            & git -C $repo ls-files --error-unmatch -- $file *> $null
            $tracked = $LASTEXITCODE -eq 0
            $ErrorActionPreference = 'Stop'
            if (-not $tracked) { $failures.Add("commands: $file is not tracked by git") }
        }
    }
}

if ($bodies.ContainsKey('conclusions')) {
    foreach ($line in $bodies['conclusions']) {
        foreach ($match in [regex]::Matches($line, '\b0x\w+')) {
            if ($match.Value -notmatch '^0x[0-9A-Fa-f]+$') { $failures.Add("conclusions: $($match.Value) is not hexadecimal") }
        }
        foreach ($match in [regex]::Matches($line, '@\s*`?(\d+)\b')) {
            $failures.Add("conclusions: address $($match.Groups[1].Value) after @ is not written in 0x hexadecimal")
        }
    }
}

if ($failures.Count -gt 0) {
    $failures | ForEach-Object { Write-Output "FAIL $_" }
    exit 1
}
Write-Output "PASS $doc"
exit 0
