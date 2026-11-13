@echo off
setlocal DisableDelayedExpansion
rem Native launcher derived from the Trinity project; PowerShell payload is embedded below.
set "RMR_LAUNCHER=%~f0"
set "RMR_INVOCATION_DIRECTORY=%CD%"
set "RMR_MODE=ui"
set "RMR_UI_PID="
set "RMR_RUN_ID="
:parse
if "%~1"=="" goto parsed
if /i "%~1"=="--help" goto help
if /i "%~1"=="-h" goto help
if /i "%~1"=="--headless" goto headless
if /i "%~1"=="--stop" goto stop
if /i "%~1"=="--check" goto check
if /i "%~1"=="--server-only" goto serveronly
if /i "%~1"=="--no-auto-client" goto noautoclient
if /i "%~1"=="--client" goto client
if /i "%~1"=="--game-port" goto gameport
if /i "%~1"=="--web-port" goto webport
if /i "%~1"=="--database-port" goto databaseport
if /i "%~1"=="--host" goto hostaddress
if /i "%~1"=="--allow-public" goto allowpublic
echo Unknown option. Use --help for usage. 1>&2
exit /b 2
:headless
set "RMR_MODE=headless"
shift
goto parse
:stop
set "RMR_MODE=stop"
shift
goto parse
:check
set "RMR_CHECK=1"
set "RMR_SERVER_ONLY=1"
set "RMR_NO_CLIENT=1"
set "RMR_MODE=headless"
shift
goto parse
:serveronly
set "RMR_NO_CLIENT=1"
set "RMR_SERVER_ONLY=1"
set "RMR_MODE=headless"
shift
goto parse
:noautoclient
set "RMR_NO_AUTO_CLIENT=1"
shift
goto parse
:client
if "%~2"=="" goto missing
set "RMR_VALUE=%~2"
if "%RMR_VALUE:~0,2%"=="--" goto missing
set "RMR_CLIENT_DIRECTORY=%RMR_VALUE%"
shift
shift
goto parse
:gameport
if "%~2"=="" goto missing
set "RMR_VALUE=%~2"
if "%RMR_VALUE:~0,2%"=="--" goto missing
set "RMR_GAME_PORT=%RMR_VALUE%"
shift
shift
goto parse
:webport
if "%~2"=="" goto missing
set "RMR_VALUE=%~2"
if "%RMR_VALUE:~0,2%"=="--" goto missing
set "RMR_WEB_PORT=%RMR_VALUE%"
shift
shift
goto parse
:databaseport
if "%~2"=="" goto missing
set "RMR_VALUE=%~2"
if "%RMR_VALUE:~0,2%"=="--" goto missing
set "RMR_DATABASE_PORT=%RMR_VALUE%"
shift
shift
goto parse
:allowpublic
set "RMR_ALLOW_PUBLIC=1"
shift
goto parse
:hostaddress
if "%~2"=="" goto missing
set "RMR_VALUE=%~2"
if "%RMR_VALUE:~0,2%"=="--" goto missing
set "RMR_HOST=%RMR_VALUE%"
shift
shift
goto parse
:missing
echo An option is missing its value. Use --help for usage. 1>&2
exit /b 2
:help
echo Red Moon Rising server launcher
echo Usage: Start.bat [--headless ^| --server-only ^| --check ^| --stop]
echo                  [--host local^|ADDRESS] [--game-port N] [--web-port N] [--database-port N]
echo No options opens the launcher window, which asks where to host the server.
echo --host local hosts for this PC only; --host ADDRESS hosts on this PC's IPv4
echo address on your local network (such as 192.168.1.20). Red Moon Rising is for
echo your own PC and local network only. Without --host the last choice is used.
echo --server-only runs in this console with the saved choices and asks nothing.
echo --check tests the server then stops it. Client.bat sets up and starts EVE;
echo Dashboard.bat puts the dashboard online behind a login.
echo Default ports: game 26000, web 18080, database 5432; busy ports advance automatically.
echo Explicit ports must be available. --help displays this text without setup.
exit /b 0
:parsed
set "RMR_PS=%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe"
if exist "%SystemRoot%\Sysnative\WindowsPowerShell\v1.0\powershell.exe" set "RMR_PS=%SystemRoot%\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
if not "%RMR_MODE%"=="ui" goto console
start "" "%RMR_PS%" -NoLogo -NoProfile -ExecutionPolicy Bypass -STA -WindowStyle Hidden -Command "try { [AppContext]::SetSwitch('Switch.System.IO.UseLegacyPathHandling',$false); [AppContext]::SetSwitch('Switch.System.IO.BlockLongPaths',$false); $parts = [IO.File]::ReadAllText($env:RMR_LAUNCHER) -split '(?m)^#==RMR_POWERSHELL==\r?$', 2; if ($parts.Count -ne 2) { throw 'Launcher payload is missing.' }; & ([scriptblock]::Create($parts[1])) } catch { Add-Type -AssemblyName System.Windows.Forms; [Windows.Forms.MessageBox]::Show($_.Exception.Message,'Red Moon Rising could not open this folder','OK','Error') | Out-Null; exit 1 }"
exit /b
:console
"%RMR_PS%" -NoLogo -NoProfile -ExecutionPolicy Bypass -STA -Command "try { [AppContext]::SetSwitch('Switch.System.IO.UseLegacyPathHandling',$false); [AppContext]::SetSwitch('Switch.System.IO.BlockLongPaths',$false); $parts = [IO.File]::ReadAllText($env:RMR_LAUNCHER) -split '(?m)^#==RMR_POWERSHELL==\r?$', 2; if ($parts.Count -ne 2) { throw 'Launcher payload is missing.' }; & ([scriptblock]::Create($parts[1])) } catch { Write-Error $_; exit 1 }"
exit /b %errorlevel%
#==RMR_POWERSHELL==

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
# The server only: Client.bat prepares and starts the game client.
$env:RMR_NO_CLIENT = '1'
$Self = [IO.Path]::GetFullPath($env:RMR_LAUNCHER)
$Root = [IO.Path]::GetDirectoryName($Self)
$SourceRoot = $Root
Set-Location -LiteralPath $Root
[Environment]::CurrentDirectory = $Root
$Runtime = Join-Path $Root '.rmr'
$Logs = Join-Path $Root 'logs'
$Utf8 = New-Object Text.UTF8Encoding($false)
[IO.Directory]::CreateDirectory($Runtime) | Out-Null
[IO.Directory]::CreateDirectory($Logs) | Out-Null
foreach ($ignoreDir in @($Runtime, $Logs)) {
    $ignoreFile = Join-Path $ignoreDir '.gitignore'
    if (-not [IO.File]::Exists($ignoreFile)) { [IO.File]::WriteAllText($ignoreFile, "*`n", $Utf8) }
}
$LogPath = Join-Path $Logs 'launcher.log'
$StopPath = Join-Path $Runtime 'stop.request'
$StatusPath = Join-Path $Runtime 'status.json'
if (-not $env:RMR_RUN_ID) { $env:RMR_RUN_ID = [Guid]::NewGuid().ToString('N') }
$script:Status = [ordered]@{ runId = $env:RMR_RUN_ID; stage = 'Preparing'; message = 'Checking local setup'; progress = 0; ready = $false; error = $false; stopped = $false; gamePort = 0; webPort = 0 }
$script:Stopping = $false

