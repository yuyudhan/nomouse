# Installs, updates or uninstalls nomouse on Windows for the current user (no administrator).
# Published with every GitHub release as releases/latest/download/install.ps1; README.md#install
# has the one-line PowerShell command.
#
# It downloads nomouse-x86_64-pc-windows-msvc.zip, checks it against the release's .sha256, checks
# that the nomouse.exe inside starts on this PC, puts it in %LOCALAPPDATA%\Programs\nomouse (or
# $env:NOMOUSE_INSTALL_DIR) and adds that folder to the user's PATH. Any failure before the copy
# leaves an existing install untouched. 64-bit x86 Windows runs the build natively; Windows 11 on
# ARM runs it under x64 emulation.
#
# Parameters (when run as a file):
#   -Version X.Y.Z   Install that release instead of the latest.
#   -Uninstall       Remove nomouse.exe and its PATH entry; keep the config.
#   -Purge           With -Uninstall, also delete %APPDATA%\nomouse.
# Environment:
#   NOMOUSE_INSTALL_DIR    Install folder.
#   NOMOUSE_INSTALL_BASE   URL of the directory holding the release assets, overriding GitHub.
#   NOMOUSE_SYSTEM_DIR     Folder searched for the Visual C++ runtime DLLs instead of the 64-bit
#                          System32 (scripts/test-install.ps1 points it at an empty folder).
[CmdletBinding()]
param(
    [string]$Version = "",
    [switch]$Uninstall,
    [switch]$Purge
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$Repo = "yuyudhan/nomouse"
$Target = "x86_64-pc-windows-msvc"
$Archive = "nomouse-$Target.zip"

function Write-Status([string]$Message) {
    Write-Output "nomouse: $Message"
}

function Get-InstallDir {
    if ($env:NOMOUSE_INSTALL_DIR) {
        return $env:NOMOUSE_INSTALL_DIR
    }
    return Join-Path $env:LOCALAPPDATA "Programs\nomouse"
}

function Get-UserPath {
    $value = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($null -eq $value) {
        return @()
    }
    return @($value -split ";" | Where-Object { $_ -ne "" })
}

function Save-UserPath([string[]]$Entries) {
    [Environment]::SetEnvironmentVariable("Path", ($Entries -join ";"), "User")
}

function Invoke-Download([string]$Url, [string]$OutFile) {
    $attempt = 0
    while ($true) {
        $attempt++
        try {
            Invoke-WebRequest -Uri $Url -OutFile $OutFile -UseBasicParsing -TimeoutSec 60
            return
        } catch {
            if ($attempt -ge 4) {
                throw "download failed: $Url ($($_.Exception.Message))"
            }
            Start-Sleep -Seconds (2 * $attempt)
        }
    }
}

# STATUS_DLL_NOT_FOUND and STATUS_INVALID_IMAGE_FORMAT as signed process exit codes.
$StatusDllNotFound = -1073741515
$StatusInvalidImageFormat = -1073741701
$VcRedistUrl = "https://aka.ms/vs/17/release/vc_redist.x64.exe"

# The x86_64 build runs natively on AMD64 and under x64 emulation on Windows 11 on ARM (build
# 22000+); Windows 10 on ARM emulates only 32-bit x86, and 32-bit Windows cannot run it at all.
function Assert-SupportedArchitecture {
    # A 32-bit PowerShell on 64-bit Windows reports x86 and keeps the OS's own in ARCHITEW6432.
    $arch = $env:PROCESSOR_ARCHITECTURE
    if ($env:PROCESSOR_ARCHITEW6432) {
        $arch = $env:PROCESSOR_ARCHITEW6432
    }
    switch ($arch) {
        "AMD64" {
            return
        }
        "ARM64" {
            $build = [Environment]::OSVersion.Version.Build
            if ($build -lt 22000) {
                throw "nomouse ships an x86_64 build, which Windows on ARM runs under x64 emulation from Windows 11 on; this is Windows build $build (Windows 10 on ARM has no x64 emulation)"
            }
            Write-Status "Windows on ARM: nomouse runs under x64 emulation"
        }
        default {
            throw "nomouse ships a 64-bit x86_64 Windows build only; this Windows is $arch"
        }
    }
}

function Get-SystemDir {
    if ($env:NOMOUSE_SYSTEM_DIR) {
        return $env:NOMOUSE_SYSTEM_DIR
    }
    # The x64 exe loads from the 64-bit System32, which a 32-bit PowerShell sees as Sysnative.
    $native = Join-Path $env:SystemRoot "Sysnative"
    if (Test-Path $native) {
        return $native
    }
    return Join-Path $env:SystemRoot "System32"
}

# A build linked against the dynamic Visual C++ runtime imports VCRUNTIME140*.dll, which a clean
# Windows lacks until the Visual C++ 2015-2022 Redistributable is installed; starting such an exe
# there pops a system error dialog. Refuse with the fix instead, before touching the install.
function Assert-VcRuntime([string]$Exe) {
    $text = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($Exe))
    $imports = @([regex]::Matches($text, "(?i)vcruntime140(_1)?\.dll") |
        ForEach-Object { $_.Value.ToUpperInvariant() } | Sort-Object -Unique)
    $systemDir = Get-SystemDir
    $missing = @($imports | Where-Object { -not (Test-Path (Join-Path $systemDir $_)) })
    if ($missing.Count -gt 0) {
        throw "nomouse.exe needs the Microsoft Visual C++ 2015-2022 Redistributable (x64), which this PC lacks ($($missing -join ', ') is not in $systemDir). Install it from $VcRedistUrl and run this installer again; nothing was installed."
    }
}

