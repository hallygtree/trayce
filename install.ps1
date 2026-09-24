# Trayce installer for Windows (current user, no admin needed).
#
# Install or update:
#   irm https://raw.githubusercontent.com/hallygtree/trayce/main/install.ps1 | iex
# Uninstall:
#   & ([scriptblock]::Create((irm https://raw.githubusercontent.com/hallygtree/trayce/main/install.ps1))) -Uninstall
#
# Downloads the latest release into %LOCALAPPDATA%\Programs\trayce, enables
# start-on-login (HKCU Run key, via `trayce --install`) and starts it.
# Settings in %APPDATA%\trayce are left alone on update and uninstall.

param([switch]$Uninstall)

$dir = Join-Path $env:LOCALAPPDATA 'Programs\trayce'
$exe = Join-Path $dir 'trayce.exe'
$url = 'https://github.com/hallygtree/trayce/releases/latest/download/trayce-windows.zip'

# A running copy locks the exe, so stop it before replacing or removing it.
Get-Process trayce -ErrorAction SilentlyContinue | ForEach-Object {
    $_ | Stop-Process -Force
    $_.WaitForExit()
}

if ($Uninstall) {
    if (Test-Path $exe) {
        Start-Process $exe -ArgumentList '--uninstall' -Wait -WindowStyle Hidden
    }
    Remove-Item $dir -Recurse -Force -ErrorAction SilentlyContinue
    Write-Host "Trayce removed. Settings kept in $env:APPDATA\trayce (delete it to wipe them)."
    return
}

$zip = Join-Path $env:TEMP 'trayce-windows.zip'
Write-Host 'Downloading Trayce...'
try {
    # Windows PowerShell 5.1 may default to TLS 1.0, which GitHub rejects.
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest $url -OutFile $zip -UseBasicParsing -ErrorAction Stop
    New-Item -ItemType Directory -Force $dir -ErrorAction Stop | Out-Null
    Expand-Archive $zip $dir -Force -ErrorAction Stop
} catch {
    Write-Host "Install failed: $($_.Exception.Message)" -ForegroundColor Red
    return
} finally {
    Remove-Item $zip -ErrorAction SilentlyContinue
}

Start-Process $exe -ArgumentList '--install' -Wait -WindowStyle Hidden
Start-Process $exe
Write-Host "Trayce installed in $dir, set to start on login, and running."
Write-Host 'Look for the coloured dot in the tray (it may be under the ^ arrow).'
