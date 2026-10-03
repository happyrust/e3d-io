<#
.SYNOPSIS
  IB_Xref_Helper against the SQL xrefs table for one address (requirement 1.6,
  Property 24 of .kiro/specs/write-path-phase-3): the same reference sources,
  ordinary flow left out.

.DESCRIPTION
  powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x<address> [--target-idb <path>]

  Runs tools/ida/ib.ps1 xrefs and tools/ida/queries/xrefs_to.sql through the
  wrapper and compares the two sets of source addresses. When the helper says
  truncated, it must hold exactly 100000 sources, all of them in the SQL set.
  Prints PASS or FAIL with the counts and the differences; exit 0 on PASS.
#>
$ErrorActionPreference = 'Continue'

$address = $null
$wrapperArgs = @()
for ($i = 0; $i -lt $args.Count; $i++) {
    if ($args[$i] -eq '--target-idb' -and $i + 1 -lt $args.Count) {
        $wrapperArgs += @('--target-idb', [string]$args[$i + 1])
        $i++
    } elseif ($null -eq $address) {
        $address = [string]$args[$i]
    }
}
if ($address -notmatch '^0x[0-9A-Fa-f]+$') {
    [Console]::Error.WriteLine('usage: tools/ida/check_xrefs.ps1 0x<address> [--target-idb <path>]')
    exit 2
}
$ib = Join-Path $PSScriptRoot 'ib.ps1'

function ConvertTo-Address($value) {
    $text = [string]$value
    if ($text -match '^0x([0-9A-Fa-f]+)$') { return [Convert]::ToUInt64($Matches[1], 16) }
    return [UInt64]$text
}

$helperText = & powershell -NoProfile -File $ib @wrapperArgs xrefs $address
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$helper = ($helperText -join "`n") | ConvertFrom-Json

$sql = ((Get-Content -LiteralPath (Join-Path $PSScriptRoot 'queries\xrefs_to.sql')) | Where-Object { $_ -notmatch '^\s*--' }) -join ' '
$sql = $sql.Replace(':to_ea', $address)
$modelText = & powershell -NoProfile -File $ib @wrapperArgs exec --json --sql $sql
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$model = ($modelText -join "`n") | ConvertFrom-Json

$fromHelper = New-Object 'System.Collections.Generic.HashSet[UInt64]'
foreach ($row in @($helper.xrefs)) { [void]$fromHelper.Add((ConvertTo-Address $row.from)) }
$fromSql = New-Object 'System.Collections.Generic.HashSet[UInt64]'
foreach ($row in @($model.result.rows)) { [void]$fromSql.Add((ConvertTo-Address @($row)[0])) }

$onlyHelper = @($fromHelper | Where-Object { -not $fromSql.Contains($_) } | Sort-Object | ForEach-Object { '0x{0:X}' -f $_ })
$onlySql = @($fromSql | Where-Object { -not $fromHelper.Contains($_) } | Sort-Object | ForEach-Object { '0x{0:X}' -f $_ })
if ($helper.truncated) {
    $pass = $fromHelper.Count -eq 100000 -and $onlyHelper.Count -eq 0
    $note = "truncated at $($fromHelper.Count)"
} else {
    $pass = $onlyHelper.Count -eq 0 -and $onlySql.Count -eq 0
    $note = 'complete'
}
$verdict = $(if ($pass) { 'PASS' } else { 'FAIL' })
Write-Output "$verdict xrefs $address helper=$($fromHelper.Count) sql=$($fromSql.Count) ($note)"
if (-not $pass) {
    Write-Output "  only in the helper: $(($onlyHelper | Select-Object -First 20) -join ', ')"
    Write-Output "  only in SQL:        $(($onlySql | Select-Object -First 20) -join ', ')"
    exit 1
}
exit 0
