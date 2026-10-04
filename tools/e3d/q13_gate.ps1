# Runs in write_gate.ps1's scope, after Q12e. Each Invoke-E3D closes its TTY.
. (Join-Path $PSScriptRoot 'q13_checks.ps1')
$qclock = [Diagnostics.Stopwatch]::StartNew()
$qr = @{}
$qids = @{}
[IO.File]::WriteAllText((Join-Path $logDir 'q13-probes.jsonl'), '')
$descriptor = 'D:\Rust\target\debug\e3d-descriptor.exe'
$qschema = Join-Path $crate 'schema\e3d31'
function Get-Q13Frame([string]$ref, [int]$session = 0) {
    $args13 = @('world-frame','--db',$copyDb,'--refno',$ref,'--schema',$qschema,'--rules','confirmed')
    if ($session) { $args13 += @('--session', "$session") }
    $run = Invoke-Native { & $descriptor @args13 }
    if ($run.ExitCode) { throw $run.Output }
    $run.Output | ConvertFrom-Json
}
function Apply-Q13([string]$name, [hashtable]$vars, [string]$policy = 'require') {
    $r = Invoke-Session $name $vars @('--spatial',$policy,'--user','e3dio')
    $source = Get-Content (Join-Path $opsDir "$name.json") -Raw | ConvertFrom-Json
    for ($i=0; $i -lt $source.ops.Count; $i++) {
        if ($source.ops[$i].id) { $qids[$source.ops[$i].id] = $r.Report.ops[$i].refno }
    }
    $r
}
function Assert-Q13Probe([string]$tag,[string]$probe,[string]$element,[bool]$expected) {
    $a = (Get-Q13Frame $qids[$probe]).aabb
    $b = (Get-Q13Frame $qids[$element]).aabb
    $hit = Test-Q13Intersection $a $b 0
    $evidence = @{tag=$tag;probe=$qids[$probe];element=$qids[$element];a=$a;b=$b;clearance=0;intersects=$hit;expected=$expected}
    $json = $evidence | ConvertTo-Json -Depth 12 -Compress
    Write-Host "PROBE $json"
    Add-Content (Join-Path $logDir 'q13-probes.jsonl') $json
    if ($hit -ne $expected) { throw "$tag intersection precondition failed" }
}
function Search-Q13([string]$tag,[string]$volume,[string[]]$present=@(),[string[]]$absent=@(),[int]$clearance=0,[switch]$volumeError) {
    $body = @"
`$P [$tag-BEGIN]
VAR !q13 EVALUATE (REF) FOR ALL BOX WITHIN VOLUME =$volume $clearance
handle any
  `$P Q13-ERROR `$!!error.text
endhandle
`$P [$tag-RESULT-BEGIN]
Q VAR !q13
handle any
  `$P Q13-ERROR `$!!error.text
endhandle
`$P [$tag-RESULT-END]
`$P [$tag-END]
"@
    Invoke-E3D 'write_gate.mac.template' "$tag.mac" @{Q13='1';TAG=$tag;BODY=$body} | Out-Host
    $path = Join-Path $logDir "$tag.log"
    if (-not (Test-Path $path)) { return $false }
    Test-Q13Search -Text (Read-LogText $path) -Tag $tag -Present $present -Absent $absent -VolumeError:$volumeError
}
try {
    $origin = (Get-Q13Frame '24384/24777').frame.origin_mm
    $o = ($origin | ForEach-Object { ([double]$_).ToString('R',[Globalization.CultureInfo]::InvariantCulture) }) -join ' '
    $null = Apply-Q13 'q13_control' @{ORIGIN=$o} 'off'
    $null = Apply-Q13 'q13_boxes' @{QZONE=$qids.zone}
    Assert-Q13Probe 'control' 'controlprobe' 'control' $true
    $c1 = Search-Q13 'Q13-control-probe' $qids.controlprobe @($qids.controlprobe) @($qids.control)
    $c2 = Search-Q13 'Q13-control-volume' $qids.control -volumeError
    $qr.Q13 = $c1 -and $c2
    Write-Host "PROBE control_scoring probe=$c1 volume_error=$c2"
    if (-not $qr.Q13) { throw 'Q13 control failed; dependent questions invalid' }
    $qr.Q13a = Search-Q13 'Q13a' $qids.main @($qids.main) @() 100
    $boxes = @('main','moving','growing','deleting','controlprobe','oldprobe','newprobe','growprobe','deleteprobe') | ForEach-Object { $qids[$_] }
    $qr.Q13b = Search-Q13 'Q13b' '24384/24777' $boxes @($qids.control) 100000
    $qr.Q13c = Search-Q13 'Q13c' $qids.zone $boxes @($qids.control)
    Assert-Q13Probe 'move-old-before' 'oldprobe' 'moving' $true
    Assert-Q13Probe 'move-new-before' 'newprobe' 'moving' $false
    $d1 = Search-Q13 'Q13d-old-before' $qids.oldprobe @($qids.moving)
    $d2 = Search-Q13 'Q13d-new-before' $qids.newprobe @($qids.newprobe) @($qids.moving)
    $null = Apply-Q13 'q13_move' @{MOVING=$qids.moving;NEWOWNER=$qids.newowner}
    Assert-Q13Probe 'move-old-after' 'oldprobe' 'moving' $false
    Assert-Q13Probe 'move-new-after' 'newprobe' 'moving' $true
    $d3 = Search-Q13 'Q13d-old-after' $qids.oldprobe @($qids.oldprobe) @($qids.moving)
    $d4 = Search-Q13 'Q13d-new-after' $qids.newprobe @($qids.moving)
    $qr.Q13d = $d1 -and $d2 -and $d3 -and $d4
    Assert-Q13Probe 'grow-before' 'growprobe' 'growing' $false
    $e1 = Search-Q13 'Q13e-before' $qids.growprobe @($qids.growprobe) @($qids.growing)
    $null = Apply-Q13 'q13_xlen' @{GROWING=$qids.growing}
    Assert-Q13Probe 'grow-after' 'growprobe' 'growing' $true
    $e2 = Search-Q13 'Q13e-after' $qids.growprobe @($qids.growing)
    $qr.Q13e = $e1 -and $e2
    Assert-Q13Probe 'delete-before' 'deleteprobe' 'deleting' $true
    $f1 = Search-Q13 'Q13f-before' $qids.deleteprobe @($qids.deleting)
    $null = Apply-Q13 'q13_delete' @{DELETING=$qids.deleting}
    $f2 = Search-Q13 'Q13f-after' $qids.deleteprobe @($qids.deleteprobe) @($qids.deleting)
    $qr.Q13f = $f1 -and $f2
    Invoke-E3D 'write_gate_e3d_saves.mac.template' 'Q13g.mac' @{Q13='1';MAIN=$qids.main} -stateful | Out-Host
    $save = Get-Q13Block (Read-LogText (Join-Path $logDir 'Q13g.log')) 'Q13g'
    $m = [regex]::Matches($save,'Last saved session number (\d+)')
    if ($m.Count -ne 1 -or $save -match 'ERROR|\(\d+,\s*\d+\)') { throw 'SAVEWORK session evidence invalid' }
    $ses = [int]$m[0].Groups[1].Value
    $wf = Get-Q13Frame $qids.main $ses
    $run = Invoke-Native { & $descriptor spamap --db $copyDb --refno $qids.main --session $ses }
    if ($run.ExitCode) { throw $run.Output }
    $table = $run.Output | ConvertFrom-Json
    $run.Output | Set-Content (Join-Path $logDir 'Q13g-spamap.json')
    $check = Invoke-Native { & $descriptor spamap-check --input $copyDb --schema $qschema --rules confirmed --refno $qids.main --session "$copyDb=$ses" }
    $check.Output | Set-Content (Join-Path $logDir 'Q13g-check.json')
    $qr.Q13g = -not $check.ExitCode -and (($wf.entries | ConvertTo-Json -Compress -Depth 8) -eq ($table.entries | ConvertTo-Json -Compress -Depth 8))
    if (-not $check.ExitCode) {
        $checked = $check.Output | ConvertFrom-Json
        foreach ($item in @('key0','entries_k','flags','built_in_aabb','coverage')) {
            $metric = $checked.databases[0].$item
            if ($metric.total -lt 1 -or $metric.agree -ne $metric.total -or $metric.waiting -ne 0) { $qr.Q13g=$false }
        }
    }
    $wf | ConvertTo-Json -Depth 16 | Set-Content (Join-Path $logDir 'Q13g-world-frame.json')
    Write-Host "PROBE SAVEWORK session=$ses refno=$($qids.main)"
    $null = Apply-Q13 'q13_cyli_equi' @{QZONE=$qids.zone}
    $body = "`$P [Q13h-BEGIN]`nMM DISTANCE`nPRECISION 3DP`n"
    foreach ($id in @('cylinder','transformed')) {
        $body += "=$($qids[$id])`nQ POS WRT /*`n"
    }
    $body += "`$P [Q13h-END]"
    Invoke-E3D 'write_gate.mac.template' 'Q13h.mac' @{Q13='1';TAG='Q13h';BODY=$body} | Out-Host
    $pos = Get-Q13Block (Read-LogText (Join-Path $logDir 'Q13h.log')) 'Q13h'
    $matches13 = [regex]::Matches($pos,'Position\s+([EW])\s+([-\d.]+)mm\s+([NS])\s+([-\d.]+)mm\s+([UD])\s+([-\d.]+)mm')
    $ok = $matches13.Count -eq 2 -and $pos -notmatch 'ERROR|\(\d+,\s*\d+\)'
    $j = 0
    foreach ($id in @('cylinder','transformed')) {
        $frame = Get-Q13Frame $qids[$id]
        $frame | ConvertTo-Json -Depth 16 | Set-Content (Join-Path $logDir "Q13h-$id-world-frame.json")
        if ($ok) { for ($axis=0; $axis -lt 3; $axis++) {
            $sign=if ($matches13[$j].Groups[$axis*2+1].Value -match '[WSD]') {-1} else {1}
            $actual=$sign * [double]$matches13[$j].Groups[$axis*2+2].Value
            if ([Math]::Abs($actual - $frame.frame.origin_mm[$axis]) -gt 0.1) { $ok=$false }
        } }
        $j++
    }
    # Include CYLI independently, since ordinary BOX queries deliberately filter noun.
    $body = "`$P [Q13h-search-BEGIN]`nVAR !q13 EVALUATE (REF) FOR ALL CYLI WITHIN VOLUME =24384/24777 100000`n`$P [Q13h-search-RESULT-BEGIN]`nQ VAR !q13`n`$P [Q13h-search-RESULT-END]`n`$P [Q13h-search-END]"
    Invoke-E3D 'write_gate.mac.template' 'Q13h-search.mac' @{Q13='1';TAG='Q13h-search';BODY=$body} | Out-Host
    $cyli = Test-Q13Search (Read-LogText (Join-Path $logDir 'Q13h-search.log')) 'Q13h-search' @($qids.cylinder)
    $box = Search-Q13 'Q13h-box' '24384/24777' @($qids.transformed) @() 100000
    $qr.Q13h = $ok -and $cyli -and $box
} catch { Write-Host "Q13 stopped: $($_.Exception.Message)" }
foreach ($line in Get-Q13Verdicts $qr) { Write-Output $line; if ($line.StartsWith('FAIL')) { $script:failures++ } }
$qids | ConvertTo-Json | Set-Content (Join-Path $logDir 'q13-refnos.json')
Write-Output "TIME q13_seconds=$($qclock.Elapsed.TotalSeconds)"
