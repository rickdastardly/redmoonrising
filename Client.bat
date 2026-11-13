@echo off
setlocal DisableDelayedExpansion
rem Red Moon Rising launcher: pick a server, press Play. It keeps a list of
rem servers (this PC, others on your local network), shows each one's status and
rem news, points your EVE 2.10 (build 3913) client at the chosen one and starts
rem it. Players need only this file and their client; Start.bat runs a server.
rem Usage: Client.bat [server-address[:port]] [--client "EVE folder"] [--no-launch]
set "RMR_LAUNCHER=%~f0"
set "RMR_INVOCATION_DIRECTORY=%CD%"
set "RMR_SERVER_ARG="
set "RMR_CLIENT_DIRECTORY="
set "RMR_NO_LAUNCH="
:parse
if "%~1"=="" goto parsed
if /i "%~1"=="--help" goto help
if /i "%~1"=="-h" goto help
if /i "%~1"=="--allow-public" (
    set "RMR_ALLOW_PUBLIC=1"
    shift
    goto parse
)
if /i "%~1"=="--no-launch" (
    set "RMR_NO_LAUNCH=1"
    shift
    goto parse
)
if /i "%~1"=="--client" (
    set "RMR_CLIENT_DIRECTORY=%~2"
    shift
    shift
    goto parse
)
set "RMR_SERVER_ARG=%~1"
shift
goto parse
:help
echo Red Moon Rising launcher
echo Usage: Client.bat [server-address[:port]] [--client "EVE folder"] [--no-launch]
echo With no address it opens the launcher: servers on the left, news, and Play.
echo With an address it points EVE there and starts it straight away (port 26000
echo unless given). --no-launch prepares the client without starting it.
exit /b 0
:parsed
set "RMR_PS=%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe"
if exist "%SystemRoot%\Sysnative\WindowsPowerShell\v1.0\powershell.exe" set "RMR_PS=%SystemRoot%\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
start "" "%RMR_PS%" -NoLogo -NoProfile -ExecutionPolicy Bypass -STA -WindowStyle Hidden -Command "try { $parts = [IO.File]::ReadAllText($env:RMR_LAUNCHER) -split '(?m)^#==RMR_POWERSHELL==\r?$', 2; if ($parts.Count -ne 2) { throw 'Client setup payload is missing.' }; & ([scriptblock]::Create($parts[1])) } catch { Add-Type -AssemblyName System.Windows.Forms; [Windows.Forms.MessageBox]::Show($_.Exception.Message,'Red Moon Rising launcher','OK','Error') | Out-Null; exit 1 }"
exit /b
#==RMR_POWERSHELL==

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$Self = [IO.Path]::GetFullPath($env:RMR_LAUNCHER)
$Root = [IO.Path]::GetDirectoryName($Self)
$SourceRoot = $Root
$Utf8 = New-Object Text.UTF8Encoding($false)
$SettingsPath = Join-Path $Root 'Client.settings.json'

