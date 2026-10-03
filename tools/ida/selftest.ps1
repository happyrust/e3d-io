<#
.SYNOPSIS
  Offline checks of tools/ida/ib.ps1 (task 2.4 of .kiro/specs/write-path-phase-3):
  the paths that need no IDA.

.DESCRIPTION
  powershell -NoProfile -File tools/ida/selftest.ps1

  - 90: no place in the lookup order is a whole installation; every place is
    reported with what it lacks, IDA_BRIDGE_HOME as not set (requirement 1.2).
    Runs a copy of ib.ps1 whose two fixed places point nowhere.
  - 92: the bridge address answers nothing; the address and the commands that
    bring the target IDB online are printed, and for a missing target IDB the
    start-idalib command is not (requirement 1.4).
  - 91: each call that could save Core_IDB is refused, and nothing connects
    to the configured bridge address meanwhile (requirement 1.5).
  - 93 / 94 / forwarding, against a stand-in ida_bridge with a canned list:
    no instance, two instances, one held by another session, the holder
    itself, reset and supervisor stop resolved (requirements 1.3, 1.10). Two
    live instances on one IDB cannot be had: IDA locks an open database.
  Prints PASS or FAIL per check; exit 0 when all pass.
#>
$ErrorActionPreference = 'Continue'
$ib = Join-Path $PSScriptRoot 'ib.ps1'
$failed = 0

function Invoke-Ib([string]$Script, [string[]]$Arguments) {
    # Each line as the plain string it was, unwrapped.
    $output = (& powershell -NoProfile -File $Script @Arguments 2>&1 | ForEach-Object { "$_" }) -join "`n"
    return [pscustomobject]@{ Code = $LASTEXITCODE; Output = $output }
}

function Check([string]$Name, [bool]$Pass, [string]$Detail) {
    if ($Pass) {
        Write-Output "PASS $Name"
    } else {
        Write-Output "FAIL $Name"
        Write-Output ($Detail -split "`n" | ForEach-Object { "    $_" } | Out-String).TrimEnd()
        $script:failed++
    }
}

