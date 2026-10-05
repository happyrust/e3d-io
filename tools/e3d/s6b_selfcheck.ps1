# Offline self-check of the S6b probe's scorer (task 13.1): no E3D, no license, no database.
# Run by tests/write_stage10_s6b_gate.rs; prints "PASS S6B offline checks=<n>".
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 's6b_checks.ps1')
$count = 0
function AssertS6b([bool]$Condition, [string]$Name) { if (-not $Condition) { throw "FAIL $Name" }; $script:count++ }
function Throws([scriptblock]$Block) { try { & $Block | Out-Null; return $false } catch { return $true } }

# Syntax of every S6b script.
foreach ($file in @('s6b_probe.ps1', 's6b_checks.ps1', 's6b_selfcheck.ps1')) {
    $tokens = $null; $errors = $null
    $null = [Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot $file), [ref]$tokens, [ref]$errors)
    AssertS6b ($errors.Count -eq 0) "syntax $file : $($errors | Out-String)"
}

# The instruction files: three ZONEs; which of them each case deletes.
$ins = New-S6bInsertOps 'Gap' '/1RX03-PIPEBJ'
AssertS6b ($ins.ops.Count -eq 3 -and @($ins.ops | Where-Object { $_.op -eq 'insert' -and $_.noun -eq 'ZONE' -and $_.owner -eq '/1RX03-PIPEBJ' }).Count -eq 3) 'insert ops: three ZONEs under the owner'
AssertS6b ((@($ins.ops | ForEach-Object { $_.name }) -join ',') -eq '/E3DIO-S6B-G1,/E3DIO-S6B-G2,/E3DIO-S6B-G3') 'insert ops: names carry the case'
$json = ($ins | ConvertTo-Json -Depth 5) | ConvertFrom-Json
AssertS6b ($json.ops.Count -eq 3 -and $json.comment -eq 'e3d-io S6b probe Gap insert') 'insert ops survive JSON'
$issued = @('24384/26506', '24384/26507', '24384/26508')
AssertS6b (@(Get-S6bDoomed 'Live' $issued).Count -eq 0) 'Live deletes nothing'
AssertS6b ((@(Get-S6bDoomed 'Deleted' $issued) -join ',') -eq ($issued -join ',')) 'Deleted deletes all three'
AssertS6b ((@(Get-S6bDoomed 'Gap' @('24384/26508', '24384/26506', '24384/26507')) -join ',') -eq '24384/26506,24384/26507') 'Gap deletes the two lowest, keeps the highest'
AssertS6b (Throws { Get-S6bDoomed 'Other' $issued }) 'unknown case refused'
$del = New-S6bDeleteOps 'Gap' @('24384/26506', '24384/26507')
AssertS6b ((@($del.ops | ForEach-Object { "$($_.op) $($_.refno)" }) -join ',') -eq 'delete =24384/26506,delete =24384/26507') 'delete ops by refno'

