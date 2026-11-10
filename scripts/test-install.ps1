# End-to-end test of install.ps1 against a local release directory holding
# nomouse-x86_64-pc-windows-msvc.zip and its .sha256 (as scripts/release/package.sh writes them):
# serves it over HTTP on 127.0.0.1 and installs, runs, uninstalls and refuses a bad checksum, with
# USERPROFILE, LOCALAPPDATA and APPDATA pointed at a throwaway directory. It also checks the
# processor gate (ARM64 accepted on Windows 11 with an emulation note, x86 refused) and that a PC
# without the Visual C++ runtime gets the redistributable fix instead of a broken install. The
# mirror of scripts/test-install.sh for Windows.
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

# Runs install.ps1 in a fresh pwsh whose profile directories all live in the temp home. Extra
# variables in $Environment (a $null value removes one) are set inside that pwsh just before
# install.ps1 runs, so the harness's own environment never changes.
function Invoke-Installer([string]$Base, [string[]]$Arguments, [hashtable]$Environment = @{}) {
    $env:USERPROFILE = $TestHome
    $env:LOCALAPPDATA = Join-Path $TestHome "AppData\Local"
    $env:APPDATA = Join-Path $TestHome "AppData\Roaming"
    $env:NOMOUSE_INSTALL_BASE = $Base
    Remove-Item Env:NOMOUSE_INSTALL_DIR -ErrorAction SilentlyContinue
    if ($Environment.Count -eq 0) {
        $output = & pwsh -NoProfile -NonInteractive -File $Installer @Arguments 2>&1 | Out-String
    } else {
        $lines = foreach ($name in $Environment.Keys) {
            if ($null -eq $Environment[$name]) {
                "Remove-Item Env:$name -ErrorAction SilentlyContinue"
            } else {
                "`$env:$name = '$($Environment[$name] -replace "'", "''")'"
            }
        }
        $command = (@($lines) + "& '$($Installer -replace "'", "''")' $($Arguments -join ' ')") -join "; "
        $output = & pwsh -NoProfile -NonInteractive -Command $command 2>&1 | Out-String
    }
    return [pscustomobject]@{ Code = $LASTEXITCODE; Output = $output }
}

# Removes an install left by a case, so the next case starts from nothing.
function Invoke-Uninstall {
    Invoke-Installer $base @("-Uninstall") | Out-Null
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

    # 6. Windows on ARM: accepted with an emulation note on Windows 11 (build 22000+), refused with
    #    the reason on older builds. The runner's real build decides which one applies.
    $build = [Environment]::OSVersion.Version.Build
    $arm = @{ PROCESSOR_ARCHITECTURE = "ARM64"; PROCESSOR_ARCHITEW6432 = $null }
    $result = Invoke-Installer $base @() $arm
    if ($build -ge 22000) {
        if ($result.Code -eq 0 -and $result.Output -match "runs under x64 emulation" -and (Test-Path $exe)) {
            Write-Pass "PROCESSOR_ARCHITECTURE=ARM64 on build $build installs with the x64 emulation note"
        } else {
            Write-Fail "ARM64 on build $build (exit $($result.Code)): $($result.Output)"
        }
    } elseif ($result.Code -ne 0 -and $result.Output -match "from Windows 11 on" -and -not (Test-Path $exe)) {
        Write-Pass "PROCESSOR_ARCHITECTURE=ARM64 on build $build (before Windows 11) is refused with the reason"
    } else {
        Write-Fail "ARM64 on build $build (exit $($result.Code)): $($result.Output)"
    }
    Invoke-Uninstall

    # 7. 32-bit Windows is refused and nothing is installed.
    $x86 = @{ PROCESSOR_ARCHITECTURE = "x86"; PROCESSOR_ARCHITEW6432 = $null }
    $result = Invoke-Installer $base @() $x86
    if ($result.Code -ne 0 -and $result.Output -match "64-bit x86_64 Windows build only; this Windows is x86" -and -not (Test-Path $exe)) {
        Write-Pass "PROCESSOR_ARCHITECTURE=x86 is refused and installs nothing"
    } else {
        Write-Fail "x86 (exit $($result.Code)): $($result.Output)"
    }
    Invoke-Uninstall

    # 8. No Visual C++ runtime on the PC (NOMOUSE_SYSTEM_DIR is an empty folder): a build that
    #    imports VCRUNTIME140.dll is refused with the redistributable link before anything is
    #    installed; a statically linked build installs regardless.
    $noRuntime = Join-Path $Work "empty-system32"
    New-Item -ItemType Directory -Path $noRuntime -Force | Out-Null
    $zipDir = Join-Path $Work "archive"
    Expand-Archive -Path (Join-Path $Dist $Archive) -DestinationPath $zipDir -Force
    $packaged = Join-Path $zipDir "nomouse-x86_64-pc-windows-msvc\nomouse.exe"
    $text = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($packaged))
    $dynamic = $text -match "(?i)vcruntime140(_1)?\.dll"
    $result = Invoke-Installer $base @() @{ NOMOUSE_SYSTEM_DIR = $noRuntime }
    if ($dynamic) {
        if ($result.Code -ne 0 -and $result.Output -match "Visual C\+\+ 2015-2022 Redistributable" -and
            $result.Output -match "aka\.ms/vs/17/release/vc_redist\.x64\.exe" -and -not (Test-Path $exe)) {
            Write-Pass "a build importing VCRUNTIME140.dll on a PC without it is refused with the redistributable link"
        } else {
            Write-Fail "missing VC++ runtime, dynamic build (exit $($result.Code)): $($result.Output)"
        }
    } elseif ($result.Code -eq 0 -and (Test-Path $exe)) {
        Write-Pass "a statically linked build installs on a PC without the Visual C++ runtime"
    } else {
        Write-Fail "missing VC++ runtime, static build (exit $($result.Code)): $($result.Output)"
    }
    Invoke-Uninstall
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
