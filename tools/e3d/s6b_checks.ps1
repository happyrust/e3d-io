# Pure checks for the S6b probe (task 13.1), shared by s6b_probe.ps1 and the offline
# s6b_selfcheck.ps1. Nothing here starts E3D, reads a database file or writes to disk:
# every function takes bytes or text and returns a value, so the self-check can feed it
# truncated pages, echoed commands and duplicated markers without a license.

# The e3d-io batches of one case: three ZONEs inserted in one session; Deleted then deletes all
# three in the next session, Gap the first two (the highest stays above them), Live none.
function New-S6bInsertOps([string]$Case, [string]$Owner) {
    $tag = $Case.Substring(0, 1).ToUpperInvariant()
    [ordered]@{
        comment = "e3d-io S6b probe $Case insert"
        ops = @(1..3 | ForEach-Object { [ordered]@{ op = 'insert'; id = "z$_"; owner = $Owner; noun = 'ZONE'; name = "/E3DIO-S6B-$tag$_" } })
    }
}
# Callers wrap the result in @(): Live yields nothing.
function Get-S6bDoomed([string]$Case, [string[]]$Issued) {
    switch ($Case) {
        'Live' { return }
        'Deleted' { return $Issued }
        'Gap' { return $Issued | Sort-Object { [int64](($_ -split '/')[1]) } | Select-Object -First 2 }
        default { throw "unknown case $Case" }
    }
}
function New-S6bDeleteOps([string]$Case, [string[]]$Doomed) {
    [ordered]@{ comment = "e3d-io S6b probe $Case delete"; ops = @($Doomed | ForEach-Object { [ordered]@{ op = 'delete'; refno = "=$_" } }) }
}

