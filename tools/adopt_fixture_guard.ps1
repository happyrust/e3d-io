# Swap `#[ignore]` for a runtime fixture check in the tests that only need the
# installed E3D 3.1 files. Slow whole-file scans keep `#[ignore]`.
#
# Run once from the crate root:  pwsh tools\adopt_fixture_guard.ps1

$targets = @{
    'bran_default_values_real.rs'   = @('DEFAULT_DB', 'DEFAULT_DESVIR', 'DEFAULT_ATTLIB')
    'bran_uda_tail_real.rs'         = @('DEFAULT_DB', 'DICTIONARY_DB')
    'complete_attlist_real.rs'      = @('DEFAULT_DB', 'DEFAULT_DESVIR', 'DEFAULT_ATTLIB')
    'dbmdat_descriptor_real.rs'     = @('DB', 'DESVIR', 'ATTLIB')
    'descriptor_extraction_real.rs' = @('DEFAULT_DB', 'DEFAULT_DESVIR', 'DEFAULT_ATTLIB')
    'dictionary_uda_real.rs'        = @('DICTIONARY_DB', 'DICVIR', 'ATTLIB', 'DESIGN_DB', 'DESVIR')
    'external_uda_real.rs'          = @('ACP7323', 'AMS5100')
    'header_field_census.rs'        = @('DEFAULT_PROJECT')
    'index_entry_staleness.rs'      = @('AMS')
    'record_scan_fallback_real.rs'  = @('DB', 'DESVIR', 'ATTLIB')
    'scratch_uda_def_copies.rs'     = @('AMS')
    'element_version_census.rs'     = @('AMS')
}

foreach ($name in $targets.Keys) {
    $path = Join-Path 'tests' $name
    if (-not (Test-Path $path)) { Write-Host "absent: $path"; continue }
    $text = Get-Content $path -Raw
    if ($text -notmatch '#\[ignore') { Write-Host "already converted: $name"; continue }

    $list = ($targets[$name] | ForEach-Object { $_ }) -join ', '
    $decl = "mod common;`r`n`r`nconst FIXTURES: &[&str] = &[$list];`r`n"

    # Drop the ignore attribute and open each test with the guard.
    $text = [regex]::Replace(
        $text,
        '(?m)^#\[ignore = "[^"]*"\]\r?\n(fn (\w+)\(\) \{)\r?\n',
        "`$1`r`n    if common::missing(FIXTURES) {`r`n        return;`r`n    }`r`n")

    # Place the module and the fixture list after the last `const` declaration.
    $lines = $text -split "`r?`n"
    $last = -1
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match '^const ') { $last = $i }
    }
    if ($last -lt 0) { Write-Host "no const block: $name"; continue }
    $out = @()
    $out += $lines[0..$last]
    $out += ''
    $out += ($decl -split "`r?`n" | Where-Object { $_ -ne '' -or $true })
    if ($last + 1 -lt $lines.Count) { $out += $lines[($last + 1)..($lines.Count - 1)] }
    Set-Content $path (($out -join "`r`n").TrimEnd() + "`r`n") -NoNewline
    Write-Host "converted: $name"
}