# Pages: dynamic size, exact entry count, truncation, the transient third word.
function Page([uint32[]]$Words, [int]$Size) {
    $b = New-Object byte[] $Size
    for ($i = 0; $i -lt $Words.Length; $i++) { $w = $Words[$i]; $o = 4 * $i; $b[$o] = [byte](($w -shr 24) -band 255); $b[$o + 1] = [byte](($w -shr 16) -band 255); $b[$o + 2] = [byte](($w -shr 8) -band 255); $b[$o + 3] = [byte]($w -band 255) }
    , $b
}
$src = [uint32[]]@(1, [uint32]::MaxValue, 8, 24384, 26506, 0, 32576, 98, 0)
foreach ($size in @(2048, 4096, 1024)) {
    $words = Get-S6bPageWords (Page $src $size) $size
    AssertS6b ($words.Length -eq $size / 4) "page of $size bytes read whole"
    $t = ConvertTo-S6bBucketTable $words
    AssertS6b ($t.valid -and $t.count -eq 2 -and $t.entries.Count -eq 2) "word 2 = 8 is exactly two entries ($size)"
}
AssertS6b (Throws { Get-S6bPageWords (Page $src 2048)[0..2046] 2048 }) 'short read refused'
AssertS6b (Throws { Get-S6bPageWords (Page $src 2048) 4096 }) 'page smaller than the header page size refused'
AssertS6b (Throws { Get-S6bPageWords (Page $src 2048) 2047 }) 'page size not a multiple of 4 refused'
$claim = ConvertTo-S6bBucketTable (Get-S6bPageWords (Page ([uint32[]]@(1, [uint32]::MaxValue, 8, 24384, 26506, 4, 32576, 98, 0)) 2048) 2048)
AssertS6b ($claim.valid -and $claim.entries[0][2] -eq 4 -and $claim.entries[1][2] -eq 0) 'transient third word kept'
AssertS6b ((ConvertTo-S6bBucketTable ([uint32[]]@(1, [uint32]::MaxValue, 2))).count -eq 0) 'no entries'
AssertS6b (-not (ConvertTo-S6bBucketTable ([uint32[]]@(0, [uint32]::MaxValue, 2))).valid) 'page type 0 refused'
AssertS6b (-not (ConvertTo-S6bBucketTable ([uint32[]]@(1, 0, 2))).valid) 'missing marker refused'
foreach ($bad in @(0, 1, 3, 7, 9)) { AssertS6b (-not (ConvertTo-S6bBucketTable ([uint32[]]@(1, [uint32]::MaxValue, $bad, 0, 0, 0))).valid) "word 2 = $bad refused" }
AssertS6b (-not (ConvertTo-S6bBucketTable ([uint32[]]@(1, [uint32]::MaxValue, 11, 24384, 26506, 0, 32576, 98, 0))).valid) 'entries beyond the page refused'
$diff = Compare-S6bWords $src ([uint32[]]@(1, [uint32]::MaxValue, 8, 24384, 26510, 4, 32576, 98, 0))
AssertS6b ($diff.Count -eq 2 -and $diff[0].word -eq 4 -and $diff[0].new -eq 26510 -and $diff[1].word -eq 5 -and $diff[1].new -eq 4) 'word differences'

# The log: strict per-run markers, echoes, error numbers, duplicate and missing answers.
$id = 'ab12cd34'
$good = @"
[S6B-$id-BEGIN]
User: SYSTEM
ZONE /1RX03-MSUP
EQUIPMENT /E3DIO-S6B-E3D-L
[S6B-$id-NEW-BEGIN]
Ref =24384/26509
[S6B-$id-NEW-END]
[S6B-$id-SAVE-BEGIN]
Db MASTER/DESIN, number 8000

Last saved session number 648
Session 648:
 Date: 11:56  5 Oct 2026 User: dpc Description: e3d-io S6b probe Live