# Is $Path inside $Root (both resolved, no '..' escapes)? Cleanup and snapshot writes are limited
# to the resolved scratch directory; the source database is never a target.
function Test-S6bUnderRoot([string]$Path, [string]$Root) {
    if (-not $Path -or -not $Root) { return $false }
    $full = [IO.Path]::GetFullPath($Path).TrimEnd('\', '/')
    $base = [IO.Path]::GetFullPath($Root).TrimEnd('\', '/')
    if ($base.Length -lt 4) { return $false }
    $full.StartsWith($base + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)
}

# The words of one page, big-endian. $Bytes must hold the whole page: a short read is
# refused, never padded, so a half-copied snapshot cannot pass as a bucket table.
function Get-S6bPageWords([byte[]]$Bytes, [int]$PageSize) {
    if ($PageSize -le 0 -or $PageSize % 4) { throw "page size $PageSize is not a positive multiple of 4" }
    if ($null -eq $Bytes -or $Bytes.Length -ne $PageSize) {
        throw "short page: $(if ($null -eq $Bytes) { 0 } else { $Bytes.Length }) of $PageSize bytes"
    }
    $words = New-Object 'uint32[]' ($PageSize / 4)
    for ($i = 0; $i -lt $words.Length; $i++) {
        $o = 4 * $i
        $words[$i] = [uint32](([uint32]$Bytes[$o] -shl 24) -bor ([uint32]$Bytes[$o + 1] -shl 16) -bor ([uint32]$Bytes[$o + 2] -shl 8) -bor [uint32]$Bytes[$o + 3])
    }
    , $words
}

# Physical page 1 read as the bucket table, the way src/meta/bucket.rs parses it: type 1,
# marker 0xFFFFFFFF, word 2 = 2 + 3n, n entries inside the page. Every entry keeps all three
# words, including a non-zero third word (a claim held during a session is not dropped).
function ConvertTo-S6bBucketTable([uint32[]]$Words) {
    $problems = New-Object System.Collections.Generic.List[string]
    $entries = @()
    if ($null -eq $Words -or $Words.Length -lt 3) {
        $problems.Add('page holds fewer than three words')
        return [pscustomobject]@{ valid = $false; problems = @($problems); count = $null; entries = @() }
    }
    if ($Words[0] -ne 1) { $problems.Add("word 0 is $($Words[0]), not the bucket page type 1") }
    if ($Words[1] -ne [uint32]::MaxValue) { $problems.Add(('word 1 is 0x{0:X8}, not the marker 0xFFFFFFFF' -f $Words[1])) }
    $n = [int64]$Words[2]
    $count = $null
    if ($n -lt 2 -or ($n - 2) % 3) {
        $problems.Add("word 2 is $n, not 2 + 3n")
    } else {
        $count = [int](($n - 2) / 3)
        if (3 + 3 * $count -gt $Words.Length) {
            $problems.Add("word 2 claims $count entries; the page holds at most $([int][math]::Floor(($Words.Length - 3) / 3))")
        } else {
            for ($i = 0; $i -lt $count; $i++) {
                $b = 3 + 3 * $i
                $entries += , @([int64]$Words[$b], [int64]$Words[$b + 1], [int64]$Words[$b + 2])
            }
        }
    }
    [pscustomobject]@{ valid = ($problems.Count -eq 0); problems = @($problems); count = $count; entries = $entries }
}

# Word-by-word differences between two pages' words: index, old value, new value.
function Compare-S6bWords([uint32[]]$Old, [uint32[]]$New) {
    $diff = @()
    $len = [math]::Max($Old.Length, $New.Length)
    for ($i = 0; $i -lt $len; $i++) {
        $a = if ($i -lt $Old.Length) { [int64]$Old[$i] } else { $null }
        $b = if ($i -lt $New.Length) { [int64]$New[$i] } else { $null }
        if ($a -ne $b) { $diff += [pscustomobject]@{ word = $i; old = $a; new = $b } }
    }
    , $diff
}

# The text between one [<Tag>-BEGIN] and one [<Tag>-END] line. Zero or duplicate markers, or an
# END before the BEGIN, throw: a second block must never be read as the first one's answer.
function Get-S6bBlock([string]$Text, [string]$Tag) {
    $Text = $Text.TrimStart([char]0xFEFF)
    $begin = '(?m)^\s*\[' + [regex]::Escape($Tag) + '-BEGIN\]\s*$'
    $end = '(?m)^\s*\[' + [regex]::Escape($Tag) + '-END\]\s*$'
    $b = [regex]::Matches($Text, $begin); $e = [regex]::Matches($Text, $end)
    if ($b.Count -ne 1 -or $e.Count -ne 1 -or $e[0].Index -le $b[0].Index) {
        throw "$Tag markers: $($b.Count) BEGIN, $($e.Count) END$(if ($b.Count -eq 1 -and $e.Count -eq 1) { ', END before BEGIN' })"
    }
    $Text.Substring($b[0].Index + $b[0].Length, $e[0].Index - $b[0].Index - $b[0].Length)
}

# E3D's answers inside one run's marker blocks. The whole run sits in [S6B-<id>-BEGIN/END];
# Q REF's answer in [S6B-<id>-NEW-BEGIN/END] and Q SESSION's in [S6B-<id>-SAVE-BEGIN/END],
# NEW before SAVE. A line counts only as E3D's answer ("Ref =a/b", "Last saved session number
# N"), never as an echoed command; any (n,m) error number or ERROR inside a block, a handled
# NEW / SAVEWORK error marker, a missing or doubled answer refuses the run.
function Read-S6bLog([string]$Text, [string]$RunId) {
    $problems = New-Object System.Collections.Generic.List[string]
    $result = [ordered]@{ refno = $null; saved_sesno = $null; user = $null; problems = $null }
    if ([string]::IsNullOrEmpty($Text)) { $problems.Add('no log'); $result.problems = @($problems); return [pscustomobject]$result }
    $tag = "S6B-$RunId"
    try { $run = Get-S6bBlock $Text $tag } catch { $problems.Add($_.Exception.Message); $result.problems = @($problems); return [pscustomobject]$result }
    foreach ($err in @('OWNER-ERROR', 'NEW-ERROR', 'SAVE-ERROR')) {
        if ($run -match ('(?m)^\s*\[' + [regex]::Escape("$tag-$err") + '\]')) { $problems.Add("E3D reported $err") }
    }
    if ($run -match '\(\d+\s*,\s*\d+\)|ERROR') { $problems.Add('run block holds an error') }
    $user = [regex]::Matches($run, '(?m)^\s*User:\s*(\S.*?)\s*$')
    if ($user.Count -eq 1) { $result.user = $user[0].Groups[1].Value }
    $newBlock = $null; $saveBlock = $null
    try { $newBlock = Get-S6bBlock $run "$tag-NEW" } catch { $problems.Add($_.Exception.Message) }
    try { $saveBlock = Get-S6bBlock $run "$tag-SAVE" } catch { $problems.Add($_.Exception.Message) }
    if ($null -ne $newBlock -and $null -ne $saveBlock -and $run.IndexOf("[$tag-NEW-BEGIN]") -gt $run.IndexOf("[$tag-SAVE-BEGIN]")) {
        $problems.Add('SAVE block before NEW block')
    }
    foreach ($pair in @(@('NEW', $newBlock), @('SAVE', $saveBlock))) {
        if ($null -ne $pair[1] -and $pair[1] -match '\(\d+\s*,\s*\d+\)|ERROR') { $problems.Add("$($pair[0]) block holds an error") }
    }
    if ($null -ne $newBlock) {
        $refs = [regex]::Matches($newBlock, '(?m)^\s*Ref\s*=\s*(\d+/\d+)\s*$')
        if ($refs.Count -eq 1) { $result.refno = $refs[0].Groups[1].Value } else { $problems.Add("NEW block holds $($refs.Count) Q REF answers") }
    }
    if ($null -ne $saveBlock) {
        $saved = [regex]::Matches($saveBlock, '(?m)^\s*Last saved session number\s+(\d+)\s*$')
        if ($saved.Count -eq 1) { $result.saved_sesno = [int]$saved[0].Groups[1].Value } else { $problems.Add("SAVE block holds $($saved.Count) Q SESSION answers") }
    }
    $result.problems = @($problems)
    [pscustomobject]$result
}

# Does $Next follow from $Previous with nothing but the third word of existing entries changed?
# That is a claim (or a release), not a counter move or a rebuild.
function Test-S6bClaimOnly($Previous, $Next) {
    if (-not $Previous.valid -or -not $Next.valid -or $Previous.count -ne $Next.count) { return $false }
    for ($i = 0; $i -lt $Previous.count; $i++) {
        if ($Previous.entries[$i][0] -ne $Next.entries[$i][0] -or $Previous.entries[$i][1] -ne $Next.entries[$i][1]) { return $false }
    }
    $true
}

# The verdict of one case. Validity (did the probe observe what it set out to observe) is kept
# apart from the observation (which number E3D handed out): an observed reuse is a valid result,
# not a probe failure. $Case carries:
#   issued, live, deleted  e3d-io refnos from its success reports ('word0/word1' strings)
#   extent                 the extent's word0
#   newest_is_e3dio        the file's newest session before E3D is the last e3d-io batch's
#   counter_before         the extent's Bucket_Counter in the file before E3D
#   tables                 ordered hashtable label -> ConvertTo-S6bBucketTable result
#                          (before_e3d, after_login, before_new, after_new, after_save)
#   newest                 ordered hashtable label -> newest session number (same labels)
#   windows                hashtable label -> $true when both copies of the snapshot agreed
#   log                    Read-S6bLog result
#   driver_ok              driver exited 0 after one session that reached L3-DONE, no timeout
function Get-S6bVerdict($Case) {
    $reasons = New-Object System.Collections.Generic.List[string]
    $notes = New-Object System.Collections.Generic.List[string]
    if (-not $Case.driver_ok) { $reasons.Add('driver did not complete one E3D session') }
    foreach ($p in @($Case.log.problems)) { if ($p) { $reasons.Add("log: $p") } }
    if (-not $Case.newest_is_e3dio) { $reasons.Add('precondition: the newest session before E3D is not the e3d-io batch') }
    $issuedWord1 = @($Case.issued | ForEach-Object { [int64](($_ -split '/')[1]) })
    if (-not $issuedWord1.Count) { $reasons.Add('precondition: e3d-io issued no refno') }
    elseif ($null -eq $Case.counter_before -or [int64]$Case.counter_before -ge ($issuedWord1 | Measure-Object -Maximum).Maximum) {
        $reasons.Add("precondition: Bucket_Counter $($Case.counter_before) is not below e3d-io's highest issued word1")
    }
    foreach ($label in @($Case.tables.Keys)) {
        if (-not $Case.tables[$label].valid) { $reasons.Add("$label page 1: $(@($Case.tables[$label].problems) -join '; ')") }
    }
    foreach ($label in @($Case.windows.Keys)) { if (-not $Case.windows[$label]) { $reasons.Add("$label snapshot copies differ or are incomplete") } }
    # Requirement 13.11: nothing rebuilt the table, and no session was written, before the NEW.
    $rebuiltBeforeNew = $null
    foreach ($label in @('after_login', 'before_new')) {
        if (-not $Case.tables.Contains($label) -or -not $Case.tables.Contains('before_e3d')) { $reasons.Add("missing $label snapshot"); continue }
        $same = (Test-S6bClaimOnly $Case.tables['before_e3d'] $Case.tables[$label])
        $counterSame = $same -and (@($Case.tables['before_e3d'].entries | ForEach-Object { $_[1] }) -join ',') -eq (@($Case.tables[$label].entries | ForEach-Object { $_[1] }) -join ',')
        if (-not $counterSame) { $rebuiltBeforeNew = $true; $reasons.Add("page 1 counters changed between before_e3d and $label (rebuild before NEW)") }
        if ($Case.newest[$label] -ne $Case.newest['before_e3d']) { $rebuiltBeforeNew = $true; $reasons.Add("a session was written between before_e3d and $label") }
    }
    if ($null -eq $rebuiltBeforeNew) { $rebuiltBeforeNew = $false }
    $refno = $Case.log.refno
    if ($refno -and ($refno -split '/')[0] -ne [string]$Case.extent) { $reasons.Add("E3D's refno $refno is not in extent $($Case.extent)") }
    $valid = ($reasons.Count -eq 0)
    $observation = 'none'
    if ($valid) {
        if (@($Case.live) -contains $refno) { $observation = 'live_duplicate' }
        elseif (@($Case.deleted) -contains $refno) { $observation = 'historical_reuse' }
        else { $observation = 'fresh' }
        if ($Case.tables.Contains('after_new') -and -not (Test-S6bClaimOnly $Case.tables['before_new'] $Case.tables['after_new'])) {
            $notes.Add('page 1 counters moved during NEW')
        }
    }
    [pscustomobject]@{
        valid = $valid
        verdict = if ($valid) { 'observed' } else { 'indeterminate' }
        observation = $observation
        live_duplicate = ($valid -and $observation -eq 'live_duplicate')
        historical_reuse = ($valid -and $observation -eq 'historical_reuse')
        rebuilt_before_new = $rebuiltBeforeNew
        reasons = @($reasons)
        notes = @($notes)
    }
}

# Requirement 13's branch from the valid cases' observations and what page 1's persistence is
# known to be ('in_place', 'copy_on_write' or $null when the evidence does not settle it).
# "Issued" counts numbers e3d-io issued and deleted again (the plan's reading of 13.3 / 13.12).
function Get-S6bBranch($Verdicts, [string]$Persist) {
    $all = @($Verdicts)
    if (-not $all.Count -or @($all | Where-Object { -not $_.valid }).Count) { return 'insufficient' }
    $issuedAgain = @($all | Where-Object { $_.live_duplicate -or $_.historical_reuse }).Count -gt 0
    if (-not $issuedAgain) { return 'no_collision' }
    switch ($Persist) {
        'in_place' { return 'collision_in_place' }
        'copy_on_write' { return 'collision_copy_on_write' }
        default { return 'insufficient' }
    }
}
