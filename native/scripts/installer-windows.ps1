# Build the Windows installer (Inno Setup) from release binaries: the console
# and the server, installed as a local station (native/packaging/windows/
# voltrus-scada.iss). Installs or upgrades Inno Setup with Chocolatey when
# ISCC 6.3 or later is not on the machine. Writes the installer's path to the
# pipeline (everything else goes to the host).
# Usage: native/scripts/installer-windows.ps1 [-BinDir target/release] [-Out dist]
#          [-Version x.y.z (default: the server's version in Cargo.toml)]
#          [-Name voltrus-scada-windows-x86_64-setup]
param(
    [string] $BinDir = 'target/release',
    [string] $Out = 'dist',
    [string] $Version = '',
    [string] $Name = 'voltrus-scada-windows-x86_64-setup'
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$script = Join-Path $root 'native/packaging/windows/voltrus-scada.iss'
$BinDir = (Resolve-Path $BinDir).Path
foreach ($exe in 'voltrus-native.exe', 'voltrus.exe') {
    if (-not (Test-Path (Join-Path $BinDir $exe))) { throw "$exe is not in ${BinDir}: build the release first" }
}
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path

if (-not $Version) {
    $line = Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "(.+)"' | Select-Object -First 1
    if (-not $line) { throw 'no version in Cargo.toml' }
    $Version = $line.Matches[0].Groups[1].Value
}
# Windows version resources take four numbers; a pre-release suffix is dropped.
if ($Version -notmatch '^(\d+)\.(\d+)\.(\d+)') { throw "version '$Version' is not x.y.z" }
$numeric = "$($Matches[1]).$($Matches[2]).$($Matches[3]).0"

function Find-Iscc {
    $candidates = @(
        (Get-Command ISCC.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source),
        (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
        (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe'),
        (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe')
    )
    foreach ($path in $candidates) {
        if ($path -and (Test-Path $path)) {
            # ISCC.exe carries no version resource; its compiler DLL and IDE
            # beside it do (a Chocolatey shim has neither and is skipped).
            $dir = Split-Path $path
            $found = [version] '0.0'
            foreach ($file in 'ISCC.exe', 'ISCmplr.dll', 'Compil32.exe') {
                $candidate = Join-Path $dir $file
                if (-not (Test-Path $candidate)) { continue }
                $info = (Get-Item $candidate).VersionInfo
                if ($info.FileMajorPart -gt 0) {
                    $found = [version]::new($info.FileMajorPart, $info.FileMinorPart)
                    break
                }
            }
            if ($found -ge [version] '6.3') { return $path }
            Write-Host "ignoring Inno Setup $found at $path (6.3 or later is needed)"
        }
    }
    return $null
}

$iscc = Find-Iscc
if (-not $iscc) {
    Write-Host 'installing Inno Setup with Chocolatey'
    choco upgrade innosetup -y --no-progress | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "choco upgrade innosetup failed ($LASTEXITCODE)" }
    $iscc = Find-Iscc
    if (-not $iscc) { throw 'Inno Setup 6.3 or later is not installed' }
}
Write-Host "Inno Setup: $iscc"

& $iscc /Q "/DAppVersion=$Version" "/DAppNumericVersion=$numeric" "/DBinDir=$BinDir" `
    "/DOutputDir=$Out" "/DOutputBaseFilename=$Name" $script | Out-Host
if ($LASTEXITCODE -ne 0) { throw "ISCC failed ($LASTEXITCODE)" }
$installer = Join-Path $Out "$Name.exe"
if (-not (Test-Path $installer)) { throw "ISCC wrote no $installer" }
Write-Host "Built $installer ($Version)"
$installer
