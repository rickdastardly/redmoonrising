@echo off
setlocal DisableDelayedExpansion
rem Red Moon Rising dashboard, behind a sign-in, for your local network: point a
rem browser on another device at this PC's network address and only you can get
rem in. When the game server is down it offers to start it. It is for your own
rem PC and local network only.
rem Nothing is kept between runs: each start asks for the sign-in and address
rem afresh, and closing the window forgets them.
rem The game server's own dashboard stays on 127.0.0.1; this sits in front of it.
set "RMR_ROOT=%~dp0"
set "RMR_SETUP="
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
echo serve on (default: every network address of this PC, port 18080, sharing the
echo port with the game server's own dashboard on 127.0.0.1). Nothing is kept:
echo closing the window forgets the sign-in. Run Start.bat once first: it builds this too.
exit /b 0
#==RMR_POWERSHELL==

$ErrorActionPreference = 'Stop'
$Root = $env:RMR_ROOT.TrimEnd('\')
$Utf8 = New-Object Text.UTF8Encoding($false)
$built = Join-Path $Root 'target\x86_64-pc-windows-gnu\release\dashboard.exe'
if (-not (Test-Path -LiteralPath $built)) {
    throw 'The dashboard has not been built yet. Run Start.bat once: it builds the dashboard along with the server.'
}
# Run a copy, so Start.bat can rebuild while the dashboard stays up.
$runDir = Join-Path $Root '.rmr\dashboard'
[IO.Directory]::CreateDirectory($runDir) | Out-Null
$exe = Join-Path $runDir 'rmr-dashboard.exe'
if (-not (Test-Path -LiteralPath $exe) -or (Get-FileHash -LiteralPath $exe).Hash -ne (Get-FileHash -LiteralPath $built).Hash) {
    try { Copy-Item -LiteralPath $built -Destination $exe -Force }
    catch { throw 'Could not update the dashboard program; is another Dashboard.bat still running? Close it and try again.' }
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

# ---- sign-in ----
$loginPath = Join-Path $Root '.rmr\dashboard-login.json'
$hostPath = Join-Path $Root '.rmr\dashboard-host.json'
# A fresh start every time: whatever an earlier run left is gone.
foreach ($old in @($loginPath, $hostPath)) { if (Test-Path -LiteralPath $old) { Remove-Item -LiteralPath $old -Force } }
if ($true) {
    Write-Host ''
    Write-Host 'Choose this session''s sign-in. Only this name and password will get in, until you close this window.' -ForegroundColor Cyan
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
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.FileName = $exe
    $start.Arguments = '--root "' + $Root + '" --set-login'
    $start.UseShellExecute = $false
    $start.RedirectStandardInput = $true
    $process = [Diagnostics.Process]::Start($start)
    $process.StandardInput.WriteLine($user)
    $process.StandardInput.WriteLine($plain)
    $process.StandardInput.Close()
    $process.WaitForExit()
    $plain = $null; $check = $null
    if ($process.ExitCode -ne 0) { throw 'The sign-in could not be saved.' }
}

# ---- where to serve ----
$serve = [pscustomobject]@{ address = 'this-pc'; port = 18080 }
if ($true) {
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
}

# 127.0.0.1 is the game server's own dashboard (no sign-in): never take it.
if ([string]$serve.address -like '127.*') {
    Write-Host 'The saved address was 127.0.0.1, which the game server''s own dashboard uses. Using every network address of this PC instead.' -ForegroundColor Yellow
    $serve.address = 'this-pc'
}

# Each of this PC's own network addresses (not 127.0.0.1, which the game
# server's dashboard may hold on the same port).
$addresses = @()
if ($serve.address -eq 'this-pc' -or $serve.address -eq '0.0.0.0') {
    foreach ($nic in [Net.NetworkInformation.NetworkInterface]::GetAllNetworkInterfaces()) {
        if ($nic.OperationalStatus -ne 'Up') { continue }
        foreach ($unicast in $nic.GetIPProperties().UnicastAddresses) {
            $a = $unicast.Address
            if ($a.AddressFamily -eq [Net.Sockets.AddressFamily]::InterNetwork -and -not [Net.IPAddress]::IsLoopback($a)) {
                # Local-network addresses only (unless the owner's --allow-public).
                if ((Test-RmrPrivateIPv4 $a.ToString()) -or $env:RMR_ALLOW_PUBLIC -eq '1') { $addresses += $a.ToString() }
            }
        }
    }
    $addresses = @($addresses | Select-Object -Unique)
    if ($addresses.Count -eq 0) { throw 'This PC has no local-network address to serve the dashboard on.' }
} else {
    $addresses = @($serve.address)
}
$bindArgs = @('--root', $Root)
foreach ($a in $addresses) { $bindArgs += @('--bind', ($a + ':' + $serve.port)) }

# Let other computers through Windows Firewall (asks for administrator once).
$ruleName = 'Red Moon Rising dashboard (TCP ' + $serve.port + ')'
try {
    if (-not (Get-NetFirewallRule -DisplayName $ruleName -ErrorAction SilentlyContinue)) {
        Write-Host ''
        $yes = (Read-Host ('Allow other computers on your local network through Windows Firewall on port ' + $serve.port + '? This asks for administrator. [Y/n]')).Trim()
        if ($yes -eq '' -or $yes -match '^[Yy]') {
            $profiles = if ($env:RMR_ALLOW_PUBLIC -eq '1') { 'Any' } else { 'Private,Domain' }
            $command = "New-NetFirewallRule -DisplayName '$ruleName' -Direction Inbound -Protocol TCP -LocalPort $($serve.port) -Action Allow -Profile $profiles | Out-Null"
            Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoProfile', '-Command', $command) -Verb RunAs -Wait -WindowStyle Hidden
        }
    }
} catch { Write-Host ('Firewall rule not added: ' + $_.Exception.Message) -ForegroundColor Yellow }

# Ask the router to forward the dashboard port for this session only.
$forwarded = $false
if ($env:RMR_ALLOW_PUBLIC -eq '1' -and ($serve.address -eq 'this-pc' -or $serve.address -eq '0.0.0.0')) {
    $yes = (Read-Host ('Forward port ' + $serve.port + ' on your router for this session (UPnP), so it can be reached from outside? [Y/n]')).Trim()
    if ($yes -eq '' -or $yes -match '^[Yy]') {
        $forward = Set-RmrUpnpForward $serve.port 'Red Moon Rising dashboard'
        if ($forward.Ok) {
            $forwarded = $true
            Write-Host $forward.Message -ForegroundColor Green
            if ($forward.External) { Write-Host ('From outside: http://' + $forward.External + ':' + $serve.port + '/') -ForegroundColor Green }
        } else { Write-Host $forward.Message -ForegroundColor Yellow }
    }
}

Write-Host ''
foreach ($a in $addresses) { Write-Host ('Dashboard: http://' + $a + ':' + $serve.port + '/') -ForegroundColor Green }
Write-Host ('On this PC, http://127.0.0.1:' + $serve.port + '/ is the game server''s own dashboard (no sign-in).') 
Write-Host 'Sign in with the name and password you set. Close this window to take the dashboard offline.'
Write-Host 'Open it from devices on your local network. It is plain HTTP and meant for your own network only; use a strong password.'
Write-Host 'Closing this window forgets the sign-in; the next start asks for a new one.'
Write-Host ''
try {
    & $exe @bindArgs
    $code = $LASTEXITCODE
} finally {
    # Closing the dashboard closes the router's door too.
    if ($forwarded) { Remove-RmrUpnpForward $serve.port }
    # The program forgets the stored sign-in once it has read it; make sure.
    if (Test-Path -LiteralPath $loginPath) { Remove-Item -LiteralPath $loginPath -Force -ErrorAction SilentlyContinue }
}
if ($code -ne 0) { exit $code }
