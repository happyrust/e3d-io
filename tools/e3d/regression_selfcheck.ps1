#Requires -Version 7.0
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'regression_checks.ps1')
$count=0
function Assert($Condition,[string]$Message) { if(-not $Condition){throw $Message}; $script:count++ }
$t=Get-RegressionTests "test result: ok. 3 passed; 0 failed; 2 ignored;`nskipped: fixture missing`ntest result: FAILED. 1 passed; 1 failed; 0 ignored;"
Assert ($t.passed -eq 4 -and $t.failed -eq 1 -and $t.ignored -eq 2 -and $t.skip_messages -eq 1) 'test counters'
Assert ((Get-RegressionWarnings "warning: a`nwarning: a`nwarning[x]: b`nwarning: crate generated 2 warnings") -eq 2) 'warning deduplication'
Assert ((Get-RegressionWarnings "warning: a`n --> a.rs:1:2`nwarning: a`n --> b.rs:1:2`nwarning: a`n --> a.rs:1:2") -eq 2) 'distinct warning locations'
Assert ((Get-RegressionWarnings ((1..64 | ForEach-Object {"warning: w$_"}) -join "`n")) -gt 63) 'clippy limit'
Assert ((Get-RegressionWarnings ((1..26 | ForEach-Object {"warning: w$_"}) -join "`n")) -gt 25) 'doc limit'
Assert (Get-RegressionVerdicts "PASS Q1`nPASS Q2" @('Q1','Q2') 0).ok 'normal verdict'
foreach($text in @('command echo PASS Q1',"PASS Q1`nPASS Q1", "PASS Q1`nFAIL Q1",'',"PASS Q2")) { Assert (-not (Get-RegressionVerdicts $text @('Q1') 0).ok) 'invalid verdict' }
Assert (-not(Get-RegressionVerdicts 'PASS Q1' @('Q1') 1).ok) 'nonzero exit'
Assert (Test-RegressionOverlap 'C:\source' 'C:\source\child') 'child overlap'
Assert (-not(Test-RegressionOverlap 'C:\source' 'C:\source-other')) 'sibling safe'
Assert ((Protect-RegressionText 'des.exe ams SYSTEM/secret /ALL') -notmatch 'secret') 'credentials redacted'
$stages=@('tests','clippy','doc','q13-offline','q14-offline','q15-offline','write','alternate','format0') | ForEach-Object { @{name=$_;status='passed'} }
Assert ((Get-RegressionStatus $stages $false 0) -eq 'passed') 'full completion'
Assert ((Get-RegressionStatus $stages $true 0) -eq 'offline_passed') 'offline never full acceptance'
$stages[6].status='failed';$stages[7].status='not_run';$stages[8].status='not_run'
Assert ((Get-RegressionStatus $stages $false 0) -eq 'failed') 'failed gate leaves successors not run'
Assert ((Get-RegressionStatus $stages $true 1) -eq 'failed') 'integrity failure overrides offline success'
$temp=Join-Path ([IO.Path]::GetTempPath()) ('e3dio-regression-check-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory $temp | Out-Null
try {
    Set-Content (Join-Path $temp 'evidence.txt') 'original'
    Assert (Test-RegressionEvidence $temp @('evidence.txt')) 'raw evidence present'
    Assert (-not(Test-RegressionEvidence $temp @('missing.log'))) 'raw log missing'
    [IO.File]::WriteAllText((Join-Path $temp 'empty.log'),'')
    Assert (-not(Test-RegressionEvidence $temp @('empty.log'))) 'raw log empty'
    $before=Get-RegressionSnapshot $temp
    Assert (Test-RegressionManifest $temp $before) 'manifest valid'
    Assert (-not(Test-RegressionManifest $temp @())) 'empty manifest rejected'
    Assert (-not(Test-RegressionManifest $temp $null)) 'null manifest rejected'
    Assert (-not(Test-RegressionManifest $temp @($before[0],$before[0]))) 'duplicate manifest entries rejected'
    $bad=@{path='..\outside.txt';length=1;sha256=('A'*64)}
    Assert (-not(Test-RegressionManifest $temp @($bad))) 'outside path rejected'
    $bad.path=Join-Path $temp 'evidence.txt'
    Assert (-not(Test-RegressionManifest $temp @($bad))) 'absolute path rejected'
    $bad.path='evidence.txt';$bad.length=-1
    Assert (-not(Test-RegressionManifest $temp @($bad))) 'negative size rejected'
    $bad.length=1;$bad.sha256='invalid'
    Assert (-not(Test-RegressionManifest $temp @($bad))) 'invalid hash rejected'
    Assert (Test-RegressionSnapshot $before (Get-RegressionSnapshot $temp)) 'snapshot same'
    Set-Content (Join-Path $temp 'evidence.txt') 'changed'
    Assert (-not(Test-RegressionManifest $temp $before)) 'manifest tamper'
    Assert (-not(Test-RegressionSnapshot $before (Get-RegressionSnapshot $temp))) 'source/golden tamper'
    $parseErrors=$null; $tokens=$null
    $null=[Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'regression.ps1'),[ref]$tokens,[ref]$parseErrors)
    Assert ($parseErrors.Count -eq 0) 'runner syntax'
    # Exercise the real runner's failure/finally path without Cargo or a license.
    $shim=Join-Path $temp 'shim'
    New-Item -ItemType Directory $shim | Out-Null
    [IO.File]::WriteAllText((Join-Path $shim 'cargo.cmd'),"@echo off`r`necho test result: FAILED. 0 passed; 1 failed; 0 ignored;`r`nexit /b 1`r`n")
    $out=Join-Path $temp 'runs'
    foreach($iteration in 1..2) {
        $psi=[Diagnostics.ProcessStartInfo]::new((Get-Command pwsh).Source)
        $psi.UseShellExecute=$false; $psi.RedirectStandardOutput=$true; $psi.RedirectStandardError=$true
        foreach($arg in @('-NoProfile','-File',(Join-Path $PSScriptRoot 'regression.ps1'),'-OfflineOnly','-OutputRoot',$out,'-AMSProject',(Join-Path $temp 'absent-ams'),'-TESProject',(Join-Path $temp 'absent-tes'))){$psi.ArgumentList.Add($arg)}
        $psi.Environment['PATH']=$shim+[IO.Path]::PathSeparator+$env:PATH
        $proc=[Diagnostics.Process]::Start($psi)
        $stdout=$proc.StandardOutput.ReadToEndAsync();$stderr=$proc.StandardError.ReadToEndAsync()
        $proc.WaitForExit()
        Assert ($proc.ExitCode -ne 0) "failure run $iteration exit"
        $null=$stdout.GetAwaiter().GetResult();$null=$stderr.GetAwaiter().GetResult()
    }
    $runs=@(Get-ChildItem $out -Directory)
    Assert ($runs.Count -eq 2) 'unique failure runs'
    foreach($dir in $runs) {
        $result=Get-Content (Join-Path $dir.FullName 'results.json') -Raw | ConvertFrom-Json
        Assert ($result.status -eq 'failed' -and -not $result.tty_verified) 'failure evidence written'
        Assert (@($result.stages | Where-Object status -eq 'not_run').Count -eq 8) 'successors not run'
        $manifest=Get-Content (Join-Path $dir.FullName 'manifest.json') -Raw | ConvertFrom-Json
        Assert (Test-RegressionManifest $dir.FullName $manifest) 'failure manifest valid'
        Assert (Test-RegressionRecord $dir.FullName) 'failure archive is a valid record'
        Assert (-not(Test-RegressionRecord $dir.FullName $true)) 'failure archive never certifies TTY'
    }
    $archive=Join-Path ((Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path) 'docs\evidence\assets\2026-10-07-unified-regression'
    Assert (Test-RegressionRecord (Join-Path $archive 'full') $true) 'archived full TTY record'
    Assert (Test-RegressionRecord (Join-Path $archive 'offline-2')) 'archived offline record'
    Assert (-not(Test-RegressionRecord (Join-Path $archive 'offline-2') $true)) 'offline archive never certifies TTY'
    $record=Join-Path $temp 'record'
    New-Item -ItemType Directory $record | Out-Null
    Copy-Item (Join-Path $archive 'offline-2\*') $record
    function Update-RecordManifest {
        $entries=@(Get-RegressionSnapshot $record | Where-Object path -ne 'manifest.json')
        ConvertTo-Json -InputObject $entries -Depth 8 | Set-Content (Join-Path $record 'manifest.json')
    }
    $result=Get-Content (Join-Path $record 'results.json') -Raw | ConvertFrom-Json
    $result.stages[0].exit_code=1
    $result | ConvertTo-Json -Depth 30 | Set-Content (Join-Path $record 'results.json')
    Update-RecordManifest
    Assert (-not(Test-RegressionRecord $record)) 'passed stage with nonzero exit rejected even with valid hashes'
    $result.stages[0].exit_code=0
    $result | ConvertTo-Json -Depth 30 | Set-Content (Join-Path $record 'results.json')
    Remove-Item (Join-Path $record 'tests.log')
    Update-RecordManifest
    Assert (-not(Test-RegressionRecord $record)) 'passed stage missing log rejected even with valid hashes'
} finally { Remove-Item -LiteralPath $temp -Recurse -Force }
Write-Output "PASS REGRESSION offline checks=$count"
