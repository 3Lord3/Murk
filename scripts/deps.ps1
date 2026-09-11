# Murk build dependency setup for Windows.
#
# The Linux counterpart (scripts/deps.sh) only has to name packages: every
# distribution ships libmpv. Windows ships nothing, and libmpv is not on any
# package manager Murk can assume, so this script fetches the upstream build
# instead:
#
#   * downloads the latest libmpv development archive (headers, the DLL and the
#     module definition file) from the mpv-winbuild project,
#   * turns mpv.def into the mpv.lib the MSVC linker wants, because
#     libmpv2-sys emits `cargo:rustc-link-lib=mpv` and nothing on Windows can
#     produce that from the DLL alone,
#   * leaves everything under src-tauri/mpv and prints the environment the
#     build needs.
#
# usage:
#   pwsh -File scripts/deps.ps1            # download, unpack, build the import library
#   pwsh -File scripts/deps.ps1 -Check     # report what is present, change nothing

[CmdletBinding()]
param(
    [switch]$Check,
    # Pinning is for CI reproducibility; a developer wants whatever is current.
    [string]$Version = 'latest'
)

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$mpvDir   = Join-Path $repoRoot 'src-tauri/mpv'
$libDir   = Join-Path $mpvDir 'lib'
$binDir   = Join-Path $mpvDir 'bin'

function Write-Status($ok, $name, $detail) {
    $mark = if ($ok) { "[ ok ]" } else { "[miss]" }
    $color = if ($ok) { 'Green' } else { 'Red' }
    Write-Host -NoNewline -ForegroundColor $color $mark
    Write-Host ("  {0,-22} {1}" -f $name, $detail)
}

function Find-VsTool($name) {
    # lib.exe and dumpbin.exe are only on PATH inside a Visual Studio developer
    # prompt. On a GitHub runner ilammy/msvc-dev-cmd puts them there; locally
    # the fallback is vswhere, which is at a fixed location on every machine
    # that has any Visual Studio or Build Tools install.
    $tool = Get-Command $name -ErrorAction SilentlyContinue
    if ($tool) { return $tool.Source }

    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    if (-not (Test-Path $vswhere)) { return $null }

    $install = & $vswhere -latest -products * -property installationPath
    if (-not $install) { return $null }

    $found = Get-ChildItem -Path (Join-Path $install 'VC/Tools/MSVC') -Recurse -Filter $name `
        -ErrorAction SilentlyContinue | Where-Object { $_.FullName -match '\\Hostx64\\x64\\' } |
        Select-Object -First 1
    if ($found) { return $found.FullName }
    return $null
}

function Report-State {
    Write-Host "libmpv development files:"
    Write-Status (Test-Path (Join-Path $libDir 'mpv.lib')) 'mpv.lib' $libDir
    Write-Status (Test-Path (Join-Path $binDir 'libmpv-2.dll')) 'libmpv-2.dll' $binDir
    Write-Host "build tools:"
    $lib = Find-VsTool 'lib.exe'
    Write-Status ($null -ne $lib) 'lib.exe' ($lib ?? 'install the MSVC build tools')
    $sevenZip = Get-Command 7z -ErrorAction SilentlyContinue
    Write-Status ($null -ne $sevenZip) '7z' ($sevenZip?.Source ?? 'install 7-Zip (winget install 7zip.7zip)')
}

if ($Check) {
    Report-State
    Write-Host ""
    Write-Host "set for the build:  `$env:MPV_LIB_DIR = '$libDir'"
    exit 0
}

# --- fetch -------------------------------------------------------------------
# The dev archive is the one carrying include/ and mpv.def; the player archive
# is a different asset and is of no use here.
$release = if ($Version -eq 'latest') {
    Invoke-RestMethod 'https://api.github.com/repos/zhongfly/mpv-winbuild/releases/latest' `
        -Headers @{ 'User-Agent' = 'murk-build' }
} else {
    Invoke-RestMethod "https://api.github.com/repos/zhongfly/mpv-winbuild/releases/tags/$Version" `
        -Headers @{ 'User-Agent' = 'murk-build' }
}