function Get-RmrFileHash([string]$Path) {
    $sha = [Security.Cryptography.SHA256]::Create()
    $stream = [IO.File]::OpenRead($Path)
    try { return [BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-', '') }
    finally { $stream.Dispose(); $sha.Dispose() }
}
function Write-Log([string]$Message) { }
function Assert-Running { }

# Client preparation payload. PowerShell 5.1; no runtime dependency on the source tree.
function Test-RmrClient {
    param([string]$Path)
    try {
        if ([string]::IsNullOrWhiteSpace($Path)) { return $false }
        $dir = [IO.Path]::GetFullPath($Path)
        foreach ($file in @('boot.ini', 'bin\exefile.exe', 'script\sys\autoexec.py', 'script\compiled.code', 'res.stuff')) {
            if (-not (Test-Path -LiteralPath (Join-Path $dir $file) -PathType Leaf)) { return $false }
        }
        if (-not (Test-Path -LiteralPath (Join-Path $dir 'lib') -PathType Container)) { return $false }
        $boot = [IO.File]::ReadAllText((Join-Path $dir 'boot.ini'))
        $main = [regex]::Match($boot, '(?ims)^\s*\[main\]\s*$([\s\S]*?)(?=^\s*\[|\z)')
        if (-not $main.Success) { return $false }
        return ($main.Groups[1].Value -match '(?im)^\s*version\s*=\s*2\.10\s*$' -and
                $main.Groups[1].Value -match '(?im)^\s*build\s*=\s*3913\s*$' -and
                $main.Groups[1].Value -match '(?im)^\s*role\s*=\s*client\s*$')
    } catch { return $false }
}

function Find-RmrClients {
    param([string]$InvocationDirectory, [string]$RepositoryRoot, [string]$SavedClientDirectory)
    if ($SavedClientDirectory -and (Test-RmrClient $SavedClientDirectory)) {
        return @([IO.Path]::GetFullPath($SavedClientDirectory))
    }
    $seen = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    $found = New-Object 'System.Collections.Generic.List[string]'
    $queue = New-Object 'System.Collections.Generic.Queue[object]'
    $roots = @($InvocationDirectory, $RepositoryRoot)
    if ($RepositoryRoot) { $roots += (Split-Path -Parent $RepositoryRoot) }
    foreach ($root in $roots) {
        if (-not $root) { continue }
        try {
            $full = [IO.Path]::GetFullPath($root).TrimEnd('\')
            # Never enumerate a drive root or a user profile as a search root.
            if ($full -eq [IO.Path]::GetPathRoot($full).TrimEnd('\') -or ($env:USERPROFILE -and $full -eq $env:USERPROFILE.TrimEnd('\'))) { continue }
            if (Test-Path -LiteralPath $full -PathType Container) { $queue.Enqueue(@($full, 0)) }
        } catch { }
    }
    $count = 0; $entries = 0
    while ($queue.Count -gt 0 -and $count -lt 3000 -and $entries -lt 3000) {
        $item = $queue.Dequeue(); $dir = [string]$item[0]; $depth = [int]$item[1]
        if (-not $seen.Add($dir)) { continue }
        $count++
        try {
            $info = Get-Item -LiteralPath $dir -Force -ErrorAction Stop
            if (($info.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { continue }
            if (Test-RmrClient $dir) { $found.Add($dir); continue }
            if ($depth -ge 3) { continue }
            foreach ($childPath in [IO.Directory]::EnumerateDirectories($dir)) {
                $entries++
                if ($entries -gt 3000) { break }
                $child = Get-Item -LiteralPath $childPath -Force -ErrorAction Stop
                if ($child.Name -in @('.git', '.local', '.rmr', '.build', 'runtime', 'builds', 'trinity', 'node_modules', 'target', 'cache', 'logs', 'capture', '.rmr-backup')) { continue }
                if (($child.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { continue }
                if ($queue.Count -ge 3000) { break }
                $queue.Enqueue(@($child.FullName, ($depth + 1)))
            }
        } catch { continue }
    }
    return $found.ToArray()
}

function Select-RmrClientFolder {
    param([string]$InvocationDirectory, [string]$Description)
    Add-Type -AssemblyName System.Windows.Forms
    $dialog = New-Object Windows.Forms.FolderBrowserDialog
    try {
        $dialog.Description = $Description
        $dialog.ShowNewFolderButton = $false
        if (Test-Path -LiteralPath $InvocationDirectory -PathType Container) { $dialog.SelectedPath = $InvocationDirectory }
        if ($dialog.ShowDialog() -eq [Windows.Forms.DialogResult]::OK) { return $dialog.SelectedPath }
        return $null
    } finally { $dialog.Dispose() }
}

function Resolve-RmrClient {
    param([string]$InvocationDirectory, [string]$RepositoryRoot, [string]$SavedClientDirectory, [string]$ClientDirectory)
    if ($ClientDirectory) {
        if (-not (Test-RmrClient $ClientDirectory)) { throw "Client directory is not an EVE 2.10 build 3913 client with its required files: $ClientDirectory" }
        return [IO.Path]::GetFullPath($ClientDirectory)
    }
    $matches = @(Find-RmrClients -InvocationDirectory $InvocationDirectory -RepositoryRoot $RepositoryRoot -SavedClientDirectory $SavedClientDirectory)
    if ($matches.Count -eq 1) { return $matches[0] }
    $description = if ($matches.Count -gt 1) { 'Multiple compatible EVE clients were found. Select the client folder to prepare. Cancel to return to server setup.' } else { 'Select your EVE 2.10 build 3913 client folder. Cancel to return to server setup.' }
    $selected = Select-RmrClientFolder -InvocationDirectory $InvocationDirectory -Description $description
    if (-not $selected) { return $null }
    if (-not (Test-RmrClient $selected)) { throw "Selected folder is not an EVE 2.10 build 3913 client with its required files: $selected" }
    return [IO.Path]::GetFullPath($selected)
}

function Save-RmrOriginal {
    param([string]$ClientDirectory, [string]$RelativePath)
    $source = Join-Path $ClientDirectory $RelativePath
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { return }
    $base = Join-Path (Join-Path $ClientDirectory '.rmr-backup') $RelativePath
    $parent = Split-Path -Parent $base
    if (-not (Test-Path -LiteralPath $parent)) { [void](New-Item -ItemType Directory -Path $parent -Force) }
    $sourceHash = Get-RmrFileHash $source
    $destination = $base
    if (Test-Path -LiteralPath $base) {
        if ((Get-RmrFileHash $base) -eq $sourceHash) { return }
        $destination = "$base.$sourceHash"
        if (Test-Path -LiteralPath $destination) { return }
    }
    [IO.File]::Copy($source, $destination, $false)
}

function Write-RmrFile {
    param([string]$ClientDirectory, [string]$RelativePath, [byte[]]$Bytes)
    $path = Join-Path $ClientDirectory $RelativePath
    if (Test-Path -LiteralPath $path -PathType Leaf) {
        $existing = [IO.File]::ReadAllBytes($path)
        if ($existing.Length -eq $Bytes.Length -and [Convert]::ToBase64String($existing) -eq [Convert]::ToBase64String($Bytes)) { return }
        Save-RmrOriginal -ClientDirectory $ClientDirectory -RelativePath $RelativePath
    }
    $parent = Split-Path -Parent $path
    if (-not (Test-Path -LiteralPath $parent)) { [void](New-Item -ItemType Directory -Path $parent -Force) }
    $temporary = Join-Path $parent ('.rmr-' + [Guid]::NewGuid().ToString('N') + '.tmp')
    try {
        [IO.File]::WriteAllBytes($temporary, $Bytes)
        if (Test-Path -LiteralPath $path -PathType Leaf) { [IO.File]::Replace($temporary, $path, [System.Management.Automation.Language.NullString]::Value) }
        else { [IO.File]::Move($temporary, $path) }
    } finally { if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force } }
}

function Prepare-RmrClient {
    param([Parameter(Mandatory=$true)][string]$ClientDirectory, [string]$ServerBind = '127.0.0.1', [int]$ServerPort = 26000)
    if (-not (Test-RmrClient $ClientDirectory)) { throw "Invalid EVE 2.10 build 3913 client: $ClientDirectory" }
    if ($ServerPort -lt 1 -or $ServerPort -gt 65535) { throw 'Server port must be between 1 and 65535.' }
    if ($ServerBind -in @('0.0.0.0', '::', '[::]', '*', '')) { $ServerBind = '127.0.0.1' }
    if ($ServerBind -notmatch '^[A-Za-z0-9.:-]+$') { throw 'Server bind address cannot be used safely in the client launcher.' }
    $ClientDirectory = [IO.Path]::GetFullPath($ClientDirectory)
    $autoPath = Join-Path $ClientDirectory 'script\sys\autoexec.py'
    # Latin-1 round-trips every original byte, including legacy non-ASCII text.
    $encoding = [Text.Encoding]::GetEncoding(28591)
    $auto = $encoding.GetString([IO.File]::ReadAllBytes($autoPath))
    $nl = if ($auto.Contains("`r`n")) { "`r`n" } else { "`n" }
    $anchor = '        if Hiword(d3d9) == 5 and d3d9 >= Long(5, 3, 1, 904): ver = (9, 0, "c") #NT'
    $marker = '        if Hiword(d3d9) >= 6: ver = (9, 0, "c")'
    $knownBlock = @(
        '    d3d9 = GetFileVersion("d3d9.dll")',
        '    if (d3d9):',
        '        ver = (9,0,"")',
        '        if Hiword(d3d9) == 4 and d3d9 >= Long(4, 9, 0, 902): ver = (9, 0, "b") #win9x',
        '        if Hiword(d3d9) == 5 and d3d9 >= Long(5, 3, 1, 902): ver = (9, 0, "b") #NT',
        '',
        '        if Hiword(d3d9) == 4 and d3d9 >= Long(4, 9, 0, 904): ver = (9, 0, "c") #win9x',
        $anchor
    ) -join $nl
    if ([regex]::Matches($auto, [regex]::Escape($knownBlock)).Count -ne 1 -or [regex]::Matches($auto, [regex]::Escape($anchor)).Count -ne 1) { throw 'The DirectX detector in autoexec.py is unfamiliar; no client files were changed.' }
    $markers = [regex]::Matches($auto, [regex]::Escape($marker)).Count
    $patchText = $anchor + $nl + '        # RMR: Modern Windows uses OS DLL versions 6.x / 10.x. Its d3d9' + $nl + '        # runtime supports 9.0c; the old tests misidentified it as 9.0.' + $nl + $marker
    if ($markers -gt 1 -or ($markers -eq 1 -and -not $auto.Contains($patchText))) { throw 'The DirectX detector has an unfamiliar RMR patch; no client files were changed.' }
    if ($markers -eq 0) {
        $auto = $auto.Replace($anchor, $patchText)
    }
    $start = $script:RmrClientStart.Replace('set "RMR_SERVER_HOST=127.0.0.1"', ('set "RMR_SERVER_HOST=' + $ServerBind + '"')).Replace('set "RMR_SERVER_PORT=26000"', ('set "RMR_SERVER_PORT=' + $ServerPort + '"'))
    Write-RmrFile $ClientDirectory 'script\sys\autoexec.py' ($encoding.GetBytes($auto))
    Write-RmrFile $ClientDirectory 'script\rmr_connection.py' ([Text.Encoding]::ASCII.GetBytes($script:RmrClientConnection))
    Write-RmrFile $ClientDirectory 'script\rmr_login.py' ([Text.Encoding]::ASCII.GetBytes($script:RmrClientLogin))
    Write-RmrFile $ClientDirectory 'Start.bat' ([Text.Encoding]::ASCII.GetBytes($start))
    return [pscustomobject]@{ ClientDirectory = $ClientDirectory; Launcher = (Join-Path $ClientDirectory 'Start.bat') }
}

function Prepare-RmrGameClient($Settings) {
    $saved = if ($Settings.PSObject.Properties['clientPath']) { [string]$Settings.clientPath } else { '' }
    if ($env:RMR_CLIENT_DIRECTORY) {
        if (-not (Test-RmrClient $env:RMR_CLIENT_DIRECTORY)) { throw 'The --client path is not a complete EVE 2.10 build 3913 client.' }
        $saved = [IO.Path]::GetFullPath($env:RMR_CLIENT_DIRECTORY)
    }
    # Search the real checkout and invocation folder, not the temporary drive alias.
    $invocation = $env:RMR_INVOCATION_DIRECTORY
    if (-not $invocation) { $invocation = $SourceRoot }
    $matches = @(Find-RmrClients -InvocationDirectory $invocation -RepositoryRoot $SourceRoot -SavedClientDirectory $saved)
    $clientPath = if ($matches.Count -eq 1) { $matches[0] } else { $null }
    while (-not $clientPath) {
        Assert-Running
        if ($env:RMR_MODE -eq 'worker') {
            $selection = Join-Path $Runtime 'client-selection.txt'
            if (Test-Path -LiteralPath $selection) { Remove-Item -LiteralPath $selection -Force }
            $script:Status['needsClient'] = $true
            $script:Status['clientRequestId'] = [Guid]::NewGuid().ToString('N')
            $detail = if ($matches.Count -gt 1) { 'Multiple clients found. Select the EVE 2.10 build 3913 client folder.' } else { 'Select your EVE 2.10 build 3913 client folder.' }
            Set-Status 'Choose your client' $detail 87
            while (-not (Test-Path -LiteralPath $selection)) { Assert-Running; Start-Sleep -Milliseconds 200 }
            $selected = [IO.File]::ReadAllText($selection).Trim()
            $script:Status['needsClient'] = $false
            if (-not $selected) { throw [OperationCanceledException]::new('Client selection cancelled. Run start.bat when your client is ready.') }
        } else {
            $selected = Read-Host 'Path to EVE 2.10 build 3913 (client folder)'
            if (-not $selected) { throw [OperationCanceledException]::new('Client selection cancelled.') }
        }
        if (Test-RmrClient $selected) { $clientPath = [IO.Path]::GetFullPath($selected) }
        else { Write-Log 'That folder is not a complete EVE 2.10 build 3913 client. Please select another folder.' }
    }
    Set-Status 'Preparing client' 'Backing up and configuring your EVE client' 89
    $port = [int]$script:Status['gamePort']
    if ($port -lt 1 -or $port -gt 65535) { throw 'The server game port is not ready; cannot configure the client.' }
    $prepared = Prepare-RmrClient -ClientDirectory $clientPath -ServerBind '127.0.0.1' -ServerPort $port
    $Settings | Add-Member -NotePropertyName clientPath -NotePropertyValue $prepared.ClientDirectory -Force
    [IO.File]::WriteAllText((Join-Path $Runtime 'settings.json'), ($Settings | ConvertTo-Json), $Utf8)
    $script:Status['clientPath'] = $prepared.ClientDirectory
    Write-Log ('Client ready: ' + $prepared.ClientDirectory + ' (EVE build 3913; 127.0.0.1:' + $port + ')')
}

function Start-RmrGameClient {
    param([string]$ClientPath, [int]$GamePort = 0, [string]$ServerHost = '')
    # Self-contained: the UI stores this function body for its callback.
    $exe = Join-Path $ClientPath 'bin\exefile.exe'
    $launcher = Join-Path $ClientPath 'Start.bat'
    foreach ($path in @($exe, $launcher, (Join-Path $ClientPath 'script\rmr_connection.py'), (Join-Path $ClientPath 'script\rmr_login.py'))) {
        if (-not [IO.File]::Exists($path)) { throw 'The prepared EVE client is no longer present. Restart setup.' }
    }
    if ($GamePort -eq 0) {
        $match = [regex]::Match([IO.File]::ReadAllText($launcher), '(?im)^if not defined RMR_SERVER_PORT set "RMR_SERVER_PORT=(\d+)"\s*$')
        if (-not $match.Success) { throw 'The prepared client launcher has no recognized server port. Restart setup.' }
        $GamePort = [int]$match.Groups[1].Value
    }
    if ($GamePort -lt 1 -or $GamePort -gt 65535) { throw 'Invalid server game port.' }
    if (-not $ServerHost) {
        $hostMatch = [regex]::Match([IO.File]::ReadAllText($launcher), '(?im)^if not defined RMR_SERVER_HOST set "RMR_SERVER_HOST=([^"]+)"\s*$')
        $ServerHost = if ($hostMatch.Success) { $hostMatch.Groups[1].Value } else { '127.0.0.1' }
    }
    $logs = Join-Path $ClientPath 'logs'
    if (-not [IO.Directory]::Exists($logs)) { [void][IO.Directory]::CreateDirectory($logs) }
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $exe
    $start.WorkingDirectory = $ClientPath
    $start.Arguments = '/nosplash /noconsole "script:/rmr_connection.py" "script:/rmr_login.py"'
    $start.UseShellExecute = $false
    $start.EnvironmentVariables['RMR_LOGIN_SCALE'] = '200'
    $start.EnvironmentVariables['RMR_SERVER_HOST'] = $ServerHost
    $start.EnvironmentVariables['RMR_SERVER_PORT'] = [string]$GamePort
    $start.EnvironmentVariables['__COMPAT_LAYER'] = 'HIGHDPIAWARE'
    if ($env:RMR_ORIGINAL_TEMP) { $start.EnvironmentVariables['TEMP'] = $env:RMR_ORIGINAL_TEMP }
    if ($env:RMR_ORIGINAL_TMP) { $start.EnvironmentVariables['TMP'] = $env:RMR_ORIGINAL_TMP }
    if ($env:RMR_ORIGINAL_PATH) { $start.EnvironmentVariables['PATH'] = $env:RMR_ORIGINAL_PATH }
    [Diagnostics.Process]::Start($start) | Out-Null
}

$script:RmrClientStart = @'
@echo off
setlocal
rem Double-click for a 200%% login UI (1440 x 920 on a large enough display).
rem Optional: Start.bat [scale-percent] [server-address] [port]
rem Example for this machine's alternate server: Start.bat 200 127.0.0.1 26004
set "RMR_LOGIN_SCALE=200"
if not "%~1"=="" set "RMR_LOGIN_SCALE=%~1"
rem Connection overrides are inherited by this game process only.
if not defined RMR_SERVER_HOST set "RMR_SERVER_HOST=127.0.0.1"
if not defined RMR_SERVER_PORT set "RMR_SERVER_PORT=26000"
if not "%~2"=="" set "RMR_SERVER_HOST=%~2"
if not "%~3"=="" set "RMR_SERVER_PORT=%~3"

if not exist "%~dp0bin\exefile.exe" (
    echo Cannot find bin\exefile.exe beside Start.bat.
    pause
    exit /b 1
)
if not exist "%~dp0script\rmr_login.py" (
    echo Cannot find script\rmr_login.py beside Start.bat.
    pause
    exit /b 1
)
if not exist "%~dp0script\rmr_connection.py" (
    echo Cannot find script\rmr_connection.py beside Start.bat.
    pause
    exit /b 1
)

rem A clean extracted client has no logs directory until its first launch.
if not exist "%~dp0logs" mkdir "%~dp0logs"

rem Fonts are rendered at the larger size by the game, without bitmap DPI blur.
rem This compatibility setting applies only to this launch and its child process.
set "__COMPAT_LAYER=HIGHDPIAWARE"
rem The old eve.exe discards command-line arguments. Start the game directly.
start "" /D "%~dp0" "%~dp0bin\exefile.exe" /nosplash /noconsole "script:/rmr_connection.py" "script:/rmr_login.py"
exit /b %errorlevel%

'@

$script:RmrClientConnection = @'
# Launch-only connection settings and real LLV status for Python 2.3 / build 3913.
# Loaded explicitly by Start.bat. No connection defaults are written to disk.
import blue
import form
import sys
import uthread


def _rmrconn_log(message):
    try:
        handle = file(blue.os.rootpath + 'logs/rmr-connection.txt', 'a')
        try:
            handle.write(str(message) + '\n')
        finally:
            handle.close()
    except:
        sys.exc_clear()


def _rmrconn_overlay(obj, read_name, write_name, initial):
    # Patch this instance so aliases to it see the same session-only settings.
    # The real IniFile.lines / Section.__values remain untouched, including when
    # the game saves other preferences or the process exits without cleanup.
    if '_rmrconn_values' in obj.__dict__:
        return obj.__dict__['_rmrconn_values']
    values = initial.copy()
    protected = initial.copy()
    read_original = getattr(obj, read_name)
    write_original = getattr(obj, write_name)

    def read(key, *args, **kwargs):
        if key in protected:
            if key in values:
                return values[key]
            if args:
                default = args[0]
            elif 'default' in kwargs:
                default = kwargs['default']
            else:
                raise KeyError(key)
            if kwargs.get('flushDef') or (len(args) > 1 and args[1]):
                values[key] = default
            return default
        return read_original(key, *args, **kwargs)

    def write(key, value, *args, **kwargs):
        if key in protected:
            values[key] = value
            return
        return write_original(key, value, *args, **kwargs)

    obj.__dict__[read_name] = read
    obj.__dict__[write_name] = write
    obj.__dict__['_rmrconn_values'] = values
    if read_name == 'GetValue':
        has_original = obj.HasKey
        delete_original = obj.DeleteValue
        keys_original = obj.GetKeys

        def has(key):
            if key in protected:
                return key in values
            return has_original(key)

        def delete(key):
            if key in protected:
                if key in values:
                    del values[key]
                return
            return delete_original(key)

        def keys(beginWith=None):
            result = [key for key in keys_original() if key not in protected]
            result.extend(values.keys())
            if beginWith is not None:
                result = [key for key in result if key.startswith(beginWith)]
            return result

        obj.__dict__['HasKey'] = has
        obj.__dict__['DeleteValue'] = delete
        obj.__dict__['GetKeys'] = keys
    return values


def _rmrconn_endpoint(self):
    if not getattr(self, 'server', None) or not getattr(self, 'portedit', None):
        return None
    host = self.GetServerIP(self.server.GetValue()).strip()
    port = int(self.portedit.GetValue())
    if not host or port < 1 or port > 65535:
        raise ValueError('Invalid server address or port')
    return (host, port)


def _rmrconn_status_current(self, serial, endpoint):
    if getattr(self, '_rmrconn_serial', None) != serial:
        return False
    if not getattr(self, 'serverStatusText', None):
        return False
    try:
        return _rmrconn_endpoint(self) == endpoint
    except:
        sys.exc_clear()
        return False


def _rmrconn_status(self, server_name=None):
    if not getattr(self, 'serverStatusText', None):
        return
    serial = getattr(self, '_rmrconn_serial', 0) + 1
    self._rmrconn_serial = serial
    try:
        endpoint = _rmrconn_endpoint(self)
    except (ValueError, TypeError):
        self.serverStatusText.text = 'INVALID SERVER ADDRESS OR PORT'
        return
    if endpoint is None:
        return
    host, port = endpoint
    self.lastServerStatus = self.server.GetValue()
    self._rmrconn_last_endpoint = endpoint
    self.serverStatus[host] = None
    self.serverStatusText.text = 'CHECKING %s:%s...' % endpoint
    try:
        state, details = eve.LocalSvc('machoNet').GetServerStatus('%s:%s' % endpoint)
    except:
        sys.exc_clear()
        state, details = 'OFFLINE', None
    if not _rmrconn_status_current(self, serial, endpoint):
        return
    if state == 'OK' and details and len(details) >= 4:
        count, version, build, codename = details[:4]
        self.serverStatus[host] = (count, version, build, str(codename))
        # This server currently sends a fixed user count, so do not present it
        # as a live population. Connectivity/version come from the real probe.
        self.serverStatusText.text = 'ONLINE %s:%s<br>VERSION %.2f.%s' % (host, port, version, build)
    elif state == 'Incompatible':
        self.serverStatusText.text = 'INCOMPATIBLE SERVER %s:%s' % endpoint
    else:
        self.serverStatusText.text = 'OFFLINE / UNAVAILABLE<br>%s:%s' % endpoint
    _rmrconn_log('Status %s:%s: %s' % (host, port, state))


def _rmrconn_focus(self, *args):
    if not getattr(self, 'serverStatusText', None):
        return
    try:
        endpoint = _rmrconn_endpoint(self)
    except:
        sys.exc_clear()
        endpoint = None
    if endpoint != getattr(self, '_rmrconn_last_endpoint', None) or not self.serverStatusText.text:
        uthread.new(self.GetServerStatus)


def _rmrconn_poll(self):
    # One polling tasklet per login view, also refreshing after server restarts.
    while getattr(self, 'serverStatusText', None) is not None:
        blue.pyos.synchro.Sleep(10000)
        if getattr(self, 'serverStatusText', None) is None:
            return
        if not getattr(self, 'connecting', 0):
            self.GetServerStatus()


def _rmrconn_open(self, *args, **kwargs):
    # gameui creates settings.ui after the command-line helper is loaded.
    _rmrconn_overlay(settings.ui, 'Get', 'Set',
                     {'server': _rmrconn_host, 'usedServers': [_rmrconn_host]})
    result = _rmrconn_original_open(self, *args, **kwargs)
    self.portedit.OnFocusLost = self.OnServerFocusLost
    self.portedit.OnAnyChar = self.ClearServerStatus
    uthread.new(_rmrconn_poll, self)
    _rmrconn_log('Launch-only connection: %s:%s; UI %s:%s; advanced=%s' %
                 (_rmrconn_host, _rmrconn_port, self.server.GetValue(),
                  self.portedit.GetValue(), prefs.networkAdvanced))
    return result


def _rmrconn_install():
    global _rmrconn_host, _rmrconn_port, _rmrconn_original_open
    if getattr(form.Login, '_rmrconn_installed', False):
        return
    environment = blue.pyos.GetEnv()
    _rmrconn_host = environment['RMR_SERVER_HOST'].strip()
    _rmrconn_port = int(environment.get('RMR_SERVER_PORT', '26000'))
    if not _rmrconn_host or _rmrconn_port < 1 or _rmrconn_port > 65535:
        raise ValueError('Start.bat has an invalid RMR_SERVER_HOST / RMR_SERVER_PORT')
    _rmrconn_overlay(prefs.__dict__['ini'], 'GetValue', 'SetValue',
                     {'networkAdvanced': 1, 'port': _rmrconn_port})
    _rmrconn_original_open = form.Login.OnOpenView
    form.Login.OnOpenView = _rmrconn_open
    form.Login.GetServerStatus = _rmrconn_status
    form.Login.OnServerFocusLost = _rmrconn_focus
    form.Login._rmrconn_installed = True
    _rmrconn_log('Launch-only connection hook installed.')


if blue.pyos.GetEnv().get('RMR_SERVER_HOST'):
    _rmrconn_install()

'@

$script:RmrClientLogin = @'
# Launch-only login scaling and session-wide pacing for Python 2.3 / build 3913.
# Loaded explicitly by Start.bat; normal eve.exe startup remains available.
# No original resources, compiled code, preferences, or server settings are edited.
import blue
import form
import types
import sys
import traceback


def _rmr_log(message):
    try:
        handle = file(blue.os.rootpath + 'logs/rmr-login.txt', 'a')
        handle.write(str(message) + '\n')
        handle.close()
    except:
        sys.exc_clear()


def _rmr_windowed_settings(device, requested):
    # Get a driver-supported windowed presentation for the saved adapter and
    # size, then retain settings that are independent of fullscreen mode.
    adapter = requested.Adapter
    if adapter not in device.adapters:
        adapter = device.adapters[0]
    width = max(1, min(requested.BackBufferWidth, device.desktopMode.Width - 64))
    height = max(1, min(requested.BackBufferHeight, device.desktopMode.Height - 96))
    creation = device.GetFailsafeMode(adapter, 1, (width, height))
    if creation is None:
        return None
    safe = device.CreationToSettings(creation)
    values = requested.__dict__.copy()
    for key in ('Adapter', 'Windowed', 'BackBufferWidth', 'BackBufferHeight',
                'BackBufferFormat', 'AutoDepthStencilFormat',
                'FullScreen_RefreshRateInHz'):
        values[key] = safe.__dict__[key]
    safe.__dict__.update(values)
    return safe


class _RmrDeviceSettings:
    pass


def _rmr_paced_set(device, requested, set_device, *args, **kwargs):
    # SetDevice otherwise replaces the interval with IMMEDIATE when advanced
    # device settings are disabled. Override only its in-memory read, retaining
    # any existing connection overlay and restoring it even after an error.
    ini = prefs.__dict__['ini']
    old_get = ini.GetValue
    advanced = bool(old_get('advancedDevice', 0))
    had_get = 'GetValue' in ini.__dict__
    instance_get = ini.__dict__.get('GetValue')
    had_set = 'SetDevice' in device.__dict__
    instance_set = device.__dict__.get('SetDevice')
    original_fixup = device.FixupPresentation
    had_fixup = 'FixupPresentation' in device.__dict__
    instance_fixup = device.__dict__.get('FixupPresentation')
    attempts = [0]
    active_request = [None]
    if len(args) > 3:
        startup_hide_title = args[3]
    else:
        startup_hide_title = kwargs.get('hideTitle', 1)
    if len(args) > 2:
        requested_keep = args[2]
    else:
        requested_keep = kwargs.get('keepSettings', 1)
    section = None
    old_section_set = None
    had_section_set = False
    instance_section_set = None
    old_interval_present = False
    old_interval = None
    if requested_keep:
        try:
            section = settings.user.ui
            previous_settings = section.Get('DeviceSettings', None)
            if previous_settings is not None:
                old_interval_present = 'PresentationInterval' in previous_settings
                if old_interval_present:
                    old_interval = previous_settings['PresentationInterval']
            old_section_set = section.Set
            had_section_set = 'Set' in section.__dict__
            instance_section_set = section.__dict__.get('Set')
        except:
            # Never persist the forced interval when settings are unavailable.
            section = None
            requested_keep = 0
            sys.exc_clear()

    def save_without_pacing(key, value, *save_args, **save_kwargs):
        if key == 'DeviceSettings':
            value = value.copy()
            if old_interval_present:
                value['PresentationInterval'] = old_interval
            else:
                value.pop('PresentationInterval', None)
        return old_section_set(key, value, *save_args, **save_kwargs)

    def get_value(key, *get_args, **get_kwargs):
        if key == 'advancedDevice':
            return 1
        return old_get(key, *get_args, **get_kwargs)

    def invoke(settings, call_args, call_kwargs):
        attempts[0] += 1
        if attempts[0] > 4:
            _rmr_log('Windowed device retry limit reached.')
            return
        paced = _RmrDeviceSettings()
        paced.__dict__.update(settings.__dict__)
        paced.PresentationInterval = device.d3dconst.D3DPRESENT_INTERVAL_ONE
        if not advanced:
            paced.MultiSampleType = 0
            paced.MultiSampleQuality = 0
        names = ('tryAgain', 'fallback', 'keepSettings', 'hideTitle')
        defaults = (1, 0, 1, 1)
        options = list(call_args)
        call_kwargs = call_kwargs.copy()
        while len(options) < 4:
            index = len(options)
            options.append(call_kwargs.pop(names[index], defaults[index]))
        # Native retries must retain the caller's persistence choice.
        options[2] = requested_keep
        call_kwargs.pop('keepSettings', None)
        previous = active_request[0]
        active_request[0] = paced
        try:
            return set_device(paced, *options, **call_kwargs)
        finally:
            active_request[0] = previous

    def fixup(adapter, presentation):
        result = original_fixup(adapter, presentation)
        # FixupPresentation may substitute a new failsafe presentation whose
        # default interval is IMMEDIATE, or fall back to fullscreen.
        if active_request[0].Windowed and not result['Windowed']:
            raise RuntimeError('Windowed presentation unavailable')
        result['PresentationInterval'] = device.d3dconst.D3DPRESENT_INTERVAL_ONE
        return result

    def retry(settings, *retry_args, **retry_kwargs):
        # Native fallback can request fullscreen; keep this startup attempt
        # windowed and never let it persist the fallback settings.
        if not settings.Windowed:
            settings = _rmr_windowed_settings(device, settings)
            if settings is None:
                _rmr_log('Windowed device retry unavailable.')
                return
        retry_args = list(retry_args)
        if len(retry_args) > 3:
            retry_args[3] = startup_hide_title
        else:
            retry_kwargs['hideTitle'] = startup_hide_title
        return invoke(settings, retry_args, retry_kwargs)

    ini.__dict__['GetValue'] = get_value
    device.__dict__['SetDevice'] = retry
    device.__dict__['FixupPresentation'] = fixup
    if section is not None:
        section.__dict__['Set'] = save_without_pacing
    try:
        result = invoke(requested, args, kwargs)
    finally:
        if section is not None:
            if had_section_set:
                section.__dict__['Set'] = instance_section_set
            else:
                del section.__dict__['Set']
        if had_fixup:
            device.__dict__['FixupPresentation'] = instance_fixup
        else:
            del device.__dict__['FixupPresentation']
        if had_set:
            device.__dict__['SetDevice'] = instance_set
        else:
            del device.__dict__['SetDevice']
        if had_get:
            ini.__dict__['GetValue'] = instance_get
        else:
            del ini.__dict__['GetValue']
    try:
        _rmr_log('Actual presentation interval after device change: %s' %
                 device.GetSettings().PresentationInterval)
    except:
        sys.exc_clear()
    return result


def _rmr_install_pacing(device, state):
    original_set = device.SetDevice

    def paced_set(requested, *args, **kwargs):
        before = None
        if not state['preparing']:
            try:
                before = device.GetSettings().Windowed
            except:
                sys.exc_clear()
        result = _rmr_paced_set(device, requested, original_set, *args, **kwargs)
        if not state['preparing'] and before is not None and before != requested.Windowed:
            try:
                if device.GetSettings().Windowed == requested.Windowed:
                    state['manual_windowed'] = requested.Windowed
            except:
                sys.exc_clear()
        return result

    device.__dict__['SetDevice'] = paced_set


def _rmr_install_windowed(device, state):
    # PrepareMain is called by the intro or character selection after login.
    # It normally restores and saves the account's fullscreen DeviceSettings.
    original_prepare = device.PrepareMain
    def prepare_windowed(*args, **kwargs):
        original_set = device.SetDevice
        had_set = 'SetDevice' in device.__dict__
        old_set = device.__dict__.get('SetDevice')

        def set_windowed(requested, *set_args, **set_kwargs):
            try:
                if state['manual_windowed'] is None or state['manual_windowed']:
                    safe = _rmr_windowed_settings(device, requested)
                else:
                    safe = requested
            except:
                _rmr_log('Could not prepare windowed mode: ' +
                         ''.join(traceback.format_exception(*sys.exc_info())))
                sys.exc_clear()
                return
            if safe is None:
                _rmr_log('Windowed mode unavailable; retaining the login device.')
                return
            # SetDevice's positional defaults are tryAgain, fallback,
            # keepSettings, hideTitle. Replace those two even on retries.
            options = list(set_args)
            names = ('tryAgain', 'fallback', 'keepSettings', 'hideTitle')
            defaults = (1, 0, 1, 1)
            while len(options) < 4:
                index = len(options)
                options.append(set_kwargs.pop(names[index], defaults[index]))
            options[2] = 0
            options[3] = not safe.Windowed
            set_kwargs.pop('keepSettings', None)
            set_kwargs.pop('hideTitle', None)
            return original_set(safe, *options, **set_kwargs)

        device.__dict__['SetDevice'] = set_windowed
        state['preparing'] += 1
        try:
            return original_prepare(*args, **kwargs)
        finally:
            state['preparing'] -= 1
            if had_set:
                device.__dict__['SetDevice'] = old_set
            else:
                del device.__dict__['SetDevice']

    device.__dict__['PrepareMain'] = prepare_windowed


def _rmr_number(value):
    return type(value) in (types.IntType, types.LongType, types.FloatType)


def _rmr_snapshot(node, result):
    geometry = {}
    for key in ('left', 'top', 'width', 'height', 'setWidth', 'resetTop',
                'fixedWidth', 'fixedHeight', 'tabMargin', 'specialIndent'):
        value = getattr(node, key, None)
        if _rmr_number(value):
            geometry[key] = value
    font = None
    if hasattr(node, 'GetParams') and hasattr(node, 'Layout'):
        params = node.GetParams()
        font = (params.fontSize or 9, getattr(node, 'letterSpacing', 0) or 0,
                getattr(node, 'linespace', 0) or 0)
    result.append((node, geometry, font))
    for child in list(getattr(node, 'children', [])):
        _rmr_snapshot(child, result)


def _rmr_apply(records, scale):
    # Parent geometry first; glyphs are freshly rasterized by ULabel's font API.
    labels = []
    for node, geometry, font in records:
        if font is not None:
            node.busy = 1
        for key, value in geometry.items():
            setattr(node, key, int(round(value * scale)))
        if font is not None:
            node.fontSize = max(1, int(round(font[0] * scale)))
            node.letterSpacing = int(round(font[1] * scale))
            node.linespace = int(round(font[2] * scale))
            node.busy = 0
            labels.append(node)
    for node in labels:
        node.Layout()
    return len(labels)


_rmr_original_open = form.Login.OnOpenView
_rmr_original_motd = form.Login.ShowMotd


def _rmr_open(self, *args, **kwargs):
    _rmr_log('Opening scaled login.')
    self._rmr_scale = 1.0
    try:
        result = _rmr_original_open(self, *args, **kwargs)
        _rmr_log('Original login constructed.')
        return _rmr_resize(self, result)
    except:
        _rmr_log(''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()
        return


def _rmr_resize(self, result):
    records = []
    try:
        requested = float(blue.pyos.GetEnv().get('RMR_LOGIN_SCALE', '200')) / 100.0
        requested = max(1.0, min(3.0, requested))
    except (ValueError, TypeError):
        requested = 2.0
    # LocalSvc returns a remote-call wrapper, including for attribute accesses.
    # Startup's service manager returns the actual local device service.
    device = sm.StartService('device')
    _rmr_log('Device ready.')
    old = device.GetSettings()
    if not old.Windowed:
        try:
            safe = _rmr_windowed_settings(device, old)
            if safe is not None:
                device.SetDevice(safe, keepSettings=0, hideTitle=0)
                old = device.GetSettings()
        except:
            _rmr_log('Login windowed mode failed: ' +
                     ''.join(traceback.format_exception(*sys.exc_info())))
            sys.exc_clear()
    _rmr_log('Current backbuffer: %sx%s' % (old.BackBufferWidth, old.BackBufferHeight))
    base_width = old.BackBufferWidth
    base_height = old.BackBufferHeight
    # Reserve room for the taskbar; retain the original login's aspect ratio.
    scale = min(requested,
                max(1.0, (device.desktopMode.Width - 64.0) / base_width),
                max(1.0, (device.desktopMode.Height - 96.0) / base_height))
    width = int(round(base_width * scale))
    height = int(round(base_height * scale))
    _rmr_log('Requested login: %sx%s, scale %.3f' % (width, height, scale))
    if scale <= 1.0:
        try:
            device.SetDevice(old, keepSettings=0, hideTitle=1)
        except:
            _rmr_log('Login pacing failed: ' +
                     ''.join(traceback.format_exception(*sys.exc_info())))
            sys.exc_clear()
        return result
    try:
        # Capture the original layout before the resize can move anchored items.
        for child in list(self.layer.uiwindow.children):
            _rmr_snapshot(child, records)
        creation = device.GetFailsafeMode(old.Adapter, 1, (width, height))
        if creation is None:
            _rmr_log('Windowed mode unavailable; retaining original login.')
            device.SetDevice(old, keepSettings=0, hideTitle=1)
            return result
        device.SetDevice(device.CreationToSettings(creation), keepSettings=0, hideTitle=1)
        actual = device.GetSettings()
        if (actual.BackBufferWidth, actual.BackBufferHeight) != (width, height):
            _rmr_log('Device rejected resize; retaining original layout.')
            device.SetDevice(old, keepSettings=0, hideTitle=1)
            return result
        count = _rmr_apply(records, scale)
        self._rmr_scale = scale
        eve.triapp.uilib.desktop.Update()
        _rmr_log('Applied login: %sx%s; %s native text labels.' %
                 (actual.BackBufferWidth, actual.BackBufferHeight, count))
    except:
        self._rmr_scale = 1.0
        _rmr_log('Scaling failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        # Keep a usable original login if a driver or UI object rejects scaling.
        sys.exc_clear()
        try:
            device.SetDevice(old, keepSettings=0, hideTitle=1)
            _rmr_apply(records, 1.0)
            eve.triapp.uilib.desktop.Update()
        except:
            _rmr_log('Original-layout recovery failed: ' + str(sys.exc_info()[1]))
            sys.exc_clear()
    return result


def _rmr_motd(self, *args, **kwargs):
    result = _rmr_original_motd(self, *args, **kwargs)
    scale = getattr(self, '_rmr_scale', 1.0)
    if scale > 1.0:
        records = []
        for child in list(self.step2[0].children):
            if child.name.startswith('servernote'):
                _rmr_snapshot(child, records)
        _rmr_apply(records, scale)
    return result


form.Login.OnOpenView = _rmr_open
form.Login.ShowMotd = _rmr_motd
_rmr_device = sm.StartService('device')
_rmr_device_state = {'preparing': 0, 'manual_windowed': None}
_rmr_install_pacing(_rmr_device, _rmr_device_state)
_rmr_install_windowed(_rmr_device, _rmr_device_state)
_rmr_log('Session-wide presentation pacing installed.')
_rmr_log('Start.bat login scaling hook loaded.')
try:
    # Character selection's OpenNews uses this exact historical URL. Redirect
    # only that request; other in-game browser pages retain their normal URLs.
    import xtriui
    _rmr_original_news_goto = xtriui.VirtualEdit.GoTo
    def _rmr_news_goto(self, url, *args, **kwargs):
        if url == 'http://myeve.eve-online.com/mb/news.asp':
            url = 'http://127.0.0.1:18080/client-news'
        return _rmr_original_news_goto(self, url, *args, **kwargs)
    xtriui.VirtualEdit.GoTo = _rmr_news_goto
    _rmr_log('Character selection news uses the local dashboard.')
except:
    _rmr_log('Local news hook failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
    sys.exc_clear()
try:
    # cfg.const is read-only; override only this lookup in its Python class.
    _rmr_const_class = const.__class__
    _rmr_original_const_getattr = _rmr_const_class.__getattr__
    def _rmr_warp_const_getattr(self, name):
        if self is const and name == 'minWarpInDistance':
            return 0
        return _rmr_original_const_getattr(self, name)
    _rmr_const_class.__getattr__ = _rmr_warp_const_getattr
    if const.minWarpInDistance != 0:
        raise RuntimeError('Warp range lookup did not change')
    _rmr_log('Warp-to-zero default range enabled.')
except:
    _rmr_warp_error = ''.join(traceback.format_exception(*sys.exc_info()))
    if '_rmr_const_class' in globals() and '_rmr_original_const_getattr' in globals():
        try:
            _rmr_const_class.__getattr__ = _rmr_original_const_getattr
        except:
            sys.exc_clear()
    _rmr_log('Warp range setup failed: ' + _rmr_warp_error)
    sys.exc_clear()

def _rmr_install_self_info():
    # infosvc.ShowInfo routes one's own character to the character sheet.
    # Use its ordinary info-window path for the same itemID instead.
    while not getattr(eve.session, 'charid', None):
        blue.pyos.synchro.Sleep(1000)
    try:
        # LocalSvc returns a ServiceConnection proxy. StartService returns
        # the service instance in this client build.
        info = sm.StartService('info')
        cls = info.__class__
        if getattr(cls, '_rmr_self_info_installed', False):
            return
        original = cls.ShowInfo
        def show_info(self, typeID, itemID=None, new=0, rec=None, parentID=None, headerOnly=0):
            if itemID != eve.session.charid:
                return original(self, typeID, itemID, new, rec, parentID, headerOnly)
            modal = eve.LocalSvc('ui').GetModalWindow()
            wnd = self.GetWnd(new, modal)
            if wnd.destroyed:
                wnd = self.GetWnd(1, modal)
            wnd.IsBrowser = 1
            wnd.GoTo = self.GoTo
            self.FormatWnd(wnd, typeID, itemID, rec=rec, parentID=parentID, headerOnly=headerOnly)
            wnd.Maximize()
            return None
        cls.ShowInfo = show_info
        cls._rmr_self_info_installed = True
        _rmr_log('Own-character Show Info opens the normal character info window.')
    except:
        _rmr_log('Own-character Show Info hook failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()

import uthread
uthread.new(_rmr_install_self_info)

def _rmr_install_scan_results():
    # The System Scanner subtracts the ship's position straight from each
    # result's position, which needs a vector rather than a tuple.
    while not getattr(eve.session, 'charid', None):
        blue.pyos.synchro.Sleep(1000)
    try:
        import foo
        import xtriui
        svc = sm.StartService('scanservice')
        cls = svc.__class__
        if getattr(cls, '_rmr_scan_results_installed', False):
            return
        original = cls.GetScans
        def get_scans(self, *args, **kwargs):
            scans = original(self, *args, **kwargs)
            if not scans:
                return scans
            probes, results = scans
            fixed = []
            for each in results or []:
                try:
                    pos = each[2]
                    if isinstance(pos, (tuple, list)):
                        pos = foo.Vector3(float(pos[0]), float(pos[1]), float(pos[2]))
                    if len(each) >= 5 and each[4]:
                        # List the site's name where the result ID was;
                        # the ID rides along at the end for Warp To.
                        each = (each[4], each[1], pos, each[3], each[0])
                    else:
                        each = (each[0], each[1], pos, each[3])
                except:
                    sys.exc_clear()
                fixed.append(each)
            return (probes, fixed)
        cls.GetScans = get_scans
        cls._rmr_scan_results_installed = True
        scanner = xtriui.SystemScanner
        original_refresh = scanner.Refresh
        def refresh_later(window):
            # Refresh once the session has settled: asking the server for
            # scans while it changes (on a jump) trips the client's
            # session check.
            for i in range(50):
                blue.pyos.synchro.Sleep(200)
                if not getattr(eve.session, 'changing', None):
                    break
            if window is not None and not getattr(window, 'destroyed', 0):
                refresh(window)
        def refresh(self, *args, **kwargs):
            if getattr(eve.session, 'changing', None):
                uthread.new(refresh_later, self)
                return
            try:
                return original_refresh(self, *args, **kwargs)
            except:
                _rmr_log('System Scanner refresh failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
                sys.exc_clear()
        scanner.Refresh = refresh
        original_warp = scanner.WarpTo
        def warp_to(self, scanResult, *args, **kwargs):
            if len(scanResult) >= 5:
                scanResult = (scanResult[4],) + tuple(scanResult[1:4])
            return original_warp(self, scanResult, *args, **kwargs)
        scanner.WarpTo = warp_to
        _rmr_log('System Scanner results use vector positions and site names.')
    except:
        _rmr_log('System Scanner results hook failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()

uthread.new(_rmr_install_scan_results)

def _rmr_install_tutorial_guard():
    # Half of the tutorials in the data have no pages (the Aura notes for
    # chat channels, cloning, insurance and more). The server no longer
    # lists them; should the client still ask for one, it is skipped
    # instead of failing on its missing first page.
    while not getattr(eve.session, 'charid', None):
        blue.pyos.synchro.Sleep(1000)
    try:
        svc = None
        for name in ('tutorial', 'tutorialSvc'):
            try:
                candidate = sm.GetService(name)
            except:
                sys.exc_clear()
                continue
            if hasattr(candidate, 'OpenTutorial'):
                svc = candidate
                break
        if svc is None:
            _rmr_log('Tutorial guard: no tutorial service found.')
            return
        cls = svc.__class__
        if getattr(cls, '_rmr_tutorial_guard', False):
            return
        original = cls.OpenTutorial
        def open_tutorial(self, *args, **kwargs):
            try:
                return original(self, *args, **kwargs)
            except (IndexError, AttributeError, KeyError):
                _rmr_log('Tutorial %r has no pages; skipped.' % (args[:1],))
                sys.exc_clear()
        cls.OpenTutorial = open_tutorial
        cls._rmr_tutorial_guard = True
        _rmr_log('Tutorials without pages are skipped.')
    except:
        _rmr_log('Tutorial guard failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()

uthread.new(_rmr_install_tutorial_guard)

def _rmr_session_busy():
    return getattr(eve.session, 'changing', None) or getattr(eve.session, 'mutating', 0)

def _rmr_defer_while_changing(cls, name):
    # Calls the server for the old location while the session is still
    # switching (on a jump, dock or undock) trip the client's session
    # check. Run them once the session has settled instead.
    original = getattr(cls, name, None)
    if original is None or getattr(original, '_rmr_deferred', False):
        return False
    def later(self, args, kwargs):
        for i in range(75):
            blue.pyos.synchro.Sleep(200)
            if not _rmr_session_busy():
                break
        try:
            original(self, *args, **kwargs)
        except:
            _rmr_log('%s after a session change failed: %s' % (name, ''.join(traceback.format_exception(*sys.exc_info()))))
            sys.exc_clear()
    def wrapper(self, *args, **kwargs):
        if _rmr_session_busy():
            uthread.new(later, self, args, kwargs)
            return None
        return original(self, *args, **kwargs)
    wrapper._rmr_deferred = True
    setattr(cls, name, wrapper)
    return True

def _rmr_install_session_guards():
    # Python 2.5 notes inside the client's own star map code (hex() of a
    # negative number, a float where an int is wanted) are logged as stack
    # traces; they change nothing.
    try:
        import warnings
        warnings.filterwarnings('ignore', category=FutureWarning)
        warnings.filterwarnings('ignore', category=DeprecationWarning)
    except:
        sys.exc_clear()
    while not getattr(eve.session, 'charid', None):
        blue.pyos.synchro.Sleep(1000)
    done = []
    for service, method in (('tactical', 'CheckInitDrones'), ('window', 'OnCapacityChange')):
        try:
            cls = sm.GetService(service).__class__
            if _rmr_defer_while_changing(cls, method):
                done.append('%s.%s' % (service, method))
        except:
            _rmr_log('Session guard for %s.%s failed: %s' % (service, method, ''.join(traceback.format_exception(*sys.exc_info()))))
            sys.exc_clear()
    # The market window asks for the station's sell orders even in space,
    # and the client then fails with "Must be in station".
    try:
        cls = sm.GetService('marketQuote').__class__
        original_asks = getattr(cls, 'GetStationAsks', None)
        if original_asks is not None and not getattr(original_asks, '_rmr_guarded', False):
            def station_asks(self, *args, **kwargs):
                if not getattr(eve.session, 'stationid', None):
                    return {}
                return original_asks(self, *args, **kwargs)
            station_asks._rmr_guarded = True
            cls.GetStationAsks = station_asks
            done.append('marketQuote.GetStationAsks')
    except:
        _rmr_log('Market guard failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()
    # Clicking Jump again while the first jump is still under way: the
    # jump is already happening, so the second click is let go quietly.
    try:
        cls = sm.GetService('menu').__class__
        original_jump = getattr(cls, 'StargateJump', None)
        if original_jump is not None and not getattr(original_jump, '_rmr_guarded', False):
            def stargate_jump(self, *args, **kwargs):
                try:
                    return original_jump(self, *args, **kwargs)
                except:
                    if 'subspace' not in str(sys.exc_info()[1]):
                        raise
                    sys.exc_clear()
            stargate_jump._rmr_guarded = True
            cls.StargateJump = stargate_jump
            done.append('menu.StargateJump')
    except:
        _rmr_log('Jump guard failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()
    _rmr_log('Session guards installed: ' + ', '.join(done))

uthread.new(_rmr_install_session_guards)

def _rmr_install_html_table_guard():
    # The client's HTML tables fail (KeyError 'spacing-right') when a cell's
    # text is wider than its column, which can stop a window drawing (the
    # character selection screen). Keep the layout it has instead.
    try:
        cls = None
        for i in range(120):
            try:
                import listentry
                cls = getattr(listentry, 'table', None)
            except:
                sys.exc_clear()
            if cls is not None:
                break
            blue.pyos.synchro.Sleep(500)
        if cls is None or not hasattr(cls, 'RefreshSizes'):
            _rmr_log('HTML table guard: no table entry class found.')
            return
        if getattr(cls, '_rmr_table_guard', False):
            return
        original = cls.RefreshSizes
        def refresh_sizes(self, *args, **kwargs):
            try:
                return original(self, *args, **kwargs)
            except KeyError, e:
                if 'spacing' not in str(e):
                    raise
                sys.exc_clear()
        cls.RefreshSizes = refresh_sizes
        cls._rmr_table_guard = True
        _rmr_log('HTML table width guard installed.')
    except:
        _rmr_log('HTML table guard failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()

uthread.new(_rmr_install_html_table_guard)

def _rmr_install_alliance_logos():
    while not getattr(eve.session, 'charid', None):
        blue.pyos.synchro.Sleep(1000)
    try:
        import uix
        import zlib
        cls = form.AlliancesHome
        if getattr(cls, '_rmr_alliance_logos', None):
            return
        original_logo = uix.GetAllianceLogo
        cached = {}

        class AllianceLogoUpdates:
            __notifyevents__ = ['OnAllianceLogoChanged']

            def OnAllianceLogoChanged(self, allianceID):
                cached.pop(allianceID, None)

        def get_logo(allianceID, *args, **kwargs):
            logo = original_logo(allianceID, *args, **kwargs)
            if not eve.session.charid:
                return logo
            try:
                if allianceID not in cached:
                    data = eve.RemoteSvc('allianceRegistry').GetAllianceLogoImage(allianceID)
                    path = None
                    if data:
                        name = 'rmr-alliance-%d-%08x.png' % (allianceID, zlib.crc32(data) & 4294967295)
                        handle = file(blue.os.cachepath + name, 'wb')
                        try:
                            handle.write(data)
                        finally:
                            handle.close()
                        path = 'cache:/' + name
                    cached[allianceID] = path
                path = cached[allianceID]
                if path:
                    # Use the alpha-blended UI pass for the RGBA logo.
                    logo.tripass = eve.rot.GetInstance('res:/ui/pass/linear.blue')
                    logo.texture.pixels = path
                    logo.rectLeft = logo.rectTop = 0
                    logo.rectWidth = logo.rectHeight = 128
            except:
                _rmr_log('Alliance logo load failed: ' + str(sys.exc_info()[1]))
                sys.exc_clear()
            return logo

        def edit_alliance(self, *args):
            alliance = eve.LocalSvc('alliance').GetAlliance()
            registry = eve.RemoteSvc('allianceRegistry')
            current = registry.GetAllianceLogoUrl() or ''
            fields = [
                {'type': 'btline'},
                {'type': 'push', 'frame': 1},
                {'type': 'edit', 'key': 'url', 'label': 'URL', 'labelwidth': 104,
                 'setvalue': alliance.url or '', 'frame': 1, 'maxLength': 2048},
                {'type': 'push', 'frame': 1},
                {'type': 'edit', 'key': 'logoUrl', 'label': 'Logo Image URL', 'labelwidth': 104,
                 'setvalue': current, 'frame': 1, 'maxLength': 2048},
                {'type': 'push', 'frame': 1},
                {'type': 'textedit', 'key': 'description', 'label': 'Description',
                 'setvalue': alliance.description or '', 'frame': 1, 'maxLength': 5000},
                {'type': 'push', 'frame': 1},
                {'type': 'bbline'}]
            desktop = eve.triapp.uilib.desktop
            pos = [(desktop.width - 500) // 2, (desktop.height - 400) // 2]
            result = uix.HybridWnd(fields, 'Update Alliance', 1, None, uix.OKCANCEL, pos, 500, 190)
            if result is not None:
                logo_url = result['logoUrl'].strip()
                if logo_url != current:
                    registry.UpdateAlliance(result['description'], result['url'], logo_url)
                    cached.pop(alliance.allianceID, None)
                else:
                    eve.LocalSvc('alliance').UpdateAlliance(result['description'], result['url'])

        updates = AllianceLogoUpdates()
        sm.RegisterNotify(updates)
        uix.GetAllianceLogo = get_logo
        cls.EditAllianceForm = edit_alliance
        cls._rmr_alliance_logos = updates
    except:
        _rmr_log('Alliance logo setup failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()

uthread.new(_rmr_install_alliance_logos)

def _rmr_editable_focus(ui):
    import xtriui
    focus = ui.GetFocus()
    if isinstance(focus, (xtriui.SingleLineEdit, xtriui.VirtualEdit)) and not getattr(focus, 'readonly', False):
        return focus
    return None


def _rmr_install_directional_scan(cmd_class):
    import listentry
    import state as state_flags
    import trinity
    import uix
    import util
    import xtriui

    max_km = 2109329976
    scanner = xtriui.SystemScanner
    nav = form.InflightNav
    state_class = sm.StartService('state').__class__
    original_startup = scanner.Startup
    original_mouse_up = nav._OnMouseUp
    original_set_state = state_class.SetState
    armed = [None]

    def start_scanner(self, *args):
        result = original_startup(self, *args)
        if not self.destroyed and hasattr(self, 'dir_rangeinput'):
            self.dir_rangeinput.IntMode(1, max_km)
            saved = settings.user.ui.Get('dir_scanrange', max_km)
            self.dir_rangeinput.SetValue(min(max_km, max(1, int(saved))))
        return result

    def space_camera():
        scene = eve.triapp.tridev.scene
        return getattr(scene, 'camera', None)

    def scan(self, direction=None):
        if self.destroyed or self.searching or not eve.session.solarsystemid:
            return
        if direction is None:
            camera = space_camera()
            if not hasattr(camera, 'rotationAroundParent'):
                return
            direction = trinity.TriVector(0, 0, 1)
            direction.TransformQuaternion(camera.rotationAroundParent)
            direction.Normalize()
        park = eve.LocalSvc('michelle').GetRemotePark()
        if not park:
            return
        window = uix.GetWindowAbove(self)
        self.searching = True
        try:
            window.ShowLoad()
            km = min(max_km, max(1, self.dir_rangeinput.GetValue()))
            angle = self.sr.scanangle
            records = park.ConeScan(angle, float(km) * 1000.0, direction.x, direction.y, direction.z)
            if self.destroyed:
                return
            settings.user.ui.Set('dir_scanrange', km)
            groups = None
            if self.sr.useoverview.checked:
                groups = eve.LocalSvc('tactical').GetGroups()
            entries = []
            for rec in records:
                if groups is not None and rec.groupID not in groups:
                    continue
                type_name = cfg.invtypes.Get(rec.typeID).name
                name = rec.name or type_name
                distance = util.FmtDist(rec.distance, maxdemicals=1)
                data = util.KeyVal()
                data.name = data.sort_name = data.hint = name
                data.sort_dist = rec.distance
                data.sort_type = type_name
                data.label = '%s<t>%s<t>%s' % (name, type_name, distance)
                data.itemID = rec.id
                data.typeID = rec.typeID
                entries.append(listentry.Get('Generic', data=data))
            self.listtype = 'location'
            if entries:
                self.SetHint()
            else:
                self.SetHint('Nothing found')
            self.sr.dirscroll.Load(fixedEntryHeight=18, contentList=entries, headers=['name', 'type', 'dist'])
        finally:
            self.searching = False
            if not window.destroyed:
                window.HideLoad()

    def direction_search(self, *args):
        scan(self)

    def context():
        return (eve.session.solarsystemid, eve.session.shipid)

    def awaiting_click():
        pending = armed[0]
        return pending and pending[0] == context() and blue.os.GetTime() < pending[1]

    def toward(item_id):
        park = eve.LocalSvc('michelle').GetBallpark()
        if not park:
            return None
        ship = park.GetBall(eve.session.shipid)
        item = park.GetBall(item_id)
        if ship is not None and item is not None:
            return trinity.TriVector(item.x - ship.x, item.y - ship.y, item.z - ship.z)

    def aim_and_scan(direction, expected):
        if expected != context() or not direction.Length():
            return
        direction.Normalize()
        camera = space_camera()
        if not hasattr(camera, 'rotationAroundParent'):
            return
        camera.rotationOfInterest.SetIdentity()
        rotation = camera.rotationAroundParent
        if direction.z < -0.999999:
            rotation.SetYawPitchRoll(3.141592653589793, 0, 0)
        else:
            rotation.SetRotationArc(trinity.TriVector(0, 0, 1), direction)
        windows = eve.LocalSvc('window')
        window = windows.GetWindow('scanner')
        if window is None or window.destroyed:
            eve.LocalSvc('cmd').OpenScanner()
            window = windows.GetWindow('scanner')
        if window is not None and not window.destroyed and expected == context():
            window.Maximize()
            if window.sr.tabs.GetSelectedArgs() != 'system':
                window.sr.tabs.SelectByIdx(0, False)
            scan(window.sr.systemscanner, direction)

    def queue_scan(direction):
        if direction is not None and direction.Length():
            armed[0] = None
            uthread.new(aim_and_scan, direction, context())

    def select_object(self, item_id, flag, value, *args):
        result = original_set_state(self, item_id, flag, value, *args)
        if value and flag == state_flags.selected and awaiting_click():
            queue_scan(toward(item_id))
        return result

    def click_space(self, button, *args):
        direction = None
        if button == 0 and awaiting_click() and (self.GetMouseTravel() or 0) <= 5:
            lib = eve.triapp.uilib
            scene = eve.triapp.tridev.scene
            if not lib.rightbtn and scene and scene.camera:
                _, picked = self.GetPick()
                curve = getattr(picked, 'translationCurve', None)
                item_id = getattr(curve, 'id', None)
                if item_id is not None:
                    direction = toward(item_id)
                if direction is None or not direction.Length():
                    direction = scene.PickInfinity(lib.x, lib.y, scene.camera.projection, scene.camera.view)
        if direction is not None:
            armed[0] = None
        result = original_mouse_up(self, button, *args)
        queue_scan(direction)
        return result

    def rescan_open_window(window, expected):
        # V with the scanner already open repeats the scan along the camera
        # and refreshes the result list in place.
        if window.destroyed or expected != context():
            return
        window.Maximize()
        if window.sr.tabs.GetSelectedArgs() != 'system':
            window.sr.tabs.SelectByIdx(0, False)
        scan(window.sr.systemscanner)

    def directional_scan(self):
        # V, then a click in space (or on anything in it): the camera turns
        # that way and the scanner scans along it, open or not. V twice
        # scans straight along the camera as it points now.
        if not eve.session.solarsystemid or not eve.session.shipid:
            return
        if _rmr_editable_focus(eve.LocalSvc('ui')) is not None:
            return
        if awaiting_click():
            armed[0] = None
            window = eve.LocalSvc('window').GetWindow('scanner')
            if window is None or window.destroyed:
                eve.LocalSvc('cmd').OpenScanner()
                window = eve.LocalSvc('window').GetWindow('scanner')
            if window is not None and not window.destroyed:
                uthread.new(rescan_open_window, window, context())
            return
        armed[0] = (context(), blue.os.GetTime() + 300000000)
        try:
            eve.Message('CustomNotify', {'notify': 'Directional scan: click where to scan (V again scans where the camera faces).'})
        except:
            sys.exc_clear()

    scanner.Startup = start_scanner
    scanner.DirectionSearch = direction_search
    nav._OnMouseUp = click_space
    state_class.SetState = select_object
    cmd_class.CmdDirectionalScan = directional_scan


def _rmr_install_shortcuts():
    while not getattr(eve.session, 'charid', None):
        blue.pyos.synchro.Sleep(1000)
    try:
        import uix
        cmd = sm.StartService('cmd')
        cls = cmd.__class__
        if not getattr(cls, '_rmr_shortcuts_installed', False):
            original_reload = cls.ReloadcommandsByFunc
            shortcut_defaults = (('CmdBookmarkCurrentLocation', (1, 0, 0, 'b')),
                                 ('CmdDronesEngage', (0, 0, 0, 'f')),
                                 ('CmdReturnDrones', (0, 0, 1, 'r')),
                                 ('CmdDirectionalScan', (0, 0, 0, 'v')))
            shortcut_names = tuple([name for name, key in shortcut_defaults])

            def bookmark_current_location(self):
                if _rmr_editable_focus(eve.LocalSvc('ui')) is not None:
                    return
                return eve.LocalSvc('addressbook').BookmarkCurrentLocation()

            def order_drones(engage):
                session = eve.session
                if not session.solarsystemid or not session.shipid:
                    return
                if _rmr_editable_focus(eve.LocalSvc('ui')) is not None:
                    return
                michelle = eve.LocalSvc('michelle')
                park = michelle.GetBallpark()
                drones = michelle.GetDrones()
                if park is None or not drones:
                    return
                ids = []
                for drone_id in drones:
                    drone = drones[drone_id]
                    if drone.ownerID != session.charid or drone.controllerID != session.shipid:
                        continue
                    if engage:
                        item = park.slimItems.get(drone_id)
                        if item is None or item.groupID in (const.groupMiningDrone, const.groupUnanchoringDrone):
                            continue
                    ids.append(drone_id)
                if ids:
                    menu = eve.LocalSvc('menu')
                    if engage:
                        menu.EngageTarget(ids)
                    else:
                        menu.ReturnToDroneBay(ids)

            def drones_engage(self):
                return order_drones(True)

            def return_drones(self):
                return order_drones(False)

            def reload_shortcuts(self):
                result = original_reload(self)
                for name, key in shortcut_defaults:
                    if not hasattr(self, name):
                        continue
                    self.defaultcmds[name] = key
                    if name not in self.commandsByFunc:
                        self.LoadCmd(name, self.GetCustomCmd(name) or key)
                return result

            # LocalSvc returns a proxy; patch the service that owns charHandler.
            ui = sm.StartService('ui')
            ui_class = ui.__class__
            original_char = ui_class.OnGlobalChar

            def on_global_char(self, char, flag):
                # The legacy character handler drops accelerator keys before
                # forwarding characters to chat, even if the action did nothing.
                focus = _rmr_editable_focus(self)
                if focus is not None and 32 <= char <= 126:
                    key = eve.triapp.uilib.Key
                    if not key(uix.VK_CONTROL) and not key(uix.VK_MENU):
                        name = chr(char).replace(' ', 'space').upper()
                        vkey = getattr(uix, 'VK_' + name, None)
                        shortcut = (0, 0, key(uix.VK_SHIFT), vkey)
                        current_cmd = sm.services.get('cmd')
                        if current_cmd and current_cmd.GetFuncByShortcut(shortcut) in shortcut_names:
                            focus.OnChar(char, flag)
                            return
                return original_char(self, char, flag)

            ui_class.OnGlobalChar = on_global_char
            eve.triapp.uilib.charHandler = ui.OnGlobalChar

            cls.CmdBookmarkCurrentLocation = bookmark_current_location
            cls.CmdDronesEngage = drones_engage
            cls.CmdReturnDrones = return_drones
            cls.ReloadcommandsByFunc = reload_shortcuts
            cls._rmr_shortcuts_installed = True
        if not getattr(cls, 'CmdDirectionalScan', None):
            try:
                _rmr_install_directional_scan(cls)
            except:
                _rmr_log('Directional scanner setup failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
                sys.exc_clear()
        cmd.ReloadcommandsByFunc()
        _rmr_log('RMR shortcuts registered in Escape shortcuts.')
    except:
        _rmr_log('Shortcut setup failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()

uthread.new(_rmr_install_shortcuts)

def _rmr_install_reload_all():
    # A weapon's right-click menu in space gets "Reload all": every fitted
    # module of that kind refills from the cargo hold with the charge it
    # already uses (or, empty, the first that fits). The server's reload
    # flag fills without swapping charges (svc/fitting.rs FLAG_RELOAD).
    while not getattr(eve.session, 'charid', None):
        blue.pyos.synchro.Sleep(1000)
    try:
        import xtriui
        cls = getattr(xtriui, 'ModuleButton', None)
        if cls is None or not hasattr(cls, 'GetMenu'):
            _rmr_log('Reload all: the module button has no menu to extend.')
            return
        if getattr(cls, '_rmr_reload_all', False):
            return
        original_menu = cls.GetMenu
        reload_flag = -9

        def add_to_modules(inv, stack):
            try:
                inv.Add(stack.itemID, stack.locationID, flag=reload_flag)
            except TypeError:
                inv.Add(stack.itemID, flag=reload_flag)

        def reload_all(button):
            try:
                info = button.sr.moduleInfo
                ship = eve.session.shipid
                if not ship:
                    return
                inv = eve.GetInventoryFromId(ship)
                items = inv.List()
                group = getattr(info, 'groupID', None)
                slots = [i.flag for i in items if i.categoryID == 7 and i.groupID == group]
                loaded = [i for i in items if i.categoryID == 8 and i.flag in slots]
                mine = getattr(info, 'flagID', None)
                types = [i.typeID for i in loaded if i.flag == mine] + [i.typeID for i in loaded]
                cargo = [i for i in items if i.categoryID == 8 and i.flag == 5]
                done = []
                for t in types:
                    if t in done:
                        continue
                    done.append(t)
                    for stack in [c for c in cargo if c.typeID == t]:
                        try:
                            add_to_modules(inv, stack)
                        except:
                            sys.exc_clear()
                            break
                if not types:
                    for stack in cargo:
                        try:
                            add_to_modules(inv, stack)
                            break
                        except:
                            sys.exc_clear()
            except:
                _rmr_log('Reload all failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
                sys.exc_clear()

        def get_menu(self, *args, **kwargs):
            menu = original_menu(self, *args, **kwargs)
            try:
                if eve.session.solarsystemid and getattr(self.sr, 'moduleInfo', None) is not None:
                    labels = [str(entry[0]).lower() for entry in (menu or []) if entry and isinstance(entry, tuple)]
                    if not [l for l in labels if 'reload all' in l]:
                        menu = list(menu or [])
                        menu.append(None)
                        menu.append(('Reload all', reload_all, (self,)))
            except:
                _rmr_log('Reload all menu failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
                sys.exc_clear()
            return menu

        cls.GetMenu = get_menu
        cls._rmr_reload_all = True
        _rmr_log('Reload all added to module menus.')
    except:
        _rmr_log('Reload all setup failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()

uthread.new(_rmr_install_reload_all)

def _rmr_install_grid_recovery():
    while not getattr(eve.session, 'charid', None):
        blue.pyos.synchro.Sleep(1000)
    try:
        import state
        target = sm.StartService('target')
        cls = target.__class__
        if getattr(cls, '_rmr_grid_recovery', None):
            return
        original_clear = cls.DoBallClear

        class GridRecovery:
            __notifyevents__ = ['OnNewState']
            active = None
            park = None
            reported = False

            # This client sends OnNewState(park) with no hint.
            def OnNewState(self, park, *args):
                uthread.new(self.DedupeOverview, park)
                if self.park is not park:
                    return
                active = self.active
                self.park = None
                self.active = None
                uthread.new(self.RestoreLocks, park, active)

            def DedupeOverview(self, park):
                # A resynchronised grid (SetState) is added to the overview
                # beside the rows already there: drop rows for balls that are
                # gone, and every row but one per item.
                try:
                    blue.pyos.synchro.Sleep(750)
                    if eve.LocalSvc('michelle').GetBallpark() is not park:
                        return
                    tactical = eve.LocalSvc('tactical')
                    overview = getattr(getattr(tactical, 'sr', None), 'overview', None)
                    if overview is None or overview.destroyed:
                        return
                    scroll = overview.sr.scroll
                    balls = getattr(park, 'balls', None) or {}
                    kept = {}
                    stale = []
                    for node in scroll.sr.entries[:]:
                        item_id = getattr(node, 'itemID', None)
                        if item_id is None:
                            continue
                        if item_id not in balls:
                            stale.append(node)
                            continue
                        ball = getattr(node, 'ball', None)
                        try:
                            if callable(ball):
                                ball = ball()
                        except:
                            sys.exc_clear()
                        current = ball is None or ball is balls.get(item_id)
                        if item_id not in kept:
                            kept[item_id] = (node, current)
                        elif current and not kept[item_id][1]:
                            stale.append(kept[item_id][0])
                            kept[item_id] = (node, current)
                        else:
                            stale.append(node)
                    if stale:
                        scroll.RemoveEntries(stale)
                    if stale and not self.reported:
                        self.reported = True
                        _rmr_report('grid recovery: removed %d stale overview row(s) after a resync' % len(stale))
                except:
                    _rmr_report('grid recovery: overview cleanup failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
                    sys.exc_clear()

            def RestoreLocks(self, park, active):
                try:
                    if eve.LocalSvc('michelle').GetBallpark() is not park:
                        return
                    eve.LocalSvc('godma').GetStateManager().RefreshTargets()
                    if active in target.GetTargets() and eve.LocalSvc('michelle').GetBallpark() is park:
                        eve.LocalSvc('state').SetState(active, state.activeTarget, 1)
                except:
                    _rmr_log('Grid lock refresh failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
                    sys.exc_clear()

        recovery = GridRecovery()

        def clear_grid(self, solitem):
            try:
                park = eve.LocalSvc('michelle').GetBallpark()
                # A reset of an already populated grid leaves old UI rows behind.
                if park is not None and park.ego == eve.session.shipid and park.slimItems:
                    recovery.park = park
                    recovery.active = self.GetActiveTargetID()
                    for target_id in list(self.targetsByID):
                        self.ClearTarget(target_id)
                    self.pendingTargets = []
                    self.pendingTargeters = []
                    self.targetedBy = []
                    tactical = eve.LocalSvc('tactical')
                    sr = getattr(tactical, 'sr', None)
                    overview = getattr(sr, 'overview', None)
                    if overview is not None and not overview.destroyed:
                        scroll = overview.sr.scroll
                        scroll.RemoveEntries(scroll.sr.entries[:])
            except:
                _rmr_log('Grid clear failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
                sys.exc_clear()
            return original_clear(self, solitem)

        cls.DoBallClear = clear_grid
        sm.RegisterNotify(recovery)
        cls._rmr_grid_recovery = recovery
    except:
        _rmr_log('Grid recovery setup failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()

uthread.new(_rmr_install_grid_recovery)


def _rmr_install_overview_dedupe():
    # tactical.py adds an overview row for every ball in an AddBalls or
    # SetState without looking for one already there, and DoBallRemove takes
    # away only the first row it finds. A ball sent twice (or a grid
    # resynchronised) therefore leaves a second row behind, with a stale
    # distance, for as long as the pilot stays on grid. Keep one row per item.
    while not getattr(eve.session, 'charid', None):
        blue.pyos.synchro.Sleep(1000)
    try:
        cls = sm.StartService('tactical').__class__
        if getattr(cls, '_rmr_overview_dedupe', False):
            return
        original_added = cls._DoBallsAdded
        original_remove = cls.DoBallRemoveThread
        counts = {'rows': 0}

        def overview_scroll(self):
            sr = getattr(self, 'sr', None)
            overview = getattr(sr, 'overview', None)
            if overview is None or getattr(overview, 'destroyed', 0):
                return None
            return getattr(overview.sr, 'scroll', None)

        def drop_rows(self, item_ids):
            scroll = overview_scroll(self)
            if scroll is None or not item_ids:
                return 0
            rows = [e for e in scroll.sr.entries[:] if getattr(e, 'itemID', None) in item_ids]
            if rows:
                scroll.RemoveEntries(rows)
            return len(rows)

        def balls_added(self, lst):
            try:
                ids = {}
                for each in lst:
                    try:
                        ids[each[1].itemID] = 1
                    except:
                        sys.exc_clear()
                dropped = drop_rows(self, ids)
                if dropped and not counts['rows']:
                    _rmr_report('overview dedupe: replaced %d existing overview row(s) for re-added balls' % dropped)
                counts['rows'] += dropped
            except:
                _rmr_log('Overview dedupe (add) failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
                sys.exc_clear()
            return original_added(self, lst)

        def ball_removed(self, ball, slimItem, terminal):
            result = original_remove(self, ball, slimItem, terminal)
            try:
                if ball is not None:
                    drop_rows(self, {ball.id: 1})
            except:
                _rmr_log('Overview dedupe (remove) failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
                sys.exc_clear()
            return result

        cls._DoBallsAdded = balls_added
        cls.DoBallRemoveThread = ball_removed
        cls._rmr_overview_dedupe = True
        _rmr_report('overview dedupe: installed on %s' % cls.__name__)
    except:
        _rmr_report('overview dedupe: setup failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()


uthread.new(_rmr_install_overview_dedupe)

def _rmr_install_local_roster():
    while not getattr(eve.session, 'charid', None):
        blue.pyos.synchro.Sleep(1000)
    reported = {}

    def report_once(key, message):
        # Problems go to the server log once each, where they can be read.
        if reported.get(key) != message:
            reported[key] = message
            try:
                _rmr_report('local roster: ' + message)
            except:
                sys.exc_clear()
                _rmr_log('local roster: ' + message)

    try:
        import listentry
        import state
        import uix
        cls = listentry.ChatUser
        if getattr(cls, '_rmr_local_roster', False):
            return
        original_load = cls.Load
        current = [None, {}]

        def local_channel():
            return (('solarsystemid2', eve.session.solarsystemid2),)

        def is_local(channel_id):
            try:
                return channel_id[0][0] == 'solarsystemid2'
            except:
                sys.exc_clear()
                return False

        def war_badge(entry, flag_id=None):
            import draw
            flags = eve.LocalSvc('state')
            if flag_id is None:
                flag_id = state.flagAtWarCanFight
            props = flags.GetStateProps(flag_id)
            badge = eve.rot.GetCopy('res:/UI/Component/flag.blue')
            badge.children[0].rectLeft = (props[4] + 1) * 10
            background = draw.Fill(badge, getInstance=0)
            background.color.SetRGB(*flags.GetStateColor(flag_id))
            badge.align = uix.UI_ALNONE
            badge.state = uix.UI_DISABLED
            badge.width = badge.height = 10
            badge.left = entry.sr.picture.left + 22
            badge.top = entry.sr.picture.top + 22
            entry.children.insert(0, badge)
            return badge

        def client_at_war(node):
            # The client's own war relationship, as its overview flags war
            # targets: the pilot's alliance, else corporation.
            try:
                war = sm.StartService('war')
                for owner in (getattr(node, 'allianceID', None), getattr(node, 'corpID', None)):
                    if owner and war.GetRelationship(owner) == const.warRelationshipAtWarCanFight:
                        return True
            except:
                report_once('client war', 'client war check failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
                sys.exc_clear()
            return False

        def load(self, node, *args):
            result = original_load(self, node, *args)
            try:
                # An entry closed while it loaded has no sr left.
                if getattr(self, 'sr', None) is None or self.destroyed or getattr(self.sr, 'node', None) is not node:
                    return result
                owner = getattr(node, 'allianceID', None) or getattr(node, 'corpID', None)
                badge = getattr(self.sr, 'rmrWarBadge', None)
                # War targets are flagged wherever they are listed: by the
                # client's own war data (as the overview flags them), else by
                # the server's list of hostile wars. Corporation mates get the
                # overview's green star, alliance mates its blue one.
                flag_id = None
                if client_at_war(node) or (owner is not None and owner in current[1]):
                    flag_id = state.flagAtWarCanFight
                elif getattr(node, 'charID', None) != eve.session.charid:
                    if getattr(node, 'corpID', None) and node.corpID == eve.session.corpid:
                        flag_id = getattr(state, 'flagSameCorp', None)
                    elif getattr(node, 'allianceID', None) and node.allianceID == eve.session.allianceid:
                        flag_id = getattr(state, 'flagSameAlliance', None)
                if badge is not None and not badge.destroyed and getattr(self.sr, 'rmrBadgeFlag', None) != flag_id:
                    badge.Close()
                    badge = None
                if flag_id is None:
                    self.sr.rmrWarBadge = None
                elif badge is None or badge.destroyed:
                    self.sr.rmrWarBadge = war_badge(self, flag_id)
                self.sr.rmrBadgeFlag = flag_id
            except:
                report_once('badge', 'war badge failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
                sys.exc_clear()
            return result

        def find_channel(lsc):
            channels = getattr(lsc, 'channels', None)
            if not hasattr(channels, 'get'):
                report_once('channels', 'no channel table on the LSC service (%s); its channel attributes: %s' % (
                    type(channels).__name__, ', '.join([n for n in dir(lsc) if 'hannel' in n.lower()])))
                return local_channel(), None
            key = local_channel()
            # The LSC channel table's get() wants a default as well.
            try:
                channel = channels.get(key, None)
            except TypeError:
                sys.exc_clear()
                channel = None
            if channel is None:
                # The table keeps its channels in an 'items' dict, which hides
                # the usual items() method.
                table = getattr(channels, 'items', None)
                if isinstance(table, dict):
                    pairs = table.items()
                else:
                    pairs = channels.items()
                for k, v in pairs:
                    if is_local(k) and k[0][1] == eve.session.solarsystemid2:
                        return k, v
            return key, channel

        def channel_pairs(lsc):
            channels = getattr(lsc, 'channels', None)
            if channels is None:
                return []
            table = getattr(channels, 'items', None)
            try:
                if isinstance(table, dict):
                    return table.items()
                return channels.items()
            except:
                sys.exc_clear()
                return []

        def show_members(lsc, channel, updated):
            for pilot_id in updated:
                if pilot_id not in cfg.eveowners and updated[pilot_id].extra:
                    cfg.eveowners.Hint(pilot_id, updated[pilot_id].extra)
            channel.memberList = updated
            try:
                lsc._LargeScaleChat__SetEstimatedMemberCount(channel, len(updated))
            except:
                sys.exc_clear()
            window = getattr(channel, 'window', None)
            if window is not None and not window.destroyed:
                entries = getattr(getattr(getattr(window, 'userlist', None), 'sr', None), 'entries', None)
                if entries is not None:
                    shown = dict([(node.charID, (node.corpID, node.allianceID)) for node in entries])
                    expected = dict([(pilot_id, (updated[pilot_id].corpID, updated[pilot_id].allianceID)) for pilot_id in updated])
                    if shown != expected:
                        window.InitUsers(0)
                window.UpdateCaption()

        def refresh_groups(lsc):
            # Corporation and alliance channels: the full member list from
            # the server, which the client otherwise never asks for again.
            pairs = channel_pairs(lsc)
            for key, ignored in pairs:
                try:
                    kind = key[0][0]
                except:
                    sys.exc_clear()
                    continue
                if kind not in ('corpid', 'allianceid'):
                    continue
                # The table's raw values are not the channel objects; its
                # get() is (as for Local).
                try:
                    channel = lsc.channels.get(key, None)
                except TypeError:
                    sys.exc_clear()
                    channel = None
                if channel is None or isinstance(channel, (list, tuple)):
                    report_once('group none ' + kind, 'no channel object for %r' % (key,))
                    continue
                result = eve.RemoteSvc('LSC').GetLocalRoster(key)
                if result is None:
                    report_once('group ' + kind, 'the server gave no members for %r' % (key,))
                    continue
                updated = result[0].Index('charID')
                show_members(lsc, channel, updated)
                report_once('group ok ' + kind, '%s channel: %d members' % (kind, len(updated)))

        def refresh():
            # eve.LocalSvc returns a call proxy; the channel table lives on
            # the service object itself.
            lsc = sm.StartService('LSC')
            try:
                refresh_groups(lsc)
            except:
                report_once('groups', 'corp/alliance members failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
                sys.exc_clear()
            channel_id, channel = find_channel(lsc)
            # The war list comes with the roster, for this system's Local,
            # whether or not the client's channel table could be read.
            result = eve.RemoteSvc('LSC').GetLocalRoster(channel_id)
            if result is None:
                report_once('roster', 'the server gave no roster for %r' % (channel_id,))
                return 5000
            members, enemies = result
            updated = members.Index('charID')
            war_flags = dict([(owner, True) for owner in enemies])
            war_changed = current != [channel_id, war_flags]
            current[:] = [channel_id, war_flags]
            if channel is None:
                if 'ok' not in reported:
                    report_once('ok', 'war list only: %d war enemies (Local window not found)' % len(war_flags))
                return 10000
            for pilot_id in updated:
                if pilot_id not in cfg.eveowners and updated[pilot_id].extra:
                    cfg.eveowners.Hint(pilot_id, updated[pilot_id].extra)
            channel.memberList = updated
            try:
                lsc._LargeScaleChat__SetEstimatedMemberCount(channel, len(updated))
            except:
                sys.exc_clear()
            window = getattr(channel, 'window', None)
            if window is not None and not window.destroyed:
                entries = getattr(getattr(getattr(window, 'userlist', None), 'sr', None), 'entries', None)
                if entries is not None:
                    shown = dict([(node.charID, (node.corpID, node.allianceID)) for node in entries])
                    expected = dict([(pilot_id, (updated[pilot_id].corpID, updated[pilot_id].allianceID)) for pilot_id in updated])
                    if shown != expected or war_changed:
                        window.InitUsers(0)
                window.UpdateCaption()
            if 'ok' not in reported:
                report_once('ok', 'working: %d pilots in Local, %d war enemies' % (len(updated), len(war_flags)))
            return 10000

        cls.Load = load
        cls._rmr_local_roster = True
    except:
        report_once('setup', 'setup failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()
        return

    while 1:
        delay = 10000
        if eve.session.charid and eve.session.solarsystemid2:
            try:
                delay = refresh()
            except:
                report_once('refresh', 'refresh failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
                sys.exc_clear()
        blue.pyos.synchro.Sleep(delay)

uthread.new(_rmr_install_local_roster)

def _rmr_install_bookmark_warp_menu():
    while not getattr(eve.session, 'charid', None):
        blue.pyos.synchro.Sleep(1000)
    try:
        cls = sm.StartService('menu').__class__
        if getattr(cls, '_rmr_bookmark_warp_installed', False):
            return
        original = cls.ParseMenu

        def adjust_bookmark_warp_range(self, entries):
            for entry in entries:
                if entry and len(entry) == 5 and entry[1] == self.ApproachLocation:
                    bookmark = entry[2][0]
                    break
            else:
                return entries
            system = eve.session.solarsystemid
            if not system or bookmark.locationID != system or getattr(bookmark, 'agentID', None):
                return entries
            park = eve.LocalSvc('michelle').GetBallpark()
            if not park:
                return entries
            coordinate = bookmark.typeID == const.typeSolarSystem and bookmark.itemID == system and bookmark.x is not None
            ball_id = bookmark.itemID
            if coordinate:
                ball_id = eve.session.shipid
            if ball_id not in park.balls:
                return entries
            ball = park.balls[ball_id]
            if coordinate:
                dx, dy, dz = ball.x - bookmark.x, ball.y - bookmark.y, ball.z - bookmark.z
                in_range = dx * dx + dy * dy + dz * dz > const.minWarpDistance ** 2
            else:
                in_range = ball.surfaceDist > const.minWarpDistance
            # Build 3913 uses a separate 1,000 km check for the bookmark warp
            # entries. Keep the native menu and replace only that distance check.
            adjusted = list(entries)
            for index, entry in enumerate(entries):
                if entry and len(entry) == 5 and entry[4] == 'warp' and entry[3]:
                    checks = list(entry[3])
                    for offset in range(0, len(checks), 2):
                        if checks[offset][1:] == ['Within range', 'Not in range']:
                            checks[offset] = [in_range, 'You are in warp range', 'You are not in warp range']
                    adjusted[index] = list(entry)
                    adjusted[index][3] = tuple(checks)
            return adjusted

        def parse_menu(self, menuEntries, showInactive=0, filterFunc=None):
            try:
                menuEntries = adjust_bookmark_warp_range(self, menuEntries)
            except:
                _rmr_log('Bookmark warp menu failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
                sys.exc_clear()
            return original(self, menuEntries, showInactive, filterFunc)

        cls.ParseMenu = parse_menu
        cls._rmr_bookmark_warp_installed = True
    except:
        _rmr_log('Bookmark warp setup failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()

uthread.new(_rmr_install_bookmark_warp_menu)

def _rmr_install_warp_distance():
    while not getattr(eve.session, 'charid', None):
        blue.pyos.synchro.Sleep(1000)
    try:
        cls = sm.StartService('space').__class__
        if getattr(cls, '_rmr_warp_distance_installed', False):
            return
        original_call = cls.OnBallparkCall
        original_confirm = cls.ConfirmWarpDestination

        def own_warp_destination(self, name, args):
            # Keep the native movement for other ships without changing our HUD.
            if name == 'WarpTo' and args[0] != eve.session.shipid:
                return
            return original_call(self, name, args)

        def confirm_landing_point(self):
            original_confirm(self)
            # Use the actual WarpTo coordinates, including the selected range.
            self.warpDestinationCache[4] = self.warpDestinationCache[3]

        cls.OnBallparkCall = own_warp_destination
        cls.ConfirmWarpDestination = confirm_landing_point
        cls._rmr_warp_distance_installed = True
    except:
        _rmr_log('Warp distance setup failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()

uthread.new(_rmr_install_warp_distance)

class _RmrSpaceQueue:
    def __init__(self, broker, queue):
        self.broker = broker
        self.queue = queue

    def put(self, stop):
        # Michelle.Stop uses this to pass its None sentinel.
        return self.queue.put(stop)

    def non_blocking_put(self, work):
        return self.queue.non_blocking_put((self.broker._Michelle__bp, work))

    def get(self):
        while True:
            queued = self.queue.get()
            if queued is None:
                return None
            if len(queued) == 5:
                # The dispatcher's wake-up or work queued during setup.
                park, work = self.broker._Michelle__bp, queued
            else:
                park, work = queued
            if park is None or park is not self.broker._Michelle__bp:
                continue
            owners, corps, alliances, balls, new_state = work
            # These lookups can yield while docking tears down the park.
            if owners:
                cfg.eveowners.Prime(owners)
            if corps:
                cfg.corptickernames.Prime(corps)
            if alliances:
                cfg.allianceshortnames.Prime(alliances)
            if park is self.broker._Michelle__bp:
                return (None, None, None, balls, new_state)


def _rmr_install_space_queue():
    try:
        broker = sm.StartService('michelle')
        queue = broker.ballQueue
        if not isinstance(queue, _RmrSpaceQueue):
            broker.ballQueue = _RmrSpaceQueue(broker, queue)
            # Wake the native dispatcher if it is already waiting on get().
            queue.non_blocking_put((None, None, None, (), None))
    except:
        _rmr_log('Space update queue setup failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()

uthread.new(_rmr_install_space_queue)

_rmr_contacts_only = [0]

def _rmr_install_mission_agent_contacts():
    while not getattr(eve.session, 'charid', None):
        blue.pyos.synchro.Sleep(1000)
    try:
        chat = sm.StartService('LSC')
        cls = chat.__class__
        if getattr(cls, '_rmr_mission_agents_installed', False):
            return
        original_buddies = cls.GetBuddies
        def buddies_with_mission_agents(self):
            buddies = original_buddies(self)
            try:
                missions = eve.LocalSvc('journal').GetMyAgentJournalDetails()[0]
                mission_agents = [mission[4] for mission in missions]
                for agent_id in mission_agents:
                    if agent_id not in buddies:
                        buddies.append(agent_id)
            except:
                _rmr_log('Mission agent contact refresh failed: ' +
                         ''.join(traceback.format_exception(*sys.exc_info())))
                sys.exc_clear()
            return buddies
        cls.GetBuddies = buddies_with_mission_agents
        cls._rmr_mission_agents_installed = True
        _rmr_log('Active mission agents appear in People and Places.')
    except:
        _rmr_log('Mission agent contact setup failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()

uthread.new(_rmr_install_mission_agent_contacts)

_RMR_AGENT_PRIME = 400

def _rmr_prime_owners(ids):
    # One GetOwners round trip per few hundred names instead of one per agent.
    ids = [i for i in ids if i]
    for start in range(0, len(ids), _RMR_AGENT_PRIME):
        try:
            cfg.eveowners.Prime(ids[start:start + _RMR_AGENT_PRIME])
        except:
            sys.exc_clear()

def _rmr_owner_name(ownerID, fallback):
    try:
        return cfg.eveowners.Get(ownerID).name
    except:
        sys.exc_clear()
        return fallback

def _rmr_by_name(a, b):
    return cmp(a[0].lower(), b[0].lower())

def _rmr_agent_group(label, ident, items, sublevel, content):
    import listentry
    data = {
        'GetSubContent': content,
        'label': label,
        'id': ident,
        'items': items,
        'headers': ['name'],
        'iconMargin': 18,
        'showlen': 0,
        'state': None,
        'npc': 1,
        'allowCopy': 1,
        'showicon': '_22_41',
        'posttext': ' [%s]' % len(items),
        'sublevel': sublevel,
        'allowGuids': ['listentry.User', 'listentry.Sender'],
    }
    return listentry.Get('Group', data)

def _rmr_agent_open(agentID):
    # Double-click: talk to the agent, as the right-click menu would.
    try:
        agents = sm.StartService('agents')
        if hasattr(agents, 'InteractWith'):
            agents.InteractWith(agentID)
            return
    except:
        _rmr_log('Agent conversation failed: ' + str(sys.exc_info()[1]))
        sys.exc_clear()
    try:
        sm.GetService('info').ShowInfo(cfg.eveowners.Get(agentID).typeID, agentID)
    except:
        sys.exc_clear()

def _rmr_agent_row(agents, agentID):
    try:
        if agentID in agents:
            return agents[agentID]
    except:
        sys.exc_clear()
    return None

def _rmr_agent_people(nodedata, *args):
    # Plain rows, not contact cards: a contact card renders the agent's face
    # in 3D on the spot, and a corporation's worth of them froze the client
    # while it scrolled. Right-click still gives the agent's own menu and
    # double-click starts the conversation.
    import listentry
    import util
    ids = list(getattr(nodedata, 'items', None) or [])
    _rmr_prime_owners(ids)
    agents = sm.StartService('agents').GetAgentsByID()
    stations = []
    for agentID in ids:
        agent = _rmr_agent_row(agents, agentID)
        if agent is not None and getattr(agent, 'stationID', None):
            stations.append(agent.stationID)
    try:
        if stations:
            cfg.evelocations.Prime(stations)
    except:
        sys.exc_clear()
    named = []
    for agentID in ids:
        try:
            named.append((cfg.eveowners.Get(agentID).name, agentID))
        except:
            sys.exc_clear()
    named.sort(_rmr_by_name)
    sublevel = getattr(nodedata, 'sublevel', 0) + 1
    entries = []
    for name, agentID in named:
        where = ''
        agent = _rmr_agent_row(agents, agentID)
        try:
            if agent is not None and getattr(agent, 'stationID', None):
                where = cfg.evelocations.Get(agent.stationID).name
        except:
            sys.exc_clear()
        data = util.KeyVal()
        data.label = name
        if where:
            data.label = '%s - %s' % (name, where)
        data.hint = data.label
        data.name = name
        data.sort_name = name.lower()
        data.itemID = agentID
        data.charID = agentID
        try:
            data.typeID = cfg.eveowners.Get(agentID).typeID
        except:
            sys.exc_clear()
        data.sublevel = sublevel
        data.OnDblClick = lambda entry, agentID=agentID: _rmr_agent_open(agentID)
        entries.append(listentry.Get('Generic', data=data))
    return entries

def _rmr_agent_corps(nodedata, *args):
    # Faction -> its corporations, each a group of that corporation's agents.
    agents = sm.StartService('agents').GetAgentsByID()
    corps = {}
    for agentID in getattr(nodedata, 'items', None) or []:
        if agentID not in agents:
            continue
        agent = agents[agentID]
        corps.setdefault(agent.corporationID, []).append(agentID)
    _rmr_prime_owners(corps.keys())
    named = [(_rmr_owner_name(corpID, 'Corporation %s' % corpID), corpID) for corpID in corps.keys()]
    named.sort(_rmr_by_name)
    sublevel = getattr(nodedata, 'sublevel', 0) + 1
    return [_rmr_agent_group(name, nodedata.id + (corpID,), corps[corpID], sublevel, _rmr_agent_people)
            for name, corpID in named]

def _rmr_agent_factions(nodedata, *args):
    # Level -> the factions with agents at that level for this pilot.
    agents = sm.StartService('agents').GetAgentsByID()
    factions = {}
    for agentID in getattr(nodedata, 'items', None) or []:
        if agentID not in agents:
            continue
        agent = agents[agentID]
        factions.setdefault(agent.factionID or 0, []).append(agentID)
    _rmr_prime_owners([f for f in factions.keys() if f])
    named = [(_rmr_owner_name(factionID, 'Independent'), factionID) for factionID in factions.keys()]
    named.sort(_rmr_by_name)
    sublevel = getattr(nodedata, 'sublevel', 0) + 1
    return [_rmr_agent_group(name, nodedata.id + (factionID,), factions[factionID], sublevel, _rmr_agent_corps)
            for name, factionID in named]

_rmr_available_cache = [None, 0]

def _rmr_available_agent_groups():
    # The server lists every mission agent that would work with this pilot on
    # the personal channel's contacts. Arrange them like the Character Sheet:
    # by level, then faction, then corporation, names fetched per group.
    # The list comes from the server when the tab opens (kept a minute), not
    # from the chat contact list, which every contact check reads through.
    agents = sm.StartService('agents').GetAgentsByID()
    now = blue.os.GetTime()
    cached = _rmr_available_cache
    if cached[0] is None or now - cached[1] > 60 * 10000000L:
        try:
            cached[0] = list(eve.RemoteSvc('factory').RmrAvailableAgents() or [])
        except:
            _rmr_log('Available agents could not be fetched: ' + str(sys.exc_info()[1]))
            sys.exc_clear()
            cached[0] = cached[0] or []
        cached[1] = now
    levels = {}
    for agentID in cached[0]:
        if agentID not in agents:
            continue
        agent = agents[agentID]
        levels.setdefault(agent.level or 1, []).append(agentID)
    keys = levels.keys()
    keys.sort()
    groups = []
    for level in keys:
        groups.append(_rmr_agent_group('Level %s agents available to you' % level,
                                       ('rmragentgroups', 'level', level),
                                       levels[level], 0, _rmr_agent_factions))
    return groups

def _rmr_drop_broken_groups(tb):
    # The innermost frame's 'groups': every group with no 'items' goes.
    frame = None
    while tb is not None:
        frame = tb.tb_frame
        tb = tb.tb_next
    while frame is not None:
        groups = frame.f_locals.get('groups')
        if isinstance(groups, dict):
            for key in groups.keys():
                value = groups[key]
                if isinstance(value, dict) and not value.has_key('items'):
                    del groups[key]
                    _rmr_log('People and Places: dropped broken agent group %r' % (key,))
            return
        frame = frame.f_back

def _rmr_scrub_agent_groups(book):
    # Remove 'rmr_available' from wherever the stock list keeps its agent
    # groups: the service itself or the character's UI settings.
    def scrub(d):
        if isinstance(d, dict) and d.has_key('rmr_available'):
            del d['rmr_available']
            return 1
        return 0
    for value in book.__dict__.values():
        try:
            scrub(value)
            if isinstance(value, dict):
                for inner in value.values():
                    scrub(inner)
        except:
            sys.exc_clear()
    for store in (settings.char.ui, settings.user.ui):
        keys = ['agentgroups', 'agentGroups', 'npcgroups', 'buddygroups', 'contactgroups']
        for source in ('datastore', 'data', '_data'):
            try:
                keys = keys + list(getattr(store, source).keys())
            except:
                sys.exc_clear()
        try:
            keys = keys + list(store.keys())
        except:
            sys.exc_clear()
        for key in keys:
            try:
                d = store.Get(key, None)
                if scrub(d):
                    store.Set(key, d)
            except:
                sys.exc_clear()

def _rmr_install_available_agents():
    while not getattr(eve.session, 'charid', None):
        blue.pyos.synchro.Sleep(1000)
    try:
        book = sm.StartService('addressbook')
        cls = book.__class__
        if getattr(cls, '_rmr_available_agents_installed', False):
            return
        original_show = cls.ShowPersonal
        original_sub = cls.GetBuddiesSubContent
        def show_personal(self, npc):
            if not npc:
                return original_show(self, npc)
            _rmr_contacts_only[0] = 1
            try:
                try:
                    result = original_show(self, npc)
                except KeyError:
                    # A group saved by an older version of this hook, with no
                    # agent list of its own, broke the stock list: take it out
                    # of the very dict the list was reading, and go again.
                    _rmr_drop_broken_groups(sys.exc_info()[2])
                    sys.exc_clear()
                    _rmr_scrub_agent_groups(self)
                    try:
                        result = original_show(self, npc)
                    except KeyError:
                        # Still there: show the available-agent groups anyway.
                        _rmr_drop_broken_groups(sys.exc_info()[2])
                        sys.exc_clear()
                        result = None
            finally:
                _rmr_contacts_only[0] = 0
            try:
                wnd = self.wnd
                if wnd is None or wnd.destroyed:
                    return result
                groups = _rmr_available_agent_groups()
                if groups:
                    wnd.sr.scroll.AddEntries(-1, groups)
                    self.SetHint()
            except:
                _rmr_log('Available agent groups failed: ' +
                         ''.join(traceback.format_exception(*sys.exc_info())))
                sys.exc_clear()
            return result
        def buddies_sub_content(self, nodedata, *args):
            # Fetch all the names in one go before the list builds its rows.
            try:
                if getattr(nodedata, 'npc', 0):
                    _rmr_prime_owners(list(getattr(nodedata, 'items', None) or []))
            except:
                sys.exc_clear()
            return original_sub(self, nodedata, *args)
        cls.ShowPersonal = show_personal
        cls.GetBuddiesSubContent = buddies_sub_content
        cls._rmr_available_agents_installed = True
        _rmr_log('People and Places lists every agent available to you.')
    except:
        _rmr_log('Available agents hook failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()

uthread.new(_rmr_install_available_agents)

# Slow-call watch: what in portraits, mail and People and Places holds the
# client up. Calls that take a while and frames that stall are summed and
# sent to the server log every so often (factory.RmrReport, "client timing").
_RMR_SLOW_MS = 120
_rmr_slow_calls = {}
_rmr_stalls = [0, 0.0, 0.0]

def _rmr_note_slow(service, method, ms, args):
    key = service + '.' + method
    entry = _rmr_slow_calls.get(key)
    if entry is None:
        entry = [0, 0.0, 0.0, '']
        _rmr_slow_calls[key] = entry
    entry[0] = entry[0] + 1
    entry[1] = entry[1] + ms
    if ms > entry[2]:
        entry[2] = ms
        try:
            entry[3] = repr(args)[:80]
        except:
            sys.exc_clear()

def _rmr_timed_method(service, method, fn):
    import time
    def timed(self, *args, **kwargs):
        start = time.clock()
        try:
            return fn(self, *args, **kwargs)
        finally:
            ms = (time.clock() - start) * 1000.0
            if ms >= _RMR_SLOW_MS:
                _rmr_note_slow(service, method, ms, args)
    return timed

def _rmr_time_service(name):
    try:
        svc = sm.GetService(name)
    except:
        sys.exc_clear()
        return
    cls = svc.__class__
    if getattr(cls, '_rmr_timed', 0):
        return
    names = []
    for key, value in cls.__dict__.items():
        if key[:2] == '__' or type(value) is not types.FunctionType:
            continue
        names.append(key)
        setattr(cls, key, _rmr_timed_method(name, key, value))
    cls._rmr_timed = 1
    names.sort()
    _rmr_report('client timing: watching %s (%s)' % (name, ', '.join(names)))

def _rmr_install_slow_watch():
    import time
    while not getattr(eve.session, 'charid', None):
        blue.pyos.synchro.Sleep(1000)
    blue.pyos.synchro.Sleep(5000)
    for name in ('photo', 'inbox', 'addressbook'):
        try:
            _rmr_time_service(name)
        except:
            _rmr_log('Slow-call watch on %s failed: %s' % (name, sys.exc_info()[1]))
            sys.exc_clear()
    last = time.clock()
    flushed = last
    while 1:
        blue.pyos.synchro.Sleep(100)
        now = time.clock()
        gap = (now - last) * 1000.0 - 100.0
        last = now
        if gap >= 250.0:
            _rmr_stalls[0] = _rmr_stalls[0] + 1
            _rmr_stalls[1] = _rmr_stalls[1] + gap
            if gap > _rmr_stalls[2]:
                _rmr_stalls[2] = gap
        if now - flushed < 20.0:
            continue
        flushed = now
        if not _rmr_slow_calls and not _rmr_stalls[0]:
            continue
        lines = []
        items = _rmr_slow_calls.items()
        items.sort(lambda a, b: cmp(b[1][1], a[1][1]))
        for key, entry in items[:12]:
            lines.append('%s x%d total %dms max %dms %s' % (key, entry[0], entry[1], entry[2], entry[3]))
        _rmr_slow_calls.clear()
        if _rmr_stalls[0]:
            lines.append('frame stalls x%d total %dms max %dms' % (_rmr_stalls[0], _rmr_stalls[1], _rmr_stalls[2]))
            _rmr_stalls[0] = 0
            _rmr_stalls[1] = 0.0
            _rmr_stalls[2] = 0.0
        try:
            where = getattr(eve.session, 'stationid', None) and 'docked' or 'in space'
        except:
            where = ''
            sys.exc_clear()
        _rmr_report('client timing (%s): %s' % (where, ' | '.join(lines)))

uthread.new(_rmr_install_slow_watch)

def _rmr_install_area_doomsday():
    # An area doomsday has no targetID. The old confirmation path tries to
    # read that missing target's ownerID before Activate reaches the server.
    while not getattr(eve.session, 'charid', None):
        blue.pyos.synchro.Sleep(1000)
    try:
        consider = sm.StartService('consider')
        cls = consider.__class__
        if getattr(cls, '_rmr_area_doomsday_installed', False):
            return
        original = cls.DoAttackConfirmations
        def confirm(self, targetID, *args, **kwargs):
            # Drone Engage calls this with only targetID. Forward the exact
            # arguments it supplied so the original confirmation still runs.
            effect = kwargs.get('effect')
            if args:
                effect = args[0]
            if targetID is None and getattr(effect, 'effectID', None) == 2071:
                _rmr_log('Area doomsday confirmation accepted without a target.')
                return 1
            return original(self, targetID, *args, **kwargs)
        cls.DoAttackConfirmations = confirm
        cls._rmr_area_doomsday_installed = True
        _rmr_log('Area doomsday client activation enabled.')
    except:
        _rmr_log('Area doomsday hook failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()

uthread.new(_rmr_install_area_doomsday)

# /moveme is a GM command. The server asks this client to choose one of its
# allowlisted stations; the selection goes back through the normal slash RPC.
_rmr_hubs = [
    ('HIGHSEC  |  Jita', 'jita'),
    ('HIGHSEC  |  Amarr', 'amarr'),
    ('HIGHSEC  |  Rens', 'rens'),
    ('HIGHSEC  |  Dodixie', 'dodixie'),
    ('HIGHSEC  |  Hek', 'hek'),
    ('NULLSEC  |  T22-QI', 't22-qi'),
    ('NULLSEC  |  FD-MLJ', 'fd-mlj'),
    ('NULLSEC  |  X-7OMU', 'x-7omu'),
    ('NULLSEC  |  VFK-IV', 'vfk-iv'),
    ('NULLSEC  |  6NJ8-V', '6nj8-v'),
    ('NULLSEC  |  NOL-M9', 'nol-m9')
]

def _rmr_moveme_key(value, depth=0):
    if value is None or depth > 3:
        return None
    for label, key in _rmr_hubs:
        if value == label or value == key:
            return key
    if type(value) in (types.ListType, types.TupleType):
        for part in value:
            key = _rmr_moveme_key(part, depth + 1)
            if key:
                return key
    else:
        for field in ('value', 'returnValue', 'itemID', 'id', 'data'):
            try:
                key = _rmr_moveme_key(getattr(value, field), depth + 1)
                if key:
                    return key
            except:
                sys.exc_clear()
    return None

def _rmr_show_moveme():
    try:
        import uix
        choice = uix.ListWnd(_rmr_hubs, 'generic', 'Move Me - market hubs', ordered=1)
        key = _rmr_moveme_key(choice)
        _rmr_log('Move Me selection: ' + repr(choice) + ' -> ' + repr(key))
        if key:
            eve.RemoteSvc('slash').SlashCmd('/moveme ' + key)
    except:
        _rmr_log('Move Me dialog failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()

def _rmr_install_moveme():
    while not getattr(eve.session, 'charid', None):
        blue.pyos.synchro.Sleep(1000)
    try:
        # MachoNet sends OnRemoteMessage directly to eve.Message, not to a
        # service subscribed through sm.RegisterNotify.
        original = eve.Message
        def message(key, *args, **kwargs):
            if key == 'RmrMoveMe':
                uthread.new(_rmr_show_moveme)
                return None
            return original(key, *args, **kwargs)
        try:
            eve.Message = message
            _rmr_log('Move Me dialog hook installed on eve.Message.')
        except:
            sys.exc_clear()
            cls = eve.__class__
            original_method = cls.Message
            def class_message(self, key, *args, **kwargs):
                if key == 'RmrMoveMe':
                    uthread.new(_rmr_show_moveme)
                    return None
                return original_method(self, key, *args, **kwargs)
            cls.Message = class_message
            _rmr_log('Move Me dialog hook installed on Eve.Message.')
    except:
        _rmr_log('Move Me hook failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()

uthread.new(_rmr_install_moveme)


def _rmr_stream_agent_standings(sheet, key, generation):
    # Build 200 entries at a time. The original Character Sheet builds every
    # standing and every skill bonus before it shows anything, freezing at
    # /maxstandings scale. The server keeps that old cache small and pages the
    # complete agent relationships here.
    try:
        import listentry
        blue.pyos.synchro.Sleep(10)
        wnd = sheet.GetWnd()
        if wnd is None or wnd.destroyed:
            return
        scroll = wnd.sr.scroll
        standing = sm.StartService('standing')
        has_header = 0
        for node in scroll.GetNodes():
            if getattr(node, 'label', None) == 'Agents':
                has_header = 1
                break
        offset = 0
        positive = key == 'mystandings_to_positive'
        while 1:
            if (getattr(sheet, '_rmr_agent_standings_generation', None) != generation or
                    sheet.showing != key or wnd.destroyed or wnd.sr.scroll is not scroll):
                return
            page = eve.RemoteSvc('standing2').GetAgentStandingsPage(offset, int(positive))
            if not page:
                break
            ids = [record.agentID for record in page]
            cfg.eveowners.Prime(ids)
            entries = []
            if not has_header:
                entries.append(listentry.Get('Header', {'label': 'Agents'}))
                has_header = 1
            for record in page:
                # A direct +10 is already at the effective-standing cap;
                # calculating skill bonuses for thousands of agents is wasted.
                calculate_bonuses = record.standing < 10.0
                entries.append(standing.GetStandingEntry(
                    record.agentID, record.agentID, eve.session.charid,
                    record.standing, int(calculate_bonuses)))
            if (getattr(sheet, '_rmr_agent_standings_generation', None) != generation or
                    sheet.showing != key or wnd.destroyed):
                return
            scroll.AddEntries(-1, entries, checkTabstops=0)
            offset += len(page)
            if len(page) < 200:
                break
            blue.pyos.synchro.Sleep(10)
        _rmr_log('Character Sheet streamed %s additional agent standings.' % offset)
    except:
        _rmr_log('Agent standings stream failed: ' +
                 ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()


def _rmr_install_agent_standings():
    while not getattr(eve.session, 'charid', None):
        blue.pyos.synchro.Sleep(1000)
    try:
        sheet = sm.StartService('charactersheet')
        cls = sheet.__class__
        if getattr(cls, '_rmr_agent_standings_installed', False):
            return
        original = cls.ShowStandings
        def show_standings(self, key):
            generation = getattr(self, '_rmr_agent_standings_generation', 0) + 1
            self._rmr_agent_standings_generation = generation
            result = original(self, key)
            if key in ('mystandings_to_positive', 'mystandings_to_negative'):
                uthread.new(_rmr_stream_agent_standings, self, key, generation)
            return result
        cls.ShowStandings = show_standings
        cls._rmr_agent_standings_installed = True
        _rmr_log('Paged Character Sheet agent standings installed.')
    except:
        _rmr_log('Agent standings hook failed: ' +
                 ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()


uthread.new(_rmr_install_agent_standings)

def _rmr_install_rush_job():
    while not getattr(eve.session, 'charid', None):
        blue.pyos.synchro.Sleep(1000)
    try:
        import uix
        cls = form.Manufacturing
        if getattr(cls, '_rmr_rush_job_installed', False):
            return
        original = cls.ShowJobDetails
        # CCP, GM and BH carry role 8; ISD BH carries 2048. Matches the server's RUSH_JOB_ROLES.
        gm_role = 8 | 2048

        def show_job_details(self, job):
            result = original(self, job)
            if (eve.session.role & gm_role and job and not job.completed
                    and job.installedItemOwnerID == eve.session.charid
                    and job.endProductionTime > blue.os.GetTime()):
                def rush_job(*args):
                    eve.ProxySvc('ramProxy').RushJob(job.jobID)
                    button.Close()
                    self.LoadJobs()

                # Cancel Job is anchored 10 pixels from the bottom.
                button = uix.GetButton(self.sr.details, 'Rush Job', func=rush_job, left=12, top=32)
                button.anchors = uix.UI_ANCHRIGHT | uix.UI_ANCHBOTTOM
            return result

        cls.ShowJobDetails = show_job_details
        cls._rmr_rush_job_installed = True
    except:
        _rmr_log('Rush Job hook failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()


uthread.new(_rmr_install_rush_job)


def _rmr_install_station_management():
    # form.StationManagement.Startup calls dlg.Show() but the dialog's
    # Show(self, show) has no default, so opening the window ends in a
    # TypeError after it is built. Give 'show' its obvious default.
    while not getattr(eve.session, 'charid', None):
        blue.pyos.synchro.Sleep(1000)
    try:
        dialog = form.StationManagement.Startup.im_func.func_globals.get('StationManagementDialog')
        if dialog is None or getattr(dialog, '_rmr_show_installed', False):
            return
        original = dialog.Show

        def show(self, show=1):
            return original(self, show)

        dialog.Show = show
        dialog._rmr_show_installed = True
    except:
        _rmr_log('Station management hook failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))


uthread.new(_rmr_install_station_management)

def _rmr_install_manufacturing_menu():
    while not getattr(eve.session, 'charid', None):
        blue.pyos.synchro.Sleep(1000)
    try:
        cls = form.Manufacturing
        if getattr(cls, '_rmr_manufacturing_menu_installed', False):
            return
        original = cls.GetMenu

        # menu.GetSingleMenu calls GetMenu() with no arguments when the
        # window background is right-clicked; the client's own method needs one.
        def get_menu(self, *args):
            if not args:
                return []
            return original(self, *args)

        cls.GetMenu = get_menu
        cls._rmr_manufacturing_menu_installed = True
    except:
        _rmr_log('Manufacturing menu hook failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()


uthread.new(_rmr_install_manufacturing_menu)


def _rmr_report(message):
    _rmr_log(message)
    try:
        eve.RemoteSvc('factory').RmrReport(str(message)[:2000])
    except:
        sys.exc_clear()


class _RmrQuietFlag:
    # Off when tested, and does nothing if called.
    def __call__(self, *args, **kwargs):
        return None

    def __nonzero__(self):
        return False


def _rmr_install_tactical_logme():
    # Right-clicking a location reaches tactical.GetDefaultGroups, which reads
    # self.logme; the service never sets it, so every such menu failed.
    while not getattr(eve.session, 'charid', None):
        blue.pyos.synchro.Sleep(1000)
    try:
        cls = None
        try:
            cls = getattr(__import__('svc'), 'TacticalSvc', None)
        except:
            sys.exc_clear()
        if cls is None:
            cls = sm.StartService('tactical').__class__
        if hasattr(cls, 'logme'):
            _rmr_report('tactical logme: already defined on %s' % cls)
        else:
            cls.logme = _RmrQuietFlag()
            _rmr_report('tactical logme: default installed on %s' % cls)
    except:
        _rmr_report('tactical logme hook failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()


uthread.new(_rmr_install_tactical_logme)


def _rmr_outpost_item(values):
    # The platform the menu is for: a slim item (space) or an inventory row.
    for value in values:
        try:
            typeID = getattr(value, 'typeID', None)
            itemID = getattr(value, 'itemID', None)
            if typeID and itemID and cfg.invtypes.Get(typeID).groupID == 307:
                return itemID
        except:
            sys.exc_clear()
    return None


def _rmr_menu_item(values):
    # The item a menu is for: a slim item (space) or an inventory row.
    for value in values:
        typeID = getattr(value, 'typeID', None)
        itemID = getattr(value, 'itemID', None)
        if typeID and itemID:
            return itemID, typeID
    return None, None


def _rmr_slash_notify(command):
    reply = eve.RemoteSvc('slash').SlashCmd(command)
    try:
        eve.Message('CustomNotify', {'notify': reply})
    except:
        sys.exc_clear()
        _rmr_log(command + ': ' + str(reply))


def _rmr_notify_text(text):
    try:
        eve.Message('CustomNotify', {'notify': text})
    except:
        sys.exc_clear()


def _rmr_time_left(seconds):
    seconds = max(0, int(seconds))
    if seconds >= 3600:
        return '%dh %02dm' % (seconds / 3600, seconds % 3600 / 60)
    if seconds >= 60:
        return '%dm %02ds' % (seconds / 60, seconds % 60)
    return '%ds' % seconds


def _rmr_structure_timer(itemID):
    # (label, seconds left) of a structure's running timer, from its slim item.
    try:
        park = sm.StartService('michelle').GetBallpark()
        slim = park and park.slimItems.get(itemID)
        ends = slim and getattr(slim, 'rmrStepEnds', None)
        if not ends:
            return None, None
        left = (ends - blue.os.GetTime()) / 10000000
        if left <= 0:
            return None, None
        return getattr(slim, 'rmrStepName', None) or 'Timer', left
    except:
        sys.exc_clear()
        return None, None


def _rmr_outpost_mats(itemID):
    reply = eve.RemoteSvc('slash').SlashCmd('/outpostmats %d' % itemID)
    try:
        eve.Message('CustomNotify', {'notify': reply})
    except:
        sys.exc_clear()
        _rmr_log('Outpost materials: ' + str(reply))


def _rmr_install_outpost_gm_menu():
    # GM / WM Extras on an outpost construction platform: fill its resource
    # hold with whatever Show Info's Material Requirements says is missing.
    while not getattr(eve.session, 'charid', None):
        blue.pyos.synchro.Sleep(1000)
    try:
        cls = sm.StartService('menu').__class__
        if getattr(cls, '_rmr_outpost_menu_installed', False):
            return
        original = getattr(cls, 'GetGMMenu', None)
        if original is None:
            _rmr_report('outpost GM menu: the menu service has no GetGMMenu')
            return

        def get_gm_menu(self, *args, **kwargs):
            menu = original(self, *args, **kwargs)
            try:
                values = list(args) + kwargs.values()
                itemID, typeID = _rmr_menu_item(values)
                if itemID is None or not menu:
                    return menu
                entries = []
                step, left = _rmr_structure_timer(itemID)
                if step:
                    import time
                    ends = time.strftime('%H:%M', time.gmtime(time.time() + left))
                    label = '%s: %s left (%s EVE)' % (step, _rmr_time_left(left), ends)
                    entries.append((label, _rmr_notify_text, (label,)))
                    entries.append(('Rush %s' % step.lower(), _rmr_slash_notify, ('/rushanchor %d' % itemID,)))
                if _rmr_outpost_item(values) is not None:
                    entries.append(('Spawn outpost materials', _rmr_outpost_mats, (itemID,)))
                if not entries:
                    return menu
                for item in menu:
                    if (type(item) in (types.TupleType, types.ListType) and len(item) >= 2 and
                            type(item[1]) is types.ListType and 'GM' in str(item[0]).upper()):
                        item[1].append(None)
                        item[1].extend(entries)
                        return menu
                menu.extend(entries)
            except:
                _rmr_report('outpost GM menu entry failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
                sys.exc_clear()
            return menu

        cls.GetGMMenu = get_gm_menu
        cls._rmr_outpost_menu_installed = True
        _rmr_report('outpost GM menu: installed on %s' % cls)
    except:
        _rmr_report('outpost GM menu hook failed: ' + ''.join(traceback.format_exception(*sys.exc_info())))
        sys.exc_clear()


uthread.new(_rmr_install_outpost_gm_menu)


'@



# ---- the launcher ------------------------------------------------------------------------------

if (-not ('RmrClient.SkyForm' -as [type])) {
    Add-Type -ReferencedAssemblies @('System.dll', 'System.Windows.Forms.dll', 'System.Drawing.dll') -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Windows.Forms;

namespace RmrClient {
    public sealed class SkyForm : Form {
        private readonly Timer animation;
        private readonly DateTime started = DateTime.UtcNow;
        public SkyForm() {
            SetStyle(ControlStyles.AllPaintingInWmPaint | ControlStyles.OptimizedDoubleBuffer | ControlStyles.ResizeRedraw, true);
            animation = new Timer();
            animation.Interval = 200;
            animation.Tick += delegate { if (Visible && WindowState != FormWindowState.Minimized && Form.ActiveForm == this) Invalidate(false); };
            animation.Start();
        }
        protected override void OnPaintBackground(PaintEventArgs e) {
            Rectangle bounds = ClientRectangle;
            if (bounds.Width < 1 || bounds.Height < 1) return;
            Graphics g = e.Graphics;
            g.SmoothingMode = SmoothingMode.AntiAlias;
            using (var background = new LinearGradientBrush(bounds, Color.FromArgb(6, 10, 18), Color.FromArgb(22, 10, 14), 35f))
                g.FillRectangle(background, bounds);
            // A red moon low on the right, as on the box art.
            int r = Math.Max(bounds.Height, 400);
            var moon = new Rectangle(bounds.Width - r / 2, bounds.Height - r / 3, r, r);
            using (var path = new GraphicsPath()) {
                path.AddEllipse(moon);
                using (var glow = new PathGradientBrush(path)) {
                    glow.CenterColor = Color.FromArgb(120, 150, 30, 26);
                    glow.SurroundColors = new Color[] { Color.FromArgb(0, 40, 6, 8) };
                    g.FillEllipse(glow, moon);
                }
            }
            double time = (DateTime.UtcNow - started).TotalSeconds;
            var random = new Random(3913);
            for (int i = 0; i < 170; i++) {
                float x = (float)random.NextDouble() * bounds.Width;
                float y = (float)random.NextDouble() * bounds.Height;
                double phase = random.NextDouble() * Math.PI * 2;
                double speed = 0.3 + random.NextDouble() * 0.5;
                float radius = i % 13 == 0 ? 1.4f : 0.7f;
                int alpha = (int)(30 + 110 * (0.5 + 0.5 * Math.Sin(time * speed + phase)));
                using (var star = new SolidBrush(Color.FromArgb(alpha, 200, 215, 230)))
                    g.FillEllipse(star, x, y, radius * 2, radius * 2);
            }
        }
        protected override void Dispose(bool disposing) {
            if (disposing) { animation.Stop(); animation.Dispose(); }
            base.Dispose(disposing);
        }
    }
}
'@
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

# Only localhost and local-network IPv4 addresses, unless this launcher was
# once started with --allow-public (remembered in its settings).
function Test-RmrAllowedHost([string]$HostName) {
    if ($HostName -eq 'localhost' -or (Test-RmrPrivateIPv4 $HostName)) { return $true }
    return ($env:RMR_ALLOW_PUBLIC -eq '1')
}

function Split-RmrServer([string]$Text) {
    $Text = $Text.Trim()
    if (-not $Text -or $Text -in @('local', 'localhost', 'this pc')) { $Text = '127.0.0.1' }
    $hostName = $Text; $port = 26000
    $m = [regex]::Match($Text, '^(.+):(\d{1,5})$')
    if ($m.Success) { $hostName = $m.Groups[1].Value; $port = [int]$m.Groups[2].Value }
    if ($hostName -notmatch '^[A-Za-z0-9.-]+$') { throw "That is not a server address: $Text" }
    if (-not (Test-RmrAllowedHost $hostName)) { throw "Use localhost, or the server's IPv4 address on your local network (such as 192.168.1.20). Red Moon Rising is for your own PC and local network only." }
    if ($port -lt 1 -or $port -gt 65535) { throw "That port is out of range: $port" }
    return [pscustomobject]@{ Host = $hostName; Port = $port }
}

# ---- settings: server list, chosen server, client folder ----

function New-RmrServer([string]$Name, [string]$HostName, [int]$Port, [int]$NewsPort) {
    return [pscustomobject]@{ name = $Name; host = $HostName; port = $Port; newsPort = $NewsPort }
}

$settings = [pscustomobject]@{ servers = @(); selected = 0; clientPath = ''; closeOnPlay = $false; allowPublic = $false }
if (Test-Path -LiteralPath $SettingsPath) {
    try {
        $saved = Get-Content -Raw -LiteralPath $SettingsPath | ConvertFrom-Json
        if ($saved.PSObject.Properties['servers'] -and $saved.servers) {
            foreach ($s in @($saved.servers)) {
                $newsPort = 18080
                if ($s.PSObject.Properties['newsPort'] -and $s.newsPort) { $newsPort = [int]$s.newsPort }
                $settings.servers += (New-RmrServer ([string]$s.name) ([string]$s.host) ([int]$s.port) $newsPort)
            }
        } elseif ($saved.PSObject.Properties['server'] -and $saved.server) {
            # The first Client.bat kept one address.
            $one = Split-RmrServer ([string]$saved.server)
            $label = 'My server'
            if ($one.Host -eq '127.0.0.1') { $label = 'This PC' }
            $settings.servers += (New-RmrServer $label $one.Host $one.Port 18080)
        }
        if ($saved.PSObject.Properties['selected']) { $settings.selected = [int]$saved.selected }
        if ($saved.PSObject.Properties['clientPath']) { $settings.clientPath = [string]$saved.clientPath }
        if ($saved.PSObject.Properties['closeOnPlay']) { $settings.closeOnPlay = [bool]$saved.closeOnPlay }
        if ($saved.PSObject.Properties['allowPublic']) { $settings.allowPublic = [bool]$saved.allowPublic }
    } catch { }
}
if (@($settings.servers).Count -eq 0) { $settings.servers = @(New-RmrServer 'This PC' '127.0.0.1' 26000 18080) }
if ($env:RMR_ALLOW_PUBLIC -eq '1') { $settings.allowPublic = $true; [IO.File]::WriteAllText($SettingsPath, ($settings | ConvertTo-Json -Depth 4), $Utf8) }
elseif ($settings.allowPublic) { $env:RMR_ALLOW_PUBLIC = '1' }

function Save-RmrSettings {
    [IO.File]::WriteAllText($SettingsPath, ($settings | ConvertTo-Json -Depth 4), $Utf8)
}

# Beside a server, its dashboard's killboard shows portraits from this client's cache.
function Save-RmrServerClientPath([string]$ClientPath) {
    $serverSettings = Join-Path $Root '.rmr\settings.json'
    if (-not (Test-Path -LiteralPath $serverSettings)) { return }
    try {
        $s = Get-Content -Raw -LiteralPath $serverSettings | ConvertFrom-Json
        $s | Add-Member -NotePropertyName clientPath -NotePropertyValue $ClientPath -Force
        [IO.File]::WriteAllText($serverSettings, ($s | ConvertTo-Json), $Utf8)
    } catch { }
}

# The client folder: the saved one, else found nearby, else picked by hand.
function Get-RmrClientFolder([switch]$Choose) {
    if (-not $Choose -and $settings.clientPath -and (Test-RmrClient $settings.clientPath)) { return $settings.clientPath }
    $found = $null
    if ($Choose) {
        $picked = Select-RmrClientFolder -InvocationDirectory $env:RMR_INVOCATION_DIRECTORY -Description 'Select your EVE 2.10 build 3913 client folder'
        if (-not $picked) { return $null }
        if (-not (Test-RmrClient $picked)) { throw "That folder is not an EVE 2.10 build 3913 client: $picked" }
        $found = [IO.Path]::GetFullPath($picked)
    } else {
        $found = Resolve-RmrClient -InvocationDirectory $env:RMR_INVOCATION_DIRECTORY -RepositoryRoot $Root -SavedClientDirectory $settings.clientPath -ClientDirectory $env:RMR_CLIENT_DIRECTORY
        if (-not $found) { return $null }
    }
    $settings.clientPath = $found
    Save-RmrSettings
    Save-RmrServerClientPath $found
    return $found
}

# ---- command line: Client.bat ADDRESS[:PORT] connects without the window ----

if ($env:RMR_SERVER_ARG) {
    try {
        $server = Split-RmrServer $env:RMR_SERVER_ARG
        $client = Get-RmrClientFolder
        if (-not $client) { exit 0 }
        $prepared = Prepare-RmrClient -ClientDirectory $client -ServerBind $server.Host -ServerPort $server.Port
        if ($env:RMR_NO_LAUNCH -ne '1') { Start-RmrGameClient -ClientPath $prepared.ClientDirectory -GamePort $server.Port -ServerHost $server.Host }
    } catch {
        [Windows.Forms.MessageBox]::Show($_.Exception.Message, 'Red Moon Rising', 'OK', 'Error') | Out-Null
        exit 1
    }
    exit 0
}

# ---- server editor ----

function Edit-RmrServerEntry($Entry) {
    $dialog = New-Object Windows.Forms.Form
    $dialog.Text = if ($Entry) { 'Edit server' } else { 'Add a server' }
    $dialog.FormBorderStyle = 'FixedDialog'
    $dialog.MaximizeBox = $false; $dialog.MinimizeBox = $false
    $dialog.StartPosition = 'CenterParent'
    $dialog.ClientSize = New-Object Drawing.Size(420, 268)
    $dialog.BackColor = [Drawing.Color]::FromArgb(14, 18, 24)
    $dialog.ForeColor = [Drawing.Color]::FromArgb(220, 228, 236)
    $dialog.Font = New-Object Drawing.Font('Segoe UI', 10)
    $fields = @()
    $y = 16
    foreach ($spec in @(@('Name', 'name', 'My server'), @('Address (localhost or a local-network IPv4 address)', 'host', ''), @('Game port', 'port', '26000'), @('News port (the server''s Dashboard.bat port)', 'newsPort', '18080'))) {
        $label = New-Object Windows.Forms.Label
        $label.Text = $spec[0]
        $label.ForeColor = [Drawing.Color]::FromArgb(140, 150, 162)
        $label.SetBounds(18, $y, 384, 20)
        $box = New-Object Windows.Forms.TextBox
        $box.SetBounds(18, ($y + 20), 384, 24)
        $box.BackColor = [Drawing.Color]::FromArgb(24, 31, 40)
        $box.ForeColor = [Drawing.Color]::White
        $box.BorderStyle = 'FixedSingle'
        $box.Text = if ($Entry) { [string]$Entry.($spec[1]) } else { $spec[2] }
        $dialog.Controls.Add($label); $dialog.Controls.Add($box)
        $fields += $box
        $y += 50
    }
    $ok = New-Object Windows.Forms.Button
    $ok.Text = 'Save'
    $ok.DialogResult = 'OK'
    $ok.FlatStyle = 'Flat'
    $ok.BackColor = [Drawing.Color]::FromArgb(79, 202, 182)
    $ok.ForeColor = [Drawing.Color]::FromArgb(6, 18, 15)
    $ok.SetBounds(222, 226, 86, 30)
    $cancel = New-Object Windows.Forms.Button
    $cancel.Text = 'Cancel'
    $cancel.DialogResult = 'Cancel'
    $cancel.FlatStyle = 'Flat'
    $cancel.SetBounds(316, 226, 86, 30)
    $dialog.Controls.Add($ok); $dialog.Controls.Add($cancel)
    $dialog.AcceptButton = $ok; $dialog.CancelButton = $cancel
    try {
        while ($true) {
            if ($dialog.ShowDialog($form) -ne [Windows.Forms.DialogResult]::OK) { return $null }
            try {
                $name = $fields[0].Text.Trim()
                if (-not $name) { throw 'Give the server a name.' }
                $address = Split-RmrServer ($fields[1].Text.Trim())
                $port = 0; $news = 0
                if (-not [int]::TryParse($fields[2].Text.Trim(), [ref]$port) -or $port -lt 1 -or $port -gt 65535) { throw 'The game port must be a number from 1 to 65535.' }
                if (-not [int]::TryParse($fields[3].Text.Trim(), [ref]$news) -or $news -lt 1 -or $news -gt 65535) { throw 'The news port must be a number from 1 to 65535.' }
                return (New-RmrServer $name $address.Host $port $news)
            } catch {
                [Windows.Forms.MessageBox]::Show($dialog, $_.Exception.Message, 'Red Moon Rising', 'OK', 'Warning') | Out-Null
            }
        }
    } finally { $dialog.Dispose() }
}

# ---- the window ----

[Windows.Forms.Application]::EnableVisualStyles()
$bright = [Drawing.Color]::FromArgb(236, 242, 248)
$muted = [Drawing.Color]::FromArgb(140, 152, 166)
$cyan = [Drawing.Color]::FromArgb(79, 202, 182)
$red = [Drawing.Color]::FromArgb(224, 83, 63)
$panel = [Drawing.Color]::FromArgb(12, 17, 24)

$form = New-Object RmrClient.SkyForm
$form.Text = 'Red Moon Rising'
$form.StartPosition = 'CenterScreen'
$form.ClientSize = New-Object Drawing.Size(1180, 700)
$form.MinimumSize = New-Object Drawing.Size(980, 620)
$form.ForeColor = $bright
$form.Font = New-Object Drawing.Font('Segoe UI', 10)

$title = New-Object Windows.Forms.Label
$title.Text = 'RED MOON RISING'
$title.Font = New-Object Drawing.Font('Segoe UI Light', 26)
$title.ForeColor = $bright
$title.BackColor = [Drawing.Color]::Transparent
$title.SetBounds(24, 14, 520, 50)
$subtitle = New-Object Windows.Forms.Label
$subtitle.Text = 'EVE ONLINE 2.10  /  BUILD 3913  /  LAUNCHER'
$subtitle.Font = New-Object Drawing.Font('Segoe UI', 8.5, [Drawing.FontStyle]::Bold)
$subtitle.ForeColor = $red
$subtitle.BackColor = [Drawing.Color]::Transparent
$subtitle.SetBounds(28, 62, 520, 18)
$statusPill = New-Object Windows.Forms.Label
$statusPill.TextAlign = 'MiddleRight'
$statusPill.Font = New-Object Drawing.Font('Segoe UI', 9.5, [Drawing.FontStyle]::Bold)
$statusPill.BackColor = [Drawing.Color]::Transparent
$statusPill.Anchor = 'Top,Right'
$statusPill.SetBounds(($form.ClientSize.Width - 524), 28, 500, 24)

# Left: the servers this launcher knows, and the client.
$side = New-Object Windows.Forms.Panel
$side.BackColor = $panel
$side.Anchor = 'Top,Bottom,Left'
$side.SetBounds(24, 96, 300, ($form.ClientSize.Height - 120))
$serversCaption = New-Object Windows.Forms.Label
$serversCaption.Text = 'SERVERS'
$serversCaption.Font = New-Object Drawing.Font('Segoe UI', 8.5, [Drawing.FontStyle]::Bold)
$serversCaption.ForeColor = $muted
$serversCaption.BackColor = [Drawing.Color]::Transparent
$serversCaption.SetBounds(16, 14, 200, 18)
$list = New-Object Windows.Forms.ListBox
$list.DrawMode = 'OwnerDrawFixed'
$list.ItemHeight = 56
$list.BorderStyle = 'None'
$list.BackColor = [Drawing.Color]::FromArgb(12, 17, 24)
$list.ForeColor = $bright
$list.IntegralHeight = $false
$list.Anchor = 'Top,Bottom,Left,Right'
$list.SetBounds(12, 38, 276, ($side.Height - 270))
$addButton = New-Object Windows.Forms.Button
$editButton = New-Object Windows.Forms.Button
$removeButton = New-Object Windows.Forms.Button
$x = 12
foreach ($pair in @(@($addButton, '+ Add'), @($editButton, 'Edit'), @($removeButton, 'Remove'))) {
    $b = $pair[0]
    $b.Text = $pair[1]
    $b.FlatStyle = 'Flat'
    $b.FlatAppearance.BorderColor = [Drawing.Color]::FromArgb(44, 54, 66)
    $b.BackColor = [Drawing.Color]::FromArgb(20, 27, 36)
    $b.ForeColor = $bright
    $b.Anchor = 'Bottom,Left'
    $b.SetBounds($x, ($side.Height - 222), 88, 30)
    $x += 94
}
$clientCaption = New-Object Windows.Forms.Label
$clientCaption.Text = 'EVE CLIENT'
$clientCaption.Font = $serversCaption.Font
$clientCaption.ForeColor = $muted
$clientCaption.BackColor = [Drawing.Color]::Transparent
$clientCaption.Anchor = 'Bottom,Left'
$clientCaption.SetBounds(16, ($side.Height - 176), 200, 18)
$clientLabel = New-Object Windows.Forms.Label
$clientLabel.AutoEllipsis = $true
$clientLabel.ForeColor = $bright
$clientLabel.BackColor = [Drawing.Color]::Transparent
$clientLabel.Anchor = 'Bottom,Left'
$clientLabel.SetBounds(16, ($side.Height - 154), 272, 40)
$folderButton = New-Object Windows.Forms.Button
$repairButton = New-Object Windows.Forms.Button
$x = 12
foreach ($pair in @(@($folderButton, 'Change folder'), @($repairButton, 'Repair client'))) {
    $b = $pair[0]
    $b.Text = $pair[1]
    $b.FlatStyle = 'Flat'
    $b.FlatAppearance.BorderColor = [Drawing.Color]::FromArgb(44, 54, 66)
    $b.BackColor = [Drawing.Color]::FromArgb(20, 27, 36)
    $b.ForeColor = $bright
    $b.Anchor = 'Bottom,Left'
    $b.SetBounds($x, ($side.Height - 106), 135, 30)
    $x += 141
}
$closeBox = New-Object Windows.Forms.CheckBox
$closeBox.Text = 'Close the launcher when EVE starts'
$closeBox.ForeColor = $muted
$closeBox.BackColor = [Drawing.Color]::Transparent
$closeBox.Anchor = 'Bottom,Left'
$closeBox.Checked = [bool]$settings.closeOnPlay
$closeBox.SetBounds(16, ($side.Height - 62), 272, 24)
$side.Controls.AddRange(@($serversCaption, $list, $addButton, $editButton, $removeButton, $clientCaption, $clientLabel, $folderButton, $repairButton, $closeBox))

# Right: the chosen server's news, and Play.
$newsCaption = New-Object Windows.Forms.Label
$newsCaption.Font = $serversCaption.Font
$newsCaption.ForeColor = $muted
$newsCaption.BackColor = [Drawing.Color]::Transparent
$newsCaption.SetBounds(348, 100, 600, 18)
$news = New-Object Windows.Forms.ListBox
$news.DrawMode = 'OwnerDrawFixed'
$news.ItemHeight = 78
$news.BorderStyle = 'None'
$news.BackColor = [Drawing.Color]::FromArgb(12, 17, 24)
$news.ForeColor = $bright
$news.IntegralHeight = $false
$news.Anchor = 'Top,Bottom,Left,Right'
$news.SetBounds(348, 124, ($form.ClientSize.Width - 372), ($form.ClientSize.Height - 240))
$playButton = New-Object Windows.Forms.Button
$playButton.Text = 'PLAY'
$playButton.Font = New-Object Drawing.Font('Segoe UI Semibold', 20)
$playButton.FlatStyle = 'Flat'
$playButton.FlatAppearance.BorderSize = 0
$playButton.BackColor = $cyan
$playButton.ForeColor = [Drawing.Color]::FromArgb(6, 18, 15)
$playButton.Cursor = [Windows.Forms.Cursors]::Hand
$playButton.Anchor = 'Bottom,Right'
$playButton.SetBounds(($form.ClientSize.Width - 284), ($form.ClientSize.Height - 96), 260, 72)
$playStatus = New-Object Windows.Forms.Label
$playStatus.TextAlign = 'MiddleRight'
$playStatus.ForeColor = $muted
$playStatus.BackColor = [Drawing.Color]::Transparent
$playStatus.Anchor = 'Bottom,Left,Right'
$playStatus.SetBounds(348, ($form.ClientSize.Height - 90), ($form.ClientSize.Width - 648), 60)

$form.Controls.AddRange(@($title, $subtitle, $statusPill, $side, $newsCaption, $news, $playButton, $playStatus))

# ---- state ----

$ui = @{ Status = @{}; Checks = @{}; NewsTask = $null; NewsClient = $null; NewsFor = ''; NewsAt = [DateTime]::MinValue; Articles = @() }

function Get-RmrSelected {
    $i = $list.SelectedIndex
    if ($i -lt 0 -or $i -ge @($settings.servers).Count) { return $null }
    return @($settings.servers)[$i]
}

function Get-RmrServerKey($s) { return ([string]$s.host + ':' + [string]$s.port) }

# This PC's own addresses, and the address a server beside this file is hosted
# on. Most home routers cannot loop a connection to their own outside address
# back inside, so a server on this PC is always reached on 127.0.0.1.
$script:RmrOwnAddresses = @('127.0.0.1', 'localhost')
try {
    foreach ($nic in [Net.NetworkInformation.NetworkInterface]::GetAllNetworkInterfaces()) {
        foreach ($unicast in $nic.GetIPProperties().UnicastAddresses) { $script:RmrOwnAddresses += $unicast.Address.ToString() }
    }
} catch { }
try {
    $hostedSettings = Join-Path $Root '.rmr\settings.json'
    if (Test-Path -LiteralPath $hostedSettings) {
        $hosted = Get-Content -Raw -LiteralPath $hostedSettings | ConvertFrom-Json
        if ($hosted.PSObject.Properties['hostAddress'] -and $hosted.hostAddress -and $hosted.hostAddress -ne 'local') { $script:RmrOwnAddresses += [string]$hosted.hostAddress }
    }
} catch { }

function Test-RmrOnThisPc($s) { return ($script:RmrOwnAddresses -contains [string]$s.host) }

# Can host:port be reached now? (a quick, blocking check for Play)
function Test-RmrReachable([string]$HostName, [int]$Port) {
    $tcp = New-Object Net.Sockets.TcpClient
    try {
        $task = $tcp.ConnectAsync($HostName, $Port)
        return ($task.Wait(1500) -and $tcp.Connected)
    } catch { return $false }
    finally { $tcp.Close() }
}

# The address EVE is given: the one chosen. Only when that cannot be reached
# from here and the server runs on this PC (an outside address, which
# most routers will not loop back inside) does it go via 127.0.0.1.
function Get-RmrConnectHost($s) {
    $chosen = [string]$s.host
    if ($ui.Status[(Get-RmrServerKey $s)] -eq 'online') { return $chosen }
    if (Test-RmrReachable $chosen ([int]$s.port)) { return $chosen }
    if ((Test-RmrOnThisPc $s) -and (Test-RmrReachable '127.0.0.1' ([int]$s.port))) { return '127.0.0.1' }
    return $chosen
}

function Update-RmrServerList {
    $keep = $list.SelectedIndex
    $list.BeginUpdate()
    $list.Items.Clear()
    foreach ($s in @($settings.servers)) { [void]$list.Items.Add($s.name) }
    $list.EndUpdate()
    if ($list.Items.Count -gt 0) {
        $want = if ($keep -ge 0) { $keep } else { [int]$settings.selected }
        $list.SelectedIndex = [Math]::Max(0, [Math]::Min($list.Items.Count - 1, $want))
    }
}

function Update-RmrClientLabel {
    if ($settings.clientPath -and (Test-RmrClient $settings.clientPath)) { $clientLabel.Text = $settings.clientPath }
    else { $clientLabel.Text = 'Not chosen yet: found or asked for when you press Play.' }
}

function Update-RmrHeader {
    $s = Get-RmrSelected
    if (-not $s) { $statusPill.Text = ''; return }
    $state = $ui.Status[(Get-RmrServerKey $s)]
    if ($state -eq 'online') { $statusPill.ForeColor = $cyan; $statusPill.Text = ([string][char]0x25CF) + '  ' + $s.name.ToUpper() + '  ONLINE' }
    elseif ($state -eq 'offline') { $statusPill.ForeColor = $red; $statusPill.Text = ([string][char]0x25CF) + '  ' + $s.name.ToUpper() + '  OFFLINE' }
    else { $statusPill.ForeColor = $muted; $statusPill.Text = ([string][char]0x25CF) + '  ' + $s.name.ToUpper() + '  CHECKING' }
    $newsCaption.Text = 'NEWS FROM ' + $s.name.ToUpper()
    if (-not $ui.Busy) {
        $playStatus.Text = 'Connects to ' + $s.host + ':' + $s.port

    }
}

# Article text without its markup.
function ConvertTo-RmrText([string]$Html) {
    $t = $Html -replace '(?i)<br\s*/?>', "`n" -replace '(?i)</(p|h\d|li)>', "`n" -replace '(?i)<li>', ([string][char]0x2022 + ' ') -replace '<[^>]+>', ''
    return ([Net.WebUtility]::HtmlDecode($t) -replace "`n{3,}", "`n`n").Trim()
}

function Start-RmrNewsFetch {
    $s = Get-RmrSelected
    if (-not $s) { return }
    $key = Get-RmrServerKey $s
    if ($ui.NewsTask -and -not $ui.NewsTask.IsCompleted) { return }
    if ($ui.NewsFor -eq $key -and ([DateTime]::UtcNow - $ui.NewsAt).TotalSeconds -lt 60) { return }
    $ui.NewsFor = $key
    $ui.NewsAt = [DateTime]::UtcNow
    $url = 'http://' + $s.host + ':' + [int]$s.newsPort + '/rmr-news'
    if (Test-RmrOnThisPc $s) {
        # A server on this PC: its own dashboard, on the port Start.bat chose.
        $port = 18080
        try {
            $serverSettings = Join-Path $Root '.rmr\settings.json'
            if (Test-Path -LiteralPath $serverSettings) { $port = [int](Get-Content -Raw -LiteralPath $serverSettings | ConvertFrom-Json).webPort }
        } catch { }
        $url = 'http://127.0.0.1:' + $port + '/api/articles'
    }
    if ($ui.NewsClient) { $ui.NewsClient.Dispose() }
    $ui.NewsClient = New-Object Net.WebClient
    $ui.NewsClient.Encoding = [Text.Encoding]::UTF8
    try { $ui.NewsTask = $ui.NewsClient.DownloadStringTaskAsync($url) } catch { $ui.NewsTask = $null }
}

function Receive-RmrNews {
    $task = $ui.NewsTask
    if (-not $task -or -not $task.IsCompleted) { return }
    $ui.NewsTask = $null
    $articles = @()
    if ($task.Status -eq 'RanToCompletion') {
        try {
            foreach ($a in @($task.Result | ConvertFrom-Json)) {
                $when = ''
                try { $when = [DateTimeOffset]::FromUnixTimeMilliseconds([long]$a.time).LocalDateTime.ToString('d MMM yyyy  HH:mm') } catch { }
                $articles += [pscustomobject]@{ Title = [string]$a.title; When = $when; Text = (ConvertTo-RmrText ([string]$a.body)) }
                if ($articles.Count -ge 12) { break }
            }
        } catch { }
    }
    $ui.Articles = $articles
    $news.BeginUpdate()
    $news.Items.Clear()
    if ($articles.Count -eq 0) { [void]$news.Items.Add('none') }
    else { foreach ($a in $articles) { [void]$news.Items.Add($a.Title) } }
    $news.EndUpdate()
}

# Every few seconds: can each server's game port be reached?
function Update-RmrChecks {
    foreach ($s in @($settings.servers)) {
        $key = Get-RmrServerKey $s
        $check = $ui.Checks[$key]
        if ($check) {
            $age = ([DateTime]::UtcNow - $check.Started).TotalSeconds
            if ($check.Task.IsCompleted -or $age -gt 3) {
                $ok = $check.Task.IsCompleted -and $check.Task.Status -eq 'RanToCompletion' -and $check.Client.Connected
                $ui.Status[$key] = if ($ok) { 'online' } else { 'offline' }
                try { $check.Client.Close() } catch { }
                $ui.Checks.Remove($key)
                $ui.LastCheck = [DateTime]::UtcNow
            }
            continue
        }
        if ($ui.Status.ContainsKey($key) -and $ui.LastCheck -and ([DateTime]::UtcNow - $ui.LastCheck).TotalSeconds -lt 6) { continue }
        try {
            $tcp = New-Object Net.Sockets.TcpClient
            $ui.Checks[$key] = [pscustomobject]@{ Client = $tcp; Task = $tcp.ConnectAsync([string]$s.host, [int]$s.port); Started = [DateTime]::UtcNow }
        } catch { $ui.Status[$key] = 'offline' }
    }
    $list.Invalidate()
    Update-RmrHeader
}

# ---- drawing ----

$list.Add_DrawItem({
    param($sender, $e)
    if ($e.Index -lt 0) { return }
    $s = @($settings.servers)[$e.Index]
    $g = $e.Graphics
    $selected = ($e.State -band [Windows.Forms.DrawItemState]::Selected) -ne 0
    $back = if ($selected) { [Drawing.Color]::FromArgb(28, 44, 52) } else { [Drawing.Color]::FromArgb(12, 17, 24) }
    $brush = New-Object Drawing.SolidBrush($back)
    $g.FillRectangle($brush, $e.Bounds); $brush.Dispose()
    if ($selected) {
        $edge = New-Object Drawing.SolidBrush($cyan)
        $g.FillRectangle($edge, $e.Bounds.X, $e.Bounds.Y, 3, $e.Bounds.Height); $edge.Dispose()
    }
    $state = $ui.Status[(Get-RmrServerKey $s)]
    $dot = if ($state -eq 'online') { $cyan } elseif ($state -eq 'offline') { $red } else { $muted }
    $dotBrush = New-Object Drawing.SolidBrush($dot)
    $g.SmoothingMode = 'AntiAlias'
    $g.FillEllipse($dotBrush, ($e.Bounds.X + 14), ($e.Bounds.Y + 15), 9, 9); $dotBrush.Dispose()
    $nameFont = New-Object Drawing.Font('Segoe UI Semibold', 11)
    $smallFont = New-Object Drawing.Font('Segoe UI', 9)
    $b1 = New-Object Drawing.SolidBrush($bright)
    $b2 = New-Object Drawing.SolidBrush($muted)
    $g.DrawString([string]$s.name, $nameFont, $b1, ($e.Bounds.X + 30), ($e.Bounds.Y + 7))
    $where = [string]$s.host + ':' + [string]$s.port
    if ($state) { $where += '  -  ' + $state }
    $g.DrawString($where, $smallFont, $b2, ($e.Bounds.X + 30), ($e.Bounds.Y + 30))
    $nameFont.Dispose(); $smallFont.Dispose(); $b1.Dispose(); $b2.Dispose()
})

$news.Add_DrawItem({
    param($sender, $e)
    if ($e.Index -lt 0) { return }
    $g = $e.Graphics
    $brush = New-Object Drawing.SolidBrush([Drawing.Color]::FromArgb(12, 17, 24))
    $g.FillRectangle($brush, $e.Bounds); $brush.Dispose()
    $b1 = New-Object Drawing.SolidBrush($bright)
    $b2 = New-Object Drawing.SolidBrush($muted)
    $b3 = New-Object Drawing.SolidBrush($red)
    $titleFont = New-Object Drawing.Font('Segoe UI Semibold', 12)
    $smallFont = New-Object Drawing.Font('Segoe UI', 9)
    if ($ui.Articles.Count -eq 0) {
        $g.DrawString('No news from this server. Its news appears here when its Dashboard.bat is running (or when the server runs on this PC).', $smallFont, $b2, (New-Object Drawing.RectangleF(($e.Bounds.X + 16), ($e.Bounds.Y + 14), ($e.Bounds.Width - 32), 50)))
    } else {
        $a = $ui.Articles[$e.Index]
        $g.DrawString($a.When.ToUpper(), $smallFont, $b3, ($e.Bounds.X + 16), ($e.Bounds.Y + 8))
        $g.DrawString($a.Title, $titleFont, $b1, ($e.Bounds.X + 16), ($e.Bounds.Y + 24))
        $snippet = ($a.Text -replace '\s+', ' ')
        if ($snippet.Length -gt 160) { $snippet = $snippet.Substring(0, 157) + '...' }
        $g.DrawString($snippet, $smallFont, $b2, (New-Object Drawing.RectangleF(($e.Bounds.X + 16), ($e.Bounds.Y + 50), ($e.Bounds.Width - 32), 22)))
        $pen = New-Object Drawing.Pen([Drawing.Color]::FromArgb(30, 38, 48))
        $g.DrawLine($pen, $e.Bounds.X, ($e.Bounds.Bottom - 1), $e.Bounds.Right, ($e.Bounds.Bottom - 1)); $pen.Dispose()
    }
    $b1.Dispose(); $b2.Dispose(); $b3.Dispose(); $titleFont.Dispose(); $smallFont.Dispose()
})

# ---- actions ----

$news.Add_DoubleClick({
    $i = $news.SelectedIndex
    if ($i -lt 0 -or $i -ge $ui.Articles.Count) { return }
    $a = $ui.Articles[$i]
    $reader = New-Object Windows.Forms.Form
    $reader.Text = $a.Title
    $reader.StartPosition = 'CenterParent'
    $reader.ClientSize = New-Object Drawing.Size(640, 520)
    $reader.BackColor = [Drawing.Color]::FromArgb(14, 18, 24)
    $box = New-Object Windows.Forms.TextBox
    $box.Multiline = $true; $box.ReadOnly = $true; $box.ScrollBars = 'Vertical'
    $box.Dock = 'Fill'; $box.BorderStyle = 'None'
    $box.BackColor = [Drawing.Color]::FromArgb(14, 18, 24); $box.ForeColor = $bright
    $box.Font = New-Object Drawing.Font('Segoe UI', 10.5)
    $box.Text = ($a.When + "`r`n`r`n" + $a.Title.ToUpper() + "`r`n`r`n" + $a.Text) -replace "(?<!`r)`n", "`r`n"
    $reader.Controls.Add($box)
    [void]$reader.ShowDialog($form)
    $reader.Dispose()
})

$list.Add_SelectedIndexChanged({
    if ($list.SelectedIndex -ge 0) {
        $settings.selected = $list.SelectedIndex
        Save-RmrSettings
    }
    $ui.NewsFor = ''
    $ui.Articles = @()
    $news.Items.Clear()
    Update-RmrHeader
    Start-RmrNewsFetch
})
$list.Add_DoubleClick({ $editButton.PerformClick() })

$addButton.Add_Click({
    $entry = Edit-RmrServerEntry $null
    if (-not $entry) { return }
    $settings.servers = @($settings.servers) + $entry
    Save-RmrSettings
    Update-RmrServerList
    $list.SelectedIndex = $list.Items.Count - 1
})
$editButton.Add_Click({
    $i = $list.SelectedIndex
    if ($i -lt 0) { return }
    $entry = Edit-RmrServerEntry (@($settings.servers)[$i])
    if (-not $entry) { return }
    $all = @($settings.servers)
    $all[$i] = $entry
    $settings.servers = $all
    Save-RmrSettings
    Update-RmrServerList
    $ui.NewsFor = ''
    Update-RmrHeader
    Start-RmrNewsFetch
})
$removeButton.Add_Click({
    $i = $list.SelectedIndex
    if ($i -lt 0) { return }
    if (@($settings.servers).Count -le 1) { [Windows.Forms.MessageBox]::Show($form, 'Keep at least one server.', 'Red Moon Rising') | Out-Null; return }
    $s = @($settings.servers)[$i]
    if ([Windows.Forms.MessageBox]::Show($form, ('Remove ' + $s.name + '?'), 'Red Moon Rising', 'YesNo', 'Question') -ne 'Yes') { return }
    $all = New-Object Collections.ArrayList
    $all.AddRange(@($settings.servers))
    $all.RemoveAt($i)
    $settings.servers = @($all.ToArray())
    $settings.selected = 0
    Save-RmrSettings
    $list.SelectedIndex = -1
    Update-RmrServerList
})
$closeBox.Add_CheckedChanged({ $settings.closeOnPlay = $closeBox.Checked; Save-RmrSettings })

$folderButton.Add_Click({
    try { if (Get-RmrClientFolder -Choose) { Update-RmrClientLabel } }
    catch { [Windows.Forms.MessageBox]::Show($form, $_.Exception.Message, 'Red Moon Rising', 'OK', 'Warning') | Out-Null }
})

function Invoke-RmrPrepare([switch]$Launch) {
    $s = Get-RmrSelected
    if (-not $s) { return }
    if (-not (Test-RmrAllowedHost ([string]$s.host))) {
        [Windows.Forms.MessageBox]::Show($form, ($s.host + ' is not localhost or a local-network IPv4 address. Red Moon Rising is for your own PC and local network only; edit this server to use one.'), 'Red Moon Rising', 'OK', 'Warning') | Out-Null
        return
    }
    $ui.Busy = $true
    $playButton.Enabled = $false
    try {
        $playStatus.Text = 'Finding your EVE client...'
        [Windows.Forms.Application]::DoEvents()
        $client = Get-RmrClientFolder
        if (-not $client) { return }
        Update-RmrClientLabel
        $playStatus.Text = 'Pointing EVE at ' + $s.name + ' (' + $s.host + ':' + $s.port + ')...'
        [Windows.Forms.Application]::DoEvents()
        $connectHost = Get-RmrConnectHost $s
        if ($connectHost -ne [string]$s.host) { $playStatus.Text = $s.host + ' cannot be reached from inside your network; this PC hosts it, so EVE connects via 127.0.0.1.' }
        [Windows.Forms.Application]::DoEvents()
        $prepared = Prepare-RmrClient -ClientDirectory $client -ServerBind $connectHost -ServerPort ([int]$s.port)
        if ($Launch) {
            Start-RmrGameClient -ClientPath $prepared.ClientDirectory -GamePort ([int]$s.port) -ServerHost $connectHost
            $playStatus.Text = 'EVE is starting, connecting to ' + $connectHost + ':' + $s.port + '. Fly safe.'
            if ($closeBox.Checked) { $form.Close() }
        } else {
            $playStatus.Text = 'Client checked and pointed at ' + $s.name + '.'
        }
    } catch {
        $playStatus.Text = 'Could not start EVE.'
        [Windows.Forms.MessageBox]::Show($form, $_.Exception.Message, 'Red Moon Rising', 'OK', 'Error') | Out-Null
    } finally {
        $ui.Busy = $false
        $playButton.Enabled = $true
    }
}
$repairButton.Add_Click({ Invoke-RmrPrepare })
$playButton.Add_Click({
    $s = Get-RmrSelected
    if ($s -and $ui.Status[(Get-RmrServerKey $s)] -eq 'offline' -and -not ((Test-RmrOnThisPc $s) -and (Test-RmrReachable '127.0.0.1' ([int]$s.port)))) {
        $answer = [Windows.Forms.MessageBox]::Show($form, ($s.name + ' is not answering on ' + $s.host + ':' + $s.port + '. Start EVE anyway?'), 'Red Moon Rising', 'YesNo', 'Warning')
        if ($answer -ne 'Yes') { return }
    }
    Invoke-RmrPrepare -Launch
})

$timer = New-Object Windows.Forms.Timer
$timer.Interval = 1000
$timer.Add_Tick({
    try {
        Update-RmrChecks
        Receive-RmrNews
        Start-RmrNewsFetch
    } catch { }
})

$form.Add_Shown({
    Update-RmrServerList
    Update-RmrClientLabel
    Update-RmrHeader
    Start-RmrNewsFetch
    $timer.Start()
})
$form.Add_FormClosed({ $timer.Stop() })

try { [Windows.Forms.Application]::Run($form) }
catch { [Windows.Forms.MessageBox]::Show($_.Exception.Message, 'Red Moon Rising', 'OK', 'Error') | Out-Null }
finally { $timer.Dispose(); $form.Dispose() }
