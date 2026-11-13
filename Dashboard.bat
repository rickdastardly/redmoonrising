@echo off
setlocal DisableDelayedExpansion
rem Red Moon Rising dashboard, behind a sign-in, for your local network: point a
rem browser on another device at this PC's network address and only you can get
rem in. When the game server is down it offers to start it. It is for your own
rem PC and local network only.
rem It keeps NOTHING: every start asks for a new sign-in, address and port, the
rem sign-in lives only in the running program's memory, and closing it removes
rem everything it set up (its temporary program copy, the firewall opening).
rem The game server's own dashboard stays on 127.0.0.1; this sits in front of it.
set "RMR_ROOT=%~dp0"
if /i "%~1"=="--help" goto help
if /i "%~1"=="-h" goto help
if /i "%~1"=="--allow-public" set "RMR_ALLOW_PUBLIC=1"
set "RMR_PS=%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe"
if exist "%SystemRoot%\Sysnative\WindowsPowerShell\v1.0\powershell.exe" set "RMR_PS=%SystemRoot%\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
title Red Moon Rising dashboard
"%RMR_PS%" -NoLogo -NoProfile -ExecutionPolicy Bypass -Command "try { $parts = [IO.File]::ReadAllText('%~f0') -split '(?m)^#==RMR_POWERSHELL==\r?$', 2; if ($parts.Count -ne 2) { throw 'Dashboard payload is missing.' }; & ([scriptblock]::Create($parts[1])) } catch { Write-Host $_.Exception.Message -ForegroundColor Red; exit 1 }"
if errorlevel 1 (
    echo.
    echo The dashboard stopped. Read the message above, then press a key to close.
    pause >nul
)
exit /b %errorlevel%
:help
echo Red Moon Rising dashboard
echo Usage: Dashboard.bat
echo Every start asks for a sign-in name and password and the address and port to
echo serve on (default: every local-network address of this PC, port 18080). Nothing
echo is saved: stop it with Ctrl+C and everything it set up is removed again.
echo Run Start.bat once first: it builds the dashboard program too.
exit /b 0
#==RMR_POWERSHELL==

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

