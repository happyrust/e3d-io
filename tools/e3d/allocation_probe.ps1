# Read-only source, isolated project copy; native NEW/delete/NEW allocation evidence.
[CmdletBinding()]
param(
    [string]$Project = 'D:\AVEVA\Projects\E3D3.1\AvevaMarineSample',
    [string]$Scratch = 'D:\e3dio-alloc-probe',
    [string]$L3Exe = 'D:\Rust\target\debug\l3_suite.exe',
    [string]$GenModel = 'D:\work\plant-code\old\gen-model',
    [string]$Descriptor = 'D:\Rust\target\debug\e3d-descriptor.exe',
    [string]$Login = 'e3dio/XXXXXX'
)
$ErrorActionPreference = 'Stop'
$source = Join-Path $Project 'ams000\ams8000_0001'
$copy = Join-Path $Scratch 'AvevaMarineSample'
$db = Join-Path $copy 'ams000\ams8000_0001'
$beforeHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
$beforeLength = (Get-Item -LiteralPath $source).Length
New-Item -ItemType Directory -Force $Scratch, $copy | Out-Null
robocopy $Project $copy /E /XD amspic amsdrg /R:1 /W:1 /NFL /NDL /NJH /NJS /NP | Out-Null
if ($LASTEXITCODE -ge 8) { throw "robocopy failed: $LASTEXITCODE" }
Copy-Item -LiteralPath $source -Destination $db -Force
function Snapshot([string]$file, [string]$label) {
    $json = & $Descriptor next-word1 --db $file --extent 24384 | Out-String
    if ($LASTEXITCODE) { throw "next-word1 failed: $label" }
    [IO.File]::WriteAllText((Join-Path $Scratch "$label.json"), $json)
    $json | ConvertFrom-Json
}
$pre = Snapshot $db 'before-login'
$macro = Join-Path $Scratch 'allocation.mac'
$log = Join-Path $Scratch 'allocation.log'
$preNew = Join-Path $Scratch 'before-new.db'
$postNew = Join-Path $Scratch 'after-new.db'
$macroText = @"
ALPHA LOG "$($log.Replace('\','/'))" OVER
`$P ALLOC-BEGIN
Q USER
SYSCOM 'cmd.exe /c copy /y "$db" "$preNew"'
/1RX03-MSUP
NEW EQUI /E3DIO-ALLOC-FIRST
`$P ALLOC-FIRST-BEGIN
Q REF
`$P ALLOC-FIRST-END
SAVEWORK
`$P ALLOC-SAVE-FIRST-BEGIN
Q SESSION
`$P ALLOC-SAVE-FIRST-END
SYSCOM 'cmd.exe /c copy /y "$db" "$postNew"'
DELETE EQUI
NEW EQUI /E3DIO-ALLOC-SECOND
`$P ALLOC-SECOND-BEGIN
Q REF
`$P ALLOC-SECOND-END
SAVEWORK
`$P ALLOC-SAVE-SECOND-BEGIN
Q SESSION
`$P ALLOC-SAVE-SECOND-END
`$P ALLOC-END
ALPHA LOG END
"@
[IO.File]::WriteAllText($macro, $macroText, [Text.Encoding]::ASCII)
$out = Join-Path $Scratch 'driver'
$previous = $env:L3_ALLOW_EXISTING_E3D_SESSION
$env:L3_ALLOW_EXISTING_E3D_SESSION = '1'
try {
    Push-Location $GenModel
    try {
        & $L3Exe --check-driver $macro --project-dir $copy --output $out --target-db-file $db --aios-project e3dio-alloc-probe --e3d-login $Login --alive-timeout-secs 60 2>&1 | Tee-Object -FilePath (Join-Path $Scratch 'driver.log')
        $driverExit = $LASTEXITCODE
    } finally { Pop-Location }
    if ($driverExit) { throw "allocation driver failed ($driverExit); logs: $out" }
    if (-not (Test-Path -LiteralPath $log)) { throw 'missing allocation log' }
    $text = [IO.File]::ReadAllText($log)
    if ($text -match '(?im)^.*(ERROR|\(\d+,\d+\)).*$') { throw "native allocation error; see $log" }
    $refs = @{}
    $saved = @{}
    foreach ($label in @('FIRST', 'SECOND')) {
        $m = [regex]::Match($text, "(?ms)^ALLOC-$label-BEGIN\s*\r?\n(.*?)^ALLOC-$label-END\s*$")
        if (-not $m.Success -or $m.Groups[1].Value -notmatch 'Ref\s*=\s*(\d+/\d+)') { throw "missing Q REF result: $label" }
        Write-Output "PROBE $label $($Matches[1])"
        $refs[$label] = $Matches[1]
        $s = [regex]::Match($text, "(?ms)^ALLOC-SAVE-$label-BEGIN\s*\r?\n(.*?)^ALLOC-SAVE-$label-END\s*$")
        if (-not $s.Success -or $s.Groups[1].Value -notmatch 'Last saved session number (\d+)') { throw "missing SAVEWORK session: $label" }
        $saved[$label] = [int]$Matches[1]
    }
    $beforeNew = Snapshot $preNew 'before-new'
    $first = Snapshot $postNew 'after-first-save'
    $last = Snapshot $db 'after-exit'
    if ($refs.FIRST -ne "24384/$($beforeNew.bucket_counter)" -or $refs.SECOND -ne "24384/$($beforeNew.bucket_counter + 1)") { throw 'NEW refnos disagree with stored next-available counter' }
    if ($first.sesno -ne $saved.FIRST -or $first.bucket_counter -ne $beforeNew.bucket_counter + 1 -or $last.bucket_counter -ne $beforeNew.bucket_counter + 2) { throw 'SAVEWORK counter/session mismatch' }
    [ordered]@{ source_sha256=$beforeHash; source_length=$beforeLength; native_login_user=([regex]::Match($text, '(?m)^User: (.+)$').Groups[1].Value.Trim()); refs=$refs; savework_sessions=$saved; before_login=$pre; before_new=$beforeNew; after_first_save=$first; after_exit=$last } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $Scratch 'result.json') -Encoding utf8
    Write-Output 'PASS allocation next-available counter; deletion does not reuse the first refno'
} finally {
    if ($null -eq $previous) { Remove-Item Env:\L3_ALLOW_EXISTING_E3D_SESSION -ErrorAction SilentlyContinue } else { $env:L3_ALLOW_EXISTING_E3D_SESSION = $previous }
    if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -ne $beforeHash -or (Get-Item -LiteralPath $source).Length -ne $beforeLength) { throw 'SOURCE DATABASE CHANGED' }
    Write-Output "SOURCE unchanged SHA256=$beforeHash length=$beforeLength"
}