$asset = $release.assets | Where-Object { $_.name -like 'mpv-dev-x86_64-2*' -and $_.name -notlike '*v3*' } |
    Select-Object -First 1
if (-not $asset) {
    throw "no mpv-dev-x86_64 asset in release $($release.tag_name)"
}

Write-Host "downloading $($asset.name)"
$archive = Join-Path ([System.IO.Path]::GetTempPath()) $asset.name
Invoke-WebRequest $asset.browser_download_url -OutFile $archive -UseBasicParsing

$sevenZip = Get-Command 7z -ErrorAction SilentlyContinue
if (-not $sevenZip) {
    throw "7z is required to unpack $($asset.name). Install it with: winget install 7zip.7zip"
}

$extract = Join-Path ([System.IO.Path]::GetTempPath()) 'murk-mpv-dev'
Remove-Item -Recurse -Force $extract -ErrorAction SilentlyContinue
& 7z x $archive "-o$extract" -y | Out-Null

New-Item -ItemType Directory -Force -Path $libDir, $binDir | Out-Null

$dll = Get-ChildItem -Path $extract -Recurse -Filter 'libmpv-2.dll' | Select-Object -First 1
$def = Get-ChildItem -Path $extract -Recurse -Filter 'mpv.def' | Select-Object -First 1
if (-not $dll) { throw "the archive contains no libmpv-2.dll" }

Copy-Item $dll.FullName (Join-Path $binDir 'libmpv-2.dll') -Force
$include = Get-ChildItem -Path $extract -Recurse -Directory -Filter 'include' | Select-Object -First 1
if ($include) {
    # Replaced, not merged: copying a directory onto an existing directory of
    # the same name nests it (include/include/...) on every run after the
    # first, and the headers quietly move a level deeper each time.
    $target = Join-Path $mpvDir 'include'
    Remove-Item -Recurse -Force $target -ErrorAction SilentlyContinue
    Copy-Item $include.FullName $target -Recurse -Force
}

# --- import library ----------------------------------------------------------
# `-name libmpv-2.dll` matters: without it lib.exe records the name of the .def
# file as the DLL to load at runtime, and the program starts looking for
# "mpv.dll", which does not exist.
$lib = Find-VsTool 'lib.exe'
if (-not $lib) {
    throw "lib.exe not found. Install the MSVC build tools, or run this from a Developer PowerShell."
}

if (-not $def) {
    # Some builds ship without mpv.def; the export table in the DLL has the same
    # information and dumpbin can print it.
    $dumpbin = Find-VsTool 'dumpbin.exe'
    if (-not $dumpbin) { throw "the archive has no mpv.def and dumpbin.exe was not found to derive one" }
    $defPath = Join-Path $libDir 'mpv.def'
    "EXPORTS" | Set-Content $defPath
    & $dumpbin /exports (Join-Path $binDir 'libmpv-2.dll') |
        Select-String -Pattern '^\s+\d+\s+[0-9A-F]+\s+[0-9A-F]+\s+(\S+)' |
        ForEach-Object { $_.Matches[0].Groups[1].Value } |
        Add-Content $defPath
    $def = Get-Item $defPath
} else {
    Copy-Item $def.FullName (Join-Path $libDir 'mpv.def') -Force
    $def = Get-Item (Join-Path $libDir 'mpv.def')
}

& $lib "/def:$($def.FullName)" '/name:libmpv-2.dll' "/out:$(Join-Path $libDir 'mpv.lib')" '/machine:x64' | Out-Null

Write-Host ""
Report-State
Write-Host ""
Write-Host "libmpv $($release.tag_name) is ready. For this shell:"
Write-Host "  `$env:MPV_LIB_DIR = '$libDir'"
Write-Host "  `$env:PATH = '$binDir;' + `$env:PATH   # so a dev build finds libmpv-2.dll"
