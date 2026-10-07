#Requires -Version 7.0
# Read-only verification; failure archives can be valid records, never TTY passes.
[CmdletBinding()]
param([Parameter(Mandatory)][string]$RunDirectory,[switch]$RequireTTY)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'regression_checks.ps1')
if(-not(Test-RegressionRecord $RunDirectory $RequireTTY.IsPresent)) {
    Write-Output 'FAIL ARCHIVE: missing, invalid or inconsistent evidence; TTY acceptance not certified'
    exit 1
}
$result=Get-Content -LiteralPath (Join-Path $RunDirectory 'results.json') -Raw | ConvertFrom-Json
Write-Output "PASS ARCHIVE: status=$($result.status); tty_verified=$($result.tty_verified)"
