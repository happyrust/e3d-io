<#
.SYNOPSIS
  One SQL query through tools/ida/ib.ps1, the rows printed tab-separated
  under a header line of column names.

.DESCRIPTION
  powershell -NoProfile -File tools/ida/sql.ps1 [--target-idb <path>] "<SQL>"

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
if ($rest.Count -ne 1) {
    [Console]::Error.WriteLine('usage: tools/ida/sql.ps1 [--target-idb <path>] "<SQL>"')
    exit 2
}

$out = & powershell -NoProfile -File (Join-Path $PSScriptRoot 'ib.ps1') @wrapper exec --json --sql ([string]$rest[0])
if ($LASTEXITCODE -ne 0) {
    $out | ForEach-Object { [Console]::Error.WriteLine($_) }
    exit $LASTEXITCODE
}
$result = (($out -join "`n") | ConvertFrom-Json).result
Write-Output ($result.columns -join "`t")
foreach ($row in $result.rows) {
    Write-Output (($result.columns | ForEach-Object { $row.$_ }) -join "`t")
}