[S6B-$id-SAVE-END]
[S6B-$id-END]
"@
$r = Read-S6bLog $good $id
AssertS6b ($r.problems.Count -eq 0 -and $r.refno -eq '24384/26509' -and $r.saved_sesno -eq 648 -and $r.user -eq 'SYSTEM') 'valid log'
AssertS6b ((Read-S6bLog ([char]0xFEFF + $good) $id).refno -eq '24384/26509') 'UTF-8 BOM'
AssertS6b ((Read-S6bLog $good 'ffffffff').problems.Count -gt 0) 'another run id does not match'
AssertS6b ((Read-S6bLog '' $id).problems -contains 'no log') 'missing log'
$bad = [ordered]@{
    'missing run END' = $good.Replace("[S6B-$id-END]", '')
    'duplicate run block' = $good + "`n" + $good
    'missing NEW block' = $good.Replace("[S6B-$id-NEW-BEGIN]", '')
    'duplicate NEW END' = $good.Replace("[S6B-$id-NEW-END]", "[S6B-$id-NEW-END]`n[S6B-$id-NEW-END]")
    'missing Q REF answer' = $good.Replace('Ref =24384/26509', '')
    'echoed Q REF only' = $good.Replace('Ref =24384/26509', 'Q REF =24384/26509')
    'two Q REF answers' = $good.Replace('Ref =24384/26509', "Ref =24384/26509`nRef =24384/26510")
    'error number in NEW' = $good.Replace('Ref =24384/26509', "(41,8) Element not found`nRef =24384/26509")
    'NEW error marker' = $good.Replace("[S6B-$id-NEW-BEGIN]", "[S6B-$id-NEW-ERROR] (2,1) no`n[S6B-$id-NEW-BEGIN]")
    'missing SAVE block' = $good.Replace("[S6B-$id-SAVE-END]", '')
    'SAVEWORK failed' = $good.Replace("[S6B-$id-SAVE-BEGIN]", "[S6B-$id-SAVE-ERROR] (61,12) fail`n[S6B-$id-SAVE-BEGIN]")
    'missing Q SESSION answer' = $good.Replace('Last saved session number 648', '')
    'error in SAVE block' = $good.Replace('Last saved session number 648', "Last saved session number 648`nERROR")
    'error number outside the answer blocks' = $good.Replace('ZONE /1RX03-MSUP', "(2,109) Element not found`nZONE /1RX03-MSUP")
    'owner navigation failed' = $good.Replace('ZONE /1RX03-MSUP', "[S6B-$id-OWNER-ERROR] no owner")
    'SAVE before NEW' = @("[S6B-$id-BEGIN]", "[S6B-$id-SAVE-BEGIN]", 'Last saved session number 648', "[S6B-$id-SAVE-END]", "[S6B-$id-NEW-BEGIN]", 'Ref =24384/26509', "[S6B-$id-NEW-END]", "[S6B-$id-END]") -join "`n"
}
foreach ($k in $bad.Keys) { AssertS6b ((Read-S6bLog $bad[$k] $id).problems.Count -gt 0) "log refused: $k" }