function Get-RmrFileHash([string]$Path) {
    $sha = [Security.Cryptography.SHA256]::Create()
    $stream = [IO.File]::OpenRead($Path)
    try { return [BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-', '') }
    finally { $stream.Dispose(); $sha.Dispose() }
}

function Write-Log([string]$Message) {
    $Message = $Message -replace '\x1B\[[0-9;]*[a-zA-Z]', ''
    [IO.File]::AppendAllText($LogPath, $Message + [Environment]::NewLine, $Utf8)
    if ($env:RMR_MODE -ne 'worker') { Write-Host $Message }
}
function Set-Status([string]$Stage, [string]$Message, [int]$Progress) {
    $script:Status['downloadPercent'] = $null
    $script:Status['downloadName'] = ''
    $script:Status['phasePercent'] = $null
    $script:Status['phaseLabel'] = ''
    $script:Status.stage = $Stage
    $script:Status.message = $Message
    $script:Status.progress = $Progress
    [IO.File]::WriteAllText($StatusPath, ($script:Status | ConvertTo-Json -Compress), $Utf8)
    Write-Log ('[{0}] {1}' -f $Stage, $Message)
}
function Invoke-RmrWindowRestart {
    # Clicks the launcher window's own Restart server button, so a dashboard
    # restart does exactly what that button does. WinForms buttons are real
    # windows: find the one with that label and send it a button click.
    if (-not $env:RMR_UI_PID) { return $false }
    try {
        if (-not ('RmrUi.WindowButtons' -as [type])) {
            Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace RmrUi {
    public static class WindowButtons {
        delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);
        [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback, IntPtr lParam);
        [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc callback, IntPtr lParam);
        [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr hWnd, StringBuilder name, int size);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint msg, IntPtr wParam, StringBuilder lParam, uint flags, uint timeout, out IntPtr result);
        [DllImport("user32.dll")] static extern bool IsWindowEnabled(IntPtr hWnd);
        [DllImport("user32.dll")] static extern bool PostMessage(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);
        const uint WM_GETTEXT = 0x000D, BM_CLICK = 0x00F5, SMTO_ABORTIFHUNG = 0x0002;

        static string Text(IntPtr hWnd) {
            StringBuilder text = new StringBuilder(256);
            IntPtr ignored;
            SendMessageTimeout(hWnd, WM_GETTEXT, (IntPtr)text.Capacity, text, SMTO_ABORTIFHUNG, 2000, out ignored);
            return text.ToString();
        }

        // Clicks the enabled button labelled `label` in a window of process
        // `pid`. Returns "clicked", or what was seen, for the log.
        public static string Click(int pid, string label) {
            List<IntPtr> windows = new List<IntPtr>();
            EnumWindows(delegate (IntPtr hWnd, IntPtr l) {
                uint owner; GetWindowThreadProcessId(hWnd, out owner);
                if (owner == (uint)pid) windows.Add(hWnd);
                return true;
            }, IntPtr.Zero);
            List<string> seen = new List<string>();
            IntPtr found = IntPtr.Zero;
            foreach (IntPtr window in windows) {
                EnumChildWindows(window, delegate (IntPtr child, IntPtr l) {
                    StringBuilder cls = new StringBuilder(256);
                    GetClassName(child, cls, cls.Capacity);
                    if (cls.ToString().IndexOf("BUTTON", StringComparison.OrdinalIgnoreCase) < 0) return true;
                    string text = Text(child).Replace("&", "");
                    seen.Add(text + (IsWindowEnabled(child) ? "" : " (disabled)"));
                    if (found == IntPtr.Zero && string.Equals(text, label, StringComparison.OrdinalIgnoreCase) && IsWindowEnabled(child)) found = child;
                    return true;
                }, IntPtr.Zero);
            }
            if (found != IntPtr.Zero && PostMessage(found, BM_CLICK, IntPtr.Zero, IntPtr.Zero)) return "clicked";
            return windows.Count + " window(s); buttons: " + (seen.Count > 0 ? string.Join(", ", seen.ToArray()) : "none");
        }
    }
}
'@
        }
        $result = [RmrUi.WindowButtons]::Click([int]$env:RMR_UI_PID, 'Restart server')
        if ($result -eq 'clicked') { return $true }
        Write-Log ('Dashboard restart: the launcher window''s Restart server button was not available (' + $result + ').')
    } catch {
        Write-Log ('Dashboard restart: could not click the launcher window''s Restart server button: ' + $_.Exception.Message)
    }
    return $false
}
function Assert-Running {
    if ($script:Stopping) { return }
    if (Test-Path -LiteralPath $StopPath) {
        $request = [IO.File]::ReadAllText($StopPath).Trim()
        if ($request -eq $env:RMR_RUN_ID -or $request -eq 'stop') { throw [OperationCanceledException]::new('Stop requested.') }
    }
    if ($env:RMR_UI_PID -and -not (Get-Process -Id ([int]$env:RMR_UI_PID) -ErrorAction SilentlyContinue)) {
        throw [OperationCanceledException]::new('Launcher window closed.')
    }
}
function Quote-Argument([string]$Value) {
    '"' + ([regex]::Replace([regex]::Replace($Value, '(\\*)"', '$1$1\"'), '(\\+)$', '$1$1')) + '"'
}
function Initialize-NativePathAliasType {
    if ('Rmr.NativePathAlias' -as [type]) { return }
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

namespace Rmr {
    public static class NativePathAlias {
        [DllImport("kernel32.dll", EntryPoint = "DefineDosDeviceW", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool DefineDosDevice(uint flags, string device, string target);

        [DllImport("kernel32.dll", EntryPoint = "QueryDosDeviceW", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern uint QueryDosDevice(string device, StringBuilder target, int capacity);

        public static string Query(string device) {
            var target = new StringBuilder(32768);
            if (QueryDosDevice(device, target, target.Capacity) != 0)
                return target.ToString();
            int error = Marshal.GetLastWin32Error();
            if (error == 2) return null;
            throw new Win32Exception(error, "Cannot inspect drive " + device);
        }

        public static void Create(string device, string target) {
            // DDD_RAW_TARGET_PATH | DDD_NO_BROADCAST_SYSTEM.
            if (!DefineDosDevice(0x00000009, device, target))
                throw new Win32Exception(Marshal.GetLastWin32Error(), "Cannot create temporary drive " + device);
        }

        public static void RemoveExact(string device, string target) {
            // RAW_TARGET_PATH | REMOVE_DEFINITION | EXACT_MATCH_ON_REMOVE | NO_BROADCAST_SYSTEM.
            if (!DefineDosDevice(0x0000000f, device, target)) {
                int error = Marshal.GetLastWin32Error();
                if (error != 2)
                    throw new Win32Exception(error, "Cannot remove temporary drive " + device);
            }
        }
    }
}
'@
}
function New-NativePathAlias {
    param([Parameter(Mandatory = $true)][string]$Path)
    Initialize-NativePathAliasType
    $source = [IO.Path]::GetFullPath($Path)
    if ($source.StartsWith('\\?\UNC\', [StringComparison]::OrdinalIgnoreCase)) {
        $target = '\??\UNC\' + $source.Substring(8)
    } elseif ($source.StartsWith('\\?\', [StringComparison]::OrdinalIgnoreCase)) {
        $target = '\??\' + $source.Substring(4)
    } elseif ($source.StartsWith('\\', [StringComparison]::Ordinal)) {
        $target = '\??\UNC\' + $source.Substring(2)
    } else {
        $target = '\??\' + $source
    }
    if ($source.Length -gt [IO.Path]::GetPathRoot($source).Length) { $target = $target.TrimEnd('\') }

    # Reuse exact existing aliases first, so a surviving private database retains
    # the same path after an interrupted launch. Existing mappings are never removed.
    foreach ($reuse in @($true, $false)) {
        for ($letter = 90; $letter -ge 68; $letter--) {
            $drive = ([char]$letter).ToString() + ':'
            $mutex = [Threading.Mutex]::new($false, ('Local\Trinity.NativePathAlias.' + [char]$letter))
            $held = $false; $created = $false; $keep = $false
            try {
                try { $held = $mutex.WaitOne(0) }
                catch [Threading.AbandonedMutexException] { $held = $true }
                if (-not $held) { continue }
                $existing = [Rmr.NativePathAlias]::Query($drive)
                if ($reuse) {
                    if (-not [string]::Equals($existing, $target, [StringComparison]::OrdinalIgnoreCase)) { continue }
                } else {
                    if ($null -ne $existing) { continue }
                    [Rmr.NativePathAlias]::Create($drive, $target)
                    $created = $true
                }
                if (-not [string]::Equals([Rmr.NativePathAlias]::Query($drive), $target, [StringComparison]::OrdinalIgnoreCase)) {
                    throw "Temporary drive $drive changed while preparing Rmr. Run start.bat again."
                }
                # Register the new OS drive with this PowerShell session too.
                # Sessions started before the mapping may not have mounted it yet.
                $driveName = ([char]$letter).ToString()
                if (-not (Get-PSDrive -Name $driveName -PSProvider FileSystem -ErrorAction SilentlyContinue)) {
                    New-PSDrive -Name $driveName -PSProvider FileSystem -Root ($drive + '\') -Scope Script | Out-Null
                }
                $alias = [pscustomobject]@{
                    Root = $drive + '\'; Drive = $drive; Target = $target
                    SourcePath = $source; Mutex = $mutex; Created = $created; Removed = $false
                }
                $keep = $true
                return $alias
            } finally {
                if (-not $keep) {
                    try { if ($created) { [Rmr.NativePathAlias]::RemoveExact($drive, $target) } }
                    finally {
                        if ($held) { $mutex.ReleaseMutex() }
                        $mutex.Dispose()
                    }
                }
            }
        }
    }
    throw 'No free drive letter is available for Rmr. Free one drive letter from D: through Z: and run start.bat again.'
}
function Remove-NativePathAlias {
    param($Alias)
    if ($null -eq $Alias -or $Alias.Removed) { return }
    try {
        if ($Alias.Created) { [Rmr.NativePathAlias]::RemoveExact($Alias.Drive, $Alias.Target) }
        $Alias.Removed = $true
    } finally {
        if ($null -ne $Alias.Mutex) {
            try { $Alias.Mutex.ReleaseMutex() }
            finally { $Alias.Mutex.Dispose(); $Alias.Mutex = $null }
        }
    }
}
function New-NativeProcess([string]$File, [string[]]$Arguments) {
    $startInfo = New-Object Diagnostics.ProcessStartInfo
    $startInfo.FileName = $File
    $startInfo.Arguments = ($Arguments | ForEach-Object { Quote-Argument $_ }) -join ' '
    $startInfo.WorkingDirectory = $Root
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $p = New-Object Diagnostics.Process
    $p.StartInfo = $startInfo
    [void]$p.Start()
    return @{ Process = $p; Out = $p.StandardOutput.ReadLineAsync(); Err = $p.StandardError.ReadLineAsync(); Lines = (New-Object 'Collections.Generic.List[string]') }
}
function Read-NativeOutput($Job, [switch]$Capture) {
    foreach ($key in @('Out', 'Err')) {
        $limit = 200
        while ($null -ne $Job[$key] -and $Job[$key].IsCompleted -and $limit-- -gt 0) {
            $line = $Job[$key].GetAwaiter().GetResult()
            if ($null -eq $line) { $Job[$key] = $null; break }
            if ($Capture -and $key -eq 'Out') { $Job.Lines.Add($line) }
            else { Write-Log $line }
            if ($key -eq 'Out') { $Job[$key] = $Job.Process.StandardOutput.ReadLineAsync() }
            else { $Job[$key] = $Job.Process.StandardError.ReadLineAsync() }
        }
    }
}
function Stop-NativeProcess([Diagnostics.Process]$NativeProcess) {
    # Keep the original process handle open while targeting its PID, so it cannot
    # be recycled for an unrelated process. /T also stops cargo/rustup children.
    if ($NativeProcess.HasExited) { return }
    $startInfo = New-Object Diagnostics.ProcessStartInfo
    $startInfo.FileName = Join-Path $env:SystemRoot 'System32\taskkill.exe'
    $startInfo.Arguments = '/PID ' + $NativeProcess.Id + ' /T /F'
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $killer = New-Object Diagnostics.Process
    $killer.StartInfo = $startInfo
    try {
        [void]$killer.Start()
        $stdout = $killer.StandardOutput.ReadToEndAsync()
        $stderr = $killer.StandardError.ReadToEndAsync()
        if (-not $killer.WaitForExit(15000)) {
            $killer.Kill()
            Write-Log 'Process-tree shutdown timed out; stopping the owned parent process.'
        } elseif ($killer.ExitCode -ne 0 -and -not $NativeProcess.HasExited) {
            Write-Log ('Process-tree shutdown reported: ' + $stderr.GetAwaiter().GetResult().Trim())
        }
    } catch { Write-Log ('Process-tree shutdown reported: ' + $_.Exception.Message) }
    finally { $killer.Dispose() }
    if (-not $NativeProcess.HasExited) { $NativeProcess.Kill() }
    [void]$NativeProcess.WaitForExit(10000)
}
function Invoke-Native([string]$File, [string[]]$Arguments, [switch]$Capture, [int]$Timeout = 1800) {
    $job = New-NativeProcess $File $Arguments
    $watch = [Diagnostics.Stopwatch]::StartNew()
    try {
        $exitedAt = $null
        while ($true) {
            Assert-Running
            Read-NativeOutput $job -Capture:$Capture
            if ($job.Process.HasExited) {
                if ($null -eq $exitedAt) { $exitedAt = $watch.Elapsed.TotalMilliseconds }
                # Daemons started by pg_ctl can inherit a pipe handle. Never wait
                # for their EOF after the actual command has already completed.
                if (($null -eq $job.Out -and $null -eq $job.Err) -or ($watch.Elapsed.TotalMilliseconds - $exitedAt -gt 250)) { break }
            }
            if ($watch.Elapsed.TotalSeconds -gt $Timeout) { throw "Timed out running $(Split-Path -Leaf $File). See logs/launcher.log." }
            Start-Sleep -Milliseconds 40
        }
        if ($job.Process.ExitCode -ne 0) { throw "$(Split-Path -Leaf $File) exited with code $($job.Process.ExitCode). See logs/launcher.log." }
        if ($Capture) { return ($job.Lines -join "`n").Trim() }
    } finally {
        Stop-NativeProcess $job.Process
        $job.Process.Dispose()
    }
}
function Initialize-DownloadProgressType {
    if ('Rmr.LiveDownload' -as [type]) { return }
    # Progress callbacks run on .NET threads, without a PowerShell runspace.
    # Keep their counters in C# and poll them from the worker's own thread.
    Add-Type -TypeDefinition @'
using System;
using System.Net;
using System.Threading;
using System.Threading.Tasks;
namespace Rmr {
    public sealed class LiveDownload : IDisposable {
        private readonly WebClient client = new WebClient();
        private long received;
        private long total = -1;
        private int percent = -1;
        public long Received { get { return Interlocked.Read(ref received); } }
        public long Total { get { return Interlocked.Read(ref total); } }
        public int Percent { get { return Interlocked.CompareExchange(ref percent, 0, 0); } }
        public LiveDownload() { client.DownloadProgressChanged += OnProgress; }
        private void OnProgress(object sender, DownloadProgressChangedEventArgs e) {
            Interlocked.Exchange(ref received, e.BytesReceived);
            Interlocked.Exchange(ref total, e.TotalBytesToReceive);
            Interlocked.Exchange(ref percent, e.TotalBytesToReceive > 0 ? e.ProgressPercentage : -1);
        }
        public Task Start(string url, string path) { return client.DownloadFileTaskAsync(new Uri(url), path); }
        public void Cancel() { client.CancelAsync(); }
        public void Dispose() { client.Dispose(); }
    }
}
'@
}
function Set-PhaseProgress([string]$Label, [string]$Message, [double]$Percent) {
    $script:Status['downloadPercent'] = $null
    $script:Status['phaseLabel'] = $Label
    $script:Status['phasePercent'] = [Math]::Max(0, [Math]::Min(100, $Percent))
    $script:Status.message = $Message
    [IO.File]::WriteAllText($StatusPath, ($script:Status | ConvertTo-Json -Compress), $Utf8)
}
function Get-FileHashWithProgress([string]$Path, [string]$DisplayName) {
    Write-Log ('Verifying ' + $DisplayName)
    Set-PhaseProgress 'VERIFYING FILE' ('Verifying {0} - 0%' -f $DisplayName) 0
    $stream = [IO.File]::OpenRead($Path)
    $hash = [Security.Cryptography.SHA256]::Create()
    $buffer = New-Object byte[] (1MB)
    $processed = [long]0
    $watch = [Diagnostics.Stopwatch]::StartNew()
    try {
        $total = $stream.Length
        while (($count = $stream.Read($buffer, 0, $buffer.Length)) -gt 0) {
            Assert-Running
            [void]$hash.TransformBlock($buffer, 0, $count, $buffer, 0)
            $processed += $count
            if ($watch.ElapsedMilliseconds -ge 150 -or $processed -eq $total) {
                $percent = if ($total -gt 0) { [Math]::Floor(100.0 * $processed / $total) } else { 100 }
                Set-PhaseProgress 'VERIFYING FILE' ('Verifying {0} - {1}% ({2:N1} / {3:N1} MB)' -f $DisplayName,$percent,($processed / 1MB),($total / 1MB)) $percent
                $watch.Restart()
            }
        }
        [void]$hash.TransformFinalBlock([byte[]]@(), 0, 0)
        Set-PhaseProgress 'VERIFYING FILE' ('Verified {0} - 100%' -f $DisplayName) 100
        return ([BitConverter]::ToString($hash.Hash)).Replace('-', '').ToLowerInvariant()
    } finally { $stream.Dispose(); $hash.Dispose() }
}
function Get-Download([string]$Url, [string]$Name, [string]$Sha256) {
    if ([string]::IsNullOrWhiteSpace($Name) -or [IO.Path]::GetFileName($Name) -ne $Name) { throw 'Invalid download filename.' }
    $directory = Join-Path $Runtime 'downloads'
    [IO.Directory]::CreateDirectory($directory) | Out-Null
    $destination = Join-Path $directory $Name
    if ([IO.File]::Exists($destination) -and (Get-FileHashWithProgress $destination $Name) -eq $Sha256) { return $destination }
    Initialize-DownloadProgressType
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    $partial = $destination + '.part'
    for ($attempt = 1; $attempt -le 3; $attempt++) {
        Assert-Running
        Write-Log "Downloading $Name (attempt $attempt/3)"
        $client = [Rmr.LiveDownload]::new()
        $script:Status['downloadName'] = $Name
        $script:Status['downloadPercent'] = 0
        $script:Status['phasePercent'] = $null
        $script:Status.message = 'Downloading {0} - 0%' -f $Name
        [IO.File]::WriteAllText($StatusPath, ($script:Status | ConvertTo-Json -Compress), $Utf8)
        try {
            $task = $client.Start($Url, $partial)
            while (-not $task.IsCompleted) {
                Assert-Running
                $received = $client.Received; $total = $client.Total; $percent = $client.Percent
                $script:Status['downloadReceived'] = $received
                $script:Status['downloadTotal'] = $total
                if ($total -gt 0) {
                    $script:Status['downloadPercent'] = [Math]::Max(0, [Math]::Min(100, $percent))
                    $script:Status.message = 'Downloading {0} - {1}% ({2:N1} / {3:N1} MB)' -f $Name,$script:Status.downloadPercent,($received / 1MB),($total / 1MB)
                } else {
                    $script:Status['downloadPercent'] = $null
                    $script:Status.message = 'Downloading {0} - {1:N1} MB received (waiting for total size)' -f $Name,($received / 1MB)
                }
                [IO.File]::WriteAllText($StatusPath, ($script:Status | ConvertTo-Json -Compress), $Utf8)
                Start-Sleep -Milliseconds 250
            }
            # WebClient's runtime Task can return a null completion object.
            # Suppress it so callers receive exactly one string: the file path.
            [void]$task.GetAwaiter().GetResult()
            if ((Get-FileHashWithProgress $partial $Name) -ne $Sha256) { throw "Checksum mismatch for $Name. Download refused." }
            if ([IO.File]::Exists($destination)) { [IO.File]::Replace($partial, $destination, [NullString]::Value) }
            else { [IO.File]::Move($partial, $destination) }
            return [string]$destination
        } catch [OperationCanceledException] { throw }
        catch { if ($attempt -eq 3) { throw }; Write-Log $_.Exception.Message }
        finally { $client.Cancel(); $client.Dispose() }
    }
}
function Expand-PortableZip([string]$Archive, [string]$Destination) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [IO.Directory]::CreateDirectory($Destination) | Out-Null
    $zip = [IO.Compression.ZipFile]::OpenRead($Archive)
    $base = [IO.Path]::GetFullPath($Destination).TrimEnd('\') + '\'
    try {
        $entries = @($zip.Entries | Where-Object { $_.FullName -match '^pgsql/(bin|lib|share)/' -and $_.Name -ne '' })
        $completed = 0
        $watch = [Diagnostics.Stopwatch]::StartNew()
        Set-PhaseProgress 'EXTRACTING POSTGRESQL' 'Download verified. Extracting PostgreSQL - 0%' 0
        foreach ($entry in $entries) {
            Assert-Running
            # pgAdmin is not needed and contains paths too long for PowerShell 5.1.
            if ($entry.FullName -notmatch '^pgsql/(bin|lib|share)/') { continue }
            $output = [IO.Path]::GetFullPath((Join-Path $Destination $entry.FullName))
            if (-not $output.StartsWith($base, [StringComparison]::OrdinalIgnoreCase)) { throw 'Archive contains an invalid path.' }
            if ($entry.Name -eq '') { [IO.Directory]::CreateDirectory($output) | Out-Null; continue }
            [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($output)) | Out-Null
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $output, $true)
            $completed++
            if ($watch.ElapsedMilliseconds -ge 150 -or $completed -eq $entries.Count) {
                $percent = [Math]::Floor(100.0 * $completed / $entries.Count)
                Set-PhaseProgress 'EXTRACTING POSTGRESQL' ('Extracting PostgreSQL - {0}% ({1} / {2} files)' -f $percent,$completed,$entries.Count) $percent
                $watch.Restart()
            }
        }
    } finally { $zip.Dispose() }
}
function Test-FreePort([int]$Port) {
    $listener = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback, $Port)
    $listener.ExclusiveAddressUse = $true
    try { $listener.Start(); return $true } catch { return $false } finally { $listener.Stop() }
}
function Test-Listening([int]$Port) {
    $tcp = New-Object Net.Sockets.TcpClient
    try { $task = $tcp.ConnectAsync('127.0.0.1', $Port); return ($task.Wait(200) -and $tcp.Connected) } catch { return $false } finally { $tcp.Dispose() }
}
function Test-OwnedPostgres([string]$DataDirectory, [string]$Executable) {
    $pidFile = Join-Path $DataDirectory 'postmaster.pid'
    if (-not (Test-Path -LiteralPath $pidFile)) { return $false }
    $pidLines = [IO.File]::ReadAllLines($pidFile)
    $postgresId = 0
    $startSeconds = [long]0
    if ($pidLines.Length -lt 3 -or -not [int]::TryParse($pidLines[0], [ref]$postgresId) -or $postgresId -le 0) {
        throw 'Cannot verify PostgreSQL ownership: invalid postmaster.pid. No process was stopped.'
    }
    $pgProcess = Get-Process -Id $postgresId -ErrorAction SilentlyContinue
    if (-not $pgProcess) { return $false } # PostgreSQL can recover its stale PID file.
    try {
        $recordedData = [IO.Path]::GetFullPath($pidLines[1]).TrimEnd('\')
        $expectedData = [IO.Path]::GetFullPath($DataDirectory).TrimEnd('\')
        $expectedExecutable = [IO.Path]::GetFullPath($Executable)
        if (-not [string]::Equals($recordedData, $expectedData, [StringComparison]::OrdinalIgnoreCase) -or
            -not [string]::Equals($pgProcess.Path, $expectedExecutable, [StringComparison]::OrdinalIgnoreCase) -or
            -not [long]::TryParse($pidLines[2], [ref]$startSeconds)) {
            throw 'Cannot verify PostgreSQL ownership: the live PID does not match this database. No process was stopped.'
        }
        $epoch = [DateTime]::SpecifyKind([DateTime]'1970-01-01', [DateTimeKind]::Utc)
        $recordedStart = $epoch.AddSeconds($startSeconds)
        if ([Math]::Abs(($pgProcess.StartTime.ToUniversalTime() - $recordedStart).TotalSeconds) -gt 3) {
            throw 'Cannot verify PostgreSQL ownership: the recorded PID has been reused. No process was stopped.'
        }
        return $true
    } finally { $pgProcess.Dispose() }
}
function Select-RmrPort([string]$Variable, $Saved, [int]$Default, [int[]]$Taken) {
    $explicit = [Environment]::GetEnvironmentVariable($Variable, 'Process')
    $chosen = 0
    if (-not [string]::IsNullOrWhiteSpace($explicit)) {
        if (-not [int]::TryParse($explicit, [ref]$chosen) -or $chosen -lt 1 -or $chosen -gt 65535) { throw "$Variable must be a port from 1 to 65535." }
        if ($Taken -contains $chosen -or -not (Test-FreePort $chosen)) { throw "$Variable requests port $chosen, which is already in use." }
        return $chosen
    }
    $candidate = 0
    if (-not [int]::TryParse([string]$Saved, [ref]$candidate) -or $candidate -lt 1 -or $candidate -gt 65535) { $candidate = $Default }
    for ($port = $candidate; $port -le 65535; $port++) {
        Assert-Running
        if ($Taken -notcontains $port -and (Test-FreePort $port)) { return $port }
    }
    throw "No free port at or above $candidate."
}
function Test-RmrOwnedPort([int]$Port, [int]$ProcessId) {
    $listeners = @(Get-NetTCPConnection -State Listen -LocalPort $Port -ErrorAction SilentlyContinue)
    return @($listeners | Where-Object { $_.OwningProcess -eq $ProcessId }).Count -gt 0
}
function Get-RmrBuildFingerprint([string]$Checkout) {
    $files = @((Join-Path $Checkout 'Cargo.toml'), (Join-Path $Checkout 'Cargo.lock'))
    $files += [IO.Directory]::GetFiles((Join-Path $Checkout 'crates'), '*', [IO.SearchOption]::AllDirectories)
    $cargoConfig = Join-Path $Checkout '.cargo\config.toml'
    if (Test-Path -LiteralPath $cargoConfig -PathType Leaf) { $files += $cargoConfig }
    [string[]]$files = $files
    [Array]::Sort($files, [StringComparer]::Ordinal)
    $hash = [Security.Cryptography.SHA256]::Create()
    try {
        foreach ($file in $files) {
            Assert-Running
            $name = [Text.Encoding]::UTF8.GetBytes($file.Substring($Checkout.Length) + "`0")
            [void]$hash.TransformBlock($name,0,$name.Length,$null,0)
            $stream = [IO.File]::OpenRead($file)
            try {
                $buffer = New-Object byte[] 65536
                while (($n = $stream.Read($buffer,0,$buffer.Length)) -gt 0) { [void]$hash.TransformBlock($buffer,0,$n,$null,0) }
            } finally { $stream.Dispose() }
        }
        [void]$hash.TransformFinalBlock([byte[]]@(),0,0)
        return ([BitConverter]::ToString($hash.Hash)).Replace('-','').ToLowerInvariant()
    } finally { $hash.Dispose() }
}

function Get-RmrDumpFingerprint([string[]]$Paths) {
    $hash = [Security.Cryptography.SHA256]::Create()
    try {
        foreach ($path in $Paths) {
            Assert-Running
            $itemHash = Get-RmrFileHash $path
            $data = [Text.Encoding]::UTF8.GetBytes(([IO.Path]::GetFileName($path) + ':' + $itemHash + "`n"))
            [void]$hash.TransformBlock($data, 0, $data.Length, $null, 0)
        }
        [void]$hash.TransformFinalBlock([byte[]]@(), 0, 0)
        return ([BitConverter]::ToString($hash.Hash)).Replace('-', '').ToLowerInvariant()
    } finally { $hash.Dispose() }
}

function Get-RmrExpectedNamesDigest {
    # Regenerate count and digest after upstream dump-name changes: python -c "import hashlib,pathlib; n=sorted(p.name for p in pathlib.Path('db').glob('*.sql.gz')); print(len(n),hashlib.sha256(('\n'.join(n)+'\n').encode()).hexdigest())"
    return 'e2f70a52416e3d4fc29bb8291e518b3d8eaae44e002fc173aa386b66cbc7a5bb'
}

function Get-RmrImportPaths([string]$Checkout) {
    $paths = [IO.Directory]::GetFiles((Join-Path $Checkout 'db'), '*.sql.gz')
    [Array]::Sort($paths, [StringComparer]::Ordinal)
    $expected = Get-RmrExpectedNamesDigest
    $names = (($paths | ForEach-Object { [IO.Path]::GetFileName($_) }) -join "`n") + "`n"
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $digest = ([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($names)))).Replace('-','').ToLowerInvariant() }
    finally { $sha.Dispose() }
    if ($paths.Length -ne 212 -or $digest -ne $expected) {
        throw 'Database dump files are incomplete or belong to a newer schema. Update the RMR launcher for this checkout before importing; no game data was changed.'
    }
    return $paths
}

function Install-PostgresRuntime([string]$Bin) {
    $runtimeDlls = @('concrt140.dll','msvcp140.dll','msvcp140_1.dll','msvcp140_2.dll','msvcp140_atomic_wait.dll','msvcp140_codecvt_ids.dll','vcruntime140.dll','vcruntime140_1.dll')
    if (Test-Path -LiteralPath (Join-Path $Bin 'vclibs-installed.ok')) { return }
    $archive = Get-Download 'https://download.microsoft.com/download/4/7/c/47c6134b-d61f-4024-83bd-b9c9ea951c25/Microsoft.VCLibs.x64.14.00.Desktop.appx' 'Microsoft.VCLibs.x64.14.00.Desktop.appx' 'b56a9101f706f9d95f815f5b7fa6efbac972e86573d378b96a07cff5540c5961'
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($archive)
    try {
        foreach ($name in $runtimeDlls) {
            $entry = $zip.GetEntry($name)
            if (-not $entry) { throw "Microsoft runtime archive is missing $name." }
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, (Join-Path $Bin $name), $true)
        }
    } finally { $zip.Dispose() }
    [IO.File]::WriteAllText((Join-Path $Bin 'vclibs-installed.ok'), '14.0.33321.0', $Utf8)
}

# ---- router port forwarding (UPnP) ---------------------------------------------------------------

# This PC's address on the network its router is on: an up, non-virtual
# adapter with an IPv4 gateway.
function Get-RmrLanAddress {
    try {
        foreach ($nic in [Net.NetworkInformation.NetworkInterface]::GetAllNetworkInterfaces()) {
            if ($nic.OperationalStatus -ne 'Up' -or $nic.NetworkInterfaceType -eq 'Loopback') { continue }
            if (($nic.Description + ' ' + $nic.Name) -match 'Hyper-V|vEthernet|Virtual|WSL|VMware|VirtualBox') { continue }
            $props = $nic.GetIPProperties()
            $gateway = @($props.GatewayAddresses | Where-Object { $_.Address.AddressFamily -eq [Net.Sockets.AddressFamily]::InterNetwork -and $_.Address.ToString() -ne '0.0.0.0' })
            if ($gateway.Count -eq 0) { continue }
            foreach ($unicast in $props.UnicastAddresses) {
                if ($unicast.Address.AddressFamily -eq [Net.Sockets.AddressFamily]::InterNetwork) { return $unicast.Address.ToString() }
            }
        }
    } catch { }
    return $null
}

# Asks the router (UPnP) to forward TCP $Port to this PC. Returns Ok, a
# message, and the router's public address when it says.
function Set-RmrUpnpForward([int]$Port, [string]$Description) {
    $lan = Get-RmrLanAddress
    if (-not $lan) { return [pscustomobject]@{ Ok = $false; Message = 'No network adapter with a router was found.'; External = '' } }
    try {
        $nat = New-Object -ComObject HNetCfg.NATUPnP
        $maps = $null
        for ($try = 0; $try -lt 4 -and $null -eq $maps; $try++) {
            $maps = $nat.StaticPortMappingCollection
            if ($null -eq $maps) { Start-Sleep -Milliseconds 1500 }
        }
        if ($null -eq $maps) {
            return [pscustomobject]@{ Ok = $false; Message = "Your router did not answer UPnP. Turn UPnP on in its settings, or forward TCP $Port to $lan by hand."; External = '' }
        }
        $existing = $null
        try { $existing = $maps.Item($Port, 'TCP') } catch { $existing = $null }
        if ($null -ne $existing) {
            if ($existing.InternalClient -eq $lan -and [int]$existing.InternalPort -eq $Port) {
                return [pscustomobject]@{ Ok = $true; Message = "Router already forwards TCP $Port to $lan."; External = [string]$existing.ExternalIPAddress }
            }
            try { $maps.Remove($Port, 'TCP') } catch { }
        }
        $mapping = $maps.Add($Port, 'TCP', $Port, $lan, $true, $Description)
        $external = ''
        try { $external = [string]$mapping.ExternalIPAddress } catch { }
        return [pscustomobject]@{ Ok = $true; Message = "Router now forwards TCP $Port to $lan (UPnP)."; External = $external }
    } catch {
        return [pscustomobject]@{ Ok = $false; Message = ("The router refused to forward TCP $Port (" + $_.Exception.Message.Trim() + "). Forward it to $lan by hand."); External = '' }
    }
}

function Remove-RmrUpnpForward([int]$Port) {
    try {
        $nat = New-Object -ComObject HNetCfg.NATUPnP
        $maps = $nat.StaticPortMappingCollection
        if ($null -ne $maps) { $maps.Remove($Port, 'TCP') }
    } catch { }
}

# Red Moon Rising is for your own PC and your local network: localhost, or a
# private IPv4 address (10.x, 172.16-31.x, 192.168.x; 100.64-127.x for a
# private VPN such as Tailscale).
function Test-RmrPrivateIPv4([string]$Address) {
    $ip = $null
    if (-not [Net.IPAddress]::TryParse($Address, [ref]$ip) -or $ip.AddressFamily -ne [Net.Sockets.AddressFamily]::InterNetwork) { return $false }
    $b = $ip.GetAddressBytes()
    return ($b[0] -eq 127 -or $b[0] -eq 10 -or ($b[0] -eq 172 -and $b[1] -ge 16 -and $b[1] -le 31) -or
            ($b[0] -eq 192 -and $b[1] -eq 168) -or ($b[0] -eq 100 -and $b[1] -ge 64 -and $b[1] -le 127))
}

# ---- where the server is hosted ----------------------------------------------------------------

function Test-RmrLocalAddress([string]$Address) {
    try {
        foreach ($nic in [Net.NetworkInformation.NetworkInterface]::GetAllNetworkInterfaces()) {
            foreach ($unicast in $nic.GetIPProperties().UnicastAddresses) {
                if ($unicast.Address.ToString() -eq $Address) { return $true }
            }
        }
    } catch { }
    return $false
}

# 'local' (or blank) for this PC only, else an IPv4 address. Returns the
# address to listen on and the one players connect to.
function Resolve-RmrHost([string]$Text) {
    $Text = ([string]$Text).Trim()
    if (-not $Text -or $Text -in @('local', 'localhost', '127.0.0.1', 'this pc')) {
        return [pscustomobject]@{ Choice = 'local'; Bind = '127.0.0.1'; Advertise = '127.0.0.1'; Note = '' }
    }
    if ($Text -in @('0.0.0.0', 'all', '*')) {
        return [pscustomobject]@{ Choice = '0.0.0.0'; Bind = '0.0.0.0'; Advertise = 'this PC''s network address'; Note = '' }
    }
    $allowAny = ($env:RMR_ALLOW_PUBLIC -eq '1')
    if (-not $allowAny -and -not (Test-RmrPrivateIPv4 $Text)) {
        throw "Use 'local', or this PC's IPv4 address on your local network (such as 192.168.1.20). Red Moon Rising is for your own PC and local network only: $Text"
    }
    $parsed = $null
    if (-not [Net.IPAddress]::TryParse($Text, [ref]$parsed) -or $parsed.AddressFamily -ne [Net.Sockets.AddressFamily]::InterNetwork -or $Text -notmatch '^\d{1,3}(\.\d{1,3}){3}$') {
        throw "Host address must be 'local' or an IPv4 address such as 192.168.1.20: $Text"
    }
    if (Test-RmrLocalAddress $Text) {
        # Listen on every address, so this PC (127.0.0.1) and the local
        # network both reach it; players are told $Text.
        return [pscustomobject]@{ Choice = $Text; Bind = '0.0.0.0'; Advertise = $Text; Note = '' }
    }
    return [pscustomobject]@{ Choice = $Text; Bind = '0.0.0.0'; Advertise = $Text
        Note = "$Text is not an address of this PC. Listening on every address of this PC." }
}

# The launcher window asks before each start; the last answer is offered.
function Read-RmrHostChoice([string]$Current) {
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing
    $form = New-Object Windows.Forms.Form
    $form.Text = 'Red Moon Rising - host the server'
    $form.FormBorderStyle = 'FixedDialog'
    $form.MaximizeBox = $false; $form.MinimizeBox = $false
    $form.StartPosition = 'CenterScreen'
    $form.ClientSize = New-Object Drawing.Size(480, 268)
    $form.BackColor = [Drawing.Color]::FromArgb(14, 18, 23)
    $form.ForeColor = [Drawing.Color]::FromArgb(201, 209, 217)
    $form.Font = New-Object Drawing.Font('Segoe UI', 10)
    $title = New-Object Windows.Forms.Label
    $title.Text = 'Where should this server be hosted?'
    $title.Font = New-Object Drawing.Font('Segoe UI', 12)
    $title.SetBounds(18, 14, 440, 26)
    $local = New-Object Windows.Forms.RadioButton
    $local.Text = 'Locally - only this PC can connect'
    $local.SetBounds(18, 50, 440, 24)
    $remote = New-Object Windows.Forms.RadioButton
    $remote.Text = 'On my local network, at this PC''s IPv4 address:'
    $remote.SetBounds(18, 80, 450, 24)
    $box = New-Object Windows.Forms.TextBox
    $box.SetBounds(38, 108, 220, 26)
    $hint = New-Object Windows.Forms.Label
    # This PC's real network addresses (virtual adapters such as WSL's are
    # not reachable from other devices).
    $real = @()
    try {
        foreach ($nic in [Net.NetworkInformation.NetworkInterface]::GetAllNetworkInterfaces()) {
            if ($nic.OperationalStatus -ne 'Up' -or $nic.NetworkInterfaceType -eq 'Loopback') { continue }
            if (($nic.Description + ' ' + $nic.Name) -match 'Hyper-V|vEthernet|Virtual|WSL|VMware|VirtualBox|Loopback') { continue }
            foreach ($unicast in $nic.GetIPProperties().UnicastAddresses) {
                if ($unicast.Address.AddressFamily -eq [Net.Sockets.AddressFamily]::InterNetwork) { $real += $unicast.Address.ToString() }
            }
        }
    } catch { }
    $lan = if ($real.Count -gt 0) { 'This PC on your network: ' + ($real -join ', ') + '. ' } else { '' }
    $hint.Text = $lan + 'Other computers on your network point Client.bat at it. Red Moon Rising is for your own PC and local network only.'
    $hint.ForeColor = [Drawing.Color]::FromArgb(140, 148, 156)
    $hint.SetBounds(18, 138, 444, 50)
    if (-not $Current -or $Current -eq 'local') { $local.Checked = $true; $box.Text = '' } else { $remote.Checked = $true; $box.Text = $Current }
    $box.Add_TextChanged({ if ($box.Text) { $remote.Checked = $true } })
    $ok = New-Object Windows.Forms.Button
    $ok.Text = 'Start server'
    $ok.DialogResult = 'OK'
    $ok.SetBounds(262, 224, 110, 30)
    $ok.FlatStyle = 'Flat'
    $ok.BackColor = [Drawing.Color]::FromArgb(79, 202, 182)
    $ok.ForeColor = [Drawing.Color]::FromArgb(6, 18, 15)
    $cancel = New-Object Windows.Forms.Button
    $cancel.Text = 'Cancel'
    $cancel.DialogResult = 'Cancel'
    $cancel.SetBounds(378, 224, 84, 30)
    $cancel.FlatStyle = 'Flat'
    $upnp = New-Object Windows.Forms.CheckBox
    $upnp.Text = 'Forward the game port on my router automatically (UPnP)'
    $upnp.Checked = ($env:RMR_ALLOW_PUBLIC -eq '1')
    $upnp.Visible = ($env:RMR_ALLOW_PUBLIC -eq '1')
    $upnp.SetBounds(18, 192, 444, 24)
    $form.Controls.AddRange(@($title, $local, $remote, $box, $hint, $upnp, $ok, $cancel))
    $form.AcceptButton = $ok
    $form.CancelButton = $cancel
    try {
        while ($true) {
            if ($form.ShowDialog() -ne [Windows.Forms.DialogResult]::OK) { return $null }
            $choice = if ($local.Checked) { 'local' } else { $box.Text.Trim() }
            try { return [pscustomobject]@{ Choice = (Resolve-RmrHost $choice).Choice; Upnp = $upnp.Checked } }
            catch { [Windows.Forms.MessageBox]::Show($_.Exception.Message, 'Red Moon Rising', 'OK', 'Warning') | Out-Null }
        }
    } finally { $form.Dispose() }
}

function Start-RmrWorker {
    $lock = $null; $server = $null; $databaseOwned = $false; $exitCode = 0
    $gamePort = 26000; $webPort = 18080; $databasePort = 5432
    $checkoutAlias = $null
    $originalTemp = $env:TEMP; $originalTmp = $env:TMP
    $previousOriginalTemp = $env:RMR_ORIGINAL_TEMP; $previousOriginalTmp = $env:RMR_ORIGINAL_TMP
    $pgBin = Join-Path $Runtime 'postgres-16.15\pgsql\bin'
    $data = Join-Path $Runtime 'data'
    try {
        try { $lock = [IO.File]::Open((Join-Path $Runtime 'launcher.lock'), 'OpenOrCreate', 'ReadWrite', 'None') }
        catch { Write-Host 'RMR is already running from this folder. Use its launcher window.'; return 2 }
        # Native PostgreSQL subprocesses and compiler tools receive short ASCII
        # paths even when the checkout lives under Unicode names or a UNC share.
        try {
            $checkoutAlias = New-NativePathAlias -Path $SourceRoot
        } catch {
            # A short local ASCII checkout already works with these native
            # tools. QueryDosDevice can be denied by a restricted Windows
            # session; keep the launcher usable without touching drive maps.
            if ($SourceRoot.Length -gt 120 -or $SourceRoot -cnotmatch '^[A-Za-z]:\\[\x20-\x7E]+$') { throw }
            Write-Log "Temporary drive alias unavailable; using the local checkout path: $SourceRoot"
            $checkoutAlias = [pscustomobject]@{
                Root = $SourceRoot; Drive = $null; Target = $null
                SourcePath = $SourceRoot; Mutex = $null; Created = $false; Removed = $false
            }
        }
        $script:Root = $checkoutAlias.Root
        $script:Runtime = Join-Path $Root '.rmr'
        $script:Logs = Join-Path $Root 'logs'
        $script:LogPath = Join-Path $Logs 'launcher.log'
        $script:StopPath = Join-Path $Runtime 'stop.request'
        $script:StatusPath = Join-Path $Runtime 'status.json'
        # Keep worker-local values as well: the embedded payload is invoked as a
        # scriptblock, whose parent locals can otherwise shadow script scope.
        $Root = $checkoutAlias.Root
        $Runtime = Join-Path $Root '.rmr'; $Logs = Join-Path $Root 'logs'
        $LogPath = Join-Path $Logs 'launcher.log'
        $StopPath = Join-Path $Runtime 'stop.request'; $StatusPath = Join-Path $Runtime 'status.json'
        $script:Runtime = $Runtime; $script:Logs = $Logs; $script:LogPath = $LogPath
        $script:StopPath = $StopPath; $script:StatusPath = $StatusPath
        Set-Location -LiteralPath $Root
        [Environment]::CurrentDirectory = $Root
        $temporaryDirectory = Join-Path $Runtime 'tmp'
        [IO.Directory]::CreateDirectory($temporaryDirectory) | Out-Null
        $env:RMR_ORIGINAL_TEMP = $originalTemp; $env:RMR_ORIGINAL_TMP = $originalTmp
        $env:TEMP = $temporaryDirectory; $env:TMP = $temporaryDirectory
        $pgBin = Join-Path $Runtime 'postgres-16.15\pgsql\bin'
        $data = Join-Path $Runtime 'data'
        if (Test-Path -LiteralPath $StopPath) {
            $request = [IO.File]::ReadAllText($StopPath).Trim()
            $started = [Diagnostics.Process]::GetCurrentProcess().StartTime.ToUniversalTime()
            if ($request -ne $env:RMR_RUN_ID -and (Get-Item -LiteralPath $StopPath).LastWriteTimeUtc -lt $started) { Remove-Item -LiteralPath $StopPath }
        }
        [IO.File]::WriteAllText($LogPath, ('RMR launcher - ' + (Get-Date -Format s) + "`r`n"), $Utf8)
        Set-Status 'Preparing' 'Checking Windows and preparing your local server' 2
        if (-not [Environment]::Is64BitOperatingSystem -or -not [Environment]::Is64BitProcess) { throw 'RMR requires 64-bit Windows and 64-bit Windows PowerShell.' }
        # rmr.yml is this server's own (accounts included): the server writes a
        # default one when there is none, and it is not shared through git.
        foreach ($required in @('Cargo.lock','Cargo.toml','db\00_shared.sql.gz','crates\eve-server\Cargo.toml')) {
            if (-not (Test-Path -LiteralPath (Join-Path $Root $required))) { throw "Missing $required. Keep start.bat in the extracted RMR repository." }
        }
        $settingsPath = Join-Path $Runtime 'settings.json'
        $settings = [pscustomobject]@{ clientPath = ''; gamePort = 26000; webPort = 18080; databasePort = 5432; webPortDefaultVersion = 0; hostAddress = 'local'; allowPublic = $false }
        if (Test-Path -LiteralPath $settingsPath) {
            $savedSettings = Get-Content -Raw -LiteralPath $settingsPath | ConvertFrom-Json
            foreach ($key in @('clientPath','gamePort','webPort','databasePort','webPortDefaultVersion','hostAddress','allowPublic')) {
                if ($savedSettings.PSObject.Properties[$key]) { $settings.$key = $savedSettings.$key }
            }
        }
        # Move the previous default once; later explicitly selected ports stay put.
        if ([int]$settings.webPortDefaultVersion -lt 2 -and [int]$settings.webPort -eq 8080 -and -not $env:RMR_WEB_PORT) {
            $settings.webPort = 18080
        }
        $selectedPorts = @()
        $gamePort = Select-RmrPort 'RMR_GAME_PORT' $settings.gamePort 26000 $selectedPorts
        $selectedPorts += $gamePort
        $webPort = Select-RmrPort 'RMR_WEB_PORT' $settings.webPort 18080 $selectedPorts
        $selectedPorts += $webPort
        $databasePort = Select-RmrPort 'RMR_DATABASE_PORT' $settings.databasePort 5432 $selectedPorts
        $settings.gamePort = $gamePort; $settings.webPort = $webPort; $settings.databasePort = $databasePort
        $settings.webPortDefaultVersion = 2
        # Where to host: --host or the launcher window's answer, else (asked in
        # an interactive console) the last choice.
        if ($env:RMR_ALLOW_PUBLIC -eq '1') { $settings.allowPublic = $true }
        elseif ([bool]$settings.allowPublic) { $env:RMR_ALLOW_PUBLIC = '1' }
        $hostChoice = if ($env:RMR_HOST) { $env:RMR_HOST } else { [string]$settings.hostAddress }
        if (-not $env:RMR_HOST -and $env:RMR_MODE -eq 'headless' -and $env:RMR_SERVER_ONLY -ne '1' -and [Environment]::UserInteractive) {
            $typed = Read-Host ("Host for this PC only, or on this PC's local-network IPv4 address? [local / address, Enter for '" + $hostChoice + "']")
            if ($typed) { $hostChoice = $typed }
        }
        $hosting = Resolve-RmrHost $hostChoice
        $settings.hostAddress = $hosting.Choice
        if ($hosting.Note) { Write-Log $hosting.Note }
        # Hosting beyond this PC: ask the router to forward the game port
        # (the launcher window's tick box; on by default otherwise).
        if ($hosting.Choice -ne 'local' -and $env:RMR_ALLOW_PUBLIC -eq '1' -and $env:RMR_UPNP -ne '0' -and -not (Test-RmrPrivateIPv4 $hosting.Choice)) {
            Set-Status 'Router' ('Asking your router to forward port ' + $gamePort + ' to this PC') 7
            $forward = Set-RmrUpnpForward $gamePort ('Red Moon Rising game server')
            Write-Log $forward.Message
            if ($forward.Ok -and $forward.External) {
                Write-Log ('Your public address, as the router reports it: ' + $forward.External)
                if ($hosting.Advertise -match '^\d' -and -not (Test-RmrLocalAddress $hosting.Advertise) -and $forward.External -ne $hosting.Advertise) {
                    Write-Log ('Note: you chose ' + $hosting.Advertise + ' but the router says your public address is ' + $forward.External + '.')
                }
            }
        }
        [IO.File]::WriteAllText($settingsPath, ($settings | ConvertTo-Json), $Utf8)
        $script:Status.gamePort = $gamePort; $script:Status.webPort = $webPort
        $script:Status['databasePort'] = $databasePort

        $binaryDir = Join-Path $Root 'target\x86_64-pc-windows-gnu\release'
        $serverExe = Join-Path $binaryDir 'eve-server.exe'
        $fakeClientExe = Join-Path $binaryDir 'fakeclient.exe'
        $buildStamp = Join-Path $Runtime 'build.json'
        $fingerprint = Get-RmrBuildFingerprint $Root
        $needsBuild = $true
        if ((Test-Path -LiteralPath $buildStamp) -and (Test-Path -LiteralPath $serverExe) -and (Test-Path -LiteralPath $fakeClientExe) -and (Test-Path -LiteralPath (Join-Path $binaryDir 'dashboard.exe'))) {
            try {
                $old = Get-Content -LiteralPath $buildStamp -Raw | ConvertFrom-Json
                if ($old.fingerprint -eq $fingerprint -and $old.serverHash -eq (Get-RmrFileHash $serverExe) -and $old.clientHash -eq (Get-RmrFileHash $fakeClientExe)) { $needsBuild = $false }
            } catch { Write-Log 'Build cache could not be verified; rebuilding.' }
        }
        if ($needsBuild) {
        Set-Status 'Toolchain' 'Preparing portable Rust 1.97 (first run downloads the compiler)' 8
        $env:CARGO_HOME = Join-Path $Runtime 'cargo'
        $env:RUSTUP_HOME = Join-Path $Runtime 'rustup'
        $env:RUSTUP_TOOLCHAIN = '1.97.0-x86_64-pc-windows-gnu'
        $env:CARGO_TARGET_DIR = Join-Path $Root 'target'
        $env:CARGO_BUILD_TARGET = 'x86_64-pc-windows-gnu'
        $env:CARGO_TERM_COLOR = 'never'
        $toolDir = Join-Path $Runtime 'tools'
        [IO.Directory]::CreateDirectory($toolDir) | Out-Null
        $rustup = Join-Path $toolDir 'rustup.exe'
        if (-not (Test-Path -LiteralPath $rustup)) {
            $bootstrap = Get-Download 'https://static.rust-lang.org/rustup/archive/1.28.2/x86_64-pc-windows-gnu/rustup-init.exe' 'rustup-init.exe' 'ccbfd951d8024856043b3a0c3903a59f39937bce8d3074768b0d3da55f21e817'
            if ([string]::IsNullOrWhiteSpace($bootstrap) -or -not [IO.File]::Exists($bootstrap)) { throw 'The Rust download did not return a valid file path. Run start.bat again to retry.' }
            [IO.File]::Copy($bootstrap, $rustup, $true)
        }
        $toolchainReady = Join-Path $Runtime 'rust-toolchain-installed.ok'
        if (-not (Test-Path -LiteralPath $toolchainReady) -or -not (Test-Path -LiteralPath (Join-Path $env:RUSTUP_HOME 'toolchains\1.97.0-x86_64-pc-windows-gnu\bin\rustc.exe'))) {
            # Pin the minimal profile archives from Rust's official 1.97.0 manifest.
            # Prefill rustup's verified cache so every large download uses the live
            # byte-based progress display, even though rustup runs without a console.
            $components = @(
                @{ name = 'rustc'; hash = 'c5b717977488eb02b56fa89c3629a7b7ebdaf1c6aecc3a6897cb924aa128c138' }
                @{ name = 'cargo'; hash = '8ef34ae3c90879860103fdb00a0120940f20f3c77b8647372c54aa3a6de21b3d' }
                @{ name = 'rust-std'; hash = '98e6ef703b003b1f7d75c3249a831f4ac07f0deb41b0e1dfe41054847ac2816f' }
                @{ name = 'rust-mingw'; hash = '5d0bb89c6652df6dd759247c3c5c92d8d681f0fe1d7650bc5db9f31f179844ee' }
            )
            $rustupDownloads = Join-Path $env:RUSTUP_HOME 'downloads'
            [IO.Directory]::CreateDirectory($rustupDownloads) | Out-Null
            $componentIndex = 0
            foreach ($component in $components) {
                Assert-Running
                $componentIndex++
                Set-Status 'Downloading Rust' ("Compiler component {0} of {1}: {2}" -f $componentIndex,$components.Count,$component.name) (8 + 2 * $componentIndex)
                $cachedArchive = Join-Path $rustupDownloads $component.hash
                if ([IO.File]::Exists($cachedArchive) -and (Get-RmrFileHash $cachedArchive) -eq $component.hash) { continue }
                $archiveName = $component.name + '-1.97.0-x86_64-pc-windows-gnu.tar.xz'
                $componentArchive = Get-Download ('https://static.rust-lang.org/dist/2026-07-09/' + $archiveName) $archiveName $component.hash
                [IO.File]::Copy($componentArchive, $cachedArchive, $true)
            }
            Set-Status 'Installing Rust' 'Downloads complete. Installing the portable compiler...' 18
            Invoke-Native $rustup @('toolchain','install','1.97.0-x86_64-pc-windows-gnu','--profile','minimal','--no-self-update')
            [IO.File]::WriteAllText($toolchainReady, '1.97.0-x86_64-pc-windows-gnu', $Utf8)
        }
        } else { Write-Log 'Using the verified existing server build.' }
        Set-Status 'PostgreSQL' 'Preparing portable PostgreSQL 16.15' 20
        $pgDir = Join-Path $Runtime 'postgres-16.15'
        if (-not (Test-Path -LiteralPath (Join-Path $pgDir 'installed.ok'))) {
            $archive = Get-Download 'https://get.enterprisedb.com/postgresql/postgresql-16.15-1-windows-x64-binaries.zip' 'postgresql-16.15-1-windows-x64-binaries.zip' '25e6fcdfb8caec38691bf461125e7564508760666f7b8e5dc6a5f0818f58f81e'
            Set-Status 'Extracting PostgreSQL' 'Download verified. Preparing the database tools...' 22
            Expand-PortableZip $archive $pgDir
            [IO.File]::WriteAllText((Join-Path $pgDir 'installed.ok'), '16.15', $Utf8)
        }
        # Install app-local Microsoft runtime DLLs here; no machine-wide installer.
        Install-PostgresRuntime $pgBin
        Invoke-Native (Join-Path $pgBin 'postgres.exe') @('--version')
        if ($needsBuild) {
            Set-Status 'Building' 'Compiling RMR; later launches reuse the verified build' 35
            # Crates that link Windows DLLs by name (windows-sys, getrandom, chrono,
            # parking_lot) need dlltool.exe. The toolchain's rust-mingw component
            # ships one, but rustc only finds it on PATH; a PC without MinGW
            # installed otherwise fails with "error calling dlltool 'dlltool.exe'".
            $selfContained = Join-Path $env:RUSTUP_HOME 'toolchains\1.97.0-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained'
            if (-not [IO.File]::Exists((Join-Path $selfContained 'dlltool.exe'))) {
                Write-Log 'The Rust toolchain has no dlltool.exe; reinstalling its MinGW component.'
                Invoke-Native $rustup @('component','add','rust-mingw','--toolchain','1.97.0-x86_64-pc-windows-gnu')
            }
            if (-not [IO.File]::Exists((Join-Path $selfContained 'dlltool.exe'))) { throw 'The Rust toolchain is missing dlltool.exe. Delete the .rmr\rustup folder and run start.bat again.' }
            # dlltool runs the GNU assembler (as.exe) to make each import
            # library, and the Rust toolchain ships no assembler: on a PC
            # without MinGW the build failed with "dlltool could not create
            # import library ... CreateProcess". vendor\binutils carries GNU
            # binutils' as.exe and a matching dlltool.exe with the DLLs they
            # need (see its README); any that are missing or changed in the
            # tools folder are put back, checked against their checksums.
            # Only these go on PATH, from a folder of their own: with the
            # toolchain's gcc on PATH, rustc takes it for an installed MinGW and
            # stops linking with its own startup files and libraries.
            $dlltoolDir = Join-Path $Runtime 'tools\dlltool'
            [IO.Directory]::CreateDirectory($dlltoolDir) | Out-Null
            $binutilsDir = Join-Path $Root 'vendor\binutils'
            $binutilsFiles = [ordered]@{
                'as.exe' = '5a5049e9834bfcd6efbb6ca0c8e9f3aa87e9c88172f8a26890e09be1e0271143'
                'x86_64-w64-mingw32-as.exe' = '5a5049e9834bfcd6efbb6ca0c8e9f3aa87e9c88172f8a26890e09be1e0271143'
                'dlltool.exe' = '757f050a9ff58bb64d485c9f0117c01dcdd988c23b185bfe4318876757eb105c'
                'libiconv-2.dll' = '132fd32416d08579fd8deaffd7123427efc6598760062abb32f491fc34c5a5bd'
                'libintl-8.dll' = '1136db298a8ad03c07e56e1088a90dd79707cfd669fd6b40eb9a10681b18b0c5'
                'libwinpthread-1.dll' = '1b801c9280f6cad8cb2a026bfa48be8c4bea07a0dc78a820d1591f900ee76f0b'
                'libzstd.dll' = '7443e62559c48209f234795b37b23676eb597ce8bba837af371629c29b698e7d'
                'zlib1.dll' = '199b05c9f6d42e897c906508c9008a5eb15b821fbc884d5d7ae4176e15bce55e'
            }
            foreach ($name in $binutilsFiles.Keys) {
                $want = $binutilsFiles[$name]
                $to = Join-Path $dlltoolDir $name
                if ([IO.File]::Exists($to) -and (Get-RmrFileHash $to).ToLowerInvariant() -eq $want) { continue }
                $from = Join-Path $binutilsDir $name
                if (-not [IO.File]::Exists($from)) { throw "vendor\binutils\$name is missing. Download Red Moon Rising again, or restore that folder from GitHub." }
                if ((Get-RmrFileHash $from).ToLowerInvariant() -ne $want) { throw "vendor\binutils\$name does not match its expected checksum. Restore that folder from GitHub." }
                [IO.File]::Copy($from, $to, $true)
                Write-Log ('Build tools: installed ' + $name)
            }
            $env:PATH = (@($dlltoolDir) + @(($env:PATH -split ';') | Where-Object { $_ -and $_ -ne $selfContained -and $_ -ne $dlltoolDir })) -join ';'
            Invoke-Native $rustup @('run','1.97.0-x86_64-pc-windows-gnu','cargo','build','--release','--locked','-p','eve-server','--bins','--target','x86_64-pc-windows-gnu')
            if (-not (Test-Path -LiteralPath $serverExe) -or -not (Test-Path -LiteralPath $fakeClientExe)) { throw 'The build did not produce both server and fakeclient executables.' }
            $buildRecord = [pscustomobject]@{ fingerprint = $fingerprint; serverHash = (Get-RmrFileHash $serverExe); clientHash = (Get-RmrFileHash $fakeClientExe) }
            [IO.File]::WriteAllText($buildStamp, ($buildRecord | ConvertTo-Json), $Utf8)
        }

        Set-Status 'Database' 'Preparing your private PostgreSQL database' 50
        $passwordPath = Join-Path $Runtime 'database-password.txt'
        if (-not (Test-Path -LiteralPath $passwordPath)) {
            if (Test-Path -LiteralPath (Join-Path $data 'PG_VERSION')) { throw 'Database password file is missing. Restore .rmr/database-password.txt from your backup.' }
            $random = New-Object byte[] 32
            $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
            try { $rng.GetBytes($random) } finally { $rng.Dispose() }
            [IO.File]::WriteAllText($passwordPath, ([BitConverter]::ToString($random).Replace('-','').ToLowerInvariant()), $Utf8)
        }
        $password = [IO.File]::ReadAllText($passwordPath).Trim()
        # Existing PostgreSQL client/service settings must not redirect or alter
        # the private cluster. These changes affect only this worker and its children.
        foreach ($variable in @(Get-ChildItem Env: | Where-Object { $_.Name -like 'PG*' })) {
            [Environment]::SetEnvironmentVariable($variable.Name, $null, 'Process')
        }
        $env:PGPASSWORD = $password
        $env:PGCLIENTENCODING = 'UTF8'
        $env:PGCONNECT_TIMEOUT = '5'
        if (-not (Test-Path -LiteralPath (Join-Path $data 'PG_VERSION'))) {
            if (Test-Path -LiteralPath $data) { throw 'Incomplete data folder found. Preserve it and move it aside before retrying.' }
            $staging = Join-Path $Runtime ('data-init-' + [Guid]::NewGuid().ToString('N'))
            Invoke-Native (Join-Path $pgBin 'initdb.exe') @('-D',$staging,'-U','rmr','--pwfile',$passwordPath,'--encoding=UTF8','--locale=C','--auth-host=scram-sha-256','--auth-local=scram-sha-256')
            # Both absolute paths are fixed children of this launcher's private runtime.
            [IO.Directory]::Move($staging, $data)
        }
        if ([IO.File]::ReadAllText((Join-Path $data 'PG_VERSION')).Trim() -ne '16') { throw 'This launcher requires a PostgreSQL 16 data directory.' }
        $pgCtl = Join-Path $pgBin 'pg_ctl.exe'
        if (Test-OwnedPostgres $data (Join-Path $pgBin 'postgres.exe')) {
            Invoke-Native $pgCtl @('status','-D',$data)
            $databaseOwned = $true
            Invoke-Native $pgCtl @('stop','-D',$data,'-m','fast','-w','-t','60') -Timeout 90
            $databaseOwned = $false
        }
        if (-not (Test-FreePort $databasePort)) { throw "PostgreSQL port $databasePort became busy; retry to choose another port." }
        $databaseOwned = $true
        $pgOptions = '-h 127.0.0.1 -p ' + $databasePort
        Invoke-Native $pgCtl @('start','-D',$data,'-l',(Join-Path $Logs 'postgres.log'),'-o',$pgOptions,'-w','-t','60') -Timeout 90
        $psql = Join-Path $pgBin 'psql.exe'
        $pgArgs = @('-X','-w','-h','127.0.0.1','-p',[string]$databasePort,'-U','rmr','-v','ON_ERROR_STOP=1')
        $exists = Invoke-Native $psql ($pgArgs + @('-d','postgres','-Atc',"SELECT 1 FROM pg_database WHERE datname='rmr'")) -Capture
        if ($exists -ne '1') { Invoke-Native (Join-Path $pgBin 'createdb.exe') @('-h','127.0.0.1','-p',[string]$databasePort,'-U','rmr','-E','UTF8','-T','template0','rmr') }
        $dbArgs = $pgArgs + @('-d','rmr')
        $dumpPaths = $null
        $marker = Invoke-Native $psql ($dbArgs + @('-Atc',"SELECT to_regclass('public.rmr_launcher_imports') IS NOT NULL")) -Capture
        if ($marker -ne 't') {
            $dumpPaths = @(Get-RmrImportPaths $Root)
            $objects = Invoke-Native $psql ($dbArgs + @('-Atc',"SELECT (SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public') + (SELECT count(*) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public')")) -Capture
            if ($objects -ne '0') { throw 'Existing game database has no RMR launcher checkpoint; it was left untouched.' }
            Invoke-Native $psql ($dbArgs + @('-q','-c','CREATE TABLE public.rmr_launcher_imports (name text PRIMARY KEY, imported_at timestamptz NOT NULL DEFAULT now());'))
        }
        $done = Invoke-Native $psql ($dbArgs + @('-Atc','SELECT name FROM public.rmr_launcher_imports ORDER BY name;')) -Capture
        $completed = @{}
        foreach ($name in ($done -split "`n")) { if ($name.Trim()) { $completed[$name.Trim()] = $true } }
        if ($completed.ContainsKey('__rmr_complete__')) { $dumps = @() }
        else {
            if ($null -eq $dumpPaths) { $dumpPaths = @(Get-RmrImportPaths $Root) }
            $dumps = @($dumpPaths | ForEach-Object { Get-Item -LiteralPath $_ })
            Set-Status 'Database' 'Verifying the database snapshot before importing' 50
            $manifestName = '__rmr_manifest__:' + (Get-RmrDumpFingerprint $dumpPaths)
            $namesCheckpoint = '__rmr_names__:' + (Get-RmrExpectedNamesDigest)
            $priorManifests = @($completed.Keys | Where-Object { $_ -like '__rmr_manifest__:*' })
            if ($priorManifests.Count -gt 1 -or ($priorManifests.Count -eq 1 -and $priorManifests[0] -ne $manifestName)) {
                throw 'The database dumps changed during an incomplete import. Restore the original dump snapshot before retrying; existing data was left untouched.'
            }
            if ($priorManifests.Count -eq 0) {
                if ($completed.Count -ne 0) { throw 'An incomplete import has no snapshot checkpoint; existing data was left untouched.' }
                Invoke-Native $psql ($dbArgs + @('-q','-c',"INSERT INTO public.rmr_launcher_imports(name) VALUES ('$manifestName'), ('$namesCheckpoint')"))
            } elseif (-not $completed.ContainsKey($namesCheckpoint)) {
                throw 'The incomplete import has an unexpected dump-name checkpoint; existing data was left untouched.'
            }
        }
        $index = 0
        foreach ($dump in $dumps) {
            Assert-Running
            $index++
            if ($completed.ContainsKey($dump.Name)) { continue }
            Set-Status 'Importing universe' ("{0}/{1} - {2}" -f $index,$dumps.Count,$dump.Name) (50 + [int](35 * $index / $dumps.Count))
            $sqlPath = Join-Path $Runtime 'import.sql'
            $inputFile = [IO.File]::OpenRead($dump.FullName)
            try {
                if ($inputFile.Length -lt 18) { throw "Invalid gzip dump: $($dump.Name)" }
                $header = New-Object byte[] 2
                if ($inputFile.Read($header,0,2) -ne 2 -or $header[0] -ne 0x1f -or $header[1] -ne 0x8b) { throw "Invalid gzip header: $($dump.Name)" }
                $footer = New-Object byte[] 4
                [void]$inputFile.Seek(-4, [IO.SeekOrigin]::End)
                if ($inputFile.Read($footer,0,4) -ne 4) { throw "Missing gzip footer: $($dump.Name)" }
                $expectedSize = [BitConverter]::ToUInt32($footer,0)
                $inputFile.Position = 0
                $gzip = New-Object IO.Compression.GzipStream($inputFile, [IO.Compression.CompressionMode]::Decompress)
                try {
                    $outputFile = [IO.File]::Create($sqlPath)
                    try {
                        $gzip.CopyTo($outputFile)
                        if ($outputFile.Length -eq 0 -or ($outputFile.Length % 4294967296) -ne $expectedSize) { throw "Truncated gzip dump: $($dump.Name)" }
                    } finally { $outputFile.Dispose() }
                } finally { $gzip.Dispose() }
            } finally { $inputFile.Dispose() }
            $safeName = $dump.Name.Replace("'", "''")
            [IO.File]::AppendAllText($sqlPath, "`nINSERT INTO public.rmr_launcher_imports(name) VALUES ('$safeName');`n", $Utf8)
            # Each dump and its checkpoint commit together, so interrupted setup resumes safely.
            Invoke-Native $psql ($dbArgs + @('-q','-o','NUL','--single-transaction','-f',$sqlPath)) -Timeout 900
        }
        if (-not $completed.ContainsKey('__rmr_complete__')) { Invoke-Native $psql ($dbArgs + @('-q','-c',"INSERT INTO public.rmr_launcher_imports(name) VALUES ('__rmr_complete__')")) }
        $sqlPath = Join-Path $Runtime 'import.sql'
        if (Test-Path -LiteralPath $sqlPath) { Remove-Item -LiteralPath $sqlPath }


        Set-Status 'Starting' 'Loading the universe and starting the game server' 92
        $env:RMR_CONFIG = Join-Path $Root 'rmr.yml'
        # The dashboard's killboard shows portraits from the client's cache.
        if ($settings.PSObject.Properties['clientPath'] -and $settings.clientPath) { $env:RMR_CLIENT_PATH = [string]$settings.clientPath }
        if ($env:RMR_CHECK -eq '1') { $env:RMR_BOTS = '0' }
        $env:RMR_STRINGS = Join-Path $Root 'crates\eve-marshal\tests\data\strings.txt'
        $env:RMR_DATABASE = 'postgres://rmr:' + $password + '@127.0.0.1:' + $databasePort + '/rmr'
        $env:RMR_BIND = $hosting.Bind + ':' + $gamePort
        $env:RMR_WEB = '127.0.0.1:' + $webPort
        $env:RMR_LOG_FILE = Join-Path $Logs 'server.log'
        $env:RMR_DUMP_DIR = Join-Path $Logs 'dumps'
        $env:RMR_SHUTDOWN_FILE = Join-Path $Runtime 'shutdown.request'
        # With a launcher window, a dashboard restart asks the window to do
        # what its Restart server button does. Headless runs restart in place.
        if ($env:RMR_MODE -eq 'worker' -and $env:RMR_UI_PID) {
            $env:RMR_RESTART_FILE = Join-Path $Runtime 'restart.request'
            if ([IO.File]::Exists($env:RMR_RESTART_FILE)) { Remove-Item -LiteralPath $env:RMR_RESTART_FILE -Force }
        } else { $env:RMR_RESTART_FILE = $null }
        # A generic shell RUST_LOG must not override this server's rmr.yml
        # log.level. RMR_LOG remains available as the deliberate override.
        $env:RUST_LOG = $null
        [IO.Directory]::CreateDirectory($env:RMR_DUMP_DIR) | Out-Null
        $binaryDir = Join-Path $Root 'target\x86_64-pc-windows-gnu\release'
        # A dashboard restart ends with the server writing 'restart' to the
        # shutdown file; the launcher then starts it again, keeping PostgreSQL.
        $firstStart = $true
        $restartServer = $true
        while ($restartServer) {
            $restartServer = $false
            if ([IO.File]::Exists($env:RMR_SHUTDOWN_FILE)) { Remove-Item -LiteralPath $env:RMR_SHUTDOWN_FILE -Force }
            $server = New-NativeProcess (Join-Path $binaryDir 'eve-server.exe') @()
            $watch = [Diagnostics.Stopwatch]::StartNew()
            while (-not (Test-RmrOwnedPort $gamePort $server.Process.Id) -or -not (Test-RmrOwnedPort $webPort $server.Process.Id)) {
                Assert-Running; Read-NativeOutput $server
                if ($server.Process.HasExited) { Read-NativeOutput $server; throw "Server exited with code $($server.Process.ExitCode). See logs/launcher.log." }
                if ($watch.Elapsed.TotalSeconds -gt 180) { throw 'Server startup timed out. See logs/server.log.' }
                Start-Sleep -Milliseconds 200
            }
            Assert-Running
            if ($server.Process.HasExited) { throw "The server stopped before setup finished. See logs/server.log." }
            [IO.File]::WriteAllText((Join-Path $Runtime 'setup-complete'), (Get-Date -Format s), $Utf8)
            $script:Status.ready = $true
            Set-Status 'Online' ("Players connect to: {0}:{1}   |   Dashboard: http://127.0.0.1:{2}" -f $hosting.Advertise,$gamePort,$webPort) 100
            if (-not $firstStart) { Write-Log 'Server restarted.' }
            Assert-Running
            $firstStart = $false
            if ($env:RMR_CHECK -eq '1') {
                Set-Status 'Checking' 'Testing the server handshake and login' 100
                $checkOutput = Invoke-Native $fakeClientExe @("127.0.0.1:$gamePort",'--no-cache') -Capture -Timeout 90
                if ($checkOutput -notmatch '(?i)session change' -or $checkOutput -match '(?im)(FAILED|\bFAIL\b|-> refused:|-> no reply|-> no usable reply)') { throw 'Protocol/login check did not complete successfully. See logs/launcher.log.' }
                Write-Log 'Protocol/login check passed.'
            } else {
                # The launcher keeps the server alive until stopped.
                $unclaimedRestart = $null
                $pressedRestart = $false
                while (-not $server.Process.HasExited) {
                    Assert-Running; Read-NativeOutput $server
                    # A dashboard restart is the launcher window's Restart server
                    # button. A current window takes the request itself; for one
                    # opened before it knew how, press that button for it.
                    if ($env:RMR_RESTART_FILE -and [IO.File]::Exists($env:RMR_RESTART_FILE)) {
                        if ($null -eq $unclaimedRestart) { $unclaimedRestart = [Diagnostics.Stopwatch]::StartNew() }
                        elseif (-not $pressedRestart -and $unclaimedRestart.Elapsed.TotalSeconds -ge 1.5) {
                            $pressedRestart = $true
                            if (Invoke-RmrWindowRestart) {
                                Remove-Item -LiteralPath $env:RMR_RESTART_FILE -Force -ErrorAction SilentlyContinue
                                Write-Log 'Dashboard restart: pressed the launcher window''s Restart server button.'
                            }
                        } elseif ($unclaimedRestart.Elapsed.TotalSeconds -ge 6) {
                            Remove-Item -LiteralPath $env:RMR_RESTART_FILE -Force -ErrorAction SilentlyContinue
                            Write-Log 'Dashboard restart: the launcher window did not respond; restarting the game server here without rebuilding.'
                            [IO.File]::WriteAllText($env:RMR_SHUTDOWN_FILE, 'restart-in-place', $Utf8)
                            $unclaimedRestart = $null
                        }
                    } else { $unclaimedRestart = $null; $pressedRestart = $false }
                    Start-Sleep -Milliseconds 100
                }
                Read-NativeOutput $server
                $planned = ''
                if ($server.Process.ExitCode -eq 0 -and $env:RMR_SHUTDOWN_FILE -and [IO.File]::Exists($env:RMR_SHUTDOWN_FILE)) {
                    $planned = [IO.File]::ReadAllText($env:RMR_SHUTDOWN_FILE).Trim()
                }
                if ($planned -eq 'restart' -and $env:RMR_MODE -eq 'worker' -and $env:RMR_UI_PID) {
                    # A dashboard restart is a full launcher restart: the window
                    # starts a fresh worker that rebuilds changed code, redeploys
                    # the client scripts and starts PostgreSQL and the server.
                    Write-Log 'Dashboard restart: game state saved; the launcher will rebuild if needed and start the server again.'
                    $script:Status['restartRequested'] = $true
                    $script:Status.ready = $false
                    Set-Status 'Restarting' 'Restarting Red Moon Rising: stopping the database before starting again' 90
                    $server.Process.Dispose()
                    $server = $null
                } elseif ($planned -eq 'restart' -or $planned -eq 'restart-in-place') {
                    Write-Log 'Scheduled restart: game state saved; starting the server again.'
                    $script:Status.ready = $false
                    Set-Status 'Restarting' 'Restarting the game server' 90
                    $server.Process.Dispose()
                    $server = $null
                    $restartServer = $true
                } elseif ($planned -eq 'downtime') {
                    Write-Log 'Scheduled downtime completed; game state saved.'
                } else {
                    throw "Server exited unexpectedly with code $($server.Process.ExitCode). See logs/server.log."
                }
            }
        }

    } catch [OperationCanceledException] { Write-Log $_.Exception.Message }
    catch {
        $exitCode = 1
        $script:Status.error = $true
        Set-Status 'Needs attention' $_.Exception.Message $script:Status.progress
        Write-Log $_.ScriptStackTrace
    } finally {
        if ($lock) {
            try {
                $script:Stopping = $true
                $script:Status.ready = $false
                if ($server) {
                    if (-not $server.Process.HasExited -and $env:RMR_SHUTDOWN_FILE) {
                        [IO.File]::WriteAllText($env:RMR_SHUTDOWN_FILE, 'save', $Utf8)
                        Write-Log 'Waiting for the game server to save live state...'
                        [void]$server.Process.WaitForExit(25000)
                    }
                    Stop-NativeProcess $server.Process
                    Read-NativeOutput $server
                    $server.Process.Dispose()
                    if ($env:RMR_SHUTDOWN_FILE -and [IO.File]::Exists($env:RMR_SHUTDOWN_FILE)) {
                        Remove-Item -LiteralPath $env:RMR_SHUTDOWN_FILE -Force
                    }
                }
                if ($databaseOwned) {
                    Write-Log 'Stopping this launcher''s PostgreSQL database...'
                    try {
                        if (Test-OwnedPostgres $data (Join-Path $pgBin 'postgres.exe')) {
                            Invoke-Native (Join-Path $pgBin 'pg_ctl.exe') @('stop','-D',$data,'-m','fast','-w','-t','60') -Timeout 90
                        }
                        $databaseOwned = $false
                    }
                    catch { $exitCode = 1; $script:Status.error = $true; Set-Status 'Needs attention' ('Database shutdown failed: ' + $_.Exception.Message) 100 }
                }
                $env:PGPASSWORD = $null; $env:RMR_DATABASE = $null
                $script:Status.stopped = $true
                if (-not $script:Status.error) { Set-Status 'Stopped' 'Server stopped. Your universe is saved for the next launch.' 100 }
                else { [IO.File]::WriteAllText($StatusPath, ($script:Status | ConvertTo-Json -Compress), $Utf8) }
            } finally {
                # Keep the path available if PostgreSQL could not be stopped.
                # A later launcher can reuse it and recover the private cluster.
                if ($databaseOwned -and $checkoutAlias) { $checkoutAlias.Created = $false }
                try {
                    try {
                        Set-Location -LiteralPath $SourceRoot
                        [Environment]::CurrentDirectory = $SourceRoot
                    } catch {
                        Set-Location -LiteralPath $env:SystemRoot
                        [Environment]::CurrentDirectory = $env:SystemRoot
                    }
                    $env:TEMP = $originalTemp; $env:TMP = $originalTmp
                    $env:RMR_ORIGINAL_TEMP = $previousOriginalTemp; $env:RMR_ORIGINAL_TMP = $previousOriginalTmp
                    # UI status uses the original files; remove only the exact
                    # mapping we created, after all native tools have finished.
                    Remove-NativePathAlias $checkoutAlias
                } catch {
                    $exitCode = 1
                    Write-Host ('Temporary drive cleanup needs attention: ' + $_.Exception.Message)
                } finally {
                    $Root = $SourceRoot
                    $Runtime = Join-Path $Root '.rmr'; $Logs = Join-Path $Root 'logs'
                    $LogPath = Join-Path $Logs 'launcher.log'
                    $StopPath = Join-Path $Runtime 'stop.request'; $StatusPath = Join-Path $Runtime 'status.json'
                    $script:Root = $SourceRoot
                    $script:Runtime = Join-Path $Root '.rmr'
                    $script:Logs = Join-Path $Root 'logs'
                    $script:LogPath = Join-Path $Logs 'launcher.log'
                    $script:StopPath = Join-Path $Runtime 'stop.request'
                    $script:StatusPath = Join-Path $Runtime 'status.json'
                    $lock.Dispose()
                }
            }
        }
    }
    return $exitCode
}


# The game client is set up by Client.bat (beside this file), not by the server.

function Start-LauncherUi {
    # Copy these into the function scope so event-handler closures retain them.
    $Root = [string]$Root
    $Runtime = [string]$Runtime
    $Logs = [string]$Logs
    $Self = [string]$Self
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing
    [System.Windows.Forms.Application]::EnableVisualStyles()
    if (-not ('Rmr.LauncherSkyForm' -as [type])) {
        Add-Type -ReferencedAssemblies @('System.dll', 'System.Windows.Forms.dll', 'System.Drawing.dll') -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Windows.Forms;

namespace Rmr {
    internal static class LauncherSky {
        public static void Paint(Graphics graphics, Rectangle bounds, double time, int seed, int count) {
            if (bounds.Width < 1 || bounds.Height < 1) return;
            graphics.SmoothingMode = SmoothingMode.AntiAlias;
            using (var background = new LinearGradientBrush(bounds,
                Color.FromArgb(8, 15, 27), Color.FromArgb(12, 23, 37), 28f))
                graphics.FillRectangle(background, bounds);
            var random = new Random(seed);
            for (int i = 0; i < count; i++) {
                float x = (float)random.NextDouble() * bounds.Width;
                float y = (float)random.NextDouble() * bounds.Height;
                double phase = random.NextDouble() * Math.PI * 2;
                double speed = 0.35 + random.NextDouble() * 0.45;
                float radius = i % 11 == 0 ? 1.5f : 0.7f;
                int alpha = (int)(35 + 105 * (0.5 + 0.5 * Math.Sin(time * speed + phase)));
                using (var star = new SolidBrush(Color.FromArgb(alpha, 165, 216, 230)))
                    graphics.FillEllipse(star, x, y, radius * 2, radius * 2);
                if (i % 19 == 0) {
                    using (var halo = new SolidBrush(Color.FromArgb(alpha / 7, 100, 220, 225)))
                        graphics.FillEllipse(halo, x - 3, y - 3, 8, 8);
                }
            }
        }
    }

    public sealed class LauncherSkyForm : Form {
        private readonly Timer animation;
        private readonly DateTime started = DateTime.UtcNow;
        public LauncherSkyForm() {
            SetStyle(ControlStyles.AllPaintingInWmPaint | ControlStyles.OptimizedDoubleBuffer |
                     ControlStyles.ResizeRedraw, true);
            animation = new Timer();
            animation.Interval = 100;
            animation.Tick += delegate { if (Visible && WindowState != FormWindowState.Minimized) Invalidate(); };
            animation.Start();
        }
        protected override void OnPaintBackground(PaintEventArgs e) {
            LauncherSky.Paint(e.Graphics, ClientRectangle, (DateTime.UtcNow - started).TotalSeconds, 45017, 140);
        }
        protected override void Dispose(bool disposing) {
            if (disposing) { animation.Stop(); animation.Dispose(); }
            base.Dispose(disposing);
        }
    }

    public sealed class LauncherSkyHeader : Panel {
        private readonly Timer animation;
        private readonly DateTime started = DateTime.UtcNow;
        public LauncherSkyHeader() {
            SetStyle(ControlStyles.UserPaint | ControlStyles.AllPaintingInWmPaint |
                     ControlStyles.OptimizedDoubleBuffer | ControlStyles.ResizeRedraw, true);
            animation = new Timer();
            animation.Interval = 100;
            animation.Tick += delegate { if (Visible) Invalidate(true); };
            animation.Start();
        }
        protected override void OnPaintBackground(PaintEventArgs e) {
            LauncherSky.Paint(e.Graphics, ClientRectangle, (DateTime.UtcNow - started).TotalSeconds, 182, 48);
        }
        protected override void Dispose(bool disposing) {
            if (disposing) { animation.Stop(); animation.Dispose(); }
            base.Dispose(disposing);
        }
    }

    public sealed class LauncherProgress : Control {
        private float value;
        public float Value {
            get { return value; }
            set { this.value = Math.Max(0, Math.Min(100, value)); Invalidate(); }
        }
        public LauncherProgress() {
            SetStyle(ControlStyles.UserPaint | ControlStyles.AllPaintingInWmPaint |
                     ControlStyles.OptimizedDoubleBuffer | ControlStyles.ResizeRedraw, true);
            BackColor = Color.FromArgb(18, 29, 43);
            Height = 10;
        }
        protected override void OnPaint(PaintEventArgs e) {
            if (Width < 1 || Height < 1) return;
            e.Graphics.SmoothingMode = SmoothingMode.AntiAlias;
            using (var track = new SolidBrush(Color.FromArgb(35, 51, 67)))
                e.Graphics.FillRectangle(track, ClientRectangle);
            float fill = Width * value / 100f;
            if (fill > 0) {
                using (var accent = new LinearGradientBrush(ClientRectangle,
                    Color.FromArgb(40, 151, 170), Color.FromArgb(112, 237, 213), 0f))
                    e.Graphics.FillRectangle(accent, 0, 0, fill, Height);
                using (var edge = new Pen(Color.FromArgb(120, 209, 255, 246)))
                    e.Graphics.DrawLine(edge, 0, 0, fill, 0);
            }
        }
    }
}
'@
    }

    $firstRun = -not [IO.File]::Exists((Join-Path $Runtime 'setup-complete'))
    $background = [System.Drawing.Color]::FromArgb(8, 15, 27)
    $panelColor = [System.Drawing.Color]::FromArgb(18, 29, 43)
    $muted = [System.Drawing.Color]::FromArgb(161, 180, 198)
    $bright = [System.Drawing.Color]::FromArgb(236, 244, 250)
    $cyan = [System.Drawing.Color]::FromArgb(112, 237, 213)
    $font = New-Object System.Drawing.Font('Segoe UI', 12.5)
    $form = New-Object Rmr.LauncherSkyForm
    $form.Text = 'Red Moon Rising | Preparing your universe'
    $form.StartPosition = 'CenterScreen'
    $workArea = [System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea
    $form.ClientSize = New-Object System.Drawing.Size(([Math]::Min(1240, $workArea.Width - 64)), ([Math]::Min(900, $workArea.Height - 88)))
    $form.MinimumSize = New-Object System.Drawing.Size(([Math]::Min(1040, $workArea.Width - 64)), ([Math]::Min(770, $workArea.Height - 64)))
    $form.AutoScaleMode = 'Dpi'
    $form.BackColor = $background
    $form.ForeColor = $bright
    $form.Font = $font
    $form.Padding = New-Object System.Windows.Forms.Padding(28)

    # Small displays can scroll the complete layout instead of clipping controls.
    $viewport = New-Object System.Windows.Forms.Panel
    $viewport.Dock = 'Fill'
    $viewport.AutoScroll = $true
    $viewport.AutoScrollMinSize = New-Object System.Drawing.Size(930, 730)
    $viewport.BackColor = [System.Drawing.Color]::Transparent
    $form.Controls.Add($viewport)
    $layout = New-Object System.Windows.Forms.TableLayoutPanel
    $layout.Dock = 'Fill'
    $layout.MinimumSize = New-Object System.Drawing.Size(930, 730)
    $layout.ColumnCount = 1
    $layout.RowCount = 6
    $layout.BackColor = [System.Drawing.Color]::Transparent
    $layout.ColumnStyles.Add((New-Object System.Windows.Forms.ColumnStyle('Percent', 100))) | Out-Null
    foreach ($height in @(112, 250, 42)) {
        $layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle('Absolute', $height))) | Out-Null
    }
    $layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle('Percent', 100))) | Out-Null
    $layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle('Absolute', 78))) | Out-Null
    $layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle('Absolute', 36))) | Out-Null
    $viewport.Controls.Add($layout)

    $header = New-Object Rmr.LauncherSkyHeader
    $header.Dock = 'Fill'
    $header.Margin = New-Object System.Windows.Forms.Padding(0, 0, 0, 12)
    $brand = New-Object System.Windows.Forms.Label
    $brand.Text = 'RED MOON RISING'
    $brand.Font = New-Object System.Drawing.Font('Segoe UI Light', 34)
    $brand.ForeColor = $bright
    $brand.BackColor = [System.Drawing.Color]::Transparent
    $brand.AutoSize = $true
    $brand.Location = New-Object System.Drawing.Point(0, 1)
    $subtitle = New-Object System.Windows.Forms.Label
    $subtitle.Text = 'YOUR OWN CORNER OF NEW EDEN'
    $subtitle.Font = New-Object System.Drawing.Font('Segoe UI', 11)
    $subtitle.ForeColor = $muted
    $subtitle.BackColor = [System.Drawing.Color]::Transparent
    $subtitle.AutoSize = $true
    $subtitle.Location = New-Object System.Drawing.Point(4, 68)
    $editionLabel = New-Object System.Windows.Forms.Label
    $editionLabel.Text = 'EVE 2.10  /  3913'
    $editionLabel.Font = New-Object System.Drawing.Font('Segoe UI', 11)
    $editionLabel.ForeColor = $cyan
    $editionLabel.BackColor = [System.Drawing.Color]::Transparent
    $editionLabel.TextAlign = 'MiddleRight'
    $editionLabel.Dock = 'Right'
    $editionLabel.Width = 265
    $header.Controls.AddRange(@($brand, $subtitle, $editionLabel))
    $layout.Controls.Add($header, 0, 0)

    $statusPanel = New-Object System.Windows.Forms.TableLayoutPanel
    $statusPanel.Dock = 'Fill'
    $statusPanel.BackColor = $panelColor
    $statusPanel.Padding = New-Object System.Windows.Forms.Padding(24, 18, 24, 16)
    $statusPanel.Margin = New-Object System.Windows.Forms.Padding(0)
    $statusPanel.RowCount = 6
    $statusPanel.ColumnCount = 1
    $statusPanel.ColumnStyles.Add((New-Object System.Windows.Forms.ColumnStyle('Percent', 100))) | Out-Null
    foreach ($height in @(24, 45, 57, 33, 10)) {
        $statusPanel.RowStyles.Add((New-Object System.Windows.Forms.RowStyle('Absolute', $height))) | Out-Null
    }
    $statusPanel.RowStyles.Add((New-Object System.Windows.Forms.RowStyle('Percent', 100))) | Out-Null
    $statusEyebrow = New-Object System.Windows.Forms.Label
    $statusEyebrow.Text = if ($firstRun) { 'FIRST LAUNCH' } else { 'WELCOME BACK' }
    $statusEyebrow.Font = New-Object System.Drawing.Font('Segoe UI Semibold', 10.5)
    $statusEyebrow.ForeColor = $cyan
    $statusEyebrow.Dock = 'Fill'
    $statusEyebrow.Margin = New-Object System.Windows.Forms.Padding(0)
    $stageLabel = New-Object System.Windows.Forms.Label
    $stageLabel.Text = 'Preparing your universe'
    $stageLabel.Font = New-Object System.Drawing.Font('Segoe UI Semibold', 23)
    $stageLabel.Dock = 'Fill'
    $stageLabel.AutoEllipsis = $true
    $stageLabel.Margin = New-Object System.Windows.Forms.Padding(0)
    $messageLabel = New-Object System.Windows.Forms.Label
    $messageLabel.Text = if ($firstRun) { 'We will download the tools, build your server and prepare its database automatically.' } else { 'Checking your saved setup and getting everything ready for launch.' }
    $messageLabel.ForeColor = $muted
    $messageLabel.Dock = 'Fill'
    $messageLabel.AutoEllipsis = $true
    $messageLabel.Margin = New-Object System.Windows.Forms.Padding(0, 5, 0, 0)
    $progressHeading = New-Object System.Windows.Forms.TableLayoutPanel
    $progressHeading.Dock = 'Fill'
    $progressHeading.ColumnCount = 2
    $progressHeading.RowCount = 1
    $progressHeading.ColumnStyles.Add((New-Object System.Windows.Forms.ColumnStyle('Percent', 100))) | Out-Null
    $progressHeading.ColumnStyles.Add((New-Object System.Windows.Forms.ColumnStyle('Absolute', 180))) | Out-Null
    $progressHeading.Margin = New-Object System.Windows.Forms.Padding(0)
    $progressCaption = New-Object System.Windows.Forms.Label
    $progressCaption.Text = 'OVERALL SETUP'
    $progressCaption.Dock = 'Fill'
    $progressCaption.Font = New-Object System.Drawing.Font('Segoe UI Semibold', 10.5)
    $progressCaption.ForeColor = $muted
    $progressCaption.TextAlign = 'MiddleLeft'
    $progressCaption.AutoEllipsis = $true
    $progressCaption.Margin = New-Object System.Windows.Forms.Padding(0)
    $progressPercent = New-Object System.Windows.Forms.Label
    $progressPercent.Text = '0%'
    $progressPercent.Dock = 'Fill'
    $progressPercent.Font = New-Object System.Drawing.Font('Segoe UI Semibold', 15)
    $progressPercent.ForeColor = $cyan
    $progressPercent.TextAlign = 'MiddleRight'
    $progressPercent.Margin = New-Object System.Windows.Forms.Padding(0)
    $progressHeading.Controls.Add($progressCaption, 0, 0)
    $progressHeading.Controls.Add($progressPercent, 1, 0)
    $progressTrack = New-Object Rmr.LauncherProgress
    $progressTrack.Dock = 'Fill'
    $progressTrack.Margin = New-Object System.Windows.Forms.Padding(0)
    $setupHint = New-Object System.Windows.Forms.Label
    $setupHint.Text = if ($firstRun) { 'First setup can take a few minutes while tools download and the database is prepared.' } else { 'Your saved universe and installed tools will be reused.' }
    $setupHint.Font = New-Object System.Drawing.Font('Segoe UI', 11.5)
    $setupHint.ForeColor = $muted
    $setupHint.Dock = 'Fill'
    $setupHint.Margin = New-Object System.Windows.Forms.Padding(0, 12, 0, 0)
    $setupHint.AutoEllipsis = $true
    $statusPanel.Controls.Add($statusEyebrow, 0, 0)
    $statusPanel.Controls.Add($stageLabel, 0, 1)
    $statusPanel.Controls.Add($messageLabel, 0, 2)
    $statusPanel.Controls.Add($progressHeading, 0, 3)
    $statusPanel.Controls.Add($progressTrack, 0, 4)
    $statusPanel.Controls.Add($setupHint, 0, 5)
    $layout.Controls.Add($statusPanel, 0, 1)

    $logHeading = New-Object System.Windows.Forms.Label
    $logHeading.Dock = 'Fill'
    $logHeading.Text = 'ACTIVITY  /  LIVE SERVER OUTPUT'
    $logHeading.ForeColor = $muted
    $logHeading.Font = New-Object System.Drawing.Font('Segoe UI Semibold', 10.5)
    $logHeading.BackColor = [System.Drawing.Color]::Transparent
    $logHeading.TextAlign = 'MiddleLeft'
    $logHeading.Margin = New-Object System.Windows.Forms.Padding(0)
    $layout.Controls.Add($logHeading, 0, 2)

    $consoleFrame = New-Object System.Windows.Forms.Panel
    $consoleFrame.Dock = 'Fill'
    $consoleFrame.BackColor = [System.Drawing.Color]::FromArgb(8, 15, 24)
    $consoleFrame.Padding = New-Object System.Windows.Forms.Padding(16, 12, 16, 12)
    $consoleFrame.Margin = New-Object System.Windows.Forms.Padding(0)
    $consoleFrame.MinimumSize = New-Object System.Drawing.Size(0, 155)
    $console = New-Object System.Windows.Forms.RichTextBox
    $console.Dock = 'Fill'
    $console.BackColor = $consoleFrame.BackColor
    $console.ForeColor = [System.Drawing.Color]::FromArgb(188, 209, 222)
    $console.Font = New-Object System.Drawing.Font('Consolas', 11.5)
    $console.BorderStyle = 'None'
    $console.ReadOnly = $true
    $console.DetectUrls = $false
    $console.WordWrap = $true
    $console.HideSelection = $false
    $console.Margin = New-Object System.Windows.Forms.Padding(0)
    $console.Text = "Welcome to your universe.`r`nPreparing Red Moon Rising...`r`n"
    $consoleFrame.Controls.Add($console)
    $layout.Controls.Add($consoleFrame, 0, 3)

    $footer = New-Object System.Windows.Forms.TableLayoutPanel
    $footer.Dock = 'Fill'
    $footer.Margin = New-Object System.Windows.Forms.Padding(0, 20, 0, 0)
    $footer.BackColor = [System.Drawing.Color]::Transparent
    $footer.ColumnCount = 5
    $footer.RowCount = 1
    foreach ($width in @(25, 20, 16, 21, 18)) {
        $footer.ColumnStyles.Add((New-Object System.Windows.Forms.ColumnStyle('Percent', $width))) | Out-Null
    }
    $connectionLabel = New-Object System.Windows.Forms.Label
    $connectionLabel.Dock = 'Fill'
    $connectionLabel.ForeColor = $muted
    $connectionLabel.TextAlign = 'MiddleLeft'
    $connectionLabel.BackColor = [System.Drawing.Color]::Transparent
    $connectionLabel.Font = New-Object System.Drawing.Font('Segoe UI', 11)
    $connectionLabel.Text = 'LOCAL UNIVERSE  /  Setup and game data stay on your computer'
    $connectionLabel.AutoEllipsis = $true
    $connectionLabel.Margin = New-Object System.Windows.Forms.Padding(0)

    $clientButton = New-Object System.Windows.Forms.Button
    $dashboardButton = New-Object System.Windows.Forms.Button
    $logsButton = New-Object System.Windows.Forms.Button
    $restartButton = New-Object System.Windows.Forms.Button
    $stopButton = New-Object System.Windows.Forms.Button
    foreach ($button in @($clientButton, $dashboardButton, $logsButton, $restartButton, $stopButton)) {
        $button.Dock = 'Fill'
        $button.FlatStyle = 'Flat'
        $button.FlatAppearance.BorderSize = 1
        $button.FlatAppearance.BorderColor = [System.Drawing.Color]::FromArgb(43, 65, 81)
        $button.FlatAppearance.MouseOverBackColor = [System.Drawing.Color]::FromArgb(31, 49, 65)
        $button.FlatAppearance.MouseDownBackColor = [System.Drawing.Color]::FromArgb(39, 65, 81)
        $button.BackColor = $panelColor
        $button.ForeColor = $bright
        $button.Cursor = [System.Windows.Forms.Cursors]::Hand
        $button.Font = New-Object System.Drawing.Font('Segoe UI Semibold', 12.5)
        $button.Margin = New-Object System.Windows.Forms.Padding(0, 0, 12, 0)
        $button.UseVisualStyleBackColor = $false
    }
    $clientButton.Text = 'Preparing...'
    $clientButton.Enabled = $false
    $clientButton.BackColor = [System.Drawing.Color]::FromArgb(36, 43, 52)
    $clientButton.ForeColor = [System.Drawing.Color]::FromArgb(137, 148, 161)
    $clientButton.FlatAppearance.BorderSize = 1
    $clientButton.FlatAppearance.MouseOverBackColor = [System.Drawing.Color]::FromArgb(150, 248, 229)
    $clientButton.FlatAppearance.MouseDownBackColor = [System.Drawing.Color]::FromArgb(79, 202, 182)
    $dashboardButton.Text = 'Preparing...'
    $dashboardButton.Enabled = $false
    $dashboardButton.BackColor = [System.Drawing.Color]::FromArgb(36, 43, 52)
    $dashboardButton.ForeColor = [System.Drawing.Color]::FromArgb(137, 148, 161)
    $logsButton.Text = 'Open logs'
    $restartButton.Text = 'Restart server'
    $restartButton.Enabled = $false
    $stopButton.Text = 'Stop server'
    $stopButton.ForeColor = [System.Drawing.Color]::FromArgb(255, 174, 156)
    $stopButton.Margin = New-Object System.Windows.Forms.Padding(0)
    $footer.Controls.Add($clientButton, 0, 0)
    $footer.Controls.Add($dashboardButton, 1, 0)
    $footer.Controls.Add($logsButton, 2, 0)
    $footer.Controls.Add($restartButton, 3, 0)
    $footer.Controls.Add($stopButton, 4, 0)
    $layout.Controls.Add($footer, 0, 4)
    $layout.Controls.Add($connectionLabel, 0, 5)
    $tooltip = New-Object System.Windows.Forms.ToolTip
    # Each start from this window asks where to host, offering the last answer.
    $readHostChoice = ${function:Read-RmrHostChoice}
    $savedHost = 'local'
    try {
        $savedSettingsPath = Join-Path $Runtime 'settings.json'
        if ([IO.File]::Exists($savedSettingsPath)) {
            $savedJson = [IO.File]::ReadAllText($savedSettingsPath) | ConvertFrom-Json
            if ($savedJson.PSObject.Properties['hostAddress'] -and $savedJson.hostAddress) { $savedHost = [string]$savedJson.hostAddress }
            if ($savedJson.PSObject.Properties['allowPublic'] -and $savedJson.allowPublic) { $env:RMR_ALLOW_PUBLIC = '1' }
        }
    } catch { }
    # Once per port: an inbound rule for the game port (asks for administrator).
    $openFirewall = {
        try {
            $gamePort = 26000
            try { $gamePort = [int]([IO.File]::ReadAllText((Join-Path $Runtime 'settings.json')) | ConvertFrom-Json).gamePort } catch { }
            if ($gamePort -lt 1) { $gamePort = 26000 }
            $ruleName = 'Red Moon Rising game server (TCP ' + $gamePort + ')'
            if (Get-NetFirewallRule -DisplayName $ruleName -ErrorAction SilentlyContinue) { return }
            $answer = [System.Windows.Forms.MessageBox]::Show(('Let other computers on your local network reach the server through Windows Firewall on port ' + $gamePort + '? Windows will ask for administrator permission.'), 'Red Moon Rising', 'YesNo', 'Question')
            if ($answer -ne 'Yes') { return }
            $profiles = if ($env:RMR_ALLOW_PUBLIC -eq '1') { 'Any' } else { 'Private,Domain' }
            $command = "New-NetFirewallRule -DisplayName '$ruleName' -Direction Inbound -Protocol TCP -LocalPort $gamePort -Action Allow -Profile $profiles | Out-Null"
            Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoProfile', '-Command', $command) -Verb RunAs -Wait -WindowStyle Hidden
        } catch { }
    }.GetNewClosure()
    # Launch EVE runs Client.bat, which asks which server to connect to.
    $clientLauncher = Join-Path $SourceRoot 'Client.bat'
    $launchClient = {
        param([string]$ClientPath, [int]$GamePort = 0)
        if (-not [IO.File]::Exists($clientLauncher)) { throw 'Client.bat is missing beside Start.bat.' }
        Start-Process -FilePath $clientLauncher -WorkingDirectory ([IO.Path]::GetDirectoryName($clientLauncher)) | Out-Null
    }.GetNewClosure()

    $state = @{
        Process = $null
        Attached = $false
        Ready = $false
        StopRequested = $false
        RestartDeadline = $null
        CloseRequested = $false
        Exited = $false
        LastLogLength = -1L
        LastLogWrite = [DateTime]::MinValue
        LastStatus = ''
        WebPort = 18080
        ClientPath = ''
        ServerOnly = ($env:RMR_NO_CLIENT -eq '1')
        PickerOpen = $false
        LastClientRequest = ''
        FirstRun = $firstRun
        Started = [DateTime]::UtcNow
        RunId = $env:RMR_RUN_ID
        StopPath = (Join-Path $Runtime 'stop.request')
        RestartRequestPath = (Join-Path $Runtime 'restart.request')
        StatusPath = (Join-Path $Runtime 'status.json')
        LogPath = (Join-Path $Logs 'launcher.log')
    }

    $lockPath = Join-Path $Runtime 'launcher.lock'
    $workerLockAvailable = {
        try {
            $probe = [IO.File]::Open($lockPath, 'OpenOrCreate', 'ReadWrite', 'None')
            $probe.Dispose()
            return $true
        } catch { return $false }
    }.GetNewClosure()

    $startWorker = {
        param([bool]$Restart = $false)
        if (-not (& $workerLockAvailable)) {
            throw 'Another Red Moon Rising launcher is already running from this folder. Use its window to manage the server.'
        }

        # Read the current launcher from disk on each start.
        $bootstrap = '[AppContext]::SetSwitch("Switch.System.IO.UseLegacyPathHandling",$false); [AppContext]::SetSwitch("Switch.System.IO.BlockLongPaths",$false); $self = $env:RMR_LAUNCHER; $source = [IO.File]::ReadAllText($self); $parts = $source -split ''(?m)^#==RMR_POWERSHELL==\r?$'', 2; if ($parts.Count -ne 2) { throw "Launcher payload is missing" }; & ([ScriptBlock]::Create($parts[1]))'
        $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($bootstrap))
        $runId = [Guid]::NewGuid().ToString('N')
        $oldMode = $env:RMR_MODE
        $oldLauncher = $env:RMR_LAUNCHER
        $oldUiPid = $env:RMR_UI_PID
        $oldRunId = $env:RMR_RUN_ID
        $oldNoAutoClient = $env:RMR_NO_AUTO_CLIENT
        $oldNoClient = $env:RMR_NO_CLIENT
        try {
            $env:RMR_MODE = 'worker'
            $env:RMR_LAUNCHER = $Self
            $env:RMR_UI_PID = [string]$PID
            $env:RMR_RUN_ID = $runId
            if ($Restart) { $env:RMR_NO_AUTO_CLIENT = '1' }
            if ($state.ServerOnly) { $env:RMR_NO_CLIENT = '1' }
            $powershell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
            $worker = Start-Process -FilePath $powershell -ArgumentList @('-NoLogo', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-EncodedCommand', $encoded) -WorkingDirectory $Root -WindowStyle Hidden -PassThru -ErrorAction Stop
        } finally {
            $env:RMR_MODE = $oldMode
            $env:RMR_LAUNCHER = $oldLauncher
            $env:RMR_UI_PID = $oldUiPid
            $env:RMR_RUN_ID = $oldRunId
            $env:RMR_NO_AUTO_CLIENT = $oldNoAutoClient
            $env:RMR_NO_CLIENT = $oldNoClient
        }

        if ($null -ne $state.Process) { $state.Process.Dispose() }
        $state.Process = $worker
        $state.RunId = $runId
        $state.Attached = $false
        $state.StopRequested = $false
        $state.RestartDeadline = $null
        $state.LastStatus = ''
        $state.LastClientRequest = ''
        $state.ClientPath = ''
        $stopButton.Text = 'Stop server'
        $stopButton.Enabled = $true
    }.GetNewClosure()

    $setReadyState = {
        param([bool]$Ready, [string]$DisabledText = 'Preparing...')
        $state.Ready = $Ready -and -not $state.StopRequested -and -not $state.Exited
        $clientButton.Enabled = $state.Ready
        $dashboardButton.Enabled = $state.Ready
        $restartButton.Enabled = $state.Ready
        if ($state.Ready) {
            $clientButton.Text = 'Launch EVE'
            $clientButton.BackColor = $cyan
            $clientButton.ForeColor = $background
            $clientButton.FlatAppearance.BorderSize = 0
            $clientButton.Cursor = [System.Windows.Forms.Cursors]::Hand
            $dashboardButton.Text = 'Dashboard'
            $dashboardButton.BackColor = $panelColor
            $dashboardButton.ForeColor = $bright
            $dashboardButton.Cursor = [System.Windows.Forms.Cursors]::Hand
        } else {
            foreach ($button in @($clientButton, $dashboardButton)) {
                $button.Text = $DisabledText
                $button.BackColor = [System.Drawing.Color]::FromArgb(36, 43, 52)
                $button.ForeColor = [System.Drawing.Color]::FromArgb(137, 148, 161)
                $button.FlatAppearance.BorderSize = 1
                $button.Cursor = [System.Windows.Forms.Cursors]::Default
            }
        }
    }.GetNewClosure()

    $requestStop = {
        param([bool]$Restart = $false)
        if ($state.StopRequested -or $state.Exited) { return }
        try {
            [System.IO.File]::WriteAllText($state.StopPath, $state.RunId, [System.Text.Encoding]::ASCII)
            $state.StopRequested = $true
            & $setReadyState $false 'Stopping...'
            $stageLabel.Text = 'Stopping Red Moon Rising'
            $form.Text = 'Red Moon Rising | Stopping'
            $statusEyebrow.Text = 'SAFE SHUTDOWN'
            $messageLabel.Text = 'Finishing the current operation and shutting down the database. Please keep this window open.'
            $setupHint.Text = 'Your universe will be kept for the next launch.'
            $stopButton.Enabled = $false
            $stopButton.Text = 'Stopping...'
            if ($Restart) {
                $state.RestartDeadline = [DateTime]::UtcNow.AddSeconds(150)
                $stageLabel.Text = 'Restarting Red Moon Rising'
                $form.Text = 'Red Moon Rising | Restarting'
                $setupHint.Text = 'The server will start again after shutdown completes.'
            }
        } catch {
            $messageLabel.Text = 'Could not request shutdown: ' + $_.Exception.Message
        }
    }.GetNewClosure()

    $clientButton.Add_Click({
        if (-not $state.Ready -or $state.StopRequested -or $state.Exited) { return }
        if ($null -eq $state.Process -or $state.Process.HasExited) { & $setReadyState $false 'Server offline'; return }
        try {
            if ($null -eq $launchClient) { throw 'The client launcher is unavailable. Restart start.bat.' }
            & $launchClient -ClientPath $state.ClientPath | Out-Null
        } catch { $messageLabel.Text = 'Could not launch EVE: ' + $_.Exception.Message }
    }.GetNewClosure())
    $dashboardButton.Add_Click({
        if (-not $state.Ready -or $state.StopRequested -or $state.Exited) { return }
        if ($null -eq $state.Process -or $state.Process.HasExited) { & $setReadyState $false 'Server offline'; return }
        try { Start-Process ('http://127.0.0.1:{0}/' -f $state.WebPort) | Out-Null }
        catch { $messageLabel.Text = 'Could not open your browser: ' + $_.Exception.Message }
    }.GetNewClosure())
    $logsButton.Add_Click({
        try { Start-Process explorer.exe -ArgumentList ('"{0}"' -f $Logs) | Out-Null }
        catch { $messageLabel.Text = 'Logs are saved in ' + $Logs }
    }.GetNewClosure())
    $stopButton.Add_Click({
        if ($state.Exited) { $form.Close() } else { & $requestStop }
    }.GetNewClosure())
    $restartButton.Add_Click({
        if ($state.Ready) { & $requestStop $true }
    }.GetNewClosure())
    $form.Add_FormClosing({
        param($sender, $eventArgs)
        if (-not $state.Exited -and $null -ne $state.Process) {
            $eventArgs.Cancel = $true
            $state.CloseRequested = $true
            $state.RestartDeadline = $null
            & $requestStop
        }
    }.GetNewClosure())

    $timer = New-Object System.Windows.Forms.Timer
    $timer.Interval = 250
    $timer.Add_Tick({
        # The web dashboard's Restart: exactly this window's Restart server button.
        try {
            if ([System.IO.File]::Exists($state.RestartRequestPath)) {
                [System.IO.File]::Delete($state.RestartRequestPath)
                if (-not $state.StopRequested -and -not $state.Exited -and -not $state.Attached) {
                    & $requestStop $true
                    $statusEyebrow.Text = 'DASHBOARD RESTART'
                }
            }
        } catch { }
        # Open with sharing: worker output and atomic status updates must never block the UI.
        try {
            if ([System.IO.File]::Exists($state.StatusPath)) {
                $stream = New-Object System.IO.FileStream($state.StatusPath, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, ([System.IO.FileShare]::ReadWrite -bor [System.IO.FileShare]::Delete))
                try {
                    $reader = New-Object System.IO.StreamReader($stream, [System.Text.Encoding]::UTF8)
                    try { $raw = $reader.ReadToEnd() } finally { $reader.Dispose() }
                } finally { $stream.Dispose() }
                if ($raw -and $raw -ne $state.LastStatus) {
                    $status = $raw | ConvertFrom-Json
                    if (-not $status.PSObject.Properties['runId'] -or [string]$status.runId -ne [string]$state.RunId) {
                        & $setReadyState $false
                        $raw = $null
                    }
                }
                if ($raw -and $raw -ne $state.LastStatus) {
                    $state.LastStatus = $raw
                    if (-not $state.StopRequested -and $status.PSObject.Properties['restartRequested'] -and $status.restartRequested) {
                        # Restart requested from the web dashboard.
                        $state.StopRequested = $true
                        $state.RestartDeadline = [DateTime]::UtcNow.AddSeconds(150)
                        $statusEyebrow.Text = 'DASHBOARD RESTART'
                        $form.Text = 'Red Moon Rising | Restarting'
                        $setupHint.Text = 'The server will start again after shutdown completes.'
                        $stopButton.Enabled = $false
                        $stopButton.Text = 'Restarting...'
                    }
                    if ($state.StopRequested -and $null -ne $state.RestartDeadline) {
                        $stageLabel.Text = 'Restarting Red Moon Rising'
                        $messageLabel.Text = 'Saving the universe and stopping the database. The launcher then rebuilds any changed code and starts the server again.'
                        $connectionLabel.Text = 'RESTARTING'
                    }
                    if (-not $state.StopRequested) {
                        if ($status.stage) { $stageLabel.Text = [string]$status.stage }
                        if ($status.message) { $messageLabel.Text = [string]$status.message }
                    }
                    $overall = [Math]::Max(0, [Math]::Min(100, [double]$status.progress))
                    $downloadActive = $status.PSObject.Properties['downloadPercent'] -and
                        $null -ne $status.downloadPercent -and [double]$status.downloadPercent -ge 0 -and
                        -not $status.ready -and -not $status.error -and -not $state.StopRequested
                    $phaseActive = $status.PSObject.Properties['phasePercent'] -and
                        $null -ne $status.phasePercent -and [double]$status.phasePercent -ge 0 -and
                        -not $status.ready -and -not $status.error -and -not $state.StopRequested
                    if ($downloadActive) {
                        $filePercent = [Math]::Max(0, [Math]::Min(100, [double]$status.downloadPercent))
                        $progressTrack.Value = [single]$filePercent
                        $progressCaption.Text = 'FILE DOWNLOAD   /   OVERALL SETUP ' + ('{0:0}%' -f $overall)
                        $progressPercent.Text = '{0:0}%' -f $filePercent
                    } elseif ($phaseActive) {
                        $phasePercent = [Math]::Max(0, [Math]::Min(100, [double]$status.phasePercent))
                        $phaseTitle = 'PREPARING'
                        if ($status.PSObject.Properties['phaseLabel'] -and $status.phaseLabel) { $phaseTitle = [string]$status.phaseLabel }
                        $progressTrack.Value = [single]$phasePercent
                        $progressCaption.Text = $phaseTitle + '   /   OVERALL SETUP ' + ('{0:0}%' -f $overall)
                        $progressPercent.Text = '{0:0}%' -f $phasePercent
                    } else {
                        $progressTrack.Value = [single]$overall
                        $progressCaption.Text = 'OVERALL SETUP'
                        $progressPercent.Text = '{0:0}%' -f $overall
                    }
                    if ($status.message) { $tooltip.SetToolTip($messageLabel, [string]$status.message) }
                    if ($status.webPort) { $state.WebPort = [int]$status.webPort }
                    if ($status.PSObject.Properties['clientPath'] -and $status.clientPath) {
                        $state.ClientPath = [string]$status.clientPath
                        $tooltip.SetToolTip($clientButton, $state.ClientPath)
                        $tooltip.SetToolTip($connectionLabel, $state.ClientPath)
                    }
                    $verifiedReady = [bool]$status.ready -and -not [bool]$status.error -and -not [bool]$status.stopped -and
                        -not $state.StopRequested -and -not $state.Exited -and
                        $null -ne $state.Process -and -not $state.Process.HasExited
                    $disabledText = if ($state.StopRequested) { 'Stopping...' } elseif ($status.error -or $status.stopped) { 'Server offline' } else { 'Preparing...' }
                    & $setReadyState $verifiedReady $disabledText
                    if ($state.Ready) {
                        $stageLabel.ForeColor = $cyan
                        $statusEyebrow.Text = 'SETUP COMPLETE'
                        $stageLabel.Text = 'READY TO LAUNCH'
                        $messageLabel.Text = 'All setup is complete. Your server is online.'
                        $messageLabel.Text += ' Click Launch EVE below to play.'
                        $tooltip.SetToolTip($messageLabel, $messageLabel.Text)
                        $progressCaption.Text = 'READY TO LAUNCH'
                        $progressPercent.Text = 'READY'
                        $progressTrack.Value = 100
                        $setupHint.Text = 'No more downloads or setup steps remain.'
                        $connectionLabel.Text = 'ONLINE  /  ' + ([string]$status.message -replace '^Players connect to: ([^ ]+).*$', '$1')
                        if ($state.ClientPath) { $connectionLabel.Text += '    |    Client: ' + [IO.Path]::GetFileName($state.ClientPath.TrimEnd('\')) }
                        $form.Text = 'Red Moon Rising | READY TO LAUNCH'
                    }
                    if ($status.error) {
                        & $setReadyState $false 'Server offline'
                        $form.Text = 'Red Moon Rising | Needs attention'
                        $statusEyebrow.Text = 'A LITTLE ATTENTION NEEDED'
                        $stageLabel.ForeColor = [System.Drawing.Color]::FromArgb(255, 151, 130)
                        $messageLabel.Text = [string]$status.message
                        $setupHint.Text = 'The activity log below has the details. Your saved data is kept.'
                        $connectionLabel.Text = 'Review the log below for details'
                    }
                    if ($status.PSObject.Properties['needsClient'] -and $status.needsClient -and -not $state.PickerOpen -and -not $state.StopRequested) {
                        $requestId = 'initial'
                        if ($status.PSObject.Properties['clientRequestId']) { $requestId = [string]$status.clientRequestId }
                        if ($requestId -ne $state.LastClientRequest) {
                            $state.LastClientRequest = $requestId
                            $state.PickerOpen = $true
                            $picker = New-Object System.Windows.Forms.FolderBrowserDialog
                            try {
                                $picker.Description = 'Select the EVE 2.10 build 3913 client folder'
                                $picker.ShowNewFolderButton = $false
                                if ($env:RMR_INVOCATION_DIRECTORY -and (Test-Path -LiteralPath $env:RMR_INVOCATION_DIRECTORY -PathType Container)) { $picker.SelectedPath = $env:RMR_INVOCATION_DIRECTORY } else { $picker.SelectedPath = $Root }
                                $chosenPath = ''
                                if ($picker.ShowDialog($form) -eq [System.Windows.Forms.DialogResult]::OK) {
                                    $chosenPath = $picker.SelectedPath
                                }
                                $choicePath = Join-Path $Runtime 'client-selection.txt'
                                [System.IO.File]::WriteAllText($choicePath, $chosenPath, (New-Object System.Text.UTF8Encoding($false)))
                            } finally {
                                $picker.Dispose()
                                $state.PickerOpen = $false
                            }
                        }
                    }
                }
            }
        } catch { } # A partially written status is retried on the next tick.

        try {
            if ([System.IO.File]::Exists($state.LogPath)) {
                $fileInfo = New-Object System.IO.FileInfo($state.LogPath)
                if ($fileInfo.Length -ne $state.LastLogLength -or $fileInfo.LastWriteTimeUtc -ne $state.LastLogWrite) {
                    $stream = New-Object System.IO.FileStream($state.LogPath, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, ([System.IO.FileShare]::ReadWrite -bor [System.IO.FileShare]::Delete))
                    try {
                        $offset = [Math]::Max(0L, $stream.Length - 65536L)
                        $stream.Position = $offset
                        $reader = New-Object System.IO.StreamReader($stream, [System.Text.Encoding]::UTF8)
                        try {
                            if ($offset -gt 0) { $null = $reader.ReadLine() }
                            $tail = $reader.ReadToEnd()
                        } finally { $reader.Dispose() }
                    } finally { $stream.Dispose() }
                    $console.Text = $tail
                    $console.SelectionStart = $console.TextLength
                    $console.ScrollToCaret()
                    $state.LastLogLength = $fileInfo.Length
                    $state.LastLogWrite = $fileInfo.LastWriteTimeUtc
                }
            }
        } catch { }

        if ($null -ne $state.Process -and -not $state.Exited) {
            $state.Process.Refresh()
            if ($state.Process.HasExited) {
                $restartError = $null
                if ($null -eq $state.RestartDeadline -and -not $state.StopRequested) {
                    # The dashboard's Restart ends the worker with restartRequested;
                    # treat it exactly like this window's Restart server button.
                    try {
                        $exitStatus = [IO.File]::ReadAllText($state.StatusPath) | ConvertFrom-Json
                        if ([string]$exitStatus.runId -eq [string]$state.RunId -and
                            $exitStatus.PSObject.Properties['restartRequested'] -and $exitStatus.restartRequested) {
                            $state.StopRequested = $true
                            $state.RestartDeadline = [DateTime]::UtcNow.AddSeconds(150)
                        }
                    } catch { }
                }
                if ($null -ne $state.RestartDeadline) {
                    try {
                        # An attached launcher watches the game process. The worker
                        # keeps the lock until PostgreSQL and path cleanup finish.
                        if (-not (& $workerLockAvailable)) {
                            if ([DateTime]::UtcNow -lt $state.RestartDeadline) { return }
                            throw 'Timed out waiting for the previous worker to finish shutting down.'
                        }
                        $finalStatus = [IO.File]::ReadAllText($state.StatusPath) | ConvertFrom-Json
                        if ($finalStatus.runId -ne $state.RunId -or -not $finalStatus.stopped -or $finalStatus.error -or
                            (-not $state.Attached -and $state.Process.ExitCode -ne 0)) {
                            throw 'The previous worker did not shut down cleanly. Open the logs for details.'
                        }
                        & $startWorker $true
                        & $setReadyState $false
                        $statusEyebrow.Text = 'STARTING SERVER'
                        $stageLabel.Text = 'Starting Red Moon Rising'
                        $messageLabel.Text = 'Checking for changes and starting the server again.'
                        $progressTrack.Value = 0
                        $progressCaption.Text = 'OVERALL SETUP'
                        $progressPercent.Text = '0%'
                        $setupHint.Text = 'The launcher will stay open while the server starts.'
                        $connectionLabel.Text = 'RESTARTING'
                        return
                    } catch { $restartError = $_.Exception.Message }
                }
                $state.Exited = $true
                & $setReadyState $false 'Server offline'
                $dashboardButton.Enabled = $false
                $clientButton.Enabled = $false
                $stopButton.Enabled = $true
                $stopButton.Text = 'Close'
                $connectionLabel.Text = 'OFFLINE  /  Your universe is saved'
                $form.Text = 'Red Moon Rising | Stopped'
                $statusEyebrow.Text = 'UNTIL NEXT TIME'
                $progressCaption.Text = 'SESSION FINISHED'
                $progressPercent.Text = 'OFFLINE'
                $setupHint.Text = 'Close this window when you are ready.'
                if ($restartError) {
                    $form.Text = 'Red Moon Rising | Needs attention'
                    $statusEyebrow.Text = 'A LITTLE ATTENTION NEEDED'
                    $stageLabel.Text = 'Unable to restart Red Moon Rising'
                    $stageLabel.ForeColor = [System.Drawing.Color]::FromArgb(255, 151, 130)
                    $messageLabel.Text = $restartError
                    $connectionLabel.Text = 'OFFLINE'
                } elseif ($state.Attached -or $state.Process.ExitCode -eq 0) {
                    $stageLabel.Text = 'Red Moon Rising stopped'
                    $messageLabel.Text = 'Your database is kept for next time. Run start.bat whenever you are ready.'
                } else {
                    $statusEyebrow.Text = 'A LITTLE ATTENTION NEEDED'
                    $stageLabel.Text = 'Red Moon Rising needs attention'
                    $stageLabel.ForeColor = [System.Drawing.Color]::FromArgb(255, 151, 130)
                    if (-not $state.LastStatus) { $messageLabel.Text = 'Setup exited unexpectedly. Open the logs for details, then run start.bat again.' }
                }
                if ($state.CloseRequested) { $form.Close() }
            }
        }
    }.GetNewClosure())

    $form.Add_Shown({
        try {
            # Bring the launcher forward once; later client launches may take focus.
            $form.BringToFront()
            $form.Activate()
            $null = [System.IO.Directory]::CreateDirectory($Runtime)
            $null = [System.IO.Directory]::CreateDirectory($Logs)
            # Attach a new launcher window to this checkout's live server.
            # A headless run keeps the worker lock, so a second worker would
            # fail; the UI can monitor the existing server process directly.
            if ([IO.File]::Exists($state.StatusPath)) {
                $online = [IO.File]::ReadAllText($state.StatusPath) | ConvertFrom-Json
                if ($online.ready -and -not $online.stopped -and -not $online.error -and $online.gamePort) {
                    $listener = Get-NetTCPConnection -LocalPort ([int]$online.gamePort) -State Listen -ErrorAction SilentlyContinue | Select-Object -First 1
                    if ($listener) {
                        $running = Get-Process -Id $listener.OwningProcess -ErrorAction SilentlyContinue
                        if ($running -and $running.ProcessName -eq 'eve-server') {
                            $state.Process = $running
                            $state.Attached = $true
                            $state.RunId = [string]$online.runId
                            $state.ServerOnly = -not [bool]$online.clientPath
                            $timer.Start()
                            return
                        }
                    }
                }
            }
            # Ask where to host before each start from this window.
            $hostAnswer = & $readHostChoice $savedHost
            if ($null -eq $hostAnswer) { $state.CloseRequested = $true; $form.Close(); return }
            $env:RMR_HOST = $hostAnswer.Choice
            $env:RMR_UPNP = if ($hostAnswer.Upnp) { '1' } else { '0' }
            # Hosting for other computers: Windows Firewall must let them in.
            if ($hostAnswer.Choice -ne 'local') { & $openFirewall }
            & $startWorker
            $timer.Start()
        } catch {
            $state.Exited = $true
            & $setReadyState $false 'Server offline'
            $statusEyebrow.Text = 'A LITTLE ATTENTION NEEDED'
            $stageLabel.Text = 'Unable to start Red Moon Rising'
            $stageLabel.ForeColor = [System.Drawing.Color]::FromArgb(255, 151, 130)
            $messageLabel.Text = $_.Exception.Message
            $console.Text = $_.Exception.ToString()
            $stopButton.Text = 'Close'
            $connectionLabel.Text = 'OFFLINE'
        }
    }.GetNewClosure())

    try { [System.Windows.Forms.Application]::Run($form) }
    finally {
        $timer.Stop()
        $timer.Dispose()
        $tooltip.Dispose()
        if ($null -ne $state.Process) { $state.Process.Dispose() }
        $form.Dispose()
        $font.Dispose()
    }
}

if ($env:RMR_MODE -eq 'stop') {
    [IO.File]::WriteAllText($StopPath, 'stop', $Utf8)
    Write-Host 'Stop requested. The launcher will save and close its database.'
    exit 0
}
if ($env:RMR_MODE -eq 'ui') {
    try { Start-LauncherUi }
    catch {
        Add-Type -AssemblyName System.Windows.Forms
        [Windows.Forms.MessageBox]::Show($_.Exception.Message, 'Red Moon Rising could not start', 'OK', 'Error') | Out-Null
    }
} else {
    $result = Start-RmrWorker
    exit $result
}
