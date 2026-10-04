function Test-Q14Ref([string]$Text, [string]$Expected) {
    try {
        $block = Get-Q13Block $Text 'Q14'
        if ($block -match 'ERROR|\(\d+,\s*\d+\)') { return $false }
        $answer = Get-Q13Block $block 'Q14-RESULT'
        $refs = [regex]::Matches($answer, '(?m)^\s*Ref\s*=(\d+/\d+)\s*$')
        return $refs.Count -eq 1 -and $refs[0].Groups[1].Value -eq $Expected
    } catch { return $false }
}
