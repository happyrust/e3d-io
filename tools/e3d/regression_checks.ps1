# Pure parsing and integrity helpers; no E3D process is launched here.
function Get-RegressionTests([string]$Text) {
    $rows = @([regex]::Matches($Text, 'test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;'))
    $sum = @{ passed=0; failed=0; ignored=0; result_lines=$rows.Count; skip_messages=0 }
    foreach ($row in $rows) { $sum.passed += [int]$row.Groups[1].Value; $sum.failed += [int]$row.Groups[2].Value; $sum.ignored += [int]$row.Groups[3].Value }
    $sum.skip_messages = @([regex]::Matches($Text, '(?im)^\s*(?:SKIP|skipped):')).Count
    $sum.skip_lines = @([regex]::Matches($Text, '(?im)^\s*(?:SKIP|skipped):[^\r\n]*') | ForEach-Object { $_.Value.Trim() })
    $sum
}
function Get-RegressionWarnings([string]$Text) {
    $keys=@([regex]::Matches($Text, '(?m)^warning(?:\[[^\]]+\])?: ([^\r\n]+)(?:\r?\n\s*-->\s*([^\r\n]+))?') | ForEach-Object {
        if($_.Groups[1].Value -notmatch 'generated \d+ warning') { $_.Groups[1].Value+'|'+$_.Groups[2].Value.Trim() }
    } | Sort-Object -Unique)
    $keys.Count
}
function Get-RegressionVerdicts([string]$Text, [string[]]$Required, [int]$ExitCode) {
    $rows = @([regex]::Matches($Text, '(?m)^(PASS|FAIL) ([A-Za-z0-9]+)(?:\s|$)'))
    $errors = [Collections.Generic.List[string]]::new()
    if ($ExitCode -ne 0) { $errors.Add("exit code $ExitCode") }
    foreach ($id in $Required) {
        $found = @($rows | Where-Object { $_.Groups[2].Value -ceq $id })
        if ($found.Count -ne 1 -or $found[0].Groups[1].Value -ne 'PASS') { $errors.Add("missing, duplicate or failed verdict: $id") }
    }
    foreach ($row in $rows) { if ($row.Groups[1].Value -eq 'FAIL') { $errors.Add("failed verdict: $($row.Groups[2].Value)") } }
    @{ ok=($errors.Count -eq 0); errors=@($errors.ToArray()); verdicts=@($rows | ForEach-Object { $_.Value.Trim() }) }
}
function Get-RegressionSnapshot([string]$Root) {
    if (-not (Test-Path -LiteralPath $Root)) { throw "Missing integrity input: $Root" }
    @(Get-ChildItem -LiteralPath $Root -File -Recurse | Sort-Object FullName | ForEach-Object {
        [ordered]@{ path=[IO.Path]::GetRelativePath($Root,$_.FullName); length=$_.Length; sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash }
    })
}
function Test-RegressionSnapshot($Before, $After) {
    (ConvertTo-Json -InputObject @($Before) -Depth 8 -Compress) -ceq (ConvertTo-Json -InputObject @($After) -Depth 8 -Compress)
}
function Test-RegressionOverlap([string]$A,[string]$B) {
    $aPath=[IO.Path]::GetFullPath($A).TrimEnd('\','/')
    $bPath=[IO.Path]::GetFullPath($B).TrimEnd('\','/')
    $aPath.Equals($bPath,[StringComparison]::OrdinalIgnoreCase) -or $aPath.StartsWith($bPath+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase) -or $bPath.StartsWith($aPath+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)
}
function Protect-RegressionText([string]$Text) {
    $Text -replace '(?i)SYSTEM/[^\s"''<>]+','SYSTEM/[REDACTED]'
}
function Test-RegressionManifest([string]$Root,$Manifest) {
    if($null -eq $Manifest -or @($Manifest).Count -eq 0){return $false}
    try {
        $rootPath=[IO.Path]::GetFullPath($Root).TrimEnd('\','/')+[IO.Path]::DirectorySeparatorChar
        $seen=[Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        foreach ($entry in $Manifest) {
            if([string]::IsNullOrWhiteSpace($entry.path) -or [IO.Path]::IsPathRooted($entry.path)){return $false}
            $path=[IO.Path]::GetFullPath((Join-Path $Root $entry.path))
            if(-not $path.StartsWith($rootPath,[StringComparison]::OrdinalIgnoreCase) -or -not $seen.Add($path)){return $false}
            $length=0L
            if(-not [long]::TryParse([string]$entry.length,[ref]$length) -or $length -lt 0 -or $entry.sha256 -notmatch '^[0-9A-Fa-f]{64}$'){return $false}
            if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { return $false }
            if ((Get-Item -LiteralPath $path).Length -ne $length -or (Get-FileHash -LiteralPath $path).Hash -ne $entry.sha256) { return $false }
        }
    } catch {return $false}
    $true
}
function Test-RegressionRecord([string]$Root,[bool]$RequireTTY=$false) {
    try {
        $manifest=Get-Content -LiteralPath (Join-Path $Root 'manifest.json') -Raw | ConvertFrom-Json
        if(-not(Test-RegressionManifest $Root $manifest)){return $false}
        $listed=@($manifest | ForEach-Object {$_.path.Replace('\','/')})
        if('results.json' -notin $listed -or 'summary.md' -notin $listed){return $false}
        $result=Get-Content -LiteralPath (Join-Path $Root 'results.json') -Raw | ConvertFrom-Json
        if($result.schema -ne 'e3dio-regression/v1' -or $result.mode -notin @('full','offline') -or $result.tty_verified -isnot [bool]){return $false}
        foreach($stage in $result.stages) {
            if($stage.status -notin @('passed','failed','not_run')){return $false}
            if($stage.status -eq 'passed' -and ($null -eq $stage.exit_code -or $stage.exit_code -ne 0)){return $false}
            if($stage.status -eq 'passed') {
                $log="$($stage.name).log"
                if($log -notin $listed -or -not(Test-RegressionEvidence $Root @($log))){return $false}
            }
        }
        $status=Get-RegressionStatus $result.stages ($result.mode -eq 'offline') @($result.errors).Count
        if($result.status -ne $status -or $result.tty_verified -ne ($status -eq 'passed')){return $false}
        if($RequireTTY -and -not $result.tty_verified){return $false}
        if($result.status -ne 'failed') {
            foreach($name in @('ams','tes','golden')) {
                $metric=$result.integrity.$name
                if($result.mode -eq 'offline' -and $name -ne 'golden' -and $metric.status -eq 'not_available'){continue}
                if($metric.status -ne 'unchanged'){return $false}
                $a="$name-before.json"; $b="$name-after.json"
                if($a -notin $listed -or $b -notin $listed){return $false}
                $before=Get-Content -LiteralPath (Join-Path $Root $a) -Raw | ConvertFrom-Json
                $after=Get-Content -LiteralPath (Join-Path $Root $b) -Raw | ConvertFrom-Json
                if(-not(Test-RegressionSnapshot $before $after)){return $false}
            }
        }
    } catch {return $false}
    $true
}
function Get-RegressionStatus($Stages,[bool]$Offline,[int]$Errors) {
    if($Errors -gt 0 -or @($Stages | Where-Object status -eq 'failed').Count){return 'failed'}
    $required=if($Offline){@('tests','clippy','doc','q13-offline','q14-offline','q15-offline')}else{@('tests','clippy','doc','q13-offline','q14-offline','q15-offline','write','alternate','format0')}
    foreach($name in $required){$found=@($Stages | Where-Object name -eq $name);if($found.Count -ne 1 -or $found[0].status -ne 'passed'){return 'failed'}}
    if($Offline){'offline_passed'}else{'passed'}
}
function Test-RegressionEvidence([string]$Root,[string[]]$Required) {
    foreach($relative in $Required){$path=Join-Path $Root $relative;if(-not(Test-Path -LiteralPath $path) -or (Get-Item -LiteralPath $path).Length -eq 0){return $false}}
    $true
}
