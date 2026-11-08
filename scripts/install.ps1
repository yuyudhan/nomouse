# Installs, updates or uninstalls nomouse on Windows for the current user (no administrator).
# Published with every GitHub release as releases/latest/download/install.ps1; README.md#install
# has the one-line PowerShell command.
#
# It downloads nomouse-x86_64-pc-windows-msvc.zip, checks it against the release's .sha256, puts
# nomouse.exe in %LOCALAPPDATA%\Programs\nomouse (or $env:NOMOUSE_INSTALL_DIR) and adds that
# folder to the user's PATH.
#
# Parameters (when run as a file):
#   -Version X.Y.Z   Install that release instead of the latest.
#   -Uninstall       Remove nomouse.exe and its PATH entry; keep the config.
#   -Purge           With -Uninstall, also delete %APPDATA%\nomouse.
# Environment:
#   NOMOUSE_INSTALL_DIR    Install folder.
#   NOMOUSE_INSTALL_BASE   URL of the directory holding the release assets, overriding GitHub.
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

function Install-Nomouse {
    if ($env:PROCESSOR_ARCHITECTURE -ne "AMD64" -and $env:PROCESSOR_ARCHITEW6432 -ne "AMD64") {
        throw "nomouse ships an x86_64 Windows build only (this is $env:PROCESSOR_ARCHITECTURE)"
    }
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
        Write-Status "installed $(& $dest --version) to $dest"
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
