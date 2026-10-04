$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'q13_checks.ps1')
. (Join-Path $PSScriptRoot 'q14_checks.ps1')
$valid = "[Q14-BEGIN]`n/E3DIO-Q14-NAMED`n[Q14-RESULT-BEGIN]`nQ REF`nRef =24384/26506`n[Q14-RESULT-END]`n[Q14-END]"
if (-not (Test-Q14Ref $valid '24384/26506')) { throw 'valid Q REF rejected' }
$bad = @($valid.Replace('Ref =24384/26506','Q REF =24384/26506'),
    $valid.Replace('Ref =24384/26506',''), $valid.Replace('[Q14-END]',''),
    $valid.Replace('Q REF', 'Q REF (2,536)'), $valid.Replace('26506','26507'),
    ($valid + "`n[Q14-END]"), $valid.Replace('Ref =24384/26506',"Ref =24384/26506`nRef =24384/26506"))
foreach ($text in $bad) { if (Test-Q14Ref $text '24384/26506') { throw "invalid Q REF accepted: $text" } }
foreach ($file in @('write_gate.ps1','q14_checks.ps1','q14_selfcheck.ps1')) {
    $tokens = $null; $errors = $null
    $null = [Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot $file), [ref]$tokens, [ref]$errors)
    if ($errors.Count) { throw ($errors | Out-String) }
}
$json = Get-Content (Join-Path $PSScriptRoot 'write_gate_ops/q14_named.json') -Raw | ConvertFrom-Json
if ($json.ops.Count -ne 1 -or $json.ops[0].name -ne '/E3DIO-Q14-NAMED') { throw 'Q14 op mismatch' }
$template = Get-Content (Join-Path $PSScriptRoot 'write_gate.mac.template') -Raw
$body = ($template -split '-- Q14-TEMPLATE-BEGIN\r?\n',2)[1]
$body = $body.Replace('{{LOGDIR}}','D:/scratch/logs').Replace('{{MACROLOG}}','D:/scratch/Q14.log')
if ($body -match '\{\{[^}]+\}\}') { throw 'unresolved Q14 placeholders' }
Write-Output 'PASS Q14 offline: valid result + 7 refusal cases, syntax, ops and macro rendering'
