<#
.SYNOPSIS
  One function's pseudocode through tools/ida/ib.ps1, a row per line as
  "n  line", n being the pseudocode table's row number that a
  Phase_3_Evidence_Doc cites as pseudocode.n.

.DESCRIPTION
  powershell -NoProfile -File tools/ida/pseudo.ps1 [--target-idb <path>] 0x<function ea> [<first n> [<last n>]]

  --target-idb goes to tools/ida/ib.ps1 (default there: Core_IDB). Exit
  codes are tools/ida/ib.ps1's.
#>
$ErrorActionPreference = 'Stop'

$wrapper = @()
$rest = @($args)
if ($rest.Count -ge 2 -and $rest[0] -eq '--target-idb') {
    $wrapper = @('--target-idb', [string]$rest[1])
    $rest = @($rest | Select-Object -Skip 2)
}
if ($rest.Count -lt 1 -or $rest.Count -gt 3 -or [string]$rest[0] -notmatch '^0x[0-9A-Fa-f]+$') {
    [Console]::Error.WriteLine('usage: tools/ida/pseudo.ps1 [--target-idb <path>] 0x<function ea> [<first n> [<last n>]]')
    exit 2
}
$where = "func_ea = $($rest[0])"
if ($rest.Count -ge 2) { $where += " AND n >= $([int]$rest[1])" }
if ($rest.Count -ge 3) { $where += " AND n <= $([int]$rest[2])" }

$out = & powershell -NoProfile -File (Join-Path $PSScriptRoot 'ib.ps1') @wrapper exec --json --sql "SELECT n, line FROM pseudocode WHERE $where ORDER BY n"
if ($LASTEXITCODE -ne 0) {
    $out | ForEach-Object { [Console]::Error.WriteLine($_) }
    exit $LASTEXITCODE
}
$response = ($out -join "`n") | ConvertFrom-Json
foreach ($row in $response.result.rows) {
    Write-Output ('{0,4}  {1}' -f $row.n, $row.line)
}
