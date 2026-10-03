<#
.SYNOPSIS
  The repository's one way into ida-bridge (requirement 1 of
  .kiro/specs/write-path-phase-3): finds the installation, refuses anything
  that could save Core_IDB, checks the bridge is up, picks the instance that
  has the target IDB open, and holds exec to a time limit.

.DESCRIPTION
  powershell -NoProfile -File tools/ida/ib.ps1 [--target-idb <path>] [--timeout-s <n>] <ida-bridge command and arguments>

  --target-idb and --timeout-s belong to the wrapper and are taken out before
  forwarding, wherever they stand; everything else reaches ida-bridge as
  given. --target-idb defaults to Core_IDB. --timeout-s limits exec (and the
  wrapper's own env / xrefs), default 60 s, 0 for none.

  The E3D_IDBs are Core_IDB and Core3D_IDB (SPAMAP entries are computed in
  Core3D.dll); a call whose --out-idb is either of them, or the target IDB,
  is refused like any other call that could save one.

  exec, reset and supervisor stop without an instance id go to the one
  connected instance that has the target IDB open and that no other stateful
  session holds.

  Wrapper commands:
    env              the target IDB's path and SHA-256, the IDA version the
                     instance reports, and git HEAD; run first and last in
                     every round of evidence.
    xrefs <0xaddr>   IB_Xref_Helper (tools/ida/xrefs_to.py) on the target
                     IDB's instance; prints JSON.

  Exit codes of the wrapper itself, with stderr lines starting [ib]:
    90  no usable installation, or ida_bridge does not import from it
    91  refused: the call could save Core_IDB; nothing was sent
    92  the bridge is not reachable
    93  no connected instance has the target IDB open, or the file is missing
    94  more than one instance has it open, or the one is held by another
        stateful session
    95  exec did not return in time; its client was stopped, the instance not
  Anything else is the forwarded command's own exit code (2: usage).

  Checked against the installation (labs-ida-bridge,
  C:\Users\dpc\.agents-old\tools\ida-bridge):
    - CLI entry: python -m ida_bridge (src/ida_bridge/__main__.py); the venv's
      editable install is not relied on, PYTHONPATH puts src first.
    - list --json: {"clients":[{"client_id":"idalib-<pid>","role":"ida",
      "meta":{"pid":...,"idb_path":"..."},"session_id":"..."}]}; session_id
      is the stateful exec owner, absent when none.
    - exec runs --sql, then the -f files in order, then -c, in one globals();
      _result_ comes back as "result" with --json. -f files are read by the
      client, relative to its working directory, which is the caller's.
    - supervisor stop: an idalib instance closes with
      close_database(save=False); a UI instance quits through idc.qexit(0)
      and IDA writes the database on close, so stopping a UI instance (or an
      instance named by PID, which may be one) is refused.
    - Bridge address: IDA_BRIDGE_HOST / IDA_BRIDGE_PORT, default
      127.0.0.1:8765.

.EXAMPLE
  powershell -NoProfile -File tools/ida/ib.ps1 exec --sql "SELECT name, start_ea FROM funcs LIMIT 3"
#>
# No param block, so every argument, -c and -f included, reaches $args as is.

$ErrorActionPreference = 'Stop'

# Where to look for ida-bridge, in order (requirement 1.1). The only list of
# installation places in this script.
$InstallLookup = @(
    @{ Label = 'IDA_BRIDGE_HOME'; Path = $env:IDA_BRIDGE_HOME },
    @{ Label = 'C:\Users\dpc\.agents-old\tools\ida-bridge'; Path = 'C:\Users\dpc\.agents-old\tools\ida-bridge' },
    @{ Label = 'C:\Users\dpc\.agents\tools\ida-bridge'; Path = 'C:\Users\dpc\.agents\tools\ida-bridge' }
)
$CoreIdb = 'D:\ida_scratch\plant3\core.dll.i64'
$Core3dIdb = 'D:\ida_scratch\plant3\Core3D.dll.i64'
$Self = 'powershell -NoProfile -File tools/ida/ib.ps1'
$Repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)

