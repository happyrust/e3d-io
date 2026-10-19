#Requires -Version 7.0
[CmdletBinding()]
param(
    [string]$OutputRoot='D:\e3dio-regression',
    [string]$AMSProject='D:\AVEVA\Projects\E3D3.1\AvevaMarineSample',
    [string]$TESProject='D:\AVEVA\Projects\E3D3.1\TEST',
    [string]$GenModel='D:\work\plant-code\old\gen-model',
    [string]$L3Exe='D:\Rust\target\debug\l3_suite.exe',
    [string]$E3DInstallDir='E:\reverse\e3d\shadow_e3d31_gen_model_test',
    [string]$Format0Launcher='D:\work\plant-code\old\gen-model\scripts\e3d\run_ams_c_entrymacro.bat',
    [switch]$OfflineOnly
)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'regression_checks.ps1')
$crate=(Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
foreach ($source in @($AMSProject,$TESProject)) {
    if (Test-RegressionOverlap $OutputRoot $source) { throw 'OutputRoot must not overlap a source project' }
}
$run=Join-Path $OutputRoot ((Get-Date -Format 'yyyyMMdd-HHmmss')+'-'+[guid]::NewGuid().ToString('N').Substring(0,8))
New-Item -ItemType Directory -Path $run | Out-Null
$stageNames=@('tests','clippy','doc','q13-offline','q14-offline','q15-offline','write','alternate','format0')
$results=[ordered]@{ schema='e3dio-regression/v1'; mode=$(if($OfflineOnly){'offline'}else{'full'}); started_at=[DateTimeOffset]::Now.ToString('o'); head=''; working_tree=@(); tools=@(); stages=@(); integrity=@{}; status='failed'; errors=@(); tty_verified=$false }
foreach($name in $stageNames) { $results.stages+= [ordered]@{name=$name;status='not_run';exit_code=$null;command=$null;started_at=$null;ended_at=$null;elapsed_seconds=$null;details=$null} }
function Write-Json($Value,[string]$Path) { [IO.File]::WriteAllText($Path,(ConvertTo-Json -InputObject $Value -Depth 30),[Text.UTF8Encoding]::new($false)) }
function Invoke-RegressionStage([string]$Name,[string]$Exe,[string[]]$Arguments) {
    $stage=$results.stages | Where-Object name -eq $Name
    $stage.command=@($Exe)+$Arguments
    $stage.started_at=[DateTimeOffset]::Now.ToString('o')
    $timer=[Diagnostics.Stopwatch]::StartNew()
    $writer=[IO.StreamWriter]::new((Join-Path $run "$Name.log"),$false,[Text.UTF8Encoding]::new($false))
    $stage.status='failed'
    Write-Host "START $Name"
    try {
        $saved=$ErrorActionPreference; $ErrorActionPreference='Continue'
        try { & $Exe @Arguments 2>&1 | ForEach-Object { $writer.WriteLine((Protect-RegressionText ([string]$_))) }; $stage.exit_code=$LASTEXITCODE }
        finally { $ErrorActionPreference=$saved }
    } finally { $writer.Dispose(); $stage.ended_at=[DateTimeOffset]::Now.ToString('o'); $stage.elapsed_seconds=$timer.Elapsed.TotalSeconds }
    $text=[IO.File]::ReadAllText((Join-Path $run "$Name.log"))
    if($Name -eq 'tests'){$stage.details=Get-RegressionTests $text}
    if ($Name -eq 'tests') {
        $stage.details=Get-RegressionTests $text
        if ($stage.details.result_lines -lt 1 -or $stage.details.failed -ne 0 -or $stage.details.ignored -gt 35 -or $stage.details.skip_messages -gt 9) { throw 'Test results exceed failure/ignored/skip baseline or are missing' }
    } elseif ($Name -in @('clippy','doc')) {
        $warnings=Get-RegressionWarnings $text
        $stage.details=@{warnings=$warnings}
        $limit=if($Name -eq 'clippy'){63}else{26}
        if($warnings -gt $limit) { throw "$Name warnings $warnings exceed $limit" }
    } elseif ($Name -like '*-offline') {
        $id=$Name.Split('-')[0].ToUpperInvariant()
        if ($text -notmatch "(?m)^PASS $id offline") { throw "$Name missing offline verdict" }
    } else {
        $required=switch($Name) {
            write { @('Q0','Q1','Q2','Q3a','Q3b','Q3c','Q3d','Q3e','Q3f','Q4a','Q4b','Q5','Q6','Q6b','Q7','Q8a','Q8b','Q9a','Q9b','Q9c','Q9d','Q10','Q11a','Q11b','Q12a','Q12b','Q12c','Q12d','Q12e','Q13','Q13a','Q13b','Q13c','Q13d','Q13e','Q13f','Q13g','Q13h','Q14') }
            alternate { @('T4a','T4b','T4c','Q15') }
            format0 { @('E1','E2','F0','F1','F2a','F2b','F2c','F3a','F3b','F4','F5') }
        }
        $stage.details=Get-RegressionVerdicts $text $required $stage.exit_code
        if(-not $stage.details.ok) { throw ($stage.details.errors -join '; ') }
        $stage.details.times=@([regex]::Matches($text,'(?m)^TIME (\w+)=([\d.]+)') | ForEach-Object { @{name=$_.Groups[1].Value;seconds=[double]::Parse($_.Groups[2].Value,[Globalization.CultureInfo]::InvariantCulture)} })
        $logFolder=switch($Name){write {'logs'} alternate {'logs-alt'} format0 {'logs-format0'}}
        if(@(Get-ChildItem (Join-Path $run "$Name\$logFolder") -File -Filter '*.log').Count -eq 0) { throw "$Name missing original logs" }
        $requiredFiles=switch($Name) {
            write { @('Q13g.log','Q13g-spamap.json','Q13g-check.json','Q13g-world-frame.json','q13-refnos.json','q13-probes.jsonl','Q14.log') }
            alternate { @('alt_e3d_1.log','alt_e3d_2.log','alt_e3d_3.log','alt_e3d_4.log','alt_e3d_5.log','alt_readback.log','q15.json') }
            format0 { @('00_project.log','10_control.log','20_zone_name.log','40_equi.log','50_zone_members.log','30_zone_desc.log','60_session.log') }
        }
        if(-not(Test-RegressionEvidence (Join-Path $run "$Name\$logFolder") $requiredFiles)){throw "$Name missing or empty original evidence"}
    }
    if ($stage.exit_code -ne 0) { throw "$Name exit code $($stage.exit_code); see $Name.log" }
    $stage.status='passed'
    Write-Host "PASS $Name ($([Math]::Round($timer.Elapsed.TotalSeconds,2)) s)"
}
$sources=@(@{name='ams';root=(Join-Path $AMSProject 'ams000')},@{name='tes';root=(Join-Path $TESProject 'TES000')},@{name='golden';root=(Join-Path $crate 'tests\golden')})
$before=@{}
Push-Location $crate
try {
    $results.head=(& git rev-parse HEAD | Out-String).Trim()
    $results.working_tree=@(& git status --short)
    if(-not (Test-Path -LiteralPath $L3Exe)) {
        $fallback=@('D:\Rust\target-l3\debug\l3_suite.exe','D:\Rust\target\release\l3_suite.exe') | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
        if($fallback){$L3Exe=$fallback}
    }
    foreach($tool in @((Get-Command cargo).Source,(Get-Command pwsh).Source,$L3Exe,$Format0Launcher)) {
        $results.tools+=@{path=$tool;sha256=$(if(Test-Path -LiteralPath $tool){(Get-FileHash -LiteralPath $tool).Hash}else{$null})}
    }
    foreach($source in $sources) {
        if($OfflineOnly -and $source.name -ne 'golden' -and -not(Test-Path -LiteralPath $source.root)) { $results.integrity[$source.name]=@{status='not_available'}; continue }
        $before[$source.name]=Get-RegressionSnapshot $source.root
        Write-Json $before[$source.name] (Join-Path $run "$($source.name)-before.json")
    }
    Invoke-RegressionStage 'tests' 'cargo' @('test','--no-fail-fast','--','--nocapture')
    Invoke-RegressionStage 'clippy' 'cargo' @('clippy','--all-targets')
    Invoke-RegressionStage 'doc' 'cargo' @('doc')
    foreach($q in @('q13','q14','q15')) { Invoke-RegressionStage "$q-offline" 'pwsh' @('-NoProfile','-File',(Join-Path $PSScriptRoot "$($q)_selfcheck.ps1")) }
    $meta=& cargo metadata --format-version 1 --no-deps | ConvertFrom-Json
    if($LASTEXITCODE -ne 0){throw 'cargo metadata failed'}
    foreach($binary in @('e3d-write.exe','e3d-descriptor.exe')) {
        $tool=Join-Path $meta.target_directory "debug\$binary"
        if(Test-Path -LiteralPath $tool){$results.tools+=@{path=$tool;sha256=(Get-FileHash -LiteralPath $tool).Hash}}
    }
    if(-not $OfflineOnly) {
        # run_ams_c_entrymacro.bat defaults to the unverified e3d31-runtime install; its
        # E3DNativeFixup build pre-initialises Core3D at OpenProject and aborts TTY
        # sessions with 0xC0020001. Pin the verified shadow runtime unless overridden.
        if(-not $env:L3_E3D_INSTALL_DIR){$env:L3_E3D_INSTALL_DIR=$E3DInstallDir}
        foreach($needed in @($GenModel,$L3Exe,$Format0Launcher,(Join-Path $env:L3_E3D_INSTALL_DIR 'des.exe'))) { if(-not(Test-Path -LiteralPath $needed)){throw "Missing TTY dependency: $needed"} }
        Invoke-RegressionStage 'write' 'pwsh' @('-NoProfile','-File',(Join-Path $PSScriptRoot 'write_gate.ps1'),'-Project',$AMSProject,'-Scratch',(Join-Path $run 'write'),'-GenModel',$GenModel,'-L3Exe',$L3Exe)
        Invoke-RegressionStage 'alternate' 'pwsh' @('-NoProfile','-File',(Join-Path $PSScriptRoot 'alternate_write_gate.ps1'),'-Rounds','5','-Project',$AMSProject,'-Scratch',(Join-Path $run 'alternate'),'-GenModel',$GenModel,'-L3Exe',$L3Exe)
        Invoke-RegressionStage 'format0' 'pwsh' @('-NoProfile','-File',(Join-Path $PSScriptRoot 'format0_gate.ps1'),'-Project',$TESProject,'-Scratch',(Join-Path $run 'format0'),'-Launcher',$Format0Launcher)
    }
    $results.status=Get-RegressionStatus $results.stages $OfflineOnly.IsPresent $results.errors.Count
} catch { $results.errors+=Protect-RegressionText $_.Exception.Message }
finally {
    foreach($source in $sources) {
        if(-not $before.ContainsKey($source.name)){continue}
        try {
            $after=Get-RegressionSnapshot $source.root
            Write-Json $after (Join-Path $run "$($source.name)-after.json")
            $same=Test-RegressionSnapshot $before[$source.name] $after
            $results.integrity[$source.name]=@{status=$(if($same){'unchanged'}else{'changed'})}
            if(-not $same){$results.errors+="$($source.name) integrity changed"}
        } catch { $results.errors+=Protect-RegressionText $_.Exception.Message }
    }
    $results.status=Get-RegressionStatus $results.stages $OfflineOnly.IsPresent $results.errors.Count
    $results.tty_verified=($results.status -eq 'passed' -and -not $OfflineOnly)
    $results.ended_at=[DateTimeOffset]::Now.ToString('o')
    # Only text evidence leaves scratch; project databases are deliberately excluded.
    foreach($name in @('write','alternate','format0')) {
        $scratch=Join-Path $run $name
        if(-not(Test-Path $scratch)){continue}
        $archive=Join-Path $run "evidence\$name"
        foreach($file in @(Get-ChildItem $scratch -File -Recurse | Where-Object { $_.FullName -notlike "*\AvevaMarineSample\*" -and $_.FullName -notlike "*\TEST\*" -and $_.Extension -in @('.log','.txt','.json','.jsonl','.mac') })) {
            $relative=[IO.Path]::GetRelativePath($scratch,$file.FullName)
            $dest=Join-Path $archive $relative
            New-Item -ItemType Directory -Force (Split-Path $dest) | Out-Null
            # Latin1 maps each byte exactly, preserving native log encodings.
            $raw=[Text.Encoding]::Latin1.GetString([IO.File]::ReadAllBytes($file.FullName))
            [IO.File]::WriteAllBytes($dest,[Text.Encoding]::Latin1.GetBytes((Protect-RegressionText $raw)))
        }
    }
    Write-Json $results (Join-Path $run 'results.json')
    $summary=@('# E3D regression',"Status: $($results.status)","HEAD: $($results.head)","TTY verified: $($results.tty_verified)",'','| Stage | Status | Seconds |','|---|---|---|')
    foreach($s in $results.stages){$summary+="| $($s.name) | $($s.status) | $($s.elapsed_seconds) |"}
    $tests=($results.stages | Where-Object name -eq 'tests').details
    $summary+=@('',"Tests: passed=$($tests.passed), failed=$($tests.failed), ignored=$($tests.ignored), sample skip messages=$($tests.skip_messages)",'Sample skip messages (not acceptance passes):')
    $summary+=@($tests.skip_lines)
    $summary+=@('',"TTY acceptance: $(if($results.tty_verified){'verified'}else{'未验收真实 TTY'})",'',($results.errors -join "`n"))
    [IO.File]::WriteAllLines((Join-Path $run 'summary.md'),[string[]]$summary)
    $manifest=@(Get-ChildItem $run -File -Recurse | Where-Object { $_.FullName -notlike "$run\write\*" -and $_.FullName -notlike "$run\alternate\*" -and $_.FullName -notlike "$run\format0\*" -and $_.Name -ne 'manifest.json' } | Sort-Object FullName | ForEach-Object { @{path=[IO.Path]::GetRelativePath($run,$_.FullName);length=$_.Length;sha256=(Get-FileHash -LiteralPath $_.FullName).Hash} })
    Write-Json $manifest (Join-Path $run 'manifest.json')
    Pop-Location
}
Write-Host "RESULT $($results.status): $run"
if($results.status -eq 'failed'){exit 1}