$ErrorActionPreference = 'Stop'
$Root = $env:RMR_ROOT.TrimEnd('\')
$built = Join-Path $Root 'target\x86_64-pc-windows-gnu\release\dashboard.exe'
# The program runs from a temporary copy (so Start.bat can rebuild meanwhile),
# deleted again when the dashboard stops.
$exe = Join-Path ([IO.Path]::GetTempPath()) 'rmr-dashboard-session.exe'
$ruleName = 'Red Moon Rising dashboard (this session)'

function Invoke-RmrElevated([string]$Command) {
    Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoProfile', '-Command', $Command) -Verb RunAs -Wait -WindowStyle Hidden
}

# ---- leftovers from a run that was closed with the window's X ----
foreach ($old in @((Join-Path $Root '.rmr\dashboard-login.json'), (Join-Path $Root '.rmr\dashboard-host.json'), $exe)) {
    if (Test-Path -LiteralPath $old) { Remove-Item -LiteralPath $old -Force -ErrorAction SilentlyContinue }
}
$oldDir = Join-Path $Root '.rmr\dashboard'
if (Test-Path -LiteralPath $oldDir) { Remove-Item -LiteralPath $oldDir -Recurse -Force -ErrorAction SilentlyContinue }
$staleRule = @(Get-NetFirewallRule -ErrorAction SilentlyContinue | Where-Object { $_.DisplayName -like 'Red Moon Rising dashboard*' })
if ($staleRule.Count -gt 0) {
    Write-Host 'Removing the firewall opening an earlier dashboard left behind (asks for administrator)...' -ForegroundColor Yellow
    try { Invoke-RmrElevated "Get-NetFirewallRule | Where-Object { `$_.DisplayName -like 'Red Moon Rising dashboard*' } | Remove-NetFirewallRule" } catch { }
}

if (-not (Test-Path -LiteralPath $built)) {
    throw 'The dashboard has not been built yet. Run Start.bat once: it builds the dashboard along with the server.'
}
$source = Join-Path $Root 'crates\eve-server\src\bin\dashboard.rs'
if ((Test-Path -LiteralPath $source) -and (Get-Item -LiteralPath $source).LastWriteTimeUtc -gt (Get-Item -LiteralPath $built).LastWriteTimeUtc) {
    throw 'The dashboard program is out of date. Restart Start.bat once so it rebuilds it, then run Dashboard.bat again.'
}

# ---- this session's sign-in (memory only) ----
Write-Host ''
Write-Host 'Choose this session''s sign-in. Nothing is saved: the next start asks again.' -ForegroundColor Cyan
$user = ''
while (-not $user) { $user = (Read-Host 'Name').Trim() }
while ($true) {
    $first = Read-Host 'Password (at least 8 characters)' -AsSecureString
    $again = Read-Host 'Password again' -AsSecureString
    $plain = [Runtime.InteropServices.Marshal]::PtrToStringBSTR([Runtime.InteropServices.Marshal]::SecureStringToBSTR($first))
    $check = [Runtime.InteropServices.Marshal]::PtrToStringBSTR([Runtime.InteropServices.Marshal]::SecureStringToBSTR($again))
    if ($plain -ne $check) { Write-Host 'Those did not match. Try again.' -ForegroundColor Yellow; continue }
    if ($plain.Length -lt 8) { Write-Host 'Use at least 8 characters.' -ForegroundColor Yellow; continue }
    break
}
$check = $null

# ---- where to serve ----
$serve = [pscustomobject]@{ address = 'this-pc'; port = 18080 }
Write-Host ''
Write-Host 'Which address should the dashboard listen on?' -ForegroundColor Cyan
Write-Host '  this-pc   every local-network address of this PC'
Write-Host '  or one local-network IPv4 address of this PC, such as 192.168.1.20'
Write-Host 'The dashboard is for your own local network only.'
Write-Host '127.0.0.1 is left to the game server''s own dashboard, so both can use port 18080.'
$typed = (Read-Host ("Address [Enter for " + $serve.address + "]")).Trim()
if ($typed -in @('0.0.0.0', 'all', '*')) { $typed = 'this-pc' }
if ($typed -and $typed -ne 'this-pc') {
    if ($typed -notmatch '^\d{1,3}(\.\d{1,3}){3}$') { throw "Not an IPv4 address: $typed" }
    if (-not (Test-RmrPrivateIPv4 $typed) -and $env:RMR_ALLOW_PUBLIC -ne '1') { throw "Use this-pc, or this PC's IPv4 address on your local network (such as 192.168.1.20). The dashboard is for your own local network only." }
    if ($typed -like '127.*') { Write-Host '127.0.0.1 belongs to the game server''s own dashboard; using every network address of this PC instead.' -ForegroundColor Yellow; $typed = 'this-pc' }
}
if ($typed) { $serve.address = $typed }
$typed = (Read-Host ("Port [Enter for " + $serve.port + "]")).Trim()
if ($typed) {
    if ($typed -notmatch '^\d{1,5}$' -or [int]$typed -lt 1 -or [int]$typed -gt 65535) { throw "Not a port: $typed" }
    $serve.port = [int]$typed
}

# This PC's local-network addresses (never 127.0.0.1: the game server's own
# dashboard may hold it on the same port).
$addresses = @()
if ($serve.address -eq 'this-pc') {
    foreach ($nic in [Net.NetworkInformation.NetworkInterface]::GetAllNetworkInterfaces()) {
        if ($nic.OperationalStatus -ne 'Up') { continue }
        foreach ($unicast in $nic.GetIPProperties().UnicastAddresses) {
            $a = $unicast.Address
            if ($a.AddressFamily -eq [Net.Sockets.AddressFamily]::InterNetwork -and -not [Net.IPAddress]::IsLoopback($a)) {
                if ((Test-RmrPrivateIPv4 $a.ToString()) -or $env:RMR_ALLOW_PUBLIC -eq '1') { $addresses += $a.ToString() }
            }
        }
    }
    $addresses = @($addresses | Select-Object -Unique)
    if ($addresses.Count -eq 0) { throw 'This PC has no local-network address to serve the dashboard on.' }
} else {
    $addresses = @($serve.address)
}
$arguments = '--root "' + $Root + '" --login-stdin'
foreach ($a in $addresses) { $arguments += ' --bind ' + $a + ':' + $serve.port }

$ruleAdded = $false
$forwarded = $false
$process = $null
try {
    # ---- this session's firewall opening ----
    Write-Host ''
    $yes = (Read-Host ('Let other devices on your local network through Windows Firewall on port ' + $serve.port + ' while the dashboard runs? This asks for administrator, now and when it stops. [Y/n]')).Trim()
    if ($yes -eq '' -or $yes -match '^[Yy]') {
        $profiles = if ($env:RMR_ALLOW_PUBLIC -eq '1') { 'Any' } else { 'Private,Domain' }
        try {
            Invoke-RmrElevated "New-NetFirewallRule -DisplayName '$ruleName' -Direction Inbound -Protocol TCP -LocalPort $($serve.port) -Action Allow -Profile $profiles | Out-Null"
            $ruleAdded = $true
        } catch { Write-Host ('The firewall was not opened: ' + $_.Exception.Message) -ForegroundColor Yellow }
    }

    if ($env:RMR_ALLOW_PUBLIC -eq '1' -and $serve.address -eq 'this-pc') {
        $yes = (Read-Host ('Forward port ' + $serve.port + ' on your router for this session (UPnP)? [Y/n]')).Trim()
        if ($yes -eq '' -or $yes -match '^[Yy]') {
            $forward = Set-RmrUpnpForward $serve.port 'Red Moon Rising dashboard'
            if ($forward.Ok) {
                $forwarded = $true
                Write-Host $forward.Message -ForegroundColor Green
                if ($forward.External) { Write-Host ('http://' + $forward.External + ':' + $serve.port + '/') -ForegroundColor Green }
            } else { Write-Host $forward.Message -ForegroundColor Yellow }
        }
    }

    Copy-Item -LiteralPath $built -Destination $exe -Force

    Write-Host ''
    foreach ($a in $addresses) { Write-Host ('Dashboard: http://' + $a + ':' + $serve.port + '/') -ForegroundColor Green }
    Write-Host 'Open it from devices on your local network and sign in with the name and password you just chose.'
    Write-Host 'It is plain HTTP and meant for your own network only.'
    Write-Host 'Press Ctrl+C to stop it: everything it set up is removed and nothing is kept.' -ForegroundColor Cyan
    Write-Host ''

    # The sign-in goes to the program over stdin and never touches the disk.
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.FileName = $exe
    $start.Arguments = $arguments
    $start.UseShellExecute = $false
    $start.RedirectStandardInput = $true
    $process = [Diagnostics.Process]::Start($start)
    $process.StandardInput.WriteLine($user)
    $process.StandardInput.WriteLine($plain)
    $process.StandardInput.Close()
    $plain = $null; $user = $null; $first = $null; $again = $null
    while (-not $process.WaitForExit(500)) { }
    $code = $process.ExitCode
} finally {
    Write-Host ''
    Write-Host 'Stopping the dashboard and removing everything it set up...' -ForegroundColor Cyan
    if ($null -ne $process -and -not $process.HasExited) { try { $process.Kill(); $process.WaitForExit(5000) | Out-Null } catch { } }
    if ($forwarded) { Remove-RmrUpnpForward $serve.port }
    for ($i = 0; $i -lt 10 -and (Test-Path -LiteralPath $exe); $i++) {
        try { Remove-Item -LiteralPath $exe -Force -ErrorAction Stop } catch { Start-Sleep -Milliseconds 300 }
    }
    if ($ruleAdded) { try { Invoke-RmrElevated "Remove-NetFirewallRule -DisplayName '$ruleName'" } catch { } }
    $plain = $null; $user = $null
    Write-Host 'Done. Nothing from this session was kept.' -ForegroundColor Green
}
if ($code -ne 0) { exit $code }