function Write-Ib([string]$Line) { [Console]::Error.WriteLine("[ib] $Line") }

function Stop-Ib([int]$Code, [string[]]$Lines) {
    foreach ($line in $Lines) { Write-Ib $line }
    exit $Code
}

# Absolute, backslashed, lower-cased: how two IDB paths are compared.
function Get-Canonical([string]$Path) {
    try { $full = [IO.Path]::GetFullPath($Path) } catch { $full = $Path }
    return $full.Replace('/', '\').TrimEnd('\').ToLowerInvariant()
}

# One command line from arguments, quoted the way the MSVC runtime splits it.
function ConvertTo-ArgumentString([string[]]$Arguments) {
    $quoted = foreach ($argument in $Arguments) {
        if ($argument.Length -gt 0 -and $argument -notmatch '[\s"]') { $argument; continue }
        $builder = New-Object System.Text.StringBuilder
        [void]$builder.Append('"')
        $backslashes = 0
        foreach ($char in $argument.ToCharArray()) {
            if ($char -eq [char]'\') { $backslashes++; continue }
            if ($char -eq [char]'"') {
                [void]$builder.Append([char]'\', 2 * $backslashes + 1)
            } else {
                [void]$builder.Append([char]'\', $backslashes)
            }
            [void]$builder.Append($char)
            $backslashes = 0
        }
        [void]$builder.Append([char]'\', 2 * $backslashes)
        [void]$builder.Append('"')
        $builder.ToString()
    }
    return ($quoted -join ' ')
}

# The values following any of $Names in $Items.
function Get-OptionValues($Items, [string[]]$Names) {
    for ($i = 0; $i -lt $Items.Count - 1; $i++) {
        if ($Names -contains $Items[$i]) { $Items[$i + 1] }
    }
}

# --- the wrapper's own options -------------------------------------------------

$TargetIdb = $CoreIdb
$TimeoutS = 60
$rest = New-Object System.Collections.Generic.List[string]
for ($i = 0; $i -lt $args.Count; $i++) {
    $argument = [string]$args[$i]
    if ($argument -eq '--target-idb' -or $argument -eq '--timeout-s') {
        if ($i + 1 -ge $args.Count) { Stop-Ib 2 @("missing value for $argument") }
        $i++
        $value = [string]$args[$i]
        if ($argument -eq '--target-idb') {
            $TargetIdb = $value
        } elseif ($value -match '^\d+$') {
            $TimeoutS = [int]$value
        } else {
            Stop-Ib 2 @("--timeout-s takes whole seconds, got $value")
        }
        continue
    }
    $rest.Add($argument)
}
if ($rest.Count -eq 0) {
    Stop-Ib 2 @('usage: tools/ida/ib.ps1 [--target-idb <path>] [--timeout-s <n>] (<ida-bridge command ...> | env | xrefs <0xaddr>)')
}
$command = $rest[0]

# --- 1. the installation ---------------------------------------------------------

$install = $null
$looked = @()
foreach ($place in $InstallLookup) {
    if ([string]::IsNullOrEmpty($place.Path)) {
        $looked += "$($place.Label): not set"
        continue
    }
    $missing = @()
    if (-not (Test-Path -LiteralPath (Join-Path $place.Path 'src\ida_bridge') -PathType Container)) { $missing += 'src\ida_bridge' }
    if (-not (Test-Path -LiteralPath (Join-Path $place.Path '.venv\Scripts\python.exe') -PathType Leaf)) { $missing += '.venv\Scripts\python.exe' }
    if ($missing.Count -eq 0) {
        $install = $place.Path
        break
    }
    $looked += "$($place.Label): missing $($missing -join ', ')"
}
if (-not $install) {
    Stop-Ib 90 (@('no usable ida-bridge installation; looked at, in order:') + ($looked | ForEach-Object { "  $_" }))
}
$python = Join-Path $install '.venv\Scripts\python.exe'
$src = Join-Path $install 'src'

# --- 2. the child's environment ----------------------------------------------------

# Run the installation's python with src first on PYTHONPATH, in the caller's
# working directory. With -Capture the output is returned instead of shown.
# With -LimitS above 0 the client's process tree is stopped at the limit.
function Invoke-Python {
    param([string[]]$PythonArgs, [switch]$Capture, [int]$LimitS = 0)
    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = $python
    $info.Arguments = ConvertTo-ArgumentString $PythonArgs
    $info.UseShellExecute = $false
    $info.WorkingDirectory = (Get-Location).ProviderPath
    $inherited = [Environment]::GetEnvironmentVariable('PYTHONPATH')
    $info.EnvironmentVariables['PYTHONPATH'] = $(if ($inherited) { "$src;$inherited" } else { $src })
    if ($Capture) {
        $info.RedirectStandardOutput = $true
        $info.RedirectStandardError = $true
        $info.StandardOutputEncoding = [Text.Encoding]::UTF8
        $info.StandardErrorEncoding = [Text.Encoding]::UTF8
    }
    $process = [System.Diagnostics.Process]::Start($info)
    if ($Capture) {
        $out = $process.StandardOutput.ReadToEndAsync()
        $err = $process.StandardError.ReadToEndAsync()
    }
    $timedOut = $false
    if ($LimitS -gt 0) {
        if (-not $process.WaitForExit($LimitS * 1000)) {
            $timedOut = $true
            # Under 'Stop', Windows PowerShell would turn taskkill's stderr into
            # a terminating error.
            $ErrorActionPreference = 'Continue'
            & taskkill.exe /T /F /PID $process.Id *> $null
            $ErrorActionPreference = 'Stop'
        }
    }
    $process.WaitForExit()
    $result = [pscustomobject]@{ ExitCode = $process.ExitCode; TimedOut = $timedOut; Stdout = ''; Stderr = '' }
    if ($Capture) {
        $result.Stdout = $out.Result
        $result.Stderr = $err.Result
    }
    return $result
}

function Invoke-Bridge {
    param([string[]]$BridgeArgs, [switch]$Capture, [int]$LimitS = 0)
    return Invoke-Python -PythonArgs (@('-m', 'ida_bridge') + $BridgeArgs) -Capture:$Capture -LimitS $LimitS
}

$check = Invoke-Python -PythonArgs @('-c', 'import ida_bridge; print(ida_bridge.__file__)') -Capture -LimitS 60
$imported = $check.Stdout.Trim()
if ($check.ExitCode -ne 0 -or -not (Get-Canonical $imported).StartsWith((Get-Canonical (Join-Path $src 'ida_bridge')))) {
    Stop-Ib 90 @("ida_bridge does not import from $src", "python said: $imported $($check.Stderr.Trim())")
}

# --- 3. refuse whatever could save Core_IDB, before any request -----------------

$isSupervisor = $command -eq 'supervisor' -or $command -eq 'sup'
$reasons = @()
if ($rest -contains '--save') { $reasons += 'it carries --save' }
if ($isSupervisor -and $rest.Count -ge 2 -and $rest[1] -eq 'save') { $reasons += 'it is supervisor save' }
$guarded = [ordered]@{ 'Core_IDB' = $CoreIdb; 'Core3D_IDB' = $Core3dIdb; 'the target IDB' = $TargetIdb }
foreach ($out in @(Get-OptionValues $rest @('--out-idb'))) {
    $hit = @($guarded.Keys | Where-Object { (Get-Canonical $out) -eq (Get-Canonical $guarded[$_]) }) | Select-Object -First 1
    if ($hit) { $reasons += "its --out-idb is $hit ($out)" }
}
$saving = 'idb\.save|save_database'
foreach ($code in @(Get-OptionValues $rest @('-c', '--code'))) {
    if ($code -match $saving) { $reasons += 'its -c text calls idb.save or save_database' }
}
foreach ($file in @(Get-OptionValues $rest @('-f', '--file'))) {
    $text = $null
    try { $text = [IO.File]::ReadAllText([IO.Path]::Combine((Get-Location).ProviderPath, $file)) } catch { }
    if ($null -eq $text) {
        $reasons += "its -f file cannot be read to check it: $file"
    } elseif ($text -match $saving) {
        $reasons += "its -f file $file calls idb.save or save_database"
    }
}
if ($isSupervisor -and $rest.Count -ge 3 -and $rest[1] -eq 'stop' -and $rest[2] -match '^(idaui-\d+|\d+)$') {
    $reasons += "it stops $($rest[2]): a UI instance writes its database when it closes (a PID may name one)"
}
if ($reasons.Count -gt 0) {
    Stop-Ib 91 (@('refused; nothing was sent to the bridge:') + ($reasons | ForEach-Object { "  $_" }))
}

# --- 4. the bridge ---------------------------------------------------------------------

function Get-StartLines([switch]$WithServer) {
    $lines = @()
    if ($WithServer) { $lines += "  $Self server start" }
    if (Test-Path -LiteralPath $TargetIdb -PathType Leaf) {
        $lines += "  $Self supervisor start-idalib --idb `"$TargetIdb`""
        return @('to bring the target IDB online, run in order:') + $lines
    }
    return @('to bring the target IDB online, run in order:') + $lines + @("the target IDB does not exist: $TargetIdb")
}

if ($command -ne 'server') {
    $bridgeHost = $(if ($env:IDA_BRIDGE_HOST) { $env:IDA_BRIDGE_HOST } else { '127.0.0.1' })
    $bridgePort = $(if ($env:IDA_BRIDGE_PORT) { [int]$env:IDA_BRIDGE_PORT } else { 8765 })
    $reachable = $false
    $client = New-Object System.Net.Sockets.TcpClient
    try {
        $attempt = $client.BeginConnect($bridgeHost, $bridgePort, $null, $null)
        if ($attempt.AsyncWaitHandle.WaitOne(2000)) {
            $client.EndConnect($attempt)
            $reachable = $true
        }
    } catch {
        $reachable = $false
    } finally {
        $client.Close()
    }
    if (-not $reachable) {
        Stop-Ib 92 (@("the bridge is not reachable at ${bridgeHost}:$bridgePort") + (Get-StartLines -WithServer))
    }
}

# --- 5. the instance -------------------------------------------------------------------

function Resolve-Instance {
    $listed = Invoke-Bridge -BridgeArgs @('list', '--json') -Capture -LimitS 60
    if ($listed.ExitCode -ne 0 -or $listed.TimedOut) {
        [Console]::Error.Write($listed.Stderr)
        Stop-Ib $(if ($listed.ExitCode -ne 0) { $listed.ExitCode } else { 92 }) @('ida-bridge list failed')
    }
    $clients = @(($listed.Stdout | ConvertFrom-Json).clients)
    $wanted = Get-Canonical $TargetIdb
    $open = @($clients | Where-Object { $_.role -eq 'ida' -and $_.meta.idb_path -and (Get-Canonical $_.meta.idb_path) -eq $wanted })
    if ($open.Count -eq 0) {
        if (-not (Test-Path -LiteralPath $TargetIdb -PathType Leaf)) {
            Stop-Ib 93 @("no connected instance has $TargetIdb open, and the file does not exist")
        }
        Stop-Ib 93 (@("no connected instance has $TargetIdb open") + (Get-StartLines))
    }
    $caller = @(Get-OptionValues $rest @('--session-id')) | Select-Object -First 1
    $held = $open[0].session_id -and $open[0].session_id -ne $caller
    if ($open.Count -gt 1 -or $held) {
        $lines = @("cannot pick one instance for ${TargetIdb}:")
        foreach ($instance in $open) {
            $holder = $(if ($instance.session_id) { $instance.session_id } else { 'none' })
            $lines += "  instance $($instance.client_id)  pid $($instance.meta.pid)  held by session $holder"
        }
        Stop-Ib 94 $lines
    }
    return $open[0]
}

function Stop-TimedOut([string]$InstanceId) {
    Stop-Ib 95 @(
        "exec on $InstanceId did not return within $TimeoutS s; its client was stopped, the instance was not, and nothing was resent",
        "stop the instance:  $Self supervisor stop $InstanceId",
        "start it again:     $Self supervisor start-idalib --idb `"$TargetIdb`""
    )
}

# Run code in the target's instance and hand back the response's result.
function Invoke-InInstance([string[]]$ExecArgs) {
    $instance = Resolve-Instance
    $run = Invoke-Bridge -BridgeArgs (@('exec', $instance.client_id, '--json') + $ExecArgs) -Capture -LimitS $TimeoutS
    if ($run.TimedOut) { Stop-TimedOut $instance.client_id }
    $response = $null
    try { $response = $run.Stdout | ConvertFrom-Json } catch { }
    if ($run.ExitCode -ne 0 -or $null -eq $response -or -not $response.ok) {
        [Console]::Error.Write($run.Stdout)
        [Console]::Error.Write($run.Stderr)
        exit $(if ($run.ExitCode -ne 0) { $run.ExitCode } else { 1 })
    }
    return $response.result
}

if ($command -eq 'env') {
    $ida = Invoke-InInstance @('-c', 'import ida_kernwin; _result_ = ida_kernwin.get_kernel_version()')
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $TargetIdb).Hash
    $head = (& git -C $Repo rev-parse HEAD).Trim()
    Write-Output "idb: $TargetIdb"
    Write-Output "idb_sha256: $hash"
    Write-Output "ida_version: $ida"
    Write-Output "head: $head"
    exit 0
}

if ($command -eq 'xrefs') {
    if ($rest.Count -ne 2 -or $rest[1] -notmatch '^0x[0-9A-Fa-f]+$') {
        Stop-Ib 2 @('usage: tools/ida/ib.ps1 [--target-idb <path>] [--timeout-s <n>] xrefs 0x<address>')
    }
    $result = Invoke-InInstance @('-f', (Join-Path $PSScriptRoot 'xrefs_to.py'), '-c', "_result_ = xrefs_to($($rest[1]))")
    Write-Output ($result | ConvertTo-Json -Depth 6 -Compress)
    exit 0
}

$forward = @($rest)
$instanceId = @($rest | Where-Object { $_ -match '^(idalib|idaui)-\d+$' }) | Select-Object -First 1
if ($command -eq 'exec' -or $command -eq 'reset') {
    if (-not $instanceId) {
        $instanceId = (Resolve-Instance).client_id
        $forward = @($command, $instanceId) + @($rest | Select-Object -Skip 1)
    }
} elseif ($isSupervisor -and $rest.Count -ge 2 -and $rest[1] -eq 'stop' -and $rest.Count -lt 3) {
    $instance = Resolve-Instance
    if ($instance.client_id -notmatch '^idalib-') {
        Stop-Ib 91 @("refused; $($instance.client_id) is a UI instance, which writes its database when it closes")
    }
    $forward = @($rest[0], 'stop', $instance.client_id)
}

# --- 6. and 7. forward ------------------------------------------------------------------

if ($command -eq 'exec' -and $TimeoutS -gt 0) {
    $run = Invoke-Bridge -BridgeArgs $forward -LimitS $TimeoutS
    if ($run.TimedOut) { Stop-TimedOut $instanceId }
    exit $run.ExitCode
}
$run = Invoke-Bridge -BridgeArgs $forward
exit $run.ExitCode