$scratch = Join-Path ([IO.Path]::GetTempPath()) ('ib-selftest-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $scratch | Out-Null
$savedHome = $env:IDA_BRIDGE_HOME
$savedHost = $env:IDA_BRIDGE_HOST
$savedPort = $env:IDA_BRIDGE_PORT
try {
    # --- 90 -------------------------------------------------------------------
    $copy = Join-Path $scratch 'ib.ps1'
    $nowhere = Join-Path $scratch 'nowhere'
    $text = [IO.File]::ReadAllText($ib)
    foreach ($fixed in @('C:\Users\dpc\.agents-old\tools\ida-bridge', 'C:\Users\dpc\.agents\tools\ida-bridge')) {
        $text = $text.Replace("'$fixed'", "'$nowhere\$(Split-Path -Leaf (Split-Path -Parent (Split-Path -Parent $fixed)))'")
    }
    [IO.File]::WriteAllText($copy, $text)

    Remove-Item Env:\IDA_BRIDGE_HOME -ErrorAction SilentlyContinue
    $run = Invoke-Ib $copy @('list')
    Check '90 when no place is an installation' (
        $run.Code -eq 90 -and
        $run.Output.Contains('IDA_BRIDGE_HOME: not set') -and
        ([regex]::Matches($run.Output, 'missing src\\ida_bridge, \.venv\\Scripts\\python\.exe')).Count -eq 2
    ) $run.Output

    $partial = Join-Path $scratch 'partial'
    New-Item -ItemType Directory -Path (Join-Path $partial 'src\ida_bridge') -Force | Out-Null
    $env:IDA_BRIDGE_HOME = $partial
    $run = Invoke-Ib $copy @('list')
    Check '90 names what a partial installation lacks' (
        $run.Code -eq 90 -and $run.Output.Contains('IDA_BRIDGE_HOME: missing .venv\Scripts\python.exe')
    ) $run.Output
    if ($savedHome) { $env:IDA_BRIDGE_HOME = $savedHome } else { Remove-Item Env:\IDA_BRIDGE_HOME -ErrorAction SilentlyContinue }

    # --- 92 -------------------------------------------------------------------
    $probe = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
    $probe.Start()
    $closedPort = ([System.Net.IPEndPoint]$probe.LocalEndpoint).Port
    $probe.Stop()
    $env:IDA_BRIDGE_HOST = '127.0.0.1'
    $env:IDA_BRIDGE_PORT = [string]$closedPort

    $run = Invoke-Ib $ib @('--target-idb', $ib, 'list')
    Check '92 when the bridge does not answer' (
        $run.Code -eq 92 -and
        $run.Output.Contains("not reachable at 127.0.0.1:$closedPort") -and
        $run.Output.Contains('tools/ida/ib.ps1 server start') -and
        $run.Output.Contains('tools/ida/ib.ps1 supervisor start-idalib --idb')
    ) $run.Output

    $absent = Join-Path $scratch 'absent.i64'
    $run = Invoke-Ib $ib @('--target-idb', $absent, 'list')
    Check '92 with a missing target IDB prints no start-idalib' (
        $run.Code -eq 92 -and $run.Output.Contains('does not exist') -and -not $run.Output.Contains('start-idalib')
    ) $run.Output

    # --- 91 -------------------------------------------------------------------
    $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
    $listener.Start()
    $env:IDA_BRIDGE_PORT = [string](([System.Net.IPEndPoint]$listener.LocalEndpoint).Port)
    $saver = Join-Path $scratch 'saver.py'
    [IO.File]::WriteAllText($saver, "import ida_loader`nida_loader.save_database('x', 0)`n")
    $refused = @(
        @{ Name = '--save'; Args = @('exec-idb', '--idb', 'D:\ida_scratch\plant3\core.dll.i64', '--sql', 'SELECT 1', '--save') },
        @{ Name = 'supervisor save'; Args = @('supervisor', 'save', 'idalib-1') },
        @{ Name = '--out-idb onto Core_IDB, other case and slashes'; Args = @('supervisor', 'start-idalib', '--input', 'core.dll', '--out-idb', 'd:/IDA_SCRATCH/plant3/CORE.DLL.i64') },
        @{ Name = '-c calling idb.save'; Args = @('exec', 'idalib-1', '-c', 'idb.save()') },
        @{ Name = '-f calling save_database'; Args = @('exec', 'idalib-1', '-f', $saver) },
        @{ Name = 'supervisor stop of a UI instance'; Args = @('supervisor', 'stop', 'idaui-12') },
        @{ Name = 'supervisor stop by PID'; Args = @('supervisor', 'stop', '4242') }
    )
    foreach ($case in $refused) {
        $run = Invoke-Ib $ib $case.Args
        Start-Sleep -Milliseconds 100
        $contacted = $listener.Pending()
        Check "91 refuses $($case.Name), sending nothing" (
            $run.Code -eq 91 -and $run.Output.Contains('refused') -and -not $contacted
        ) "exit $($run.Code), bridge contacted: $contacted`n$($run.Output)"
        while ($listener.Pending()) { $listener.AcceptTcpClient().Close() }
    }
    $listener.Stop()

    # --- 93, 94 and forwarding, against a stand-in ida_bridge ----------------------
    # A venv's python.exe is a launcher that finds its interpreter through the
    # pyvenv.cfg one level up, so the two files make a working stand-in whose
    # ida_bridge prints a canned list and echoes what it was asked to run.
    $fake = Join-Path $scratch 'fake'
    New-Item -ItemType Directory -Path (Join-Path $fake 'src\ida_bridge'), (Join-Path $fake '.venv\Scripts') -Force | Out-Null
    $venv = 'C:\Users\dpc\.agents-old\tools\ida-bridge\.venv'
    Copy-Item -LiteralPath (Join-Path $venv 'Scripts\python.exe') -Destination (Join-Path $fake '.venv\Scripts\python.exe')
    Copy-Item -LiteralPath (Join-Path $venv 'pyvenv.cfg') -Destination (Join-Path $fake '.venv\pyvenv.cfg')
    [IO.File]::WriteAllText((Join-Path $fake 'src\ida_bridge\__init__.py'), '')
    [IO.File]::WriteAllText((Join-Path $fake 'src\ida_bridge\__main__.py'),
        "import json, os, sys`nargs = sys.argv[1:]`nif args[:2] == ['list', '--json']:`n    print(os.environ['IB_FAKE_LIST'])`nelse:`n    print(json.dumps({'forwarded': args}))`n")
    $target = Join-Path $scratch 'Target.i64'
    [IO.File]::WriteAllText($target, 'stands in for an IDB')
    $env:IDA_BRIDGE_HOME = $fake
    $bridge = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
    $bridge.Start()
    $env:IDA_BRIDGE_PORT = [string](([System.Net.IPEndPoint]$bridge.LocalEndpoint).Port)

    function Set-Listed([object[]]$Instances) {
        $env:IB_FAKE_LIST = (@{ ok = $true; clients = @($Instances) } | ConvertTo-Json -Depth 5 -Compress)
    }
    function New-Instance([string]$Id, [string]$IdbPath, [string]$Session) {
        $instance = [ordered]@{ client_id = $Id; role = 'ida'; meta = @{ pid = [int]($Id -replace '\D', ''); idb_path = $IdbPath } }
        if ($Session) { $instance.session_id = $Session }
        return $instance
    }
    function Invoke-Against([object[]]$Instances, [string[]]$Arguments) {
        Set-Listed $Instances
        $run = Invoke-Ib $ib (@('--target-idb', $target) + $Arguments)
        while ($bridge.Pending()) { $bridge.AcceptTcpClient().Close() }
        return $run
    }

    $run = Invoke-Against @(New-Instance 'idalib-11' 'C:\elsewhere\other.i64') @('exec', '-c', '1')
    Check '93 when no instance has the target open, with the start command' (
        $run.Code -eq 93 -and $run.Output.Contains("supervisor start-idalib --idb `"$target`"")
    ) $run.Output

    $twice = @((New-Instance 'idalib-21' $target), (New-Instance 'idalib-22' $target.Replace('\', '/').ToUpperInvariant()))
    $run = Invoke-Against $twice @('exec', '-c', '1')
    Check '94 when two instances have the target open, both listed' (
        $run.Code -eq 94 -and $run.Output.Contains('instance idalib-21  pid 21  held by session none') -and
        $run.Output.Contains('instance idalib-22  pid 22  held by session none')
    ) $run.Output

    $held = @(New-Instance 'idalib-31' $target 'other')
    $run = Invoke-Against $held @('exec', '-c', '1')
    Check '94 when the one instance is held by another session' (
        $run.Code -eq 94 -and $run.Output.Contains('held by session other')
    ) $run.Output
    $run = Invoke-Against $held @('exec', '--stateful', '--session-id', 'other', '-c', '1')
    Check 'the holding session itself is forwarded, instance inserted' (
        $run.Code -eq 0 -and $run.Output.Contains('"exec", "idalib-31", "--stateful", "--session-id", "other", "-c", "1"')
    ) $run.Output

    $run = Invoke-Against @(New-Instance 'idalib-41' $target) @('reset', '--session-id', 's', '--release')
    Check 'reset is forwarded to the resolved instance' (
        $run.Code -eq 0 -and $run.Output.Contains('"reset", "idalib-41", "--session-id", "s", "--release"')
    ) $run.Output

    $run = Invoke-Against @(New-Instance 'idalib-61' $target) @('supervisor', 'stop')
    Check 'supervisor stop is forwarded to a resolved idalib instance' (
        $run.Code -eq 0 -and $run.Output.Contains('"supervisor", "stop", "idalib-61"')
    ) $run.Output
    $run = Invoke-Against @(New-Instance 'idaui-51' $target) @('supervisor', 'stop')
    Check '91 when supervisor stop resolves to a UI instance' (
        $run.Code -eq 91 -and $run.Output.Contains('idaui-51 is a UI instance')
    ) $run.Output
    $bridge.Stop()
} finally {
    if ($savedHome) { $env:IDA_BRIDGE_HOME = $savedHome } else { Remove-Item Env:\IDA_BRIDGE_HOME -ErrorAction SilentlyContinue }
    if ($savedHost) { $env:IDA_BRIDGE_HOST = $savedHost } else { Remove-Item Env:\IDA_BRIDGE_HOST -ErrorAction SilentlyContinue }
    if ($savedPort) { $env:IDA_BRIDGE_PORT = $savedPort } else { Remove-Item Env:\IDA_BRIDGE_PORT -ErrorAction SilentlyContinue }
    Remove-Item Env:\IB_FAKE_LIST -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force -LiteralPath $scratch -ErrorAction SilentlyContinue
}

if ($failed -gt 0) {
    Write-Output "$failed check(s) failed"
    exit 1
}
Write-Output 'all checks passed'
exit 0