# Runs `nomouse.exe --version` and returns its output, or explains why the exe does not start.
function Get-NomouseVersion([string]$Exe) {
    $output = (& $Exe --version | Out-String).Trim()
    $code = $LASTEXITCODE
    if ($code -eq 0) {
        return $output
    }
    $hint = ""
    if ($code -eq $StatusDllNotFound) {
        $hint = ": Windows could not find a DLL it needs; install the Microsoft Visual C++ 2015-2022 Redistributable (x64) from $VcRedistUrl and run this installer again"
    } elseif ($code -eq $StatusInvalidImageFormat) {
        $hint = ": a DLL it loads is built for another processor; repair the Microsoft Visual C++ 2015-2022 Redistributable (x64) from $VcRedistUrl"
    }
    throw "nomouse.exe from $Archive does not start (exit code $code)$hint. Nothing was installed. Output: $output"
}

function Install-Nomouse {
    Assert-SupportedArchitecture
    if ($env:NOMOUSE_INSTALL_BASE) {
        $base = $env:NOMOUSE_INSTALL_BASE.TrimEnd("/")
    } elseif ($Version) {
        $base = "https://github.com/$Repo/releases/download/v$Version"
    } else {
        $base = "https://github.com/$Repo/releases/latest/download"
    }
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("nomouse-" + [Guid]::NewGuid())
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        Write-Status "downloading $Archive from $base"
        $zip = Join-Path $tmp $Archive
        Invoke-Download "$base/$Archive" $zip
        Invoke-Download "$base/$Archive.sha256" "$zip.sha256"
        $expected = ((Get-Content "$zip.sha256" -Raw).Trim() -split "\s+")[0].ToLowerInvariant()
        $actual = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLowerInvariant()
        if ($expected -ne $actual) {
            throw "checksum mismatch for $Archive (expected $expected, got $actual)"
        }
        Write-Status "checksum ok ($actual)"

        Expand-Archive -Path $zip -DestinationPath $tmp -Force
        $exe = Join-Path $tmp "nomouse-$Target\nomouse.exe"
        if (-not (Test-Path $exe)) {
            throw "archive has no nomouse-$Target\nomouse.exe"
        }
        Assert-VcRuntime $exe
        $installed = Get-NomouseVersion $exe

        $dir = Get-InstallDir
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
        $dest = Join-Path $dir "nomouse.exe"
        # A running nomouse.exe cannot be overwritten, but it can be renamed out of the way.
        if (Test-Path $dest) {
            $old = "$dest.old"
            Remove-Item $old -Force -ErrorAction SilentlyContinue
            Move-Item $dest $old -Force
        }
        Copy-Item $exe $dest -Force
        Remove-Item "$dest.old" -Force -ErrorAction SilentlyContinue

        $entries = Get-UserPath
        if ($entries -notcontains $dir) {
            Save-UserPath ($entries + $dir)
            Write-Status "added $dir to your user PATH (open a new terminal to use it)"
        }
        $env:Path = "$dir;$env:Path"
        Write-Status "installed $installed to $dest"
        Write-Status "next: nomouse doctor"
    } finally {
        Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Uninstall-Nomouse {
    $dir = Get-InstallDir
    $dest = Join-Path $dir "nomouse.exe"
    if (Test-Path $dest) {
        Remove-Item $dest -Force
        Remove-Item "$dest.old" -Force -ErrorAction SilentlyContinue
        Write-Status "removed $dest"
    } else {
        Write-Status "nomouse is not installed in $dir"
    }
    if ((Test-Path $dir) -and -not (Get-ChildItem $dir -Force)) {
        Remove-Item $dir -Force
    }
    $entries = Get-UserPath
    if ($entries -contains $dir) {
        Save-UserPath ($entries | Where-Object { $_ -ne $dir })
        Write-Status "removed $dir from your user PATH"
    }
    $config = Join-Path $env:APPDATA "nomouse"
    if ($Purge) {
        Remove-Item $config -Recurse -Force -ErrorAction SilentlyContinue
        Write-Status "removed $config"
    } elseif (Test-Path $config) {
        Write-Status "kept the config in $config (add -Purge to delete it)"
    }
}

if ($Version -and $Version -notmatch "^v?\d+\.\d+\.\d+$") {
    throw "-Version wants MAJOR.MINOR.PATCH, got $Version"
}
$Version = $Version.TrimStart("v")
if ($Purge -and -not $Uninstall) {
    throw "-Purge only goes with -Uninstall"
}
if ($Uninstall) {
    Uninstall-Nomouse
} else {
    Install-Nomouse
}
