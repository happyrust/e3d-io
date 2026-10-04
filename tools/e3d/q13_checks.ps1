# Pure checks shared by the real driver and the offline self-check.
function Get-Q13Block([string]$Text, [string]$Tag) {
    $Text = $Text.TrimStart([char]0xFEFF)
    $begin = '(?m)^\s*\[' + [regex]::Escape($Tag) + '-BEGIN\]\s*$'
    $end = '(?m)^\s*\[' + [regex]::Escape($Tag) + '-END\]\s*$'
    $b = [regex]::Matches($Text, $begin); $e = [regex]::Matches($Text, $end)
    if ($b.Count -ne 1 -or $e.Count -ne 1 -or $e[0].Index -le $b[0].Index) { throw "$Tag missing or duplicate markers" }
    $Text.Substring($b[0].Index + $b[0].Length, $e[0].Index - $b[0].Index - $b[0].Length)
}
function Test-Q13Search([string]$Text, [string]$Tag, [string[]]$Present = @(), [string[]]$Absent = @(), [switch]$VolumeError) {
    try { $block = Get-Q13Block $Text $Tag } catch { return $false }
    if ($VolumeError) {
        $errors13 = [regex]::Matches($block, '\(\d+,\s*\d+\)')
        return ($errors13.Count -eq 1 -and $errors13[0].Value -match '^\(2,\s*536\)$')
    }
    if ($block -match '\(\d+,\s*\d+\)|ERROR') { return $false }
    # Only the bracketed Q VAR result. Never match the echoed search command.
    try { $answer = Get-Q13Block $block "$Tag-RESULT" } catch { return $false }
    if ($answer -notmatch '(?m)^\s*<ARRAY>(?: - Unset and Empty)?\s*$') { return $false }
    $refs = @([regex]::Matches($answer, "(?m)^\s*\[\d+\]\s+<STRING>\s+'=(\d+/\d+)'\s*$") | ForEach-Object { $_.Groups[1].Value })
    if (-not $refs.Count -and $answer -notmatch '<ARRAY> - Unset and Empty') { return $false }
    foreach ($r in $Present) { if ($refs -notcontains $r) { return $false } }
    foreach ($r in $Absent) { if ($refs -contains $r) { return $false } }
    return $true
}
function Test-Q13Intersection($A, $B, [double]$Clearance) {
    for ($i=0; $i -lt 3; $i++) {
        if ($A.max_mm[$i] + $Clearance -lt $B.min_mm[$i] -or $B.max_mm[$i] -lt $A.min_mm[$i] - $Clearance) { return $false }
    }
    return $true
}
function Get-Q13Verdicts([hashtable]$Results) {
    foreach ($q in @('Q13','Q13a','Q13b','Q13c','Q13d','Q13e','Q13f','Q13g','Q13h')) {
        $pass = [bool]$Results[$q] -and ($q -eq 'Q13' -or [bool]$Results['Q13'])
        if ($pass) { "PASS $q" } else { "FAIL $q" }
    }
}