# Source protection: only paths inside the resolved scratch root.
AssertS6b (Test-S6bUnderRoot 'D:\e3dio-s6b-probe\runs\x\after-new.a.db' 'D:\e3dio-s6b-probe') 'inside scratch'
AssertS6b (-not (Test-S6bUnderRoot 'D:\e3dio-s6b-probe\..\AVEVA\ams8000_0001' 'D:\e3dio-s6b-probe')) 'no .. escape'
AssertS6b (-not (Test-S6bUnderRoot 'D:\e3dio-s6b-probe-other\x' 'D:\e3dio-s6b-probe')) 'sibling prefix is outside'
AssertS6b (-not (Test-S6bUnderRoot 'D:\e3dio-s6b-probe' 'D:\e3dio-s6b-probe')) 'the root itself is not a target'
AssertS6b (-not (Test-S6bUnderRoot 'D:\x' 'D:\')) 'a drive root is never a scratch root'

# Verdicts: validity apart from the observation, and the four counterexamples.
function Table([int64[]]$Counters, [int64[]]$Third) {
    $w = [System.Collections.Generic.List[uint32]]@(1, [uint32]::MaxValue, [uint32](2 + 3 * $Counters.Count))
    $ext = @(24384, 32576)
    for ($i = 0; $i -lt $Counters.Count; $i++) { $w.Add([uint32]$ext[$i]); $w.Add([uint32]$Counters[$i]); $w.Add([uint32]$Third[$i]) }
    ConvertTo-S6bBucketTable ([uint32[]]$w.ToArray())
}
function Case([string]$Refno, [string[]]$Live, [string[]]$Deleted) {
    $log = Read-S6bLog ($good.Replace('24384/26509', $Refno)) $id
    [pscustomobject]@{
        issued = $issued; live = $Live; deleted = $Deleted; extent = 24384
        newest_is_e3dio = $true; counter_before = 26506
        tables = [ordered]@{ before_e3d = (Table @(26506, 98) @(0, 0)); after_login = (Table @(26506, 98) @(0, 0)); before_new = (Table @(26506, 98) @(0, 0)); after_new = (Table @(26506, 98) @(4, 0)); after_save = (Table @(26510, 98) @(4, 0)) }
        newest = @{ before_e3d = 647; after_login = 647; before_new = 647; after_new = 647; after_save = 648 }
        windows = @{ after_login = $true; before_new = $true; after_new = $true; after_save = $true }
        log = $log; driver_ok = $true
    }
}
$fresh = Get-S6bVerdict (Case '24384/26509' $issued @())
AssertS6b ($fresh.valid -and $fresh.observation -eq 'fresh' -and -not $fresh.live_duplicate -and -not $fresh.historical_reuse) 'Live: a fresh number'
$reuse = Get-S6bVerdict (Case '24384/26506' @() $issued)
AssertS6b ($reuse.valid -and $reuse.verdict -eq 'observed' -and $reuse.historical_reuse -and $reuse.observation -eq 'historical_reuse') 'historical reuse is a valid observation, not a probe failure'
$dup = Get-S6bVerdict (Case '24384/26507' $issued @())
AssertS6b ($dup.valid -and $dup.live_duplicate -and -not $dup.historical_reuse) 'live duplicate is reported apart from reuse'
$c = Case '24384/26509' $issued @(); $c.tables['before_new'] = (Table @(26509, 98) @(0, 0))
$v = Get-S6bVerdict $c
AssertS6b (-not $v.valid -and $v.rebuilt_before_new -and $v.observation -eq 'none') 'rebuild before NEW: no conclusion'
$c = Case '24384/26509' $issued @(); $c.tables['after_login'] = (Table @(26506, 98) @(4, 0))
AssertS6b ((Get-S6bVerdict $c).valid) 'a claim alone before NEW is not a rebuild'
$c = Case '24384/26509' $issued @(); $c.newest['before_new'] = 648
$v = Get-S6bVerdict $c
AssertS6b (-not $v.valid -and $v.rebuilt_before_new) 'session written before NEW: no conclusion'
$c = Case '24384/26509' $issued @(); $c.windows['before_new'] = $false
AssertS6b (-not (Get-S6bVerdict $c).valid) 'unstable snapshot (timing unclear): no conclusion'
$c = Case '24384/26509' $issued @(); $c.tables.Remove('after_login')
AssertS6b (-not (Get-S6bVerdict $c).valid) 'missing snapshot: no conclusion'
$c = Case '24384/26509' $issued @(); $c.driver_ok = $false
AssertS6b (-not (Get-S6bVerdict $c).valid) 'timeout or driver failure: no conclusion'
$c = Case '24384/26509' $issued @(); $c.newest_is_e3dio = $false
AssertS6b (-not (Get-S6bVerdict $c).valid) 'precondition: newest session not e3d-io'
$c = Case '24384/26509' $issued @(); $c.counter_before = 26507
AssertS6b ((Get-S6bVerdict $c).valid) 'counter below the highest issued word1 passes'
$c = Case '24384/26509' $issued @(); $c.counter_before = 26508
AssertS6b (-not (Get-S6bVerdict $c).valid) 'precondition: counter not below the highest issued word1'
$c = Case '24384/26509' $issued @(); $c.tables['after_new'] = [pscustomobject]@{ valid = $false; problems = @('short page'); count = $null; entries = @() }
AssertS6b (-not (Get-S6bVerdict $c).valid) 'invalid page refuses the case'
$c = Case '16192/5' $issued @()
AssertS6b (-not (Get-S6bVerdict $c).valid) 'refno outside the extent refused'

# The branch.
AssertS6b ((Get-S6bBranch @($fresh, $fresh) 'in_place') -eq 'no_collision') 'branch: no number issued again'
AssertS6b ((Get-S6bBranch @($fresh, $reuse) 'in_place') -eq 'collision_in_place') 'branch: reuse, page 1 in place'
AssertS6b ((Get-S6bBranch @($dup) 'copy_on_write') -eq 'collision_copy_on_write') 'branch: duplicate, copy on write'
AssertS6b ((Get-S6bBranch @($reuse) $null) -eq 'insufficient') 'branch: persistence unknown'
AssertS6b ((Get-S6bBranch @($reuse, (Get-S6bVerdict $c)) 'in_place') -eq 'insufficient') 'branch: one invalid case'
AssertS6b ((Get-S6bBranch @() 'in_place') -eq 'insufficient') 'branch: no case'

Write-Output "PASS S6B offline checks=$count"
