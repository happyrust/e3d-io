# Q15 (requirement 13, S6b): a number e3d-io issued and then deleted is not handed out
# again by an E3D NEW. alternate_write_gate.ps1 runs the Deleted case of s6b_probe.ps1,
# sized so that the index-maximum allocation of c61b097 also reaches the counter: in the
# extent of the gate's ZONE, whose index maximum M is below the Bucket_Counter C, e3d-io
# inserts K = C - M elements in one session and deletes them all in the next; then E3D
# NEWs one element there and SAVEWORKs.
#
# PASS: E3D's word1 is page 1's counter right after e3d-io's insert commit, and none of
# the numbers e3d-io issued. An e3d-io that leaves page 1 alone issues M+1..C, the counter
# stays at C, and E3D is handed C again, a number e3d-io issued and deleted: FAIL.

# $Run: extent, index_max (M), counter_before (C), issued ("word0/word1", from the insert
# report), floor_after_insert (page 1's counter after that commit), e3d_saved, e3d_ref
# (E3D's Q REF, "word0/word1"). The verdict is indeterminate when the case does not hold:
# M not below C, a count other than K issued or issued outside the extent, or no E3D
# element in the extent.
function Get-Q15Verdict($Run) {
    $reasons = New-Object System.Collections.Generic.List[string]
    $k = [int64]$Run.counter_before - [int64]$Run.index_max
    $issued = @($Run.issued | ForEach-Object { ([string]$_).TrimStart('=') })
    if ($k -le 0) {
        $reasons.Add("the index maximum $($Run.index_max) is not below the counter $($Run.counter_before)")
    } elseif ($issued.Count -ne $k) {
        $reasons.Add("e3d-io issued $($issued.Count) refnos, wanted K = $k")
    }
    $outside = @($issued | Where-Object { ($_ -split '/')[0] -ne [string]$Run.extent })
    if ($outside.Count) { $reasons.Add("e3d-io issued outside extent $($Run.extent): $($outside -join ', ')") }
    $e3dWord1 = $null
    if (-not $Run.e3d_saved -or [string]$Run.e3d_ref -notmatch '^(\d+)/(\d+)$') {
        $reasons.Add('E3D did not NEW and SAVEWORK an element')
    } elseif ($Matches[1] -ne [string]$Run.extent) {
        $reasons.Add("E3D took $($Run.e3d_ref), outside extent $($Run.extent)")
    } else {
        $e3dWord1 = [int64]$Matches[2]
    }
    if ($reasons.Count) { return [pscustomobject]@{ verdict = 'indeterminate'; reasons = @($reasons) } }

    $issuedWord1 = @($issued | ForEach-Object { [int64](($_ -split '/')[1]) })
    if ($issuedWord1 -contains $e3dWord1) {
        $reasons.Add("E3D was handed $($Run.e3d_ref), a number e3d-io issued and deleted")
    }
    if ($e3dWord1 -ne [int64]$Run.floor_after_insert) {
        $reasons.Add("E3D took word1 $e3dWord1; page 1's counter after e3d-io's insert was $($Run.floor_after_insert)")
    }
    if ($reasons.Count) { return [pscustomobject]@{ verdict = 'fail'; reasons = @($reasons) } }
    [pscustomobject]@{ verdict = 'pass'; reasons = @() }
}
