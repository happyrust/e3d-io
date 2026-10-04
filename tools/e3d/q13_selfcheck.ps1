[CmdletBinding()]
param([switch]$RealBatches,
 [string]$SourceDb='D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001',
 [string]$WriteExe='D:\Rust\target\debug\e3d-write.exe',
 [string]$Descriptor='D:\Rust\target\debug\e3d-descriptor.exe')
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'q13_checks.ps1')
$count=0
function Assert13([bool]$condition,[string]$name) { if (-not $condition) { throw $name }; $script:count++ }
foreach ($file in Get-ChildItem $PSScriptRoot -Filter '*.ps1') {
    $tokens=$null; $errors=$null
    $null=[Management.Automation.Language.Parser]::ParseFile($file.FullName,[ref]$tokens,[ref]$errors)
    Assert13 ($errors.Count -eq 0) "syntax $($file.Name): $errors"
}
$good="[T-BEGIN]`nVAR !q EVALUATE (REF) FOR ALL BOX WITHIN VOLUME =1/2 0`n[T-RESULT-BEGIN]`n 1 =1/3`n[T-RESULT-END]`n[T-END]"
Assert13 (Test-Q13Search $good 'T' @('1/3') @('1/2')) 'valid result / echo excluded'
Assert13 (-not (Test-Q13Search $good 'T' @('1/2'))) 'echo cannot satisfy presence'
Assert13 (-not (Test-Q13Search ($good.Replace(' 1 =1/3','')) 'T')) 'missing result'
Assert13 (-not (Test-Q13Search ($good.Replace('[T-END]','')) 'T')) 'missing marker'
Assert13 (-not (Test-Q13Search ($good+"`n[T-END]") 'T')) 'duplicate marker'
Assert13 (-not (Test-Q13Search ($good.Replace(' 1 =1/3','(2,536)')) 'T')) 'error rejected'
Assert13 (Test-Q13Search "[T-BEGIN]`n(2,536)`n[T-END]" 'T' -VolumeError) 'expected control error'
$v=@(Get-Q13Verdicts @{Q13=$false;Q13a=$true;Q13h=$true})
Assert13 (@($v | Where-Object { $_ -like 'PASS*' }).Count -eq 0) 'control failure propagation'
$vars=@{ORIGIN='0 0 0';QZONE='1/2';MOVING='1/3';NEWOWNER='1/4';GROWING='1/5';DELETING='1/6'}
foreach ($file in Get-ChildItem (Join-Path $PSScriptRoot 'write_gate_ops') -Filter 'q13_*.json') {
    $text=Get-Content $file.FullName -Raw
    foreach ($k in $vars.Keys) { $text=$text.Replace("{{$k}}",$vars[$k]) }
    Assert13 ($text -notmatch '\{\{') "unresolved $file"
    $ops=$text | ConvertFrom-Json
    Assert13 ($ops.ops.Count -gt 0 -and $text -notmatch '"aabb"') "operations / no caller AABB $file"
}
if ($RealBatches) {
    $root=Join-Path ([IO.Path]::GetTempPath()) ('q13-'+[guid]::NewGuid())
    $null=New-Item $root -ItemType Directory
    $db=Join-Path $root 'ams8000_0001'
    $hash=(Get-FileHash $SourceDb).Hash
    Copy-Item $SourceDb $db
    $ids=@{}
    $schema=Join-Path $PSScriptRoot '..\..\schema\e3d31'
    function Frame13([string]$ref) {
        $out=& $Descriptor world-frame --db $db --refno $ref --schema $schema --rules confirmed
        if ($LASTEXITCODE) { throw "world-frame $ref failed" }
        $out | ConvertFrom-Json
    }
    $origin=(Frame13 '24384/24777').frame.origin_mm
    $vars.ORIGIN=($origin | ForEach-Object { ([double]$_).ToString('R',[Globalization.CultureInfo]::InvariantCulture) }) -join ' '
    foreach ($name in @('q13_control','q13_boxes','q13_move','q13_xlen','q13_delete','q13_cyli_equi')) {
        $text=Get-Content (Join-Path $PSScriptRoot "write_gate_ops/$name.json") -Raw
        foreach ($k in $vars.Keys) { $text=$text.Replace("{{$k}}",$vars[$k]) }
        $path=Join-Path $root "$name.json"; $report=Join-Path $root "$name.report.json"
        [IO.File]::WriteAllText($path,$text)
        $policy=if ($name -eq 'q13_control') {'off'} else {'require'}
        & $WriteExe apply --db $db --ops $path --report $report --user e3dio --spatial $policy
        Assert13 ($LASTEXITCODE -eq 0) "$name CLI"
        $r=Get-Content $report -Raw | ConvertFrom-Json
        $ops=$text | ConvertFrom-Json
        for ($i=0;$i -lt $ops.ops.Count;$i++) { if ($ops.ops[$i].id) { $ids[$ops.ops[$i].id]=$r.ops[$i].refno } }
        $vars.QZONE=$ids.zone;$vars.MOVING=$ids.moving;$vars.NEWOWNER=$ids.newowner;$vars.GROWING=$ids.growing;$vars.DELETING=$ids.deleting
        if ($name -eq 'q13_boxes' -or $name -eq 'q13_move') {
            $old=Test-Q13Intersection (Frame13 $ids.oldprobe).aabb (Frame13 $ids.moving).aabb 0
            $new=Test-Q13Intersection (Frame13 $ids.newprobe).aabb (Frame13 $ids.moving).aabb 0
            Assert13 ($old -eq ($name -eq 'q13_boxes') -and $new -eq ($name -eq 'q13_move')) 'move preconditions'
        }
        if ($name -eq 'q13_boxes' -or $name -eq 'q13_xlen') {
            $hit=Test-Q13Intersection (Frame13 $ids.growprobe).aabb (Frame13 $ids.growing).aabb 0
            Assert13 ($hit -eq ($name -eq 'q13_xlen')) 'growth preconditions'
        }
        if ($policy -eq 'require') {
            Assert13 ($null -ne $r.spatial -or $null -ne $r.commit.spatial) "$name spatial report"
            foreach ($id in @('main','moving','growing','controlprobe','oldprobe','newprobe','growprobe','deleteprobe','cylinder','transformed')) {
                if (-not $ids[$id]) { continue }
                $frame=Frame13 $ids[$id]
                $out=& $Descriptor spamap --db $db --refno $ids[$id]
                Assert13 ($LASTEXITCODE -eq 0) "spamap $id"
                $table=$out | ConvertFrom-Json
                Assert13 (($frame.entries | ConvertTo-Json -Depth 8 -Compress) -eq ($table.entries | ConvertTo-Json -Depth 8 -Compress)) "11 words $id"
            }
        }
    }
    Assert13 ((Get-FileHash $SourceDb).Hash -eq $hash) 'source unchanged'
    Write-Output "EVIDENCE $root"
}
Write-Output "PASS Q13 offline checks=$count"
