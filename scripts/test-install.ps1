# End-to-end test of install.ps1 against a local release directory holding
# nomouse-x86_64-pc-windows-msvc.zip and its .sha256 (as scripts/release/package.sh writes them):
# serves it over HTTP on 127.0.0.1 and installs, runs, uninstalls and refuses a bad checksum, with
# USERPROFILE, LOCALAPPDATA and APPDATA pointed at a throwaway directory. The mirror of
# scripts/test-install.sh for Windows.
#
# install.ps1 edits the user PATH in the registry; the harness saves it first and restores it on
# exit, and checks that the install added its folder and the uninstall removed it.
#
# Usage: pwsh -NoProfile -File scripts/test-install.ps1 -Dist dist
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Dist
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
$Dist = (Resolve-Path $Dist).Path
$Installer = Join-Path $Root "scripts\install.ps1"
$Archive = "nomouse-x86_64-pc-windows-msvc.zip"
if (-not (Test-Path (Join-Path $Dist $Archive))) {
    throw "test-install: no $Archive in $Dist"
}

$Work = Join-Path ([IO.Path]::GetTempPath()) ("nomouse-test-" + [Guid]::NewGuid())
$TestHome = Join-Path $Work "home"
$Bad = Join-Path $Dist ".bad-checksum"
New-Item -ItemType Directory -Path $TestHome | Out-Null
$SavedPath = [Environment]::GetEnvironmentVariable("Path", "User")
$Failures = 0
$Server = $null

function Write-Pass([string]$Message) {
    Write-Output "ok   - $Message"
}

function Write-Fail([string]$Message) {
    Write-Output "FAIL - $Message"
    $script:Failures++
}

# Runs install.ps1 in a fresh pwsh whose profile directories all live in the temp home.
function Invoke-Installer([string]$Base, [string[]]$Arguments) {
    $env:USERPROFILE = $TestHome
    $env:LOCALAPPDATA = Join-Path $TestHome "AppData\Local"
    $env:APPDATA = Join-Path $TestHome "AppData\Roaming"
    $env:NOMOUSE_INSTALL_BASE = $Base
    Remove-Item Env:NOMOUSE_INSTALL_DIR -ErrorAction SilentlyContinue
    $output = & pwsh -NoProfile -NonInteractive -File $Installer @Arguments 2>&1 | Out-String
    return [pscustomobject]@{ Code = $LASTEXITCODE; Output = $output }
}

try {
    $port = Get-Random -Minimum 20000 -Maximum 40000
    $Server = Start-Process -PassThru -WindowStyle Hidden python `
        -ArgumentList "-m", "http.server", "$port", "--bind", "127.0.0.1", "--directory", $Dist
    $base = "http://127.0.0.1:$port"
    for ($i = 0; $i -lt 50; $i++) {
        try {
            Invoke-WebRequest -Uri "$base/$Archive.sha256" -UseBasicParsing -TimeoutSec 2 | Out-Null
            break
        } catch {
            Start-Sleep -Milliseconds 200
        }
    }

    $installDir = Join-Path $TestHome "AppData\Local\Programs\nomouse"
    $exe = Join-Path $installDir "nomouse.exe"

    # 1. Install.
    $result = Invoke-Installer $base @()
    if ($result.Code -eq 0 -and $result.Output -match "checksum ok" -and (Test-Path $exe)) {
        Write-Pass "install.ps1 installs into LOCALAPPDATA\Programs\nomouse with a verified checksum"
    } else {
        Write-Fail "install (exit $($result.Code)): $($result.Output)"
    }

    # 2. The install folder is on the user PATH.
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if (@($userPath -split ";") -contains $installDir) {
        Write-Pass "install adds $installDir to the user PATH"
    } else {
        Write-Fail "user PATH lacks ${installDir}: $userPath"
    }

    # 3. The installed binary runs.
    $version = (& $exe --version 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -eq 0 -and $version -like "nomouse *") {
        Write-Pass "installed binary runs: $version"
    } else {
        Write-Fail "nomouse --version: $version"
    }

    # 4. Uninstall removes the binary and the PATH entry.
    $result = Invoke-Installer $base @("-Uninstall")
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($result.Code -eq 0 -and -not (Test-Path $exe) -and -not (@($userPath -split ";") -contains $installDir)) {
        Write-Pass "-Uninstall removes nomouse.exe and its user PATH entry"
    } else {
        Write-Fail "uninstall (exit $($result.Code)): $($result.Output)"
    }

    # 5. A wrong checksum aborts without installing.
    New-Item -ItemType Directory -Path $Bad -Force | Out-Null
    Copy-Item (Join-Path $Dist $Archive) $Bad
    Set-Content -Path (Join-Path $Bad "$Archive.sha256") -Value "0000  $Archive"
    $result = Invoke-Installer "$base/.bad-checksum" @()
    if ($result.Code -ne 0 -and $result.Output -match "checksum mismatch" -and -not (Test-Path $exe)) {
        Write-Pass "a wrong checksum fails and installs nothing"
    } else {
        Write-Fail "wrong checksum (exit $($result.Code)): $($result.Output)"
    }
} finally {
    if ($null -ne $Server) {
        Stop-Process -Id $Server.Id -Force -ErrorAction SilentlyContinue
    }
    [Environment]::SetEnvironmentVariable("Path", $SavedPath, "User")
    Remove-Item $Bad -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item $Work -Recurse -Force -ErrorAction SilentlyContinue
}

if ($Failures -ne 0) {
    Write-Output "test-install: $Failures failure(s)"
    exit 1
}
Write-Output "test-install: all passed"
